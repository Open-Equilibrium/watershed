//! The agent bridge end to end against the pinned T3 Code server with the core's scripted ACP
//! agent; needs `META_HARNESS_T3`, as the core's contract tests do.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, mpsc};
use std::time::Duration;

use meta_harness_core::{AcpAgent, CONNECTION_FILE, Service, ServiceConfig};

use crate::api::details::BridgeLineageRelation;
use crate::api::events::{BridgeChangeStatus, BridgeEvent, BridgeEventKind, BridgeItemKind};
use crate::api::git::{BridgeGitAction, BridgeGitProgressKind};
use crate::api::session::RUNTIME;
use crate::api::session::{
    AgentHost, AgentProject, AgentSession, BridgeModelChoice, BridgePermissions,
};
use crate::api::thread::{BridgeSnapshot, BridgeTurnStatus};

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

    /// Meta-Harness's data folder for the test's service.
    fn home(&self) -> PathBuf {
        self.0.join("meta-harness")
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

/// Starts a service in the fixture's folder with the scripted agent, and connects the bridge to it
/// as Liquid does. The test ends the service with [`stop`].
fn host(fixture: &Fixture) -> (Service, AgentHost) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../meta-harness/meta-harness-core/tests/fixtures/scripted_acp_agent.py");
    let home = fixture.home();
    let service = RUNTIME
        .block_on(Service::start(ServiceConfig {
            home: home.clone(),
            t3: t3_binary(),
            acp_agents: vec![AcpAgent {
                id: "scripted".to_owned(),
                command: PathBuf::from("/usr/bin/python3"),
                args: vec![script.to_string_lossy().into_owned()],
                models: vec!["scripted".to_owned()],
            }],
        }))
        .expect("the service starts");
    let host =
        AgentHost::connect(Some(home.to_string_lossy().into_owned())).expect("Liquid connects");
    (service, host)
}

fn stop(service: Service) {
    RUNTIME.block_on(service.stop());
}

fn scripted() -> BridgeModelChoice {
    BridgeModelChoice {
        instance: "scripted".to_owned(),
        model: "scripted".to_owned(),
        option_ids: Vec::new(),
        values: Vec::new(),
    }
}

/// Follows `session` until `done` holds, the way Liquid applies its stream: the snapshot first,
/// then events past its sequence, each confirmed. `done` sees the snapshot and the events so far.
fn follow_until(
    session: &AgentSession,
    done: impl Fn(&AgentSession, &BridgeSnapshot, &[BridgeEvent]) -> bool,
) -> (BridgeSnapshot, Vec<BridgeEvent>) {
    let (sender, receiver) = mpsc::channel();
    let snapshot = session
        .forward(move |event| sender.send(event).is_ok())
        .expect("the session follows");
    let mut events = Vec::new();
    while !done(session, &snapshot, &events) {
        let event = receiver
            .recv_timeout(Duration::from_secs(60))
            .expect("the session goes on");
        assert!(event.sequence > snapshot.sequence || event.kind == BridgeEventKind::FellBehind);
        events.push(event);
        session.confirm_applied(i64::try_from(events.len()).expect("count"));
    }
    (snapshot, events)
}

/// Receives events until one satisfies `done`, and gives every event received, that one included.
fn receive_until(
    receiver: &mpsc::Receiver<BridgeEvent>,
    done: impl Fn(&BridgeEvent) -> bool,
) -> Vec<BridgeEvent> {
    let mut events = Vec::new();
    loop {
        let event = receiver
            .recv_timeout(Duration::from_secs(60))
            .expect("the session goes on");
        let last = done(&event);
        events.push(event);
        if last {
            return events;
        }
    }
}

/// Follows `session` until `done` holds for its state.
fn follow(session: &AgentSession, done: impl Fn(&AgentSession) -> bool) -> Vec<BridgeEvent> {
    follow_until(session, |session, _, _| done(session)).1
}

