use crate::{
    cli::{ComponentLayout, Detail, Format, GroupBy, InnerGroupBy},
    model::*,
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    path::Path,
};

pub struct Options {
    pub format: Format,
    pub detail: Detail,
    pub group_by: GroupBy,
    pub component_group_by: InnerGroupBy,
    pub component_layout: ComponentLayout,
    pub width: usize,
    pub line_spacing: u16,
    pub colors: bool,
    pub links: bool,
    /// Source directory relative to the exported report's directory.
    pub source_prefix: String,
    pub absolute_source_links: bool,
}

pub fn render(report: &ScanReport, options: &Options) -> Result<String> {
    Ok(match options.format {
        Format::Json => format!("{}\n", serde_json::to_string_pretty(report)?),
        Format::Markdown => markdown(report, options)?,
        Format::Html => html(report, options)?,
        Format::Console | Format::Text => text(report, options),
    })
}

pub fn metadata(text: &str) -> Result<Metadata> {
    if text.trim_start().starts_with('{') {
        #[derive(serde::Deserialize)]
        struct Envelope {
            metadata: Metadata,
        }
        let envelope: Envelope =
            serde_json::from_str(text).context("report has no supported metadata")?;
        validate_metadata(&envelope.metadata)?;
        return Ok(envelope.metadata);
    }
    for line in text.lines().take(3) {
        if let Some(json) = line
            .strip_prefix("<!-- todo-unearther:")
            .and_then(|s| s.strip_suffix(" -->"))
        {
            let metadata = serde_json::from_str(json).context("invalid report metadata")?;
            validate_metadata(&metadata)?;
            return Ok(metadata);
        }
    }
    bail!("report has no source fingerprint; regenerate it or use --compare full")
}

fn validate_metadata(metadata: &Metadata) -> Result<()> {
    ensure!(
        metadata.generator == GENERATOR && metadata.schema_version == SCHEMA_VERSION,
        "unsupported report metadata; regenerate with this version"
    );
    Ok(())
}

pub fn owns(text: &str) -> bool {
    metadata(text).is_ok()
        || text.starts_with("todo-unearther report\n")
        || text
            .strip_prefix("todo-unearther\n")
            .is_some_and(|rest| rest.trim_start_matches('\n').starts_with("report\n"))
}

fn header(report: &ScanReport) -> Result<String> {
    Ok(format!(
        "<!-- todo-unearther:{} -->\n",
        serde_json::to_string(&report.metadata)?
    ))
}

fn flat_groups<'a>(
    findings: impl IntoIterator<Item = &'a Finding>,
    group_by: GroupBy,
    root: &str,
) -> BTreeMap<String, Vec<&'a Finding>> {
    let mut groups = BTreeMap::<String, Vec<&Finding>>::new();
    for finding in findings {
        let path = relative_source_path(&finding.path, root);
        let key = match group_by {
            GroupBy::File => path.to_owned(),
            GroupBy::Marker => format!("{} / {path}", finding.marker),
            GroupBy::Component => unreachable!("component grouping is an outer grouping"),
        };
        groups.entry(key).or_default().push(finding);
    }
    groups
}

struct RenderGroup<'a> {
    heading: String,
    findings: Vec<&'a Finding>,
    depth: usize,
    prefix: String,
    indent: String,
    root: String,
}

enum RenderEvent<'a> {
    Start {
        heading: String,
        count: usize,
        depth: usize,
        prefix: String,
    },
    Group(RenderGroup<'a>),
    End,
}

