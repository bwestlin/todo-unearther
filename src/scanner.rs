use crate::{components::Discovery, config::ScanConfig, model::*, references};
use aho_corasick::{AhoCorasick, AhoCorasickBuilder};
use anyhow::{Context, Result, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::{WalkBuilder, WalkState};
use regex::Regex;
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tree_sitter::{Language, Parser};

const EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    "coverage",
];
const VCS: &[&str] = &[".git", ".hg", ".svn"];

pub struct ScanResult {
    pub findings: Vec<Finding>,
    pub stats: ScanStats,
    pub diagnostics: Vec<Diagnostic>,
}

struct Matcher {
    prefilter: AhoCorasick,
    marker: Regex,
    boundary: Regex,
}

impl Matcher {
    fn new(markers: &[String]) -> Result<Self> {
        let alternatives = markers
            .iter()
            .map(|m| regex::escape(m))
            .collect::<Vec<_>>()
            .join("|");
        Ok(Self {
            prefilter: AhoCorasickBuilder::new()
                .ascii_case_insensitive(true)
                .build(markers)?,
            marker: Regex::new(&format!(
                r"(?i)^@?(?P<marker>{alternatives})(?:\b)(?P<rest>.*)$"
            ))?,
            boundary: Regex::new(&format!(
                r"(?i)^@?(?:{alternatives}|{})(?:$|[\s:(])",
                crate::config::DEFAULT_MARKERS.join("|")
            ))?,
        })
    }

    fn parse_marker(&self, text: &str) -> Option<(String, Option<String>, String)> {
        let captures = self.marker.captures(text.trim_start())?;
        let marker = captures.name("marker")?.as_str().to_ascii_uppercase();
        let mut rest = captures.name("rest")?.as_str().trim_start();
        if !rest.is_empty()
            && !rest.starts_with([':', '('])
            && !rest.chars().next()?.is_whitespace()
        {
            // Whitespace was trimmed above; a word boundary alone also matches punctuation.
            let untrimmed = captures.name("rest")?.as_str();
            if !untrimmed.starts_with(char::is_whitespace) {
                return None;
            }
        }
        rest = rest.strip_prefix(':').unwrap_or(rest).trim_start();
        let annotation = if rest.starts_with('(') {
            if let Some(end) = rest.find(')') {
                let label = rest[1..end].trim().to_owned();
                rest = rest[end + 1..].trim_start();
                Some(label)
            } else {
                None
            }
        } else {
            None
        };
        rest = rest.strip_prefix(':').unwrap_or(rest).trim_start();
        Some((marker, annotation, rest.trim_end().to_owned()))
    }
}

pub fn scan(root: &Path, config: &ScanConfig, output: Option<&Path>) -> Result<ScanResult> {
    scan_with_components(root, config, output, None)
}

pub fn scan_with_components(
    root: &Path,
    config: &ScanConfig,
    output: Option<&Path>,
    discovery: Option<&Discovery>,
) -> Result<ScanResult> {
    let matcher = Arc::new(Matcher::new(&config.markers)?);
    let root = root.to_path_buf();
    let excluded = globs(&config.exclude)?;
    let included = globs(&config.include)?;
    let output = output.map(|path| output_in_scan_coordinates(path, &root));
    let filter_root = root.clone();
    let defaults = config.default_excludes;
    let respect = config.respect_ignore;
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false)
        .follow_links(false)
        .threads(config.jobs)
        .git_ignore(respect)
        .git_exclude(respect)
        .ignore(respect)
        .git_global(false)
        .require_git(false)
        .parents(false);
    builder.filter_entry(move |entry| {
        let path = entry.path();
        if path == filter_root {
            return true;
        }
        if entry.file_type().is_some_and(|t| t.is_symlink()) {
            return false;
        }
        let name = entry.file_name().to_str().unwrap_or("");
        if VCS.contains(&name) {
            return false;
        }
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        if is_dir && (path.join(".git").exists() || (defaults && EXCLUDED_DIRS.contains(&name))) {
            return false;
        }
        if output
            .as_ref()
            .is_some_and(|p| p == &absolute_lexical(path))
        {
            return false;
        }
        let relative = path.strip_prefix(&filter_root).unwrap_or(path);
        if excluded.is_match(relative)
            || (is_dir && excluded.is_match(format!("{}/", relative.display())))
        {
            return false;
        }
        is_dir || included.is_empty() || included.is_match(relative)
    });
    let accumulated = Mutex::new(Accumulated::default());
    builder.build_parallel().run(|| {
        let matcher = Arc::clone(&matcher);
        let accumulated = &accumulated;
        let root = &root;
        let mut worker = Worker {
            matcher,
            parser: Parser::new(),
            discovery,
        };
        Box::new(move |entry| {
            match entry {
                Err(error) => accumulated.lock().unwrap().errors.push(error.to_string()),
                Ok(entry) if entry.file_type().is_some_and(|t| t.is_file()) => {
                    let result = worker.file(entry.path(), root, config);
                    let mut all = accumulated.lock().unwrap();
                    match result {
                        Ok(file) => {
                            all.stats.files_read += file.stats.files_read;
                            all.stats.bytes_read += file.stats.bytes_read;
                            all.stats.candidate_files += file.stats.candidate_files;
                            all.stats.skipped_binary += file.stats.skipped_binary;
                            all.stats.skipped_encoding += file.stats.skipped_encoding;
                            all.stats.skipped_oversized += file.stats.skipped_oversized;
                            all.findings.extend(file.findings);
                            all.diagnostics.extend(file.diagnostics);
                        }
                        Err(error) => all
                            .errors
                            .push(format!("{}: {error:#}", entry.path().display())),
                    }
                }
                _ => {}
            }
            WalkState::Continue
        })
    });
    let mut all = accumulated.into_inner().unwrap();
    if !all.errors.is_empty() {
        all.errors.sort();
        bail!("source scan incomplete:\n{}", all.errors.join("\n"));
    }
    all.findings
        .sort_by(|a, b| (&a.path, a.line, &a.marker).cmp(&(&b.path, b.line, &b.marker)));
    all.diagnostics.sort_by(|a, b| a.message.cmp(&b.message));
    Ok(ScanResult {
        findings: all.findings,
        stats: all.stats,
        diagnostics: all.diagnostics,
    })
}