/// Follows `session` until `turns` turns completed. The state can run ahead of the stream, so a
/// turn running at the subscription is awaited until its end arrives as an event.
fn follow_turns(session: &AgentSession, turns: usize) -> (BridgeSnapshot, Vec<BridgeEvent>) {
    follow_until(session, |session, snapshot, events| {
        let running = snapshot.turns.iter().any(|t| {
            matches!(
                t.status,
                BridgeTurnStatus::Queued | BridgeTurnStatus::Running
            )
        });
        let ended = events.iter().any(|e| {
            matches!(
                e.kind,
                BridgeEventKind::TurnCompleted | BridgeEventKind::TurnAborted
            )
        });
        completed(turns)(session) && (!running || ended)
    })
}

/// Whether the answer reached Liquid: streamed by a turn that ran after the subscription, or in the
/// snapshot of a turn that finished before it.
fn answer_reached_liquid(snapshot: &BridgeSnapshot, events: &[BridgeEvent]) -> bool {
    events
        .iter()
        .any(|e| e.kind == BridgeEventKind::ItemCompleted && e.text.as_deref() == Some("Appended."))
        || snapshot
            .transcript
            .iter()
            .any(|item| item.text == "Appended.")
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
            .expect("the turns")
            .iter()
            .filter(|t| t.status == BridgeTurnStatus::Completed && t.checkpoint)
            .count()
            >= n
    }
}

#[test]
fn a_session_streams_diffs_reverts_and_exports_through_the_bridge() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
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
    let (snapshot, events) = follow_turns(&session, 1);
    assert!(
        answer_reached_liquid(&snapshot, &events),
        "the answer streams to Liquid"
    );
    session
        .send("append second".to_owned(), false)
        .expect("second turn");
    follow_turns(&session, 2);

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
    let statistics = session.statistics().expect("statistics");
    assert_eq!(statistics.turns.len(), 2);
    assert!(statistics.tools.iter().any(|t| t.succeeded >= 1));

    // The record goes into a Downloads folder of the test's own. Finding that folder from HOME
    // and XDG_CONFIG_HOME is `download_dir_in`'s job, covered by its own test; setting those
    // variables here would leak into every other test of a shared process.
    let downloads = fixture.0.join("Downloads");
    std::fs::create_dir_all(&downloads).expect("downloads");
    let folder = session
        .export_record_to(&downloads, "host", "2026-11-15")
        .expect("exported");
    assert!(folder.starts_with(&downloads), "{}", folder.display());
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join("record.json")).expect("record"))
            .expect("json");
    assert_eq!(record["turns"].as_array().map(Vec::len), Some(2));
    assert!(folder.join("thread.json").is_file() && folder.join("README.txt").is_file());

    assert_eq!(session.revert(2, true).expect("reverted"), "append second");
    follow(&session, |s| s.turns().expect("the turns").len() == 1);
    let worktree = PathBuf::from(
        session
            .worktree()
            .expect("the worktree")
            .expect("own worktree"),
    );
    assert_eq!(
        std::fs::read_to_string(worktree.join("contract.txt")).expect("file"),
        "first\n"
    );
    let snapshot = session.snapshot().expect("the snapshot");
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
    stop(service);
}

#[test]
fn a_turn_that_finished_before_its_follow_has_its_answer_in_the_snapshot() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let project = project_at(&host, &fixture.repo());
    let session = project
        .start(
            "append first".to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts");
    until(|| completed(1)(&session));
    let (snapshot, events) = follow_turns(&session, 1);
    assert!(
        answer_reached_liquid(&snapshot, &events),
        "the answer reaches Liquid"
    );
    stop(service);
}

