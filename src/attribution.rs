use crate::{cli::AttributionMode, config::GitConfig, model::*, process};
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use std::{
    collections::BTreeMap,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
struct BlamedLine {
    author: GitAuthor,
    original_line: usize,
    original_path: String,
    boundary: bool,
}

pub fn enrich(
    report: &mut ScanReport,
    root: &Path,
    config: &GitConfig,
    markers: &[String],
) -> Result<bool> {
    if config.attribution == AttributionMode::Off || report.findings.is_empty() {
        return Ok(true);
    }
    let shallow = git(
        root,
        &["rev-parse", "--is-shallow-repository"],
        None,
        Duration::from_secs(2),
    )
    .ok()
    .map(|out| String::from_utf8_lossy(&out.stdout).trim() == "true");
    if shallow.is_none() {
        for finding in &mut report.findings {
            finding.attribution = Some(Attribution {
                origin_status: "unavailable".into(),
                note: Some("Git or repository history is unavailable".into()),
                ..Default::default()
            });
        }
        report.warn(
            "git",
            "Git or repository history is unavailable; attribution skipped",
        );
        return Ok(false);
    }
    let mut groups = BTreeMap::<String, Vec<(usize, usize)>>::new();
    for (index, finding) in report.findings.iter().enumerate() {
        groups
            .entry(finding.path.clone())
            .or_default()
            .push((index, finding.line));
    }
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build()?;
    let results: Vec<_> = pool.install(|| {
        groups
            .par_iter()
            .map(|(path, findings)| {
                (
                    path.clone(),
                    attribute_file(
                        root,
                        path,
                        findings,
                        config,
                        markers,
                        shallow.unwrap_or(true),
                    ),
                )
            })
            .collect()
    });
    let mut complete = true;
    for (path, result) in results {
        match result {
            Ok(attributions) => {
                let mut notes = Vec::new();
                for (index, attribution) in attributions {
                    if attribution.note.is_some()
                        && matches!(
                            attribution.origin_status.as_str(),
                            "incomplete" | "unavailable" | "uncommitted"
                        )
                    {
                        complete = false;
                        notes.push(format!(
                            "{}: {}",
                            report.findings[index].line,
                            attribution.note.as_deref().unwrap()
                        ));
                    }
                    report.findings[index].attribution = Some(attribution);
                }
                if !notes.is_empty() {
                    report.warn("git", format!("{path}: {}", notes.join("; ")));
                }
            }
            Err(error) => {
                complete = false;
                for &(index, _) in &groups[&path] {
                    report.findings[index].attribution = Some(Attribution {
                        origin_status: "unavailable".into(),
                        note: Some(error.to_string()),
                        ..Default::default()
                    });
                }
                report.warn("git", format!("{path}: {error:#}"));
            }
        }
    }
    Ok(complete)
}

fn attribute_file(
    root: &Path,
    path: &str,
    findings: &[(usize, usize)],
    config: &GitConfig,
    markers: &[String],
    shallow: bool,
) -> Result<Vec<(usize, Attribution)>> {
    let deadline = Instant::now() + Duration::from_secs(config.timeout_seconds);
    let mut args = vec![
        "blame".to_owned(),
        "--line-porcelain".into(),
        "--root".into(),
    ];
    if config.ignore_whitespace {
        args.push("-w".into());
    }
    if config.detect_moves {
        args.push("-M".into());
    }
    if config.detect_copies {
        args.push("-C".into());
    }
    // Read the worktree version so dirty line numbers cannot be attributed to the wrong HEAD line.
    args.extend(["--contents".into(), "-".into(), "--".into(), path.into()]);
    let contents = std::fs::read(root.join(path)).context("cannot read worktree for blame")?;
    let output = git(
        root,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        Some(contents),
        remaining(deadline)?,
    )?;
    let blamed = parse_blame(&String::from_utf8_lossy(&output.stdout));
    let mut cache = BTreeMap::new();
    let mut results = Vec::new();
    for &(index, line) in findings {
        let Some(blamed) = blamed.get(&line) else {
            results.push((
                index,
                Attribution {
                    origin_status: "unavailable".into(),
                    note: Some("marker line was not present in blame output".into()),
                    ..Default::default()
                },
            ));
            continue;
        };
        if blamed.author.commit.chars().all(|c| c == '0') {
            results.push((
                index,
                Attribution {
                    origin_status: "uncommitted".into(),
                    note: Some("marker line has uncommitted changes".into()),
                    ..Default::default()
                },
            ));
            continue;
        }
        let mut attribution = Attribution {
            blame: Some(blamed.author.clone()),
            origin_status: "not_requested".into(),
            ..Default::default()
        };
        if config.attribution == AttributionMode::History {
            let key = (
                blamed.author.commit.clone(),
                blamed.original_path.clone(),
                blamed.original_line,
            );
            let history = cache
                .entry(key)
                .or_insert_with(|| history(root, blamed, config, markers, deadline, shallow));
            match history {
                Ok((author, status, note)) => {
                    attribution.origin = author.clone();
                    attribution.origin_status.clone_from(status);
                    attribution.note.clone_from(note);
                }
                Err(error) => {
                    attribution.origin_status = "incomplete".into();
                    attribution.note = Some(error.to_string());
                }
            }
        } else if shallow && blamed.boundary {
            attribution.origin_status = "incomplete".into();
            attribution.note = Some("blame reached a shallow history boundary".into());
        }
        results.push((index, attribution));
    }
    Ok(results)
}

type HistoryResult = (Option<GitAuthor>, String, Option<String>);

fn history(
    root: &Path,
    blamed: &BlamedLine,
    config: &GitConfig,
    markers: &[String],
    deadline: Instant,
    shallow: bool,
) -> Result<HistoryResult> {
    let range = format!(
        "{},{}:{}",
        blamed.original_line, blamed.original_line, blamed.original_path
    );
    let limit = format!("--max-count={}", config.history_limit.saturating_add(1));
    let output = git(
        root,
        &[
            "log",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--format=%x1e%H%x00%an%x00%ae%x00%at%x1f",
            &limit,
            "-L",
            &range,
            &blamed.author.commit,
        ],
        None,
        remaining(deadline)?,
    )?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(trace_history(&text, markers, config.history_limit, shallow))
}

fn trace_history(text: &str, markers: &[String], limit: usize, shallow: bool) -> HistoryResult {
    let records: Vec<_> = text
        .split('\u{1e}')
        .filter(|s| !s.trim().is_empty())
        .collect();
    let truncated = records.len() > limit;
    let mut earliest = None;
    for record in records.iter().take(limit) {
        let Some((header, patch)) = record.split_once('\u{1f}') else {
            continue;
        };
        let fields: Vec<_> = header.split('\0').collect();
        if fields.len() != 4 {
            continue;
        }
        let author = GitAuthor {
            commit: fields[0].trim().into(),
            name: fields[1].into(),
            email: fields[2].into(),
            author_time: fields[3].parse().unwrap_or_default(),
        };
        let mut added = false;
        let mut removed = false;
        for line in patch.lines() {
            if !line.starts_with("+++")
                && let Some(text) = line.strip_prefix('+')
            {
                added |= contains_marker(text, markers);
            }
            if !line.starts_with("---")
                && let Some(text) = line.strip_prefix('-')
            {
                removed |= contains_marker(text, markers);
            }
        }
        if added {
            earliest = Some(author);
        }
        if added && !removed && !shallow {
            return (earliest, "inferred".into(), Some("inferred introduction along Git's traced line history; copies or ambiguous edits may have an earlier origin".into()));
        }
        // Do not walk through older, unrelated content after the marker disappears.
        if !added && removed {
            break;
        }
    }
    let note = if truncated {
        "history record limit reached"
    } else if shallow {
        "shallow history: earliest observed occurrence is not a proven introduction"
    } else {
        "line history did not establish an unambiguous introduction"
    };
    (earliest, "incomplete".into(), Some(note.into()))
}

fn contains_marker(text: &str, markers: &[String]) -> bool {
    let text = text
        .trim()
        .trim_start_matches(['/', '*', '#', '!', '@', ' ']);
    markers.iter().any(|marker| {
        text.get(..marker.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(marker))
            && text
                .as_bytes()
                .get(marker.len())
                .is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_')
    })
}

fn remaining(deadline: Instant) -> Result<Duration> {
    let duration = deadline.saturating_duration_since(Instant::now());
    if duration.is_zero() {
        bail!("per-file Git time budget exhausted");
    }
    Ok(duration)
}

fn git(
    root: &Path,
    args: &[&str],
    input: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<std::process::Output> {
    let output = process::capture(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["-c", "core.quotePath=false"])
            .args(args),
        input,
        timeout,
    )?;
    if !output.status.success() {
        // Git errors contain paths, but never credentials or command-line tokens.
        bail!(
            "Git failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}

fn parse_blame(text: &str) -> BTreeMap<usize, BlamedLine> {
    let mut lines = BTreeMap::new();
    let mut current: Option<(usize, BlamedLine)> = None;
    for line in text.lines() {
        if let Some(content) = line.strip_prefix('\t') {
            let _ = content;
            if let Some((row, blame)) = current.take() {
                lines.insert(row, blame);
            }
            continue;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() >= 3
            && fields[0].len() >= 40
            && fields[0].chars().all(|c| c.is_ascii_hexdigit())
        {
            if let (Ok(original_line), Ok(row)) = (fields[1].parse(), fields[2].parse()) {
                current = Some((
                    row,
                    BlamedLine {
                        author: GitAuthor {
                            commit: fields[0].into(),
                            name: String::new(),
                            email: String::new(),
                            author_time: 0,
                        },
                        original_line,
                        original_path: String::new(),
                        boundary: false,
                    },
                ));
            }
            continue;
        }
        if let Some((_, blame)) = &mut current {
            if let Some(v) = line.strip_prefix("author ") {
                blame.author.name = v.into();
            }
            if let Some(v) = line.strip_prefix("author-mail ") {
                blame.author.email = v.trim_matches(['<', '>']).into();
            }
            if let Some(v) = line.strip_prefix("author-time ") {
                blame.author.author_time = v.parse().unwrap_or_default();
            }
            if let Some(v) = line.strip_prefix("filename ") {
                blame.original_path = unquote_path(v);
            }
            if line == "boundary" {
                blame.boundary = true;
            }
        }
    }
    lines
}

fn unquote_path(path: &str) -> String {
    // Git uses C quoting for control characters; JSON handles the common escapes.
    if path.starts_with('"') {
        serde_json::from_str(path).unwrap_or_else(|_| path.trim_matches('"').into())
    } else {
        path.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edited_marker_is_traced_back_to_its_addition() {
        let text = "\u{1e}bbb\0Bob\0bob@example.com\x00200\u{1f}\n-TODO: old\n+TODO: edited\n\u{1e}aaa\0Alice\0alice@example.com\x00100\u{1f}\n+TODO: old\n";
        let (origin, status, _) = trace_history(text, &["TODO".into()], 200, false);
        assert_eq!(origin.unwrap().name, "Alice");
        assert_eq!(status, "inferred");
        assert_eq!(
            trace_history(text, &["TODO".into()], 1, false).1,
            "incomplete"
        );
        assert_eq!(
            trace_history(text, &["TODO".into()], 200, true).1,
            "incomplete"
        );
    }
}
