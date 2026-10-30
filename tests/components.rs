use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use todo_unearther::{
    components::{Discovery, SHARED_ID},
    config::Config,
    model::*,
    scanner,
};

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn run(root: &Path, args: &[&str]) -> Output {
    // There is intentionally no cargo, npm, Python, Go, Maven, Gradle, dotnet, or Git on PATH.
    Command::new(env!("CARGO_BIN_EXE_todo-unearther"))
        .current_dir(root)
        .arg(root)
        .args(args)
        .env("PATH", "")
        .env_remove("GITHUB_TOKEN")
        .output()
        .unwrap()
}

fn report(root: &Path, extra: &[&str]) -> ScanReport {
    let mut args = vec!["--group-by", "component", "--format", "json"];
    args.extend(extra);
    let output = run(root, &args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn owner<'a>(report: &'a ScanReport, path: &str) -> &'a Component {
    let finding = report
        .findings
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("missing finding {path}"));
    report
        .components
        .iter()
        .find(|c| Some(&c.id) == finding.component_id.as_ref())
        .unwrap()
}

fn names(component: &Component) -> Vec<&str> {
    component
        .identities
        .iter()
        .map(|i| i.name.as_str())
        .collect()
}

#[test]
fn detects_all_ecosystems_and_shared_files_without_build_tools() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Cargo.toml", "[workspace]\nmembers=['rust']\n");
    write(root, "README.md", "TODO: shared work\n");
    write(root, "rust/Cargo.toml", "[package]\nname='native-api'\n");
    write(root, "rust/src/lib.rs", "// TODO: rust\n");
    write(
        root,
        "web/package.json",
        r#"{"name":"@acme/web","private":true,"workspaces":["nested/*"]}"#,
    );
    write(root, "web/src/index.ts", "// TODO: web\n");
    write(
        root,
        "web/nested/lib/package.json",
        r#"{"name":"@acme/lib"}"#,
    );
    write(root, "web/nested/lib/index.js", "// TODO: library\n");
    write(
        root,
        "python/pyproject.toml",
        "[project]\nname='modern'\n[tool.poetry]\nname='old-poetry'\n",
    );
    write(
        root,
        "python/setup.cfg",
        "[metadata]\nname = old-setuptools\n",
    );
    write(
        root,
        "python/setup.py",
        "raise RuntimeError('must never execute')\n",
    );
    write(root, "python/main.py", "# TODO: python\n");
    write(
        root,
        "poetry/pyproject.toml",
        "[tool.poetry]\nname='poetry-only'\n",
    );
    write(root, "poetry/main.py", "# TODO: poetry\n");
    write(root, "legacy/setup.cfg", "[metadata]\nname = legacy-name\n");
    write(root, "legacy/main.py", "# TODO: legacy\n");
    write(
        root,
        "setup/setup.py",
        "raise RuntimeError('do not execute')\n",
    );
    write(root, "setup/main.py", "# TODO: setup\n");
    write(
        root,
        "tools/pyproject.toml",
        "[tool.ruff]\nline-length=100\n",
    );
    write(root, "tools/check.py", "# TODO: repository tooling\n");
    write(root, "go/go.work", "go 1.25\nuse ./module\n");
    write(
        root,
        "go/module/go.mod",
        "// module misleading\nmodule (\n \"example.test/api/v2\"\n)\n",
    );
    write(root, "go/module/main.go", "package main\n// TODO: go\n");
    write(
        root,
        "jvm/pom.xml",
        r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><parent><artifactId>parent</artifactId></parent><artifactId>jvm-api</artifactId><dependencies><dependency><artifactId>dependency</artifactId></dependency></dependencies></project>"#,
    );
    write(root, "jvm/src/Main.java", "// TODO: java\n");
    write(
        root,
        "gradle/settings.gradle.kts",
        "rootProject.name = \"jvm-build\"\ninclude(\"service\", \"deep:child\")\nproject(\":service\").projectDir = file(\"custom/service\")\nproject(\":service\").name = \"renamed-service\"\n",
    );
    write(
        root,
        "gradle/custom/service/Main.kt",
        "// TODO: remapped project without build script\n",
    );
    write(root, "gradle/deep/child/build.gradle.kts", "plugins {}\n");
    write(
        root,
        "gradle/deep/child/Main.kt",
        "// TODO: nested project\n",
    );
    write(root, "solution.sln", "a workspace, not a component\n");
    write(
        root,
        "dotnet/Company.Web.csproj",
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><AssemblyName>NotTheProjectName</AssemblyName></PropertyGroup></Project>",
    );
    write(
        root,
        "dotnet/Company.Tools.fsproj",
        "<Project Sdk=\"Microsoft.NET.Sdk\" />",
    );
    write(root, "dotnet/Program.cs", "// TODO: dotnet\n");
    write(root, "vb/Legacy.vbproj", "<Project />");
    write(root, "vb/README.md", "TODO: vb project\n");
    let report = report(root, &[]);
    assert_eq!(report.findings.len(), 15);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(owner(&report, "README.md").id, SHARED_ID);
    assert_eq!(owner(&report, "tools/check.py").id, SHARED_ID);
    assert!(names(owner(&report, "rust/src/lib.rs")).contains(&"native-api"));
    assert!(names(owner(&report, "web/nested/lib/index.js")).contains(&"@acme/lib"));
    assert_eq!(
        names(owner(&report, "python/main.py")),
        ["modern", "modern", "modern"]
    );
    assert!(names(owner(&report, "poetry/main.py")).contains(&"poetry-only"));
    assert!(names(owner(&report, "legacy/main.py")).contains(&"legacy-name"));
    assert_eq!(
        owner(&report, "setup/main.py").status,
        ComponentStatus::Inferred
    );
    assert!(names(owner(&report, "go/module/main.go")).contains(&"example.test/api/v2"));
    assert_eq!(names(owner(&report, "jvm/src/Main.java")), ["jvm-api"]);
    assert_eq!(
        names(owner(&report, "gradle/custom/service/Main.kt")),
        ["renamed-service"]
    );
    assert!(
        names(owner(&report, "gradle/deep/child/Main.kt"))
            .iter()
            .all(|n| *n == "child")
    );
    assert_eq!(
        names(owner(&report, "dotnet/Program.cs")),
        ["Company.Tools", "Company.Web"]
    );
    assert_eq!(names(owner(&report, "vb/README.md")), ["Legacy"]);
}