#[test]
fn git_runs_through_the_bridge_with_its_progress() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
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
    // A run that never ends stops the collection after a while, so the test fails, not hangs.
    let steps: Vec<_> =
        std::iter::from_fn(|| receiver.recv_timeout(Duration::from_secs(10)).ok()).collect();
    assert!(steps.iter().any(|s| s.kind == BridgeGitProgressKind::Phase));
    assert_eq!(
        steps.last().map(|s| s.kind),
        Some(BridgeGitProgressKind::Finished),
        "{steps:?}"
    );
    assert!(git.status().expect("status").changes.is_empty());
    stop(service);
}

#[test]
fn a_status_callback_reads_its_checkout_and_the_watch_goes_on() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let project = project_at(&host, &fixture.repo());
    let git = Arc::new(project.git());
    let (sender, reads) = mpsc::channel();
    let reader = git.clone();
    // The callback runs on the bridge's runtime, where a Rust consumer may still read the checkout.
    git.watch_with(move |_| sender.send(reader.status()).is_ok())
        .expect("the status watch");
    reads
        .recv_timeout(Duration::from_secs(60))
        .expect("the first read is delivered")
        .expect("the first read answers");
    std::fs::write(fixture.repo().join("second.txt"), "second\n").expect("change");
    let second = reads
        .recv_timeout(Duration::from_secs(60))
        .expect("the watch goes on after a read")
        .expect("the second read answers");
    assert!(
        second
            .changes
            .iter()
            .any(|change| change.path == "second.txt")
    );
    stop(service);
}

