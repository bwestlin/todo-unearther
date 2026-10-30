use todo_unearther::{
    cli::{ComponentLayout, Detail, Format, GroupBy, InnerGroupBy},
    config::{GithubConfig, ScanConfig},
    github,
    model::*,
    output, scanner,
};

fn fixture() -> ScanReport {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("handlers.ts"), "\t\t// TODO:(bwestlin) Leave this fallback undefined so bypassed requests without a device_id\n\t\t// fail validation instead of provisioning or reusing unknown_device.\n\t\t// https://github.com/UXStream/uxstream/issues/2370\n").unwrap();
    let scan = scanner::scan(temp.path(), &ScanConfig::default(), None).unwrap();
    let mut report = ScanReport {
        metadata: Metadata {
            generator: GENERATOR.into(),
            schema_version: SCHEMA_VERSION,
            source_fingerprint: "fixture".into(),
        },
        findings: scan.findings,
        counts: Default::default(),
        stats: scan.stats,
        diagnostics: vec![],
        violations: vec![],
        components: vec![Component {
            id: "directory:services/main-api".into(),
            root: "services/main-api".into(),
            status: ComponentStatus::Detected,
            identities: vec![ComponentIdentity {
                ecosystem: Ecosystem::Npm,
                name: "main-api".into(),
                manifest: "services/main-api/package.json".into(),
                status: ComponentStatus::Detected,
            }],
        }],
    };
    report.findings[0].path = "services/main-api/src/handlers.ts".into();
    report.findings[0].component_id = Some("directory:services/main-api".into());
    github::resolve_references(&mut report, &GithubConfig::default(), temp.path(), false);
    report.counts = scanner::counts(&report.findings);
    report
}

fn options(format: Format, width: usize) -> output::Options {
    output::Options {
        format,
        detail: Detail::Full,
        group_by: GroupBy::Component,
        component_group_by: InnerGroupBy::File,
        component_layout: ComponentLayout::Tree,
        width,
        line_spacing: 1,
        colors: false,
        links: false,
        source_prefix: if cfg!(windows) {
            r"C:\tmp\repository"
        } else {
            "/tmp/repository"
        }
        .into(),
        absolute_source_links: false,
    }
}

#[test]
fn findings_own_their_issue_details_without_noisy_or_duplicate_metadata() {
    let mut report = fixture();
    report.findings[0].detection = Detection::Heuristic;
    report.findings[0].references[0].status = ReferenceStatus::Closed;
    report.findings[0].references[0].issue = Some(IssueMetadata {
        title: "Handle missing identifiers".into(),
        kind: "issue".into(),
    });
    // Multiple issue references must never duplicate the finding.
    let mut other = report.findings[0].references[0].clone();
    other.number = 42;
    other.raw = "#42".into();
    other.url = Some("https://github.com/UXStream/uxstream/issues/42".into());
    report.findings[0].references.push(other);
    for format in [
        Format::Console,
        Format::Text,
        Format::Markdown,
        Format::Html,
    ] {
        let text = output::render(&report, &options(format, 140)).unwrap();
        assert!(!text.contains("Unlinked"), "{text}");
        assert!(!text.contains("heuristic detection"), "{text}");
        assert!(!text.contains("lines 1–3"), "{text}");
        assert!(!text.contains("Closed"), "{text}");
        assert!(!text.contains("Not enriched"), "{text}");
        assert_eq!(text.matches("Leave this fallback").count(), 1, "{text}");
        assert!(text.contains("src/handlers.ts"));
        assert!(!text.contains("services/main-api/src/handlers.ts:"));
        assert!(
            text.find("Leave this fallback").unwrap()
                < text.find("Handle missing identifiers").unwrap()
        );
    }
    let json: ScanReport =
        serde_json::from_str(&output::render(&report, &options(Format::Json, 20)).unwrap())
            .unwrap();
    assert_eq!(json.findings[0].end_line, 3);
    assert_eq!(json.findings[0].detection, Detection::Heuristic);
    assert!(
        json.findings[0]
            .message
            .contains("https://github.com/UXStream/uxstream/issues/2370")
    );
    assert_eq!(json.findings[0].path, "services/main-api/src/handlers.ts");
}

