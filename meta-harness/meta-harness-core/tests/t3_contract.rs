//! Contract tests against the pinned T3 Code server (ADR-0281), with a scripted ACP agent in
//! place of a model. They need the pinned `t3` executable in `META_HARNESS_T3`; CI installs it.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use meta_harness_core::{
    AcpAgent, AgentEvent, Host, HostConfig, ModelChoice, Permissions, Request, Snapshot,
    StartThread, Thread, ThreadEvent, TurnStatus, Workspace,
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

fn worktree() -> Workspace {
    Workspace::NewWorktree {
        base: "main".to_owned(),
    }
}

fn start(text: &str, workspace: Workspace, permissions: Permissions) -> StartThread {
    StartThread {
        text: text.to_owned(),
        model: ModelChoice {
            instance: AGENT.to_owned(),
            model: AGENT.to_owned(),
            options: Vec::new(),
        },
        workspace,
        permissions,
    }
}

/// Waits until the thread's snapshot satisfies `done`, failing after a minute.
async fn wait(thread: &Thread, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let waited = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let (snapshot, mut events) = thread.subscribe();
            if done(&snapshot) {
                return snapshot;
            }
            events.recv().await;
        }
    })
    .await;
    waited.unwrap_or_else(|_| panic!("not there within a minute: {:#?}", thread.snapshot()))
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
        .start(start("append first", worktree(), Permissions::FullAccess))
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
        .start(start("append once", worktree(), Permissions::FullAccess))
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
        .start(start("ask", Workspace::Checkout, Permissions::Supervised))
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

#[tokio::test(flavor = "multi_thread")]
async fn statistics_count_a_thread_s_turns_and_tool_calls() {
    let (_dir, repo, home) = fixture();
    let host = host(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("append one", worktree(), Permissions::FullAccess))
        .await
        .unwrap();
    wait(&thread, completed(1)).await;
    let statistics = thread.statistics();
    assert_eq!(statistics.turns.len(), 1);
    assert_eq!(statistics.turns[0].prompt, "append one");
    let edits: u32 = statistics
        .tools
        .iter()
        .map(|t| t.succeeded + t.failed + t.cancelled + t.open)
        .sum();
    assert!(edits >= 1, "the append's tool call counts: {statistics:?}");
    assert_eq!(
        statistics.turns[0].model_ms, None,
        "T3 reports no model time (ADR-0293)"
    );
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_sent_while_a_turn_runs_queues_and_can_be_removed() {
    let (_dir, repo, home) = fixture();
    let host = host(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("wait", Workspace::Checkout, Permissions::FullAccess))
        .await
        .unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
    thread.send("append later", false).await.unwrap();
    let queued = wait(&thread, |s| {
        s.turns
            .iter()
            .any(|t| t.status == TurnStatus::Queued && !t.prompt.is_empty())
    })
    .await;
    let turn = queued
        .turns
        .iter()
        .find(|t| t.status == TurnStatus::Queued)
        .unwrap();
    assert_eq!(turn.prompt, "append later");
    thread.cancel_queued(turn.number).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().all(|t| t.status != TurnStatus::Queued)
    })
    .await;
    thread.stop().await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().all(|t| t.status == TurnStatus::Aborted)
    })
    .await;
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_worktree_s_only_thread_can_delete_the_worktree() {
    let (_dir, repo, home) = fixture();
    let host = host(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("append one", worktree(), Permissions::FullAccess))
        .await
        .unwrap();
    wait(&thread, completed(1)).await;
    let listed = project.threads().await.unwrap();
    let summary = listed.iter().find(|t| t.id == thread.id()).unwrap();
    let folder = summary.worktree.clone().unwrap();
    assert!(project.only_thread_in_worktree(summary).await.unwrap());
    project.delete(summary, true).await.unwrap();
    assert!(!folder.exists());
    assert!(
        project
            .threads()
            .await
            .unwrap()
            .iter()
            .all(|t| t.id != thread.id())
    );
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forking_a_response_starts_a_thread_with_the_turns_up_to_it() {
    let (_dir, repo, home) = fixture();
    let host = host(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("append first", worktree(), Permissions::FullAccess))
        .await
        .unwrap();
    wait(&thread, completed(1)).await;
    thread.send("append second", false).await.unwrap();
    wait(&thread, completed(2)).await;
    let fork = project.open(&thread.fork(1).await.unwrap()).await.unwrap();
    let forked = wait(&fork, |s| {
        s.transcript.iter().any(|i| i.text == "append first")
    })
    .await;
    assert!(forked.transcript.iter().all(|i| i.text != "append second"));
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_writer_model_is_set_once_for_every_thread() {
    let (_dir, _repo, home) = fixture();
    let host = host(&home).await;
    let writer = ModelChoice {
        instance: "claudeAgent".to_owned(),
        model: "claude-haiku-5-5".to_owned(),
        options: vec![("effort".to_owned(), "high".to_owned())],
    };
    host.set_writer_model(&writer).await.unwrap();
    assert_eq!(host.writer_model().await.unwrap(), Some(writer));
    assert!(
        host.providers()
            .await
            .unwrap()
            .iter()
            .any(|p| p.instance == AGENT)
    );
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
