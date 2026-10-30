//! Static package boundaries. This module never starts build tools or follows dependencies.
use crate::{config::ComponentConfig, model::*};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component as PathComponent, Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

pub const SHARED_ID: &str = "repository:shared";

#[derive(Clone, Copy, Debug)]
enum Kind {
    Cargo,
    Npm,
    Pyproject,
    SetupCfg,
    SetupPy,
    Go,
    Maven,
    GradleBuild,
    GradleSettings,
    Dotnet,
}

impl Kind {
    fn ecosystem(self) -> Ecosystem {
        match self {
            Self::Cargo => Ecosystem::Cargo,
            Self::Npm => Ecosystem::Npm,
            Self::Pyproject | Self::SetupCfg | Self::SetupPy => Ecosystem::Python,
            Self::Go => Ecosystem::Go,
            Self::Maven => Ecosystem::Maven,
            Self::GradleBuild | Self::GradleSettings => Ecosystem::Gradle,
            Self::Dotnet => Ecosystem::Dotnet,
        }
    }
}

#[derive(Clone, Debug)]
struct Boundary {
    root: PathBuf,
    manifest: PathBuf,
    ecosystem: Ecosystem,
    name: Option<String>,
    status: ComponentStatus,
    priority: u8,
}

#[derive(Clone, Debug, Default)]
struct Facts {
    boundaries: Vec<Boundary>,
    problem: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CacheStats {
    pub manifests_read: usize,
    pub additional_reads: usize,
    pub directories_read: usize,
    pub cache_hits: usize,
}

/// Shared with the scanner only in component mode. Cache small parsed facts, not file contents.
pub struct Discovery {
    root: PathBuf,
    ecosystems: BTreeSet<Ecosystem>,
    max_size: u64,
    facts: Mutex<BTreeMap<PathBuf, Facts>>,
    additional_reads: AtomicUsize,
    directories_read: AtomicUsize,
    cache_hits: AtomicUsize,
}

impl Discovery {
    pub fn new(root: &Path, config: &ComponentConfig, max_size: u64) -> Self {
        Self {
            root: root.to_path_buf(),
            ecosystems: config.ecosystems.iter().copied().collect(),
            max_size,
            facts: Mutex::new(BTreeMap::new()),
            additional_reads: AtomicUsize::new(0),
            directories_read: AtomicUsize::new(0),
            cache_hits: AtomicUsize::new(0),
        }
    }

    fn kind(&self, path: &Path) -> Option<Kind> {
        let name = path.file_name()?.to_str()?;
        let kind = match name {
            "Cargo.toml" => Kind::Cargo,
            "package.json" => Kind::Npm,
            "pyproject.toml" => Kind::Pyproject,
            "setup.cfg" => Kind::SetupCfg,
            "setup.py" => Kind::SetupPy,
            "go.mod" => Kind::Go,
            "pom.xml" => Kind::Maven,
            "build.gradle" | "build.gradle.kts" => Kind::GradleBuild,
            "settings.gradle" | "settings.gradle.kts" => Kind::GradleSettings,
            _ if path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| {
                    ["csproj", "fsproj", "vbproj"].contains(&ext.to_ascii_lowercase().as_str())
                }) =>
            {
                Kind::Dotnet
            }
            _ => return None,
        };
        self.ecosystems.contains(&kind.ecosystem()).then_some(kind)
    }

    pub fn is_manifest(&self, path: &Path) -> bool {
        self.kind(path).is_some()
    }

    pub fn observe(&self, path: &Path, bytes: &[u8]) {
        if let Some(kind) = self.kind(path) {
            let facts = std::str::from_utf8(bytes)
                .context("manifest is not UTF-8")
                .and_then(|text| parse(path, kind, text, &self.root));
            self.remember(path, kind, facts);
        }
    }

    pub fn observe_problem(&self, path: &Path, message: &str) {
        if let Some(kind) = self.kind(path) {
            self.remember(path, kind, Err(anyhow::anyhow!("{message}")));
        }
    }

