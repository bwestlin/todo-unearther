use clap::Parser;
use std::{fs, path::PathBuf};
use todo_unearther::{app, cli::Cli, preview};

#[test]
fn preview_arguments_conflict_with_explicit_exports_and_checks() {
    assert!(Cli::try_parse_from(["tool", "--open"]).unwrap().open);
    for conflict in [
        vec!["--format", "html"],
        vec!["--output", "report.html"],
        vec!["--check"],
    ] {
        let mut args = vec!["tool", "--open"];
        args.extend(conflict);
        assert!(Cli::try_parse_from(args).is_err());
    }
    assert!(Cli::try_parse_from(["tool", "--component-group-by", "component"]).is_err());
}

#[test]
fn retained_preview_survives_return_and_opener_failure_reports_the_path() {
    let path = preview::show("<!doctype html><title>Retained</title>", |path| {
        assert_eq!(path.extension().unwrap(), "html");
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("todo-unearther-")
        );
        assert!(fs::read_to_string(path)?.contains("Retained"));
        Ok(())
    })
    .unwrap();
    assert!(fs::read_to_string(&path).unwrap().contains("Retained"));
    fs::remove_file(path).unwrap();
    let mut failed_path = PathBuf::new();
    let error = preview::show("saved", |path| {
        failed_path = path.to_path_buf();
        anyhow::bail!("synthetic desktop failure")
    })
    .unwrap_err();
    assert!(error.to_string().contains(failed_path.to_str().unwrap()));
    assert_eq!(fs::read_to_string(&failed_path).unwrap(), "saved");
    fs::remove_file(failed_path).unwrap();
}

#[test]
fn open_ignores_configured_exports_and_uses_encoded_absolute_links() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    let source = temp.path().join("src/a space#å.rs");
    fs::write(&source, "// TODO: preview <input>\n").unwrap();
    fs::write(
        temp.path().join("package.json"),
        r#"{"name":"@demo/preview"}"#,
    )
    .unwrap();
    let export = temp.path().join("existing.md");
    fs::write(&export, "preserve me").unwrap();
    let config = temp.path().join("config.toml");
    fs::write(&config, "[report]\nformat='json'\noutput='existing.md'\ngroup_by='component'\ncomponent_group_by='file'\ncomponent_layout='tree'\n[policy]\nmax_total=0\n").unwrap();
    let cli = Cli::try_parse_from([
        "tool",
        temp.path().to_str().unwrap(),
        "--config",
        config.to_str().unwrap(),
        "--open",
    ])
    .unwrap();
    let mut preview_path = None;
    let code = app::run_with_opener(cli, |path| {
        let html = fs::read_to_string(path)?;
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("@demo/preview"));
        assert!(html.contains("<h2>. — @demo/preview [npm]</h2>"));
        assert!(html.contains("<h3>src/a space#å.rs</h3>"));
        let expected = reqwest::Url::from_file_path(&source).unwrap();
        assert!(
            html.contains(&format!("{expected}#L1")),
            "expected encoded file URL {expected} in {html}"
        );
        assert!(html.contains("&lt;input&gt;"));
        assert!(html.contains("max_total"));
        preview_path = Some(path.to_path_buf());
        Ok(())
    })
    .unwrap();
    assert_eq!(code, 1);
    assert_eq!(fs::read_to_string(&export).unwrap(), "preserve me");
    fs::remove_file(preview_path.unwrap()).unwrap();
}

#[test]
fn strict_component_failure_prevents_preview_creation() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("package.json"), "invalid json").unwrap();
    fs::write(temp.path().join("main.rs"), "// TODO: task\n").unwrap();
    let cli = Cli::try_parse_from([
        "tool",
        temp.path().to_str().unwrap(),
        "--group-by",
        "component",
        "--open",
        "--strict",
    ])
    .unwrap();
    let mut called = false;
    let result = app::run_with_opener(cli, |_| {
        called = true;
        Ok(())
    });
    assert!(result.is_err());
    assert!(!called);
}
