//! The pinned T3 Code server Meta-Harness starts on loopback as its owner (ADR-0281).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// The T3 Code release Meta-Harness runs, as `meta-harness/t3.lock` pins it.
pub fn t3_version() -> &'static str {
    include_str!("../../../t3.lock")
        .lines()
        .find_map(|line| line.strip_prefix("version "))
        .map_or("", str::trim)
}

/// Environment that keeps T3 from sending anything to its analytics (ADR-0281). Agent activity
/// publishing stays off because Meta-Harness never runs `t3 connect`, which stores what
/// publishing needs.
const QUIET_ENV: [(&str, &str); 2] = [
    ("T3CODE_TELEMETRY_ENABLED", "false"),
    ("T3CODE_OTEL_SDK_DISABLED", "true"),
];

/// A running T3 server.
pub(crate) struct Server {
    child: tokio::process::Child,
    /// `http://127.0.0.1:<port>`.
    pub origin: String,
    home: PathBuf,
    binary: PathBuf,
}

impl Server {
    /// Starts `binary serve` on 127.0.0.1 with its data in `home` and waits until it listens.
    /// A T3 server that a dead Meta-Harness left running in `home` is stopped first, since it
    /// holds the folder.
    pub async fn start(binary: &Path, home: &Path) -> Result<Self, String> {
        let runtime = home.join("userdata").join("server-runtime.json");
        stop_left_behind(binary, &runtime).await;
        let _ = std::fs::remove_file(&runtime);
        let mut command = tokio::process::Command::new(binary);
        command
            .args(["serve", "--host", "127.0.0.1", "--no-browser"])
            .current_dir(home)
            .env("T3CODE_HOME", home)
            .envs(QUIET_ENV)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let child = crate::spawn(&mut command).map_err(|e| format!("Could not start T3: {e}"))?;
        let origin = wait_for_origin(&runtime, child.id()).await?;
        Ok(Self {
            child,
            origin,
            home: home.to_owned(),
            binary: binary.to_owned(),
        })
    }

    /// A new owner bearer token; `t3 auth session issue` needs no person. Never logged.
    pub fn token(&self) -> Result<String, String> {
        let args = [
            "auth",
            "session",
            "issue",
            "--label",
            "meta-harness",
            "--ttl",
            "1h",
            "--token-only",
        ];
        let output = std::process::Command::new(&self.binary)
            .args(args)
            .env("T3CODE_HOME", &self.home)
            .envs(QUIET_ENV)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("Could not run T3: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "T3 issued no session: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    /// Stops the server as `stop_left_behind` does: TERM, then KILL if it has not exited after
    /// five seconds. T3 stops its runs and their provider processes on TERM.
    pub async fn stop(mut self) {
        if let Some(pid) = self
            .child
            .id()
            .and_then(|pid| rustix::process::Pid::from_raw(i32::try_from(pid).ok()?))
        {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
        }
        if tokio::time::timeout(Duration::from_secs(5), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
    }
}

/// Stops the `t3 serve` whose pid `runtime` records, if it still runs `binary`.
async fn stop_left_behind(binary: &Path, runtime: &Path) {
    let Some(pid) = std::fs::read_to_string(runtime)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value["pid"].as_i64())
        .and_then(|pid| rustix::process::Pid::from_raw(i32::try_from(pid).ok()?))
    else {
        return;
    };
    let runs_t3 = |pid: rustix::process::Pid| {
        std::process::Command::new("ps")
            .args(["-o", "command=", "-p", &pid.as_raw_nonzero().to_string()])
            .output()
            .is_ok_and(|out| {
                let command = String::from_utf8_lossy(&out.stdout);
                command.starts_with(&*binary.to_string_lossy()) && command.contains(" serve ")
            })
    };
    if !runs_t3(pid) {
        return;
    }
    let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
    for _ in 0..50 {
        if !runs_t3(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
}

async fn wait_for_origin(runtime: &Path, pid: Option<u32>) -> Result<String, String> {
    for _ in 0..300 {
        if let Ok(text) = std::fs::read_to_string(runtime)
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
            && value["pid"].as_u64() == pid.map(u64::from)
            && value["serverVersion"].as_str() == Some(t3_version())
            && let Some(origin) = value["origin"].as_str()
        {
            return Ok(origin.to_owned());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!(
        "T3 {} did not start listening within 30 s",
        t3_version()
    ))
}
