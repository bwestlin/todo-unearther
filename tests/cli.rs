use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn run(root: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_todo-unearther"));
    command
        .current_dir(root)
        .env("NO_COLOR", "1")
        .env_remove("GITHUB_TOKEN");
    if args.first() == Some(&"config") {
        command
            .arg("--config")
            .arg(root.join(".todo-unearther.toml"));
    } else if args.first() != Some(&"completions") {
        command.arg(".");
    }
    command.args(args).output().unwrap()
}

fn ok(output: Output) -> Output {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn exports_all_formats_and_source_checks_without_git() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("main.rs"),
        "// TODO:(bwestlin) handle <input> & [links] #12\n",
    )
    .unwrap();
    let plain = ok(run(temp.path(), &[]));
    let plain = String::from_utf8(plain.stdout).unwrap();
    assert!(plain.contains("1 finding in 1 file"));
    assert!(!plain.contains('\x1b'));
    for (format, file) in [
        ("markdown", "TODO.md"),
        ("json", "report.json"),
        ("html", "report.html"),
        ("text", "report.txt"),
    ] {
        ok(run(temp.path(), &["--format", format, "--output", file]));
        let content = fs::read_to_string(temp.path().join(file)).unwrap();
        assert!(content.contains("bwestlin"));
        assert!(!content.contains('\x1b'));
        if format == "html" {
            assert!(content.contains("&lt;input&gt;"));
        }
        if format != "text" {
            ok(run(temp.path(), &["--check", "--output", file]));
        }
    }
    let original = fs::read(temp.path().join("TODO.md")).unwrap();
    // Even configured enrichment is unnecessary for source freshness.
    fs::write(temp.path().join(".todo-unearther.toml"), "[github]\nenabled=true\ngh_auth=false\napi_url='http://127.0.0.1:1'\n[git]\nattribution='history'\n").unwrap();
    let checked = Command::new(env!("CARGO_BIN_EXE_todo-unearther"))
        .current_dir(temp.path())
        .env("PATH", "")
        .env_remove("GITHUB_TOKEN")
        .args([".", "--check"])
        .output()
        .unwrap();
    ok(checked);
    assert_eq!(fs::read(temp.path().join("TODO.md")).unwrap(), original);
    fs::write(temp.path().join("main.rs"), "// TODO:(bwestlin) changed\n").unwrap();
    assert_eq!(run(temp.path(), &["--check"]).status.code(), Some(1));
}

#[test]
fn regenerating_is_deterministic_and_checks_never_write() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.py"), "# FIXME: broken\n").unwrap();
    ok(run(temp.path(), &["--output", "TODO.md"]));
    let first = fs::read(temp.path().join("TODO.md")).unwrap();
    ok(run(temp.path(), &["--output", "TODO.md"]));
    assert_eq!(fs::read(temp.path().join("TODO.md")).unwrap(), first);
    ok(run(temp.path(), &["--check", "--compare", "full"]));
    ok(run(temp.path(), &["--check", "--jobs", "1"]));
    // Source checks intentionally ignore manual edits to rendering.
    let edited = String::from_utf8(first)
        .unwrap()
        .replace("broken", "hand-edited");
    fs::write(temp.path().join("TODO.md"), &edited).unwrap();
    ok(run(temp.path(), &["--check"]));
    assert_eq!(
        run(temp.path(), &["--check", "--compare", "full"])
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("TODO.md")).unwrap(),
        edited
    );
}

#[test]
fn policies_and_overwrite_protection_have_distinct_exit_codes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.rs"), "// FIXME: needs reference\n").unwrap();
    fs::write(temp.path().join("TODO.md"), "handwritten").unwrap();
    assert_eq!(
        run(temp.path(), &["--output", "TODO.md"]).status.code(),
        Some(2)
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("TODO.md")).unwrap(),
        "handwritten"
    );
    let result = run(
        temp.path(),
        &["--output", "TODO.md", "--force", "--max-total", "0"],
    );
    assert_eq!(result.status.code(), Some(1));
    assert!(
        fs::read_to_string(temp.path().join("TODO.md"))
            .unwrap()
            .contains("Policy violations")
    );
    fs::write(
        temp.path().join(".todo-unearther.toml"),
        "[policy]\nrequire_references=['FIXME']\nmax_by_marker={FIXME=0}\n",
    )
    .unwrap();
    assert_eq!(run(temp.path(), &[]).status.code(), Some(1));
}

