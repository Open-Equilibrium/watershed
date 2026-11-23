#![cfg(windows)]
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use meta_harness_core::{
    AcpAgent, CONNECTION_FILE, ModelChoice, Permissions, Service, ServiceConfig, Snapshot,
    StartThread, TurnStatus, Workspace, remote,
};

const AGENT: &str = "scripted";

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("mh-{name}"));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn python() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("python.exe"))
        .find(|candidate| candidate.is_file())
        .expect("Python is on PATH")
}

fn config(home: &Path) -> ServiceConfig {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scripted_acp_agent.py");
    ServiceConfig {
        home: home.to_owned(),
        t3: PathBuf::from(std::env::var_os("META_HARNESS_T3").expect("META_HARNESS_T3 is set")),
        acp_agents: vec![AcpAgent {
            id: AGENT.to_owned(),
            command: python(),
            args: vec![
                script.to_string_lossy().into_owned(),
                home.to_string_lossy().into_owned(),
            ],
            models: vec![AGENT.to_owned()],
        }],
    }
}

fn start(text: &str) -> StartThread {
    StartThread {
        message_id: None,
        text: text.to_owned(),
        model: ModelChoice {
            instance: AGENT.to_owned(),
            model: AGENT.to_owned(),
            options: Vec::new(),
        },
        workspace: Workspace::Checkout,
        permissions: Permissions::FullAccess,
    }
}

async fn wait(thread: &remote::Thread, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let (snapshot, mut events) = thread.subscribe().await.unwrap();
            if done(&snapshot) {
                return snapshot;
            }
            events.recv().await;
        }
    })
    .await
    .expect("there within two minutes")
}

fn alive(pid: u32) -> bool {
    let listing = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listing.stdout).contains(&format!("\"{pid}\""))
}

fn readers(path: &Path) -> String {
    let listing = Command::new("icacls").arg(path).output().unwrap();
    String::from_utf8_lossy(&listing.stdout).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_windows_host_runs_a_turn_keeps_its_token_private_and_stops_t3_with_its_agents() {
    let dir = Dir::new();
    let repo = dir.0.join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "contract\n").unwrap();
    git(&repo, &["config", "user.name", "Contract"]);
    git(&repo, &["config", "user.email", "contract@example.invalid"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "start"]);
    let home = dir.0.join("home");
    let service = Service::start(config(&home))
        .await
        .expect("the service starts");
    let host = remote::Host::connect(&home, "contract")
        .await
        .expect("a client connects");

    let access = readers(&home.join(CONNECTION_FILE));
    let user = std::env::var("USERNAME").unwrap();
    assert!(access.contains(&user), "{access}");
    for other in ["Everyone", "BUILTIN\\Users", "Authenticated Users"] {
        assert!(!access.contains(other), "{other} reads the token: {access}");
    }

    let project = host.project(&repo).await.unwrap();
    let thread = project.start(start("append first")).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Completed)
    })
    .await;
    assert_eq!(
        std::fs::read_to_string(repo.join("contract.txt"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["first"]
    );

    thread.send("wait", false).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
    let runtime: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join("t3/userdata/server-runtime.json")).unwrap(),
    )
    .unwrap();
    let server = u32::try_from(runtime["pid"].as_u64().unwrap()).unwrap();
    assert!(alive(server));
    service.stop().await;
    for _ in 0..50 {
        if !alive(server) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!alive(server), "T3 still runs after shutdown");
}
