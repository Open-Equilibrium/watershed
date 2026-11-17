//! Contract tests against the pinned T3 Code server (ADR-0281), with a scripted ACP agent in
//! place of a model. They need the pinned `t3` executable in `META_HARNESS_T3`; CI installs it.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use futures_util::FutureExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use meta_harness_core::{
    Access, CONNECTION_FILE, Change, ClientKind, Service, ServiceConfig, remote, t3_version,
};
use meta_harness_core::{
    AcpAgent, AgentEvent, GitAction, GitActionRequest, GitProgress, Group, Host, HostConfig,
    LineCount, ModelChoice, Permissions, Relation, Request, Snapshot, StartThread, Thread,
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
    // T3's commits need an identity; the host's may have none.
    git(&repo, &["config", "user.name", "Contract"]);
    git(&repo, &["config", "user.email", "contract@example.invalid"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "start"]);
    let home = dir.path().join("t3");
    (dir, repo, home)
}

async fn host(home: &Path) -> Host {
    Host::start(HostConfig {
        binary: t3(),
        home: home.to_owned(),
        acp_agents: agents(home),
    })
    .await
    .expect("T3 starts")
}

/// Starts a service on `home`, which keeps T3's data in `home/t3`, and connects a client to it.
/// Its remote listener binds loopback.
async fn start_service(home: &Path) -> (Service, remote::Host) {
    let service = Service::start_with_remote(
        service_config(home),
        Some(vec![std::net::Ipv4Addr::LOCALHOST.into()]),
    )
    .await
    .expect("the service starts");
    let host = remote::Host::connect(home, "contract")
        .await
        .expect("a client connects");
    (service, host)
}

fn agents(home: &Path) -> Vec<AcpAgent> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scripted_acp_agent.py");
    vec![AcpAgent {
        id: AGENT.to_owned(),
        command: PathBuf::from("/usr/bin/python3"),
        // The script ignores its arguments; the home folder names this test's agents in the
        // process list.
        args: vec![
            script.to_string_lossy().into_owned(),
            home.to_string_lossy().into_owned(),
        ],
        models: vec![AGENT.to_owned()],
    }]
}

fn service_config(home: &Path) -> ServiceConfig {
    ServiceConfig {
        home: home.to_owned(),
        t3: t3(),
        acp_agents: agents(home),
    }
}

/// A free loopback port, for a remote listener.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
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
async fn wait(thread: &remote::Thread, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let waited = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let (snapshot, mut events) = thread.subscribe().await.unwrap();
            if done(&snapshot) {
                return snapshot;
            }
            events.recv().await;
        }
    })
    .await;
    match waited {
        Ok(snapshot) => snapshot,
        Err(_) => panic!("not there within a minute: {:#?}", thread.snapshot().await),
    }
}

/// `wait` for a thread of the core Host, for the one test that stays on it.
async fn wait_core(thread: &Thread, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
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
    let (service, host) = start_service(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("append first", worktree(), Permissions::FullAccess))
        .await
        .unwrap();
    wait(&thread, completed(1)).await;
    thread.send("append second", false).await.unwrap();
    wait(&thread, completed(2)).await;

    let worktree = thread
        .worktree()
        .await
        .unwrap()
        .expect("the thread has its own worktree");
    let file = worktree.join("contract.txt");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\nsecond\n");
    let turns = thread.turns().await.unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(
        (turns[1].number, turns[1].added, turns[1].removed),
        (2, 1, 0)
    );
    assert!(thread.diff(2, 2, false).await.unwrap().contains("+second"));
    // The thread's Git reads its own worktree, where the append is still uncommitted.
    let changed = thread.git().status().await.unwrap().changes;
    assert_eq!(
        changed.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        ["contract.txt"]
    );

    assert_eq!(thread.revert(2, true).await.unwrap(), "append second");
    wait(&thread, |s| s.turns.len() == 1).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\n");
    // Turn 1 reverts to the thread's start, which removes the file it created.
    assert_eq!(thread.revert(1, true).await.unwrap(), "append first");
    wait(&thread, |s| s.turns.is_empty()).await;
    assert!(!file.exists());

    // The branch picker chose the project folder's checkout; Git follows it at once.
    thread.set_branch("main", Some(&repo)).await.unwrap();
    assert_eq!(
        thread.worktree().await.unwrap().as_deref(),
        Some(repo.as_path())
    );
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_after_a_rewind_reverts_and_diffs_from_its_visible_predecessor() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("append first", worktree(), Permissions::FullAccess))
        .await
        .unwrap();
    wait(&thread, completed(1)).await;
    thread.send("append second", false).await.unwrap();
    wait(&thread, completed(2)).await;
    assert_eq!(thread.revert(2, true).await.unwrap(), "append second");
    wait(&thread, |s| s.turns.len() == 1).await;

    // T3 numbers the next run after the rolled back one, so the new turn is number 3.
    thread.send("append third", false).await.unwrap();
    wait(&thread, completed(2)).await;
    let numbers: Vec<u64> = thread
        .turns()
        .await
        .unwrap()
        .iter()
        .map(|t| t.number)
        .collect();
    assert_eq!(numbers, [1, 3]);
    let file = thread
        .worktree()
        .await
        .unwrap()
        .unwrap()
        .join("contract.txt");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\nthird\n");
    assert!(thread.diff(3, 3, false).await.unwrap().contains("+third"));
    assert_eq!(thread.revert(3, true).await.unwrap(), "append third");
    wait(&thread, |s| s.turns.len() == 1).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\n");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_stops_t3_and_the_agent_processes_it_started() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("wait", Workspace::Checkout, Permissions::FullAccess))
        .await
        .unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
    let mut agents = Vec::new();
    for _ in 0..100 {
        agents = processes_naming(&home);
        if !agents.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!agents.is_empty(), "T3 started its agent");
    service.stop().await;
    // T3 stops its provider processes as it stops; an agent left running would wait forever.
    for _ in 0..50 {
        agents.retain(|pid| running(*pid));
        if agents.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for pid in &agents {
        kill(*pid);
    }
    assert!(
        agents.is_empty(),
        "agent processes still run after shutdown"
    );
}