fn groups<'a>(report: &'a ScanReport, options: &Options) -> Vec<RenderEvent<'a>> {
    if options.group_by != GroupBy::Component {
        return flat_groups(&report.findings, options.group_by, "")
            .into_iter()
            .map(|(heading, findings)| {
                RenderEvent::Group(RenderGroup {
                    heading,
                    findings,
                    depth: 0,
                    prefix: String::new(),
                    indent: String::new(),
                    root: String::new(),
                })
            })
            .collect();
    }
    let metadata: BTreeMap<_, _> = report
        .components
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();
    let mut owned = BTreeMap::<&str, Vec<&Finding>>::new();
    for finding in &report.findings {
        owned
            .entry(
                finding
                    .component_id
                    .as_deref()
                    .unwrap_or(crate::components::SHARED_ID),
            )
            .or_default()
            .push(finding);
    }
    // Sort by normalized path segments, independently of IDs or package names.
    let mut owned: Vec<_> = owned.into_iter().collect();
    owned.sort_by_key(|(id, _)| {
        metadata
            .get(id)
            .map(|c| c.root.split('/').collect::<Vec<_>>())
    });
    if options.component_layout == ComponentLayout::Tree {
        return tree_groups(owned, &metadata, options.component_group_by);
    }
    let mut result = Vec::new();
    for (id, findings) in owned {
        let count = findings.len();
        let label = metadata.get(id).map_or_else(
            || "Repository / shared".into(),
            |component| component.heading(),
        );
        result.push(RenderEvent::Start {
            heading: label,
            count,
            depth: 0,
            prefix: String::new(),
        });
        for (heading, findings) in flat_groups(findings, options.component_group_by.into(), "") {
            result.push(RenderEvent::Group(RenderGroup {
                heading,
                findings,
                depth: 1,
                prefix: "  ".into(),
                indent: "  ".into(),
                root: String::new(),
            }));
        }
        result.push(RenderEvent::End);
    }
    result
}

#[derive(Default)]
struct DirectoryNode<'a> {
    component: Option<&'a Component>,
    findings: Vec<&'a Finding>,
    children: BTreeMap<String, DirectoryNode<'a>>,
}

impl DirectoryNode<'_> {
    fn count(&self) -> usize {
        self.findings.len() + self.children.values().map(Self::count).sum::<usize>()
    }
}

fn tree_groups<'a>(
    owned: Vec<(&str, Vec<&'a Finding>)>,
    metadata: &BTreeMap<&str, &'a Component>,
    inner: InnerGroupBy,
) -> Vec<RenderEvent<'a>> {
    let mut root = DirectoryNode::default();
    for (id, findings) in owned {
        let component = metadata.get(id).copied();
        let shared = component.is_none_or(|c| c.status == ComponentStatus::Shared);
        let mut node = &mut root;
        if shared {
            // A separate leaf distinguishes unowned findings from root-component findings.
            node = node
                .children
                .entry("Repository / shared".into())
                .or_default();
        } else if let Some(component) = component {
            for part in component.root.split('/').filter(|part| *part != ".") {
                node = node.children.entry(part.to_owned()).or_default();
            }
        }
        node.component = component;
        node.findings = findings;
    }
    let mut result = Vec::new();
    if root.count() != 0 {
        append_tree(&root, ".", inner, 0, "", "", &mut result);
    }
    result
}

fn append_tree<'a>(
    node: &DirectoryNode<'a>,
    folder: &str,
    inner: InnerGroupBy,
    depth: usize,
    prefix: &str,
    indent: &str,
    result: &mut Vec<RenderEvent<'a>>,
) {
    let heading = node.component.map_or_else(
        || folder.to_owned(),
        |component| {
            // The hierarchy already supplies the ancestor directories.
            let mut component = component.clone();
            component.root = folder.to_owned();
            component.heading()
        },
    );
    result.push(RenderEvent::Start {
        heading,
        count: node.count(),
        depth,
        prefix: prefix.to_owned(),
    });
    let root = node
        .component
        .map_or("", |component| component.root.as_str());
    let groups = flat_groups(node.findings.iter().copied(), inner.into(), root);
    let total = groups.len() + node.children.len();
    for (index, (heading, findings)) in groups.into_iter().enumerate() {
        let (prefix, indent) = tree_branch(indent, index + 1 == total);
        result.push(RenderEvent::Group(RenderGroup {
            heading,
            findings,
            depth: depth + 1,
            prefix,
            indent,
            root: root.to_owned(),
        }));
    }
    for (index, (folder, child)) in node.children.iter().enumerate() {
        let (prefix, indent) = tree_branch(indent, index + 1 == node.children.len());
        append_tree(child, folder, inner, depth + 1, &prefix, &indent, result);
    }
    result.push(RenderEvent::End);
}

fn tree_branch(indent: &str, last: bool) -> (String, String) {
    (
        format!("{indent}{}", if last { "└── " } else { "├── " }),
        format!("{indent}{}", if last { "    " } else { "│   " }),
    )
}

fn is_tree(options: &Options) -> bool {
    options.group_by == GroupBy::Component && options.component_layout == ComponentLayout::Tree
}

