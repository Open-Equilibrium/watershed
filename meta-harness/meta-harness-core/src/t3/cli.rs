use std::path::Path;
use std::process::Stdio;

use serde_json::Value;

use super::server::QUIET_ENV;
use crate::hosts::Access;
use crate::util::TIMEOUT;

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

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Linked {
    pub(crate) environment_id: String,
    pub(crate) urls: Vec<String>,
    pub(crate) status: String,
}

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

const MCP_CLIENT_SUBJECT: &str = "mcp-client";

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Session {
    pub(crate) session_id: String,
    pub(crate) subject: String,
    pub(crate) client: SessionClient,
}

#[derive(serde::Deserialize)]
pub(crate) struct SessionClient {
    pub(crate) label: Option<String>,
}

impl Session {
    pub(crate) fn is_link_of(&self, client: &str) -> bool {
        self.subject == MCP_CLIENT_SUBJECT && self.client.label.as_deref() == Some(client)
    }
}

pub(crate) async fn sessions(binary: &Path, home: &Path) -> Result<Vec<Session>, String> {
    let text = run(
        binary,
        home,
        "list the sessions",
        &["auth", "session", "list", "--json"],
    )
    .await?;
    serde_json::from_str(&text).map_err(|e| format!("T3 listed its sessions unexpectedly: {e}"))
}

pub(crate) async fn revoke_session(
    binary: &Path,
    home: &Path,
    session_id: &str,
) -> Result<(), String> {
    run(
        binary,
        home,
        "revoke a session",
        &["auth", "session", "revoke", session_id],
    )
    .await
    .map(|_| ())
}

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
