use std::{
    fs,
    path::Path,
    process::{Command, Output},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};
use tiny_http::{Response, Server, StatusCode};
use todo_unearther::{config::GithubConfig, github, model::ReferenceStatus, references};

fn issue(title: &str, state: &str, pr: bool) -> String {
    serde_json::json!({"title": title, "state": state, "html_url": "https://github.com/org/repo/issues/12", "labels": [{"name":"bug"}], "assignees":[{"login":"alice"}], "pull_request": if pr { Some(serde_json::json!({"url":"example"})) } else { None }}).to_string()
}

fn server(responses: Vec<(u16, String)>) -> (String, thread::JoinHandle<Vec<String>>) {
    let server = Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr());
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for (code, body) in responses {
            let request = server
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .expect("expected GitHub request");
            assert_eq!(request.method().as_str(), "GET");
            requests.push(request.url().to_owned());
            request
                .respond(Response::from_string(body).with_status_code(StatusCode(code)))
                .unwrap();
        }
        requests
    });
    (url, handle)
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_todo-unearther"))
        .current_dir(root)
        .env_remove("GITHUB_TOKEN")
        .arg(".")
        .args(args)
        .output()
        .unwrap()
}

fn config(root: &Path, url: &str, policy: &str) {
    fs::write(root.join(".todo-unearther.toml"), format!("[github]\nenabled=true\ngh_auth=false\nrepository='org/repo'\napi_url='{url}'\ntimeout_seconds=1\n{policy}")).unwrap();
}

#[test]
fn requests_are_deduplicated_and_live_metadata_does_not_invalidate_source_checks() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("a.rs"),
        "// TODO(#12): first\n// FIXME: second ORG/REPO#12 https://github.com/org/repo/issues/12\n",
    )
    .unwrap();
    let (url, handle) = server(vec![
        (200, issue("Original title", "open", false)),
        (200, issue("Changed title", "closed", false)),
    ]);
    config(temp.path(), &url, "");
    let generated = run(temp.path(), &["--output", "TODO.md", "--strict"]);
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let text = fs::read_to_string(temp.path().join("TODO.md")).unwrap();
    assert!(text.contains("Original title"));
    assert!(!text.contains("labels:"));
    assert!(!text.contains("assignees:"));
    assert!(!text.contains("Open"));
    assert!(run(temp.path(), &["--check"]).status.success());
    assert_eq!(
        run(temp.path(), &["--check", "--compare", "full", "--strict"])
            .status
            .code(),
        Some(1)
    );
    let requests = handle.join().unwrap();
    assert_eq!(
        requests,
        ["/repos/org/repo/issues/12", "/repos/org/repo/issues/12"]
    );
    // The mock server is gone. A strict source-only check still needs no network.
    assert!(run(temp.path(), &["--check", "--strict"]).status.success());
}

#[test]
fn unavailable_is_a_policy_result_not_a_transport_error() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.py"), "# TODO(#12): task\n").unwrap();
    let (url, handle) = server(vec![(404, "{}".into())]);
    config(temp.path(), &url, "[policy]\nfail_on_unavailable=true");
    let generated = run(temp.path(), &["--output", "TODO.md", "--strict"]);
    assert_eq!(
        generated.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    assert!(
        fs::read_to_string(temp.path().join("TODO.md"))
            .unwrap()
            .contains("Unavailable")
    );
    handle.join().unwrap();
}

#[test]
fn routine_enrichment_ignores_volatile_fields_and_policies_opt_into_state() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("a.rs"),
        "// TODO(#12): fix ORG/REPO#12 https://github.com/org/repo/issues/12\n",
    )
    .unwrap();
    let (url, handle) = server(vec![
        (200, issue("Stable title", "open", false)),
        (200, issue("Stable title", "closed", false)),
        (200, issue("Stable title", "closed", false)),
        (200, issue("Stable title", "closed", false)),
    ]);
    config(temp.path(), &url, "");
    let generated = run(temp.path(), &["--output", "TODO.md"]);
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let checked = run(temp.path(), &["--check", "--compare", "full"]);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let json = run(temp.path(), &["--format", "json"]);
    assert!(json.status.success());
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let references = report["findings"][0]["references"].as_array().unwrap();
    assert_eq!(references.len(), 1);
    assert_eq!(references[0]["status"], "available");
    assert!(references[0]["issue"].get("labels").is_none());
    assert!(references[0]["issue"].get("assignees").is_none());
    config(temp.path(), &url, "[policy]\nfail_on_closed=true\n");
    let policy = run(temp.path(), &["--output", "TODO.md", "--strict"]);
    assert_eq!(
        policy.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&policy.stderr)
    );
    assert!(String::from_utf8_lossy(&policy.stderr).contains("closed item"));
    assert_eq!(handle.join().unwrap().len(), 4);
}