#[test]
fn connecting_says_when_meta_harness_is_not_running_and_where_it_looked() {
    let fixture = Fixture::new();
    let home = fixture.home().to_string_lossy().into_owned();
    let error = AgentHost::connect(Some(home.clone()))
        .err()
        .expect("no service runs in a new folder");
    assert!(error.starts_with("Meta-Harness is not running"), "{error}");
    assert!(
        error.contains(&home) && error.contains("META_HARNESS_HOME"),
        "{error}"
    );

    // A connection file whose address refuses connections is left by a service that stopped
    // without removing it.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a port")
        .local_addr()
        .expect("an address");
    let file = fixture.home().join(CONNECTION_FILE);
    std::fs::create_dir_all(fixture.home()).expect("the folder");
    std::fs::write(
        &file,
        format!(r#"{{"url":"ws://{closed}","token":"unused"}}"#),
    )
    .expect("the file");
    let error = AgentHost::connect(Some(home.clone()))
        .err()
        .expect("the address refuses");
    assert!(error.starts_with("Meta-Harness is not running"), "{error}");

    // A damaged file is not a missing service: the error keeps its own text.
    std::fs::write(&file, "{").expect("the file");
    let error = AgentHost::connect(Some(home.clone()))
        .err()
        .expect("the file is damaged");
    assert!(
        error.contains("damaged") && !error.contains("not running") && error.contains(&home),
        "{error}"
    );
}

#[test]
fn a_followed_session_resumes_after_meta_harness_restarts() {
    let fixture = Fixture::new();
    let (service, bridge) = host(&fixture);
    let session = project_at(&bridge, &fixture.repo())
        .start(
            "append first".to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts");
    let (sender, receiver) = mpsc::channel();
    let snapshot = session
        .forward(move |event| sender.send(event).is_ok())
        .expect("the session follows");
    // A first turn that finished before the follow has its end in the snapshot, not the stream.
    if !snapshot
        .turns
        .iter()
        .any(|t| t.status == BridgeTurnStatus::Completed)
    {
        receive_until(&receiver, |e| e.kind == BridgeEventKind::TurnCompleted);
    }

    stop(service);
    // Nothing answers for a while, as a restart takes time; the stream retries meanwhile.
    std::thread::sleep(Duration::from_secs(1));
    let (service, _) = host(&fixture);
    // The stream resubscribes once Meta-Harness answers, and marks the gap with a fell-behind event.
    receive_until(&receiver, |e| e.kind == BridgeEventKind::FellBehind);
    session
        .send("append second".to_owned(), false)
        .expect("second turn");
    // The scripted agent cannot load its session, so T3 aborts the turn after the restart; the
    // turn's events reach Liquid all the same.
    let events = receive_until(&receiver, |e| {
        matches!(
            e.kind,
            BridgeEventKind::TurnCompleted | BridgeEventKind::TurnAborted
        )
    });
    assert!(
        events.iter().any(|e| {
            e.kind == BridgeEventKind::ItemCompleted && e.text.as_deref() == Some("append second")
        }),
        "the second turn reaches Liquid: {events:?}"
    );
    stop(service);
}

#[test]
fn an_event_stream_takes_the_subscription_it_names() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let session = project_at(&host, &fixture.repo())
        .start(
            "append first".to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts");
    let first = session.subscribe().expect("the first subscription");
    session
        .send("append second".to_owned(), false)
        .expect("second turn");
    follow_turns(&session, 2);
    let second = session.subscribe().expect("the second subscription");
    assert_ne!(first.id, second.id);

    // The turn ran between the two snapshots, so only the first subscription streams its events:
    // its first event comes after its snapshot and no later than the second's.
    let (sender, received) = mpsc::channel();
    session
        .events_with(first.id, move |event| sender.send(event).is_ok())
        .expect("the first subscription streams");
    let event = received
        .recv_timeout(Duration::from_secs(60))
        .expect("the first subscription's events");
    assert!(
        first.snapshot.sequence < event.sequence && event.sequence <= second.snapshot.sequence,
        "{event:?} is not between the snapshots"
    );
    assert!(
        session.events_with(first.id, |_| true).is_err(),
        "a subscription streams once"
    );
    assert!(
        session.events_with(first.id + 1000, |_| true).is_err(),
        "an unknown subscription is an error"
    );
    // Subscriptions nobody streamed end once enough newer ones replace them.
    for _ in 0..4 {
        session.subscribe().expect("a newer subscription");
    }
    assert!(
        session.events_with(second.id, |_| true).is_err(),
        "an abandoned subscription ends"
    );
    stop(service);
}

#[test]
fn servers_automations_and_lineage_follows_resume_after_meta_harness_restarts() {
    let fixture = Fixture::new();
    let (service, bridge) = host(&fixture);
    let session = project_at(&bridge, &fixture.repo())
        .start(
            "append first".to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts");
    let (servers, servers_seen) = mpsc::channel();
    session.servers_subscribe().expect("the servers");
    session
        .servers_with(move |list| servers.send(list).is_ok())
        .expect("the servers follow");
    let (tasks, tasks_seen) = mpsc::channel();
    session
        .automations_subscribe()
        .expect("the scheduled tasks");
    session
        .automations_with(move |list| tasks.send(list).is_ok())
        .expect("the scheduled tasks follow");
    let (lineage, lineage_seen) = mpsc::channel();
    session.lineage_subscribe().expect("the lineage");
    session
        .lineage_with(move |result| lineage.send(result).is_ok())
        .expect("the lineage follows");

    stop(service);
    // Nothing answers for a while, as a restart takes time; each follow retries meanwhile.
    std::thread::sleep(Duration::from_secs(1));
    // What was delivered before the outage, such as the first turn's changes, says nothing about
    // the restarted service, so it is dropped before the service comes back.
    while servers_seen.try_recv().is_ok() {}
    while tasks_seen.try_recv().is_ok() {}
    while lineage_seen.try_recv().is_ok() {}
    let (service, _) = host(&fixture);
    // Nothing changes meanwhile, so each follow's first delivery is the one after its reconnect.
    let wait = Duration::from_secs(60);
    servers_seen.recv_timeout(wait).expect("the servers resume");
    tasks_seen
        .recv_timeout(wait)
        .expect("the scheduled tasks resume");
    // A change that came as the service stopped may read the lineage during the outage, which
    // fails; the follow then reads again once Meta-Harness answers.
    let mut lineage_reads = std::iter::from_fn(|| lineage_seen.recv_timeout(wait).ok());
    assert!(
        lineage_reads.any(|read| read.is_ok()),
        "the lineage resumes"
    );
    stop(service);
}

const FOLLOWERS: usize = 8;
const ROUNDS: usize = 4;

/// Starts `FOLLOWERS` forwards of `session` at once; gives each follow's snapshot sequence and
/// its receiver.
fn follow_together(session: &Arc<AgentSession>) -> Vec<(i64, mpsc::Receiver<BridgeEvent>)> {
    let together = Arc::new(Barrier::new(FOLLOWERS));
    let threads: Vec<_> = (0..FOLLOWERS)
        .map(|_| {
            let (session, together) = (session.clone(), together.clone());
            std::thread::spawn(move || {
                let (sender, receiver) = mpsc::channel();
                together.wait();
                let snapshot = session
                    .forward(move |event| sender.send(event).is_ok())
                    .expect("the session follows");
                (snapshot.sequence, receiver)
            })
        })
        .collect();
    threads
        .into_iter()
        .map(|thread| thread.join().expect("no concurrent follow panics"))
        .collect()
}

#[test]
fn concurrent_forwards_of_one_session_each_stream_their_own_events() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let session = Arc::new(
        project_at(&host, &fixture.repo())
            .start(
                "append first".to_owned(),
                scripted(),
                Some("main".to_owned()),
                None,
                BridgePermissions::FullAccess,
            )
            .expect("session starts"),
    );
    until(|| completed(1)(&session));
    // Rounds of followers that start together, so their subscriptions interleave.
    let followers: Vec<_> = (0..ROUNDS)
        .flat_map(|_| follow_together(&session))
        .collect();
    session
        .send("append second".to_owned(), false)
        .expect("second turn");
    for (after, receiver) in followers {
        let events = receive_until(&receiver, |e| {
            matches!(
                e.kind,
                BridgeEventKind::TurnCompleted | BridgeEventKind::TurnAborted
            )
        });
        assert!(
            events
                .iter()
                .all(|e| e.sequence > after || e.kind == BridgeEventKind::FellBehind),
            "every event is past the follow's snapshot: {events:?}"
        );
        assert!(
            events.iter().any(|e| {
                e.kind == BridgeEventKind::ItemCompleted
                    && e.text.as_deref() == Some("append second")
            }),
            "the second turn reaches every follow: {events:?}"
        );
    }
    stop(service);
}

#[test]
fn closing_liquid_leaves_the_service_running() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let home = fixture.home().to_string_lossy().into_owned();
    drop(host);
    let reconnected = AgentHost::connect(Some(home)).expect("the service still runs");
    assert!(
        reconnected
            .providers()
            .is_ok_and(|providers| !providers.is_empty())
    );
    stop(service);
    assert!(
        reconnected.providers().is_err(),
        "a stopped service answers no call"
    );
}

#[test]
fn a_project_starts_finds_opens_archives_and_deletes_its_sessions() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
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
    follow_turns(&first, 1);
    let worktree = PathBuf::from(
        first
            .worktree()
            .expect("the worktree")
            .expect("its own worktree"),
    );
    let second = project
        .start(
            "append second".to_owned(),
            scripted(),
            None,
            Some(worktree.to_string_lossy().into_owned()),
            BridgePermissions::FullAccess,
        )
        .expect("session starts in the existing worktree");
    follow_turns(&second, 1);
    assert_eq!(
        second.worktree().expect("the worktree").map(PathBuf::from),
        Some(worktree.clone())
    );
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
    follow_turns(&checkout, 1);
    assert_eq!(checkout.worktree().expect("the worktree"), None);
    let forked = checkout.fork(1).expect("forked from the first turn");
    assert_ne!(forked, checkout.id(), "a fork is a session of its own");
    stop(service);
}