#[test]
fn width_limits_include_unicode_links_headings_and_deep_tree_prefixes() {
    let mut report = fixture();
    report.findings[0].annotation = Some("界面开发者".repeat(5));
    report.findings[0].message = format!(
        "Unicode 界面 🛶 e\u{301} words\n{}",
        "long_identifier_".repeat(20)
    );
    report.components[0].identities[0].name = "界面包".repeat(20);
    report.components[0].root = "a/b/c/d/e/f/g/h/i/j/k/l/m".into();
    for width in [20, 40, 60, 100, 140] {
        for format in [Format::Text, Format::Console] {
            for styled in [false, true] {
                let mut opts = options(format, width);
                opts.colors = styled;
                opts.links = styled;
                let text = output::render(&report, &opts).unwrap();
                for line in text.lines() {
                    assert!(
                        textwrap::core::display_width(line) <= width,
                        "width {width}, line {line:?} in {text}"
                    );
                }
                if format == Format::Text || !styled {
                    assert!(!text.contains('\x1b'));
                }
            }
        }
    }
}

#[test]
fn comment_lines_align_inline_at_wide_width_and_stack_uniformly_when_narrow() {
    let mut report = fixture();
    report.findings[0].message = "First line\nSecond line".into();
    report.findings[0].references.clear();
    let mut second = report.findings[0].clone();
    second.marker = "FIXME".into();
    second.line = 8;
    second.message = "Another first\nAnother second".into();
    report.findings.push(second);
    for width in [60, 160] {
        let text = output::render(&report, &options(Format::Text, width)).unwrap();
        let lines: Vec<_> = text.lines().collect();
        for (first, second) in [
            ("First line", "Second line"),
            ("Another first", "Another second"),
        ] {
            let first_line = lines.iter().find(|line| line.contains(first)).unwrap();
            let second_line = lines.iter().find(|line| line.contains(second)).unwrap();
            assert_eq!(
                textwrap::core::display_width(&first_line[..first_line.find(first).unwrap()]),
                textwrap::core::display_width(&second_line[..second_line.find(second).unwrap()]),
                "{text}"
            );
            assert_eq!(first_line.contains(" — "), width >= 100, "{text}");
        }
    }
}

#[test]
fn long_multiline_comments_respect_width_with_an_inline_header() {
    let mut report = fixture();
    report.findings[0].message = "Split option into\n`enablePinchToZoom` - Desktop and mobile: viewport_enabled = true for touchscreen visual pinch zoom.\n`enableMobileViewport` - Mobile only: viewport_meta_enabled = true and shrinks_viewport_contents_to_fit = true.".into();
    for width in [100, 120, 140, 160, 200] {
        for format in [Format::Text, Format::Console] {
            for styled in [false, true] {
                let mut opts = options(format, width);
                opts.colors = styled;
                opts.links = styled;
                let text = output::render(&report, &opts).unwrap();
                for line in text.lines() {
                    assert!(
                        textwrap::core::display_width(line) <= width,
                        "width {width}, actual {}: {line:?}\n{text}",
                        textwrap::core::display_width(line)
                    );
                }
            }
        }
    }
}

