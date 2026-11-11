//! Pull requests on GitHub through the host's `gh` ([mockups](../../../../docs/design/mockups/liquid-git-prs-worktrees.html)
//! §1–5): the head is the branch's pushed upstream on its own remote, under Push's destination
//! checks; a fork targets GitHub's parent. Liquid never sees gh's login, and gh never prompts.

use super::github_remote::{GITHUB, Repository, valid_full, valid_host};
use super::process::{gh, git_bytes};
use super::pull_error::gh_error;
use super::*;
use serde_json::Value;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::task::Poll;

/// Pull requests read per head lookup, newest first: one page of GitHub's maximum.
const MAX_LISTED: usize = 100;

/// Branches per page GitHub lists to choose a base from, GitHub's maximum.
const BRANCHES_PER_PAGE: usize = 100;

/// Pages of branches read to choose a base from.
const MAX_BRANCH_PAGES: usize = 5;

/// Files GitHub's comparison lists at most; past them Git counts the files itself.
const MAX_COMPARED_FILES: u64 = 300;

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
    /// The head as GitHub shows it for the target: the branch, or `owner:branch` from a fork.
    pub head: String,
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
    /// GitHub could not compare the head with the base, though both exist.
    pub comparison_failed: bool,
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
    /// The commit the head and base share, as GitHub names it; empty when it named none.
    pub merge_base: String,
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

/// The checkout's pushed head, checked as Push checks its destination.
struct Head {
    repository: Repository,
    remote: String,
    /// The local branch.
    local: String,
    /// The branch on the remote.
    branch: String,
    commit: String,
    /// The branch was never pushed there.
    first_push: bool,
    status: GitStatus,
}

impl Head {
    /// The refusal when the upstream has commits the branch lacks.
    fn behind(&self) -> Option<PullRequestError> {
        (self.status.behind > 0).then(|| PullRequestError::Behind {
            branch: self.local.clone(),
            upstream: self.status.upstream.clone().unwrap_or_default(),
            ahead: self.status.ahead,
            behind: self.status.behind,
        })
    }
}

/// The targets a head repository offers, and the target and base chosen or preselected.
struct Chosen {
    targets: Vec<String>,
    parent_unknown: bool,
    target: Option<String>,
    default_branch: Option<String>,
    base: Option<String>,
}

impl Chosen {
    /// GitHub's parent first for a fork, then the head's own; without a choice the parent, or
    /// none when GitHub did not name a fork's parent; the base as chosen, else the target's
    /// default.
    fn of(repository: &HeadRepository, target: Option<&str>, base: Option<&str>) -> Self {
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
        Self {
            targets,
            parent_unknown,
            target,
            default_branch,
            base,
        }
    }
}