#[test]
fn the_details_card_reads_and_acts_on_a_session_through_the_bridge() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    assert!(
        !host
            .environment_name()
            .expect("the host has a name")
            .is_empty()
    );
    let project = project_at(&host, &fixture.repo());
    let session = project
        .start(
            "append first".to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts");
    follow_turns(&session, 1);
    assert!(session.servers_subscribe().expect("servers").is_empty());
    assert!(
        session
            .automations_subscribe()
            .expect("automations")
            .is_empty()
    );
    session.usage_limits().expect("usage limits");

    let fork_id = session.fork(1).expect("forked");
    let fork = project.open(fork_id.clone()).expect("opened");
    fork.send("append in fork".to_owned(), false).expect("sent");
    until(|| {
        fork.turns().expect("the turns").iter().any(|t| {
            t.prompt == "append in fork" && t.status == BridgeTurnStatus::Completed && t.checkpoint
        })
    });
    let parent = session.lineage_subscribe().expect("lineage");
    assert!(
        parent
            .rows
            .iter()
            .any(|row| { row.thread_id == fork_id && row.relation == BridgeLineageRelation::Fork })
    );
    let lineage = fork.lineage_subscribe().expect("the fork's lineage");
    assert!(lineage.can_merge && lineage.rows.iter().any(|row| row.merge_target));
    fork.merge_back().expect("merged back");
    stop(service);
}