fn reference_label(reference: &IssueReference) -> String {
    let identity = reference.repository.as_ref().map_or_else(
        || reference.raw.clone(),
        |r| format!("{}#{}", r.to_ascii_lowercase(), reference.number),
    );
    if let Some(host) = reference
        .host
        .as_deref()
        .filter(|host| *host != "github.com")
    {
        format!("{host}/{identity}")
    } else {
        identity
    }
}

fn relative_source_path<'a>(path: &'a str, root: &str) -> &'a str {
    if root.is_empty() || root == "." {
        return path;
    }
    path.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
}

fn summary(report: &ScanReport) -> String {
    summary_with(report, |marker, count| format!("{marker}: {count}"))
}

fn summary_with(report: &ScanReport, marker_count: impl Fn(&str, usize) -> String) -> String {
    let counts = report
        .counts
        .iter()
        .map(|(marker, count)| marker_count(marker, *count))
        .collect::<Vec<_>>()
        .join(" · ");
    let files = report
        .findings
        .iter()
        .map(|f| &f.path)
        .collect::<BTreeSet<_>>()
        .len();
    format!(
        "{} in {files} file{}{}",
        finding_count(report.findings.len()),
        if files == 1 { "" } else { "s" },
        if counts.is_empty() {
            String::new()
        } else {
            format!(" · {counts}")
        }
    )
}

fn finding_count(count: usize) -> String {
    format!("{count} finding{}", if count == 1 { "" } else { "s" })
}

fn markdown(report: &ScanReport, options: &Options) -> Result<String> {
    let mut out = String::from("<!-- markdownlint-disable MD013 MD024 MD033 -->\n");
    out.push_str(&header(report)?);
    writeln!(out, "# TODO report\n\n{}\n", md(&summary(report))).unwrap();
    if report.findings.is_empty() {
        out.push_str("No marked comments found.\n\n");
    }
    for event in groups(report, options) {
        let group = match event {
            RenderEvent::Start {
                heading,
                count,
                depth,
                ..
            } => {
                if is_tree(options) {
                    writeln!(
                        out,
                        "{}- **{}** — {}\n",
                        "  ".repeat(depth),
                        md(&heading),
                        finding_count(count)
                    )
                    .unwrap();
                } else {
                    writeln!(out, "## {}\n\n{}\n", md(&heading), finding_count(count)).unwrap();
                }
                continue;
            }
            RenderEvent::End => continue,
            RenderEvent::Group(group) => group,
        };
        let heading = group.heading;
        let findings = group.findings;
        let level = if options.group_by == GroupBy::Component {
            "###"
        } else {
            "##"
        };
        let indent = if is_tree(options) {
            "  ".repeat(group.depth + 1)
        } else {
            String::new()
        };
        if is_tree(options) {
            writeln!(out, "{}- **{}**\n", "  ".repeat(group.depth), md(&heading)).unwrap();
        } else {
            writeln!(out, "{level} {}\n", md(&heading)).unwrap();
        }
        if options.detail == Detail::Summary {
            writeln!(out, "{indent}{}.\n", finding_count(findings.len())).unwrap();
            continue;
        }
        for finding in findings {
            let location = format!(
                "{}:{}",
                relative_source_path(&finding.path, &group.root),
                finding.line
            );
            let source = source_url(finding, options);
            let annotation = finding
                .annotation
                .as_ref()
                .map_or(String::new(), |label| format!(" ({})", md(label)));
            writeln!(
                out,
                "{indent}- **{}**{annotation} — [{}]({source}): {}",
                md(&finding.marker),
                md(&location),
                md(&presentation_message(finding)).replace('\n', "<br>")
            )
            .unwrap();
            let details = details(finding);
            if !details.is_empty() {
                writeln!(out, "{indent}  - {}", md(&details)).unwrap();
            }
            for reference in &finding.references {
                let label = reference_description(reference);
                if let Some(url) = safe_url(reference.url.as_deref()) {
                    writeln!(
                        out,
                        "{indent}  - [{}]({})",
                        md(&label),
                        url.replace('(', "%28").replace(')', "%29")
                    )
                    .unwrap();
                } else {
                    writeln!(out, "{indent}  - {}", md(&label)).unwrap();
                }
            }
        }
        out.push('\n');
    }
    markdown_diagnostics(&mut out, report);
    out.truncate(out.trim_end().len());
    out.push('\n');
    Ok(out)
}