#[test]
fn state_is_only_required_in_api_responses_when_a_policy_needs_it() {
    for include_state in [false, true] {
        let (url, handle) = server(vec![(
            200,
            r#"{"title":"Title only","html_url":"https://github.com/org/repo/issues/12"}"#.into(),
        )]);
        let config = GithubConfig {
            api_url: Some(url),
            gh_auth: false,
            ..Default::default()
        };
        let mut reference = references::parse("org/repo#12").remove(0);
        references::resolve(&mut reference, "github.com", None);
        let result = github::lookup(
            &github::client(&config, None).unwrap(),
            &github::api_url(&config).unwrap(),
            &reference,
            &AtomicBool::new(false),
            include_state,
        );
        assert_eq!(
            result.status,
            if include_state {
                ReferenceStatus::Error
            } else {
                ReferenceStatus::Available
            }
        );
        handle.join().unwrap();
    }
}

#[test]
fn pr_rate_limit_auth_invalid_response_and_timeout_states_are_distinct() {
    for (code, body, expected, stopped) in [
        (
            200,
            issue("PR title", "closed", true),
            ReferenceStatus::Closed,
            false,
        ),
        (401, "{}".into(), ReferenceStatus::Error, true),
        (403, "{}".into(), ReferenceStatus::Error, true),
        (429, "{}".into(), ReferenceStatus::Error, true),
        (500, "{}".into(), ReferenceStatus::Error, false),
        (200, "invalid json".into(), ReferenceStatus::Error, false),
    ] {
        let (url, handle) = server(vec![(code, body)]);
        let config = GithubConfig {
            api_url: Some(url),
            gh_auth: false,
            ..Default::default()
        };
        let client = github::client(&config, None).unwrap();
        let mut reference = references::parse("org/repo#12").remove(0);
        references::resolve(&mut reference, "github.com", None);
        let limited = AtomicBool::new(false);
        let result = github::lookup(
            &client,
            &github::api_url(&config).unwrap(),
            &reference,
            &limited,
            true,
        );
        assert_eq!(result.status, expected);
        assert_eq!(limited.load(Ordering::Acquire), stopped);
        if code == 200
            && let Some(issue) = result.issue
        {
            assert_eq!(issue.kind, "pull_request");
        }
        handle.join().unwrap();
    }
    let server = Server::http("127.0.0.1:0").unwrap();
    let config = GithubConfig {
        api_url: Some(format!("http://{}", server.server_addr())),
        timeout_seconds: 1,
        gh_auth: false,
        ..Default::default()
    };
    let handle = thread::spawn(move || {
        let request = server
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        thread::sleep(Duration::from_millis(1200));
        let _ = request.respond(Response::from_string(issue("late", "open", false)));
    });
    let mut reference = references::parse("org/repo#12").remove(0);
    references::resolve(&mut reference, "github.com", None);
    let result = github::lookup(
        &github::client(&config, None).unwrap(),
        &github::api_url(&config).unwrap(),
        &reference,
        &AtomicBool::new(false),
        false,
    );
    assert_eq!(result.error.as_deref(), Some("GitHub request timed out"));
    handle.join().unwrap();
}

#[test]
fn redirects_do_not_forward_credentials() {
    let server = Server::http("127.0.0.1:0").unwrap();
    let config = GithubConfig {
        api_url: Some(format!("http://{}", server.server_addr())),
        ..Default::default()
    };
    let handle = thread::spawn(move || {
        let request = server
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        request
            .respond(Response::empty(302).with_header(
                tiny_http::Header::from_bytes("Location", "https://elsewhere.invalid/").unwrap(),
            ))
            .unwrap();
    });
    let mut reference = references::parse("org/repo#12").remove(0);
    references::resolve(&mut reference, "github.com", None);
    let result = github::lookup(
        &github::client(&config, Some("synthetic-test-token")).unwrap(),
        &github::api_url(&config).unwrap(),
        &reference,
        &AtomicBool::new(false),
        false,
    );
    assert_eq!(result.status, ReferenceStatus::Error);
    assert!(result.error.unwrap().contains("302"));
    handle.join().unwrap();
}
