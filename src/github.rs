use crate::{config::GithubConfig, model::*, process, references};
use anyhow::{Context, Result, ensure};
use rayon::prelude::*;
use reqwest::{
    Url,
    blocking::Client,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
    redirect::Policy,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct Lookup {
    pub status: ReferenceStatus,
    pub url: Option<String>,
    pub issue: Option<IssueMetadata>,
    pub error: Option<String>,
}

impl Lookup {
    fn error(message: impl Into<String>) -> Self {
        Self {
            status: ReferenceStatus::Error,
            url: None,
            issue: None,
            error: Some(message.into()),
        }
    }
}

pub fn resolve_references(
    report: &mut ScanReport,
    config: &GithubConfig,
    root: &Path,
    use_git: bool,
) {
    if !report.findings.iter().any(|f| !f.references.is_empty()) {
        return;
    }
    let needs_repo = report
        .findings
        .iter()
        .flat_map(|f| &f.references)
        .any(|r| r.repository.is_none());
    let repository = config.repository.clone().or_else(|| {
        if !needs_repo || !use_git {
            return None;
        }
        let out = process::capture(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["remote", "get-url", "origin"]),
            None,
            Duration::from_secs(2),
        )
        .ok()?;
        if !out.status.success() {
            return None;
        }
        references::remote_repository(&String::from_utf8_lossy(&out.stdout), &config.host)
    });
    for finding in &mut report.findings {
        for reference in &mut finding.references {
            references::resolve(reference, &config.host, repository.as_deref());
        }
        let mut seen = BTreeSet::new();
        finding.references.retain(|reference| {
            seen.insert(reference.key().unwrap_or_else(|| reference.raw.clone()))
        });
    }
}

/// Returns whether enrichment completed. A 404 is a completed, unavailable result.
pub fn enrich(report: &mut ScanReport, config: &GithubConfig, include_state: bool) -> Result<bool> {
    let mut unique = BTreeMap::new();
    for reference in report.findings.iter().flat_map(|f| &f.references) {
        if let Some(key) = reference.key()
            && reference
                .host
                .as_deref()
                .is_some_and(|h| h.eq_ignore_ascii_case(&config.host))
        {
            unique.entry(key).or_insert_with(|| reference.clone());
        }
    }
    let results =
        if unique.is_empty() {
            BTreeMap::new()
        } else {
            let api = api_url(config)?;
            let token = authentication(config);
            let client = client(config, token.as_deref())?;
            let limited = AtomicBool::new(false);
            let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build()?;
            pool.install(|| {
                unique.par_iter().map(|(key, reference)| {
            let lookup = if limited.load(Ordering::Acquire) {
                Lookup::error("GitHub requests stopped after rate limit or authentication failure")
            } else { lookup(&client, &api, reference, &limited, include_state) };
            (key.clone(), lookup)
        }).collect::<BTreeMap<_, _>>()
            })
        };
    let mut warnings = BTreeMap::new();
    for finding in &mut report.findings {
        for reference in &mut finding.references {
            if let Some(result) = reference.key().and_then(|key| results.get(&key)) {
                reference.status = result.status;
                reference.issue.clone_from(&result.issue);
                reference.error.clone_from(&result.error);
                if let Some(url) = &result.url {
                    reference.url = Some(url.clone());
                }
                if let Some(error) = &result.error {
                    warnings.insert(reference.key().unwrap(), error.clone());
                }
            } else {
                reference.status = ReferenceStatus::Unresolved;
                reference.error =
                    Some("no repository identity or reference is for another GitHub host".into());
                warnings.insert(reference.raw.clone(), reference.error.clone().unwrap());
            }
        }
    }
    let complete = warnings.is_empty();
    for (reference, warning) in warnings {
        report.warn("github", format!("{reference}: {warning}"));
    }
    Ok(complete)
}

