use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, mpsc};
use std::time::{Duration, Instant};

use meta_harness_core::{
    AcpAgent, CONNECTION_FILE, Harness, ModelChoice, MovePackage, Permissions, Service,
    ServiceConfig,
};

use crate::api::details::BridgeLineageRelation;
use crate::api::events::{BridgeChangeStatus, BridgeEvent, BridgeEventKind, BridgeItemKind};
use crate::api::git::{BridgeGitAction, BridgeGitProgressKind};
use crate::api::moves::{BridgeArrivalKind, BridgeLandingKind, BridgeMovePackage};
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

    fn home(&self) -> PathBuf {
        self.0.join("meta-harness")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn t3_binary() -> PathBuf {
    std::env::var_os("META_HARNESS_T3")
        .expect("META_HARNESS_T3 names the pinned t3")
        .into()
}

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

fn begin(project: &AgentProject, prompt: &str) -> AgentSession {
    project
        .start(
            prompt.to_owned(),
            scripted(),
            Some("main".to_owned()),
            None,
            BridgePermissions::FullAccess,
        )
        .expect("session starts")
}

fn follow_until(
    session: &AgentSession,
    done: impl Fn(&AgentSession, &BridgeSnapshot, &[BridgeEvent]) -> bool,
) -> (BridgeSnapshot, Vec<BridgeEvent>) {
    let (sender, receiver) = mpsc::channel();
    let snapshot = session
        .forward(move |event| sender.send(event).is_ok())
        .expect("the session follows");
    let deadline = Instant::now() + STREAM_DEADLINE;
    let mut events = Vec::new();
    while !done(session, &snapshot, &events) {
        let event =
            next_before(&receiver, deadline).expect("the session goes on before its deadline");
        assert!(event.sequence > snapshot.sequence || event.kind == BridgeEventKind::FellBehind);
        events.push(event);
        session.confirm_applied(i64::try_from(events.len()).expect("count"));
    }
    (snapshot, events)
}

const STREAM_DEADLINE: Duration = Duration::from_secs(60);

fn next_before<T>(receiver: &mpsc::Receiver<T>, deadline: Instant) -> Option<T> {
    let left = deadline.checked_duration_since(Instant::now())?;
    receiver.recv_timeout(left).ok()
}

fn receive_until(
    receiver: &mpsc::Receiver<BridgeEvent>,
    done: impl Fn(&BridgeEvent) -> bool,
) -> Vec<BridgeEvent> {
    let deadline = Instant::now() + STREAM_DEADLINE;
    let mut events = Vec::new();
    loop {
        let event =
            next_before(receiver, deadline).expect("the session goes on before its deadline");
        let last = done(&event);
        events.push(event);
        if last {
            return events;
        }
    }
}

fn follow(session: &AgentSession, done: impl Fn(&AgentSession) -> bool) -> Vec<BridgeEvent> {
    follow_until(session, |session, _, _| done(session)).1
}

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

fn answer_reached_liquid(snapshot: &BridgeSnapshot, events: &[BridgeEvent]) -> bool {
    events
        .iter()
        .any(|e| e.kind == BridgeEventKind::ItemCompleted && e.text.as_deref() == Some("Appended."))
        || snapshot
            .transcript
            .iter()
            .any(|item| item.text == "Appended.")
}

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
    let project = project_at(&host, &fixture.repo());
    let session = begin(&project, "append first");
    until(|| completed(1)(&session));
    let (snapshot, events) = follow_turns(&session, 1);
    assert!(
        answer_reached_liquid(&snapshot, &events),
        "the answer reaches Liquid"
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
fn a_status_callback_reads_its_checkout_and_the_watch_goes_on() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let project = project_at(&host, &fixture.repo());
    let git = Arc::new(project.git());
    let (sender, reads) = mpsc::channel();
    let reader = git.clone();
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
fn follows_resume_after_meta_harness_restarts() {
    let fixture = Fixture::new();
    let (service, bridge) = host(&fixture);
    let project = project_at(&bridge, &fixture.repo());
    let session = begin(&project, "append first");
    let (sender, receiver) = mpsc::channel();
    let snapshot = session
        .forward(move |event| sender.send(event).is_ok())
        .expect("the session follows");
    if !snapshot
        .turns
        .iter()
        .any(|t| t.status == BridgeTurnStatus::Completed)
    {
        receive_until(&receiver, |e| e.kind == BridgeEventKind::TurnCompleted);
    }
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
    std::thread::sleep(Duration::from_secs(1));
    while servers_seen.try_recv().is_ok() {}
    while tasks_seen.try_recv().is_ok() {}
    while lineage_seen.try_recv().is_ok() {}
    let (service, _) = host(&fixture);
    receive_until(&receiver, |e| e.kind == BridgeEventKind::FellBehind);
    session
        .send("append second".to_owned(), false)
        .expect("second turn");
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
    let wait = Duration::from_secs(60);
    servers_seen.recv_timeout(wait).expect("the servers resume");
    tasks_seen
        .recv_timeout(wait)
        .expect("the scheduled tasks resume");
    let deadline = Instant::now() + STREAM_DEADLINE;
    assert!(
        std::iter::from_fn(|| next_before(&lineage_seen, deadline)).any(|read| read.is_ok()),
        "the lineage resumes before its deadline"
    );
    stop(service);
}

#[test]
fn an_event_stream_takes_the_subscription_it_names() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let session = begin(&project_at(&host, &fixture.repo()), "append first");
    let first = session.subscribe().expect("the first subscription");
    session
        .send("append second".to_owned(), false)
        .expect("second turn");
    follow_turns(&session, 2);
    let second = session.subscribe().expect("the second subscription");
    assert_ne!(first.id, second.id);

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
    for _ in 0..4 {
        session.subscribe().expect("a newer subscription");
    }
    assert!(
        session.events_with(second.id, |_| true).is_err(),
        "an abandoned subscription ends"
    );
    stop(service);
}

const FOLLOWERS: usize = 8;
const ROUNDS: usize = 4;

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
    let session = Arc::new(begin(&project_at(&host, &fixture.repo()), "append first"));
    until(|| completed(1)(&session));
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
    let project = project_at(&host, &fixture.repo());
    let first = begin(&project, "append first");
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
    let session = begin(&project, "append first");
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

struct Ends(mpsc::Sender<()>);

impl Drop for Ends {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

fn holding<T>(ends: Ends) -> impl FnMut(T) -> bool + Send + 'static {
    move |_| {
        let _ = &ends;
        true
    }
}

#[test]
fn follows_end_when_replaced_stopped_or_their_session_or_checkout_drops() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let project = project_at(&host, &fixture.repo());
    let session = begin(&project, "append first");
    follow_turns(&session, 1);
    let git = project.git();
    let (sender, ended) = mpsc::channel();
    let end = |what: &str| {
        ended
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("the {what} ends"))
    };

    session.servers_subscribe().expect("the servers");
    session
        .servers_with(holding(Ends(sender.clone())))
        .expect("the servers follow");
    session.servers_subscribe().expect("the servers");
    session
        .servers_with(holding(Ends(sender.clone())))
        .expect("the servers follow");
    end("replaced servers follow");

    session.lineage_subscribe().expect("the lineage");
    session
        .lineage_with(holding(Ends(sender.clone())))
        .expect("the lineage follows");
    session.lineage_subscribe().expect("the lineage");
    session
        .lineage_with(holding(Ends(sender.clone())))
        .expect("the lineage follows");
    end("replaced lineage follow");

    let first = session.subscribe().expect("the first subscription");
    session
        .events_with(first.id, holding(Ends(sender.clone())))
        .expect("the first subscription streams");
    let second = session.subscribe().expect("the second subscription");
    session
        .events_with(second.id, holding(Ends(sender.clone())))
        .expect("the second subscription streams");
    end("replaced Chat stream");

    session.lineage_stop();
    end("stopped lineage follow");
    session.events_stop();
    end("stopped Chat stream");

    session
        .automations_subscribe()
        .expect("the scheduled tasks");
    session
        .automations_with(holding(Ends(sender.clone())))
        .expect("the scheduled tasks follow");
    git.watch_with(holding(Ends(sender.clone())))
        .expect("the status watch");
    session.automations_stop();
    end("stopped scheduled tasks follow");
    git.watch_stop();
    end("stopped status watch");

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
    git.watch_with(holding(Ends(sender)))
        .expect("the status watch");
    drop(session);
    for section in ["servers follow", "scheduled tasks follow", "lineage follow"] {
        end(section);
    }
    drop(git);
    end("status watch");
    stop(service);
}

