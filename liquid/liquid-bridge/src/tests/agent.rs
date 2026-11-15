//! The agent bridge end to end against the pinned T3 Code server with the core's scripted ACP
//! agent; needs `META_HARNESS_T3`, as the core's contract tests do.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use meta_harness_core::{AcpAgent, Host, HostConfig};

use crate::api::events::{BridgeChangeStatus, BridgeEvent, BridgeEventKind, BridgeItemKind};
use crate::api::git::{BridgeGitAction, BridgeGitProgressKind};
use crate::api::session::RUNTIME;
use crate::api::session::{
    AgentHost, AgentProject, AgentSession, BridgeModelChoice, BridgePermissions,
};
use crate::api::thread::BridgeTurnStatus;

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .is_ok_and(|status| status.success());
    assert!(ok, "git {args:?}");
}

/// A temporary folder with a repository, removed on drop.
struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).expect("random bytes");
        let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let dir = std::env::temp_dir().join(format!("lb-{name}"));
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).expect("folders");
        let dir = dir.canonicalize().expect("canonical");
        let repo = dir.join("repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "Bridge"]);
        git(&repo, &["config", "user.email", "bridge@example.invalid"]);
        std::fs::write(repo.join("README.md"), "bridge\n").expect("readme");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "start"]);
        Self(dir)
    }

    fn repo(&self) -> PathBuf {
        self.0.join("repo")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The pinned T3 Code binary the tests run against.
fn t3_binary() -> PathBuf {
    std::env::var_os("META_HARNESS_T3")
        .expect("META_HARNESS_T3 names the pinned t3")
        .into()
}

fn host(fixture: &Fixture) -> AgentHost {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../meta-harness/meta-harness-core/tests/fixtures/scripted_acp_agent.py");
    let host = RUNTIME
        .block_on(Host::start(HostConfig {
            binary: t3_binary(),
            home: fixture.0.join("t3"),
            acp_agents: vec![AcpAgent {
                id: "scripted".to_owned(),
                command: PathBuf::from("/usr/bin/python3"),
                args: vec![script.to_string_lossy().into_owned()],
                models: vec!["scripted".to_owned()],
            }],
        }))
        .expect("T3 starts");
    AgentHost::from_host(host)
}

fn scripted() -> BridgeModelChoice {
    BridgeModelChoice {
        instance: "scripted".to_owned(),
        model: "scripted".to_owned(),
        option_ids: Vec::new(),
        values: Vec::new(),
    }
}

/// Follows `session` until `done` holds for its turns, the way Liquid applies its stream:
/// the snapshot first, then events past its sequence, each confirmed.
fn follow(session: &AgentSession, done: impl Fn(&AgentSession) -> bool) -> Vec<BridgeEvent> {
    let (sender, receiver) = mpsc::channel();
    let snapshot = session.forward(move |event| sender.send(event).is_ok());
    let mut events = Vec::new();
    while !done(session) {
        let event = receiver
            .recv_timeout(Duration::from_secs(60))
            .expect("the session goes on");
        assert!(event.sequence > snapshot.sequence || event.kind == BridgeEventKind::FellBehind);
        events.push(event);
        session.confirm_applied(i64::try_from(events.len()).expect("count"));
    }
    events
}

/// Polls `ready` until it holds: T3 applies a project's changes after the call returns.
#[track_caller]
fn until(ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "T3 did not apply the change"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn completed(n: usize) -> impl Fn(&AgentSession) -> bool {
    move |session| {
        session
            .turns()
            .iter()
            .filter(|t| t.status == BridgeTurnStatus::Completed && t.checkpoint)
            .count()
            >= n
    }
}

#[test]
fn a_session_streams_diffs_reverts_and_exports_through_the_bridge() {
    let fixture = Fixture::new();
    let host = host(&fixture);
    let project = host
        .project(fixture.repo().to_string_lossy().into_owned())
        .expect("project");
    let session = project
        .start(
            "append first".to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts");
    let events = follow(&session, completed(1));
    assert!(
        events
            .iter()
            .any(|e| e.kind == BridgeEventKind::ItemCompleted
                && e.text.as_deref() == Some("Appended.")),
        "the answer streams to Liquid"
    );
    session
        .send("append second".to_owned(), false)
        .expect("second turn");
    follow(&session, completed(2));

    let changes = session.turn_changes(2, 2, false).expect("changes");
    assert_eq!(changes.len(), 1);
    assert_eq!(
        (changes[0].path.as_str(), changes[0].added),
        ("contract.txt", 1)
    );
    assert_eq!(changes[0].status, BridgeChangeStatus::Modified);
    let patch = session
        .turn_patch(1, 2, "contract.txt".to_owned(), false)
        .expect("patch");
    assert!(patch.contains("+first") && patch.contains("+second"));
    let statistics = session.statistics();
    assert_eq!(statistics.turns.len(), 2);
    assert!(statistics.tools.iter().any(|t| t.succeeded >= 1));

    // The record goes to the Downloads folder of a home folder of the test's own.
    let home = fixture.0.join("home");
    std::fs::create_dir_all(home.join("Downloads")).expect("downloads");
    // SAFETY: nextest runs each test in its own process, and no thread of this one reads the
    // environment while the test sets it.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_CONFIG_HOME", fixture.0.join("config"));
    }
    let folder = PathBuf::from(
        session
            .export_record("host".to_owned(), "2026-11-15".to_owned())
            .expect("exported"),
    );
    assert!(
        folder.starts_with(home.join("Downloads")),
        "{}",
        folder.display()
    );
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join("record.json")).expect("record"))
            .expect("json");
    assert_eq!(record["turns"].as_array().map(Vec::len), Some(2));
    assert!(folder.join("thread.json").is_file() && folder.join("README.txt").is_file());

    assert_eq!(session.revert(2, true).expect("reverted"), "append second");
    follow(&session, |s| s.turns().len() == 1);
    let worktree = PathBuf::from(session.worktree().expect("own worktree"));
    assert_eq!(
        std::fs::read_to_string(worktree.join("contract.txt")).expect("file"),
        "first\n"
    );
    let snapshot = session.snapshot();
    assert!(
        snapshot
            .transcript
            .iter()
            .all(|item| item.text != "append second"),
        "the reverted prompt left the Chat"
    );
    assert!(
        snapshot.transcript.iter().any(
            |item| item.item.kind == BridgeItemKind::UserMessage && item.text == "append first"
        )
    );
    host.shutdown();
}