/// A pull request in a head lookup.
struct Listed {
    number: u64,
    open: bool,
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
        self.planned(program, head, target, base, true).await
    }

    /// The branch's target, base and any open pull request before it is pushed, for "Commit,
    /// push & create PR", which checks them before any write; GitHub has no comparison yet, but
    /// says whether it has the base.
    pub(crate) async fn pull_request_prepared(
        &self,
        program: &OsStr,
        target: Option<&str>,
        base: Option<&str>,
    ) -> Result<PullRequestPlan, PullRequestError> {
        let head = self.github_head().await?;
        if let Some(behind) = head.behind() {
            return Err(behind);
        }
        self.planned(program, head, target, base, false).await
    }

    /// The plan for `head`; a `pushed` head must be on GitHub at its commit and is compared
    /// with the base, else GitHub is only asked whether it has the base.
    async fn planned(
        &self,
        program: &OsStr,
        head: Head,
        target: Option<&str>,
        base: Option<&str>,
        pushed: bool,
    ) -> Result<PullRequestPlan, PullRequestError> {
        let host = head.repository.host.clone();
        if !self.gh_login(program, &host).await? {
            return Err(PullRequestError::NotGitHub {
                remote: head.remote,
                page: Some(format!("https://{host}/{}", head.repository.full())),
                host: Some(host),
            });
        }
        let repository = self.head_repository(program, &head.repository).await?;
        if pushed {
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
                        branch: head.local,
                        remote: head.remote,
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
        }
        let Chosen {
            targets,
            parent_unknown,
            target,
            default_branch,
            base,
        } = Chosen::of(&repository, target, base);
        let (open, (comparison, base_missing, comparison_failed)) = match &target {
            Some(target) if pushed => {
                both(
                    self.open_pull_request(program, &host, target, &repository, &head.branch),
                    self.compared(program, &host, target, base.as_deref(), &head.commit),
                )
                .await?
            }
            Some(target) => {
                let (open, missing) = both(
                    self.open_pull_request(program, &host, target, &repository, &head.branch),
                    self.base_missing(program, &host, target, base.as_deref()),
                )
                .await?;
                (open, (None, missing, false))
            }
            None => (None, (None, false, false)),
        };
        let (subject, body) = self.message_of(&head.commit).await?;
        Ok(PullRequestPlan {
            host,
            head: head_label(
                target.as_deref().unwrap_or(&repository.full),
                &repository.full,
                &head.branch,
            ),
            head_repository: repository.full,
            head_branch: head.branch,
            head_commit: head.commit,
            targets,
            parent_unknown,
            target,
            default_branch,
            base,
            comparison,
            comparison_failed,
            base_missing,
            open,
            uncommitted: head.status.changed,
            subject,
            body,
        })
    }

    /// Whether GitHub lacks the branch `base` in `target`; none to check without a base.
    async fn base_missing(
        &self,
        program: &OsStr,
        host: &str,
        target: &str,
        base: Option<&str>,
    ) -> Result<bool, PullRequestError> {
        let Some(base) = base else {
            return Ok(false);
        };
        if !valid_base(base) {
            return Ok(true);
        }
        match self
            .gh_api(
                program,
                host,
                target,
                &[
                    &format!("repos/{target}/branches/{}", path_segment(base)),
                    "--jq",
                    ".name",
                ],
                None,
            )
            .await
        {
            Ok(_) => Ok(false),
            Err(PullRequestError::NoAccess { .. }) => Ok(true),
            Err(error) => Err(error),
        }
    }

    /// The subject and body of `commit` as Git has them.
    async fn message_of(&self, commit: &str) -> Result<(String, String), PullRequestError> {
        let message = git_bytes(
            &self.root,
            &[],
            &["log", "-1", "--format=%s%x00%b", commit, "--"],
        )
        .await?;
        let message = String::from_utf8_lossy(&message);
        let (subject, body) = message.split_once('\0').unwrap_or((&message, ""));
        Ok((subject.trim().to_owned(), body.trim().to_owned()))
    }

    /// The plan `draft` was reviewed against, read again; a changed identity or an open pull
    /// request refuses it.
    pub(crate) async fn recheck_pull_request(
        &self,
        program: &OsStr,
        draft: &PullRequestDraft,
    ) -> Result<(), PullRequestError> {
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
        match plan.open {
            Some(open) => Err(PullRequestError::Exists(Box::new(open))),
            None => Ok(()),
        }
    }

    /// Sends the rechecked draft to GitHub; only the reviewed fields go.
    pub(crate) async fn post_pull_request(
        &self,
        program: &OsStr,
        draft: &PullRequestDraft,
    ) -> Result<PullRequest, PullRequestError> {
        let mut request = serde_json::json!({
            "title": draft.title,
            "body": draft.body,
            "head": qualified(owner_of(&draft.head_repository), &draft.head_branch),
            "base": draft.base,
            "draft": draft.draft,
        });
        // GitHub needs the head's `owner/name` when one organization owns both.
        if !draft.target.eq_ignore_ascii_case(&draft.head_repository) {
            request["head_repo"] = draft.head_repository.as_str().into();
        }
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
            head: head_label(&draft.target, &draft.head_repository, &draft.head_branch),
            checks_passed: 0,
            checks_failed: 0,
            checks_pending: 0,
            review: None,
        })
    }

    /// The latest pull request from the branch's upstream, open ones first and the fork's
    /// parent before the fork; none without an upstream. A remote gh has no GitHub login for
    /// is refused before any request names its host.
    pub(crate) async fn pull_request(
        &self,
        program: &OsStr,
    ) -> Result<Option<PullRequest>, PullRequestError> {
        let Some(branch) = self.branch().await? else {
            return Ok(None);
        };
        let Some((remote, reference)) = self.configured_upstream(&branch).await? else {
            return Ok(None);
        };
        let Some(branch) = reference.strip_prefix("refs/heads/") else {
            return Ok(None);
        };
        let repository = match self.github_remote(&remote).await {
            Ok(repository) => repository,
            Err(error @ PullRequestError::NotGitHub { .. }) => return Err(error),
            Err(_) => return Ok(None),
        };
        if !self.gh_login(program, &repository.host).await? {
            return Err(PullRequestError::NotGitHub {
                remote,
                page: Some(format!("https://{}/{}", repository.host, repository.full())),
                host: Some(repository.host),
            });
        }
        let host = &repository.host;
        let head = self.head_repository(program, &repository).await?;
        let parent = head.parent.as_ref().map(|(parent, _)| parent.as_str());
        let (in_parent, in_fork) = both(
            async {
                match parent {
                    Some(parent) => {
                        self.pull_requests_from(program, host, parent, &head, branch)
                            .await
                    }
                    None => Ok(Vec::new()),
                }
            },
            self.pull_requests_from(program, host, &head.full, &head, branch),
        )
        .await?;
        let in_parent = parent.map(|parent| (parent, in_parent));
        let in_fork = Some((head.full.as_str(), in_fork));
        let lists = [&in_parent, &in_fork];
        let chosen = lists
            .iter()
            .flat_map(|list| list.iter())
            .find_map(|(target, listed)| {
                listed
                    .iter()
                    .find(|pull| pull.open)
                    .map(|pull| (*target, pull.number))
            })
            .or_else(|| {
                lists
                    .iter()
                    .flat_map(|list| list.iter())
                    .find_map(|(target, listed)| listed.first().map(|pull| (*target, pull.number)))
            });
        match chosen {
            Some((target, number)) => self
                .pull_request_details(program, host, target, &head.full, number)
                .await
                .map(Some),
            None => Ok(None),
        }
    }

    /// The target's branches, up to [`MAX_BRANCH_PAGES`] pages, to choose a base from; `true`
    /// when GitHub may list more.
    pub(crate) async fn pull_request_bases(
        &self,
        program: &OsStr,
        host: &str,
        target: &str,
    ) -> Result<(Vec<String>, bool), PullRequestError> {
        valid_full(target)?;
        if valid_host(host).as_deref() != Some(host) {
            return Err(PullRequestError::Failed(format!(
                "GitHub named an unexpected host: {host}"
            )));
        }
        if !self.gh_login(program, host).await? {
            return Err(PullRequestError::SignIn {
                host: host.to_owned(),
            });
        }
        let mut names = Vec::new();
        for page in 1..=MAX_BRANCH_PAGES {
            let listed = self
                .gh_api(
                    program,
                    host,
                    target,
                    &[
                        &format!(
                            "repos/{target}/branches?per_page={BRANCHES_PER_PAGE}&page={page}"
                        ),
                        "--jq",
                        ".[].name",
                    ],
                    None,
                )
                .await?;
            let before = names.len();
            names.extend(listed.lines().map(str::to_owned));
            if names.len() - before < BRANCHES_PER_PAGE {
                return Ok((names, false));
            }
        }
        Ok((names, true))
    }

    /// The branch's pushed upstream on its own remote, refused as Push refuses it.
    async fn pushed_head(&self) -> Result<Head, PullRequestError> {
        let head = self.github_head().await?;
        let (status, branch, remote) = (&head.status, head.local.clone(), head.remote.clone());
        if head.first_push || status.gone {
            return Err(PullRequestError::NeedsPush {
                branch,
                remote,
                ahead: status.ahead,
                first: true,
            });
        }
        if let Some(behind) = head.behind() {
            return Err(behind);
        }
        if status.ahead > 0 {
            return Err(PullRequestError::NeedsPush {
                branch,
                remote,
                ahead: status.ahead,
                first: false,
            });
        }
        Ok(head)
    }

    /// The branch, its commit and the GitHub repository and branch Push sends it to, under
    /// Push's destination checks, pushed or not.
    async fn github_head(&self) -> Result<Head, PullRequestError> {
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
        let transfer = self.destination_of(&status).await?;
        self.check_push_destination(&transfer).await?;
        let repository = self.github_remote(&remote).await?;
        let remote_branch = transfer
            .confirmation
            .reference
            .strip_prefix("refs/heads/")
            .unwrap_or(&transfer.confirmation.reference)
            .to_owned();
        Ok(Head {
            repository,
            remote,
            local: branch,
            branch: remote_branch,
            commit: transfer.confirmation.commit,
            first_push: transfer.first_push,
            status,
        })
    }

    /// Whether gh has a working login for `host`; a host other than github.com without one is
    /// not GitHub, and github.com without one needs a sign-in.
    async fn gh_login(&self, program: &OsStr, host: &str) -> Result<bool, PullRequestError> {
        let output = gh(
            &self.root,
            program,
            &["auth", "status", "--json", "hosts", "--hostname", host],
            None,
        )
        .await
        .map_err(|error| match gh_error(host, "", error) {
            PullRequestError::Failed(detail) if detail.contains("unknown flag") => {
                PullRequestError::GhOutdated(detail)
            }
            error => error,
        })?;
        let hosts: Value = serde_json::from_slice(&output).map_err(|_| {
            PullRequestError::Failed("gh returned an unreadable login status.".into())
        })?;
        let accounts = hosts["hosts"][host].as_array().cloned().unwrap_or_default();
        if accounts.is_empty() {
            return if host == GITHUB {
                Err(PullRequestError::SignIn {
                    host: host.to_owned(),
                })
            } else {
                Ok(false)
            };
        }
        let active = accounts
            .iter()
            .find(|account| account["active"].as_bool() == Some(true))
            .unwrap_or(&accounts[0]);
        match active["state"].as_str() {
            Some("success") => Ok(true),
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
            owner: owner_of(&full).to_owned(),
            default_branch: json["default_branch"].as_str().map(str::to_owned),
            fork: json["fork"].as_bool() == Some(true),
            parent,
            full,
        })
    }

    /// The open pull request from `branch` of the head repository to `target`, if any.
    async fn open_pull_request(
        &self,
        program: &OsStr,
        host: &str,
        target: &str,
        head: &HeadRepository,
        branch: &str,
    ) -> Result<Option<PullRequest>, PullRequestError> {
        let listed = self
            .pull_requests_from(program, host, target, head, branch)
            .await?;
        match listed.iter().find(|pull| pull.open) {
            Some(pull) => self
                .pull_request_details(program, host, target, &head.full, pull.number)
                .await
                .map(Some),
            None => Ok(None),
        }
    }

    /// Pull requests in `target` whose head is `branch` of the head repository, newest first.
    /// GitHub filters by the head's owner, so other forks' pull requests take no place; a full
    /// page without an open one is not read as none.
    async fn pull_requests_from(
        &self,
        program: &OsStr,
        host: &str,
        target: &str,
        head: &HeadRepository,
        branch: &str,
    ) -> Result<Vec<Listed>, PullRequestError> {
        let output = self
            .gh_api(
                program,
                host,
                target,
                &[
                    &format!(
                        "repos/{target}/pulls?head={}&state=all&per_page={MAX_LISTED}",
                        qualified(&head.owner, &path_segment(branch))
                    ),
                    "--jq",
                    r#".[] | "\(.number) \(.state) \(.head.repo.full_name // "") \(.head.ref)""#,
                ],
                None,
            )
            .await?;
        let rows = output.lines().count();
        let listed: Vec<_> = output
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(4, ' ');
                let number = fields.next()?.parse().ok()?;
                let state = fields.next()?;
                let repository = fields.next()?;
                (repository.eq_ignore_ascii_case(&head.full) && fields.next()? == branch).then(
                    || Listed {
                        number,
                        open: state == "open",
                    },
                )
            })
            .collect();
        if rows >= MAX_LISTED && !listed.iter().any(|pull| pull.open) {
            return Err(PullRequestError::Failed(format!(
                "GitHub lists more pull requests from {} than Liquid reads; check on GitHub.",
                qualified(&head.owner, branch)
            )));
        }
        Ok(listed)
    }

    /// One pull request's state, checks and review.
    async fn pull_request_details(
        &self,
        program: &OsStr,
        host: &str,
        target: &str,
        head_repository: &str,
        number: u64,
    ) -> Result<PullRequest, PullRequestError> {
        let output = gh(
            &self.root,
            program,
            &[
                "pr",
                "view",
                &number.to_string(),
                "--repo",
                &format!("{host}/{target}"),
                "--json",
                FIELDS,
            ],
            None,
        )
        .await
        .map_err(|error| gh_error(host, target, error))?;
        let pull: Value = serde_json::from_slice(&output).map_err(|_| {
            PullRequestError::Failed("gh returned an unreadable pull request.".into())
        })?;
        summary(&pull, target, head_repository).ok_or_else(|| {
            PullRequestError::Failed("gh returned an unreadable pull request.".into())
        })
    }

    /// The comparison of the head `commit` with `base`, whether GitHub lacks `base`, and whether
    /// GitHub could not compare them; only gh and login failures end the plan.
    async fn compared(
        &self,
        program: &OsStr,
        host: &str,
        target: &str,
        base: Option<&str>,
        commit: &str,
    ) -> Result<(Option<Comparison>, bool, bool), PullRequestError> {
        let Some(base) = base else {
            return Ok((None, false, false));
        };
        if !valid_base(base) {
            return Ok((None, true, false));
        }
        match self.compare(program, host, target, base, commit).await {
            Ok(mut compared) => {
                if compared.files >= MAX_COMPARED_FILES {
                    self.count_files(&mut compared, commit).await?;
                }
                Ok((Some(compared), false, false))
            }
            Err(PullRequestError::NoAccess { .. }) => Ok((None, true, false)),
            Err(PullRequestError::Failed(_) | PullRequestError::Unreachable { .. }) => {
                Ok((None, false, true))
            }
            Err(error) => Err(error),
        }
    }

    /// GitHub's comparison of `base` with the head by its pushed commit, which names it in
    /// the fork network even where `owner:branch` would name the target's own branch, as when
    /// one organization owns both.
    async fn compare(
        &self,
        program: &OsStr,
        host: &str,
        target: &str,
        base: &str,
        commit: &str,
    ) -> Result<Comparison, PullRequestError> {
        let output = self
            .gh_api(
                program,
                host,
                target,
                &[
                    &format!("repos/{target}/compare/{}...{commit}", path_segment(base)),
                    "--jq",
                    "{commits: .ahead_by, url: .html_url, merge_base: .merge_base_commit.sha, files: (.files | length), added: ([.files[].additions] | add // 0), removed: ([.files[].deletions] | add // 0)}",
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
            merge_base: text(&json["merge_base"]),
        })
    }

    /// The files and lines from the shared commit GitHub named to the head `commit`, as Git
    /// counts them; GitHub's counts stay when Git lacks that commit or the list is too long.
    async fn count_files(
        &self,
        comparison: &mut Comparison,
        commit: &str,
    ) -> Result<(), PullRequestError> {
        let base = comparison.merge_base.as_str();
        if !commit_id(base) || self.resolve(&format!("{base}^{{commit}}")).await?.is_none() {
            return Ok(());
        }
        let counts = match self.diff(&["--numstat", "-z", base, commit]).await {
            Ok(numstat) => line_counts(&numstat),
            Err(DiffError::TooLarge) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        comparison.files = counts.len() as u64;
        comparison.added = counts.values().map(|(added, _)| added).sum();
        comparison.removed = counts.values().map(|(_, removed)| removed).sum();
        Ok(())
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

/// Runs both lookups at once; the first failure ends both.
async fn both<A, B, E>(
    first: impl Future<Output = Result<A, E>>,
    second: impl Future<Output = Result<B, E>>,
) -> Result<(A, B), E> {
    let (mut first, mut second) = (pin!(first), pin!(second));
    let (mut a, mut b) = (None, None);
    poll_fn(|context| {
        if a.is_none()
            && let Poll::Ready(result) = first.as_mut().poll(context)
        {
            a = Some(result?);
        }
        if b.is_none()
            && let Poll::Ready(result) = second.as_mut().poll(context)
        {
            b = Some(result?);
        }
        if a.is_some() && b.is_some() {
            Poll::Ready(Ok((
                a.take()
                    .expect("first result is stored before both are checked"),
                b.take()
                    .expect("second result is stored before both are checked"),
            )))
        } else {
            Poll::Pending
        }
    })
    .await
}

/// A base branch name that can go into an API path: not empty, no leading `-`, no `.` or `..`
/// segment and no control character.
fn valid_base(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.chars().any(char::is_control)
        && name
            .split('/')
            .all(|segment| segment != "." && segment != "..")
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

/// The head as GitHub shows it in `target`: the branch, or `owner:branch` from a fork.
fn head_label(target: &str, head_repository: &str, branch: &str) -> String {
    if target.eq_ignore_ascii_case(head_repository) {
        branch.to_owned()
    } else {
        qualified(owner_of(head_repository), branch)
    }
}

/// A head as GitHub's API names it across forks.
fn qualified(owner: &str, branch: &str) -> String {
    format!("{owner}:{branch}")
}

fn owner_of(full: &str) -> &str {
    full.split_once('/').map_or("", |(owner, _)| owner)
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
fn escapes_branch_names_in_api_paths() {
    assert_eq!(
        path_segment("feature/a b#?{x}"),
        "feature/a%20b%23%3F%7Bx%7D"
    );
}