#[test]
fn co_located_and_duplicate_names_keep_unique_directory_ownership() {
    let temp = tempfile::tempdir().unwrap();
    for dir in ["one", "two"] {
        write(
            temp.path(),
            &format!("{dir}/Cargo.toml"),
            "[package]\nname='api'\n",
        );
        write(
            temp.path(),
            &format!("{dir}/package.json"),
            r#"{"name":"api"}"#,
        );
        write(
            temp.path(),
            &format!("{dir}/main.rs"),
            "// TODO(#1): first #2\n// FIXME: second\n",
        );
    }
    let report = report(temp.path(), &[]);
    assert_eq!(report.findings.len(), 4);
    assert_eq!(report.components.len(), 2);
    assert!(report.components.iter().all(|c| c.identities.len() == 2));
    assert_ne!(
        owner(&report, "one/main.rs").id,
        owner(&report, "two/main.rs").id
    );
    for format in ["text", "console", "markdown", "html"] {
        for inner in ["marker", "file"] {
            let output = run(
                temp.path(),
                &[
                    "--group-by",
                    "component",
                    "--component-group-by",
                    inner,
                    "--format",
                    format,
                ],
            );
            assert!(output.status.success());
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(text.contains("one — "));
            assert!(text.contains("two — "));
            assert!(text.contains("4 findings in 2 files"));
            if format == "markdown" {
                assert!(text.contains("### "));
            }
            if format == "html" {
                assert_eq!(text.matches("<section class=\"component\">").count(), 2);
                assert_eq!(
                    text.matches("<section").count(),
                    text.matches("</section>").count()
                );
            }
            if format == "text" {
                assert_eq!(text.matches("\n2 findings\n").count(), 2);
            }
        }
    }
}