/// The processes whose command line names `home`, such as the agents T3 started in it.
fn processes_naming(home: &Path) -> Vec<u32> {
    let listing = Command::new("ps")
        .args(["-axww", "-o", "pid=,command="])
        .output()
        .unwrap();
    let marker = home.to_string_lossy();
    String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter(|line| line.contains(&*marker))
        .filter_map(|line| line.split_whitespace().next()?.parse().ok())
        .collect()
}

fn running(pid: u32) -> bool {
    rustix::process::Pid::from_raw(pid as i32)
        .is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
}

fn kill(pid: u32) {
    if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
    }
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
async fn t3_starts_in_a_data_folder_that_does_not_exist_yet() {
    let (_dir, _repo, home) = fixture();
    // Liquid's first start: no data folder yet and no ACP agents to write into it.
    let host = Host::start(HostConfig {
        binary: t3(),
        home,
        acp_agents: Vec::new(),
    })
    .await
    .expect("T3 starts");
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_server_record_never_stops_another_process() {
    let (_dir, _repo, home) = fixture();
    // A server that died left its record behind; its pid now belongs to another program.
    let mut other = Command::new("sleep").arg("60").spawn().unwrap();
    std::fs::create_dir_all(home.join("userdata")).unwrap();
    std::fs::write(
        home.join("userdata").join("server-runtime.json"),
        serde_json::json!({"pid": other.id()}).to_string(),
    )
    .unwrap();
    let host = host(&home).await;
    assert!(
        other.try_wait().unwrap().is_none(),
        "the other program still runs"
    );
    host.shutdown().await;
    other.kill().unwrap();
    other.wait().unwrap();
}

// Stays on the core Host: the service delivers a subscription's events from its own task, so the
// remote stream can lag the snapshots its calls return, and the remote API cannot wait until the
// stream has caught up.
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
    wait_core(&thread, completed(1)).await;
    // The stream applies each change and sends its events under the lock that snapshots take, so
    // once `wait` read the turn complete, every later event is already in `events` (ADR-0290).
    // The turn may also have ended before `before`; then its completion is in the snapshot instead.
    let received: Vec<ThreadEvent> = std::iter::from_fn(|| events.try_recv().ok()).collect();
    let after = thread.snapshot();
    assert!(
        received
            .iter()
            .all(|e| before.sequence < e.sequence && e.sequence <= after.sequence)
    );
    assert!(
        received
            .windows(2)
            .all(|pair| pair[0].sequence <= pair[1].sequence)
    );
    let finished_before = before
        .turns
        .iter()
        .any(|t| t.status == TurnStatus::Completed);
    let finished_after = received
        .iter()
        .any(|e| matches!(e.event, AgentEvent::TurnCompleted { .. }));
    assert!(
        finished_before != finished_after,
        "the completion is in the snapshot or the stream, exactly once"
    );
    assert!(after.transcript.iter().any(|item| item.text == "Appended."));
    host.shutdown().await;
}

/// The events `events` already holds, without waiting for more. The remote stream has no
/// non-blocking receive, so this takes only the events that are ready now.
fn buffered(events: &mut remote::Stream<ThreadEvent>) -> Vec<ThreadEvent> {
    std::iter::from_fn(|| events.recv().now_or_never().flatten()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_supervised_thread_waits_for_the_person_to_approve() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("ask", Workspace::Checkout, Permissions::Supervised))
        .await
        .unwrap();
    let (asked, mut events) = {
        let snapshot = wait(&thread, |s| !s.requests.is_empty()).await;
        (
            snapshot.requests[0].clone(),
            thread.subscribe().await.unwrap().1,
        )
    };
    let Request::Approval { options, .. } = &asked.request else {
        panic!("an approval, not {asked:?}");
    };
    assert!(options.iter().any(|o| o.decision == "accept"));
    thread.respond(&asked.id, "accept").await.unwrap();
    let resolved = events.recv().await.unwrap();
    let mut seen = vec![resolved.event];
    let done = wait(&thread, completed(1)).await;
    seen.extend(buffered(&mut events).into_iter().map(|e| e.event));
    assert!(seen.contains(&AgentEvent::RequestResolved {
        request: asked.id.clone()
    }));
    assert!(done.requests.is_empty());
    assert!(done.transcript.iter().any(|item| item.text == "Approved."));
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn statistics_count_a_thread_s_turns_and_tool_calls() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("append one", worktree(), Permissions::FullAccess))
        .await
        .unwrap();
    wait(&thread, completed(1)).await;
    let statistics = thread.statistics().await.unwrap();
    assert_eq!(statistics.turns.len(), 1);
    assert_eq!(statistics.turns[0].prompt, "append one");
    let edits: u32 = statistics
        .tools
        .iter()
        .map(|t| t.succeeded + t.failed + t.cancelled + t.open)
        .sum();
    assert!(edits >= 1, "the append's tool call counts: {statistics:?}");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_sent_while_a_turn_runs_queues_and_can_be_removed() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
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
    thread.send("append more", false).await.unwrap();
    let queued = wait(&thread, |s| {
        s.turns
            .iter()
            .filter(|t| t.status == TurnStatus::Queued && !t.prompt.is_empty())
            .count()
            == 2
    })
    .await;
    let [later, more] = [0, 1].map(|i| {
        queued
            .turns
            .iter()
            .filter(|t| t.status == TurnStatus::Queued)
            .nth(i)
            .unwrap()
            .clone()
    });
    assert_eq!(
        (later.prompt.as_str(), more.prompt.as_str()),
        ("append later", "append more")
    );
    // Subscribers learn of an edit, so Liquid's queue shows the new text.
    thread
        .edit_queued(more.number, "append edited")
        .await
        .unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.prompt == "append edited")
    })
    .await;
    thread.cancel_queued(more.number).await.unwrap();
    thread.cancel_queued(later.number).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().all(|t| t.status != TurnStatus::Queued)
    })
    .await;
    thread.stop().await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().all(|t| t.status == TurnStatus::Aborted)
    })
    .await;
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_worktree_s_only_thread_can_delete_the_worktree() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
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
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forking_a_response_starts_a_thread_with_the_turns_up_to_it() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
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
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_writer_model_is_set_once_for_every_thread() {
    let (_dir, _repo, home) = fixture();
    let (service, host) = start_service(&home).await;
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
    service.stop().await;
}

