use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use futures_util::future::BoxFuture;

pub fn t3_version() -> &'static str {
    include_str!("../../../t3.lock")
        .lines()
        .find_map(|line| line.strip_prefix("version "))
        .map_or("", str::trim)
}

pub(crate) const QUIET_ENV: [(&str, &str); 2] = [
    ("T3CODE_TELEMETRY_ENABLED", "false"),
    ("T3CODE_OTEL_SDK_DISABLED", "true"),
];

pub(crate) struct Server {
    child: tokio::process::Child,
    tree: Option<crate::platform::Tree>,
    pub origin: String,
    home: PathBuf,
    binary: PathBuf,
}

impl Server {
    pub async fn start(binary: &Path, home: &Path) -> Result<Self, String> {
        let runtime = home.join("userdata").join("server-runtime.json");
        stop_left_behind(binary, home).await;
        for attempt in 1..=START_ATTEMPTS {
            let _ = std::fs::remove_file(&runtime);
            let mut command = tokio::process::Command::new(binary);
            command
                .args(["serve", "--host", "127.0.0.1", "--no-browser", "--base-dir"])
                .arg(home)
                .current_dir(home)
                .env("T3CODE_HOME", home)
                .envs(QUIET_ENV)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let mut child = crate::spawn(&mut command)
                .map_err(|e| format!("Could not start T3 at {}: {e}", binary.display()))?;
            let tree = match child.id() {
                Some(pid) => match crate::platform::hold_tree(pid) {
                    Some(tree) => Some(tree),
                    None => {
                        let _ = child.kill().await;
                        return Err("Could not hold T3 and its agents in one job".to_owned());
                    }
                },
                None => None,
            };
            match wait_for_origin(&runtime, &mut child).await? {
                Some(origin) => {
                    return Ok(Self {
                        child,
                        tree,
                        origin,
                        home: home.to_owned(),
                        binary: binary.to_owned(),
                    });
                }
                None if attempt < START_ATTEMPTS => continue,
                None => break,
            }
        }
        Err(format!(
            "T3 {} ended {START_ATTEMPTS} times before it listened",
            t3_version()
        ))
    }

    pub fn issuer(
        &self,
    ) -> impl Fn() -> BoxFuture<'static, Result<String, String>> + Send + Sync + 'static {
        let (binary, home) = (self.binary.clone(), self.home.clone());
        move || Box::pin(issue(binary.clone(), home.clone(), ISSUE_WAIT))
    }

    pub async fn stop(&mut self) {
        if let Some(pid) = self.child.id() {
            crate::platform::terminate(pid);
        }
        if tokio::time::timeout(Duration::from_secs(5), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
        self.tree = None;
    }
}

const OWNER_TTL: &str = "24h";

const ISSUE_WAIT: Duration = Duration::from_secs(30);

pub(crate) async fn issue(
    binary: PathBuf,
    home: PathBuf,
    wait: Duration,
) -> Result<String, String> {
    let child = tokio::process::Command::new(&binary)
        .args([
            "auth",
            "session",
            "issue",
            "--label",
            "meta-harness",
            "--ttl",
            OWNER_TTL,
            "--token-only",
        ])
        .env("T3CODE_HOME", &home)
        .envs(QUIET_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Could not run T3: {e}"))?;
    let output = tokio::time::timeout(wait, child.wait_with_output())
        .await
        .map_err(|_| format!("T3 issued no session within {wait:?}"))?
        .map_err(|e| format!("Could not run T3: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "T3 issued no session: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

async fn stop_left_behind(binary: &Path, home: &Path) {
    let runtime = home.join("userdata").join("server-runtime.json");
    let written = std::fs::metadata(&runtime)
        .and_then(|meta| meta.modified())
        .ok();
    let Some(pid) = std::fs::read_to_string(&runtime)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value["pid"].as_u64())
        .and_then(|pid| u32::try_from(pid).ok())
    else {
        return;
    };
    let base = home.to_string_lossy();
    let arguments = [
        "serve",
        "--host",
        "127.0.0.1",
        "--no-browser",
        "--base-dir",
        &base,
    ];
    let runs_t3 = || crate::platform::runs(pid, binary, &arguments, written);
    if !runs_t3() {
        return;
    }
    crate::platform::terminate(pid);
    for _ in 0..50 {
        if !runs_t3() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if runs_t3() {
        crate::platform::kill(pid);
    }
}

const START_ATTEMPTS: u32 = 3;

async fn wait_for_origin(
    runtime: &Path,
    child: &mut tokio::process::Child,
) -> Result<Option<String>, String> {
    let pid = child.id();
    for _ in 0..300 {
        if let Ok(text) = std::fs::read_to_string(runtime)
            && let Some(origin) = listening_origin(&text, pid)?
        {
            return Ok(Some(origin));
        }
        if matches!(child.try_wait(), Ok(Some(_))) {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!(
        "T3 {} did not start listening within 30 s",
        t3_version()
    ))
}

pub(crate) fn listening_origin(text: &str, pid: Option<u32>) -> Result<Option<String>, String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Ok(None);
    };
    if value["pid"].as_u64() != pid.map(u64::from) {
        return Ok(None);
    }
    match value["serverVersion"].as_str() {
        Some(version) if version != t3_version() => Err(format!(
            "The t3 executable runs T3 {version}; Meta-Harness needs {}",
            t3_version()
        )),
        Some(_) => Ok(value["origin"].as_str().map(str::to_owned)),
        None => Ok(None),
    }
}