#[test]
fn folder_first_headings_and_path_order_are_independent_of_package_names() {
    let temp = tempfile::tempdir().unwrap();
    for (folder, name) in [
        ("z", "aaa"),
        ("a", "zzz"),
        ("a/nested", "middle"),
        ("a-sibling", "first"),
    ] {
        write(
            temp.path(),
            &format!("{folder}/package.json"),
            &format!(r#"{{"name":"{name}"}}"#),
        );
        write(temp.path(), &format!("{folder}/main.ts"), "// TODO: task\n");
    }
    let inventory = report(temp.path(), &[]);
    assert_eq!(
        inventory
            .components
            .iter()
            .map(|c| c.root.as_str())
            .collect::<Vec<_>>(),
        ["a", "a/nested", "a-sibling", "z"]
    );
    for format in ["console", "text", "markdown", "html"] {
        let result = run(
            temp.path(),
            &["--group-by", "component", "--format", format],
        );
        assert!(result.status.success());
        let text = String::from_utf8(result.stdout).unwrap();
        let headings = [
            "a — zzz",
            "a/nested — middle",
            "a-sibling — first",
            "z — aaa",
        ];
        let positions: Vec<_> = headings
            .iter()
            .map(|heading| text.find(heading).unwrap())
            .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
    }
}

#[test]
fn tree_layout_preserves_directory_hierarchy_unique_totals_and_inner_groups() {
    let temp = tempfile::tempdir().unwrap();
    for folder in ["packages/api", "packages/api/nested", "packages/cache"] {
        write(
            temp.path(),
            &format!("{folder}/package.json"),
            r#"{"name":"same"}"#,
        );
        write(
            temp.path(),
            &format!("{folder}/main.ts"),
            "// TODO(#1): first #2\n// FIXME: second\n",
        );
    }
    write(temp.path(), "shared.rs", "// TODO: shared\n");
    for format in ["console", "text", "markdown", "html", "json"] {
        for detail in ["summary", "full"] {
            for inner in ["file", "marker"] {
                let result = run(
                    temp.path(),
                    &[
                        "--group-by",
                        "component",
                        "--component-layout",
                        "tree",
                        "--component-group-by",
                        inner,
                        "--detail",
                        detail,
                        "--format",
                        format,
                    ],
                );
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                let text = String::from_utf8(result.stdout).unwrap();
                if format == "json" {
                    let report: ScanReport = serde_json::from_str(&text).unwrap();
                    assert_eq!(report.findings.len(), 7);
                    assert_eq!(report.components.len(), 4);
                    continue;
                }
                assert!(text.contains("7 findings in 4 files"));
                assert!(text.contains("Repository / shared"));
                if detail == "full" {
                    assert_eq!(text.matches("first").count(), 3);
                    assert_eq!(text.matches("second").count(), 3);
                }
                if format == "text" || format == "console" {
                    let tree = text.split_once("\n. — ").unwrap().1.trim_end();
                    // Full reports may contain blank item gaps in the final
                    // leaf, where there are no remaining branches to continue.
                    if detail == "summary" && format == "text" {
                        assert!(
                            tree.lines().all(|line| !line.trim().is_empty()),
                            "tree branches should continue across separators: {text}"
                        );
                    }
                    assert!(tree.lines().any(|line| line == "│"), "{text}");
                    assert!(tree.lines().any(|line| line == "    │"), "{text}");
                    assert!(tree.lines().any(|line| line == "    │   │"), "{text}");
                    assert!(text.lines().all(|line| line.trim_end() == line), "{text}");
                    assert!(text.contains("└── packages — 6 findings"), "{text}");
                    assert!(
                        text.contains("    ├── api — same [npm] — 4 findings"),
                        "{text}"
                    );
                    assert!(
                        text.contains("    │   └── nested — same [npm] — 2 findings"),
                        "{text}"
                    );
                    assert!(
                        text.contains("    └── cache — same [npm] — 2 findings"),
                        "{text}"
                    );
                } else if format == "markdown" {
                    assert!(text.contains("  - **packages** — 6 findings"), "{text}");
                    assert!(
                        text.contains("    - **api — same \\[npm\\]** — 4 findings"),
                        "{text}"
                    );
                    assert!(
                        text.contains("      - **nested — same \\[npm\\]** — 2 findings"),
                        "{text}"
                    );
                } else if format == "html" {
                    assert!(
                        text.contains("<h3>packages</h3><p class=\"muted\">6 findings</p>"),
                        "{text}"
                    );
                    assert!(text.contains("<h4>api — same [npm]</h4>"), "{text}");
                    assert!(text.contains("<h5>nested — same [npm]</h5>"), "{text}");
                    assert_eq!(
                        text.matches("<section").count(),
                        text.matches("</section>").count()
                    );
                }
            }
        }
    }
}

#[test]
fn tree_layout_configuration_freshness_and_noncomponent_compatibility() {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "package.json", r#"{"name":"root"}"#);
    write(temp.path(), "src/main.ts", "// TODO: task\n");
    write(
        temp.path(),
        ".todo-unearther.toml",
        "[report]\ngroup_by='component'\ncomponent_layout='tree'\ncomponent_group_by='file'\n",
    );
    let tree = run(temp.path(), &["--format", "text"]);
    assert!(
        String::from_utf8(tree.stdout)
            .unwrap()
            .contains(". — root [npm] — 1 finding\n└── src/main.ts")
    );
    assert!(run(temp.path(), &["--output", "TODO.md"]).status.success());
    assert!(run(temp.path(), &["--check"]).status.success());
    assert_eq!(
        run(temp.path(), &["--check", "--component-layout", "flat"])
            .status
            .code(),
        Some(1)
    );
    let flat_json = report(temp.path(), &["--component-layout", "flat"]);
    let tree_json = report(temp.path(), &[]);
    assert_eq!(flat_json.findings, tree_json.findings);
    assert_eq!(flat_json.components, tree_json.components);
    assert_ne!(
        flat_json.metadata.source_fingerprint,
        tree_json.metadata.source_fingerprint
    );
    let before = run(
        temp.path(),
        &[
            "--group-by",
            "file",
            "--format",
            "json",
            "--component-layout",
            "flat",
        ],
    );
    let after = run(
        temp.path(),
        &[
            "--group-by",
            "file",
            "--format",
            "json",
            "--component-layout",
            "tree",
        ],
    );
    assert_eq!(before.stdout, after.stdout);
    assert!(
        run(temp.path(), &["--component-layout", "invalid"])
            .status
            .code()
            == Some(2)
    );
}

#[test]
fn manifests_are_cached_even_without_markers_and_filtered_metadata_is_read_once() {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "package.json", r#"{"name":"root"}"#);
    for i in 0..100 {
        write(temp.path(), &format!("src/item{i}.ts"), "// TODO: task\n");
    }
    let config = Config::default();
    let discovery = Discovery::new(temp.path(), &config.components, config.scan.max_file_size);
    let scan =
        scanner::scan_with_components(temp.path(), &config.scan, None, Some(&discovery)).unwrap();
    let mut report = ScanReport {
        metadata: Metadata {
            generator: GENERATOR.into(),
            schema_version: SCHEMA_VERSION,
            source_fingerprint: String::new(),
        },
        counts: scanner::counts(&scan.findings),
        findings: scan.findings,
        stats: scan.stats,
        diagnostics: vec![],
        violations: vec![],
        components: vec![],
    };
    assert!(discovery.assign(&mut report));
    assert_eq!(discovery.stats().manifests_read, 1);
    assert_eq!(discovery.stats().additional_reads, 0);
    assert_eq!(discovery.stats().directories_read, 2);
    assert!(discovery.stats().cache_hits > 0);
    let report = self::report(temp.path(), &["--include", "src/**"]);
    assert_eq!(report.findings.len(), 100);
    assert_eq!(report.components.len(), 1);
    assert!(names(&report.components[0]).contains(&"root"));
}

#[test]
fn malformed_metadata_warns_or_fails_strict_and_existing_report_is_preserved() {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "broken/package.json", "not valid json");
    write(temp.path(), "broken/main.ts", "// TODO: task\n");
    let report = report(temp.path(), &[]);
    assert_eq!(
        owner(&report, "broken/main.ts").status,
        ComponentStatus::Unresolved
    );
    assert!(!report.diagnostics.is_empty());
    write(temp.path(), "TODO.md", "keep this file");
    let result = run(
        temp.path(),
        &["--group-by", "component", "--strict", "--output", "TODO.md"],
    );
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(
        fs::read_to_string(temp.path().join("TODO.md")).unwrap(),
        "keep this file"
    );
}

