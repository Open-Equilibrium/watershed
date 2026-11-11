//! Why a pull request could not be planned, created or read, and how gh's failures become those
//! reasons.

use super::*;

/// Why no pull request could be planned, created or read.
#[derive(Debug)]
pub enum PullRequestError {
    /// Git failed, or the checkout is on no branch, merging, or pushes elsewhere.
    Git(DiffError),
    /// The branch has no commit to publish.
    NoCommit,
    /// The repository has no remote.
    NoRemote,
    /// Several remotes and no upstream or origin.
    AmbiguousRemote,
    /// The remote is not on GitHub or a GitHub Enterprise host gh has a login for.
    NotGitHub {
        /// The remote's name.
        remote: String,
        /// Its host, none for a local path.
        host: Option<String>,
        /// The repository's page on its host, to create the host's merge request there.
        page: Option<String>,
    },
    /// The branch was never pushed, or has commits its upstream lacks.
    NeedsPush {
        /// The local branch.
        branch: String,
        /// Its remote.
        remote: String,
        /// Commits its upstream lacks, as last fetched.
        ahead: u64,
        /// It has no upstream to compare with yet.
        first: bool,
    },
    /// The upstream has commits the branch lacks.
    Behind {
        /// The local branch.
        branch: String,
        /// Its upstream.
        upstream: String,
        /// Commits the upstream lacks.
        ahead: u64,
        /// Commits the branch lacks.
        behind: u64,
    },
    /// GitHub's head branch is at another commit than the checkout's.
    HeadMoved {
        /// The head branch on GitHub.
        branch: String,
    },
    /// gh could not run on the host.
    GhMissing(String),
    /// gh is too old for the commands Liquid runs.
    GhOutdated(String),
    /// gh has no working login for the host.
    SignIn {
        /// The host whose login failed.
        host: String,
    },
    /// gh cannot access the repository.
    NoAccess {
        /// The repository as `owner/name`.
        repository: String,
        /// gh's message.
        detail: String,
    },
    /// GitHub could not be reached, or limited the rate.
    Unreachable {
        /// The host.
        host: String,
        /// gh's message.
        detail: String,
    },
    /// gh failed otherwise.
    Failed(String),
    /// GitHub refused the pull request; none was created.
    NotCreated(String),
    /// GitHub returned no result; a pull request may exist.
    Unknown(String),
    /// The head, base or target changed since the plan was shown.
    Changed(Box<PullRequestPlan>),
    /// An open pull request from this head already exists.
    Exists(Box<PullRequest>),
}

impl From<DiffError> for PullRequestError {
    fn from(error: DiffError) -> Self {
        Self::Git(error)
    }
}

impl fmt::Display for PullRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(error) => error.fmt(f),
            Self::GhMissing(detail)
            | Self::GhOutdated(detail)
            | Self::Failed(detail)
            | Self::NotCreated(detail)
            | Self::Unknown(detail)
            | Self::NoAccess { detail, .. }
            | Self::Unreachable { detail, .. } => f.write_str(detail),
            error => write!(f, "{error:?}"),
        }
    }
}

/// A gh failure as Liquid explains it; gh's own message stays as the detail.
pub(super) fn gh_error(host: &str, repository: &str, error: DiffError) -> PullRequestError {
    let detail = match error {
        DiffError::GitStart(error) => return PullRequestError::GhMissing(error.to_string()),
        DiffError::Git(detail) => without_credentials(&detail),
        error => return PullRequestError::Git(error),
    };
    let lower = detail.to_ascii_lowercase();
    let has = |markers: &[&str]| markers.iter().any(|marker| lower.contains(marker));
    if has(&[
        "rate limit",
        "error connecting",
        "dial tcp",
        "no such host",
        "i/o timeout",
        "connection refused",
        "connection reset",
        "(http 5",
    ]) {
        PullRequestError::Unreachable {
            host: host.to_owned(),
            detail,
        }
    } else if has(&[
        "(http 401)",
        "gh auth login",
        "not logged in",
        "bad credentials",
    ]) {
        PullRequestError::SignIn {
            host: host.to_owned(),
        }
    } else if has(&[
        "(http 403)",
        "(http 404)",
        "could not resolve to a repository",
        "resource not accessible",
    ]) {
        PullRequestError::NoAccess {
            repository: repository.to_owned(),
            detail,
        }
    } else {
        PullRequestError::Failed(detail)
    }
}