    fn remember(&self, path: &Path, kind: Kind, result: Result<Vec<Boundary>>) {
        let facts = match result {
            Ok(boundaries) => Facts {
                boundaries,
                problem: None,
            },
            Err(error) => Facts {
                boundaries: vec![Boundary {
                    root: path.parent().unwrap().to_path_buf(),
                    manifest: path.to_path_buf(),
                    ecosystem: kind.ecosystem(),
                    name: None,
                    status: ComponentStatus::Unresolved,
                    priority: 0,
                }],
                problem: Some(format!("{error:#}")),
            },
        };
        self.facts
            .lock()
            .unwrap()
            .entry(path.to_path_buf())
            .or_insert(facts);
    }

    fn load(&self, path: &Path) -> Facts {
        if let Some(facts) = self.facts.lock().unwrap().get(path).cloned() {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            return facts;
        }
        self.additional_reads.fetch_add(1, Ordering::Relaxed);
        let result = (|| -> Result<Vec<u8>> {
            ensure!(
                !fs::symlink_metadata(path)?.file_type().is_symlink(),
                "manifest symlinks are not followed"
            );
            let file = fs::File::open(path)?;
            ensure!(
                file.metadata()?.len() <= self.max_size,
                "manifest exceeds max-file-size"
            );
            let mut bytes = Vec::new();
            file.take(self.max_size.saturating_add(1))
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= self.max_size,
                "manifest exceeds max-file-size"
            );
            Ok(bytes)
        })();
        match result {
            Ok(bytes) => self.observe(path, &bytes),
            Err(error) => self.observe_problem(path, &error.to_string()),
        }
        self.facts
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .unwrap_or_default()
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            manifests_read: self.facts.lock().unwrap().len(),
            additional_reads: self.additional_reads.load(Ordering::Relaxed),
            directories_read: self.directories_read.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
        }
    }

    pub fn assign(&self, report: &mut ScanReport) -> bool {
        // Seed declarations from the normal traversal, including literal Gradle remappings.
        let mut boundaries = BTreeMap::<PathBuf, Vec<Boundary>>::new();
        let mut loaded = BTreeSet::new();
        for (path, facts) in self.facts.lock().unwrap().iter() {
            loaded.insert(path.clone());
            for boundary in &facts.boundaries {
                boundaries
                    .entry(boundary.root.clone())
                    .or_default()
                    .push(boundary.clone());
            }
        }
        let mut directories = BTreeSet::new();
        let mut owners = BTreeMap::<PathBuf, String>::new();
        let mut components = BTreeMap::<String, Component>::new();
        let mut relevant_manifests = BTreeSet::new();
        let mut directory_errors = BTreeMap::new();
        // Discover relevant ancestor metadata first, so assignment is independent of finding order.
        for finding in &report.findings {
            let parent = self
                .root
                .join(&finding.path)
                .parent()
                .unwrap()
                .to_path_buf();
            for directory in parent
                .ancestors()
                .take_while(|dir| dir.starts_with(&self.root))
            {
                if !directories.insert(directory.to_path_buf()) {
                    break;
                }
                self.directories_read.fetch_add(1, Ordering::Relaxed);
                match fs::read_dir(directory) {
                    Ok(entries) => {
                        let mut paths = Vec::new();
                        for entry in entries {
                            match entry {
                                Ok(entry) => {
                                    if self.is_manifest(&entry.path()) {
                                        match entry.file_type() {
                                            Ok(kind) if kind.is_file() => paths.push(entry.path()),
                                            Ok(_) => {}
                                            Err(error) => {
                                                directory_errors.insert(
                                                    directory.to_path_buf(),
                                                    error.to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                                Err(error) => {
                                    directory_errors
                                        .insert(directory.to_path_buf(), error.to_string());
                                }
                            }
                        }
                        paths.sort();
                        for path in paths {
                            let facts = self.load(&path);
                            if loaded.insert(path) {
                                for boundary in facts.boundaries {
                                    boundaries
                                        .entry(boundary.root.clone())
                                        .or_default()
                                        .push(boundary);
                                }
                            }
                        }
                    }
                    Err(error) => {
                        directory_errors.insert(directory.to_path_buf(), error.to_string());
                    }
                }
            }
        }
        for finding in &mut report.findings {
            let parent = self
                .root
                .join(&finding.path)
                .parent()
                .unwrap()
                .to_path_buf();
            if !owners.contains_key(&parent) {
                let deepest = parent
                    .ancestors()
                    .take_while(|dir| dir.starts_with(&self.root))
                    .find(|directory| {
                        boundaries.contains_key(*directory)
                            || directory_errors.contains_key(*directory)
                    });
                let component = if let Some(root) = deepest {
                    let at_root: Vec<_> = boundaries.get(root).into_iter().flatten().collect();
                    relevant_manifests.extend(at_root.iter().map(|b| b.manifest.clone()));
                    let mut component = assemble(root, &at_root, &self.root);
                    if directory_errors.contains_key(root) {
                        component.status = ComponentStatus::Unresolved;
                    }
                    component
                } else {
                    Component {
                        id: SHARED_ID.into(),
                        root: ".".into(),
                        identities: vec![],
                        status: ComponentStatus::Shared,
                    }
                };
                owners.insert(parent.clone(), component.id.clone());
                components.entry(component.id.clone()).or_insert(component);
            }
            finding.component_id = owners.get(&parent).cloned();
        }
        report.components = components.into_values().collect();
        report.components.sort_by(|a, b| {
            a.root
                .split('/')
                .cmp(b.root.split('/'))
                .then(a.id.cmp(&b.id))
        });
        let mut complete = true;
        for (directory, error) in directory_errors {
            complete = false;
            report.warn(
                "components",
                format!(
                    "{}: cannot inspect component directory: {error}",
                    relative(&directory, &self.root)
                ),
            );
        }
        for manifest in relevant_manifests {
            if let Some(problem) = self
                .facts
                .lock()
                .unwrap()
                .get(&manifest)
                .and_then(|facts| facts.problem.clone())
            {
                complete = false;
                report.warn(
                    "components",
                    format!(
                        "{}: {problem}; using unresolved directory boundary",
                        relative(&manifest, &self.root)
                    ),
                );
            }
        }
        complete
    }
}

fn assemble(root: &Path, boundaries: &[&Boundary], scan_root: &Path) -> Component {
    let root_name = relative(root, scan_root);
    let fallback = if root_name == "." {
        "Repository root"
    } else {
        &root_name
    };
    let mut preferred = BTreeMap::<Ecosystem, &Boundary>::new();
    for &boundary in boundaries {
        if boundary.name.is_some()
            && preferred
                .get(&boundary.ecosystem)
                .is_none_or(|old| boundary.priority > old.priority)
        {
            preferred.insert(boundary.ecosystem, boundary);
        }
    }
    let identities: BTreeSet<_> = boundaries
        .iter()
        .map(|boundary| {
            let selected = if boundary.ecosystem == Ecosystem::Python || boundary.name.is_none() {
                preferred
                    .get(&boundary.ecosystem)
                    .copied()
                    .unwrap_or(boundary)
            } else {
                boundary
            };
            ComponentIdentity {
                ecosystem: boundary.ecosystem,
                name: selected.name.clone().unwrap_or_else(|| fallback.into()),
                manifest: relative(&boundary.manifest, scan_root),
                status: if boundary.status == ComponentStatus::Unresolved {
                    ComponentStatus::Unresolved
                } else {
                    selected.status
                },
            }
        })
        .collect();
    let status = identities
        .iter()
        .map(|i| i.status)
        .max()
        .unwrap_or(ComponentStatus::Unresolved);
    Component {
        id: format!("directory:{root_name}"),
        root: root_name,
        identities: identities.into_iter().collect(),
        status,
    }
}

fn relative(path: &Path, root: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    if relative.as_os_str().is_empty() {
        ".".into()
    } else {
        relative.to_string_lossy().replace('\\', "/")
    }
}

fn boundary(path: &Path, kind: Kind, name: Option<String>, priority: u8) -> Boundary {
    let status = if name.is_some() {
        ComponentStatus::Detected
    } else {
        ComponentStatus::Inferred
    };
    Boundary {
        root: path.parent().unwrap().to_path_buf(),
        manifest: path.to_path_buf(),
        ecosystem: kind.ecosystem(),
        name,
        status,
        priority,
    }
}

fn literal(value: Option<&str>) -> Option<String> {
    value
        .filter(|s| !s.trim().is_empty() && !s.contains(['$', '\n', '\r']))
        .map(str::to_owned)
}

fn parse(path: &Path, kind: Kind, text: &str, root: &Path) -> Result<Vec<Boundary>> {
    let result = match kind {
        Kind::Cargo => {
            let manifest: toml::Value = toml::from_str(text)?;
            manifest.get("package").map(|package| {
                boundary(
                    path,
                    kind,
                    literal(package.get("name").and_then(toml::Value::as_str)),
                    100,
                )
            })
        }
        Kind::Npm => {
            let manifest: serde_json::Value = serde_json::from_str(text)?;
            ensure!(manifest.is_object(), "package.json must be an object");
            Some(boundary(
                path,
                kind,
                literal(manifest.get("name").and_then(serde_json::Value::as_str)),
                100,
            ))
        }
        Kind::Pyproject => {
            let manifest: toml::Value = toml::from_str(text)?;
            let project = manifest.get("project");
            let poetry = manifest.get("tool").and_then(|t| t.get("poetry"));
            if project.is_some() || poetry.is_some() || manifest.get("build-system").is_some() {
                let name = project
                    .and_then(|p| p.get("name"))
                    .and_then(toml::Value::as_str)
                    .or_else(|| {
                        poetry
                            .and_then(|p| p.get("name"))
                            .and_then(toml::Value::as_str)
                    });
                Some(boundary(path, kind, literal(name), 100))
            } else {
                None
            }
        }
        Kind::SetupCfg => {
            let mut metadata = false;
            let mut section = "";
            let mut name = None;
            for line in text
                .lines()
                .map(str::trim)
                .filter(|l| !l.starts_with(['#', ';']) && !l.is_empty())
            {
                if line.starts_with('[') {
                    ensure!(line.ends_with(']'), "malformed setup.cfg section");
                    section = &line[1..line.len() - 1];
                    metadata |= section == "metadata";
                } else if section == "metadata"
                    && let Some((key, value)) = line.split_once('=')
                    && key.trim() == "name"
                {
                    name = literal(Some(value.trim()));
                }
            }
            metadata.then(|| boundary(path, kind, name, 50))
        }
        Kind::SetupPy => Some(boundary(path, kind, None, 0)),
        Kind::Go => {
            let tokens = lex(text)?;
            let mut name = None;
            for (i, token) in tokens.iter().enumerate() {
                if token.word() != Some("module") || (i > 0 && tokens[i - 1] != Token::Symbol('\n'))
                {
                    continue;
                }
                let parts: Vec<_> = tokens[i + 1..]
                    .iter()
                    .skip_while(|t| matches!(t, Token::Symbol('(' | '\n')))
                    .take_while(|t| !matches!(t, Token::Symbol('\n' | ')')))
                    .collect();
                let value = parts.iter().map(|t| t.text()).collect::<String>();
                name = literal(Some(value.trim()));
                break;
            }
            ensure!(name.is_some(), "go.mod has no readable module directive");
            Some(boundary(path, kind, name, 100))
        }
        Kind::Maven | Kind::Dotnet => {
            let document = roxmltree::Document::parse(text)?;
            let element = document.root_element();
            ensure!(
                element.tag_name().name()
                    == if matches!(kind, Kind::Maven) {
                        "project"
                    } else {
                        "Project"
                    },
                "unexpected project XML root"
            );
            let name = if matches!(kind, Kind::Maven) {
                literal(
                    element
                        .children()
                        .find(|n| n.is_element() && n.tag_name().name() == "artifactId")
                        .and_then(|n| n.text())
                        .map(str::trim),
                )
            } else {
                path.file_stem().and_then(|s| s.to_str()).map(str::to_owned)
            };
            Some(boundary(path, kind, name, 100))
        }
        Kind::GradleBuild => Some(boundary(path, kind, None, 0)),
        Kind::GradleSettings => return gradle(path, text, root),
    };
    Ok(result.into_iter().collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    String(String),
    Symbol(char),
}
impl Token {
    fn word(&self) -> Option<&str> {
        if let Self::Word(s) = self {
            Some(s)
        } else {
            None
        }
    }
    fn string(&self) -> Option<String> {
        if let Self::String(s) = self {
            literal(Some(s))
        } else {
            None
        }
    }
    fn text(&self) -> String {
        match self {
            Self::Word(s) | Self::String(s) => s.clone(),
            Self::Symbol(c) => c.to_string(),
        }
    }
}

/// Small literal lexer, not an evaluator. Comments and strings cannot introduce fake declarations.
fn lex(text: &str) -> Result<Vec<Token>> {
    let chars: Vec<_> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                if chars[i] == '\n' {
                    tokens.push(Token::Symbol('\n'));
                }
                i += 1;
            }
            ensure!(i < chars.len(), "unterminated block comment in manifest");
            i = (i + 2).min(chars.len());
        } else if matches!(c, '\'' | '"' | '`') {
            i += 1;
            let mut value = String::new();
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' && c != '`' && i + 1 < chars.len() {
                    i += 1;
                }
                value.push(chars[i]);
                i += 1;
            }
            ensure!(i < chars.len(), "unterminated quoted string in manifest");
            tokens.push(Token::String(value));
            i = (i + 1).min(chars.len());
        } else if c.is_ascii_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Word(chars[start..i].iter().collect()));
        } else {
            if !c.is_whitespace() || c == '\n' {
                tokens.push(Token::Symbol(c));
            }
            i += 1;
        }
    }
    Ok(tokens)
}

#[derive(Clone)]
struct Project {
    directory: String,
    name: String,
    inferred: bool,
}

fn gradle(path: &Path, text: &str, scan_root: &Path) -> Result<Vec<Boundary>> {
    let tokens = lex(text)?;
    let base = path.parent().unwrap();
    let mut root_name = None;
    let mut uncertain = false;
    let mut projects = BTreeMap::<String, Project>::new();
    let mut mutations = Vec::new();
    for i in 0..tokens.len() {
        if tokens[i].word() == Some("rootProject")
            && tokens.get(i + 1) == Some(&Token::Symbol('.'))
            && tokens.get(i + 2).and_then(Token::word) == Some("name")
            && tokens.get(i + 3) == Some(&Token::Symbol('='))
        {
            root_name = complete_string(&tokens, i + 4);
            uncertain |= root_name.is_none();
        }
        if tokens[i].word() == Some("include") {
            let mut j = i + 1;
            let parenthesized = tokens.get(j) == Some(&Token::Symbol('('));
            if parenthesized {
                j += 1;
            }
            let mut names = Vec::new();
            let mut valid = true;
            while let Some(token) = tokens.get(j) {
                match token {
                    Token::String(_) => {
                        if let Some(name) = token.string() {
                            names.push(name);
                        } else {
                            valid = false;
                        }
                    }
                    Token::Symbol(',') => {}
                    Token::Symbol('\n') if parenthesized => {}
                    Token::Symbol(')') if parenthesized => break,
                    Token::Symbol('\n' | ';') if !parenthesized => break,
                    _ => {
                        valid = false;
                        break;
                    }
                }
                j += 1;
            }
            if valid {
                for name in names {
                    let parts: Vec<_> = name.trim_matches(':').split(':').collect();
                    if parts
                        .iter()
                        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains(['/', '\\']))
                    {
                        continue;
                    }
                    projects
                        .entry(name.trim_matches(':').into())
                        .or_insert(Project {
                            directory: parts.join("/"),
                            name: parts.last().unwrap().to_string(),
                            inferred: false,
                        });
                }
            } else {
                uncertain = true;
            }
        }
        if tokens[i].word() == Some("project")
            && tokens.get(i + 1) == Some(&Token::Symbol('('))
            && let Some(project) = tokens.get(i + 2).and_then(Token::string)
            && tokens.get(i + 3) == Some(&Token::Symbol(')'))
            && tokens.get(i + 4) == Some(&Token::Symbol('.'))
            && let Some(property) = tokens.get(i + 5).and_then(Token::word)
            && tokens.get(i + 6) == Some(&Token::Symbol('='))
        {
            let value = if property == "projectDir"
                && tokens.get(i + 7).and_then(Token::word) == Some("file")
                && tokens.get(i + 8) == Some(&Token::Symbol('('))
            {
                tokens.get(i + 9).and_then(Token::string).filter(|_| {
                    tokens.get(i + 10) == Some(&Token::Symbol(')'))
                        && expression_end(&tokens, i + 11)
                })
            } else if property == "name" {
                complete_string(&tokens, i + 7)
            } else {
                continue;
            };
            mutations.push((
                project.trim_matches(':').to_owned(),
                property.to_owned(),
                value,
            ));
        }
    }
    for (project, property, value) in mutations {
        if let Some(descriptor) = projects.get_mut(&project) {
            match (property.as_str(), value) {
                ("name", Some(name)) => descriptor.name = name,
                ("projectDir", Some(directory)) => descriptor.directory = directory,
                _ => {
                    descriptor.inferred = true;
                    uncertain = true;
                }
            }
        }
    }
    let mut boundaries = vec![boundary(path, Kind::GradleSettings, root_name, 100)];
    if uncertain {
        boundaries[0].status = ComponentStatus::Inferred;
    }
    for project in projects.into_values() {
        if let Some(root) = contained_directory(base, &project.directory, scan_root) {
            boundaries.push(Boundary {
                root,
                manifest: path.to_path_buf(),
                ecosystem: Ecosystem::Gradle,
                name: Some(project.name),
                priority: 100,
                status: if uncertain || project.inferred {
                    ComponentStatus::Inferred
                } else {
                    ComponentStatus::Detected
                },
            });
        }
    }
    Ok(boundaries)
}

fn complete_string(tokens: &[Token], index: usize) -> Option<String> {
    tokens
        .get(index)
        .and_then(Token::string)
        .filter(|_| expression_end(tokens, index + 1))
}

fn expression_end(tokens: &[Token], index: usize) -> bool {
    tokens
        .get(index)
        .is_none_or(|t| matches!(t, Token::Symbol('\n' | ';' | '}')))
}

fn contained_directory(base: &Path, value: &str, root: &Path) -> Option<PathBuf> {
    let value = Path::new(value);
    let mut directory = if value.is_absolute() {
        root.to_path_buf()
    } else {
        base.to_path_buf()
    };
    let value = if value.is_absolute() {
        value.strip_prefix(root).ok()?
    } else {
        value
    };
    for part in value.components() {
        match part {
            PathComponent::CurDir => {}
            PathComponent::Normal(part) => directory.push(part),
            PathComponent::ParentDir => {
                directory.pop();
            }
            _ => return None,
        }
        if !directory.starts_with(root)
            || fs::symlink_metadata(&directory).is_ok_and(|m| m.file_type().is_symlink())
        {
            return None;
        }
    }
    directory.is_dir().then_some(directory)
}