#[test]
fn lenient_enrichment_warns_and_strict_preserves_previous_output() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.rs"), "// TODO(#1): task\n").unwrap();
    fs::write(temp.path().join(".todo-unearther.toml"), "[github]\nenabled=true\ngh_auth=false\nrepository='org/repo'\napi_url='http://127.0.0.1:1'\ntimeout_seconds=1\n[git]\nattribution='blame'\n[policy]\nfail_on_closed=true\n").unwrap();
    let result = ok(run(temp.path(), &["--output", "TODO.md"]));
    assert!(String::from_utf8_lossy(&result.stderr).contains("warning"));
    let first = fs::read(temp.path().join("TODO.md")).unwrap();
    assert_eq!(
        run(temp.path(), &["--output", "TODO.md", "--strict"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(temp.path().join("TODO.md")).unwrap(), first);
    ok(run(temp.path(), &["--check", "--compare", "full"]));
    assert_eq!(
        run(temp.path(), &["--check", "--strict"]).status.code(),
        Some(2)
    );
}

#[test]
fn config_completions_filters_and_summary_work() {
    let temp = tempfile::tempdir().unwrap();
    ok(run(temp.path(), &["config", "init"]));
    assert_eq!(run(temp.path(), &["config", "init"]).status.code(), Some(2));
    let completion = ok(run(temp.path(), &["completions", "bash"]));
    assert!(String::from_utf8_lossy(&completion.stdout).contains("todo-unearther"));
    fs::write(
        temp.path().join("a.rs"),
        "// NOTE: chosen\n// TODO: ignored\n",
    )
    .unwrap();
    for group in ["marker", "file"] {
        let result = ok(run(
            temp.path(),
            &[
                "--markers",
                "NOTE",
                "--group-by",
                group,
                "--detail",
                "summary",
                "--format",
                "json",
            ],
        ));
        let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(value["findings"].as_array().unwrap().len(), 1);
        assert_eq!(value["findings"][0]["message"], "chosen");
    }
    let excluded = ok(run(temp.path(), &["--exclude", "*.rs", "--format", "json"]));
    let value: serde_json::Value = serde_json::from_slice(&excluded.stdout).unwrap();
    assert!(value["findings"].as_array().unwrap().is_empty());
    assert_eq!(run(temp.path(), &["--jobs", "0"]).status.code(), Some(2));
}

#[test]
fn width_config_and_overrides_wrap_piped_output_and_protect_generated_text() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.rs"), "// TODO:(person) This deliberately long comment should wrap with Unicode 界面 and preserve all of its words.\n// Continue at the same indentation.\n").unwrap();
    fs::write(
        temp.path().join(".todo-unearther.toml"),
        "[report]\nwidth=60\n",
    )
    .unwrap();
    for (args, width) in [
        (vec!["--format", "text"], 60),
        (
            vec!["--format", "console", "--links", "enabled", "--width", "40"],
            40,
        ),
    ] {
        let output = ok(run(temp.path(), &args));
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.lines()
                .all(|line| textwrap::core::display_width(line) <= width),
            "{text}"
        );
        assert!(
            !text.contains('\x1b'),
            "piped output must never contain escapes"
        );
        assert!(text.contains("Continue at the same indentation."));
    }
    for _ in 0..2 {
        ok(run(
            temp.path(),
            &["--output", "report.txt", "--width", "20"],
        ));
    }
    ok(run(
        temp.path(),
        &[
            "--check",
            "--output",
            "report.txt",
            "--compare",
            "full",
            "--width",
            "20",
        ],
    ));
    assert_eq!(run(temp.path(), &["--width", "19"]).status.code(), Some(2));
    fs::write(
        temp.path().join(".todo-unearther.toml"),
        "[report]\nwidth=19\n",
    )
    .unwrap();
    assert_eq!(run(temp.path(), &[]).status.code(), Some(2));
}

#[test]
fn line_spacing_defaults_to_one_and_cli_overrides_repository_configuration() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("a.rs"),
        "// TODO: first\n// TODO: second\n",
    )
    .unwrap();
    let text = String::from_utf8(ok(run(temp.path(), &["--format", "console"])).stdout).unwrap();
    assert!(text.contains("— first\n\n  TODO"), "{text}");
    assert!(
        text.starts_with("todo-unearther report\n\n2 findings"),
        "{text}"
    );
    fs::write(
        temp.path().join(".todo-unearther.toml"),
        "[report]\nline_spacing=2\n",
    )
    .unwrap();
    let text = String::from_utf8(ok(run(temp.path(), &["--format", "console"])).stdout).unwrap();
    assert!(text.contains("— first\n\n\n  TODO"), "{text}");
    let compact = String::from_utf8(
        ok(run(
            temp.path(),
            &["--format", "console", "--line-spacing", "0"],
        ))
        .stdout,
    )
    .unwrap();
    assert!(compact.contains("— first\n  TODO"), "{compact}");
    assert_eq!(
        run(temp.path(), &["--line-spacing=-1"]).status.code(),
        Some(2)
    );
    for _ in 0..2 {
        ok(run(
            temp.path(),
            &[
                "--format",
                "console",
                "--output",
                "report.txt",
                "--width",
                "20",
            ],
        ));
    }
    let exported = fs::read_to_string(temp.path().join("report.txt")).unwrap();
    assert!(
        exported.starts_with("todo-unearther\n\n\nreport\n"),
        "{exported}"
    );
}

#[cfg(unix)]
#[test]
fn refuses_output_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("original"), "keep").unwrap();
    std::os::unix::fs::symlink(temp.path().join("original"), temp.path().join("TODO.md")).unwrap();
    assert_eq!(
        run(temp.path(), &["--output", "TODO.md", "--force"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("original")).unwrap(),
        "keep"
    );
}