#[derive(Default)]
struct Accumulated {
    findings: Vec<Finding>,
    stats: ScanStats,
    diagnostics: Vec<Diagnostic>,
    errors: Vec<String>,
}

#[derive(Default)]
struct FileResult {
    findings: Vec<Finding>,
    stats: ScanStats,
    diagnostics: Vec<Diagnostic>,
}

struct Worker<'a> {
    matcher: Arc<Matcher>,
    parser: Parser,
    discovery: Option<&'a Discovery>,
}

impl Worker<'_> {
    fn file(&mut self, path: &Path, root: &Path, config: &ScanConfig) -> Result<FileResult> {
        let mut result = FileResult::default();
        let file = File::open(path).context("cannot open source")?;
        if file.metadata()?.len() > config.max_file_size {
            if let Some(discovery) = self.discovery {
                discovery.observe_problem(path, "manifest exceeds max-file-size");
            }
            result.stats.skipped_oversized = 1;
            return Ok(result);
        }
        let mut bytes = Vec::new();
        file.take(config.max_file_size.saturating_add(1))
            .read_to_end(&mut bytes)?;
        result.stats.files_read = 1;
        result.stats.bytes_read = bytes.len() as u64;
        if bytes.len() as u64 > config.max_file_size {
            if let Some(discovery) = self.discovery {
                discovery.observe_problem(path, "manifest exceeds max-file-size");
            }
            result.stats.skipped_oversized = 1;
            return Ok(result);
        }
        if bytes.contains(&0) {
            if let Some(discovery) = self.discovery {
                discovery.observe_problem(path, "manifest contains binary data");
            }
            result.stats.skipped_binary = 1;
            return Ok(result);
        }
        if let Some(discovery) = self.discovery {
            discovery.observe(path, &bytes);
        }
        if !self.matcher.prefilter.is_match(&bytes) {
            return Ok(result);
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            result.stats.skipped_encoding = 1;
            return Ok(result);
        };
        if is_generated(text) {
            return Ok(result);
        }
        result.stats.candidate_files = 1;
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_str()
            .context("candidate source path is not valid UTF-8")?
            .replace('\\', "/");
        let segments = if let Some(language) = language(path, text) {
            self.parser.set_language(&language)?;
            let tree = self.parser.parse(text, None).context("parser cancelled")?;
            let mut segments = Vec::new();
            let mut cursor = tree.walk();
            loop {
                let node = cursor.node();
                if node.kind().contains("comment") {
                    segments.push((
                        node.start_byte(),
                        node.end_byte(),
                        node.start_position().row + 1,
                        node.start_position().column,
                    ));
                } else if cursor.goto_first_child() {
                    continue;
                }
                if cursor.goto_next_sibling() {
                    continue;
                }
                loop {
                    if !cursor.goto_parent() {
                        break;
                    }
                    if cursor.goto_next_sibling() {
                        break;
                    }
                }
                if cursor.node() == tree.root_node() {
                    break;
                }
            }
            segments.sort_unstable();
            Some(segments)
        } else {
            None
        };
        if let Some(segments) = segments {
            result.findings = extract_segments(text, &segments, &relative, &self.matcher);
        } else if config.fallback {
            result.findings = extract_fallback(text, &relative, &self.matcher);
        }
        Ok(result)
    }
}

