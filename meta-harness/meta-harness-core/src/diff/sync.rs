//! Explicit push and fast-forward pull through the host's Git and synchronous hooks.

use super::branches::without_credentials;
use super::process::{git_config, git_ref_output, no_prompts, user_git};
use super::stash::listed_files;
use super::*;

/// The exact branch, destination and commit approved for a default-branch push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushConfirmation {
    /// The local branch.
    pub branch: String,
    /// The named destination remote.
    pub remote: String,
    /// The full destination ref.
    pub reference: String,
    /// The full branch-tip object ID.
    pub commit: String,
    /// The one repository the remote fetches from and pushes to, without URL credentials.
    pub repository: String,
}

/// The destination and counts captured immediately before a push or pull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitTransfer {
    /// The local branch.
    pub branch: String,
    /// The exact remote; strip URL credentials before display.
    pub remote: String,
    /// The upstream name as Git shows it.
    pub upstream: String,
    /// Captured commits to push.
    pub ahead: u64,
    /// Captured commits to pull.
    pub behind: u64,
    /// The branch had no configured upstream before this push.
    pub first_push: bool,
    /// The upstream ref was gone before this transfer.
    pub gone: bool,
    /// Exact identities to return unchanged when confirming the push.
    pub confirmation: PushConfirmation,
}

impl WorkingTree {
    pub(super) async fn configured_upstream(
        &self,
        branch: &str,
    ) -> Result<Option<(String, String)>, DiffError> {
        let merge = git_config(
            &self.root,
            &["--get-all", &format!("branch.{branch}.merge")],
        )
        .await?;
        let remote = git_config(&self.root, &["--get", &format!("branch.{branch}.remote")]).await?;
        let merge = merge.split('\n').next().unwrap_or("");
        let remote = remote.trim_end_matches('\n');
        Ok(
            (!merge.is_empty() && !remote.is_empty())
                .then(|| (remote.to_owned(), merge.to_owned())),
        )
    }

    /// One destination policy for status, Fetch and transfers: upstream, origin, sole remote.
    pub(super) async fn remote(
        &self,
        branch: Option<&str>,
    ) -> Result<(Option<String>, bool), DiffError> {
        let remotes = git_ref_output(&self.root, &["remote"]).await?;
        let names: Vec<_> = remotes.lines().collect();
        if let Some(branch) = branch
            && let Some((remote, _)) = self.configured_upstream(branch).await?
        {
            return Ok((Some(remote), !names.is_empty()));
        }
        let remote = if names.contains(&"origin") {
            Some("origin")
        } else if names.len() == 1 {
            Some(names[0])
        } else {
            None
        };
        Ok((remote.map(str::to_owned), !names.is_empty()))
    }

    pub(super) async fn destination(&self) -> Result<(GitTransfer, GitStatus), DiffError> {
        let status = self.status().await?;
        Ok((self.destination_of(&status).await?, status))
    }

    /// Where the branch `status` reads transfers to.
    pub(super) async fn destination_of(
        &self,
        status: &GitStatus,
    ) -> Result<GitTransfer, DiffError> {
        let branch = status.branch.clone().ok_or(DiffError::NoBranch)?;
        let commit = status
            .commit
            .clone()
            .ok_or_else(|| DiffError::Git("Commit before transferring this branch.".to_owned()))?;
        let remote = status.remote.clone().ok_or_else(|| {
            DiffError::Git(
                if status.has_remotes {
                    "The destination remote is ambiguous; configure an upstream in Git."
                } else {
                    "The repository has no remote."
                }
                .to_owned(),
            )
        })?;
        let configured = self.configured_upstream(&branch).await?;
        let first_push = configured.is_none();
        let reference = configured
            .map(|(_, reference)| reference)
            .unwrap_or_else(|| format!("refs/heads/{branch}"));
        // Never let a short or malformed merge target bypass the default-branch comparison.
        if !reference.starts_with("refs/heads/")
            || git(&self.root, &[], &["check-ref-format", &reference])
                .await
                .is_err()
        {
            return Err(DiffError::Git(
                "The upstream must be a full refs/heads/ branch reference accepted by Git."
                    .to_owned(),
            ));
        }
        let upstream = status.upstream.clone().unwrap_or_else(|| {
            format!(
                "{remote}/{}",
                reference
                    .strip_prefix("refs/heads/")
                    .expect("the reference was checked to start with refs/heads/")
            )
        });
        let confirmation = PushConfirmation {
            branch: branch.clone(),
            remote: remote.clone(),
            reference,
            commit,
            repository: String::new(),
        };
        Ok(GitTransfer {
            upstream,
            branch,
            remote,
            ahead: 0,
            behind: 0,
            first_push,
            gone: status.gone,
            confirmation,
        })
    }