#[test]
fn console_badges_and_clickable_compact_links_obey_render_options() {
    let mut report = fixture();
    for marker in ["FIXME", "BUG", "HACK", "XXX", "NOTE"] {
        let mut finding = report.findings[0].clone();
        finding.marker = marker.into();
        report.findings.push(finding);
    }
    report.counts = scanner::counts(&report.findings);
    let mut opts = options(Format::Console, 80);
    opts.colors = true;
    opts.links = true;
    let text = output::render(&report, &opts).unwrap();
    for style in ["1;97;44", "1;97;41", "1;30;43", "1;97;45", "1;30;46"] {
        assert!(text.contains(&format!("\x1b[{style}m")), "{text}");
    }
    let summary = text.split("\n. — ").next().unwrap();
    for (marker, style) in [
        ("TODO", "1;97;44"),
        ("FIXME", "1;97;41"),
        ("BUG", "1;97;41"),
        ("HACK", "1;30;43"),
        ("XXX", "1;97;45"),
        ("NOTE", "1;30;46"),
    ] {
        assert!(
            summary.contains(&format!("\x1b[{style}m{marker}: 1\x1b[0m")),
            "{summary}"
        );
    }
    let html = output::render(&report, &options(Format::Html, 80)).unwrap();
    let html_summary = html.split("</p>").next().unwrap();
    for (marker, class) in [
        ("TODO", "todo"),
        ("FIXME", "fixme"),
        ("HACK", "hack"),
        ("XXX", "xxx"),
        ("NOTE", "other"),
    ] {
        assert!(
            html_summary.contains(&format!(
                "<span class=\"marker marker-{class}\">{marker}: 1</span>"
            )),
            "{html_summary}"
        );
    }
    let mut source_url = reqwest::Url::from_file_path(
        std::path::Path::new(&opts.source_prefix).join(&report.findings[0].path),
    )
    .unwrap();
    source_url.set_fragment(Some("L1"));
    assert!(text.contains(&format!("\x1b]8;;{source_url}")));
    assert!(text.contains(
        "\x1b]8;;https://github.com/UXStream/uxstream/issues/2370\x1b\\uxstream/uxstream#2370"
    ));
    opts.colors = false;
    let links_only = output::render(&report, &opts).unwrap();
    assert!(!links_only.contains("\x1b["));
    assert!(links_only.contains("\x1b]8;;"));
    opts.links = false;
    assert!(!output::render(&report, &opts).unwrap().contains('\x1b'));
}

#[cfg(windows)]
#[test]
fn absolute_source_links_preserve_verbatim_drive_and_unc_paths() {
    let report = fixture();
    for (prefix, expected) in [
        (
            r"\\?\C:\repo with spaces",
            "file:///C:/repo%20with%20spaces/services/main-api/src/handlers.ts#L1",
        ),
        (
            r"\\?\UNC\server\share\repo",
            "file://server/share/repo/services/main-api/src/handlers.ts#L1",
        ),
    ] {
        let mut opts = options(Format::Html, 100);
        opts.absolute_source_links = true;
        opts.source_prefix = prefix.into();
        let text = output::render(&report, &opts).unwrap();
        assert!(text.contains(expected), "{text}");
    }
}

#[test]
fn wrapped_console_hyperlinks_close_before_newlines_and_spacer_rows() {
    let mut report = fixture();
    report.findings[0].path =
        "services/main-api/src/a long filename with Unicode å and many words.ts".into();
    let reference = &mut report.findings[0].references[0];
    reference.repository = Some("example/very-long-repository-name-for-wrapping".into());
    reference.url = Some(
        "https://github.com/example/very-long-repository-name-for-wrapping/issues/2370".into(),
    );
    let escapes = regex::Regex::new(r"\x1b\]8;;([^\x1b]*)\x1b\\|\x1b\[[0-9;]*m|\n").unwrap();
    for width in [20, 40, 80, 140] {
        for spacing in [0, 1, 2] {
            let mut opts = options(Format::Console, width);
            opts.line_spacing = spacing;
            opts.links = true;
            opts.colors = true;
            let text = output::render(&report, &opts).unwrap();
            let mut active = None;
            let mut linked = std::collections::BTreeMap::<&str, String>::new();
            let mut offset = 0;
            for capture in escapes.captures_iter(&text) {
                let token = capture.get(0).unwrap();
                if let Some(target) = active {
                    linked
                        .entry(target)
                        .or_default()
                        .push_str(&text[offset..token.start()]);
                }
                if token.as_str() == "\n" {
                    assert!(
                        active.is_none(),
                        "hyperlink remains open at a newline (width {width}, spacing {spacing}): {text:?}"
                    );
                } else if let Some(url) = capture.get(1) {
                    active = (!url.as_str().is_empty()).then_some(url.as_str());
                }
                offset = token.end();
            }
            assert_eq!(linked.len(), 2, "{text:?}");
            let compact = |label: &str| {
                label
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
            };
            for (url, label) in linked {
                assert!(
                    !label.contains(['│', '├', '└']),
                    "guides must not be clickable: {text:?}"
                );
                let expected = if url.starts_with("https:") {
                    assert_eq!(
                        url,
                        "https://github.com/example/very-long-repository-name-for-wrapping/issues/2370"
                    );
                    "example/very-long-repository-name-for-wrapping#2370"
                } else {
                    assert!(url.contains("/services/main-api/src/a%20long%20filename%20with%20Unicode%20%C3%A5%20and%20many%20words.ts#L1"));
                    "src/a long filename with Unicode å and many words.ts:1"
                };
                assert_eq!(compact(&label), compact(expected), "{text:?}");
            }
        }
    }
}

