use crate::{
    attribution,
    cli::{Cli, Command, Compare, ConfigCommand, Format, GroupBy, Links},
    components::Discovery,
    config::{self, Config},
    github,
    model::*,
    output, policy, preview, scanner,
};
use anyhow::{Context, Result, bail, ensure};
use clap::CommandFactory;
use sha2::{Digest, Sha256};
use std::{
    io::{IsTerminal, Write},
    path::{Component, Path, PathBuf},
    time::Instant,
};

pub fn run(cli: Cli) -> Result<u8> {
    run_with_opener(cli, |path| open::that_detached(path).map_err(Into::into))
}

/// Dependency injection keeps automated tests from launching a real desktop application.
pub fn run_with_opener(cli: Cli, opener: impl FnOnce(&Path) -> Result<()>) -> Result<u8> {
    let started = Instant::now();
    let cwd = std::env::current_dir()?;
    if let Some(Command::Completions { shell }) = &cli.command {
        clap_complete::generate(
            *shell,
            &mut Cli::command(),
            "todo-unearther",
            &mut std::io::stdout(),
        );
        return Ok(0);
    }
    let root = if let Some(path) = &cli.path {
        path.canonicalize()
            .with_context(|| format!("cannot open scan root {}", path.display()))?
    } else {
        config::discover_root(&cwd)
    };
    ensure!(root.is_dir(), "scan path must be a directory");
    if let Some(Command::Config {
        command: ConfigCommand::Init { force },
    }) = &cli.command
    {
        let destination = cli
            .config
            .clone()
            .unwrap_or_else(|| root.join(config::CONFIG_NAME));
        write_atomic(&destination, config::DOCUMENTED_CONFIG, *force, false)?;
        eprintln!("Created {}", destination.display());
        return Ok(0);
    }
    let (mut config, _) = Config::load(&root, cli.config.as_deref())?;
    config.apply(&cli)?;
    let destination = if cli.open {
        None
    } else {
        cli.output
            .clone()
            .or_else(|| config.report.output.clone())
            .or_else(|| cli.check.then(|| root.join("TODO.md")))
    };
    if cli.check {
        ensure!(
            destination.as_deref() != Some(Path::new("-")),
            "--check requires a report file"
        );
    }
    let terminal = destination.as_deref().is_none_or(|p| p == Path::new("-"))
        && std::io::stdout().is_terminal();
    let format = if cli.open {
        Format::Html
    } else {
        config
            .report
            .format
            .unwrap_or_else(|| output::infer_format(destination.as_deref(), terminal))
    };
    if cli.check && config.check.compare == Compare::Source {
        ensure!(
            matches!(format, Format::Markdown | Format::Json | Format::Html),
            "source freshness requires Markdown, JSON, or HTML; use --compare full for text"
        );
    }
    let output_path = destination.as_deref().filter(|p| *p != Path::new("-"));
    let discovery = (config.report.group_by == GroupBy::Component)
        .then(|| Discovery::new(&root, &config.components, config.scan.max_file_size));
    let scanned =
        scanner::scan_with_components(&root, &config.scan, output_path, discovery.as_ref())?;
    let scan_duration = started.elapsed();
    let mut report = ScanReport {
        metadata: Metadata {
            generator: GENERATOR.into(),
            schema_version: SCHEMA_VERSION,
            source_fingerprint: String::new(),
        },
        counts: scanner::counts(&scanned.findings),
        findings: scanned.findings,
        stats: scanned.stats,
        diagnostics: scanned.diagnostics,
        violations: vec![],
        components: vec![],
    };
    let component_started = Instant::now();
    let mut complete = discovery
        .as_ref()
        .is_none_or(|discovery| discovery.assign(&mut report));
    let component_duration = component_started.elapsed();
    report.metadata.source_fingerprint = fingerprint_with_components(&report, &config, format)?;
    let source_only = cli.check && config.check.compare == Compare::Source;
    let needs_issues = config.policy.needs_github();
    let enrich_github = config.github.enabled && (!source_only || needs_issues);
    let enrichment_started = Instant::now();
    // Source-only checks do not need remotes or history. Literal references are fingerprinted.
    github::resolve_references(&mut report, &config.github, &root, enrich_github);
    if enrich_github {
        match github::enrich(&mut report, &config.github, config.policy.fail_on_closed) {
            Ok(done) => complete &= done,
            Err(error) => {
                complete = false;
                report.warn("github", error.to_string());
            }
        }
    }
    let github_duration = enrichment_started.elapsed();
    let git_started = Instant::now();
    if !source_only {
        match attribution::enrich(&mut report, &root, &config.git, &config.scan.markers) {
            Ok(done) => complete &= done,
            Err(error) => {
                complete = false;
                report.warn("git", error.to_string());
            }
        }
    }
    let git_duration = git_started.elapsed();
    complete &= policy::evaluate(&mut report, &config.policy, enrich_github);
    report
        .diagnostics
        .sort_by(|a, b| (&a.stage, &a.message).cmp(&(&b.stage, &b.message)));
    for diagnostic in &report.diagnostics {
        eprintln!(
            "warning ({}): {}",
            diagnostic.stage,
            output::clean(&diagnostic.message)
        );
    }
    for violation in &report.violations {
        eprintln!(
            "policy ({}): {}",
            violation.policy,
            output::clean(&violation.message)
        );
    }
    if config.check.strict && !complete {
        bail!("requested enrichment or policy evaluation did not complete (--strict)");
    }
    let source_prefix = if cli.open || format == Format::Console {
        root.to_string_lossy().into_owned()
    } else {
        source_prefix(&root, output_path, &cwd)?
    };
    let ansi = terminal && std::env::var("TERM").is_ok_and(|term| term != "dumb");
    // An unset TERM is common on Windows and is still compatible with modern terminals.
    let ansi = ansi || (terminal && std::env::var_os("TERM").is_none());
    let render_started = Instant::now();
    let rendered = output::render(
        &report,
        &output::Options {
            format,
            detail: config.report.detail,
            group_by: config.report.group_by,
            component_group_by: config.report.component_group_by,
            component_layout: config.report.component_layout,
            line_spacing: config.report.line_spacing,
            width: config.report.width.map(usize::from).unwrap_or_else(|| {
                if format == Format::Console && terminal {
                    terminal_size::terminal_size()
                        .map(|(terminal_size::Width(w), _)| usize::from(w))
                        .or_else(|| std::env::var("COLUMNS").ok()?.parse().ok())
                        .filter(|width| *width >= 20)
                        .unwrap_or(100)
                } else {
                    100
                }
            }),
            colors: ansi && std::env::var_os("NO_COLOR").is_none(),
            links: ansi && config.report.links != Links::Disabled,
            source_prefix,
            absolute_source_links: cli.open,
        },
    )?;
    let render_duration = render_started.elapsed();
    let mut code = u8::from(!report.violations.is_empty());
    if cli.check {
        let target = destination.as_ref().unwrap();
        match std::fs::read_to_string(target) {
            Ok(existing) => {
                if config.check.compare == Compare::Source {
                    let metadata = output::metadata(&existing)?;
                    if metadata.source_fingerprint != report.metadata.source_fingerprint {
                        eprintln!(
                            "stale report: {} (source findings or report settings changed)",
                            target.display()
                        );
                        code = 1;
                    } else {
                        eprintln!("Source findings match {}", target.display());
                    }
                } else if complete {
                    if existing != rendered {
                        eprintln!("stale report: {} (full comparison)", target.display());
                        code = 1;
                    } else {
                        eprintln!("Report matches {}", target.display());
                    }
                } else {
                    eprintln!(
                        "warning: full report comparison skipped because enrichment is incomplete"
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("stale report: {} does not exist", target.display());
                code = 1;
            }
            Err(error) => return Err(error).context("cannot read report for comparison"),
        }
    } else if cli.open {
        let path = preview::show(&rendered, opener)?;
        eprintln!("Opened HTML preview at {}", path.display());
    } else if let Some(path) = output_path {
        write_atomic(path, &rendered, cli.force, true)?;
        eprintln!(
            "Wrote {} ({} findings)",
            path.display(),
            report.findings.len()
        );
    } else {
        if let Err(error) = std::io::stdout().lock().write_all(rendered.as_bytes())
            && error.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(error.into());
        }
    }
    if cli.timings {
        if let Some(discovery) = &discovery {
            let stats = discovery.stats();
            eprintln!(
                "components: {:.3}s assignment; {} manifests, {} additional reads, {} directory lookups, {} cache hits",
                component_duration.as_secs_f64(),
                stats.manifests_read,
                stats.additional_reads,
                stats.directories_read,
                stats.cache_hits
            );
        }
        eprintln!(
            "timings: scan={:.3}s github={:.3}s git={:.3}s render={:.3}s total={:.3}s; {} files read, {} candidates, {} bytes",
            scan_duration.as_secs_f64(),
            github_duration.as_secs_f64(),
            git_duration.as_secs_f64(),
            render_duration.as_secs_f64(),
            started.elapsed().as_secs_f64(),
            report.stats.files_read,
            report.stats.candidate_files,
            report.stats.bytes_read
        );
    }
    Ok(code)
}

pub fn fingerprint(findings: &[Finding], config: &Config, format: Format) -> Result<String> {
    let findings: Vec<_> = findings.iter().map(|finding| serde_json::json!({
        "path": finding.path, "line": finding.line, "end_line": finding.end_line,
        "marker": finding.marker, "annotation": finding.annotation, "message": finding.message,
        "detection": finding.detection,
        "references": finding.references.iter().map(|r| &r.raw).collect::<Vec<_>>()
    })).collect();
    let config = serde_json::json!({
        "markers": config.scan.markers, "include": config.scan.include, "exclude": config.scan.exclude,
        "default_excludes": config.scan.default_excludes, "respect_ignore": config.scan.respect_ignore,
        "fallback": config.scan.fallback, "max_file_size": config.scan.max_file_size,
        "format": format, "detail": config.report.detail, "group_by": config.report.group_by,
        "policy": config.policy
    });
    let bytes = serde_json::to_vec(&(SCHEMA_VERSION, findings, config))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn fingerprint_with_components(
    report: &ScanReport,
    config: &Config,
    format: Format,
) -> Result<String> {
    let legacy = fingerprint(&report.findings, config, format)?;
    if config.report.group_by != GroupBy::Component {
        return Ok(legacy);
    }
    let ownership: Vec<_> = report.findings.iter().map(|f| &f.component_id).collect();
    let bytes = serde_json::to_vec(&(
        "components-v1",
        legacy,
        ownership,
        &report.components,
        &config.components,
        config.report.component_group_by,
        config.report.component_layout,
    ))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn write_atomic(path: &Path, text: &str, force: bool, allow_owned: bool) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                !metadata.file_type().is_symlink(),
                "refusing to replace a symlink: {}",
                path.display()
            );
            ensure!(
                metadata.is_file(),
                "output is not a regular file: {}",
                path.display()
            );
            if !force {
                let owned =
                    allow_owned && std::fs::read_to_string(path).is_ok_and(|s| output::owns(&s));
                ensure!(
                    owned,
                    "{} already exists and is not a generated report; use --force to replace it",
                    path.display()
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    ensure!(
        parent.is_dir(),
        "output directory does not exist: {}",
        parent.display()
    );
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(text.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|e| e.error)
        .context("cannot atomically replace output")?;
    Ok(())
}

fn source_prefix(root: &Path, output: Option<&Path>, cwd: &Path) -> Result<String> {
    let parent = output
        .and_then(Path::parent)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(cwd);
    let parent = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        cwd.join(parent)
    };
    // Resolve both sides consistently: Windows canonical paths have a verbatim
    // drive prefix, and macOS temporary directories may pass through /var.
    let root = root
        .canonicalize()
        .context("cannot resolve source directory")?;
    let parent = parent
        .canonicalize()
        .context("cannot resolve report directory")?;
    Ok(relative_path(&root, &parent)
        .to_string_lossy()
        .replace('\\', "/"))
}

fn relative_path(path: &Path, base: &Path) -> PathBuf {
    let path: Vec<_> = path
        .components()
        .filter(|c| *c != Component::CurDir)
        .collect();
    let base: Vec<_> = base
        .components()
        .filter(|c| *c != Component::CurDir)
        .collect();
    if path.first() != base.first() {
        return path.iter().collect();
    }
    let common = path.iter().zip(&base).take_while(|(a, b)| a == b).count();
    std::iter::repeat_n(Component::ParentDir, base.len() - common)
        .chain(path[common..].iter().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_output_protects_handwritten_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("TODO.md");
        std::fs::write(&path, "handwritten").unwrap();
        assert!(write_atomic(&path, "generated", false, true).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "handwritten");
        write_atomic(&path, "generated", true, true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "generated");
    }

    #[test]
    fn config_template_parses_and_paths_are_relative() {
        let _: Config = toml::from_str(config::DOCUMENTED_CONFIG).unwrap();
        assert_eq!(
            relative_path(Path::new("/r/src"), Path::new("/r/docs")),
            PathBuf::from("../src")
        );
    }
}
