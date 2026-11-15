//! Spike: starts the pinned T3 Code server on loopback as its owner (ADR-0281).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// The T3 Code release Meta-Harness runs, and the SHA-256 of its macOS arm64 archive as the
/// release's `SHA256SUMS` lists it. A T3 upgrade changes both after the contract tests pass.
pub const T3_VERSION: &str = "0.1.9";

/// SHA-256 of `t3-0.1.9-darwin-arm64.tar.gz`, compiled in rather than read from the release,
/// so a replaced release asset fails the check.
pub const T3_DARWIN_ARM64_SHA256: &str =
    "7d2eb8c0687b70ca03a9d35498381852d3792983589e82d1fb1831c08f0b2301";

/// Environment that keeps a T3 server from sending anything to T3's analytics (ADR-0281).
/// Agent activity publishing stays off because Meta-Harness never runs `t3 connect`, which is
/// what stores the relay URL and credential that publishing needs.
pub const QUIET_ENV: [(&str, &str); 2] = [
    ("T3CODE_TELEMETRY_ENABLED", "false"),
    ("T3CODE_OTEL_SDK_DISABLED", "true"),
];

/// A running T3 server and the bearer token Meta-Harness uses as its owner.
pub struct T3Server {
    child: tokio::process::Child,
    /// `http://127.0.0.1:<port>`.
    pub origin: String,
    /// The T3 data folder (`T3CODE_HOME`).
    pub home: PathBuf,
    /// The pinned `t3` executable.
    pub binary: PathBuf,
    token: String,
}

impl T3Server {
    /// Starts `binary serve` on 127.0.0.1 with its data in `home`, waits until it listens and
    /// issues an owner bearer session through `t3 auth session issue`, which needs no person.
    pub async fn start(binary: &Path, home: &Path, cwd: &Path) -> Result<Self, String> {
        let runtime = home.join("userdata").join("server-runtime.json");
        let _ = std::fs::remove_file(&runtime);
        let mut command = tokio::process::Command::new(binary);
        command
            .args(["serve", "--host", "127.0.0.1", "--no-browser"])
            .current_dir(cwd)
            .env("T3CODE_HOME", home)
            .envs(QUIET_ENV)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let child = crate::spawn(&mut command).map_err(|e| format!("start t3: {e}"))?;
        let pid = child.id();
        let origin = wait_for_origin(&runtime, pid).await?;
        let token = run_cli(
            binary,
            home,
            &[
                "auth",
                "session",
                "issue",
                "--label",
                "meta-harness",
                "--ttl",
                "1h",
                "--token-only",
            ],
        )?
        .trim()
        .to_owned();
        Ok(Self {
            child,
            origin,
            home: home.to_owned(),
            binary: binary.to_owned(),
            token,
        })
    }

    /// The owner's bearer token; never logged.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Runs a `t3` CLI command against this server's data folder.
    pub fn cli(&self, args: &[&str]) -> Result<String, String> {
        run_cli(&self.binary, &self.home, args)
    }

    /// Stops the server.
    pub async fn stop(mut self) {
        let _ = self.child.kill().await;
    }
}

async fn wait_for_origin(runtime: &Path, pid: Option<u32>) -> Result<String, String> {
    for _ in 0..300 {
        if let Ok(text) = std::fs::read_to_string(runtime)
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
            && value["pid"].as_u64().map(|p| p as u32) == pid
            && value["serverVersion"].as_str() == Some(T3_VERSION)
            && let Some(origin) = value["origin"].as_str()
        {
            return Ok(origin.to_owned());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("t3 did not start listening within 30 s".to_owned())
}

fn run_cli(binary: &Path, home: &Path, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new(binary)
        .args(args)
        .env("T3CODE_HOME", home)
        .envs(QUIET_ENV)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("run t3 {}: {e}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "t3 {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}
