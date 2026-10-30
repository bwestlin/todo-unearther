//! Deterministic synthetic scenes, rendered using the same functions as the CLI.
use std::{fmt::Write as _, fs, path::PathBuf};
use todo_unearther::{
    cli::{ComponentLayout, Detail, Format, GroupBy, InnerGroupBy},
    model::*,
    output, references, scanner,
};

fn main() -> anyhow::Result<()> {
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("usage: demo OUTPUT_DIRECTORY"),
    );
    let font = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "DejaVu Sans Mono".into());
    fs::create_dir_all(&directory)?;
    let mut report = fixture();
    let scenes = [
        (
            "$ todo-unearther --github",
            Format::Console,
            Detail::Full,
            GroupBy::File,
        ),
        (
            "$ todo-unearther --group-by file --detail summary",
            Format::Console,
            Detail::Summary,
            GroupBy::File,
        ),
        (
            "$ todo-unearther --format markdown --group-by marker",
            Format::Markdown,
            Detail::Full,
            GroupBy::Marker,
        ),
        (
            "$ todo-unearther --format json",
            Format::Json,
            Detail::Full,
            GroupBy::File,
        ),
        (
            "$ todo-unearther --github --attribution history",
            Format::Console,
            Detail::Full,
            GroupBy::File,
        ),
        (
            "$ todo-unearther --group-by component --component-group-by file --detail summary",
            Format::Console,
            Detail::Summary,
            GroupBy::Component,
        ),
        (
            "$ todo-unearther --group-by component --component-layout tree --detail summary --open",
            Format::Html,
            Detail::Summary,
            GroupBy::Component,
        ),
        (
            "$ todo-unearther --group-by component --component-layout tree --width 64",
            Format::Console,
            Detail::Full,
            GroupBy::Component,
        ),
    ];
    for (index, (command, format, detail, group_by)) in scenes.into_iter().enumerate() {
        if index == 5 {
            add_components(&mut report);
        }
        if index == 4 {
            report.findings[0].attribution = Some(Attribution {
                blame: Some(author("Morgan", "b72e9ac142")),
                origin: Some(author("Alex", "a41f8d6332")),
                origin_status: "inferred".into(),
                note: None,
            });
        }
        if index == 7 {
            report.findings.truncate(1);
            report.findings[0].attribution = None;
            report.findings[0].message = "Validate request payloads before processing\nand reject malformed identifiers before opening a session.".into();
            report.counts = scanner::counts(&report.findings);
        }
        let options = output::Options {
            format,
            detail,
            group_by,
            component_group_by: InnerGroupBy::File,
            component_layout: if index >= 6 {
                ComponentLayout::Tree
            } else {
                ComponentLayout::Flat
            },
            width: if index == 7 { 64 } else { 104 },
            line_spacing: 1,
            colors: true,
            links: false,
            source_prefix: String::new(),
            absolute_source_links: false,
        };
        let rendered = output::render(&report, &options)?;
        let display = if format == Format::Html {
            fs::write(directory.join("preview.html"), &rendered)?;
            let plain = output::render(
                &report,
                &output::Options {
                    format: Format::Console,
                    ..options
                },
            )?;
            format!("HTML preview · file:///tmp/todo-unearther-demo.html\n\n{plain}")
        } else {
            rendered.clone()
        };
        let mut preview = Vec::new();
        for line in display.lines().filter(|line| {
            !line.starts_with("<!-- todo-unearther:") && !line.starts_with("<!-- markdownlint-")
        }) {
            // Simulate terminal wrapping so long Markdown links remain inside the frame.
            preview.extend(
                textwrap::wrap(line, 108)
                    .into_iter()
                    .map(|line| line.into_owned()),
            );
        }
        let displayed = preview.into_iter().take(24).collect::<Vec<_>>().join("\n");
        let frame = format!("{command}\n\n{displayed}");
        fs::write(directory.join(format!("frame-{index}.txt")), &frame)?;
        fs::write(
            directory.join(format!("frame-{index}.svg")),
            terminal_svg(&frame, &font),
        )?;
        fs::write(directory.join(format!("report-{index}.txt")), rendered)?;
    }
    Ok(())
}

// Render the production console's SGR colors into a deterministic terminal
// frame. No terminal recording, browser, or additional graphics library is needed.
fn terminal_svg(content: &str, font: &str) -> String {
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1360" height="860" viewBox="0 0 1360 860">
<rect x="1" y="1" width="1358" height="858" rx="14" fill="#0d1117" stroke="#30363d" stroke-width="2"/>
<path d="M15 2H1345Q1357 2 1357 15V48H2V15Q2 2 15 2" fill="#161b22"/>
<circle cx="25" cy="24" r="6" fill="#ff5f56"/><circle cx="46" cy="24" r="6" fill="#ffbd2e"/><circle cx="67" cy="24" r="6" fill="#27c93f"/>
<g font-family="{}"><text x="490" y="30" font-size="17" fill="#8b949e">todo-unearther — repository report</text>
"##,
        xml(font)
    );
    let mut style = TerminalStyle::default();
    for (row, line) in content.lines().enumerate() {
        let mut chars = line.chars();
        let mut span = String::new();
        let mut column = 0;
        while let Some(character) = chars.next() {
            if character == '\x1b' && chars.next() == Some('[') {
                draw_span(&mut svg, &mut span, row, &mut column, &style);
                let codes: String = chars.by_ref().take_while(|c| *c != 'm').collect();
                for code in codes.split(';').filter_map(|code| code.parse::<u8>().ok()) {
                    match code {
                        0 => style = TerminalStyle::default(),
                        1 => style.bold = true,
                        30..=37 => style.foreground = ANSI_COLORS[usize::from(code - 30)],
                        90..=97 => style.foreground = ANSI_COLORS[usize::from(code - 90 + 8)],
                        40..=47 => style.background = Some(ANSI_COLORS[usize::from(code - 40)]),
                        100..=107 => {
                            style.background = Some(ANSI_COLORS[usize::from(code - 100 + 8)])
                        }
                        _ => {}
                    }
                }
            } else {
                span.push(character);
            }
        }
        draw_span(&mut svg, &mut span, row, &mut column, &style);
    }
    svg.push_str(r##"<text x="32" y="828" font-size="13" fill="#8b949e">Synthetic examples · no real repositories, history, credentials, or API calls</text></g></svg>"##);
    svg
}