fn markdown_diagnostics(out: &mut String, report: &ScanReport) {
    let skips = skipped(report);
    if !skips.is_empty() {
        writeln!(out, "## Scan exclusions\n\n{}\n", md(&skips)).unwrap();
    }
    if !report.diagnostics.is_empty() {
        out.push_str("## Warnings\n\n");
        for diagnostic in &report.diagnostics {
            writeln!(
                out,
                "- {}: {}",
                md(&diagnostic.stage),
                md(&diagnostic.message)
            )
            .unwrap();
        }
        out.push('\n');
    }
    if !report.violations.is_empty() {
        out.push_str("## Policy violations\n\n");
        for violation in &report.violations {
            writeln!(
                out,
                "- {}: {}",
                md(&violation.policy),
                md(&violation.message)
            )
            .unwrap();
        }
    }
}

fn text(report: &ScanReport, options: &Options) -> String {
    let console = options.format == Format::Console;
    let mut out = String::new();
    write_wrapped(&mut out, "todo-unearther report", "", "", options);
    let summary = summary_with(report, |marker, count| {
        paint(&format!("{marker}: {count}"), marker_style(marker), options)
    });
    write_wrapped(&mut out, &summary, "", "", options);
    if !console {
        out.push('\n');
    }
    // Use one body layout for the entire report. A long location should not
    // make adjacent TODOs switch unpredictably between inline and stacked text.
    let events = groups(report, options);
    let stacked = options.width < 100
        || events.iter().any(|event| {
            if let RenderEvent::Group(group) = event {
                group.findings.iter().any(|finding| {
                    textwrap::core::display_width(&group.indent)
                        + textwrap::core::display_width(&finding_header(finding, &group.root))
                        + 5
                        + 32
                        > options.width
                })
            } else {
                false
            }
        });
    if report.findings.is_empty() {
        write_wrapped(&mut out, "No marked comments found.", "", "", options);
    }
    for event in events {
        let group = match event {
            RenderEvent::Start {
                heading,
                count,
                prefix,
                ..
            } => {
                let heading = if is_tree(options) {
                    format!("{} — {}", clean(&heading), finding_count(count))
                } else {
                    clean(&heading)
                };
                let rest = continuation_prefix(&prefix);
                write_wrapped(
                    &mut out,
                    &paint(&heading, "1;35", options),
                    &prefix,
                    &rest,
                    options,
                );
                if !is_tree(options) {
                    write_wrapped(&mut out, &finding_count(count), "", "", options);
                    if !console {
                        out.push('\n');
                    }
                }
                continue;
            }
            RenderEvent::End => continue,
            RenderEvent::Group(group) => group,
        };
        let heading = group.heading;
        let findings = group.findings;
        let indent = group.indent;
        let prefix = group.prefix;
        let separator = if is_tree(options) {
            indent.trim_end()
        } else {
            ""
        };
        let heading = clean(&heading);
        write_wrapped(
            &mut out,
            &paint(&heading, "1;36", options),
            &prefix,
            &indent,
            options,
        );
        if options.detail == Detail::Summary {
            write_wrapped(
                &mut out,
                &finding_count(findings.len()),
                &format!("{indent}  "),
                &format!("{indent}  "),
                options,
            );
            if !console {
                write_wrapped(&mut out, separator, "", "", options);
            }
            continue;
        }
        for finding in findings {
            let location = format!(
                "{}:{}",
                clean(relative_source_path(&finding.path, &group.root)),
                finding.line
            );
            let location = if console && options.links {
                osc_link(&location, &source_url(finding, options))
            } else {
                location
            };
            let label = finding
                .annotation
                .as_ref()
                .map_or(String::new(), |a| format!(" ({})", clean(a)));
            let header = format!(
                "{}{label} {location}",
                marker_badge(&finding.marker, options)
            );
            let message = presentation_message(finding);
            let base = format!("{indent}  ");
            if stacked || message.is_empty() {
                write_wrapped(&mut out, &header, &base, &format!("{base}  "), options);
                if !message.is_empty() {
                    let body = format!("{base}  ");
                    write_wrapped(&mut out, &message, &body, &body, options);
                }
            } else {
                let plain_header = finding_header(finding, &group.root);
                let body = format!(
                    "{base}{}",
                    " ".repeat(textwrap::core::display_width(&plain_header) + 3)
                );
                write_wrapped(
                    &mut out,
                    &format!("{header} — {message}"),
                    &base,
                    &body,
                    options,
                );
            }
            let details = details(finding);
            if !details.is_empty() {
                write_wrapped(
                    &mut out,
                    &clean(&details),
                    &format!("{indent}    "),
                    &format!("{indent}    "),
                    options,
                );
            }
            for reference in &finding.references {
                let name = clean(&reference_label(reference));
                let name = if console && options.links {
                    safe_url(reference.url.as_deref())
                        .map_or(name.clone(), |url| osc_link(&name, &url))
                } else {
                    name
                };
                let details = clean(&reference_details(reference));
                let reference = if details.is_empty() {
                    name
                } else {
                    format!("{name} — {details}")
                };
                write_wrapped(
                    &mut out,
                    &reference,
                    &format!("{indent}    "),
                    &format!("{indent}    "),
                    options,
                );
            }
        }
        if !console {
            write_wrapped(&mut out, separator, "", "", options);
        }
    }
    let skips = skipped(report);
    if !skips.is_empty() {
        write_wrapped(&mut out, &format!("Skipped: {skips}"), "", "  ", options);
    }
    for diagnostic in &report.diagnostics {
        write_wrapped(
            &mut out,
            &format!(
                "Warning ({}): {}",
                clean(&diagnostic.stage),
                clean(&diagnostic.message)
            ),
            "",
            "  ",
            options,
        );
    }
    for violation in &report.violations {
        write_wrapped(
            &mut out,
            &format!(
                "Policy ({}): {}",
                clean(&violation.policy),
                clean(&violation.message)
            ),
            "",
            "  ",
            options,
        );
    }
    out
}

