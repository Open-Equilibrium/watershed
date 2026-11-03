//! Pull requests on GitHub through the host's `gh` ([mockups](../../../../docs/design/mockups/liquid-git-prs-worktrees.html)
//! §1–5): the head is the branch's pushed upstream on its own remote, under Push's destination
//! checks; a fork targets GitHub's parent. Liquid never sees gh's login, and gh never prompts.

use super::process::{gh, git_bytes};
use super::*;
use serde_json::Value;

/// The host gh always knows; any other host must be one gh has a login for.
const GITHUB: &str = "github.com";

/// Pull requests read per head lookup, newest first.
const MAX_LISTED: &str = "20";

/// Branches GitHub lists to choose a base from, one page.
const MAX_BRANCHES: &str = "100";

/// The pull request fields Liquid reads.
const FIELDS: &str = "number,title,state,isDraft,url,baseRefName,headRefName,headRepository,headRepositoryOwner,statusCheckRollup,reviewDecision";

/// The pushed head of a pull request and what it can target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestPlan {
    /// The GitHub host, `github.com` or a GitHub Enterprise host gh has a login for.
    pub host: String,
    /// The pushed head repository as `owner/name`.
    pub head_repository: String,
    /// The head branch on GitHub.
    pub head_branch: String,
    /// The full commit GitHub has for the head branch, which is the branch's own.
    pub head_commit: String,
    /// The repositories it can target: GitHub's parent first for a fork, then the head's own.
    pub targets: Vec<String>,
    /// The head repository is a fork whose parent GitHub did not name.
    pub parent_unknown: bool,
    /// The chosen or preselected target; none while the user must choose.
    pub target: Option<String>,
    /// The target's default branch as GitHub reports it.
    pub default_branch: Option<String>,
    /// The requested base, else the target's default branch.
    pub base: Option<String>,
    /// The commits and files the base lacks; none without a base or when GitHub could not
    /// compare.
    pub comparison: Option<Comparison>,
    /// GitHub has no branch `base` in the target.
    pub base_missing: bool,
    /// An open pull request from this head to the target, shown instead of creating one.
    pub open: Option<PullRequest>,
    /// Files that differ from the last commit; a pull request does not include them.
    pub uncommitted: u64,
    /// The latest commit's subject, which prefills the title.
    pub subject: String,
    /// The latest commit's body, which prefills the description.
    pub body: String,
}

/// What the head adds to the base on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    /// Commits the base lacks.
    pub commits: u64,
    /// Changed files.
    pub files: u64,
    /// Added lines.
    pub added: u64,
    /// Removed lines.
    pub removed: u64,
    /// GitHub's comparison page.
    pub url: String,
}

/// A pull request's state on GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullRequestState {
    /// Open for review.
    Open,
    /// Open as a draft.
    Draft,
    /// Merged on GitHub.
    Merged,
    /// Closed without merging.
    Closed,
}

/// A pull request's review decision; none when GitHub requires no review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    /// A review is required.
    Required,
    /// Reviewers approved.
    Approved,
    /// Reviewers requested changes.
    ChangesRequested,
}

/// A pull request as the PR chip and its summary show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    /// Its number in the target repository.
    pub number: u64,
    /// Its title on GitHub.
    pub title: String,
    /// Its page on GitHub.
    pub url: String,
    /// Open, draft, merged or closed.
    pub state: PullRequestState,
    /// The target repository as `owner/name`.
    pub repository: String,
    /// The base branch.
    pub base: String,
    /// The head as GitHub shows it: the branch, or `owner:branch` from a fork.
    pub head: String,
    /// Checks that passed, were neutral or skipped; all counts zero when none are reported.
    pub checks_passed: u64,
    /// Checks that failed, errored, timed out or were cancelled.
    pub checks_failed: u64,
    /// Checks still queued or running.
    pub checks_pending: u64,
    /// The review decision, if GitHub requires one.
    pub review: Option<ReviewDecision>,
}

