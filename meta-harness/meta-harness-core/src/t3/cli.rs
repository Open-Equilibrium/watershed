//! Meta-Harness's calls of T3 Code's command line in T3's data folder (ADR-0281): the pairing codes
//! that other hosts redeem, and the environments this host links (ADR-0285). They need no running
//! T3 server.

use std::path::Path;
use std::process::Stdio;

use serde_json::Value;

use super::server::QUIET_ENV;
use crate::hosts::Access;
use crate::util::TIMEOUT;

/// The orchestration scopes an agent sign-in at `access` needs, as T3's own `mcpClientScopes` gives
/// them (ADR-0300): a pairing code carries exactly these, so T3 grants no more than the level.
fn scopes(access: Access) -> &'static [&'static str] {
    match access {
        Access::ReadOnly => &["orchestration:read"],
        Access::FullAccess => &[
            "orchestration:read",
            "orchestration:operate",
            "settings:write",
            "source-control:write",
        ],
        Access::ApprovalRequired | Access::AutoAcceptEdits | Access::Auto => {
            &["orchestration:read", "orchestration:operate"]
        }
    }
}

/// Runs `binary` with `args` in T3's data folder `home`, and answers its standard output. `what`
/// names the step in the error, since `args` may hold a pairing code.
async fn run(binary: &Path, home: &Path, what: &str, args: &[&str]) -> Result<String, String> {
    let mut command = tokio::process::Command::new(binary);
    command
        .args(args)
        .env("T3CODE_HOME", home)
        .envs(QUIET_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = crate::spawn(&mut command).map_err(|e| format!("Could not run T3: {e}"))?;
    // Dropped when it gives up, which kills the child (kill_on_drop).
    let output = tokio::time::timeout(TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("T3 could not {what}: it took longer than 30 seconds."))?
        .map_err(|e| format!("Could not run T3: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "T3 could not {what}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// A single-use pairing code, valid for five minutes, that another host's T3 link redeems at
/// `access` (ADR-0288, ADR-0300).
pub(crate) async fn pairing_code(
    binary: &Path,
    home: &Path,
    access: Access,
) -> Result<String, String> {
    let mut args = vec![
        "auth",
        "pairing",
        "create",
        "--json",
        "--ttl",
        "5m",
        "--label",
        "meta-harness",
    ];
    for scope in scopes(access) {
        args.extend(["--scope", scope]);
    }
    let text = run(binary, home, "issue a pairing code", &args).await?;
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| value["credential"].as_str().map(str::to_owned))
        .ok_or_else(|| "T3 issued no pairing code.".to_owned())
}

/// A link that T3 keeps, as its command line lists it.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Linked {
    /// The linked environment's id, which unlinking names.
    pub(crate) environment_id: String,
    /// The addresses the link was made at.
    pub(crate) urls: Vec<String>,
    /// `reachable`, `unreachable` or `expired`, as T3 found it now.
    pub(crate) status: String,
}

/// The environments linked in T3's data folder `home`.
pub(crate) async fn links(binary: &Path, home: &Path) -> Result<Vec<Linked>, String> {
    let text = run(
        binary,
        home,
        "list the linked environments",
        &["environment", "list", "--json"],
    )
    .await?;
    serde_json::from_str(&text).map_err(|e| format!("T3 listed its links unexpectedly: {e}"))
}

/// Links T3 in `home` to the environment at `url`, with the pairing code `code` the peer issued
/// (ADR-0288). The link's agents have `access`, a T3 access level.
pub(crate) async fn link(
    binary: &Path,
    home: &Path,
    url: &str,
    code: &str,
    access: &str,
) -> Result<(), String> {
    run(
        binary,
        home,
        "link the environment",
        &[
            "environment",
            "link",
            url,
            "--pairing-code",
            code,
            "--access",
            access,
        ],
    )
    .await
    .map(|_| ())
}

/// Forgets the linked environment `environment_id` in T3's data folder `home`.
pub(crate) async fn unlink(binary: &Path, home: &Path, environment_id: &str) -> Result<(), String> {
    run(
        binary,
        home,
        "unlink the environment",
        &["environment", "unlink", environment_id],
    )
    .await
    .map(|_| ())
}
