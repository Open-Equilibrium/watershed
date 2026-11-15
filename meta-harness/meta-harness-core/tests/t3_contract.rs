//! Contract tests against the pinned T3 Code server (ADR-0281), with a scripted ACP agent in
//! place of a model. They need the pinned `t3` executable in `META_HARNESS_T3`; CI installs it.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use meta_harness_core::{
    AcpAgent, AgentEvent, Host, HostConfig, Permissions, Request, Snapshot, StartThread, Thread,
    ThreadEvent, TurnStatus, Workspace,
};

const AGENT: &str = "scripted";

fn t3() -> PathBuf {
    PathBuf::from(
        std::env::var_os("META_HARNESS_T3")
            .expect("META_HARNESS_T3 names the pinned t3 executable"),
    )
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A repository with one commit on `main`, and a fresh T3 data folder beside it.
fn fixture() -> (tempdir::Dir, PathBuf, PathBuf) {
    let dir = tempdir::Dir::new();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "contract\n").unwrap();
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "start",
        ],
    );
    let home = dir.path().join("t3");
    (dir, repo, home)
}

async fn host(home: &Path) -> Host {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scripted_acp_agent.py");
    Host::start(HostConfig {
        binary: t3(),
        home: home.to_owned(),
        acp_agents: vec![AcpAgent {
            id: AGENT.to_owned(),
            command: PathBuf::from("/usr/bin/python3"),
            args: vec![script.to_string_lossy().into_owned()],
            models: vec![AGENT.to_owned()],
        }],
    })
    .await
    .expect("T3 starts")
}

/// Waits until the thread's snapshot satisfies `done`, failing after a minute.
async fn wait(thread: &Thread, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let (snapshot, mut events) = thread.subscribe();
            if done(&snapshot) {
                return snapshot;
            }
            events.recv().await;
        }
    })
    .await
    .expect("the thread gets there within a minute")
}

/// Whether `n` turns completed with their checkpoints.
fn completed(n: usize) -> impl Fn(&Snapshot) -> bool {
    move |s| {
        s.turns
            .iter()
            .filter(|t| t.status == TurnStatus::Completed && t.checkpoint)
            .count()
            >= n
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_runs_turns_and_reverts_files_in_its_worktree() {
    let (_dir, repo, home) = fixture();
    let host = host(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(StartThread {
            text: "append first".to_owned(),
            instance: AGENT.to_owned(),
            model: AGENT.to_owned(),
            workspace: Workspace::NewWorktree {
                base: "main".to_owned(),
            },
            permissions: Permissions::FullAccess,
        })
        .await
        .unwrap();
    wait(&thread, completed(1)).await;
    thread.send("append second", false).await.unwrap();
    wait(&thread, completed(2)).await;

    let worktree = thread.worktree().expect("the thread has its own worktree");
    let file = worktree.join("contract.txt");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\nsecond\n");
    let turns = thread.turns();
    assert_eq!(turns.len(), 2);
    assert_eq!(
        (turns[1].number, turns[1].added, turns[1].removed),
        (2, 1, 0)
    );
    assert!(thread.diff(1, 2).await.unwrap().contains("+second"));

    assert_eq!(thread.revert(2, true).await.unwrap(), "append second");
    wait(&thread, |s| s.turns.len() == 1).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\n");
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_stops_the_t3_server_a_dead_host_left_behind() {
    let (_dir, _repo, home) = fixture();
    let first = host(&home).await;
    // A Meta-Harness that dies leaves its T3 server holding the data folder.
    std::mem::forget(first);
    let second = host(&home).await;
    second.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_snapshot_names_the_events_it_already_contains() {
    let (_dir, repo, home) = fixture();
    let host = host(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(StartThread {
            text: "append once".to_owned(),
            instance: AGENT.to_owned(),
            model: AGENT.to_owned(),
            workspace: Workspace::NewWorktree {
                base: "main".to_owned(),
            },
            permissions: Permissions::FullAccess,
        })
        .await
        .unwrap();
    let (before, mut events) = thread.subscribe();
    wait(&thread, completed(1)).await;
    // Every event after the snapshot comes later in the stream, so applying them on top of it
    // counts each once (ADR-0290).
    let first = events.recv().await.expect("events followed the snapshot");
    assert!(first.sequence > before.sequence);
    let after = thread.snapshot();
    assert!(after.transcript.iter().any(|item| item.text == "Appended."));
    let events: Vec<ThreadEvent> = std::iter::from_fn(|| events.try_recv().ok()).collect();
    assert!(events.iter().all(|e| e.sequence <= after.sequence));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, AgentEvent::TurnCompleted { .. }))
    );
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_supervised_thread_waits_for_the_person_to_approve() {
    let (_dir, repo, home) = fixture();
    let host = host(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(StartThread {
            text: "ask".to_owned(),
            instance: AGENT.to_owned(),
            model: AGENT.to_owned(),
            workspace: Workspace::Checkout,
            permissions: Permissions::Supervised,
        })
        .await
        .unwrap();
    let (asked, mut events) = {
        let snapshot = wait(&thread, |s| !s.requests.is_empty()).await;
        (snapshot.requests[0].clone(), thread.subscribe().1)
    };
    let Request::Approval { options, .. } = &asked.request else {
        panic!("an approval, not {asked:?}");
    };
    assert!(options.iter().any(|o| o.decision == "accept"));
    thread.respond(&asked.id, "accept").await.unwrap();
    let resolved = events.recv().await.unwrap();
    let mut seen = vec![resolved.event];
    let done = wait(&thread, completed(1)).await;
    seen.extend(std::iter::from_fn(|| events.try_recv().ok()).map(|e| e.event));
    assert!(seen.contains(&AgentEvent::RequestResolved {
        request: asked.id.clone()
    }));
    assert!(done.requests.is_empty());
    assert!(done.transcript.iter().any(|item| item.text == "Approved."));
    host.shutdown().await;
}

mod tempdir {
    /// A temporary folder removed on drop.
    pub struct Dir(std::path::PathBuf);

    impl Dir {
        pub fn new() -> Self {
            let mut bytes = [0u8; 8];
            getrandom::fill(&mut bytes).unwrap();
            let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            // A short path: T3 derives socket and worktree paths from it.
            let path = std::env::temp_dir().join(format!("mh-{name}"));
            std::fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }

        pub fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
