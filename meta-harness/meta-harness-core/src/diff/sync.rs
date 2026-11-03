//! Explicit push and fast-forward pull through the host's Git and synchronous hooks.

use super::branches::without_credentials;
use super::process::{git_ref_output, user_git};
use super::stash::listed_files;
use super::*;

/// The destination and counts captured immediately before a push or pull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitTransfer {
    /// The local branch.
    pub branch: String,
    /// The named remote; no URL or credentials.
    pub remote: String,
    /// The upstream name as Git shows it.
    pub upstream: String,
    /// Commits to push.
    pub ahead: u64,
    /// Commits to pull.
    pub behind: u64,
    /// The branch had no upstream before this push.
    pub first_push: bool,
}

impl WorkingTree {
    /// The upstream's remote, falling back to origin only when it is configured.
    pub(super) async fn remote(&self, branch: Option<&str>) -> Result<Option<String>, DiffError> {
        if let Some(branch) = branch {
            let remote = git_ref_output(
                &self.root,
                &[
                    "for-each-ref",
                    "--format=%(upstream:remotename)",
                    &format!("refs/heads/{branch}"),
                ],
            )
            .await?;
            if !remote.trim_end_matches('\n').is_empty() {
                return Ok(Some(remote.trim_end_matches('\n').to_owned()));
            }
        }
        let remotes = git_ref_output(&self.root, &["remote"]).await?;
        Ok(remotes
            .lines()
            .any(|name| name == "origin")
            .then(|| "origin".to_owned()))
    }

    /// Explicit Fetch also works before the first push, using origin.
    pub async fn fetch_now(&self) -> Result<bool, DiffError> {
        let branch = self.branch().await?;
        let Some(remote) = self.remote(branch.as_deref()).await? else {
            return Ok(false);
        };
        if remote == "." {
            return Ok(false);
        }
        self.fetch_remote(&remote).await?;
        Ok(true)
    }

    async fn destination(&self) -> Result<(GitTransfer, String), DiffError> {
        let status = self.status().await?;
        let branch = status.branch.ok_or(DiffError::NoBranch)?;
        let remote = status.remote.ok_or_else(|| {
            DiffError::Git(
                "No remote: commits stay in this repository. Push needs a remote added in Git."
                    .to_owned(),
            )
        })?;
        let reference = git_ref_output(
            &self.root,
            &[
                "for-each-ref",
                "--format=%(upstream:remoteref)",
                &format!("refs/heads/{branch}"),
            ],
        )
        .await?;
        let first_push = status.upstream.is_none();
        let reference = if first_push {
            format!("refs/heads/{branch}")
        } else {
            reference.trim_end_matches('\n').to_owned()
        };
        Ok((
            GitTransfer {
                upstream: status
                    .upstream
                    .unwrap_or_else(|| format!("{remote}/{branch}")),
                branch,
                remote,
                ahead: status.ahead,
                behind: status.behind,
                first_push,
            },
            reference,
        ))
    }

    /// Pushes only the current branch to its upstream, never forcibly. A first push tracks
    /// origin under the same name. It does not acquire a checkout lock or change checkout files.
    pub async fn push(&self, confirmed_default: bool) -> Result<GitTransfer, DiffError> {
        let (mut transfer, reference) = self.destination().await?;
        self.fetch_now()
            .await
            .map_err(|error| remote_error(&transfer.remote, error))?;
        self.transfer_counts(&mut transfer, &reference).await?;
        if !confirmed_default {
            let env: Vec<(&str, OsString)> = process::NO_PROMPTS
                .iter()
                .map(|(name, value)| (*name, OsString::from(value)))
                .collect();
            let advertised = git(
                &self.root,
                &env,
                &["ls-remote", "--symref", "--", &transfer.remote, "HEAD"],
            )
            .await
            .map_err(|error| remote_error(&transfer.remote, error))?;
            if advertised
                .lines()
                .any(|line| line == format!("ref: {reference}\tHEAD"))
            {
                return Err(DiffError::PushDefault(
                    reference
                        .strip_prefix("refs/heads/")
                        .unwrap_or(&reference)
                        .to_owned(),
                    transfer,
                ));
            }
        }
        let spec = format!("refs/heads/{}:{reference}", transfer.branch);
        // Explicit source/destination avoid push.default and configured push refspecs. These
        // flags also prevent mirror, tags and force settings from broadening this one action.
        let result = user_git(
            &self.root,
            &[
                "push",
                "--porcelain",
                "--no-force",
                "--no-mirror",
                "--no-follow-tags",
                "--set-upstream",
                "--",
                &transfer.remote,
                &spec,
            ],
        )
        .await;
        match result {
            Ok(_) => Ok(transfer),
            Err(DiffError::Git(detail))
                if detail.contains("[rejected]")
                    && (detail.contains("non-fast-forward") || detail.contains("fetch first")) =>
            {
                // A remote may advance after the preflight fetch. Refresh its counts after
                // rejection too; a failed read does not turn stale counts into fresh ones.
                self.fetch_now()
                    .await
                    .map_err(|error| remote_error(&transfer.remote, error))?;
                self.transfer_counts(&mut transfer, &reference).await?;
                Err(DiffError::PushRejected(
                    transfer,
                    without_credentials(&detail),
                ))
            }
            Err(error) => Err(remote_error(&transfer.remote, error)),
        }
    }