#[test]
fn git_runs_through_the_bridge_with_its_progress() {
    let fixture = Fixture::new();
    let host = host(&fixture);
    let project = host
        .project(fixture.repo().to_string_lossy().into_owned())
        .expect("project");
    let git = project.git();
    std::fs::write(fixture.repo().join("README.md"), "bridge\nchanged\n").expect("change");
    let status = git.status().expect("status");
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.changes.len(), 1);
    assert!(
        git.uncommitted_patch("README.md".to_owned(), false)
            .expect("patch")
            .contains("+changed")
    );
    git.create_branch("feature".to_owned()).expect("branch");
    assert!(
        git.refs("feat".to_owned(), 0, 10)
            .expect("refs")
            .refs
            .iter()
            .any(|r| r.name == "feature" && r.current)
    );
    let (sender, receiver) = mpsc::channel();
    git.run_with(
        BridgeGitAction::Commit,
        Some("Change the readme".to_owned()),
        None,
        false,
        move |step| sender.send(step).is_ok(),
    )
    .expect("runs");
    let steps: Vec<_> = receiver.iter().collect();
    assert!(steps.iter().any(|s| s.kind == BridgeGitProgressKind::Phase));
    assert_eq!(
        steps.last().map(|s| s.kind),
        Some(BridgeGitProgressKind::Finished),
        "{steps:?}"
    );
    assert!(git.status().expect("status").changes.is_empty());
    host.shutdown();
}