fn authentication(config: &GithubConfig) -> Option<String> {
    // A token is used only for the explicitly configured host/API endpoint.
    if let Ok(token) = std::env::var("GITHUB_TOKEN")
        && !token.trim().is_empty()
    {
        return Some(token);
    }
    if !config.gh_auth {
        return None;
    }
    let result = process::capture(
        Command::new("gh").args(["auth", "token", "--hostname", &config.host]),
        None,
        Duration::from_secs(5),
    )
    .ok()?;
    if !result.status.success() {
        return None;
    }
    let token = String::from_utf8(result.stdout).ok()?.trim().to_owned();
    (!token.is_empty()).then_some(token)
}

pub fn api_url(config: &GithubConfig) -> Result<Url> {
    let default = if config.host == "github.com" {
        "https://api.github.com".into()
    } else {
        format!("https://{}/api/v3", config.host)
    };
    let url = Url::parse(config.api_url.as_deref().unwrap_or(&default))
        .context("invalid GitHub API URL")?;
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))),
        "GitHub API URL must use HTTPS (HTTP is allowed only on loopback for tests)"
    );
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "GitHub API URL cannot contain credentials, query, or fragment"
    );
    Ok(url)
}

pub fn client(config: &GithubConfig, token: Option<&str>) -> Result<Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "accept",
        HeaderValue::from_static("application/vnd.github+json"),
    );
    if let Some(token) = token {
        let mut value =
            HeaderValue::from_str(&format!("Bearer {token}")).context("invalid GitHub token")?;
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);
    }
    Ok(Client::builder()
        .user_agent(concat!("todo-unearther/", env!("CARGO_PKG_VERSION")))
        .default_headers(headers)
        .redirect(Policy::none())
        .timeout(Duration::from_secs(config.timeout_seconds))
        .build()?)
}

#[derive(Deserialize)]
struct ApiIssue {
    title: String,
    state: Option<String>,
    html_url: String,
    pull_request: Option<serde_json::Value>,
}

pub fn lookup(
    client: &Client,
    api: &Url,
    reference: &IssueReference,
    limited: &AtomicBool,
    include_state: bool,
) -> Lookup {
    let mut url = api.clone();
    {
        let Ok(mut path) = url.path_segments_mut() else {
            return Lookup::error("invalid API base path");
        };
        path.pop_if_empty().push("repos");
        for part in reference.repository.as_deref().unwrap_or("").split('/') {
            path.push(part);
        }
        path.push("issues").push(&reference.number.to_string());
    }
    let response = match client.get(url).send() {
        Ok(response) => response,
        Err(error) => {
            return Lookup::error(if error.is_timeout() {
                "GitHub request timed out"
            } else {
                "GitHub request failed"
            });
        }
    };
    let status = response.status();
    if status.as_u16() == 404 || status.as_u16() == 410 {
        return Lookup {
            status: ReferenceStatus::Unavailable,
            url: None,
            issue: None,
            error: None,
        };
    }
    if matches!(status.as_u16(), 401 | 403 | 429) {
        limited.store(true, Ordering::Release);
        return Lookup::error(format!(
            "GitHub returned HTTP {}; check permissions or rate limits",
            status.as_u16()
        ));
    }
    if !status.is_success() {
        return Lookup::error(format!("GitHub returned HTTP {}", status.as_u16()));
    }
    // Bound response memory as well as elapsed request time.
    let issue: ApiIssue =
        match serde_json::from_reader(std::io::Read::take(response, 2 * 1024 * 1024)) {
            Ok(issue) => issue,
            Err(_) => return Lookup::error("GitHub returned an invalid issue response"),
        };
    let status = if include_state {
        match issue.state.as_deref() {
            Some("closed") => ReferenceStatus::Closed,
            Some("open") => ReferenceStatus::Open,
            _ => {
                return Lookup::error(
                    "GitHub response has no valid issue state for policy evaluation",
                );
            }
        }
    } else {
        ReferenceStatus::Available
    };
    Lookup {
        status,
        url: Some(issue.html_url),
        issue: Some(IssueMetadata {
            title: issue.title,
            kind: if issue.pull_request.is_some() {
                "pull_request"
            } else {
                "issue"
            }
            .into(),
        }),
        error: None,
    }
}
