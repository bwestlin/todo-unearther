use std::{fs, io::Write, time::Instant};
use todo_unearther::{
    app,
    cli::{ComponentLayout, Detail, Format, GroupBy},
    components::Discovery,
    config::Config,
    model::*,
    output, scanner,
};

fn main() -> anyhow::Result<()> {
    let sizes: Vec<usize> = std::env::var("TODO_BENCH_FILES").ok().map_or_else(
        || vec![10_000, 100_000],
        |v| {
            v.split(',')
                .map(str::parse)
                .collect::<Result<_, _>>()
                .expect("TODO_BENCH_FILES must be comma-separated counts")
        },
    );
    let runs = std::env::var("TODO_BENCH_RUNS").ok().map_or(3, |s| {
        s.parse::<usize>()
            .expect("TODO_BENCH_RUNS must be a positive integer")
    });
    anyhow::ensure!(runs > 0, "TODO_BENCH_RUNS must be positive");
    println!(
        "files,scenario,pass,group,layout,format,scan_ms,render_ms,total_ms,candidates,findings,bytes,manifests,additional_manifest_reads,directory_lookups"
    );
    for size in sizes {
        for scenario in ["sparse", "dense", "dependencies", "mixed"] {
            let temp = tempfile::tempdir()?;
            for index in 0..size {
                let directory = if scenario == "mixed" {
                    let package = temp
                        .path()
                        .join(format!("packages/project-{}", index / 1000));
                    if index % 1000 == 0 {
                        package_manifest(&package, index / 1000)?;
                    }
                    package.join(format!("src/group-{}", index / 100))
                } else if scenario == "dependencies" && index % 2 == 0 {
                    temp.path()
                        .join("node_modules")
                        .join(format!("package-{}", index / 1000))
                } else {
                    temp.path().join(format!("src-{}", index / 1000))
                };
                fs::create_dir_all(&directory)?;
                let mut file = fs::File::create(directory.join(format!("module-{index}.rs")))?;
                let marked = if scenario == "dense" || scenario == "mixed" {
                    index % 4 == 0
                } else {
                    index % 1000 == 1
                };
                file.write_all(if marked {
                    b"// TODO:(demo) implement validation #12\nfn main() {}\n"
                } else {
                    b"pub fn add(a: u64, b: u64) -> u64 { a + b }\n"
                })?;
            }
            for pass in 0..runs {
                let groupings = if scenario == "mixed" {
                    vec![
                        (GroupBy::File, ComponentLayout::Flat),
                        (GroupBy::Component, ComponentLayout::Flat),
                        (GroupBy::Component, ComponentLayout::Tree),
                    ]
                } else {
                    vec![(GroupBy::File, ComponentLayout::Flat)]
                };
                for (grouping, layout) in groupings {
                    for format in [
                        Format::Console,
                        Format::Markdown,
                        Format::Json,
                        Format::Html,
                    ] {
                        let mut config = Config::default();
                        config.report.group_by = grouping;
                        config.report.component_layout = layout;
                        let start = Instant::now();
                        let discovery = (grouping == GroupBy::Component).then(|| {
                            Discovery::new(
                                temp.path(),
                                &config.components,
                                config.scan.max_file_size,
                            )
                        });
                        let scanned = scanner::scan_with_components(
                            temp.path(),
                            &config.scan,
                            None,
                            discovery.as_ref(),
                        )?;
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
                        if let Some(discovery) = &discovery {
                            anyhow::ensure!(
                                discovery.assign(&mut report),
                                "component fixture must be resolvable"
                            );
                        }
                        report.metadata.source_fingerprint =
                            app::fingerprint_with_components(&report, &config, format)?;
                        let scan_ms = start.elapsed().as_secs_f64() * 1000.0;
                        let render = Instant::now();
                        let text = output::render(
                            &report,
                            &output::Options {
                                format,
                                detail: Detail::Full,
                                group_by: grouping,
                                component_group_by: Default::default(),
                                component_layout: layout,
                                width: 100,
                                line_spacing: 1,
                                colors: false,
                                links: false,
                                source_prefix: String::new(),
                                absolute_source_links: false,
                            },
                        )?;
                        std::hint::black_box(&text);
                        let render_ms = render.elapsed().as_secs_f64() * 1000.0;
                        let stats = discovery.as_ref().map(|d| d.stats()).unwrap_or_default();
                        println!(
                            "{size},{scenario},{pass},{grouping:?},{layout:?},{format:?},{scan_ms:.3},{render_ms:.3},{:.3},{},{},{},{},{},{}",
                            start.elapsed().as_secs_f64() * 1000.0,
                            report.stats.candidate_files,
                            report.findings.len(),
                            report.stats.bytes_read,
                            stats.manifests_read,
                            stats.additional_reads,
                            stats.directories_read
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn package_manifest(directory: &std::path::Path, index: usize) -> anyhow::Result<()> {
    fs::create_dir_all(directory)?;
    let name = format!("project-{index}");
    let (file, contents) = match index % 7 {
        0 => (
            "Cargo.toml".to_owned(),
            format!("[package]\nname='{name}'\n"),
        ),
        1 => (
            "package.json".to_owned(),
            format!(r#"{{"name":"@benchmark/{name}"}}"#),
        ),
        2 => (
            "pyproject.toml".to_owned(),
            format!("[project]\nname='{name}'\n"),
        ),
        3 => ("go.mod".to_owned(), format!("module example.test/{name}\n")),
        4 => (
            "pom.xml".to_owned(),
            format!("<project><artifactId>{name}</artifactId></project>"),
        ),
        5 => (
            "settings.gradle.kts".to_owned(),
            format!("rootProject.name = \"{name}\"\n"),
        ),
        _ => (
            format!("{name}.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk\" />".to_owned(),
        ),
    };
    fs::write(directory.join(file), contents)?;
    Ok(())
}