#[test]
fn markdown_suppressions_and_multiline_rendering_preserve_metadata() {
    let mut report = fixture();
    report.findings[0].message = "First <tag> & [literal]\n2. number with **syntax**".into();
    let text = output::render(&report, &options(Format::Markdown, 20)).unwrap();
    assert!(text.contains("<!-- markdownlint-disable MD013 MD024 MD033 -->"));
    assert!(text.contains("&lt;tag\\>"));
    assert!(text.contains("<br>2. number"));
    assert!(output::owns(&text));
    assert!(scanner::is_generated(&text));
    assert_eq!(output::metadata(&text).unwrap(), report.metadata);
    assert!(output::owns(
        &output::render(&report, &options(Format::Text, 20)).unwrap()
    ));
}

#[test]
fn console_line_spacing_is_uniform_across_headers_wrapping_details_and_summaries() {
    let mut report = fixture();
    let mut second = report.findings[0].clone();
    second.marker = "FIXME".into();
    second.line = 10;
    second.message = "Second item".into();
    second.references.clear();
    report.findings.push(second);
    let mut nested = report.components[0].clone();
    nested.id = "directory:services/main-api/worker".into();
    nested.root = "services/main-api/worker".into();
    let mut third = report.findings[1].clone();
    third.path = "services/main-api/worker/main.ts".into();
    third.component_id = Some(nested.id.clone());
    report.components.push(nested);
    report.findings.push(third);
    report.counts = scanner::counts(&report.findings);
    report.warn(
        "example",
        "A warning with enough words to wrap across multiple lines at a narrow console width.",
    );
    for detail in [Detail::Full, Detail::Summary] {
        for tree in [false, true] {
            for width in [60, 160] {
                let mut opts = options(Format::Console, width);
                opts.detail = detail;
                opts.line_spacing = 0;
                if !tree {
                    opts.group_by = GroupBy::File;
                }
                let compact = output::render(&report, &opts).unwrap();
                let content: Vec<_> = compact.lines().collect();
                let is_gap = |line: &str| line.chars().all(|c| c.is_whitespace() || c == '│');
                assert!(content.iter().all(|line| !is_gap(line)));
                for spacing in [1, 3] {
                    opts.line_spacing = spacing;
                    let text = output::render(&report, &opts).unwrap();
                    let lines: Vec<_> = text.lines().collect();
                    assert_eq!(
                        lines.len(),
                        content.len() + (content.len() - 1) * usize::from(spacing),
                        "{text}"
                    );
                    assert_eq!(
                        lines
                            .iter()
                            .filter(|line| !is_gap(line))
                            .copied()
                            .collect::<Vec<_>>(),
                        content
                    );
                    for group in lines.chunks(usize::from(spacing) + 1) {
                        assert!(!is_gap(group[0]));
                        assert!(group[1..].iter().all(|line| is_gap(line)), "{text}");
                    }
                    if tree && spacing == 1 {
                        assert!(text.contains("\n│\n└── services"), "{text}");
                        assert!(
                            text.contains("\n        │\n        ├── src/handlers.ts"),
                            "{text}"
                        );
                    }
                }
            }
        }
    }
    for format in [Format::Text, Format::Markdown, Format::Html, Format::Json] {
        let mut opts = options(format, 160);
        let original = output::render(&report, &opts).unwrap();
        opts.line_spacing = 3;
        assert_eq!(original, output::render(&report, &opts).unwrap());
    }
}

#[test]
fn console_line_spacing_preserves_comment_content_that_looks_like_tree_guides() {
    let mut report = fixture();
    report.findings[0].message = "Diagram\n│\n└─ work".into();
    let mut opts = options(Format::Console, 60);
    opts.group_by = GroupBy::File;
    let text = output::render(&report, &opts).unwrap();
    assert!(
        text.contains("    Diagram\n\n    │\n\n    └─ work"),
        "{text}"
    );
}