/// Reports to its test when it is dropped, which is when the task that owns it ends.
struct Ends(mpsc::Sender<()>);

impl Drop for Ends {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

/// A delivery that holds `ends` for as long as the task that runs it lives.
fn holding<T>(ends: Ends) -> impl FnMut(T) -> bool + Send + 'static {
    move |_| {
        let _ = &ends;
        true
    }
}

#[test]
fn dropping_the_session_or_checkout_ends_its_live_follows() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let project = project_at(&host, &fixture.repo());
    let session = project
        .start(
            "append first".to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts");
    follow_turns(&session, 1);
    let (sender, ended) = mpsc::channel();
    session.servers_subscribe().expect("the servers");
    session
        .servers_with(holding(Ends(sender.clone())))
        .expect("the servers follow");
    session
        .automations_subscribe()
        .expect("the scheduled tasks");
    session
        .automations_with(holding(Ends(sender.clone())))
        .expect("the scheduled tasks follow");
    session.lineage_subscribe().expect("the lineage");
    session
        .lineage_with(holding(Ends(sender.clone())))
        .expect("the lineage follows");
    drop(session);
    for section in ["servers", "scheduled tasks", "lineage"] {
        ended
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("the {section} follow ends with its session"));
    }

    let git = project.git();
    let (sender, ended) = mpsc::channel();
    git.watch_with(holding(Ends(sender)))
        .expect("the status watch");
    drop(git);
    ended
        .recv_timeout(Duration::from_secs(10))
        .expect("the status watch ends with its checkout");
    stop(service);
}

#[test]
fn a_stopped_or_replaced_follow_ends_its_task_while_the_session_stays_open() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let project = project_at(&host, &fixture.repo());
    let session = project
        .start(
            "append first".to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts");
    follow_turns(&session, 1);
    let git = project.git();
    let (sender, ended) = mpsc::channel();
    session.servers_subscribe().expect("the servers");
    session
        .servers_with(holding(Ends(sender.clone())))
        .expect("the servers follow");
    // A newer follow of the same kind replaces the one before it, which ends before anything stops.
    session.servers_subscribe().expect("the servers");
    session
        .servers_with(holding(Ends(sender.clone())))
        .expect("the servers follow");
    ended
        .recv_timeout(Duration::from_secs(10))
        .expect("the replaced servers follow ends");
    session
        .automations_subscribe()
        .expect("the scheduled tasks");
    session
        .automations_with(holding(Ends(sender.clone())))
        .expect("the scheduled tasks follow");
    git.watch_with(holding(Ends(sender)))
        .expect("the status watch");
    // Nothing changes, yet each stop ends its task while the session and checkout stay open.
    session.servers_stop();
    session.automations_stop();
    git.watch_stop();
    for _ in 0..3 {
        ended
            .recv_timeout(Duration::from_secs(10))
            .expect("a stopped follow ends");
    }
    stop(service);
}