    async fn transfer_counts(
        &self,
        transfer: &mut GitTransfer,
        reference: &str,
    ) -> Result<(), DiffError> {
        // Let Git map the destination through the configured fetch refspec, including before
        // an upstream is set. The overrides affect this read only.
        let upstream = git_ref_output(
            &self.root,
            &[
                "-c",
                &format!("branch.{}.remote={}", transfer.branch, transfer.remote),
                "-c",
                &format!("branch.{}.merge={reference}", transfer.branch),
                "for-each-ref",
                "--format=%(upstream)",
                &format!("refs/heads/{}", transfer.branch),
            ],
        )
        .await?;
        let upstream = upstream.trim_end_matches('\n');
        if upstream.is_empty() || self.resolve(upstream).await?.is_none() {
            transfer.ahead = git_ref_output(&self.root, &["rev-list", "--count", "HEAD", "--"])
                .await?
                .trim()
                .parse()
                .map_err(|_| DiffError::Git("Git returned an invalid commit count.".to_owned()))?;
            transfer.behind = 0;
            return Ok(());
        }
        let range = format!("HEAD...{upstream}");
        let counts = git_ref_output(
            &self.root,
            &["rev-list", "--left-right", "--count", &range, "--"],
        )
        .await?;
        let mut counts = counts.split_whitespace();
        transfer.ahead = counts.next().unwrap_or("0").parse().map_err(|_| {
            DiffError::Git("Git returned invalid ahead and behind counts.".to_owned())
        })?;
        transfer.behind = counts.next().unwrap_or("0").parse().map_err(|_| {
            DiffError::Git("Git returned invalid ahead and behind counts.".to_owned())
        })?;
        Ok(())
    }

    /// Fetches the upstream through the checked refspecs, then fast-forwards through the
    /// user's Git. The caller holds the checkout lock until Git and its hooks finish.
    pub async fn pull(&self) -> Result<GitTransfer, DiffError> {
        let (mut transfer, _) = self.destination().await?;
        if transfer.first_push {
            return Err(DiffError::Git(
                "This branch has no upstream. Push it first.".to_owned(),
            ));
        }
        self.fetch_now()
            .await
            .map_err(|error| remote_error(&transfer.remote, error))?;
        let status = self.status().await?;
        transfer.ahead = status.ahead;
        transfer.behind = status.behind;
        if transfer.ahead > 0 && transfer.behind > 0 {
            return Err(DiffError::PushRejected(transfer, String::new()));
        }
        let result = user_git(&self.root, &["merge", "--ff-only", "--", "@{upstream}"]).await;
        match result {
            Ok(_) => Ok(transfer),
            Err(DiffError::Git(detail)) if detail.contains("would be overwritten by merge") => {
                Err(DiffError::WouldOverwrite(listed_files(&detail)))
            }
            Err(error) => Err(remote_error(&transfer.remote, error)),
        }
    }
}

fn remote_error(remote: &str, error: DiffError) -> DiffError {
    let detail = without_credentials(&match error {
        DiffError::Git(detail) => detail,
        error => error.to_string(),
    });
    let lower = detail.to_ascii_lowercase();
    if [
        "authentication failed",
        "could not read username",
        "could not read password",
        "permission denied (publickey",
        "terminal prompts disabled",
        "invalid username or token",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        DiffError::SignInFailed(remote.to_owned(), detail)
    } else {
        DiffError::TransferFailed(detail)
    }
}