    /// Network preflight is bounded by Checkpoints, outside any checkout write lock.
    pub(crate) async fn prepare_push(
        &self,
        confirmed: Option<&PushConfirmation>,
    ) -> Result<GitTransfer, DiffError> {
        let (mut transfer, status) = self.destination().await?;
        if status.merging {
            return Err(DiffError::MergeInProgress);
        }
        transfer.confirmation.repository = self.check_push_destination(&transfer).await?;
        self.fetch_transfer(&transfer).await?;
        let advertised = self.transfer_counts(&mut transfer).await?;
        if transfer.behind > 0 {
            return Err(DiffError::Diverged(Box::new(transfer), String::new()));
        }
        // A symbolic destination updates the branch it names, so that branch is checked.
        let reference = &transfer.confirmation.reference;
        let reference = advertised
            .lines()
            .find_map(|line| {
                line.strip_prefix("ref: ")?
                    .strip_suffix(&format!("\t{reference}"))
            })
            .unwrap_or(reference);
        let default = status
            .default_branch
            .as_ref()
            .is_some_and(|branch| reference == format!("refs/heads/{branch}"))
            || advertised
                .lines()
                .any(|line| line == format!("ref: {reference}\tHEAD"));
        // A stale approval always asks again, even if the new destination is not a default.
        if (default && confirmed != Some(&transfer.confirmation))
            || confirmed.is_some_and(|confirmed| *confirmed != transfer.confirmation)
        {
            return Err(DiffError::PushDefault(Box::new(transfer)));
        }
        Ok(transfer)
    }

    /// Pushes the captured commit, never a branch ref that could move after confirmation.
    pub(crate) async fn push(&self, transfer: GitTransfer) -> Result<GitTransfer, DiffError> {
        let spec = format!(
            "{}:{}",
            transfer.confirmation.commit, transfer.confirmation.reference
        );
        let result = user_git(
            &self.root,
            &[
                "push",
                "--porcelain",
                "--no-force",
                "--no-mirror",
                "--no-follow-tags",
                "--no-recurse-submodules",
                "--",
                &transfer.remote,
                &spec,
            ],
        )
        .await;
        match result {
            Ok(_) if !transfer.first_push => Ok(transfer),
            Ok(_) => {
                // Git cannot infer a local branch from an OID source. Track only the captured
                // branch, even if another process switched HEAD while the push ran.
                for (key, value) in [
                    ("remote", &transfer.remote),
                    ("merge", &transfer.confirmation.reference),
                ] {
                    git(
                        &self.root,
                        &[],
                        &[
                            "config",
                            "--replace-all",
                            &format!("branch.{}.{key}", transfer.branch),
                            value,
                        ],
                    )
                    .await
                    .map_err(|error| {
                        DiffError::TransferFailed(without_credentials(&error.to_string()))
                    })?;
                }
                Ok(transfer)
            }
            Err(DiffError::Git(detail))
                if detail.contains("[rejected]")
                    && (detail.contains("non-fast-forward") || detail.contains("fetch first")) =>
            {
                Err(DiffError::Diverged(
                    Box::new(transfer),
                    without_credentials(&detail),
                ))
            }
            Err(error) => {
                let detail = without_credentials(&error.to_string());
                if sign_in_failed(&detail) {
                    Err(DiffError::SignInFailed(Box::new(transfer), detail))
                } else {
                    Err(DiffError::TransferFailed(detail))
                }
            }
        }
    }

    /// Keep the default-branch check and push on the same single repository, which it returns.
    /// Git resolves URL rewrites; compare only after removing any URL credentials, and never
    /// report URLs.
    pub(super) async fn check_push_destination(
        &self,
        transfer: &GitTransfer,
    ) -> Result<String, DiffError> {
        let mut fetch = self.remote_urls(&transfer.remote, false).await?;
        let push = self.remote_urls(&transfer.remote, true).await?;
        if fetch.len() != 1 || push != fetch {
            return Err(DiffError::PushDestinations);
        }
        let branch_remote = git_config(
            &self.root,
            &["--get", &format!("branch.{}.pushRemote", transfer.branch)],
        )
        .await?;
        let default_remote = git_config(&self.root, &["--get", "remote.pushDefault"]).await?;
        for remote in [&branch_remote, &default_remote] {
            let remote = remote.trim_end_matches('\n');
            if !remote.is_empty() && self.remote_urls(remote, true).await? != fetch {
                return Err(DiffError::PushDestinations);
            }
        }
        Ok(fetch.swap_remove(0))
    }