/// Runs a stacked action and returns how it ended.
async fn run(git: &remote::Git, action: GitAction, message: Option<&str>) -> GitProgress {
    let mut progress = git
        .run(GitActionRequest {
            action,
            message: message.map(str::to_owned),
            files: None,
            feature_branch: false,
        })
        .await
        .unwrap();
    let mut last = None;
    while let Some(step) = progress.recv().await {
        last = Some(step);
    }
    last.expect("the action reports how it ended")
}

#[tokio::test(flavor = "multi_thread")]
async fn git_commits_branches_and_pushes_as_t3_does() {
    let (dir, repo, home) = fixture();
    let origin = dir.path().join("origin.git");
    git(dir.path(), &["init", "-q", "--bare", "origin.git"]);
    git(
        &repo,
        &["remote", "add", "origin", &origin.to_string_lossy()],
    );
    git(&repo, &["push", "-q", "-u", "origin", "main"]);
    let (service, host) = start_service(&home).await;
    let project = host.project(&repo).await.unwrap();
    let git = project.git();

    std::fs::write(repo.join("README.md"), "contract\nchanged\n").unwrap();
    let status = git.status().await.unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert!(status.repository && status.remote && status.default_branch);
    assert_eq!(status.changes.len(), 1);
    assert!(
        git.uncommitted_diff(false)
            .await
            .unwrap()
            .contains("+changed")
    );

    git.create_branch("feature").await.unwrap();
    let refs = git.refs("feat", 0, 50).await.unwrap();
    assert!(refs.refs.iter().any(|r| r.name == "feature" && r.current));
    let ended = run(&git, GitAction::CommitPush, Some("Change the readme")).await;
    assert!(matches!(ended, GitProgress::Finished { .. }), "{ended:?}");
    let status = git.status().await.unwrap();
    assert!(status.changes.is_empty() && status.upstream);
    assert_eq!(status.ahead, 0);
    let pushed = Command::new("git")
        .args(["log", "-1", "--format=%s", "feature"])
        .current_dir(&origin)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&pushed.stdout).trim(),
        "Change the readme"
    );

    git.switch("main").await.unwrap();
    assert_eq!(git.status().await.unwrap().branch.as_deref(), Some("main"));
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_changes_and_the_branch_diff_compare_the_branch_with_its_base_as_t3_does() {
    let (dir, repo, home) = fixture();
    let origin = dir.path().join("origin.git");
    git(dir.path(), &["init", "-q", "--bare", "origin.git"]);
    git(
        &repo,
        &["remote", "add", "origin", &origin.to_string_lossy()],
    );
    git(&repo, &["push", "-q", "-u", "origin", "main"]);
    let (service, host) = start_service(&home).await;
    let checkout = host.project(&repo).await.unwrap().git();

    checkout.create_branch("feature").await.unwrap();
    std::fs::write(repo.join("README.md"), "contract\nbranch\n").unwrap();
    git(&repo, &["commit", "-qam", "Change the readme"]);
    std::fs::write(repo.join("README.md"), "contract\nbranch\nmore\n").unwrap();

    let status = checkout.status().await.unwrap();
    assert_eq!(
        status.working_tree_lines,
        LineCount {
            added: 1,
            removed: 0
        }
    );
    let branch = status
        .branch_changes
        .expect("T3 counts the branch against its base");
    assert_eq!(branch.base.as_deref(), Some("origin/main"));
    assert_eq!(
        branch.lines,
        LineCount {
            added: 2,
            removed: 0
        }
    );
    let diff = checkout.branch_diff(false).await.unwrap();
    assert!(diff.contains("+branch") && diff.contains("+more"), "{diff}");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn git_initializes_a_folder_without_a_repository() {
    let (dir, _repo, home) = fixture();
    let plain = dir.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    let (service, host) = start_service(&home).await;
    let git = host.project(&plain).await.unwrap().git();
    assert!(!git.status().await.unwrap().repository);
    git.init().await.unwrap();
    assert!(git.status().await.unwrap().repository);
    service.stop().await;
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

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_details_card_reads_t3_and_a_fork_merges_back() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    assert!(!host.environment_name().await.unwrap().is_empty());
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("append first", worktree(), Permissions::FullAccess))
        .await
        .unwrap();
    wait(&thread, completed(1)).await;
    assert!(thread.servers().await.unwrap().current().is_empty());
    assert!(thread.automations().await.unwrap().current().is_empty());
    thread.usage_limits().await.unwrap();

    let fork_id = thread.fork(1).await.unwrap();
    let fork = project.open(&fork_id).await.unwrap();
    fork.send("append in fork", false).await.unwrap();
    wait(&fork, |s| {
        s.turns.iter().any(|t| {
            t.prompt == "append in fork" && t.status == TurnStatus::Completed && t.checkpoint
        })
    })
    .await;

    let parent = thread.lineage().await.unwrap();
    assert!(parent.rows.iter().any(|row| row.thread == fork_id
        && row.relation == Relation::Fork
        && row.group == Group::Related));
    let lineage = fork.lineage().await.unwrap();
    let parent_row = lineage.rows.iter().find(|row| row.thread == thread.id());
    assert!(parent_row.is_some_and(|row| row.merge_target));
    assert!(lineage.can_merge);
    fork.merge_back().await.unwrap();
    service.stop().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn the_service_admits_only_clients_with_its_token_and_owns_its_folder() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, _repo, _) = fixture();
    let home = dir.path().join("meta");
    let service = Service::start(service_config(&home)).await.unwrap();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&home), 0o700);
    assert_eq!(mode(&home.join(CONNECTION_FILE)), 0o600);

    assert!(
        remote::Host::connect_to(&service.url, "mh-wrong", "test")
            .await
            .is_err()
    );
    let client = remote::Host::connect(&home, "test").await.unwrap();
    assert_eq!(client.status().await.unwrap().t3_version, t3_version());
    let second = Service::start(service_config(&home)).await;
    assert!(second.is_err_and(|e| e.contains("Another Meta-Harness service")));

    service.stop().await;
    assert!(!home.join(CONNECTION_FILE).exists());
    assert!(
        client.status().await.is_err(),
        "a stopped service answers nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_service_stops_serving_its_clients() {
    let (dir, _repo, _) = fixture();
    let home = dir.path().join("meta");
    let service = Service::start(service_config(&home)).await.unwrap();
    let (url, token) = (service.url.clone(), service.token.clone());
    let client = remote::Host::connect(&home, "test").await.unwrap();
    drop(service);
    assert!(!home.join(CONNECTION_FILE).exists());
    tokio::time::timeout(Duration::from_secs(10), async {
        while client.status().await.is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        while remote::Host::connect_to(&url, &token, "test").await.is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a dropped service ends its connections and stops listening");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approved_configuration_reaches_the_harness_homes_t3_runs_them_in() {
    let (dir, _repo, _) = fixture();
    let home = dir.path().join("meta");
    let service = Service::start(service_config(&home)).await.unwrap();
    let client = remote::Host::connect(&home, "liquid").await.unwrap();
    let number = client
        .propose(
            "Test first",
            vec![Change::Instruction {
                name: "rules".to_owned(),
                text: Some("Run the tests.".to_owned()),
            }],
        )
        .await
        .unwrap();
    let claude = home.join("harness/claude");
    let codex = home.join("harness/codex");
    assert!(
        !claude.join("CLAUDE.md").exists(),
        "nothing before approval"
    );
    assert_eq!(client.approve(number).await.unwrap().revision, 1);
    assert_eq!(
        std::fs::read_to_string(claude.join("CLAUDE.md")).unwrap(),
        "Run the tests.\n"
    );
    assert_eq!(
        std::fs::read_to_string(codex.join("AGENTS.md")).unwrap(),
        "Run the tests.\n"
    );
    let log = client.log().await.unwrap();
    assert!(log.iter().all(|entry| entry.client == "liquid"), "{log:?}");

    // T3 keeps the homes in its settings, so it starts Claude Code and Codex in them.
    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join("t3/userdata/settings.json")).unwrap(),
    )
    .unwrap();
    let instances = &settings["providerInstances"];
    assert_eq!(
        instances["codex"]["config"]["homePath"],
        codex.to_string_lossy().as_ref()
    );
    assert_eq!(
        instances["claudeAgent"]["config"]["homePath"],
        claude.to_string_lossy().as_ref()
    );
    assert_eq!(
        instances[AGENT]["driver"], "acpRegistry",
        "the ACP agent stays"
    );
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_goes_on_after_the_service_restarts() {
    let (dir, repo, _) = fixture();
    let home = dir.path().join("meta");
    let (service, client) = start_service(&home).await;
    let mut statuses = client
        .project(&repo)
        .await
        .unwrap()
        .git()
        .watch()
        .await
        .unwrap();
    let before = client.status().await.unwrap();

    service.stop().await;
    assert!(client.status().await.is_err(), "nothing answers meanwhile");
    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        while statuses.recv().await.is_some() {}
    })
    .await;
    assert!(ended.is_ok(), "a stream of the lost connection ends");

    // The login service starts it again with a new port and token; the client finds both.
    let service = Service::start(service_config(&home)).await.unwrap();
    let after = client.status().await.unwrap();
    assert_eq!(after.revision, before.revision);
    assert!(client.project(&repo).await.is_ok());
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_service_refuses_a_client_that_names_no_api_version_or_another() {
    use tokio_tungstenite::tungstenite::{Error, client::IntoClientRequest, http::StatusCode};
    let (dir, _repo, _) = fixture();
    let home = dir.path().join("meta");
    let service = Service::start(service_config(&home)).await.unwrap();
    for version in [None, Some("2")] {
        let mut request = service.url.as_str().into_client_request().unwrap();
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {}", service.token).parse().unwrap(),
        );
        if let Some(version) = version {
            request
                .headers_mut()
                .insert("meta-harness-api", version.parse().unwrap());
        }
        let refused = match tokio_tungstenite::connect_async(request).await {
            Err(Error::Http(response)) => response,
            Ok(_) => panic!("the service admits a client of version {version:?}"),
            Err(other) => panic!("{other}"),
        };
        assert_eq!(refused.status(), StatusCode::UPGRADE_REQUIRED);
        // The refusal names the version the service speaks.
        assert_eq!(refused.headers()["meta-harness-api"], "1");
    }
    service.stop().await;
}