#[test]
fn a_stopped_or_replaced_lineage_or_chat_follow_ends_while_the_session_stays_open() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let project = project_at(&host, &fixture.repo());
    let session = project
        .start(
            "append first".to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts");
    follow_turns(&session, 1);
    let (sender, ended) = mpsc::channel();
    let wait = Duration::from_secs(10);
    session.lineage_subscribe().expect("the lineage");
    session
        .lineage_with(holding(Ends(sender.clone())))
        .expect("the lineage follows");
    // Nothing changes, so the first follow would run on; a newer one replaces it.
    session.lineage_subscribe().expect("the lineage");
    session
        .lineage_with(holding(Ends(sender.clone())))
        .expect("the lineage follows");
    ended
        .recv_timeout(wait)
        .expect("the replaced lineage follow ends");
    session.lineage_stop();
    ended
        .recv_timeout(wait)
        .expect("the stopped lineage follow ends");

    // The Chat's events follow the same rule: each subscription streams once, and a newer stream
    // replaces the one before it.
    let first = session.subscribe().expect("the first subscription");
    session
        .events_with(first.id, holding(Ends(sender.clone())))
        .expect("the first subscription streams");
    let second = session.subscribe().expect("the second subscription");
    session
        .events_with(second.id, holding(Ends(sender)))
        .expect("the second subscription streams");
    ended
        .recv_timeout(wait)
        .expect("the replaced Chat stream ends");
    session.events_stop();
    ended
        .recv_timeout(wait)
        .expect("the stopped Chat stream ends");
    stop(service);
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
            .expect("the turns")
            .get(1)
            .is_some_and(|t| t.status == BridgeTurnStatus::Queued)
    });
    session
}

#[test]
fn a_session_queues_edits_cancels_and_stops_turns_through_the_bridge() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let project = host
        .project(fixture.repo().to_string_lossy().into_owned())
        .expect("project");

    let cancelled = queued_behind_a_running_turn(&project, "append doomed");
    cancelled.cancel_queued(2).expect("cancelled");
    // The cancelled turn stays listed, aborted, and never runs.
    until(|| {
        cancelled
            .turns()
            .expect("the turns")
            .get(1)
            .is_some_and(|t| t.status == BridgeTurnStatus::Aborted)
    });
    cancelled.stop().expect("stopped");
    until(|| cancelled.turns().expect("the turns")[0].status == BridgeTurnStatus::Aborted);

    let edited = queued_behind_a_running_turn(&project, "append queued");
    edited
        .edit_queued(2, "append edited".to_owned())
        .expect("edited");
    until(|| {
        edited
            .turns()
            .expect("the turns")
            .get(1)
            .is_some_and(|t| t.prompt == "append edited" && t.status == BridgeTurnStatus::Queued)
    });
    edited.stop().expect("stopped");
    until(|| edited.turns().expect("the turns")[0].status == BridgeTurnStatus::Aborted);

    let sent = queued_behind_a_running_turn(&project, "append now");
    // T3 refuses to steer a turn that has not started; the refusal reaches Liquid.
    assert!(sent.send_queued_now(2).is_err());
    sent.stop().expect("stopped");
    let session = sent;
    assert!(
        session.context().expect("the context").is_none(),
        "the agent reports no window"
    );
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
    follow(&asked, |s| {
        !s.snapshot().expect("the snapshot").requests.is_empty()
    });
    let request = asked.snapshot().expect("the snapshot").requests.remove(0);
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
    stop(service);
}

#[test]
fn git_switches_commits_pulls_pushes_and_initializes_through_the_bridge() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
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
    stop(service);
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