/// The reviewed pull request to create, with the identities its plan showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestDraft {
    /// The plan's host.
    pub host: String,
    /// The chosen target repository.
    pub target: String,
    /// The plan's head repository.
    pub head_repository: String,
    /// The plan's head branch.
    pub head_branch: String,
    /// The plan's head commit.
    pub head_commit: String,
    /// The chosen base branch.
    pub base: String,
    /// The reviewed title.
    pub title: String,
    /// The reviewed description.
    pub body: String,
    /// Create it as a draft.
    pub draft: bool,
}

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
            | Self::Failed(detail)
            | Self::NotCreated(detail)
            | Self::Unknown(detail)
            | Self::NoAccess { detail, .. }
            | Self::Unreachable { detail, .. } => f.write_str(detail),
            error => write!(f, "{error:?}"),
        }
    }
}

/// A GitHub repository named by a remote's URL.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Repository {
    host: String,
    owner: String,
    name: String,
}

impl Repository {
    fn full(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// The checkout's pushed head, checked as Push checks its destination.
struct Head {
    repository: Repository,
    branch: String,
    commit: String,
    status: GitStatus,
}

/// What GitHub says about the head repository.
struct HeadRepository {
    full: String,
    owner: String,
    default_branch: Option<String>,
    parent: Option<(String, Option<String>)>,
    fork: bool,
}

impl WorkingTree {
    /// The pushed head, its targets, base and comparison, and any open pull request; with
    /// `target` and `base` as the user chose them.
    pub(crate) async fn pull_request_plan(
        &self,
        program: &OsStr,
        target: Option<&str>,
        base: Option<&str>,
    ) -> Result<PullRequestPlan, PullRequestError> {
        let head = self.pushed_head().await?;
        let host = head.repository.host.clone();
        self.gh_login(program, &host)
            .await
            .map_err(|error| match error {
                PullRequestError::NotGitHub { host, .. } => PullRequestError::NotGitHub {
                    remote: head.status.remote.clone().unwrap_or_default(),
                    host,
                },
                error => error,
            })?;
        let repository = self.head_repository(program, &head.repository).await?;
        let on_github = self
            .gh_api(
                program,
                &host,
                &head.repository.full(),
                &[
                    &format!(
                        "repos/{}/branches/{}",
                        repository.full,
                        path_segment(&head.branch)
                    ),
                    "--jq",
                    ".commit.sha",
                ],
                None,
            )
            .await;
        let on_github = match on_github {
            Err(PullRequestError::NoAccess { .. }) => {
                return Err(PullRequestError::NeedsPush {
                    branch: head.status.branch.clone().unwrap_or_default(),
                    remote: head.status.remote.clone().unwrap_or_default(),
                    ahead: head.status.ahead,
                    first: true,
                });
            }
            result => result?,
        };
        if on_github.trim() != head.commit {
            return Err(PullRequestError::HeadMoved {
                branch: head.branch,
            });
        }
        let mut targets = Vec::new();
        if let Some((parent, _)) = &repository.parent {
            targets.push(parent.clone());
        }
        targets.push(repository.full.clone());
        let parent_unknown = repository.fork && repository.parent.is_none();
        let target = match target {
            Some(target) => targets
                .iter()
                .find(|known| known.as_str() == target)
                .cloned(),
            None if parent_unknown => None,
            None => targets.first().cloned(),
        };
        let default_branch = match (&target, &repository.parent) {
            (Some(target), Some((parent, default))) if target == parent => default.clone(),
            (Some(_), _) => repository.default_branch.clone(),
            (None, _) => None,
        };
        let base = base.map(str::to_owned).or_else(|| default_branch.clone());
        let (mut comparison, mut base_missing, mut open) = (None, false, None);
        if let Some(target) = &target {
            open = self
                .pull_requests(program, &host, target, &repository, &head.branch)
                .await?
                .into_iter()
                .find(|pull| {
                    matches!(pull.state, PullRequestState::Open | PullRequestState::Draft)
                });
            if let Some(base) = &base {
                match self
                    .compare(
                        program,
                        &host,
                        target,
                        base,
                        &repository.owner,
                        &head.branch,
                    )
                    .await
                {
                    Ok(compared) => comparison = Some(compared),
                    Err(PullRequestError::NoAccess { .. }) => base_missing = true,
                    Err(error) => return Err(error),
                }
            }
        }
        let message = git_bytes(
            &self.root,
            &[],
            &["log", "-1", "--format=%s%x00%b", &head.commit, "--"],
        )
        .await?;
        let message = String::from_utf8_lossy(&message);
        let (subject, body) = message.split_once('\0').unwrap_or((&message, ""));
        Ok(PullRequestPlan {
            host,
            head_repository: repository.full,
            head_branch: head.branch,
            head_commit: head.commit,
            targets,
            parent_unknown,
            target,
            default_branch,
            base,
            comparison,
            base_missing,
            open,
            uncommitted: head.status.changed,
            subject: subject.trim().to_owned(),
            body: body.trim().to_owned(),
        })
    }

    /// Creates the reviewed pull request after rechecking every identity its plan showed.
    pub(crate) async fn create_pull_request(
        &self,
        program: &OsStr,
        draft: &PullRequestDraft,
    ) -> Result<PullRequest, PullRequestError> {
        let plan = self
            .pull_request_plan(program, Some(&draft.target), Some(&draft.base))
            .await?;
        if plan.host != draft.host
            || plan.head_repository != draft.head_repository
            || plan.head_branch != draft.head_branch
            || plan.head_commit != draft.head_commit
            || plan.target.as_deref() != Some(&draft.target)
            || plan.base.as_deref() != Some(&draft.base)
            || plan.base_missing
        {
            return Err(PullRequestError::Changed(Box::new(plan)));
        }
        if let Some(open) = plan.open {
            return Err(PullRequestError::Exists(Box::new(open)));
        }
        let owner = plan
            .head_repository
            .split_once('/')
            .map_or("", |(owner, _)| owner);
        let request = serde_json::json!({
            "title": draft.title,
            "body": draft.body,
            "head": format!("{owner}:{}", draft.head_branch),
            "base": draft.base,
            "draft": draft.draft,
        });
        let created = self
            .gh_api(
                program,
                &draft.host,
                &draft.target,
                &[
                    "--method",
                    "POST",
                    &format!("repos/{}/pulls", draft.target),
                    "--input",
                    "-",
                ],
                Some(request.to_string().as_bytes()),
            )
            .await
            .map_err(|error| match error {
                PullRequestError::SignIn { .. }
                | PullRequestError::NoAccess { .. }
                | PullRequestError::GhMissing(_) => error,
                PullRequestError::Failed(detail) if detail.contains("(HTTP 4") => {
                    PullRequestError::NotCreated(detail)
                }
                error => PullRequestError::Unknown(error.to_string()),
            })?;
        let created: Value = serde_json::from_str(&created)
            .map_err(|_| PullRequestError::Unknown("GitHub returned no pull request.".into()))?;
        let number = created["number"]
            .as_u64()
            .ok_or_else(|| PullRequestError::Unknown("GitHub returned no pull request.".into()))?;
        Ok(PullRequest {
            number,
            title: text(&created["title"]),
            url: text(&created["html_url"]),
            state: if created["draft"].as_bool() == Some(true) {
                PullRequestState::Draft
            } else {
                PullRequestState::Open
            },
            repository: draft.target.clone(),
            base: draft.base.clone(),
            head: head_label(&draft.target, &plan.head_repository, &draft.head_branch),
            checks_passed: 0,
            checks_failed: 0,
            checks_pending: 0,
            review: None,
        })
    }

    /// The latest pull request from the branch's upstream, open ones first; none without an
    /// upstream on GitHub.
    pub(crate) async fn pull_request(
        &self,
        program: &OsStr,
    ) -> Result<Option<PullRequest>, PullRequestError> {
        let status = self.status().await?;
        let Some(branch) = status.branch.as_deref() else {
            return Ok(None);
        };
        let Some((remote, reference)) = self.configured_upstream(branch).await? else {
            return Ok(None);
        };
        let Some(branch) = reference.strip_prefix("refs/heads/") else {
            return Ok(None);
        };
        let Ok(repository) = self.github_remote(&remote).await else {
            return Ok(None);
        };
        let head = self.head_repository(program, &repository).await?;
        let mut targets: Vec<&str> = head
            .parent
            .iter()
            .map(|(parent, _)| parent.as_str())
            .collect();
        targets.push(&head.full);
        let mut latest = None;
        for target in targets {
            let found = self
                .pull_requests(program, &repository.host, target, &head, branch)
                .await?;
            if let Some(open) = found
                .iter()
                .find(|pull| matches!(pull.state, PullRequestState::Open | PullRequestState::Draft))
            {
                return Ok(Some(open.clone()));
            }
            latest = latest.or(found.into_iter().next());
        }
        Ok(latest)
    }

    /// The target's branches GitHub lists, to choose a base from.
    pub(crate) async fn pull_request_bases(
        &self,
        program: &OsStr,
        host: &str,
        target: &str,
    ) -> Result<Vec<String>, PullRequestError> {
        valid_full(target)?;
        let listed = self
            .gh_api(
                program,
                host,
                target,
                &[
                    &format!("repos/{target}/branches?per_page={MAX_BRANCHES}"),
                    "--jq",
                    ".[].name",
                ],
                None,
            )
            .await?;
        Ok(listed.lines().map(str::to_owned).collect())
    }

    /// The branch's pushed upstream on its own remote, refused as Push refuses it.
    async fn pushed_head(&self) -> Result<Head, PullRequestError> {
        let status = self.status().await?;
        if status.merging {
            return Err(DiffError::MergeInProgress.into());
        }
        let branch = status.branch.clone().ok_or(DiffError::NoBranch)?;
        if status.commit.is_none() {
            return Err(PullRequestError::NoCommit);
        }
        let remote = match &status.remote {
            Some(remote) => remote.clone(),
            None if status.has_remotes => return Err(PullRequestError::AmbiguousRemote),
            None => return Err(PullRequestError::NoRemote),
        };
        let (transfer, _) = self.destination().await?;
        self.check_push_destination(&transfer).await?;
        let repository = self.github_remote(&remote).await?;
        if transfer.first_push || status.gone {
            return Err(PullRequestError::NeedsPush {
                branch,
                remote,
                ahead: status.ahead,
                first: true,
            });
        }
        let upstream = status.upstream.clone().unwrap_or_default();
        if status.behind > 0 {
            return Err(PullRequestError::Behind {
                branch,
                upstream,
                ahead: status.ahead,
                behind: status.behind,
            });
        }
        if status.ahead > 0 {
            return Err(PullRequestError::NeedsPush {
                branch,
                remote,
                ahead: status.ahead,
                first: false,
            });
        }
        let remote_branch = transfer
            .confirmation
            .reference
            .strip_prefix("refs/heads/")
            .unwrap_or(&transfer.confirmation.reference)
            .to_owned();
        Ok(Head {
            repository,
            branch: remote_branch,
            commit: transfer.confirmation.commit,
            status,
        })
    }

    /// The GitHub repository `remote` fetches from; Git resolves URL rewrites.
    async fn github_remote(&self, remote: &str) -> Result<Repository, PullRequestError> {
        let urls = self.remote_urls(remote, false).await?;
        let not_github = |host| PullRequestError::NotGitHub {
            remote: remote.to_owned(),
            host,
        };
        match urls.as_slice() {
            [url] => github_repository(url).ok_or_else(|| not_github(url_host(url))),
            _ => Err(DiffError::PushDestinations.into()),
        }
    }

    /// Requires gh's working login for `host`; a host gh does not know is not GitHub.
    async fn gh_login(&self, program: &OsStr, host: &str) -> Result<(), PullRequestError> {
        let output = gh(
            &self.root,
            program,
            &["auth", "status", "--json", "hosts", "--hostname", host],
            None,
        )
        .await
        .map_err(|error| gh_error(host, "", error))?;
        let hosts: Value = serde_json::from_slice(&output).map_err(|_| {
            PullRequestError::Failed("gh returned an unreadable login status.".into())
        })?;
        let accounts = hosts["hosts"][host].as_array().cloned().unwrap_or_default();
        if accounts.is_empty() {
            return Err(if host == GITHUB {
                PullRequestError::SignIn {
                    host: host.to_owned(),
                }
            } else {
                PullRequestError::NotGitHub {
                    remote: String::new(),
                    host: Some(host.to_owned()),
                }
            });
        }
        let active = accounts
            .iter()
            .find(|account| account["active"].as_bool() == Some(true))
            .unwrap_or(&accounts[0]);
        match active["state"].as_str() {
            Some("success") => Ok(()),
            Some("timeout") => Err(PullRequestError::Unreachable {
                host: host.to_owned(),
                detail: text(&active["error"]),
            }),
            _ => Err(PullRequestError::SignIn {
                host: host.to_owned(),
            }),
        }
    }

    async fn head_repository(
        &self,
        program: &OsStr,
        repository: &Repository,
    ) -> Result<HeadRepository, PullRequestError> {
        let full = repository.full();
        let output = self
            .gh_api(
                program,
                &repository.host,
                &full,
                &[&format!("repos/{full}")],
                None,
            )
            .await?;
        let json: Value = serde_json::from_str(&output).map_err(|_| {
            PullRequestError::Failed("GitHub returned an unreadable repository.".into())
        })?;
        let full = json["full_name"].as_str().unwrap_or(&full).to_owned();
        valid_full(&full)?;
        let parent = json["parent"]["full_name"]
            .as_str()
            .filter(|parent| valid_full(parent).is_ok())
            .map(|parent| {
                (
                    parent.to_owned(),
                    json["parent"]["default_branch"].as_str().map(str::to_owned),
                )
            });
        Ok(HeadRepository {
            owner: full
                .split_once('/')
                .map_or("", |(owner, _)| owner)
                .to_owned(),
            default_branch: json["default_branch"].as_str().map(str::to_owned),
            fork: json["fork"].as_bool() == Some(true),
            parent,
            full,
        })
    }

    /// Pull requests in `target` whose head is `branch` of the head repository, newest first.
    async fn pull_requests(
        &self,
        program: &OsStr,
        host: &str,
        target: &str,
        head: &HeadRepository,
        branch: &str,
    ) -> Result<Vec<PullRequest>, PullRequestError> {
        let output = gh(
            &self.root,
            program,
            &[
                "pr",
                "list",
                "--repo",
                &format!("{host}/{target}"),
                "--head",
                branch,
                "--state",
                "all",
                "--limit",
                MAX_LISTED,
                "--json",
                FIELDS,
            ],
            None,
        )
        .await
        .map_err(|error| gh_error(host, target, error))?;
        let listed: Value = serde_json::from_slice(&output).map_err(|_| {
            PullRequestError::Failed("gh returned unreadable pull requests.".into())
        })?;
        Ok(listed
            .as_array()
            .into_iter()
            .flatten()
            .filter(|pull| {
                pull["headRefName"].as_str() == Some(branch)
                    && format!(
                        "{}/{}",
                        text(&pull["headRepositoryOwner"]["login"]),
                        text(&pull["headRepository"]["name"])
                    )
                    .eq_ignore_ascii_case(&head.full)
            })
            .filter_map(|pull| summary(pull, target, &head.full))
            .collect())
    }

    async fn compare(
        &self,
        program: &OsStr,
        host: &str,
        target: &str,
        base: &str,
        owner: &str,
        branch: &str,
    ) -> Result<Comparison, PullRequestError> {
        let output = self
            .gh_api(
                program,
                host,
                target,
                &[
                    &format!(
                        "repos/{target}/compare/{}...{owner}:{}",
                        path_segment(base),
                        path_segment(branch)
                    ),
                    "--jq",
                    "{commits: .ahead_by, url: .html_url, files: (.files | length), added: ([.files[].additions] | add // 0), removed: ([.files[].deletions] | add // 0)}",
                ],
                None,
            )
            .await?;
        let json: Value = serde_json::from_str(&output).map_err(|_| {
            PullRequestError::Failed("GitHub returned an unreadable comparison.".into())
        })?;
        Ok(Comparison {
            commits: json["commits"].as_u64().unwrap_or(0),
            files: json["files"].as_u64().unwrap_or(0),
            added: json["added"].as_u64().unwrap_or(0),
            removed: json["removed"].as_u64().unwrap_or(0),
            url: text(&json["url"]),
        })
    }

    async fn gh_api(
        &self,
        program: &OsStr,
        host: &str,
        repository: &str,
        args: &[&str],
        input: Option<&[u8]>,
    ) -> Result<String, PullRequestError> {
        let mut all = vec!["api", "--hostname", host];
        all.extend_from_slice(args);
        gh(&self.root, program, &all, input)
            .await
            .map(|output| String::from_utf8_lossy(&output).into_owned())
            .map_err(|error| gh_error(host, repository, error))
    }
}

/// A gh failure as Liquid explains it; gh's own message stays as the detail.
fn gh_error(host: &str, repository: &str, error: DiffError) -> PullRequestError {
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

/// The repository a resolved remote URL names on GitHub: HTTPS, SSH or scp-like, with exactly
/// an owner and a name.
fn github_repository(url: &str) -> Option<Repository> {
    let (host, path) = match url.split_once("://") {
        Some((scheme, rest)) => {
            if !["https", "http", "ssh", "git"].contains(&scheme.to_ascii_lowercase().as_str()) {
                return None;
            }
            let (authority, path) = rest.split_once('/')?;
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            // SSH's port is not part of the host gh knows.
            let host = if scheme.eq_ignore_ascii_case("ssh") {
                host.split(':').next()?
            } else {
                host
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
    let host = host.to_ascii_lowercase();
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
    (!repository.host.is_empty() && valid_full(&repository.full()).is_ok()).then_some(repository)
}

/// The host of a URL that names no GitHub repository, if it has one.
fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', ':']).next()?;
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    (!host.is_empty() && !url.starts_with('/') && !url.starts_with('.'))
        .then(|| host.to_ascii_lowercase())
}

/// `owner/name` with only the characters GitHub allows, so it cannot leave its API path.
fn valid_full(full: &str) -> Result<(), PullRequestError> {
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

/// A branch name in an API path: `/` stays, everything gh or HTTP could read is escaped.
fn path_segment(name: &str) -> String {
    name.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn head_label(target: &str, head_repository: &str, branch: &str) -> String {
    if target.eq_ignore_ascii_case(head_repository) {
        branch.to_owned()
    } else {
        let owner = head_repository
            .split_once('/')
            .map_or("", |(owner, _)| owner);
        format!("{owner}:{branch}")
    }
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// gh's pull request JSON as the summary shows it.
fn summary(pull: &Value, target: &str, head_repository: &str) -> Option<PullRequest> {
    let state = match (pull["state"].as_str()?, pull["isDraft"].as_bool()) {
        ("OPEN", Some(true)) => PullRequestState::Draft,
        ("OPEN", _) => PullRequestState::Open,
        ("MERGED", _) => PullRequestState::Merged,
        ("CLOSED", _) => PullRequestState::Closed,
        _ => return None,
    };
    let (mut passed, mut failed, mut pending) = (0, 0, 0);
    for check in pull["statusCheckRollup"].as_array().into_iter().flatten() {
        let outcome = match check["__typename"].as_str() {
            Some("CheckRun") if check["status"].as_str() != Some("COMPLETED") => "PENDING",
            Some("CheckRun") => check["conclusion"].as_str().unwrap_or(""),
            _ => check["state"].as_str().unwrap_or(""),
        };
        match outcome {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => passed += 1,
            "PENDING" | "EXPECTED" | "QUEUED" | "IN_PROGRESS" | "WAITING" => pending += 1,
            _ => failed += 1,
        }
    }
    Some(PullRequest {
        number: pull["number"].as_u64()?,
        title: text(&pull["title"]),
        url: text(&pull["url"]),
        state,
        repository: target.to_owned(),
        base: text(&pull["baseRefName"]),
        head: head_label(target, head_repository, pull["headRefName"].as_str()?),
        checks_passed: passed,
        checks_failed: failed,
        checks_pending: pending,
        review: match pull["reviewDecision"].as_str() {
            Some("APPROVED") => Some(ReviewDecision::Approved),
            Some("CHANGES_REQUESTED") => Some(ReviewDecision::ChangesRequested),
            Some("REVIEW_REQUIRED") => Some(ReviewDecision::Required),
            _ => None,
        },
    })
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
            "https://GitHub.Acme.example/team/tool",
            Some("github.acme.example/team/tool"),
        ),
        ("https://gitlab.com/group/sub/api.git", None),
        ("/srv/git/api.git", None),
        ("file:///srv/git/api.git", None),
        ("https://github.com/acme/{owner}", None),
    ] {
        assert_eq!(full(url).as_deref(), expected, "{url}");
    }
    assert_eq!(
        url_host("https://gitlab.com/group/sub/api.git").as_deref(),
        Some("gitlab.com")
    );
    assert_eq!(url_host("/srv/git/api.git"), None);
    assert_eq!(
        path_segment("feature/a b#?{x}"),
        "feature/a%20b%23%3F%7Bx%7D"
    );
}