struct TerminalStyle {
    foreground: &'static str,
    background: Option<&'static str>,
    bold: bool,
}

impl Default for TerminalStyle {
    fn default() -> Self {
        Self {
            foreground: "#e6edf3",
            background: None,
            bold: false,
        }
    }
}

const ANSI_COLORS: [&str; 16] = [
    "#0c0c0c", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
    "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff",
];

fn draw_span(
    svg: &mut String,
    span: &mut String,
    row: usize,
    column: &mut usize,
    style: &TerminalStyle,
) {
    if span.is_empty() {
        return;
    }
    let columns = textwrap::core::display_width(span);
    let x = 32.0 + *column as f64 * 12.0;
    let y = 90 + row * 27;
    let width = columns as f64 * 12.0;
    if let Some(background) = style.background {
        writeln!(svg, r#"<rect x="{x:.1}" y="{}" width="{width:.1}" height="24" rx="2" fill="{background}"/>"#, y - 18).unwrap();
    }
    // Position words explicitly: SVG backends differ in whether they preserve
    // leading/repeated spaces, which otherwise destroys tree indentation.
    let mut offset = 0;
    for word in span.split_inclusive(' ') {
        let visible = word.trim_end_matches(' ');
        if !visible.is_empty() {
            let word_x = x + offset as f64 * 12.0;
            writeln!(svg, r#"<text x="{word_x:.1}" y="{y}" font-size="19" font-weight="{}" fill="{}">{}</text>"#, if style.bold { 700 } else { 400 }, style.foreground, xml(visible)).unwrap();
        }
        offset += textwrap::core::display_width(word);
    }
    *column += columns;
    span.clear();
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn add_components(report: &mut ScanReport) {
    report.findings[0].path = "crates/api/src/api.rs".into();
    report.findings[1].path = "crates/api/src/api.rs".into();
    report.findings[2].path = "packages/cache/src/cache.ts".into();
    report.findings[0].component_id = Some("directory:crates/api".into());
    report.findings[1].component_id = Some("directory:crates/api".into());
    report.findings[2].component_id = Some("directory:packages/cache".into());
    report.components = vec![
        Component {
            id: "directory:crates/api".into(),
            root: "crates/api".into(),
            status: ComponentStatus::Detected,
            identities: vec![ComponentIdentity {
                ecosystem: Ecosystem::Cargo,
                name: "api".into(),
                manifest: "crates/api/Cargo.toml".into(),
                status: ComponentStatus::Detected,
            }],
        },
        Component {
            id: "directory:packages/cache".into(),
            root: "packages/cache".into(),
            status: ComponentStatus::Detected,
            identities: vec![ComponentIdentity {
                ecosystem: Ecosystem::Npm,
                name: "@acme/cache".into(),
                manifest: "packages/cache/package.json".into(),
                status: ComponentStatus::Detected,
            }],
        },
    ];
}

fn author(name: &str, commit: &str) -> GitAuthor {
    GitAuthor {
        name: name.into(),
        email: format!("{}@example.test", name.to_lowercase()),
        commit: commit.into(),
        author_time: 1_790_856_000,
    }
}

fn fixture() -> ScanReport {
    let mut reference = references::parse("acme/api#42").remove(0);
    references::resolve(&mut reference, "github.com", None);
    reference.status = ReferenceStatus::Available;
    reference.issue = Some(IssueMetadata {
        title: "Validate incoming requests".into(),
        kind: "issue".into(),
    });
    let findings = vec![
        Finding {
            path: "src/api.rs".into(),
            line: 42,
            end_line: 42,
            marker: "TODO".into(),
            annotation: Some("alex".into()),
            message: "Validate request payloads before processing".into(),
            detection: Detection::Comment,
            references: vec![reference.clone()],
            attribution: None,
            component_id: None,
        },
        Finding {
            path: "src/api.rs".into(),
            line: 73,
            end_line: 73,
            marker: "FIXME".into(),
            annotation: None,
            message: "Reject malformed headers".into(),
            detection: Detection::Comment,
            references: vec![reference],
            attribution: None,
            component_id: None,
        },
        Finding {
            path: "src/cache.rs".into(),
            line: 87,
            end_line: 87,
            marker: "TODO".into(),
            annotation: Some("morgan".into()),
            message: "Add an eviction strategy for cached responses".into(),
            detection: Detection::Comment,
            references: vec![],
            attribution: None,
            component_id: None,
        },
    ];
    ScanReport {
        metadata: Metadata {
            generator: GENERATOR.into(),
            schema_version: SCHEMA_VERSION,
            source_fingerprint: "synthetic-demo".into(),
        },
        counts: scanner::counts(&findings),
        findings,
        stats: ScanStats::default(),
        diagnostics: vec![],
        violations: vec![],
        components: vec![],
    }
}