    pub(super) async fn remote_urls(
        &self,
        remote: &str,
        push: bool,
    ) -> Result<Vec<String>, DiffError> {
        let mut args = vec!["remote", "get-url", "--all"];
        if push {
            args.push("--push");
        }
        args.extend(["--", remote]);
        let output = git_ref_output(&self.root, &args).await.map_err(|_| {
            DiffError::Git(
                "Git could not resolve this remote's destinations. Push it with Git.".to_owned(),
            )
        })?;
        // Git separates URLs with LF; a CR remains part of the destination.
        Ok(output
            .split_terminator('\n')
            .map(destination_without_credentials)
            .collect())
    }

    pub(crate) async fn fetch_transfer(&self, transfer: &GitTransfer) -> Result<(), DiffError> {
        if transfer.remote != "." {
            self.fetch_remote(&transfer.remote)
                .await
                .map_err(|error| fetch_error(transfer, error))?;
        }
        Ok(())
    }

    async fn advertised_destination(&self, transfer: &GitTransfer) -> Result<String, DiffError> {
        git(
            &self.root,
            &no_prompts(),
            &[
                "ls-remote",
                "--symref",
                "--",
                &transfer.remote,
                "HEAD",
                &transfer.confirmation.reference,
            ],
        )
        .await
        .map_err(|error| fetch_error(transfer, error))
    }

    /// Counts against the advertised destination and returns its listing for default detection.
    pub(crate) async fn transfer_counts(
        &self,
        transfer: &mut GitTransfer,
    ) -> Result<String, DiffError> {
        let advertised = self.advertised_destination(transfer).await?;
        let upstream = advertised_commit(&advertised, transfer)?;
        let Some(upstream) = upstream else {
            // Only a true first publication counts all history when no destination exists.
            transfer.ahead = if transfer.first_push {
                git_ref_output(
                    &self.root,
                    &["rev-list", "--count", &transfer.confirmation.commit, "--"],
                )
                .await?
                .trim()
                .parse()
                .map_err(|_| DiffError::Git("Git returned an invalid commit count.".to_owned()))?
            } else {
                0
            };
            transfer.behind = 0;
            return Ok(advertised);
        };
        if self
            .resolve(&format!("{upstream}^{{commit}}"))
            .await?
            .is_none()
        {
            self.fetch_remote_commit(&transfer.remote, upstream)
                .await
                .map_err(|error| fetch_error(transfer, error))?;
        }
        let range = format!("{}...{upstream}", transfer.confirmation.commit);
        let counts = git_ref_output(
            &self.root,
            &["rev-list", "--left-right", "--count", &range, "--"],
        )
        .await?;
        let mut counts = counts.split_whitespace();
        let invalid =
            |_| DiffError::Git("Git returned invalid ahead and behind counts.".to_owned());
        transfer.ahead = counts.next().unwrap_or("0").parse().map_err(invalid)?;
        transfer.behind = counts.next().unwrap_or("0").parse().map_err(invalid)?;
        Ok(advertised)
    }

    /// Whether `git merge` may autostash on `branch`: `merge.autoStash`, or `--autostash` among
    /// the branch's merge options.
    async fn may_autostash(&self, branch: &str) -> Result<bool, DiffError> {
        let default = git_config(&self.root, &["--bool", "--get", "merge.autoStash"]).await?;
        let options = git_config(
            &self.root,
            &["--get-all", &format!("branch.{branch}.mergeOptions")],
        )
        .await?;
        Ok(default.trim() == "true"
            || options
                .split_whitespace()
                .any(|option| option == "--autostash"))
    }

    pub(crate) async fn prepare_pull(&self) -> Result<GitTransfer, DiffError> {
        let (transfer, status) = self.destination().await?;
        check_pull_status(&status)?;
        self.fetch_transfer(&transfer).await?;
        let advertised = self.advertised_destination(&transfer).await?;
        let upstream = advertised_commit(&advertised, &transfer)?;
        if upstream.is_none() || self.resolve("@{upstream}").await?.as_deref() != upstream {
            return Err(DiffError::Git(
                "Git did not update this upstream. Pull it with Git.".to_owned(),
            ));
        }
        Ok(transfer)
    }

