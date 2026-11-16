//! Meta-Harness's calls of T3 Code's command line in T3's data folder (ADR-0281): the pairing codes
//! that other hosts redeem, and the environments this host links (ADR-0285). They need no running
//! T3 server.

use std::path::Path;
use std::process::Stdio;

use serde_json::Value;

use super::server::QUIET_ENV;

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
    let output = child
        .wait_with_output()
        .await
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

/// A single-use pairing code, valid for five minutes, that another host's T3 link redeems
/// (ADR-0288).
pub(crate) async fn pairing_code(binary: &Path, home: &Path) -> Result<String, String> {
    let text = run(
        binary,
        home,
        "issue a pairing code",
        &[
            "auth",
            "pairing",
            "create",
            "--json",
            "--ttl",
            "5m",
            "--label",
            "meta-harness",
        ],
    )
    .await?;
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| value["credential"].as_str().map(str::to_owned))
        .ok_or_else(|| "T3 issued no pairing code.".to_owned())
}