fn queued_behind_a_running_turn(project: &AgentProject, prompt: &str) -> AgentSession {
    let session = begin(project, "wait");
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
    let project = project_at(&host, &fixture.repo());

    let cancelled = queued_behind_a_running_turn(&project, "append doomed");
    cancelled.cancel_queued(2).expect("cancelled");
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
    assert!(sent.send_queued_now(2).is_err());
    sent.stop().expect("stopped");
    assert!(
        sent.context().expect("the context").is_none(),
        "the agent reports no window"
    );
    assert!(sent.dismiss("no such request".to_owned()).is_err());
    assert!(
        sent.answer("no such request".to_owned(), Vec::new())
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
    let project = project_at(&host, &repo);
    let git = project.git();

    git.create_branch("feature".to_owned()).expect("branch");
    assert!(
        git.refs("feat".to_owned(), 0, 10)
            .expect("refs")
            .refs
            .iter()
            .any(|r| r.name == "feature" && r.current)
    );
    git.switch("main".to_owned()).expect("switched");
    assert_eq!(
        git.status().expect("status").branch.as_deref(),
        Some("main")
    );
    std::fs::write(repo.join("README.md"), "bridge\nchanged\n").expect("change");
    assert!(
        git.uncommitted_patch("README.md".to_owned(), false)
            .expect("patch")
            .contains("+changed")
    );
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
        steps.iter().any(|s| s.kind == BridgeGitProgressKind::Phase),
        "the phases stream: {steps:?}"
    );
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

fn project_at(host: &AgentHost, folder: &Path) -> AgentProject {
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

#[test]
fn a_move_in_lands_in_the_project_it_names_not_the_handles_one() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);
    let repo = fixture.repo();
    let url = "https://example.invalid/acme/api.git";
    git(&repo, &["remote", "add", "origin", url]);
    let other = fixture.0.join("other");
    std::fs::create_dir_all(&other).expect("folder");
    git(&other, &["init", "-q", "-b", "main"]);
    let handle = host
        .add_host_project(repo.to_string_lossy().into_owned())
        .expect("the handle's project");
    host.add_host_project(other.to_string_lossy().into_owned())
        .expect("the project the move names");
    assert_eq!(
        handle.remote_url.as_deref(),
        Some(url),
        "T3 resolves the handle's remote"
    );
    let package = BridgeMovePackage {
        package: MovePackage {
            from_host: "elsewhere".to_owned(),
            title: "Move".to_owned(),
            harness: Harness::Codex,
            model: ModelChoice {
                instance: "codex".to_owned(),
                model: "gpt".to_owned(),
                options: Vec::new(),
            },
            permissions: Permissions::Supervised,
            branch: "feature".to_owned(),
            remote_url: url.to_owned(),
            handoff: "handoff".to_owned(),
            transcript: Vec::new(),
            transcript_capped: false,
            queued: Vec::new(),
            agent: None,
            transfer: "transfer".to_owned(),
        },
    };

    let refused = host.move_in(other.to_string_lossy().into_owned(), package);

    assert!(refused.thread.is_none() && !refused.lost);
    assert!(
        refused
            .message
            .contains("no Git remote that T3 can resolve"),
        "the project named for the move has no remote: {}",
        refused.message
    );
    stop(service);
}