fn language(path: &Path, text: &str) -> Option<Language> {
    Some(
        match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
            "rs" => tree_sitter_rust::LANGUAGE.into(),
            "js" | "jsx" | "mjs" | "cjs" => tree_sitter_javascript::LANGUAGE.into(),
            "ts" | "mts" | "cts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
            "py" | "pyw" | "pyi" => tree_sitter_python::LANGUAGE.into(),
            "go" => tree_sitter_go::LANGUAGE.into(),
            "c" | "h" => tree_sitter_c::LANGUAGE.into(),
            "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => tree_sitter_cpp::LANGUAGE.into(),
            "sh" | "bash" => tree_sitter_bash::LANGUAGE.into(),
            _ if text.lines().next().is_some_and(|s| {
                s.starts_with("#!") && (s.ends_with("/sh") || s.contains("bash"))
            }) =>
            {
                tree_sitter_bash::LANGUAGE.into()
            }
            _ => return None,
        },
    )
}

struct CommentLine<'a> {
    text: &'a str,
    line: usize,
    group: usize,
    indent: usize,
    block: bool,
}

fn extract_segments(
    text: &str,
    segments: &[(usize, usize, usize, usize)],
    path: &str,
    matcher: &Matcher,
) -> Vec<Finding> {
    let mut lines = Vec::new();
    let mut previous: Option<(usize, usize, usize)> = None;
    let mut next_group = 0;
    for &(start, end, row, col) in segments {
        let raw = &text[start..end];
        let block = raw.starts_with("/*");
        let body = if block {
            raw.strip_prefix("/*")
                .unwrap()
                .strip_suffix("*/")
                .unwrap_or(&raw[2..])
        } else {
            raw.strip_prefix("//")
                .or_else(|| raw.strip_prefix('#'))
                .unwrap_or(raw)
        };
        let group = if !block {
            let line_start = text[..start].rfind('\n').map_or(0, |offset| offset + 1);
            let standalone = text[line_start..start].trim().is_empty();
            previous
                .filter(|&(prev_row, prev_col, _)| prev_row + 1 == row && prev_col == col)
                .filter(|_| standalone)
                .map(|(_, _, group)| group)
                .unwrap_or_else(|| {
                    next_group += 1;
                    next_group
                })
        } else {
            next_group += 1;
            next_group
        };
        for (offset, content) in body.lines().enumerate() {
            let content = if block {
                content.trim_start().trim_start_matches('*')
            } else {
                content.trim_start_matches(['/', '!'])
            };
            lines.push(CommentLine {
                text: content.trim_end_matches('\r'),
                line: row + offset,
                group,
                indent: content.len() - content.trim_start().len(),
                block,
            });
        }
        previous = if block { None } else { Some((row, col, group)) };
    }
    extract_lines(&lines, path, matcher, Detection::Comment)
}

fn extract_fallback(text: &str, path: &str, matcher: &Matcher) -> Vec<Finding> {
    let mut lines = Vec::new();
    let mut group = 0;
    let mut previous_row = 0;
    let mut previous_prefix = None;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let prefix = ["<!--", "//", "/*", "--", "#", ";", "%", "*"]
            .into_iter()
            .find(|p| trimmed.starts_with(p));
        let content = prefix.map(|p| &trimmed[p.len()..]).unwrap_or(trimmed);
        let content = content
            .strip_suffix("-->")
            .or_else(|| content.strip_suffix("*/"))
            .unwrap_or(content)
            .trim_end();
        // Bare fallback markers require a colon to avoid harvesting prose headings.
        let bare = matcher.parse_marker(content).is_some() && content.contains(':');
        if prefix.is_none() && !bare {
            previous_row = 0;
            continue;
        }
        let source_indent = line.len() - trimmed.len();
        if previous_row != index || previous_prefix != Some((prefix, source_indent)) {
            group += 1;
        }
        lines.push(CommentLine {
            text: content,
            line: index + 1,
            group,
            indent: content.len() - content.trim_start().len(),
            block: false,
        });
        previous_row = index + 1;
        previous_prefix = Some((prefix, source_indent));
    }
    extract_lines(&lines, path, matcher, Detection::Heuristic)
}