#[test]
fn malformed_and_computed_gradle_metadata_are_distinguished() {
    let temp = tempfile::tempdir().unwrap();
    write(
        temp.path(),
        "settings.gradle.kts",
        "rootProject.name = \"literal-root\"\ninclude(computedProjects())\n",
    );
    write(temp.path(), "src/Main.kt", "// TODO: task\n");
    assert_eq!(
        report(temp.path(), &[]).components[0].status,
        ComponentStatus::Inferred
    );
    write(
        temp.path(),
        "settings.gradle.kts",
        "rootProject.name = \"unterminated",
    );
    let report = report(temp.path(), &[]);
    assert_eq!(report.components[0].status, ComponentStatus::Unresolved);
    assert!(report.diagnostics[0].message.contains("unterminated"));
}

#[cfg(unix)]
#[test]
fn unreadable_filtered_manifests_keep_an_unresolved_directory_boundary() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "package.json", r#"{"name":"private"}"#);
    write(temp.path(), "src/index.ts", "// TODO: task\n");
    let path = temp.path().join("package.json");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    // Some development environments run tests as root; a read-only fixture cannot deny root.
    if fs::File::open(&path).is_err() {
        let report = report(temp.path(), &["--include", "src/**"]);
        assert_eq!(report.components[0].status, ComponentStatus::Unresolved);
        assert!(!report.diagnostics.is_empty());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn component_freshness_tracks_names_only_when_component_detection_is_requested() {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "sample.rs", "// TODO:(person) fix #12\n");
    let output = run(temp.path(), &["--format", "json"]);
    assert!(output.status.success());
    let legacy: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        legacy["metadata"]["source_fingerprint"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert!(legacy.get("components").is_none());
    assert!(legacy["findings"][0].get("component_id").is_none());
    let _: ScanReport = serde_json::from_slice(&output.stdout).unwrap();
    write(temp.path(), "package.json", r#"{"name":"before"}"#);
    assert!(
        run(
            temp.path(),
            &["--group-by", "component", "--output", "TODO.md"]
        )
        .status
        .success()
    );
    assert!(
        run(temp.path(), &["--group-by", "component", "--check"])
            .status
            .success()
    );
    write(temp.path(), "package.json", r#"{"name":"after"}"#);
    assert_eq!(
        run(temp.path(), &["--group-by", "component", "--check"])
            .status
            .code(),
        Some(1)
    );
    let unchanged = run(temp.path(), &["--format", "json"]);
    let unchanged: serde_json::Value = serde_json::from_slice(&unchanged.stdout).unwrap();
    assert_eq!(
        unchanged["metadata"]["source_fingerprint"],
        legacy["metadata"]["source_fingerprint"]
    );
}

#[test]
fn ecosystem_selection_scan_boundaries_and_unnamed_packages_are_explicit() {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "Cargo.toml", "[package]\nname='root'\n");
    write(temp.path(), "nested/package.json", "{}");
    write(temp.path(), "nested/main.ts", "// TODO: task\n");
    write(temp.path(), "nested/deeper/main.ts", "// TODO: other\n");
    let initial = report(temp.path(), &[]);
    assert!(names(owner(&initial, "nested/main.ts")).contains(&"nested"));
    let scoped = report(&temp.path().join("nested/deeper"), &[]);
    assert_eq!(scoped.components[0].id, SHARED_ID);
    write(
        temp.path(),
        ".todo-unearther.toml",
        "[components]\necosystems=['cargo']\n",
    );
    let selected = report(temp.path(), &[]);
    assert_eq!(owner(&selected, "nested/main.ts").root, ".");
    assert!(names(owner(&selected, "nested/main.ts")).contains(&"root"));
}

#[test]
fn gradle_groovy_literals_and_dynamic_fallbacks_do_not_evaluate_scripts() {
    let temp = tempfile::tempdir().unwrap();
    write(
        temp.path(),
        "settings.gradle",
        "rootProject.name = computeName()\n// include 'fake'\ndef x = \"include('fake')\"\ninclude 'app', 'nested:lib'\nproject(':app').name = dynamicName()\ninclude 'outside'\nproject(':outside').projectDir = file('../outside-the-scan')\n",
    );
    write(temp.path(), "app/code.java", "// TODO: app\n");
    write(temp.path(), "nested/lib/code.java", "// TODO: nested\n");
    write(
        temp.path(),
        "fake/code.java",
        "// TODO: not a project declaration\n",
    );
    let report = report(temp.path(), &[]);
    assert_eq!(
        owner(&report, "app/code.java").status,
        ComponentStatus::Inferred
    );
    assert_eq!(owner(&report, "nested/lib/code.java").root, "nested/lib");
    assert_eq!(owner(&report, "fake/code.java").root, ".");
    assert_eq!(report.findings.len(), 3);
}

#[cfg(unix)]
#[test]
fn symlinks_and_nested_repositories_do_not_expand_component_discovery() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "package.json", r#"{"name":"external"}"#);
    write(outside.path(), "main.ts", "// TODO: external\n");
    std::os::unix::fs::symlink(outside.path(), temp.path().join("external")).unwrap();
    write(temp.path(), "nested/.git/HEAD", "ref: refs/heads/main\n");
    write(
        temp.path(),
        "nested/package.json",
        r#"{"name":"nested-repository"}"#,
    );
    write(temp.path(), "nested/main.ts", "// TODO: nested\n");
    write(temp.path(), "main.ts", "// TODO: local\n");
    std::os::unix::fs::symlink(
        outside.path().join("package.json"),
        temp.path().join("package.json"),
    )
    .unwrap();
    let report = report(temp.path(), &[]);
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.components[0].id, SHARED_ID);
}
