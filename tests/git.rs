use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn git(root: &Path, args: &[&str], author: &str) {
    let result = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "commit.gpgsign=false", "-c", "core.hooksPath="])
        .env("GIT_AUTHOR_NAME", author)
        .env(
            "GIT_AUTHOR_EMAIL",
            format!("{}@example.test", author.to_lowercase()),
        )
        .env("GIT_COMMITTER_NAME", author)
        .env(
            "GIT_COMMITTER_EMAIL",
            format!("{}@example.test", author.to_lowercase()),
        )
        .env("GIT_AUTHOR_DATE", "2026-10-01T12:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-10-01T12:00:00Z")
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn report(root: &Path, mode: &str) -> (Output, serde_json::Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_todo-unearther"))
        .current_dir(root)
        .args(["--format", "json", "--attribution", mode])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = serde_json::from_slice(&output.stdout).unwrap();
    (output, json)
}

#[test]
fn blame_and_history_distinguish_authors_and_handle_renames_and_dirty_lines() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-b", "main"], "Alice");
    fs::write(
        temp.path().join("a.rs"),
        "// TODO:(owner) original task\nfn main() {}\n",
    )
    .unwrap();
    git(temp.path(), &["add", "."], "Alice");
    git(temp.path(), &["commit", "-m", "initial task"], "Alice");
    fs::write(
        temp.path().join("a.rs"),
        "// TODO:(owner) updated task\nfn main() {}\n",
    )
    .unwrap();
    git(temp.path(), &["commit", "-am", "update task"], "Bob");
    git(temp.path(), &["mv", "a.rs", "renamed.rs"], "Bob");
    git(temp.path(), &["commit", "-m", "rename"], "Bob");
    let (_, json) = report(temp.path(), "history");
    let attribution = &json["findings"][0]["attribution"];
    assert_eq!(json["findings"][0]["annotation"], "owner");
    assert_eq!(attribution["blame"]["name"], "Bob");
    assert_eq!(attribution["origin"]["name"], "Alice");
    assert_eq!(attribution["origin_status"], "inferred");
    fs::write(
        temp.path().join("renamed.rs"),
        "// TODO:(owner) dirty task\nfn main() {}\n",
    )
    .unwrap();
    let (_, json) = report(temp.path(), "history");
    assert_eq!(
        json["findings"][0]["attribution"]["origin_status"],
        "uncommitted"
    );
}

#[test]
fn shallow_history_is_explicit_and_never_deepened() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let clone = temp.path().join("clone");
    fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"], "Alice");
    fs::write(source.join("a.py"), "# TODO: original\n").unwrap();
    git(&source, &["add", "."], "Alice");
    git(&source, &["commit", "-m", "initial"], "Alice");
    fs::write(source.join("a.py"), "# TODO: edited\n").unwrap();
    git(&source, &["commit", "-am", "edit"], "Bob");
    let url = reqwest::Url::from_directory_path(&source).unwrap();
    git(
        temp.path(),
        &[
            "clone",
            "--depth",
            "1",
            url.as_str(),
            clone.to_str().unwrap(),
        ],
        "Alice",
    );
    let (output, json) = report(&clone, "history");
    assert_eq!(
        json["findings"][0]["attribution"]["origin_status"],
        "incomplete"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("shallow"));
    assert!(clone.join(".git/shallow").exists());
}

#[test]
fn moves_copies_merges_and_limits_remain_bounded() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-b", "main"], "Alice");
    let comment =
        "// TODO: preserve this sufficiently long comment when moving it between functions\n";
    fs::write(
        temp.path().join("a.rs"),
        format!("{comment}fn a() {{}}\nfn b() {{}}\n"),
    )
    .unwrap();
    git(temp.path(), &["add", "."], "Alice");
    git(temp.path(), &["commit", "-m", "initial"], "Alice");
    git(temp.path(), &["checkout", "-b", "topic"], "Bob");
    fs::write(
        temp.path().join("a.rs"),
        format!("fn a() {{}}\nfn b() {{}}\n{comment}"),
    )
    .unwrap();
    fs::write(temp.path().join("copied.rs"), comment).unwrap();
    git(temp.path(), &["add", "."], "Bob");
    git(temp.path(), &["commit", "-m", "move and copy"], "Bob");
    git(temp.path(), &["checkout", "main"], "Bob");
    git(
        temp.path(),
        &["merge", "--no-ff", "topic", "-m", "merge topic"],
        "Bob",
    );
    let (_, json) = report(temp.path(), "blame");
    let findings = json["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0]["attribution"]["blame"]["name"], "Alice");
    let output = Command::new(env!("CARGO_BIN_EXE_todo-unearther"))
        .current_dir(temp.path())
        .args([
            "--format",
            "json",
            "--attribution",
            "history",
            "--git-detect-copies",
            "true",
            "--history-limit",
            "1",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        json["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["attribution"].is_object())
    );
}