#[test]
fn the_host_starts_t3_and_refuses_calls_once_stopped() {
    let fixture = Fixture::new();
    let home = fixture.0.join("t3");
    // Meta-Harness creates a home folder only when it writes ACP agents into it, and T3 starts in it.
    std::fs::create_dir(&home).expect("home");
    let host = AgentHost::start(
        t3_binary().to_string_lossy().into_owned(),
        home.to_string_lossy().into_owned(),
    )
    .expect("T3 starts");
    assert!(
        host.providers()
            .is_ok_and(|providers| !providers.is_empty())
    );
    host.shutdown();
    assert_eq!(host.providers(), Err("T3 has stopped.".to_owned()));
}

#[test]
fn a_project_starts_finds_opens_archives_and_deletes_its_sessions() {
    let fixture = Fixture::new();
    let host = host(&fixture);
    let project = host
        .project(fixture.repo().to_string_lossy().into_owned())
        .expect("project");
    let first = project
        .start(
            "append first".to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts in a new worktree");
    follow(&first, completed(1));
    let worktree = PathBuf::from(first.worktree().expect("its own worktree"));
    let second = project
        .start(
            "append second".to_owned(),
            scripted(),
            None,
            Some(worktree.to_string_lossy().into_owned()),
            BridgePermissions::FullAccess,
        )
        .expect("session starts in the existing worktree");
    follow(&second, completed(1));
    assert_eq!(second.worktree().map(PathBuf::from), Some(worktree.clone()));
    assert_eq!(project.threads().expect("threads").len(), 2);
    assert!(!project.only_session_in_worktree(first.id()).expect("query"));
    assert_eq!(project.open(second.id()).expect("opened").id(), second.id());

    project.archive(first.id(), true).expect("archived");
    project.archive(first.id(), false).expect("restored");
    project.delete(second.id(), false).expect("deleted");
    until(|| project.threads().expect("threads").len() == 1);
    assert!(project.only_session_in_worktree(first.id()).expect("query"));
    project
        .delete(first.id(), true)
        .expect("deleted with its worktree");
    until(|| project.threads().expect("threads").is_empty());
    until(|| !worktree.exists());

    let checkout = project
        .start(
            "Done.".to_owned(),
            scripted(),
            None,
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts in the project folder");
    follow(&checkout, completed(1));
    assert_eq!(checkout.worktree(), None);
    let forked = checkout.fork(1).expect("forked from the first turn");
    assert_ne!(forked, checkout.id(), "a fork is a session of its own");
    host.shutdown();
}

/// Starts a session that runs until stopped and queues one follow-up behind it.
fn queued_behind_a_running_turn(project: &AgentProject, prompt: &str) -> AgentSession {
    let session = project
        .start(
            "wait".to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts");
    session.send(prompt.to_owned(), false).expect("queued");
    until(|| {
        session
            .turns()
            .get(1)
            .is_some_and(|t| t.status == BridgeTurnStatus::Queued)
    });
    session
}

#[test]
fn a_session_queues_edits_cancels_and_stops_turns_through_the_bridge() {
    let fixture = Fixture::new();
    let host = host(&fixture);
    let project = host
        .project(fixture.repo().to_string_lossy().into_owned())
        .expect("project");

    let cancelled = queued_behind_a_running_turn(&project, "append doomed");
    cancelled.cancel_queued(2).expect("cancelled");
    cancelled.stop().expect("stopped");
    until(|| cancelled.turns()[0].status == BridgeTurnStatus::Aborted);

    let edited = queued_behind_a_running_turn(&project, "append queued");
    edited
        .edit_queued(2, "append edited".to_owned())
        .expect("edited");
    edited.stop().expect("stopped");
    until(|| edited.turns()[0].status == BridgeTurnStatus::Aborted);

    let sent = queued_behind_a_running_turn(&project, "append now");
    // T3 refuses to steer a turn that has not started; the refusal reaches Liquid.
    assert!(sent.send_queued_now(2).is_err());
    sent.stop().expect("stopped");
    let session = sent;
    assert!(session.context().is_none(), "the agent reports no window");
    assert!(session.dismiss("no such request".to_owned()).is_err());
    assert!(
        session
            .answer("no such request".to_owned(), Vec::new())
            .is_err()
    );

    let asked = project
        .start(
            "ask".to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::Supervised,
        )
        .expect("session starts");
    follow(&asked, |s| !s.snapshot().requests.is_empty());
    let request = asked.snapshot().requests.remove(0);
    let decision = request
        .approval
        .expect("an approval")
        .options
        .remove(0)
        .decision;
    asked.respond(request.id, decision).expect("answered");
    asked.set_model(scripted()).expect("model set");
    for permissions in [
        BridgePermissions::AutoAcceptEdits,
        BridgePermissions::Auto,
        BridgePermissions::Supervised,
    ] {
        asked.set_permissions(permissions).expect("permissions set");
    }
    host.shutdown();
}

#[test]
fn git_switches_commits_pulls_pushes_and_initializes_through_the_bridge() {
    let fixture = Fixture::new();
    let host = host(&fixture);
    let repo = fixture.repo();
    git(
        &fixture.0,
        &["init", "-q", "--bare", "-b", "main", "remote.git"],
    );
    let remote = fixture.0.join("remote.git");
    git(
        &repo,
        &["remote", "add", "origin", &remote.to_string_lossy()],
    );
    git(&repo, &["push", "-q", "-u", "origin", "main"]);
    let hook = repo.join(".git").join("hooks").join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho checking the hook\n").expect("hook");
    make_executable(&hook);
    let project = host
        .project(repo.to_string_lossy().into_owned())
        .expect("project");
    let git = project.git();

    git.create_branch("feature".to_owned()).expect("branch");
    git.switch("main".to_owned()).expect("switched");
    assert_eq!(
        git.status().expect("status").branch.as_deref(),
        Some("main")
    );
    std::fs::write(repo.join("README.md"), "bridge\nchanged\n").expect("change");
    let changes = git.uncommitted_changes(false).expect("changes");
    assert_eq!(
        changes.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        ["README.md"]
    );
    assert_eq!(git.pull().expect("pulled").len(), 2);

    let (sender, receiver) = mpsc::channel();
    git.run_with(
        BridgeGitAction::CommitPush,
        Some("Change the readme".to_owned()),
        None,
        false,
        move |step| sender.send(step).is_ok(),
    )
    .expect("runs");
    let steps: Vec<_> = receiver.iter().collect();
    assert!(
        steps
            .iter()
            .any(|s| s.kind == BridgeGitProgressKind::Output && s.text.contains("checking")),
        "the hook's output streams: {steps:?}"
    );
    assert_eq!(
        steps.last().map(|s| s.kind),
        Some(BridgeGitProgressKind::Finished),
        "{steps:?}"
    );
    let pushed = git.status().expect("status");
    assert!(pushed.upstream && pushed.ahead == 0 && pushed.changes.is_empty());

    let (sender, receiver) = mpsc::channel();
    git.run_with(
        BridgeGitAction::CreatePullRequest,
        None,
        None,
        false,
        move |step| sender.send(step).is_ok(),
    )
    .expect("runs");
    let steps: Vec<_> = receiver.iter().collect();
    assert_eq!(
        steps.last().map(|s| s.kind),
        Some(BridgeGitProgressKind::Failed),
        "a remote that is no hosting provider cannot open a pull request: {steps:?}"
    );

    assert!(git.resolve_pull_request("#1".to_owned()).is_err());
    assert!(git.check_out_pull_request("#1".to_owned(), false).is_err());

    let plain = fixture.0.join("plain");
    std::fs::create_dir(&plain).expect("folder");
    let plain = project_at(&host, &plain);
    assert!(!plain.git().status().expect("status").repository);
    plain.git().init().expect("initialized");
    assert!(plain.git().status().expect("status").repository);
    host.shutdown();
}

fn project_at(host: &AgentHost, folder: &Path) -> crate::api::session::AgentProject {
    host.project(folder.to_string_lossy().into_owned())
        .expect("project")
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("mode");
}

#[cfg(not(unix))]
fn make_executable(_: &Path) {}