    /// Only local reads and merge run under the checkout lock, to completion with hooks.
    pub(crate) async fn pull(&self, fetched: &GitTransfer) -> Result<GitTransfer, DiffError> {
        let (mut transfer, status) = self.destination().await?;
        if transfer.confirmation.branch != fetched.confirmation.branch
            || transfer.confirmation.remote != fetched.confirmation.remote
            || transfer.confirmation.reference != fetched.confirmation.reference
        {
            return Err(DiffError::Git(
                "The branch or upstream changed. Pull again.".to_owned(),
            ));
        }
        check_pull_status(&status)?;
        transfer.ahead = status.ahead;
        transfer.behind = status.behind;
        if transfer.ahead > 0 && transfer.behind > 0 {
            return Err(DiffError::Diverged(Box::new(transfer), String::new()));
        }
        // Git overwrites ignored files where the upstream adds one; refuse as for a stash.
        // The checked commit is merged, even if a fetch moves the upstream meanwhile.
        let upstream = self
            .resolve("@{upstream}^{commit}")
            .await?
            .ok_or_else(|| DiffError::Git("The branch has no resolvable upstream.".to_owned()))?;
        // Git writes what the upstream changed since the branches parted, so commits only
        // this branch has change nothing.
        let base = git(&self.root, &[], &["merge-base", "HEAD", &upstream])
            .await
            .ok();
        let base = base.as_deref().map_or("HEAD", str::trim);
        if let (Some(base), Some(tree)) = (
            self.checkpointed(base).await?,
            self.checkpointed(&upstream).await?,
        ) {
            let changes = self.changes(&base, &tree, Whitespace::Shown).await?;
            let ignored = self.ignored_in_the_way(&changes, &[]).await?;
            if !ignored.is_empty() {
                return Err(DiffError::WouldOverwrite(ignored.into_iter().collect()));
            }
        }
        // An autostashing merge resets the checkout to HEAD first, so refuse what that reset
        // would overwrite, as a stash does.
        if self.may_autostash(&transfer.branch).await? {
            let blocked = {
                let _busy = self.busy.lock().await;
                let index = self.copy_index().await?;
                self.reset_in_the_way(&index).await?
            };
            if !blocked.is_empty() {
                return Err(DiffError::WouldOverwrite(blocked.into_iter().collect()));
            }
        }
        match user_git(
            &self.root,
            &["merge", "--ff-only", "--no-squash", "--", &upstream],
        )
        .await
        {
            Ok(output) => {
                let status = self.status().await.map_err(|error| {
                    DiffError::TransferFailed(without_credentials(&error.to_string()))
                })?;
                if !status.conflicts.is_empty() {
                    return Err(DiffError::TransferFailed(without_credentials(&format!(
                        "Git left conflicts after pulling. Resolve them in an editor or terminal; check Git's stash list for autostashed changes.\n{output}"
                    ))));
                }
                Ok(transfer)
            }
            Err(DiffError::Git(detail)) if detail.contains("would be overwritten by merge") => {
                Err(DiffError::WouldOverwrite(listed_files(&detail)))
            }
            Err(error) => Err(DiffError::TransferFailed(without_credentials(
                &error.to_string(),
            ))),
        }
    }
}

/// A resolved URL is one value: only its leading authority can contain credentials. An SSH
/// user name selects the account, so it stays part of the destination.
fn destination_without_credentials(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    if scheme.is_empty()
        || !scheme.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic()
                || (index > 0 && (byte.is_ascii_digit() || b"+-.".contains(&byte)))
        })
    {
        return url.to_owned();
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let Some(at) = rest[..authority_end].rfind('@') else {
        return url.to_owned();
    };
    let user = match scheme.to_ascii_lowercase().as_str() {
        "ssh" | "git+ssh" | "ssh+git" => {
            let user = rest[..at]
                .split_once(':')
                .map_or(&rest[..at], |(user, _)| user);
            format!("{user}@")
        }
        _ => String::new(),
    };
    format!("{scheme}://{user}{}", &rest[at + 1..])
}

fn advertised_commit<'a>(
    advertised: &'a str,
    transfer: &GitTransfer,
) -> Result<Option<&'a str>, DiffError> {
    let upstream = advertised
        .lines()
        .filter(|line| !line.starts_with("ref: "))
        .filter_map(|line| line.split_once('\t'))
        .find_map(|(oid, reference)| (reference == transfer.confirmation.reference).then_some(oid));
    if upstream.is_some_and(|oid| {
        oid.len() != transfer.confirmation.commit.len()
            || !oid.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err(DiffError::Git(
            "Git returned an invalid destination commit.".to_owned(),
        ));
    }
    Ok(upstream)
}

fn check_pull_status(status: &GitStatus) -> Result<(), DiffError> {
    if status.merging {
        return Err(DiffError::MergeInProgress);
    }
    if status.upstream.is_none() || status.gone {
        return Err(DiffError::Git(
            "The branch has no resolvable upstream.".to_owned(),
        ));
    }
    Ok(())
}

fn fetch_error(transfer: &GitTransfer, error: DiffError) -> DiffError {
    let detail = without_credentials(&match error {
        DiffError::Git(detail) => detail,
        error => error.to_string(),
    });
    if sign_in_failed(&detail) {
        DiffError::SignInFailed(Box::new(transfer.clone()), detail)
    } else {
        DiffError::FetchFailed(Box::new(transfer.clone()), detail)
    }
}

fn sign_in_failed(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    [
        "authentication failed",
        "could not read username",
        "could not read password",
        "permission denied (publickey",
        "terminal prompts disabled",
        "invalid username or token",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}
