//! The gh CLI as Liquid runs it for pull requests: a host's login status and API calls, with gh
//! never prompting. A failure becomes a [`PullRequestError`] through [`gh_error`].

use super::github_remote::GITHUB;
use super::process::gh;
use super::pull_error::gh_error;
use super::*;
use serde_json::Value;

impl WorkingTree {
    /// Whether gh has a working login for `host`; a host other than github.com without one is
    /// not GitHub, and github.com without one needs a sign-in.
    pub(super) async fn gh_login(
        &self,
        program: &OsStr,
        host: &str,
    ) -> Result<bool, PullRequestError> {
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

    pub(super) async fn gh_api(
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

pub(super) fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}