fn extract_lines(
    lines: &[CommentLine<'_>],
    path: &str,
    matcher: &Matcher,
    detection: Detection,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index];
        if let Some((marker, annotation, mut message)) = matcher.parse_marker(line.text) {
            let mut end_line = line.line;
            let mut next = index + 1;
            while let Some(continuation) = lines.get(next) {
                if continuation.group != line.group
                    || continuation.line != end_line + 1
                    || continuation.text.trim().is_empty()
                    || matcher.boundary.is_match(continuation.text.trim_start())
                    || (!line.block && continuation.indent < line.indent)
                {
                    break;
                }
                if !message.is_empty() {
                    message.push('\n');
                }
                message.push_str(continuation.text.trim());
                end_line = continuation.line;
                next += 1;
            }
            let refs = references::parse(&format!(
                "{}\n{message}",
                annotation.as_deref().unwrap_or("")
            ));
            findings.push(Finding {
                path: path.into(),
                line: line.line,
                end_line,
                marker,
                annotation,
                message,
                detection,
                references: refs,
                attribution: None,
                component_id: None,
            });
            index = next;
        } else {
            index += 1;
        }
    }
    findings
}

fn globs(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).with_context(|| format!("invalid glob {pattern:?}"))?);
    }
    Ok(builder.build()?)
}

fn output_in_scan_coordinates(path: &Path, root: &Path) -> PathBuf {
    let path = absolute_lexical(path);
    // Resolve aliases once, then express the output using the traversal's root
    // spelling. This also matches Windows normal and verbatim drive prefixes,
    // without canonicalizing every file visited by the fast scanner.
    if let Some(name) = path.file_name()
        && let Some(parent) = path.parent()
        && let (Ok(parent), Ok(canonical_root)) = (parent.canonicalize(), root.canonicalize())
        && let Ok(relative) = parent.join(name).strip_prefix(canonical_root)
    {
        return absolute_lexical(&root.join(relative));
    }
    path
}

fn absolute_lexical(path: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            other => result.push(other),
        }
    }
    result
}

pub fn is_generated(text: &str) -> bool {
    let prefix = &text[..text.floor_char_boundary(text.len().min(2048))];
    prefix.starts_with("<!-- todo-unearther:")
        || (prefix.starts_with("<!-- markdownlint-disable ")
            && prefix
                .lines()
                .take(3)
                .any(|line| line.starts_with("<!-- todo-unearther:")))
        || prefix.starts_with("<!doctype html>\n<!-- todo-unearther:")
        || prefix.starts_with("todo-unearther report")
        || prefix
            .strip_prefix("todo-unearther\n")
            .is_some_and(|rest| rest.trim_start_matches('\n').starts_with("report\n"))
        || (prefix.starts_with('{') && prefix.contains("\"generator\": \"todo-unearther\""))
}

