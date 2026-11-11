//! The GitHub repository a remote's URL names, and the host and repository names gh is asked
//! about. Git resolves URL rewrites first; a URL that names no GitHub repository is not GitHub.

use super::*;

/// The host gh always knows; any other host must be one gh has a login for.
pub(super) const GITHUB: &str = "github.com";

/// A GitHub repository named by a remote's URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Repository {
    pub(super) host: String,
    owner: String,
    name: String,
}

impl Repository {
    pub(super) fn full(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

impl WorkingTree {
    /// The GitHub repository `remote` fetches from; Git resolves URL rewrites.
    pub(super) async fn github_remote(&self, remote: &str) -> Result<Repository, PullRequestError> {
        let urls = self.remote_urls(remote, false).await?;
        match urls.as_slice() {
            [url] => github_repository(url).ok_or_else(|| PullRequestError::NotGitHub {
                remote: remote.to_owned(),
                host: url_host(url),
                page: web_page(url),
            }),
            _ => Err(DiffError::PushDestinations.into()),
        }
    }
}

/// The repository a resolved remote URL names on GitHub: HTTPS, SSH or scp-like, with exactly
/// an owner and a name.
fn github_repository(url: &str) -> Option<Repository> {
    let (host, path) = match url.split_once("://") {
        Some((scheme, rest)) => {
            if !["https", "http", "ssh", "git"].contains(&scheme.to_ascii_lowercase().as_str()) {
                return None;
            }
            let (authority, path) = rest.split_at(rest.find(['/', '?', '#'])?);
            let path = path.strip_prefix('/')?;
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            // A port is not part of the host gh knows.
            let host = match host.split_once(':') {
                Some((host, port))
                    if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) =>
                {
                    host
                }
                Some(_) => return None,
                None => host,
            };
            (host, path)
        }
        None => {
            let (authority, path) = url.split_once(':')?;
            if authority.contains('/') || path.starts_with('/') {
                return None;
            }
            (
                authority
                    .rsplit_once('@')
                    .map_or(authority, |(_, host)| host),
                path,
            )
        }
    };
    let host = valid_host(host)?;
    // GitHub's SSH over port 443.
    let host = if host == "ssh.github.com" {
        GITHUB.to_owned()
    } else {
        host
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    let repository = Repository {
        host,
        owner: owner.to_owned(),
        name: name.to_owned(),
    };
    valid_full(&repository.full()).is_ok().then_some(repository)
}

/// The host of a URL that names no GitHub repository, if it has a plain one.
fn url_host(url: &str) -> Option<String> {
    if url.starts_with('/') || url.starts_with('.') {
        return None;
    }
    let authority = match url.split_once("://") {
        Some((_, rest)) => rest.split(['/', '?', '#']).next()?,
        None => url.split(['/', ':']).next()?,
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    valid_host(host.split(':').next()?)
}

/// The HTTPS page of the repository a remote URL names on a host that is not GitHub.
fn web_page(url: &str) -> Option<String> {
    let host = url_host(url)?;
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => url.split_once(':')?.1,
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let safe = path.split('/').all(|segment| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    });
    safe.then(|| format!("https://{host}/{path}"))
}

/// A host name gh can be asked about, in lowercase: letters, digits, dots and hyphens, not
/// starting with a hyphen, so it cannot read as an option.
pub(super) fn valid_host(host: &str) -> Option<String> {
    let host = host.to_ascii_lowercase();
    (!host.is_empty()
        && !host.starts_with('-')
        && host.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(&byte)
        }))
    .then_some(host)
}

/// `owner/name` with only the characters GitHub allows, so it cannot leave its API path.
pub(super) fn valid_full(full: &str) -> Result<(), PullRequestError> {
    let valid = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    };
    match full.split_once('/') {
        Some((owner, name)) if valid(owner) && valid(name) => Ok(()),
        _ => Err(PullRequestError::Failed(format!(
            "GitHub named an unexpected repository: {full}"
        ))),
    }
}

#[cfg(test)]
#[test]
fn reads_github_repositories_from_resolved_remote_urls() {
    let full = |url| github_repository(url).map(|repo| format!("{}/{}", repo.host, repo.full()));
    for (url, expected) in [
        (
            "https://github.com/acme/api.git",
            Some("github.com/acme/api"),
        ),
        ("https://github.com/acme/api/", Some("github.com/acme/api")),
        ("git@github.com:bw/api.git", Some("github.com/bw/api")),
        (
            "ssh://git@ssh.github.com:443/acme/api.git",
            Some("github.com/acme/api"),
        ),
        (
            "https://github.com:443/acme/api.git",
            Some("github.com/acme/api"),
        ),
        (
            "https://github.acme.example:8443/team/tool",
            Some("github.acme.example/team/tool"),
        ),
        ("https://github.com:x/acme/api", None),
        (
            "https://GitHub.Acme.example/team/tool",
            Some("github.acme.example/team/tool"),
        ),
        ("https://gitlab.com/group/sub/api.git", None),
        ("/srv/git/api.git", None),
        ("file:///srv/git/api.git", None),
        ("https://github.com/acme/{owner}", None),
        ("https://github.com?x=/acme/api", None),
        ("https://github.com#/acme/api", None),
        ("https://-oProxyCommand=x/acme/api", None),
        ("https://git hub.com/acme/api", None),
        ("git@github.com:acme/api?x", None),
    ] {
        assert_eq!(full(url).as_deref(), expected, "{url}");
    }
    assert_eq!(
        url_host("https://gitlab.com/group/sub/api.git").as_deref(),
        Some("gitlab.com")
    );
    assert_eq!(url_host("/srv/git/api.git"), None);
    for (url, page) in [
        (
            "https://user:token@gitlab.com/group/sub/api.git",
            Some("https://gitlab.com/group/sub/api"),
        ),
        (
            "git@gitlab.com:group/api.git",
            Some("https://gitlab.com/group/api"),
        ),
        ("https://gitlab.com/group/../api", None),
        ("/srv/git/api.git", None),
    ] {
        assert_eq!(web_page(url).as_deref(), page, "{url}");
    }
    assert_eq!(url_host("https://-oProxyCommand=x/acme/api"), None);
    assert_eq!(
        url_host("https://gitlab.com?x/api").as_deref(),
        Some("gitlab.com")
    );
}
