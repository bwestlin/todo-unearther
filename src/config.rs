use crate::cli::{
    AttributionMode, Cli, Compare, ComponentLayout, Detail, Format, GroupBy, InnerGroupBy, Links,
};
use crate::model::Ecosystem;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const CONFIG_NAME: &str = ".todo-unearther.toml";
pub const DEFAULT_MARKERS: [&str; 5] = ["TODO", "FIXME", "HACK", "XXX", "BUG"];

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub scan: ScanConfig,
    pub report: ReportConfig,
    pub components: ComponentConfig,
    pub github: GithubConfig,
    pub git: GitConfig,
    pub check: CheckConfig,
    pub policy: PolicyConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanConfig {
    pub markers: Vec<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub default_excludes: bool,
    pub respect_ignore: bool,
    pub fallback: bool,
    pub jobs: usize,
    pub max_file_size: u64,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            markers: DEFAULT_MARKERS.map(String::from).into(),
            include: vec![],
            exclude: vec![],
            default_excludes: true,
            respect_ignore: true,
            fallback: true,
            jobs: std::thread::available_parallelism().map_or(1, usize::from),
            max_file_size: 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReportConfig {
    pub format: Option<Format>,
    pub output: Option<PathBuf>,
    pub detail: Detail,
    pub group_by: GroupBy,
    pub component_group_by: InnerGroupBy,
    pub component_layout: ComponentLayout,
    pub width: Option<u16>,
    pub line_spacing: u16,
    pub links: Links,
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self {
            format: None,
            output: None,
            detail: Detail::default(),
            group_by: GroupBy::default(),
            component_group_by: InnerGroupBy::default(),
            component_layout: ComponentLayout::default(),
            width: None,
            line_spacing: 1,
            links: Links::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ComponentConfig {
    pub ecosystems: Vec<Ecosystem>,
}

impl Default for ComponentConfig {
    fn default() -> Self {
        Self {
            ecosystems: Ecosystem::ALL.to_vec(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GithubConfig {
    pub enabled: bool,
    pub host: String,
    pub api_url: Option<String>,
    pub repository: Option<String>,
    pub gh_auth: bool,
    pub timeout_seconds: u64,
}

impl Default for GithubConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "github.com".into(),
            api_url: None,
            repository: None,
            gh_auth: true,
            timeout_seconds: 10,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GitConfig {
    pub attribution: AttributionMode,
    pub ignore_whitespace: bool,
    pub detect_moves: bool,
    pub detect_copies: bool,
    pub history_limit: usize,
    pub timeout_seconds: u64,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            attribution: AttributionMode::Off,
            ignore_whitespace: true,
            detect_moves: true,
            detect_copies: false,
            history_limit: 200,
            timeout_seconds: 30,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CheckConfig {
    pub compare: Compare,
    pub strict: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PolicyConfig {
    pub max_total: Option<usize>,
    pub max_by_marker: BTreeMap<String, usize>,
    pub require_references: Vec<String>,
    pub fail_on_closed: bool,
    pub fail_on_unavailable: bool,
}

impl PolicyConfig {
    pub fn needs_github(&self) -> bool {
        self.fail_on_closed || self.fail_on_unavailable
    }
}

impl Config {
    pub fn load(root: &Path, explicit: Option<&Path>) -> Result<(Self, PathBuf)> {
        let path = explicit
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join(CONFIG_NAME));
        let mut config: Self = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("invalid configuration {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && explicit.is_none() => {
                Self::default()
            }
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        if let Some(output) = &mut config.report.output
            && output != Path::new("-")
            && output.is_relative()
        {
            *output = path.parent().unwrap_or(root).join(&output);
        }
        Ok((config, path))
    }

    pub fn apply(&mut self, cli: &Cli) -> Result<()> {
        if let Some(v) = &cli.markers {
            self.scan.markers.clone_from(v);
        }
        self.scan.include.extend(cli.include.clone());
        self.scan.exclude.extend(cli.exclude.clone());
        if cli.no_default_excludes {
            self.scan.default_excludes = false;
        }
        if cli.no_ignore {
            self.scan.respect_ignore = false;
        }
        if cli.no_fallback {
            self.scan.fallback = false;
        }
        if let Some(v) = cli.jobs {
            self.scan.jobs = v;
        }
        if let Some(v) = cli.max_file_size {
            self.scan.max_file_size = v;
        }
        if let Some(v) = cli.format {
            self.report.format = Some(v);
        }
        if let Some(v) = cli.detail {
            self.report.detail = v;
        }
        if let Some(v) = cli.group_by {
            self.report.group_by = v;
        }
        if let Some(v) = cli.component_group_by {
            self.report.component_group_by = v;
        }
        if let Some(v) = cli.component_layout {
            self.report.component_layout = v;
        }
        if let Some(v) = cli.width {
            self.report.width = Some(v);
        }
        if let Some(v) = cli.line_spacing {
            self.report.line_spacing = v;
        }
        self.components.ecosystems.sort();
        self.components.ecosystems.dedup();
        if let Some(v) = cli.links {
            self.report.links = v;
        }
        if cli.github {
            self.github.enabled = true;
        }
        if cli.no_github {
            self.github.enabled = false;
        }
        if let Some(v) = &cli.repository {
            self.github.repository = Some(v.clone());
        }
        if let Some(v) = &cli.github_host {
            self.github.host = v.clone();
        }
        if let Some(v) = &cli.api_url {
            self.github.api_url = Some(v.clone());
        }
        if let Some(v) = cli.attribution {
            self.git.attribution = v;
        }
        if let Some(v) = cli.history_limit {
            self.git.history_limit = v;
        }
        if let Some(v) = cli.git_timeout {
            self.git.timeout_seconds = v;
        }
        if let Some(v) = cli.git_ignore_whitespace {
            self.git.ignore_whitespace = v;
        }
        if let Some(v) = cli.git_detect_moves {
            self.git.detect_moves = v;
        }
        if let Some(v) = cli.git_detect_copies {
            self.git.detect_copies = v;
        }
        if let Some(v) = cli.compare {
            self.check.compare = v;
        }
        self.check.strict |= cli.strict;
        if let Some(v) = cli.max_total {
            self.policy.max_total = Some(v);
        }
        self.policy.fail_on_closed |= cli.fail_on_closed;
        self.policy.fail_on_unavailable |= cli.fail_on_unavailable;
        ensure!(self.scan.jobs > 0, "jobs must be greater than zero");
        ensure!(
            self.report.width.is_none_or(|width| width >= 20),
            "report.width must be at least 20 columns"
        );
        ensure!(
            self.scan.max_file_size > 0,
            "max-file-size must be greater than zero"
        );
        ensure!(
            self.git.history_limit > 0,
            "history-limit must be greater than zero"
        );
        ensure!(
            self.git.timeout_seconds > 0 && self.github.timeout_seconds > 0,
            "timeouts must be greater than zero"
        );
        ensure!(
            !self.scan.markers.is_empty(),
            "at least one marker is required"
        );
        for marker in &mut self.scan.markers {
            ensure!(
                !marker.is_empty()
                    && marker
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "markers must be nonempty ASCII words: {marker:?}"
            );
            marker.make_ascii_uppercase();
        }
        self.scan.markers.sort();
        self.scan.markers.dedup();
        self.policy
            .require_references
            .iter_mut()
            .for_each(|m| m.make_ascii_uppercase());
        self.policy.max_by_marker = std::mem::take(&mut self.policy.max_by_marker)
            .into_iter()
            .map(|(m, n)| (m.to_ascii_uppercase(), n))
            .collect();
        self.github.host.make_ascii_lowercase();
        if self.github.host.contains(['/', ':', ' ', '\\']) || self.github.host.is_empty() {
            bail!("github.host must be a hostname, without a scheme or port");
        }
        if let Some(repo) = &self.github.repository {
            validate_repository(repo)?;
        }
        crate::github::api_url(&self.github)?;
        Ok(())
    }
}

pub fn validate_repository(repo: &str) -> Result<()> {
    let parts: Vec<_> = repo.split('/').collect();
    ensure!(
        parts.len() == 2
            && parts.iter().all(|part| !part.is_empty()
                && *part != "."
                && *part != ".."
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_ .".contains(c))
                && !part.contains(' ')),
        "repository must be owner/repo"
    );
    Ok(())
}

pub fn discover_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|p| {
            let git = p.join(".git");
            if git.is_dir() {
                git.join("HEAD").exists()
            } else {
                std::fs::read_to_string(git).is_ok_and(|s| s.starts_with("gitdir: "))
            }
        })
        .unwrap_or(cwd)
        .to_path_buf()
}

pub const DOCUMENTED_CONFIG: &str = r#"# Command-line options override these repository settings.
[scan]
markers = ["TODO", "FIXME", "HACK", "XXX", "BUG"]
include = [] # Optional whitelist globs, relative to the scan root.
exclude = [] # Additional exclusion globs.
default_excludes = true # Dependencies and build directories.
respect_ignore = true # .gitignore and .ignore; global Git ignores are not used.
fallback = true # Conservative heuristic for languages without a grammar.
max_file_size = 1048576 # Bytes; jobs defaults to available parallelism.

[report]
group_by = "file" # file, marker, component
component_group_by = "file" # file or marker; inside component sections only.
component_layout = "flat" # flat or tree; component folders are always sorted by path.
detail = "full" # full, summary; JSON always includes all fields.
links = "auto" # auto, enabled, disabled; console only.
line_spacing = 1 # Blank rows between every console line; 0 is compact. Other formats ignore it.
# width = 100 # Console/text columns. Console detects terminal width; text defaults to 100.
# output = "TODO.md" # Relative to this configuration file.
# format = "markdown" # Otherwise inferred from destination/terminal.

[components]
# Only inspected when group_by = "component"; no build tools are executed.
ecosystems = ["cargo", "npm", "python", "go", "maven", "gradle", "dotnet"]

[github]
enabled = false
host = "github.com"
gh_auth = true # GITHUB_TOKEN first; then optional gh auth token.
timeout_seconds = 10
# repository = "owner/repo" # Otherwise read the origin remote when needed.
# api_url = "https://github.example.com/api/v3"

[git]
attribution = "off" # off, blame, history
ignore_whitespace = true
detect_moves = true
detect_copies = false
history_limit = 200 # Maximum line-history records inspected per finding.
timeout_seconds = 30 # Total Git work budget per file.

[check]
compare = "source" # source or full
strict = false # Missing enrichment warns rather than failing.

[policy]
require_references = [] # Example: ["FIXME"]
fail_on_closed = false
fail_on_unavailable = false # A 404; network/authentication errors are distinct.
# max_total = 100
# max_by_marker = { FIXME = 10, HACK = 5 }
"#;