pub fn counts(findings: &[Finding]) -> BTreeMap<String, usize> {
    let mut result = BTreeMap::new();
    for finding in findings {
        *result.entry(finding.marker.clone()).or_default() += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(extension: &str, text: &str) -> Vec<Finding> {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(format!("test.{extension}")), text).unwrap();
        scan(temp.path(), &ScanConfig::default(), None)
            .unwrap()
            .findings
    }

    #[test]
    fn parses_annotations_on_both_sides_of_colon() {
        for syntax in [
            "TODO(bwestlin): fix",
            "TODO:(bwestlin) fix",
            "TODO: (bwestlin): fix",
            "@todo ( bwestlin ) : fix",
        ] {
            let findings = extract("rs", &format!("// {syntax}"));
            assert_eq!(findings.len(), 1, "{syntax}");
            assert_eq!(findings[0].annotation.as_deref(), Some("bwestlin"));
            assert_eq!(findings[0].message, "fix");
        }
    }

    #[test]
    fn rejects_strings_and_extracts_block_and_indented_comments() {
        let findings = extract(
            "rs",
            "let x = r#\"// TODO: not a comment\"#;\n// TODO: first\n//   continued\n//\n// ordinary comment\n/* FIXME(#12): second\n * more details\n *\n * not part of second\n */\n",
        );
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].message, "first\ncontinued");
        assert_eq!((findings[0].line, findings[0].end_line), (2, 3));
        assert_eq!(findings[1].message, "second\nmore details");
        assert_eq!(findings[1].references[0].number, 12);
    }

    #[test]
    fn supported_languages_ignore_string_literals() {
        let samples = [
            ("js", "const x = `// TODO: false`; // FIXME: real"),
            ("tsx", "const x = <div>{/* TODO: real */}</div>;"),
            ("py", "x = '# TODO: false'\n# FIXME: real"),
            (
                "go",
                "package main\nvar s = `// TODO: false`\n// FIXME: real",
            ),
            ("c", "char *x = \"// TODO: false\"; // FIXME: real"),
            ("cpp", "auto x = R\"(// TODO: false)\"; // FIXME: real"),
            ("sh", "echo '# TODO: false'\n# FIXME: real"),
        ];
        for (extension, text) in samples {
            assert_eq!(extract(extension, text).len(), 1, "{extension}");
        }
    }

    #[test]
    fn fallback_crlf_unicode_and_marker_boundaries() {
        let findings = extract(
            "unknown",
            "TODO: åtgärda 🛶\r\n# FIXME:(owner) next\r\nTODOLOG: no\r\n",
        );
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].message, "åtgärda 🛶");
        assert_eq!(findings[0].detection, Detection::Heuristic);
    }

    #[test]
    fn continuation_stops_when_code_precedes_the_next_comment() {
        let findings = extract(
            "rs",
            "fn main() {\n    one(); // TODO: task\n    two(); //   an unrelated trailing comment\n}\n",
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].message, "task");
    }

    #[test]
    fn same_indent_continuations_include_issue_urls_and_stop_at_boundaries() {
        for extension in ["ts", "cpp", "rs", "java"] {
            let findings = extract(
                extension,
                "\t\t// TODO: Leave this fallback undefined so bypassed requests without a device_id\r\n\t\t// fail validation instead of provisioning or reusing unknown_device.\r\n\t\t// https://github.com/UXStream/uxstream/issues/2370\r\n\t\t//\r\n\t\t// unrelated comment\r\n\t\t// FIXME: next task\r\n\t\t// continued too\r\ncode();\r\n\t\t// must not attach after code\r\n",
            );
            assert_eq!(findings.len(), 2, "{extension}");
            assert_eq!(findings[0].end_line, 3, "{extension}");
            assert!(findings[0].message.contains("\nfail validation"));
            assert!(!findings[0].message.contains("unrelated"));
            assert_eq!(findings[0].references[0].number, 2370);
            assert_eq!(findings[1].message, "next task\ncontinued too");
            assert_eq!(findings[1].end_line, 7);
        }
        for (extension, prefix) in [("py", "#"), ("sh", "#"), ("sql", "--")] {
            let findings = extract(
                extension,
                &format!(
                    "{prefix} TODO: first\n{prefix} second\n{prefix} FIXME: third\n{prefix} fourth\n"
                ),
            );
            assert_eq!(findings.len(), 2, "{extension}");
            assert_eq!(findings[0].message, "first\nsecond");
            assert_eq!(findings[1].message, "third\nfourth");
        }
        let findings = extract("ts", "  // TODO: first\n// outdented, unrelated\n");
        assert_eq!(findings[0].message, "first");
    }

    #[test]
    fn skips_binary_encoding_and_size_limits_and_can_disable_fallback() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("binary.rs"), b"// TODO: x\0").unwrap();
        std::fs::write(temp.path().join("encoding.rs"), b"// TODO: \xff").unwrap();
        std::fs::write(temp.path().join("big.rs"), "// TODO: too big".repeat(20)).unwrap();
        std::fs::write(temp.path().join("small.unknown"), "TODO: heuristic").unwrap();
        let config = ScanConfig {
            max_file_size: 100,
            fallback: false,
            ..Default::default()
        };
        let scan = scan(temp.path(), &config, None).unwrap();
        assert!(scan.findings.is_empty());
        assert_eq!(scan.stats.skipped_binary, 1);
        assert_eq!(scan.stats.skipped_encoding, 1);
        assert_eq!(scan.stats.skipped_oversized, 1);
    }

    #[test]
    fn traversal_excludes_dependencies_nested_repos_and_output() {
        let temp = tempfile::tempdir().unwrap();
        for dir in ["node_modules", "nested/.git", ".github"] {
            std::fs::create_dir_all(temp.path().join(dir)).unwrap();
        }
        for file in [
            "a.rs",
            "ignored.rs",
            "node_modules/a.rs",
            "nested/a.rs",
            ".github/a.rs",
            "output.rs",
        ] {
            std::fs::write(temp.path().join(file), "// TODO: test").unwrap();
        }
        std::fs::write(temp.path().join(".gitignore"), "ignored.rs\n").unwrap();
        let scan = scan(
            temp.path(),
            &ScanConfig::default(),
            Some(&temp.path().join("output.rs")),
        )
        .unwrap();
        assert_eq!(
            scan.findings
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            [".github/a.rs", "a.rs"]
        );
    }
}