fn finding_header(finding: &Finding, root: &str) -> String {
    let annotation = finding
        .annotation
        .as_ref()
        .map_or(String::new(), |annotation| {
            format!(" ({})", clean(annotation))
        });
    format!(
        "{}{annotation} {}:{}",
        finding.marker,
        clean(relative_source_path(&finding.path, root)),
        finding.line
    )
}

fn paint(text: &str, code: &str, options: &Options) -> String {
    if options.format == Format::Console && options.colors {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

fn marker_badge(marker: &str, options: &Options) -> String {
    paint(marker, marker_style(marker), options)
}

fn marker_style(marker: &str) -> &'static str {
    match marker {
        "TODO" => "1;97;44",
        "FIXME" | "BUG" => "1;97;41",
        "HACK" => "1;30;43",
        "XXX" => "1;97;45",
        _ => "1;30;46",
    }
}

fn continuation_prefix(prefix: &str) -> String {
    prefix
        .chars()
        .map(|c| {
            if matches!(c, '├' | '└' | '─') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn write_wrapped(out: &mut String, text: &str, first: &str, rest: &str, options: &Options) {
    let width = options.width.max(20);
    let fit_prefix = |prefix: &str| {
        let columns = textwrap::core::display_width(prefix);
        if columns > width.saturating_sub(12) {
            let keep = width.saturating_sub(14);
            format!(
                "… {}",
                prefix
                    .chars()
                    .skip(prefix.chars().count().saturating_sub(keep))
                    .collect::<String>()
            )
        } else {
            prefix.to_owned()
        }
    };
    let first = fit_prefix(first);
    let rest = fit_prefix(rest);
    let text = text.replace('\t', "    ");
    let mut active_link = None;
    // Each source line is a new paragraph to textwrap. Its first-line budget
    // must use the continuation indent after the first source line too.
    for (index, line) in text.split('\n').enumerate() {
        if options.format == Format::Console && line.trim().is_empty() {
            continue;
        }
        let initial = if index == 0 { &first } else { &rest };
        let layout = textwrap::Options::new(width)
            .initial_indent(initial)
            .subsequent_indent(&rest)
            .word_splitter(textwrap::WordSplitter::NoHyphenation);
        for (wrapped_index, wrapped) in textwrap::wrap(line, layout).into_iter().enumerate() {
            let prefix = if wrapped_index == 0 { initial } else { &rest };
            if options.format == Format::Console && !out.is_empty() {
                let guide = if is_tree(options) {
                    prefix
                        .chars()
                        .map(|c| match c {
                            '├' | '└' => '│',
                            '─' | '…' => ' ',
                            other => other,
                        })
                        .collect::<String>()
                } else {
                    String::new()
                };
                for _ in 0..options.line_spacing {
                    writeln!(out, "{}", guide.trim_end()).unwrap();
                }
            }
            if options.format == Format::Console && options.links {
                write_linked_line(out, &wrapped, prefix, &mut active_link);
            } else {
                writeln!(out, "{wrapped}").unwrap();
            }
        }
    }
}

const LINK_START: &str = "\x1b]8;;";
const LINK_END: &str = "\x1b\\";
const LINK_CLOSE: &str = "\x1b]8;;\x1b\\";

fn write_linked_line(out: &mut String, line: &str, prefix: &str, active: &mut Option<String>) {
    // Reopen a wrapped link after indentation, keeping guides and spacer rows
    // outside the clickable span. Track only original sequences, so closing at
    // the physical line boundary does not discard the logical link's target.
    let mut content = if let Some(content) = line.strip_prefix(prefix) {
        out.push_str(prefix);
        content
    } else {
        line
    };
    if let Some(url) = active.as_deref() {
        write!(out, "{LINK_START}{url}{LINK_END}").unwrap();
    }
    while let Some(start) = content.find(LINK_START) {
        let after_start = &content[start + LINK_START.len()..];
        let Some(end) = after_start.find(LINK_END) else {
            break;
        };
        let url = &after_start[..end];
        *active = (!url.is_empty()).then(|| url.to_owned());
        let sequence_end = start + LINK_START.len() + end + LINK_END.len();
        out.push_str(&content[..sequence_end]);
        content = &content[sequence_end..];
    }
    out.push_str(content);
    if active.is_some() {
        out.push_str(LINK_CLOSE);
    }
    out.push('\n');
}

fn html(report: &ScanReport, options: &Options) -> Result<String> {
    let css = if options.group_by == GroupBy::Component {
        format!("{HTML_CSS}{COMPONENT_CSS}")
    } else {
        HTML_CSS.to_owned()
    };
    let mut out = format!(
        "<!doctype html>\n{}<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>TODO report</title><style>{css}</style></head><body><main><h1>TODO report</h1><p class=\"muted\">{}</p>\n",
        header(report)?,
        summary_with(report, |marker, count| format!(
            "<span class=\"marker marker-{}\">{}: {count}</span>",
            marker_class(marker),
            escape_html(marker)
        ))
    );
    if report.findings.is_empty() {
        out.push_str("<p>No marked comments found.</p>");
    }
    for event in groups(report, options) {
        let group = match event {
            RenderEvent::Start {
                heading,
                count,
                depth,
                ..
            } => {
                // Lists in Markdown and sections here retain arbitrary directory depth.
                let level = (depth + 2).min(6);
                writeln!(
                    out,
                    "<section class=\"component\"><h{level}>{}</h{level}><p class=\"muted\">{}</p>",
                    escape_html(&heading),
                    finding_count(count)
                )
                .unwrap();
                continue;
            }
            RenderEvent::End => {
                out.push_str("</section>\n");
                continue;
            }
            RenderEvent::Group(group) => group,
        };
        let heading = group.heading;
        let findings = group.findings;
        let level = format!("h{}", (group.depth + 2).min(6));
        writeln!(out, "<section><{level}>{}</{level}>", escape_html(&heading)).unwrap();
        if options.detail == Detail::Summary {
            writeln!(out, "<p>{}.</p></section>", finding_count(findings.len())).unwrap();
            continue;
        }
        out.push_str("<ul>");
        for finding in findings {
            write!(
                out,
                "<li><strong class=\"marker marker-{}\">{}</strong> <a href=\"{}\">{}:{}</a>",
                marker_class(&finding.marker),
                escape_html(&finding.marker),
                escape_html(&source_url(finding, options)),
                escape_html(relative_source_path(&finding.path, &group.root)),
                finding.line
            )
            .unwrap();
            if let Some(annotation) = &finding.annotation {
                write!(
                    out,
                    " <span class=\"muted\">({})</span>",
                    escape_html(annotation)
                )
                .unwrap();
            }
            write!(
                out,
                "<p class=\"message\">{}</p>",
                escape_html(&presentation_message(finding))
            )
            .unwrap();
            let details = details(finding);
            if !details.is_empty() {
                write!(out, "<p class=\"muted\">{}</p>", escape_html(&details)).unwrap();
            }
            if !finding.references.is_empty() {
                out.push_str("<ul>");
            }
            for reference in &finding.references {
                out.push_str("<li>");
                if let Some(url) = safe_url(reference.url.as_deref()) {
                    write!(
                        out,
                        "<a href=\"{}\">{}</a>",
                        escape_html(&url),
                        escape_html(&reference_label(reference))
                    )
                    .unwrap();
                } else {
                    out.push_str(&escape_html(&reference_label(reference)));
                }
                let extra = reference_details(reference);
                if !extra.is_empty() {
                    write!(out, " — {}", escape_html(&extra)).unwrap();
                }
                out.push_str("</li>");
            }
            if !finding.references.is_empty() {
                out.push_str("</ul>");
            }
            out.push_str("</li>");
        }
        out.push_str("</ul></section>\n");
    }
    let skips = skipped(report);
    if !skips.is_empty() {
        write!(
            out,
            "<p class=\"muted\">Skipped: {}</p>",
            escape_html(&skips)
        )
        .unwrap();
    }
    for diagnostic in &report.diagnostics {
        write!(
            out,
            "<p class=\"warning\">{}: {}</p>",
            escape_html(&diagnostic.stage),
            escape_html(&diagnostic.message)
        )
        .unwrap();
    }
    for violation in &report.violations {
        write!(
            out,
            "<p class=\"warning\">Policy {}: {}</p>",
            escape_html(&violation.policy),
            escape_html(&violation.message)
        )
        .unwrap();
    }
    out.push_str("</main></body></html>\n");
    Ok(out)
}

fn details(finding: &Finding) -> String {
    let mut details = Vec::new();
    if let Some(attribution) = &finding.attribution {
        if let Some(author) = &attribution.blame {
            details.push(format!(
                "last changed by {} · {} · {}",
                author.name,
                short_commit(&author.commit),
                author_date(author.author_time)
            ));
        }
        if let Some(author) = &attribution.origin {
            details.push(format!(
                "origin ({}) {} · {} · {}",
                attribution.origin_status,
                author.name,
                short_commit(&author.commit),
                author_date(author.author_time)
            ));
        }
        if let Some(note) = &attribution.note {
            details.push(note.clone());
        }
    }
    details.join(" · ")
}

fn short_commit(commit: &str) -> &str {
    &commit[..commit.floor_char_boundary(commit.len().min(10))]
}

fn author_date(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0).map_or_else(
        || timestamp.to_string(),
        |date| date.format("%Y-%m-%d").to_string(),
    )
}

fn reference_details(reference: &IssueReference) -> String {
    let mut parts = vec![];
    if let Some(issue) = &reference.issue {
        if issue.kind == "pull_request" {
            parts.push("pull request".into());
        }
        parts.push(issue.title.clone());
    }
    if reference.status == ReferenceStatus::Unavailable {
        parts.push("Unavailable".into());
    }
    if let Some(error) = &reference.error {
        parts.push(error.clone());
    }
    parts.join(" · ")
}

fn reference_description(reference: &IssueReference) -> String {
    let label = reference_label(reference);
    let details = reference_details(reference);
    if details.is_empty() {
        label
    } else {
        format!("{label} — {details}")
    }
}

// Keep the original text in JSON/fingerprints. Standalone links belong in the
// reference row; inline links use a compact identity without losing surrounding prose.
fn presentation_message(finding: &Finding) -> String {
    finding
        .message
        .lines()
        .filter_map(|line| {
            if finding
                .references
                .iter()
                .any(|reference| line.trim() == reference.raw)
            {
                return None;
            }
            let mut line = clean(line);
            for reference in &finding.references {
                if reference.raw.starts_with("http") {
                    if let Some(prose) = line.strip_suffix(&reference.raw)
                        && prose.ends_with(char::is_whitespace)
                        && prose.trim_end().ends_with(['.', '?', '!'])
                    {
                        line = prose.trim_end().to_owned();
                    } else {
                        line = line.replace(&reference.raw, &reference_label(reference));
                    }
                }
            }
            Some(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn skipped(report: &ScanReport) -> String {
    let mut parts = Vec::new();
    for (count, label) in [
        (report.stats.skipped_binary, "binary"),
        (report.stats.skipped_encoding, "invalid UTF-8 candidates"),
        (report.stats.skipped_oversized, "oversized"),
    ] {
        if count != 0 {
            parts.push(format!("{count} {label}"));
        }
    }
    parts.join(", ")
}

fn source_url(finding: &Finding, options: &Options) -> String {
    // Keep native prefixes intact until URL conversion. Replacing backslashes
    // first corrupts Windows verbatim drive and UNC paths returned by canonicalize.
    if (options.format == Format::Console || options.absolute_source_links)
        && let Ok(mut url) =
            reqwest::Url::from_file_path(Path::new(&options.source_prefix).join(&finding.path))
    {
        url.set_fragment(Some(&format!("L{}", finding.line)));
        return url.to_string();
    }
    let prefix = options.source_prefix.trim_end_matches('/');
    let path = if prefix.is_empty() || prefix == "." {
        finding.path.clone()
    } else {
        format!("{prefix}/{}", finding.path)
    };
    let encoded = path
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect::<String>();
    // Keep relative paths with colon-bearing first components from becoming URL schemes.
    let encoded = if !encoded.starts_with(['.', '/']) {
        format!("./{encoded}")
    } else {
        encoded
    };
    format!("{encoded}#L{}", finding.line)
}

fn safe_url(url: Option<&str>) -> Option<String> {
    let url = reqwest::Url::parse(url?).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

fn osc_link(label: &str, url: &str) -> String {
    let url: String = url.chars().filter(|c| !c.is_control()).collect();
    format!("\x1b]8;;{url}\x1b\\{}\x1b]8;;\x1b\\", clean(label))
}

fn marker_class(marker: &str) -> &'static str {
    match marker {
        "TODO" => "todo",
        "FIXME" | "BUG" => "fixme",
        "HACK" => "hack",
        "XXX" => "xxx",
        _ => "other",
    }
}

pub fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

fn md(text: &str) -> String {
    let mut out = String::new();
    for c in clean(text).chars() {
        if "\\`*_{}[]()#+!|>".contains(c) {
            out.push('\\');
        }
        match c {
            '<' => out.push_str("&lt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_html(text: &str) -> String {
    clean(text)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn infer_format(output: Option<&Path>, terminal: bool) -> Format {
    if let Some(path) = output.filter(|p| *p != Path::new("-")) {
        return match path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "json" => Format::Json,
            "html" | "htm" => Format::Html,
            "txt" => Format::Text,
            _ => Format::Markdown,
        };
    }
    if terminal {
        Format::Console
    } else {
        Format::Text
    }
}

const HTML_CSS: &str = r#"
:root{color-scheme:light dark;--bg:#f6f8fa;--card:white;--text:#1f2328;--muted:#59636e;--border:#d0d7de;--link:#0969da}
@media(prefers-color-scheme:dark){:root{--bg:#0d1117;--card:#161b22;--text:#e6edf3;--muted:#9198a1;--border:#30363d;--link:#58a6ff}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font:15px/1.6 system-ui,sans-serif}main{max-width:1100px;margin:auto;padding:32px 24px}h1{color:var(--link)}h2{font-size:1.1rem;overflow-wrap:anywhere}section{background:var(--card);border:1px solid var(--border);border-radius:10px;margin:20px 0;padding:12px 24px}a{color:var(--link)}li{margin:12px 0;overflow-wrap:anywhere}.message{white-space:pre-wrap;margin:4px 0}.muted{color:var(--muted)}.warning{border-left:3px solid #d29922;padding:8px 16px}ul{padding-left:20px}
@media print{:root{--bg:white;--card:white;--text:black;--muted:#444}section{break-inside:avoid}main{max-width:none;padding:0}}
.marker{display:inline-block;border-radius:4px;padding:0 5px;color:white;background:#0550ae}.marker-fixme{background:#a40e26}.marker-hack{background:#7d4e00}.marker-xxx{background:#6639ba}.marker-other{background:#0a6060}
"#;

const COMPONENT_CSS: &str = "section.component>h2{font-size:1.25rem}section.component>section{background:var(--bg);margin:12px 0;padding:8px 16px}h3{font-size:1rem;overflow-wrap:anywhere}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_inference_and_escaping() {
        assert_eq!(infer_format(None, true), Format::Console);
        assert_eq!(infer_format(None, false), Format::Text);
        assert_eq!(
            infer_format(Some(Path::new("TODO.md")), false),
            Format::Markdown
        );
        assert_eq!(
            infer_format(Some(Path::new("output.JSON")), false),
            Format::Json
        );
        assert_eq!(escape_html("<script>'&"), "&lt;script&gt;&#39;&amp;");
        assert!(!clean("hello\x1b[31m").contains('\x1b'));
        assert!(safe_url(Some("javascript:alert(1)")).is_none());
    }
}