/// Whether a client named `name` with `token` connects to the remote listener at `url`.
async fn connects(url: &str, token: &str, name: &str) -> bool {
    remote::Host::connect_to(url, token, name).await.is_ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn turning_remote_access_off_ends_the_connections_it_admitted() {
    let (dir, _repo, _home) = fixture();
    let (_service, local) = start_service(&dir.path().join("meta")).await;
    let port = free_port();
    local.remote_on(port).await.unwrap();
    let url = format!("ws://127.0.0.1:{port}");
    let code = local
        .invite(ClientKind::Device, Access::Auto)
        .await
        .unwrap();
    let token = remote::pair(&url, &code, "phone").await.unwrap().token;
    let device = remote::Host::connect_to(&url, &token, "phone")
        .await
        .unwrap();
    device.status().await.unwrap();

    local.remote_off().await.unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        while device.status().await.is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        ended.is_ok(),
        "the device's connection ended with remote access"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_pairs_once_and_keeps_its_access_across_a_restart_while_a_host_only_asks_for_a_pairing_code()
 {
    let (_dir, _repo, home) = fixture();
    let (service, local) = start_service(&home).await;
    let port = free_port();
    let status = local.remote_on(port).await.unwrap();
    let url = format!("ws://127.0.0.1:{port}");
    assert_eq!(status.addresses, [url.as_str()]);

    let code = local
        .invite(ClientKind::Device, Access::Auto)
        .await
        .unwrap();
    let token = remote::pair(&url, &code, "phone").await.unwrap().token;
    let used = remote::pair(&url, &code, "tablet").await.unwrap_err();
    assert_eq!(used, "That pairing code is wrong, already used or expired.");
    let wrong = remote::pair(&url, "AAAA-AAAA", "tablet").await.unwrap_err();
    assert_eq!(wrong, used);

    let device = remote::Host::connect_to(&url, &token, "phone")
        .await
        .unwrap();
    device.status().await.unwrap();
    device.config().await.unwrap();
    // Remote access, invitations, the host list and host agents stay with this computer.
    assert!(device.remote_status().await.is_err());
    assert!(
        device
            .invite(ClientKind::Device, Access::Auto)
            .await
            .is_err()
    );
    assert!(device.hosts().await.is_err());
    assert!(device.host_agents(true).await.is_err());
    assert!(!may_tunnel(&url, &token).await, "a device is no tunnel");

    // Remote access that is on, and the device's pairing, survive a restart.
    drop(local);
    service.stop().await;
    let (_service, local) = start_service(&home).await;
    let status = local.remote_status().await.unwrap().expect("still on");
    assert_eq!(status.port, port);
    assert!(connects(&url, &token, "phone").await);
    local.revoke_client("phone").await.unwrap();
    assert!(!connects(&url, &token, "phone").await);

    let host_code = local.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let paired = remote::pair(&url, &host_code, "other").await.unwrap();
    let (name, host_token) = (paired.name, paired.token);
    assert_eq!(name, local.environment_name().await.unwrap().to_lowercase());
    let host = remote::Host::connect_to(&url, &host_token, "other")
        .await
        .unwrap();
    assert!(
        host.status().await.is_err(),
        "a host asks for a T3 pairing code only"
    );
    assert!(!host.t3_pairing().await.unwrap().is_empty());
    assert!(!connects(&url, &"0".repeat(64), "other").await);

    local.remote_off().await.unwrap();
    assert!(local.remote_status().await.unwrap().is_none());
}

/// T3's own list of the environments linked in `home`.
fn t3_environments(home: &Path) -> Vec<serde_json::Value> {
    let output = Command::new(t3())
        .args(["environment", "list", "--json"])
        .env("T3CODE_HOME", home.join("t3"))
        .env("T3CODE_TELEMETRY_ENABLED", "false")
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Whether T3's settings file in `home` turns linked environments on. T3 leaves out a setting at
/// its default, which is off.
fn linked_environments_setting(home: &Path) -> bool {
    let text = std::fs::read_to_string(home.join("t3/userdata/settings.json")).unwrap();
    serde_json::from_str::<serde_json::Value>(&text).unwrap()["linkedEnvironmentsEnabled"]
        .as_bool()
        .unwrap_or(false)
}

/// The tunnel port and link time that Meta-Harness keeps for the host `name` in `home`.
fn peer_row(home: &Path, name: &str) -> (u16, Option<i64>) {
    rusqlite::Connection::open(home.join("meta-harness.sqlite"))
        .unwrap()
        .query_row(
            "SELECT tunnel_port, linked_ms FROM peer WHERE name = ?1",
            [name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_links_another_hosts_t3_through_a_tunnel_behind_the_switch_and_removing_it_unlinks_it()
 {
    let (dir, _repo, _home) = fixture();
    let (home_a, home_b) = (dir.path().join("a"), dir.path().join("b"));
    let (_a, a) = start_service(&home_a).await;
    let (_b, b) = start_service(&home_b).await;
    let (port_a, port_b) = (free_port(), free_port());
    a.remote_on(port_a).await.unwrap();
    b.remote_on(port_b).await.unwrap();
    let address = format!("ws://127.0.0.1:{port_b}");
    let waiting = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let refused = a
        .host_add(&address, &waiting, Some(Access::Auto))
        .await
        .unwrap_err();
    assert!(refused.contains("meta host agents on"), "{refused}");
    a.host_agents(true).await.unwrap();
    b.host_agents(true).await.unwrap();
    assert!(linked_environments_setting(&home_a));

    let code = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let (name, _) = a
        .host_add(&address, &code, Some(Access::Auto))
        .await
        .unwrap();
    assert_eq!(name, b.environment_name().await.unwrap().to_lowercase());
    let listed = a.hosts().await.unwrap();
    assert_eq!(listed.hosts.len(), 1);
    assert!(listed.hosts[0].reachable);
    assert_eq!(listed.hosts[0].access, Access::Auto);
    // T3 itself lists the link to B's environment, which answers.
    let linked = t3_environments(&home_a);
    let environment = std::fs::read_to_string(home_b.join("t3/userdata/environment-id")).unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0]["environmentId"], environment.trim());
    assert_eq!(linked[0]["status"], "reachable");

    // The name is already linked, so the add fails after B has paired with A, and B keeps a pairing.
    let again = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let error = a
        .host_add(&address, &again, Some(Access::Auto))
        .await
        .unwrap_err();
    let me = a.environment_name().await.unwrap().to_lowercase();
    assert!(error.contains(&format!("meta host revoke {me}")), "{error}");

    let (tunnel, _) = peer_row(&home_a, &name);
    a.host_remove(&name).await.unwrap();
    assert!(t3_environments(&home_a).is_empty(), "T3 forgets the link");
    assert!(a.hosts().await.unwrap().hosts.is_empty());
    assert!(std::net::TcpStream::connect(("127.0.0.1", tunnel)).is_err());
    a.host_agents(false).await.unwrap();
    assert!(!linked_environments_setting(&home_a));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_without_an_access_level_takes_the_level_its_host_invited() {
    let (dir, _repo, _home) = fixture();
    let (home_a, home_b) = (dir.path().join("a"), dir.path().join("b"));
    let (_a, a) = start_service(&home_a).await;
    let (_b, b) = start_service(&home_b).await;
    let port_b = free_port();
    a.remote_on(free_port()).await.unwrap();
    b.remote_on(port_b).await.unwrap();
    a.host_agents(true).await.unwrap();
    b.host_agents(true).await.unwrap();
    let url_b = format!("ws://127.0.0.1:{port_b}");
    let code = b
        .invite(ClientKind::Host, Access::FullAccess)
        .await
        .unwrap();
    let (_, access) = a.host_add(&url_b, &code, None).await.unwrap();
    assert_eq!(access, Access::FullAccess);
    let listed = a.hosts().await.unwrap();
    assert_eq!(listed.hosts[0].access, Access::FullAccess);
    assert!(listed.hosts[0].reachable, "a full-access link answers");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_invited_at_approval_required_cannot_link_at_auto_but_links_at_that_level() {
    let (dir, _repo, _home) = fixture();
    let (home_a, home_b) = (dir.path().join("a"), dir.path().join("b"));
    let (_a, a) = start_service(&home_a).await;
    let (_b, b) = start_service(&home_b).await;
    let port_b = free_port();
    a.remote_on(free_port()).await.unwrap();
    b.remote_on(port_b).await.unwrap();
    a.host_agents(true).await.unwrap();
    b.host_agents(true).await.unwrap();
    let url_b = format!("ws://127.0.0.1:{port_b}");
    // The tunnel of b refuses T3's approval of a link above the level b invited (ADR-0301).
    let code = b
        .invite(ClientKind::Host, Access::ApprovalRequired)
        .await
        .unwrap();
    assert!(a.host_add(&url_b, &code, Some(Access::Auto)).await.is_err());
    let code = b
        .invite(ClientKind::Host, Access::ApprovalRequired)
        .await
        .unwrap();
    let (_, access) = a
        .host_add(&url_b, &code, Some(Access::ApprovalRequired))
        .await
        .unwrap();
    assert_eq!(access, Access::ApprovalRequired);
    assert!(a.hosts().await.unwrap().hosts[0].reachable);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_invited_read_only_gets_a_pairing_code_that_grants_orchestration_read_only() {
    let (dir, _repo, _home) = fixture();
    let home = dir.path().join("b");
    let (_b, b) = start_service(&home).await;
    let port = free_port();
    b.remote_on(port).await.unwrap();
    let url = format!("ws://127.0.0.1:{port}");
    let code = b.invite(ClientKind::Host, Access::ReadOnly).await.unwrap();
    let paired = remote::pair(&url, &code, "other").await.unwrap();
    assert_eq!(paired.access, Access::ReadOnly);
    let host = remote::Host::connect_to(&url, &paired.token, "other")
        .await
        .unwrap();
    let credential = host.t3_pairing().await.unwrap();
    let answer = exchange(&t3_address(&home), &credential, None);
    assert_eq!(status_of(&answer), 200, "{answer}");
    assert_eq!(body_json(&answer)["scope"], "orchestration:read");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_link_older_than_25_days_is_made_again_when_the_service_starts() {
    let (dir, _repo, _home) = fixture();
    let (home_a, home_b) = (dir.path().join("a"), dir.path().join("b"));
    let (a_service, a) = start_service(&home_a).await;
    let (_b, b) = start_service(&home_b).await;
    let port_b = free_port();
    b.remote_on(port_b).await.unwrap();
    a.host_agents(true).await.unwrap();
    b.host_agents(true).await.unwrap();
    let code = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let (name, _) = a
        .host_add(
            &format!("ws://127.0.0.1:{port_b}"),
            &code,
            Some(Access::ReadOnly),
        )
        .await
        .unwrap();
    drop(a);
    a_service.stop().await;

    // The link was made 26 days ago; the service makes it again as it starts.
    let aged = peer_row(&home_a, &name).1.unwrap() - 26 * 24 * 3_600_000;
    rusqlite::Connection::open(home_a.join("meta-harness.sqlite"))
        .unwrap()
        .execute("UPDATE peer SET linked_ms = ?1", [aged])
        .unwrap();
    let (_service, a) = start_service(&home_a).await;
    let relinked = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let linked = a.hosts().await.unwrap().hosts[0].linked_ms.unwrap_or(0);
            if linked > aged as u64 {
                return linked;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await;
    assert!(relinked.is_ok(), "linked again within a minute");
    assert_eq!(t3_environments(&home_a)[0]["status"], "reachable");
}

/// Whether a connection with `token` to the remote listener at `url` may tunnel: it must name the
/// tunnel header, which only a host may use.
async fn may_tunnel(url: &str, token: &str) -> bool {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = url.into_client_request().unwrap();
    let headers = request.headers_mut();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    headers.insert("meta-harness-api", "1".parse().unwrap());
    headers.insert("meta-harness-tunnel", "t3".parse().unwrap());
    tokio_tungstenite::connect_async(request).await.is_ok()
}

/// Two services, each with its remote listener on loopback and agents from other hosts on, where
/// `a` has linked `b` at the default access.
struct Linked {
    a: (Service, remote::Host),
    b: (Service, remote::Host),
    name: String,
    url_b: String,
}

async fn linked(dir: &Path) -> Linked {
    let (home_a, home_b) = (dir.join("a"), dir.join("b"));
    let (a_service, a) = start_service(&home_a).await;
    let (b_service, b) = start_service(&home_b).await;
    let port_b = free_port();
    a.remote_on(free_port()).await.unwrap();
    b.remote_on(port_b).await.unwrap();
    a.host_agents(true).await.unwrap();
    b.host_agents(true).await.unwrap();
    let url_b = format!("ws://127.0.0.1:{port_b}");
    let code = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let (name, _) = a.host_add(&url_b, &code, Some(Access::Auto)).await.unwrap();
    Linked {
        a: (a_service, a),
        b: (b_service, b),
        name,
        url_b,
    }
}

/// The headers of a form body.
const FORM: &[(&str, &str)] = &[("Content-Type", "application/x-www-form-urlencoded")];
/// The headers of a JSON body.
const JSON: &[(&str, &str)] = &[("Content-Type", "application/json")];

/// Sends one HTTP/1.1 request with `headers` and `body` to `addr` and answers everything sent
/// back, the connection closing after the answer.
fn http(addr: &str, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> String {
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    std::io::Write::write_all(&mut stream, request.as_bytes()).unwrap();
    let mut answer = String::new();
    let _ = std::io::Read::read_to_string(&mut stream, &mut answer);
    answer
}

/// Posts `body` with `headers` to `path` at `addr`, which must answer with success; answers its
/// JSON body.
fn post(addr: &str, path: &str, headers: &[(&str, &str)], body: &str) -> serde_json::Value {
    let answer = http(addr, "POST", path, headers, body);
    assert!((200..300).contains(&status_of(&answer)), "{path}: {answer}");
    body_json(&answer)
}

/// The status code of an HTTP answer.
fn status_of(answer: &str) -> u16 {
    answer
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

/// The JSON object in an HTTP answer's body.
fn body_json(answer: &str) -> serde_json::Value {
    let body = &answer[answer.find('{').unwrap()..=answer.rfind('}').unwrap()];
    serde_json::from_str(body).unwrap()
}

/// The address T3 in `home` listens on, from the runtime record it writes.
fn t3_address(home: &Path) -> String {
    let text = std::fs::read_to_string(home.join("t3/userdata/server-runtime.json")).unwrap();
    let origin = serde_json::from_str::<serde_json::Value>(&text).unwrap()["origin"]
        .as_str()
        .unwrap()
        .to_owned();
    origin.trim_start_matches("http://").to_owned()
}

/// Exchanges the bootstrap pairing `credential` at T3 in `origin` for a bearer token, asking for
/// `scope` when given; answers the HTTP answer.
fn exchange(origin: &str, credential: &str, scope: Option<&str>) -> String {
    let mut form = format!(
        "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange\
         &subject_token={credential}\
         &subject_token_type=urn%3At3%3Aparams%3Aoauth%3Atoken-type%3Aenvironment-bootstrap\
         &requested_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Aaccess_token"
    );
    if let Some(scope) = scope {
        form.push_str(&format!("&scope={scope}"));
    }
    http(origin, "POST", "/oauth/token", FORM, &form)
}

/// Percent-encodes `text` for a form body, keeping only the unreserved characters.
fn form_value(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `bytes` in unpadded base64url, as PKCE writes a code challenge.
fn base64url(bytes: &[u8]) -> String {
    const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            text.push(char::from(LETTERS[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    text
}

/// The first `key` in the query of `url`.
fn query(url: &str, key: &str) -> String {
    let query = url.split_once('?').unwrap().1;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
        .unwrap()
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tunnel_passes_only_the_paths_a_link_uses() {
    let (dir, _repo, _home) = fixture();
    let link = linked(dir.path()).await;
    let port = peer_row(&dir.path().join("a"), &link.name).0;
    let addr = format!("127.0.0.1:{port}");
    for (method, path) in [
        ("GET", "/api/auth/session"),
        ("POST", "/api/auth/browser-session"),
        ("POST", "/oauth/token"),
        ("POST", "/api/auth/websocket-ticket"),
        ("GET", "/api/projects"),
        ("GET", "/api/orchestration/shell"),
        ("GET", "/ws"),
        ("GET", "/"),
    ] {
        let answer = http(&addr, method, path, &[], "");
        assert_eq!(status_of(&answer), 403, "{method} {path}");
    }
    let descriptor = http(&addr, "GET", "/.well-known/t3/environment", &[], "");
    assert_eq!(status_of(&descriptor), 200, "the link's descriptor");
    let mcp = http(&addr, "GET", "/mcp", &[], "");
    assert_ne!(status_of(&mcp), 403, "the link's MCP path");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_code_from_a_host_token_redeems_only_orchestration_scopes() {
    let (dir, _repo, _home) = fixture();
    let (home_b, port_b) = (dir.path().join("b"), free_port());
    let (_b, b) = start_service(&home_b).await;
    b.remote_on(port_b).await.unwrap();
    let url = format!("ws://127.0.0.1:{port_b}");
    let code = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let token = remote::pair(&url, &code, "other").await.unwrap().token;
    let host = remote::Host::connect_to(&url, &token, "other")
        .await
        .unwrap();
    let origin = t3_address(&home_b);
    // Each exchange spends its code, so each one takes a new code from the host token.
    for scope in ["terminal:operate", "filesystem:write", "settings:write"] {
        let credential = host.t3_pairing().await.unwrap();
        let answer = exchange(&origin, &credential, Some(scope));
        assert_ne!(status_of(&answer), 200, "{scope} is refused: {answer}");
    }
    let credential = host.t3_pairing().await.unwrap();
    let answer = exchange(&origin, &credential, None);
    assert_eq!(status_of(&answer), 200, "{answer}");
    assert_eq!(
        body_json(&answer)["scope"],
        "orchestration:read orchestration:operate"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_client_closes_the_connections_it_has_open() {
    let (dir, _repo, _home) = fixture();
    let link = linked(dir.path()).await;
    let (_a, a) = &link.a;
    let (_b, b) = &link.b;
    let code = b.invite(ClientKind::Device, Access::Auto).await.unwrap();
    let token = remote::pair(&link.url_b, &code, "phone")
        .await
        .unwrap()
        .token;
    let device = remote::Host::connect_to(&link.url_b, &token, "phone")
        .await
        .unwrap();
    device.status().await.unwrap();
    // A connection through a's tunnel to b's T3 stays open between requests.
    let port = peer_row(&dir.path().join("a"), &link.name).0;
    let mut tunnel = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let request =
        format!("GET /.well-known/t3/environment HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    tunnel.write_all(request.as_bytes()).await.unwrap();
    let mut answer = [0u8; 12];
    tunnel.read_exact(&mut answer).await.unwrap();
    assert_eq!(&answer, b"HTTP/1.1 200");

    b.revoke_client("phone").await.unwrap();
    b.revoke_client(&a.environment_name().await.unwrap().to_lowercase())
        .await
        .unwrap();
    assert!(
        device.status().await.is_err(),
        "the device's connection ended"
    );
    let ended = tokio::time::timeout(Duration::from_secs(20), async {
        let mut rest = Vec::new();
        tunnel.read_to_end(&mut rest).await
    })
    .await;
    assert!(ended.is_ok(), "the tunnel's connection ended");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_agents_mcp_request_reaches_t3_through_the_tunnel() {
    use sha2::{Digest, Sha256};
    let (dir, _repo, _home) = fixture();
    let link = linked(dir.path()).await;
    let (_b, b) = &link.b;
    let port = peer_row(&dir.path().join("a"), &link.name).0;
    let addr = format!("127.0.0.1:{port}");
    let resource = format!("http://{addr}/mcp");
    let redirect = "http://127.0.0.1/t3code-peer-link";
    // The sign-in a link makes, run over the tunnel: register, approve with a pairing code, then
    // exchange the code for a token that the linked agents' MCP requests carry.
    let code = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let host_token = remote::pair(&link.url_b, &code, "tester")
        .await
        .unwrap()
        .token;
    let host = remote::Host::connect_to(&link.url_b, &host_token, "tester")
        .await
        .unwrap();
    let pairing = host.t3_pairing().await.unwrap();

    let registered = post(
        &addr,
        "/oauth/mcp/register",
        JSON,
        &serde_json::json!({"client_name": "tester", "redirect_uris": [redirect]}).to_string(),
    );
    let client_id = registered["client_id"].as_str().unwrap().to_owned();
    let verifier = "a".repeat(64);
    let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
    let decided = post(
        &addr,
        "/oauth/mcp/decision",
        JSON,
        &serde_json::json!({
            "authorization": {
                "response_type": "code",
                "client_id": client_id,
                "redirect_uri": redirect,
                "code_challenge": challenge,
                "code_challenge_method": "S256",
                "state": "state",
                "resource": resource,
            },
            "decision": {"_tag": "pairing-code", "access": "auto", "code": pairing},
        })
        .to_string(),
    );
    let code = query(decided["redirectTo"].as_str().unwrap(), "code");
    let form = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={verifier}&resource={}",
        form_value(&code),
        form_value(redirect),
        form_value(&client_id),
        form_value(&resource),
    );
    let token = post(&addr, "/oauth/mcp/token", FORM, &form);
    let bearer = format!("Bearer {}", token["access_token"].as_str().unwrap());

    let initialize = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "tester", "version": "0"},
        },
    });
    let answer = http(
        &addr,
        "POST",
        "/mcp",
        &[
            ("Authorization", &bearer),
            ("Accept", "application/json, text/event-stream"),
            ("Content-Type", "application/json"),
        ],
        &initialize.to_string(),
    );
    assert_eq!(status_of(&answer), 200, "{answer}");
    assert!(answer.contains("protocolVersion"), "{answer}");
}