#[test]
fn a_transfer_that_never_landed_answers_no_thread() {
    let fixture = Fixture::new();
    let (service, host) = host(&fixture);

    let arrival = host
        .move_status("never-sent".to_owned())
        .expect("the status of a transfer");
    assert_eq!(arrival.kind, BridgeArrivalKind::Unknown);
    assert!(arrival.thread.is_none());
    stop(service);
}

#[test]
fn a_target_project_answers_where_a_move_would_land() {
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
    let path = repo.to_string_lossy().into_owned();
    host.add_host_project(path.clone())
        .expect("the project the move names");
    git(&repo, &["branch", "existing"]);
    git(&repo, &["push", "-q", "-u", "origin", "existing"]);
    std::thread::sleep(Duration::from_secs(6));

    let fresh = host
        .preflight(path.clone(), "feature/fresh".to_owned())
        .expect("a landing");
    let attach = host
        .preflight(path.clone(), "existing".to_owned())
        .expect("a landing");
    let continued = host
        .preflight(path.clone(), "main".to_owned())
        .expect("a landing");
    std::fs::write(repo.join("README.md"), "dirty\n").expect("edit");
    let blocked = host.preflight(path, "main".to_owned()).expect("a landing");

    assert_eq!(fresh.kind, BridgeLandingKind::NewWorktree);
    assert_eq!(attach.kind, BridgeLandingKind::Attach, "{attach:?}");
    assert_eq!(continued.kind, BridgeLandingKind::Continue);
    assert_eq!(blocked.kind, BridgeLandingKind::Blocked);
    assert!(
        blocked
            .reason
            .is_some_and(|reason| reason.contains("uncommitted"))
    );
    stop(service);
}
