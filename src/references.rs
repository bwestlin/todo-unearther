use crate::model::{IssueReference, ReferenceStatus};
use regex::Regex;
use std::{collections::BTreeSet, sync::LazyLock};

static REFERENCES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?P<url>https?://[A-Za-z0-9.-]+(?::[0-9]+)?/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/(?:issues|pull)/[0-9]+)|(?P<repo>[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)\#(?P<qualified>[0-9]+)|\#(?P<bare>[0-9]+)"
).expect("valid issue reference regex")
});

pub fn parse(text: &str) -> Vec<IssueReference> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for captures in REFERENCES.captures_iter(text) {
        let found = captures.get(0).unwrap();
        if found.start() > 0 {
            let before = text.as_bytes()[found.start() - 1];
            if before.is_ascii_alphanumeric() || matches!(before, b'_' | b'/' | b'#') {
                continue;
            }
        }
        if text
            .as_bytes()
            .get(found.end())
            .is_some_and(u8::is_ascii_alphanumeric)
        {
            continue;
        }
        let (host, repository, number, url) = if let Some(url) = captures.name("url") {
            let Ok(parsed) = reqwest::Url::parse(url.as_str()) else {
                continue;
            };
            let parts: Vec<_> = parsed.path().trim_matches('/').split('/').collect();
            let Ok(number) = parts[3].parse::<u64>() else {
                continue;
            };
            (
                parsed.host_str().map(str::to_ascii_lowercase),
                Some(format!("{}/{}", parts[0], parts[1])),
                number,
                Some(url.as_str().to_owned()),
            )
        } else {
            let Some(number) = captures
                .name("qualified")
                .or_else(|| captures.name("bare"))
                .and_then(|n| n.as_str().parse::<u64>().ok())
            else {
                continue;
            };
            (
                None,
                captures.name("repo").map(|m| m.as_str().to_owned()),
                number,
                None,
            )
        };
        if number == 0 {
            continue;
        }
        let key = (
            host.clone(),
            repository.as_ref().map(|r| r.to_ascii_lowercase()),
            number,
        );
        if !seen.insert(key) {
            continue;
        }
        result.push(IssueReference {
            raw: found.as_str().into(),
            host,
            repository,
            number,
            url,
            status: ReferenceStatus::NotRequested,
            issue: None,
            error: None,
        });
    }
    result
}

/// Resolve only the structured identity; raw source text stays untouched for fingerprints.
pub fn resolve(reference: &mut IssueReference, host: &str, repository: Option<&str>) {
    if reference.host.is_none() {
        reference.host = Some(host.to_owned());
    }
    if reference.repository.is_none() {
        reference.repository = repository.map(str::to_owned);
    }
    if reference.url.is_none()
        && let (Some(host), Some(repo)) = (&reference.host, &reference.repository)
    {
        reference.url = Some(format!("https://{host}/{repo}/issues/{}", reference.number));
    }
}

pub fn remote_repository(remote: &str, host: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    if let Ok(url) = reqwest::Url::parse(remote)
        && url
            .host_str()
            .is_some_and(|remote_host| remote_host.eq_ignore_ascii_case(host))
    {
        let repo = url.path().trim_matches('/');
        return crate::config::validate_repository(repo)
            .ok()
            .map(|()| repo.to_owned());
    }
    let (server, repo) = remote.split_once(':')?;
    if server.rsplit('@').next()?.eq_ignore_ascii_case(host) {
        return crate::config::validate_repository(repo)
            .ok()
            .map(|()| repo.to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_all_reference_shapes_without_fragment_false_positives() {
        let refs = parse(
            "TODO(#12): see org/repo#34 and https://github.com/other/lib/issues/56. #12 C#1234 #123abc",
        );
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].number, 12);
        assert_eq!(refs[1].repository.as_deref(), Some("org/repo"));
        assert_eq!(refs[2].host.as_deref(), Some("github.com"));
    }

    #[test]
    fn handles_ssh_https_and_enterprise_remotes() {
        for remote in [
            "git@github.com:me/repo.git",
            "github.com:me/repo.git",
            "https://github.com/me/repo.git",
            "ssh://git@github.com/me/repo.git",
        ] {
            assert_eq!(
                remote_repository(remote, "github.com").as_deref(),
                Some("me/repo")
            );
        }
        assert!(remote_repository("https://elsewhere.test/me/repo", "github.com").is_none());
    }
}
