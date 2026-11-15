//! The agent bridge end to end against the pinned T3 Code server with the core's scripted ACP
//! agent; needs `META_HARNESS_T3`, as the core's contract tests do.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use meta_harness_core::{AcpAgent, Host, HostConfig};

use crate::api::events::{BridgeChangeStatus, BridgeEvent, BridgeEventKind, BridgeItemKind};
use crate::api::git::{BridgeGitAction, BridgeGitProgressKind};
use crate::api::session::RUNTIME;
use crate::api::session::{AgentHost, AgentSession, BridgeModelChoice, BridgePermissions};
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

fn host(fixture: &Fixture) -> AgentHost {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../meta-harness/meta-harness-core/tests/fixtures/scripted_acp_agent.py");
    let host = RUNTIME
        .block_on(Host::start(HostConfig {
            binary: std::env::var_os("META_HARNESS_T3")
                .expect("META_HARNESS_T3 names the pinned t3")
                .into(),
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

    let downloads = fixture.0.join("Downloads");
    std::fs::create_dir(&downloads).expect("downloads");
    let folder = session
        .export_record_to(&downloads, "host", "2026-11-15")
        .expect("exported");
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
