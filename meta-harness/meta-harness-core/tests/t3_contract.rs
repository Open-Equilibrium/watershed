#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use futures_util::FutureExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use meta_harness_core::{
    Access, Arrival, CONNECTION_FILE, Change, ClientKind, Landing, MoveFinish, MovePackage,
    MovePhase, Service, ServiceConfig, remote, t3_version,
};
use meta_harness_core::{
    AcpAgent, AgentChange, AgentChangeKind, AgentEvent, GitAction, GitActionRequest, GitProgress,
    Group, Harness, Host, HostConfig, ItemDetail, ItemKind, LineCount, ModelChoice, NewAgent,
    NewRoutine, Permissions, Relation, Request, Schedule, ScheduleKind, Snapshot, StartThread,
    StepStatus, TaskStep, Thread, ThreadEvent, TurnStatus, Workspace,
};

const AGENT: &str = "scripted";

fn t3() -> PathBuf {
    PathBuf::from(
        std::env::var_os("META_HARNESS_T3")
            .expect("META_HARNESS_T3 names the pinned t3 executable"),
    )
}

fn transfer() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    format!(
        "{:016x}{:016x}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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

fn seed_repo(repo: &Path) {
    std::fs::create_dir(repo).unwrap();
    git(repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "contract\n").unwrap();
    git(repo, &["config", "user.name", "Contract"]);
    git(repo, &["config", "user.email", "contract@example.invalid"]);
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", "start"]);
}

fn fixture() -> (tempdir::Dir, PathBuf, PathBuf) {
    let dir = tempdir::Dir::new();
    let repo = dir.path().join("repo");
    seed_repo(&repo);
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

async fn start_service(home: &Path) -> (Service, remote::Host) {
    start_with(service_config(home), home).await
}

async fn start_with(config: ServiceConfig, home: &Path) -> (Service, remote::Host) {
    let service =
        Service::start_with_remote(config, Some(vec![std::net::Ipv4Addr::LOCALHOST.into()]))
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

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn remote_on_free(host: &remote::Host) -> u16 {
    for _ in 0..5 {
        let port = free_port();
        match host.remote_on(port).await {
            Ok(_) => return port,
            Err(error) if error.contains("already in use") => continue,
            Err(error) => panic!("{error}"),
        }
    }
    panic!("no free port for remote access");
}

fn worktree() -> Workspace {
    Workspace::NewWorktree {
        base: "main".to_owned(),
    }
}

fn start(text: &str, workspace: Workspace, permissions: Permissions) -> StartThread {
    StartThread {
        message_id: None,
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
    let statistics = thread.statistics().await.unwrap();
    assert_eq!(statistics.turns.len(), 1);
    assert_eq!(statistics.turns[0].prompt, "append first");
    let edits: u32 = statistics
        .tools
        .iter()
        .map(|t| t.succeeded + t.failed + t.cancelled + t.open)
        .sum();
    assert!(edits >= 1, "the append's tool call counts: {statistics:?}");
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
    let changed = thread.git().status().await.unwrap().changes;
    assert_eq!(
        changed.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        ["contract.txt"]
    );

    assert_eq!(thread.revert(2, true).await.unwrap(), "append second");
    wait(&thread, |s| s.turns.len() == 1).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\n");

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
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\nthird\n");
    assert!(thread.diff(3, 3, false).await.unwrap().contains("+third"));
    assert_eq!(thread.revert(3, true).await.unwrap(), "append third");
    wait(&thread, |s| s.turns.len() == 1).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\n");

    assert_eq!(thread.revert(1, true).await.unwrap(), "append first");
    wait(&thread, |s| s.turns.is_empty()).await;
    assert!(!file.exists());

    thread.set_branch("main", Some(&repo)).await.unwrap();
    assert_eq!(
        thread.worktree().await.unwrap().as_deref(),
        Some(repo.as_path())
    );
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_s_plan_reaches_the_transcript_as_a_task_list() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let project = host.project(&repo).await.unwrap();
    let thread = project
        .start(start("plan", worktree(), Permissions::FullAccess))
        .await
        .unwrap();

    let snapshot = wait(&thread, completed(1)).await;

    let tasks = snapshot
        .transcript
        .iter()
        .find(|item| item.kind == ItemKind::TaskList)
        .expect("the plan is a task list item");
    let step = |text: &str| TaskStep {
        text: text.to_owned(),
        status: StepStatus::Completed,
    };
    assert_eq!(
        tasks.detail,
        ItemDetail::Tasks {
            steps: vec![step("Explore"), step("Explore › scraper-1")]
        }
    );
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_persistent_agent_moves_to_a_fresh_thread_with_its_routines_when_its_context_fills() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let created = host
        .agent_create(NewAgent {
            name: Some("news-bot".to_owned()),
            root: repo.to_string_lossy().into_owned(),
            model: start("", Workspace::Checkout, Permissions::FullAccess).model,
            permissions: Permissions::FullAccess,
            instructions: "Keep the news page current.".to_owned(),
            request: None,
        })
        .await
        .unwrap();
    let project = host.project(&repo).await.unwrap();
    let first = project.open(&created.thread).await.unwrap();
    wait(&first, completed(1)).await;
    let routine = host
        .agent_routine_add(
            "news-bot",
            NewRoutine {
                title: "Morning".to_owned(),
                prompt: "Check the feeds.".to_owned(),
                schedule: Schedule {
                    kind: ScheduleKind::FixedTime,
                    every_ms: None,
                    time_of_day: Some("07:30".to_owned()),
                    weekdays: Vec::new(),
                    run_at: None,
                },
            },
        )
        .await
        .unwrap();

    first.send("fill", false).await.unwrap();
    let moved = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let agents = host.agents().await.unwrap();
            if agents[0].generation == 1 && agents[0].previous.is_none() {
                return agents[0].clone();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("the agent moves on within a minute");
    let next = project.open(&moved.thread).await.unwrap();
    let opened = wait(&next, completed(1)).await;

    assert!(created.memory.is_dir());
    assert_ne!(moved.thread, created.thread);
    assert_eq!(
        opened.transcript[0].detail,
        ItemDetail::Opening { handoff: true }
    );
    assert_eq!(
        opened.transcript[0].text, "",
        "Liquid shows no opening text"
    );
    let opening = &opened.turns[0].prompt;
    assert!(opening.contains("Keep the news page current."), "{opening}");
    assert!(opening.contains(&created.memory.display().to_string()));
    assert!(opening.contains("Imported conversation history"));
    let routines = host.agent_routines("news-bot").await.unwrap();
    assert_eq!(
        routines.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        [routine.id.as_str()]
    );
    assert!(first.automations().await.unwrap().current().is_empty());

    service.stop().await;
    let (service, host) = start_service(&home).await;
    assert_eq!(host.agents().await.unwrap()[0].thread, moved.thread);
    host.agent_remove("news-bot").await.unwrap();
    assert!(host.agents().await.unwrap().is_empty());
    let project = host.project(&repo).await.unwrap();
    let next = project.open(&moved.thread).await.unwrap();
    assert!(next.automations().await.unwrap().current().is_empty());
    assert!(created.memory.is_dir(), "the memory stays");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_made_by_chat_names_itself_logs_its_changes_and_answers_a_mention() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let model = start("", Workspace::Checkout, Permissions::FullAccess).model;
    let created = host
        .agent_create(NewAgent {
            name: None,
            root: repo.to_string_lossy().into_owned(),
            model,
            permissions: Permissions::FullAccess,
            instructions: String::new(),
            request: Some("instruct".to_owned()),
        })
        .await
        .unwrap();
    let project = host.project(&repo).await.unwrap();
    let thread = project.open(&created.thread).await.unwrap();
    let opened = wait(&thread, completed(1)).await;
    assert_eq!(
        opened.transcript[0].text, "instruct",
        "Liquid shows the person's words"
    );
    let named = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let agents = host.agents().await.unwrap();
            if agents[0].named {
                return agents[0].clone();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("the agent names itself after its first turn");
    assert_eq!(named.name, "news-bot");
    assert_eq!(named.memory, created.memory);
    let kinds = |changes: Vec<AgentChange>| changes.into_iter().map(|c| c.kind).collect::<Vec<_>>();
    let changes = host.agent_changes("news-bot").await.unwrap();
    assert_eq!(
        kinds(changes.clone()),
        [AgentChangeKind::Name, AgentChangeKind::Instructions]
    );
    assert_eq!(changes[1].turn, Some(1));

    let routine = host
        .agent_routine_add(
            "news-bot",
            NewRoutine {
                title: "Morning".to_owned(),
                prompt: "reinstruct".to_owned(),
                schedule: Schedule {
                    kind: ScheduleKind::Interval,
                    every_ms: Some(3_600_000),
                    time_of_day: None,
                    weekdays: Vec::new(),
                    run_at: None,
                },
            },
        )
        .await
        .unwrap();
    host.agent_undo("news-bot", changes[1].id).await.unwrap();
    assert_eq!(host.agent_instructions("news-bot").await.unwrap(), "");
    assert!(host.agent_undo("news-bot", changes[1].id).await.is_err());
    let after = host.agent_changes("news-bot").await.unwrap();
    assert_eq!(
        after.len(),
        2,
        "the person's own changes are not the agent's"
    );
    assert!(after[1].undone);

    thread.run_automation_now(&routine.id).await.unwrap();
    wait(&thread, completed(2)).await;
    let held = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let changes = host.agent_changes("news-bot").await.unwrap();
            if changes.len() == 3 {
                return changes[2].clone();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("the routine's change is noticed");
    assert_eq!(held.kind, AgentChangeKind::Instructions);
    assert!(held.held, "a change from a routine waits for approval");
    assert_eq!(
        host.agent_instructions("news-bot").await.unwrap(),
        "",
        "and is undone until then"
    );
    host.agent_apply("news-bot", held.id).await.unwrap();
    assert_eq!(
        host.agent_instructions("news-bot").await.unwrap(),
        "# news-bot\n\nRead the feeds twice.\n"
    );
    assert!(!host.agent_changes("news-bot").await.unwrap()[2].held);
    assert!(host.agent_apply("news-bot", held.id).await.is_err());

    let stopped = host
        .agent_routine_add(
            "news-bot",
            NewRoutine {
                title: "Stopped".to_owned(),
                prompt: "rewrite-wait".to_owned(),
                schedule: Schedule {
                    kind: ScheduleKind::Interval,
                    every_ms: Some(3_600_000),
                    time_of_day: None,
                    weekdays: Vec::new(),
                    run_at: None,
                },
            },
        )
        .await
        .unwrap();
    thread.run_automation_now(&stopped.id).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    thread.stop().await.unwrap();
    let aborted = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let changes = host.agent_changes("news-bot").await.unwrap();
            if changes.len() == 4 {
                return changes[3].clone();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("a stopped routine run's change is noticed");
    assert!(aborted.held, "a routine run that was stopped is held too");
    assert_eq!(
        host.agent_instructions("news-bot").await.unwrap(),
        "# news-bot\n\nRead the feeds twice.\n"
    );
    host.agent_set_instructions("news-bot", "# news-bot\n\nRead.\n")
        .await
        .unwrap();
    assert!(
        host.agent_apply("news-bot", aborted.id).await.is_err(),
        "a held change does not overwrite what changed since"
    );
    host.agent_undo("news-bot", aborted.id).await.unwrap();
    assert_eq!(
        host.agent_instructions("news-bot").await.unwrap(),
        "# news-bot\n\nRead.\n",
        "discarding a held change keeps the current instructions"
    );
    host.agent_routine_remove("news-bot", &stopped.id)
        .await
        .unwrap();

    let chat = project
        .start(start(
            "Fix the move dialog",
            Workspace::Checkout,
            Permissions::FullAccess,
        ))
        .await
        .unwrap();
    wait(&chat, completed(1)).await;
    let message = host
        .agent_mention(
            "news-bot",
            &repo.to_string_lossy(),
            chat.id(),
            "Is main green?",
        )
        .await
        .unwrap();
    let answered = wait(&thread, completed(3)).await;
    let asked = answered
        .turns
        .iter()
        .find(|turn| turn.prompt.contains("Is main green?"))
        .expect("the mention is a turn of the agent");
    assert!(asked.prompt.contains("t3-context://v1/thread/"));
    let mentions = host.agent_mentions(chat.id()).await.unwrap();
    assert_eq!(
        (
            mentions[0].name.as_str(),
            mentions[0].message.as_str(),
            mentions[0].answered,
            mentions[0].answer.as_str()
        ),
        ("news-bot", message.as_str(), true, "Done.")
    );

    host.agent_remove("news-bot").await.unwrap();
    assert!(host.agent_mentions(chat.id()).await.unwrap().is_empty());
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
    std::mem::forget(first);
    let second = host(&home).await;
    second.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn t3_starts_in_a_data_folder_that_does_not_exist_yet() {
    let (_dir, _repo, home) = fixture();
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
    let (dir, _repo, home) = fixture();
    let other_home = dir.path().join("other");
    let other = host(&other_home).await;
    let record = runtime_record(&other_home);
    let address = record["origin"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://");
    let mut sleeper = Command::new("sleep").arg("60").spawn().unwrap();
    for pid in [sleeper.id().into(), record["pid"].clone()] {
        std::fs::create_dir_all(home.join("userdata")).unwrap();
        std::fs::write(
            home.join("userdata").join("server-runtime.json"),
            serde_json::json!({ "pid": pid }).to_string(),
        )
        .unwrap();
        let host = host(&home).await;
        assert!(
            sleeper.try_wait().unwrap().is_none(),
            "the other program still runs"
        );
        assert!(
            std::net::TcpStream::connect(address).is_ok(),
            "the other folder's server still listens"
        );
        host.shutdown().await;
    }
    other.shutdown().await;
    sleeper.kill().unwrap();
    sleeper.wait().unwrap();
}

fn runtime_record(home: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(home.join("userdata").join("server-runtime.json")).unwrap();
    serde_json::from_str(&text).unwrap()
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
    wait_core(&thread, completed(1)).await;
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
    let completions_after = received
        .iter()
        .filter(|e| matches!(e.event, AgentEvent::TurnCompleted { .. }))
        .count();
    assert_eq!(
        usize::from(finished_before) + completions_after,
        1,
        "the completion is in the snapshot or the stream, exactly once"
    );
    assert!(after.transcript.iter().any(|item| item.text == "Appended."));
    host.shutdown().await;
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
    seen.extend(std::iter::from_fn(|| events.recv().now_or_never().flatten()).map(|e| e.event));
    assert!(seen.contains(&AgentEvent::RequestResolved {
        request: asked.id.clone()
    }));
    assert!(done.requests.is_empty());
    assert!(done.transcript.iter().any(|item| item.text == "Approved."));
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
async fn the_host_names_its_machine_kind_and_keeps_one_writer_model() {
    let (_dir, _repo, home) = fixture();
    std::fs::create_dir_all(home.join("t3/userdata")).unwrap();
    std::fs::write(
        home.join("t3/userdata/settings.json"),
        r#"{"environmentIcon":"laptop"}"#,
    )
    .unwrap();
    let (service, host) = start_service(&home).await;
    assert_eq!(host.environment_machine().await.unwrap(), "laptop");
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
    assert_eq!(pushed_subject(&origin, "feature"), "Change the readme");

    git.switch("main").await.unwrap();
    assert_eq!(git.status().await.unwrap().branch.as_deref(), Some("main"));
    let plain = dir.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    let plain_git = host.project(&plain).await.unwrap().git();
    assert!(!plain_git.status().await.unwrap().repository);
    plain_git.init().await.unwrap();
    assert!(plain_git.status().await.unwrap().repository);
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

mod tempdir {
    pub struct Dir(std::path::PathBuf);

    impl Dir {
        pub fn new() -> Self {
            let mut bytes = [0u8; 8];
            getrandom::fill(&mut bytes).unwrap();
            let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
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
    thread.send("append second", false).await.unwrap();
    wait(&thread, completed(2)).await;

    let fork_id = thread.fork(1).await.unwrap();
    let fork = project.open(&fork_id).await.unwrap();
    let forked = wait(&fork, |s| {
        s.transcript.iter().any(|i| i.text == "append first")
    })
    .await;
    assert!(forked.transcript.iter().all(|i| i.text != "append second"));
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
async fn a_dropped_service_ends_its_connections_and_streams_and_frees_its_folder() {
    let (dir, repo, _) = fixture();
    let home = dir.path().join("meta");
    let (service, client) = start_service(&home).await;
    let _statuses = client
        .project(&repo)
        .await
        .unwrap()
        .git()
        .watch()
        .await
        .unwrap();
    let (url, token) = (service.url.clone(), service.token.clone());
    drop(service);
    assert!(!home.join(CONNECTION_FILE).exists());
    tokio::time::timeout(Duration::from_secs(10), async {
        while client.status().await.is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        while remote::Host::connect_to(&url, &token, "contract")
            .await
            .is_ok()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a dropped service ends its connections and stops listening");
    let started = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(service) = Service::start(service_config(&home)).await {
                return service;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(
        started.is_ok(),
        "the folder is still held by the dropped service's stream"
    );
    started.unwrap().stop().await;
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
        assert_eq!(refused.headers()["meta-harness-api"], "1");
    }
    service.stop().await;
}

async fn open_client(
    service: &Service,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = service.url.as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", service.token).parse().unwrap(),
    );
    request
        .headers_mut()
        .insert("meta-harness-api", "1".parse().unwrap());
    tokio_tungstenite::connect_async(request).await.unwrap().0
}

async fn disconnected_after_flood(
    service: &Service,
    text: &str,
    limit: Duration,
) -> Option<Duration> {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let mut socket = open_client(service).await;
    let started = std::time::Instant::now();
    tokio::time::timeout(limit, async {
        loop {
            if socket
                .send(Message::Text(text.to_owned().into()))
                .await
                .is_err()
            {
                return;
            }
        }
    })
    .await
    .ok()
    .map(|()| started.elapsed())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_stops_reading_is_disconnected_for_calls_and_malformed_requests() {
    let (dir, _repo, _home) = fixture();
    let (service, _local) = start_service(&dir.path().join("meta")).await;
    for text in [
        r#"{"id":1,"bogus":true}"#,
        r#"{"id":1,"call":{"host":"status"}}"#,
    ] {
        assert!(
            disconnected_after_flood(&service, text, Duration::from_secs(8))
                .await
                .is_some(),
            "a client that never reads its replies is disconnected: {text}"
        );
    }
    service.stop().await;
}

async fn connects(url: &str, token: &str, name: &str) -> bool {
    remote::Host::connect_to(url, token, name).await.is_ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_names_and_host_addresses_are_checked_before_use() {
    let (dir, _repo, _home) = fixture();
    let (_service, local) = start_service(&dir.path().join("meta")).await;
    let port = remote_on_free(&local).await;
    let url = format!("ws://127.0.0.1:{port}");
    let code = local
        .invite(ClientKind::Device, Access::Auto)
        .await
        .unwrap();
    remote::pair(&url, &code, "phone\tlaptop").await.unwrap();
    let clients = local.hosts().await.unwrap().clients;
    assert_eq!(
        clients[0].name, "phonelaptop",
        "meta host list prints the name"
    );
    local.host_agents(true).await.unwrap();
    let error = local
        .host_add("127.0.0.1:3774", "AAAA-AAAA", None)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        "Enter the address as ws://HOST:PORT, as meta invite prints it."
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_access_refuses_port_zero() {
    let (dir, _repo, _home) = fixture();
    let (_service, local) = start_service(&dir.path().join("meta")).await;
    assert!(
        local.remote_on(0).await.is_err(),
        "port 0 is no port to listen on"
    );
    assert!(local.remote_status().await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_access_that_cannot_bind_at_start_says_so_and_binds_once_the_port_is_free() {
    let (dir, _repo, _home) = fixture();
    let meta = dir.path().join("meta");
    let (service, local) = start_service(&meta).await;
    let port = remote_on_free(&local).await;
    drop(local);
    service.stop().await;
    let holder = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let (service, local) = start_service(&meta).await;
    let waiting = local.remote_status().await.unwrap().expect("still on");
    assert!(waiting.addresses.is_empty());
    assert!(waiting.waiting.is_some(), "on, not listening yet");
    drop(holder);
    let listening = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let status = local.remote_status().await.unwrap().expect("still on");
            if status.waiting.is_none() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await;
    assert!(listening.is_ok(), "bound again");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_remote_listener_closes_a_handshake_beyond_its_limit_at_once() {
    let (dir, _repo, _home) = fixture();
    let (_service, local) = start_service(&dir.path().join("meta")).await;
    let port = remote_on_free(&local).await;
    let address = format!("127.0.0.1:{port}");
    let idle =
        futures_util::future::join_all((0..32).map(|_| tokio::net::TcpStream::connect(&address)))
            .await;
    let mut extra = tokio::net::TcpStream::connect(&address).await.unwrap();
    let mut byte = [0u8; 1];
    let closed = tokio::time::timeout(Duration::from_secs(5), extra.read(&mut byte))
        .await
        .expect("closed without waiting for a handshake")
        .unwrap();
    assert_eq!(closed, 0, "the extra connection is closed");
    drop(idle);
}

#[tokio::test(flavor = "multi_thread")]
async fn turning_remote_access_off_ends_the_connections_it_admitted() {
    let (dir, _repo, _home) = fixture();
    let (_service, local) = start_service(&dir.path().join("meta")).await;
    let port = remote_on_free(&local).await;
    let url = format!("ws://127.0.0.1:{port}");
    let device = connect_paired(&local, &url, ClientKind::Device, "phone").await;
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
    let second = local
        .invite(ClientKind::Device, Access::Auto)
        .await
        .unwrap();
    let refused = remote::pair(&url, &second, "phone").await.unwrap_err();
    assert_eq!(
        refused,
        "A client named phone is paired already; pair under another name."
    );
    remote::pair(&url, &second, "tablet")
        .await
        .expect("the code was not spent");

    let device = remote::Host::connect_to(&url, &token, "phone")
        .await
        .unwrap();
    device.status().await.unwrap();
    device.config().await.unwrap();
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

fn t3_environments(home: &Path) -> Vec<serde_json::Value> {
    let output = Command::new(t3())
        .args(["environment", "list", "--json"])
        .env("T3CODE_HOME", home.join("t3"))
        .env("T3CODE_TELEMETRY_ENABLED", "false")
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

fn linked_environments_setting(home: &Path) -> bool {
    let text = std::fs::read_to_string(home.join("t3/userdata/settings.json")).unwrap();
    serde_json::from_str::<serde_json::Value>(&text).unwrap()["linkedEnvironmentsEnabled"]
        .as_bool()
        .unwrap_or(false)
}

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
    let linked = t3_environments(&home_a);
    let environment = std::fs::read_to_string(home_b.join("t3/userdata/environment-id")).unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0]["environmentId"], environment.trim());
    assert_eq!(linked[0]["status"], "reachable");

    let (tunnel, _) = peer_row(&home_a, &name);
    let client = a.host_remove(&name).await.unwrap();
    assert!(host_client_names(&b).await.contains(&client), "{client}");
    assert!(t3_environments(&home_a).is_empty(), "T3 forgets the link");
    assert!(a.hosts().await.unwrap().hosts.is_empty());
    assert!(std::net::TcpStream::connect(("127.0.0.1", tunnel)).is_err());
    a.host_agents(false).await.unwrap();
    assert!(!linked_environments_setting(&home_a));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_without_an_access_level_takes_the_level_its_host_invited() {
    let (dir, _repo, _home) = fixture();
    let ((_a, a), (_b, b), url_b) = two_hosts(dir.path()).await;
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
async fn a_host_pairs_under_a_tagged_name_that_its_revoke_message_quotes() {
    let (dir, _repo, _home) = fixture();
    let ((_a, a), (_b, b), url_b) = two_hosts(dir.path()).await;
    let code = b
        .invite(ClientKind::Host, Access::ApprovalRequired)
        .await
        .unwrap();
    let error = a
        .host_add(&url_b, &code, Some(Access::Auto))
        .await
        .unwrap_err();
    let me = a.environment_name().await.unwrap().to_lowercase();
    let pairing = b
        .hosts()
        .await
        .unwrap()
        .clients
        .into_iter()
        .find(|client| client.kind == ClientKind::Host)
        .expect("b paired with a")
        .name;
    let (host, tag) = pairing.rsplit_once('-').expect("the host name, then a tag");
    assert!(!host.is_empty() && me.trim().starts_with(host), "{pairing}");
    assert!(
        tag.len() == 4 && tag.chars().all(|c| c.is_ascii_hexdigit()),
        "{pairing}"
    );
    assert!(
        error.contains(&format!("meta host revoke {pairing}")),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_already_linked_at_an_address_is_not_paired_again() {
    let (dir, _repo, _home) = fixture();
    let (link, name) = linked(dir.path()).await;
    let (_b, b) = &link.b;
    let (_a, a) = &link.a;
    let clients = b.hosts().await.unwrap().clients.len();
    let code = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let error = a
        .host_add(&link.url_b, &code, Some(Access::Auto))
        .await
        .unwrap_err();
    assert!(error.contains(&name), "{error}");
    assert_eq!(
        b.hosts().await.unwrap().clients.len(),
        clients,
        "no stray pairing"
    );
    assert_eq!(a.hosts().await.unwrap().hosts.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_link_whose_tunnel_port_is_taken_at_restart_moves_to_a_new_port() {
    let (dir, _repo, _home) = fixture();
    let home_a = dir.path().join("a");
    let ((a_service, a), (_b, b), url_b) = two_hosts(dir.path()).await;
    let code = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let (name, _) = a.host_add(&url_b, &code, None).await.unwrap();
    let old = peer_row(&home_a, &name).0;
    drop(a);
    a_service.stop().await;

    let holder = std::net::TcpListener::bind(("127.0.0.1", old)).unwrap();
    let (_a_service, a) = start_service(&home_a).await;
    assert_ne!(peer_row(&home_a, &name).0, old, "the tunnel moved");
    let linked = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if t3_environments(&home_a)
                .iter()
                .any(|link| link["status"] == "reachable")
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await;
    assert!(linked.is_ok(), "linked again through the new port");
    assert!(a.hosts().await.unwrap().hosts[0].reachable);
    drop(holder);
}

#[allow(clippy::result_large_err)]
async fn silent_peer() -> String {
    use futures_util::SinkExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            let mut pairing = false;
            let Ok(mut socket) = tokio_tungstenite::accept_hdr_async(
                socket,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                 response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    pairing = request.headers().contains_key(meta_harness_core::api::PAIRING_HEADER);
                    Ok(response)
                },
            )
            .await
            else {
                continue;
            };
            if pairing {
                let answer = r#"{"name":"silent","token":"silent-token","access":"auto"}"#;
                let _ = socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(answer.into()))
                    .await;
                let _ = socket.close(None).await;
            } else {
                held.push(socket);
            }
        }
    });
    url
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_that_never_answers_a_link_is_given_up_on_and_holds_no_other_change() {
    let (dir, _repo, _home) = fixture();
    let ((_a, a), (_b, b), url_b) = two_hosts(dir.path()).await;
    let silent = silent_peer().await;
    let code = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let given_up = tokio::time::timeout(Duration::from_secs(60), a.host_add(&silent, &code, None))
        .await
        .expect("the link gives up on a peer that never answers");
    assert!(given_up.is_err());
    let code = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let linked = tokio::time::timeout(Duration::from_secs(60), a.host_add(&url_b, &code, None))
        .await
        .expect("a link is not held behind the silent one");
    assert!(linked.is_ok(), "{linked:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_host_ends_the_tunnel_connections_open_to_it() {
    let (dir, _repo, _home) = fixture();
    let (link, name) = linked(dir.path()).await;
    let (_a, a) = &link.a;
    let port = peer_row(&dir.path().join("a"), &name).0;
    let mut tunnel = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    a.host_remove(&name).await.unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        let mut byte = [0u8; 1];
        tunnel.read(&mut byte).await
    })
    .await
    .expect("the connection ends with its tunnel");
    assert_eq!(ended.unwrap(), 0, "the tunnel's connection is closed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removal_the_store_refuses_keeps_the_tunnel_and_can_be_repeated() {
    let (dir, _repo, _home) = fixture();
    let (link, name) = linked(dir.path()).await;
    let (_a, a) = &link.a;
    let home_a = dir.path().join("a");
    let (tunnel, _) = peer_row(&home_a, &name);
    let trigger = "CREATE TRIGGER refuse_peer_delete BEFORE DELETE ON peer \
                   BEGIN SELECT RAISE(ABORT, 'refused'); END;";
    rusqlite::Connection::open(home_a.join("meta-harness.sqlite"))
        .unwrap()
        .execute_batch(trigger)
        .unwrap();

    assert!(a.host_remove(&name).await.is_err());
    assert_eq!(a.hosts().await.unwrap().hosts.len(), 1);
    assert!(std::net::TcpStream::connect(("127.0.0.1", tunnel)).is_ok());
    assert_eq!(peer_row(&home_a, &name).1, None);

    rusqlite::Connection::open(home_a.join("meta-harness.sqlite"))
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_peer_delete")
        .unwrap();
    a.host_remove(&name).await.unwrap();
    assert!(a.hosts().await.unwrap().hosts.is_empty());
    assert!(std::net::TcpStream::connect(("127.0.0.1", tunnel)).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_asks_for_no_more_than_its_invite_and_names_the_pairing_it_made() {
    let (dir, _repo, _home) = fixture();
    let ((_a, a), (_b, b), url_b) = two_hosts(dir.path()).await;
    let code = b.invite(ClientKind::Host, Access::ReadOnly).await.unwrap();
    let error = a
        .host_add(&url_b, &code, Some(Access::Auto))
        .await
        .unwrap_err();
    assert!(
        error.contains("invited this host at read-only; ask for that or less."),
        "{error}"
    );
    assert!(
        a.hosts().await.unwrap().hosts.is_empty(),
        "nothing is linked"
    );

    let code = b
        .invite(ClientKind::Host, Access::ApprovalRequired)
        .await
        .unwrap();
    assert!(
        a.host_add(&url_b, &code, Some(Access::Auto)).await.is_err(),
        "an approval-required invite does not link at auto"
    );
    let code = b
        .invite(ClientKind::Host, Access::ApprovalRequired)
        .await
        .unwrap();
    a.host_add(&url_b, &code, Some(Access::ReadOnly))
        .await
        .unwrap();
    let listed = a.hosts().await.unwrap();
    assert_eq!(listed.hosts[0].access, Access::ReadOnly);
    assert!(listed.hosts[0].reachable);
    let clients = b.hosts().await.unwrap().clients;
    let pairing = clients
        .iter()
        .find(|client| client.name == listed.hosts[0].client)
        .expect("the pairing the link made");
    assert_eq!(
        pairing.access,
        Access::ApprovalRequired,
        "the level the host was invited at"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_made_is_reported_made_even_when_its_time_cannot_be_stored() {
    let (dir, _repo, _home) = fixture();
    let home_a = dir.path().join("a");
    let ((_a, a), (_b, b), url_b) = two_hosts(dir.path()).await;
    rusqlite::Connection::open(home_a.join("meta-harness.sqlite"))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse_link_time BEFORE UPDATE ON peer \
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
    let code = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let (name, _) = a
        .host_add(&url_b, &code, None)
        .await
        .expect("the link is made, so the add reports it");
    assert!(a.hosts().await.unwrap().hosts[0].reachable, "{name}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_link_older_than_25_days_is_made_again_when_the_service_starts() {
    let (dir, _repo, _home) = fixture();
    let home_a = dir.path().join("a");
    let ((a_service, a), (_b, b), url_b) = two_hosts(dir.path()).await;
    let code = b.invite(ClientKind::Host, Access::Auto).await.unwrap();
    let (name, _) = a
        .host_add(&url_b, &code, Some(Access::ReadOnly))
        .await
        .unwrap();
    drop(a);
    a_service.stop().await;

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

async fn may_tunnel(url: &str, token: &str) -> bool {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = url.into_client_request().unwrap();
    let headers = request.headers_mut();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    headers.insert("meta-harness-api", "1".parse().unwrap());
    headers.insert("meta-harness-tunnel", "t3".parse().unwrap());
    tokio_tungstenite::connect_async(request).await.is_ok()
}

struct Hosts {
    a: (Service, remote::Host),
    b: (Service, remote::Host),
    url_b: String,
}

async fn two_hosts(dir: &Path) -> ((Service, remote::Host), (Service, remote::Host), String) {
    let (a_service, a) = start_service(&dir.join("a")).await;
    let (b_service, b) = start_service(&dir.join("b")).await;
    remote_on_free(&a).await;
    let port_b = remote_on_free(&b).await;
    a.host_agents(true).await.unwrap();
    b.host_agents(true).await.unwrap();
    (
        (a_service, a),
        (b_service, b),
        format!("ws://127.0.0.1:{port_b}"),
    )
}

fn tunnel_addr(home: &Path, name: &str) -> String {
    format!("127.0.0.1:{}", peer_row(home, name).0)
}

async fn connect_paired(
    on: &remote::Host,
    url: &str,
    kind: ClientKind,
    name: &str,
) -> remote::Host {
    let code = on.invite(kind, Access::Auto).await.unwrap();
    let token = remote::pair(url, &code, name).await.unwrap().token;
    remote::Host::connect_to(url, &token, name).await.unwrap()
}

async fn linked(dir: &Path) -> (Hosts, String) {
    let (a, b, url_b) = two_hosts(dir).await;
    let link = Hosts { a, b, url_b };
    let code = link
        .b
        .1
        .invite(ClientKind::Host, Access::Auto)
        .await
        .unwrap();
    let (name, _) = link
        .a
        .1
        .host_add(&link.url_b, &code, Some(Access::Auto))
        .await
        .unwrap();
    (link, name)
}

const FORM: &[(&str, &str)] = &[("Content-Type", "application/x-www-form-urlencoded")];
const JSON: &[(&str, &str)] = &[("Content-Type", "application/json")];

fn http(addr: &str, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> String {
    http_at(addr, addr, method, path, headers, body)
}

fn http_at(
    connect: &str,
    host: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> String {
    let mut stream = std::net::TcpStream::connect(connect).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    std::io::Write::write_all(&mut stream, request.as_bytes()).unwrap();
    let mut answer = String::new();
    let _ = std::io::Read::read_to_string(&mut stream, &mut answer);
    answer
}

fn post(addr: &str, path: &str, headers: &[(&str, &str)], body: &str) -> serde_json::Value {
    let answer = http(addr, "POST", path, headers, body);
    assert!((200..300).contains(&status_of(&answer)), "{path}: {answer}");
    body_json(&answer)
}

fn status_of(answer: &str) -> u16 {
    answer
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

fn body_json(answer: &str) -> serde_json::Value {
    let body = &answer[answer.find('{').unwrap()..=answer.rfind('}').unwrap()];
    serde_json::from_str(body).unwrap()
}

fn t3_address(home: &Path) -> String {
    let text = std::fs::read_to_string(home.join("t3/userdata/server-runtime.json")).unwrap();
    let origin = serde_json::from_str::<serde_json::Value>(&text).unwrap()["origin"]
        .as_str()
        .unwrap()
        .to_owned();
    origin.trim_start_matches("http://").to_owned()
}

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
    let (_link, name) = linked(dir.path()).await;
    let port = peer_row(&dir.path().join("a"), &name).0;
    let addr = format!("127.0.0.1:{port}");
    for (method, path) in [
        ("GET", "/api/auth/session"),
        ("POST", "/api/auth/browser-session"),
        ("POST", "/oauth/token"),
        ("POST", "/api/auth/websocket-ticket"),
        ("GET", "/api/projects"),
        ("GET", "/api/orchestration/shell"),
        ("GET", "/ws"),
        ("POST", "/.well-known/t3/environment"),
        ("GET", "/oauth/mcp/token"),
        ("PUT", "/mcp"),
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
    let origin = t3_address(&home_b);
    for (access, name, scope) in [
        (
            Access::Auto,
            "other",
            "orchestration:read orchestration:operate",
        ),
        (Access::ReadOnly, "reader", "orchestration:read"),
    ] {
        let code = b.invite(ClientKind::Host, access).await.unwrap();
        let paired = remote::pair(&url, &code, name).await.unwrap();
        assert_eq!(paired.access, access);
        let host = remote::Host::connect_to(&url, &paired.token, name)
            .await
            .unwrap();
        for requested in ["terminal:operate", "filesystem:write", "settings:write"] {
            let credential = host.t3_pairing().await.unwrap();
            let answer = exchange(&origin, &credential, Some(requested));
            assert_ne!(status_of(&answer), 200, "{requested} is refused: {answer}");
        }
        let answer = exchange(&origin, &host.t3_pairing().await.unwrap(), None);
        assert_eq!(status_of(&answer), 200, "{answer}");
        assert_eq!(body_json(&answer)["scope"], scope);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_client_closes_the_connections_it_has_open() {
    let (dir, _repo, _home) = fixture();
    let (link, name) = linked(dir.path()).await;
    let (_b, b) = &link.b;
    let device = connect_paired(b, &link.url_b, ClientKind::Device, "phone").await;
    device.status().await.unwrap();
    let port = peer_row(&dir.path().join("a"), &name).0;
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
    let host = b
        .hosts()
        .await
        .unwrap()
        .clients
        .into_iter()
        .find(|client| client.kind == ClientKind::Host)
        .unwrap();
    b.revoke_client(&host.name).await.unwrap();
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

const REDIRECT: &str = "http://127.0.0.1/t3code-peer-link";

const PKCE_CHALLENGE: &str = "_-BU_nrgy23GXDr5th1SCfQ5hR20PQulmXM33xVGaOs";

fn register_client(addr: &str) -> String {
    let registered = post(
        addr,
        "/oauth/mcp/register",
        JSON,
        &serde_json::json!({"client_name": "tester", "redirect_uris": [REDIRECT]}).to_string(),
    );
    registered["client_id"].as_str().unwrap().to_owned()
}

fn decide(addr: &str, client_id: &str, pairing: &str) -> String {
    http(
        addr,
        "POST",
        "/oauth/mcp/decision",
        JSON,
        &serde_json::json!({
            "authorization": {
                "response_type": "code",
                "client_id": client_id,
                "redirect_uri": REDIRECT,
                "code_challenge": PKCE_CHALLENGE,
                "code_challenge_method": "S256",
                "state": "state",
                "resource": format!("http://{addr}/mcp"),
            },
            "decision": {"_tag": "pairing-code", "access": "auto", "code": pairing},
        })
        .to_string(),
    )
}

fn approve(addr: &str, client_id: &str, pairing: &str) -> String {
    let answer = decide(addr, client_id, pairing);
    assert_eq!(status_of(&answer), 200, "{answer}");
    query(body_json(&answer)["redirectTo"].as_str().unwrap(), "code")
}

fn code_form(code: &str, client_id: &str, addr: &str) -> String {
    format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}&resource={}",
        form_value(code),
        form_value(REDIRECT),
        form_value(client_id),
        "a".repeat(64),
        form_value(&format!("http://{addr}/mcp")),
    )
}

fn sign_in(addr: &str, pairing: &str) -> String {
    let client_id = register_client(addr);
    let code = approve(addr, &client_id, pairing);
    let token = post(
        addr,
        "/oauth/mcp/token",
        FORM,
        &code_form(&code, &client_id, addr),
    );
    format!("Bearer {}", token["access_token"].as_str().unwrap())
}

fn mcp_answer(connect: &str, host: &str, bearer: &str) -> String {
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
    http_at(
        connect,
        host,
        "POST",
        "/mcp",
        &[
            ("Authorization", bearer),
            ("Accept", "application/json, text/event-stream"),
            ("Content-Type", "application/json"),
        ],
        &initialize.to_string(),
    )
}

fn t3_sessions(home: &Path) -> Vec<serde_json::Value> {
    let output = Command::new(t3())
        .args(["auth", "session", "list", "--json"])
        .env("T3CODE_HOME", home.join("t3"))
        .env("T3CODE_TELEMETRY_ENABLED", "false")
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_agents_mcp_request_reaches_t3_through_the_tunnel() {
    let (dir, _repo, _home) = fixture();
    let (link, name) = linked(dir.path()).await;
    let host = connect_paired(&link.b.1, &link.url_b, ClientKind::Host, "tester").await;
    let addr = tunnel_addr(&dir.path().join("a"), &name);
    let bearer = sign_in(&addr, &host.t3_pairing().await.unwrap());
    let answer = mcp_answer(&addr, &addr, &bearer);
    assert_eq!(status_of(&answer), 200, "{answer}");
    assert!(answer.contains("protocolVersion"), "{answer}");
}

async fn host_client_names(on: &remote::Host) -> Vec<String> {
    on.hosts()
        .await
        .unwrap()
        .clients
        .into_iter()
        .filter(|client| client.kind == ClientKind::Host)
        .map(|client| client.name)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_decision_or_token_exchange_with_another_pairings_client_id_is_refused_by_the_tunnel() {
    let (dir, _repo, _home) = fixture();
    let (link, name_a) = linked(dir.path()).await;
    let home_c = dir.path().join("c");
    let (_c, c) = start_service(&home_c).await;
    c.host_agents(true).await.unwrap();
    let code = link
        .b
        .1
        .invite(ClientKind::Host, Access::Auto)
        .await
        .unwrap();
    let (name_c, _) = c.host_add(&link.url_b, &code, None).await.unwrap();
    let addr_a = tunnel_addr(&dir.path().join("a"), &name_a);
    let addr_c = tunnel_addr(&home_c, &name_c);
    let client_id = register_client(&addr_a);
    let tester = connect_paired(&link.b.1, &link.url_b, ClientKind::Host, "tester").await;
    let refused = decide(&addr_c, &client_id, &tester.t3_pairing().await.unwrap());
    assert_eq!(status_of(&refused), 403, "a decision: {refused}");
    let code = approve(&addr_a, &client_id, &tester.t3_pairing().await.unwrap());
    let answer = http_at(
        &addr_c,
        &addr_a,
        "POST",
        "/oauth/mcp/token",
        FORM,
        &code_form(&code, &client_id, &addr_a),
    );
    assert_eq!(status_of(&answer), 403, "a token exchange: {answer}");
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_name_with_no_client_still_ends_the_sign_ins_made_under_it() {
    let (dir, _repo, _home) = fixture();
    let (link, name) = linked(dir.path()).await;
    let (_b, b) = &link.b;
    let home_b = dir.path().join("b");
    let addr_a = tunnel_addr(&dir.path().join("a"), &name);
    let clients = host_client_names(b).await;
    let [client] = clients.as_slice() else {
        panic!("one host pairs with b: {clients:?}")
    };
    let tester = connect_paired(b, &link.url_b, ClientKind::Host, "tester").await;
    let _ = sign_in(&addr_a, &tester.t3_pairing().await.unwrap());
    assert!(
        t3_sessions(&home_b)
            .iter()
            .any(|s| s["client"]["label"] == client.as_str())
    );
    rusqlite::Connection::open(home_b.join("meta-harness.sqlite"))
        .unwrap()
        .execute("DELETE FROM client WHERE name = ?1", [client])
        .unwrap();
    b.revoke_client(client).await.unwrap();
    assert!(
        t3_sessions(&home_b)
            .iter()
            .all(|s| s["client"]["label"] != client.as_str()),
        "the name's sessions end"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_host_ends_the_t3_sessions_its_link_made() {
    let (dir, _repo, _home) = fixture();
    let (link, name_a) = linked(dir.path()).await;
    let a_clients = host_client_names(&link.b.1).await;
    assert_eq!(a_clients.len(), 1, "{a_clients:?}");
    let a_client = a_clients[0].clone();
    let home_c = dir.path().join("c");
    let (_c, c) = start_service(&home_c).await;
    c.host_agents(true).await.unwrap();
    let code = link
        .b
        .1
        .invite(ClientKind::Host, Access::Auto)
        .await
        .unwrap();
    let (name_c, _) = c.host_add(&link.url_b, &code, None).await.unwrap();

    let tester = connect_paired(&link.b.1, &link.url_b, ClientKind::Host, "tester").await;
    let addr_a = tunnel_addr(&dir.path().join("a"), &name_a);
    let addr_c = tunnel_addr(&home_c, &name_c);
    let bearer = sign_in(&addr_a, &tester.t3_pairing().await.unwrap());
    assert_eq!(status_of(&mcp_answer(&addr_c, &addr_a, &bearer)), 200);

    link.b.1.revoke_client(&a_client).await.unwrap();
    assert!(
        t3_sessions(&dir.path().join("b"))
            .iter()
            .all(|session| session["client"]["label"] != a_client.as_str()),
        "A's T3 sessions end with its pairing"
    );
    let refused = mcp_answer(&addr_c, &addr_a, &bearer);
    assert_eq!(
        status_of(&refused),
        401,
        "T3 refuses the revoked session's token: {refused:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_reads_its_replies_gets_each_burst_answered_and_no_call_limit() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let (dir, _repo, _home) = fixture();
    let (service, _local) = start_service(&dir.path().join("meta")).await;
    let mut socket = open_client(&service).await;
    for id in [1u64, 2] {
        let bogus = serde_json::json!({"id": id, "bogus": true});
        socket
            .send(Message::Text(bogus.to_string().into()))
            .await
            .unwrap();
    }
    for id in [1u64, 2] {
        let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
            panic!("a reply is text");
        };
        let reply: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(reply["id"], id);
        assert!(reply.get("error").is_some(), "{reply}");
    }
    for id in 3..=302u64 {
        let call = serde_json::json!({"id": id, "call": {"host": "status"}});
        socket
            .send(Message::Text(call.to_string().into()))
            .await
            .unwrap();
        let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
            panic!("a reply is text");
        };
        let reply: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(reply["id"], id);
        assert!(reply.get("ok").is_some(), "{reply}");
    }
    service.stop().await;
}

async fn start_codex(home: &Path) -> (Service, remote::Host) {
    let mut config = service_config(home);
    for agent in &mut config.acp_agents {
        agent.id = "codex".to_owned();
    }
    start_with(config, home).await
}

fn codex_start(text: &str) -> StartThread {
    let mut thread = start(text, Workspace::Checkout, Permissions::FullAccess);
    thread.model.instance = "codex".to_owned();
    thread
}

fn shared_remote(dir: &Path) -> (PathBuf, String) {
    let origin = dir.join("origin.git");
    git(dir, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
    let seed = dir.join("seed");
    seed_repo(&seed);
    git(&seed, &["push", "-q", &origin.to_string_lossy(), "main"]);
    let transport = dir.join("fake-ssh");
    std::fs::write(
        &transport,
        "#!/bin/sh\nfor last; do :; done\nexec sh -c \"$last\"\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&transport, std::fs::Permissions::from_mode(0o755)).unwrap();
    (
        origin.clone(),
        format!("git@example.invalid:{}", origin.display()),
    )
}

fn pushed_subject(origin: &Path, branch: &str) -> String {
    let log = Command::new("git")
        .args(["log", "-1", "--format=%s", branch])
        .current_dir(origin)
        .output()
        .unwrap();
    String::from_utf8_lossy(&log.stdout).trim().to_owned()
}

fn clone_project(dir: &Path, origin: &Path, url: &str, name: &str) -> PathBuf {
    let repo = dir.join(name);
    git(dir, &["clone", "-q", &origin.to_string_lossy(), name]);
    git(&repo, &["remote", "set-url", "origin", url]);
    git(
        &repo,
        &[
            "config",
            "core.sshCommand",
            &dir.join("fake-ssh").to_string_lossy(),
        ],
    );
    git(&repo, &["config", "user.name", "Contract"]);
    git(&repo, &["config", "user.email", "contract@example.invalid"]);
    repo
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_moves_to_another_host_with_its_branch_and_its_source_is_sealed() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/move"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/move"]);

    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let listed = project_a.threads().await.unwrap();
    let summary = listed.iter().find(|t| t.id == thread.id()).unwrap();
    assert_eq!(summary.harness, Some(Harness::Codex));
    assert!(summary.harness_known, "the service reports the harness");

    thread.send("wait", false).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
    thread.send("append one", false).await.unwrap();
    thread.send("append two", false).await.unwrap();
    wait(&thread, |s| {
        s.turns
            .iter()
            .filter(|t| t.status == TurnStatus::Queued && !t.prompt.is_empty())
            .count()
            == 2
    })
    .await;
    let check = thread.move_check().await.unwrap();
    assert_eq!(check.harness, Some(Harness::Codex));
    assert_eq!(check.branch.as_deref(), Some("feature/move"));
    assert_eq!((check.uncommitted_files, check.running), (1, true));
    assert_eq!(check.remote_url.as_deref(), Some(url.as_str()));
    assert_eq!(check.blocked, None);

    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append the contract text".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(package.branch, "feature/move");
    assert_eq!(package.remote_url, url);
    assert_eq!(package.queued, ["append one", "append two"]);
    assert!(
        thread
            .turns()
            .await
            .unwrap()
            .iter()
            .all(|t| t.status != TurnStatus::Queued)
    );
    assert!(package.handoff.contains("feature/move"));
    assert!(
        package
            .transcript
            .iter()
            .any(|item| item.text == "append first")
    );
    assert!(!package.transcript_capped);
    assert_eq!(
        pushed_subject(&origin, "feature/move"),
        "Append the contract text"
    );
    let refused = thread.send("append late", false).await.unwrap_err();
    assert!(refused.contains("moving to another host"), "{refused}");

    let moved = project_b.move_in(package.clone()).await.unwrap();
    let file = moved
        .worktree()
        .await
        .unwrap()
        .expect("the branch has its own worktree")
        .join("contract.txt");
    wait(&moved, completed(3)).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\none\ntwo\n");
    assert!(
        moved
            .snapshot()
            .await
            .unwrap()
            .transcript
            .iter()
            .any(|item| item.text == package.handoff)
    );
    let record = moved
        .move_record()
        .await
        .unwrap()
        .expect("the new host keeps the move");
    assert_eq!(record.from_host, package.from_host);
    assert_eq!(
        record.to_host,
        host_b.environment_name().await.unwrap().to_lowercase()
    );
    assert!(
        record
            .transcript
            .iter()
            .any(|item| item.text == "append first")
    );

    thread
        .move_finish(
            &package.transfer,
            MoveFinish::Seal {
                to_host: record.to_host.clone(),
            },
        )
        .await
        .unwrap();
    assert!(
        project_a
            .threads()
            .await
            .unwrap()
            .iter()
            .all(|t| t.id != thread.id())
    );
    let sealed = thread.send("append late", false).await.unwrap_err();
    assert!(sealed.contains("moved to another host"), "{sealed}");
    assert!(thread.move_check().await.unwrap().blocked.is_some());
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_the_new_host_refuses_leaves_the_session_on_its_host() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();

    let mut unknown = package.clone();
    unknown.branch = "no-such-branch".to_owned();
    assert!(project_b.move_in(unknown.clone()).await.is_err());
    assert_eq!(
        host_b.move_status(&unknown.transfer).await.unwrap(),
        Arrival::Starting
    );
    assert_eq!(
        host_b.move_abort(&unknown.transfer).await.unwrap(),
        Arrival::Refused
    );
    assert!(project_b.threads().await.unwrap().is_empty());
    let mut same = package.clone();
    same.transfer = transfer();
    same.branch = "main".to_owned();
    std::fs::write(repo_b.join("README.md"), "changed here\n").unwrap();
    let refused = project_b.move_in(same).await.err().unwrap();
    assert!(refused.contains("uncommitted changes"), "{refused}");
    git(&repo_b, &["checkout", "-q", "--", "README.md"]);
    let other_url = format!("{url}-other");
    let other = clone_project(dir.path(), &origin, &other_url, "c");
    let project_c = host_b.project(&other).await.unwrap();
    let mut foreign = package.clone();
    foreign.transfer = transfer();
    let refused = project_c.move_in(foreign).await.err();
    assert_eq!(
        refused.as_deref(),
        Some(
            "This project's Git remote is not the session's remote, so the session cannot move here."
        )
    );
    let listed = host_b.projects().await.unwrap();
    assert!(listed.iter().any(|p| {
        p.root == other.to_string_lossy().into_owned()
            && p.remote_url.as_deref() == Some(other_url.as_str())
    }));

    thread
        .move_finish(&package.transfer, MoveFinish::Release)
        .await
        .unwrap();
    assert_eq!(thread.move_check().await.unwrap().blocked, None);
    thread.send("append again", false).await.unwrap();
    wait(&thread, completed(2)).await;
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_clones_a_remote_into_a_project_and_names_it() {
    let dir = tempdir::Dir::new();
    let (origin, _url) = shared_remote(dir.path());
    let (service, host) = start_codex(&dir.path().join("host")).await;
    let destination = dir.path().join("cloned");
    let cloned = host
        .clone_project(&origin.to_string_lossy(), &destination.to_string_lossy())
        .await
        .unwrap();
    assert_eq!(cloned.root, destination.to_string_lossy());
    assert!(destination.join("README.md").exists());
    let folder = dir.path().join("plain");
    std::fs::create_dir(&folder).unwrap();
    let added = host.add_project(&folder.to_string_lossy()).await.unwrap();
    assert_eq!(added.root, folder.to_string_lossy().into_owned());
    assert!(
        host.projects()
            .await
            .unwrap()
            .iter()
            .any(|p| p.root == cloned.root)
    );
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_preparing_move_is_kept_through_a_restart_until_its_finish() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let home_a = dir.path().join("host-a");
    let (service, host) = start_codex(&home_a).await;
    let project = host.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let id = thread.id().to_owned();
    let transfer = transfer();
    thread
        .move_out(
            &transfer,
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    service.stop().await;
    rusqlite::Connection::open(home_a.join("meta-harness.sqlite"))
        .unwrap()
        .execute("UPDATE moved_thread SET state = 'preparing'", [])
        .unwrap();

    let (service, host) = start_codex(&home_a).await;
    let thread = host
        .project(&repo_a)
        .await
        .unwrap()
        .open(&id)
        .await
        .unwrap();
    assert_eq!(
        thread
            .move_check()
            .await
            .unwrap()
            .moving
            .map(|m| m.transfer),
        Some(transfer.clone())
    );
    assert!(thread.send("append late", false).await.is_err());
    let resumed = thread
        .move_out(
            &transfer,
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .expect("a repeat resumes the preparing move");
    assert_eq!(resumed.transfer, transfer);
    thread
        .move_finish(&transfer, MoveFinish::Release)
        .await
        .unwrap();
    assert_eq!(thread.move_check().await.unwrap().blocked, None);
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finish_repeats_and_refuses_the_other_outcome_by_name() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let (service, host) = start_codex(&dir.path().join("host-a")).await;
    let project = host.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;

    let released = transfer();
    thread
        .move_out(
            &released,
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    thread
        .move_finish(&released, MoveFinish::Release)
        .await
        .unwrap();
    thread
        .move_finish(&released, MoveFinish::Release)
        .await
        .expect("a repeated release is answered again");
    let refused = thread
        .move_finish(
            &released,
            MoveFinish::Seal {
                to_host: "studio".to_owned(),
            },
        )
        .await
        .unwrap_err();
    assert!(refused.contains("released"), "{refused}");

    let sealed = transfer();
    let package = thread
        .move_out(
            &sealed,
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let seal = MoveFinish::Seal {
        to_host: "studio".to_owned(),
    };
    thread.move_finish(&sealed, seal.clone()).await.unwrap();
    thread
        .move_finish(&sealed, seal)
        .await
        .expect("a repeated seal is answered again");
    let refused = thread
        .move_finish(&sealed, MoveFinish::Release)
        .await
        .unwrap_err();
    assert!(refused.contains("sealed"), "{refused}");
    assert_eq!(package.transfer, sealed);
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_released_message_is_returned_exactly_once() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let (service, host) = start_codex(&dir.path().join("host-a")).await;
    let project = host.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    thread.send("wait", false).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
    thread.send("append one", false).await.unwrap();
    wait(&thread, |s| {
        s.turns
            .iter()
            .any(|t| t.status == TurnStatus::Queued && t.prompt == "append one")
    })
    .await;
    let transfer = transfer();
    thread
        .move_out(
            &transfer,
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    thread
        .move_finish(&transfer, MoveFinish::Release)
        .await
        .unwrap();
    thread
        .move_finish(&transfer, MoveFinish::Release)
        .await
        .unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| {
            t.prompt == "append one"
                && matches!(t.status, TurnStatus::Running | TurnStatus::Completed)
        })
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let runs = thread
        .turns()
        .await
        .unwrap()
        .iter()
        .filter(|t| {
            t.prompt == "append one"
                && matches!(t.status, TurnStatus::Running | TurnStatus::Completed)
        })
        .count();
    assert_eq!(runs, 1);
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_handed_move_waits_for_its_finish_through_a_restart() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let home_a = dir.path().join("host-a");
    let (service, host) = start_codex(&home_a).await;
    let project = host.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let id = thread.id().to_owned();
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    service.stop().await;

    let (service, host) = start_codex(&home_a).await;
    let thread = host
        .project(&repo_a)
        .await
        .unwrap()
        .open(&id)
        .await
        .unwrap();
    assert!(thread.move_check().await.unwrap().blocked.is_some());
    assert!(thread.send("append late", false).await.is_err());
    thread
        .move_finish(&package.transfer, MoveFinish::Release)
        .await
        .unwrap();
    assert_eq!(thread.move_check().await.unwrap().blocked, None);
    thread
        .send("append again", false)
        .await
        .expect("the session takes messages once its move is released");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_message_comes_back_to_its_session_when_its_move_is_released() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let (service, host) = start_codex(&dir.path().join("host-a")).await;
    let project = host.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    thread.send("wait", false).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
    thread.send("append one", false).await.unwrap();
    wait(&thread, |s| {
        s.turns
            .iter()
            .any(|t| t.status == TurnStatus::Queued && t.prompt == "append one")
    })
    .await;

    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(package.queued, ["append one"]);
    thread
        .move_finish(&package.transfer, MoveFinish::Release)
        .await
        .unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| {
            t.prompt == "append one"
                && matches!(t.status, TurnStatus::Running | TurnStatus::Completed)
        })
    })
    .await;
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_message_comes_back_when_its_preparing_move_is_released_after_a_restart() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let home_a = dir.path().join("host-a");
    let (service, host) = start_codex(&home_a).await;
    let project = host.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    thread.send("wait", false).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
    thread.send("append one", false).await.unwrap();
    wait(&thread, |s| {
        s.turns
            .iter()
            .any(|t| t.status == TurnStatus::Queued && t.prompt == "append one")
    })
    .await;
    let id = thread.id().to_owned();
    let transfer = transfer();
    thread
        .move_out(
            &transfer,
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    service.stop().await;
    rusqlite::Connection::open(home_a.join("meta-harness.sqlite"))
        .unwrap()
        .execute("UPDATE moved_thread SET state = 'preparing'", [])
        .unwrap();

    let (service, host) = start_codex(&home_a).await;
    let thread = host
        .project(&repo_a)
        .await
        .unwrap()
        .open(&id)
        .await
        .unwrap();
    thread
        .move_finish(&transfer, MoveFinish::Release)
        .await
        .unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| {
            t.prompt == "append one"
                && matches!(t.status, TurnStatus::Running | TurnStatus::Completed)
        })
    })
    .await;
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_moved_again_carries_the_transcript_of_its_earlier_host() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/again"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/again"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let moved = project_b.move_in(package.clone()).await.unwrap();
    wait(&moved, completed(1)).await;

    let again = moved
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append again".to_owned()),
        )
        .await
        .unwrap();
    assert!(
        again
            .transcript
            .iter()
            .any(|item| item.text == "append first"),
        "the earlier host's transcript is carried"
    );
    assert!(
        again
            .transcript
            .iter()
            .all(|item| item.text != package.handoff),
        "the handoff is not carried twice"
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_in_whose_record_cannot_be_kept_stays_retryable_until_it_is_withdrawn() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/unrecorded"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/unrecorded"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    rusqlite::Connection::open(home_b.join("meta-harness.sqlite"))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse BEFORE INSERT ON move_record \
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();

    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("refused"), "{refused}");
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Starting
    );
    assert_eq!(
        host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Refused
    );
    assert!(project_b.threads().await.unwrap().is_empty());
    let worktrees = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&repo_b)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&worktrees.stdout)
            .matches("worktree ")
            .count(),
        2,
        "the launch's worktree stays: a move never removes a worktree"
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_repeats_of_one_move_out_return_the_same_package() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    git(&repo_a, &["checkout", "-q", "-b", "feature/repeated"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/repeated"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    thread.send("append second", false).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Queued)
    })
    .await;
    let transfer = transfer();
    let (first, second) = tokio::join!(
        thread.move_out(
            &transfer,
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned())
        ),
        thread.move_out(
            &transfer,
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned())
        ),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.handoff, second.handoff);
    assert_eq!(first.queued, second.queued);
    service_a.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_that_tracks_another_is_not_attached_by_a_move() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/tracked"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/tracked"]);
    git(
        &repo_b,
        &["branch", "--track", "feature/tracked", "origin/main"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let refused = project_b.move_in(package).await.err().unwrap();
    assert!(refused.contains("tracks another branch"), "{refused}");
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_whose_arrival_cannot_be_recorded_keeps_its_worktree_and_refuses() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/late-record"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/late-record"],
    );
    git(&repo_b, &["fetch", "-q", "origin"]);
    git(
        &repo_b,
        &[
            "branch",
            "feature/late-record",
            "origin/feature/late-record",
        ],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse_row BEFORE INSERT ON arrival \
         WHEN NEW.state = 'starting' BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();

    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("refused"), "{refused}");
    raw.execute_batch("DROP TRIGGER refuse_row").unwrap();
    assert_eq!(
        host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Refused
    );
    let worktrees = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&repo_b)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&worktrees.stdout)
            .matches("worktree ")
            .count(),
        2,
        "the worktree the move made stays: a move never removes a worktree"
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_attached_branch_with_unpushed_commits_is_refused() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/unpushed"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/unpushed"]);
    git(&repo_b, &["fetch", "-q", "origin"]);
    git(
        &repo_b,
        &["branch", "feature/unpushed", "origin/feature/unpushed"],
    );
    git(&repo_b, &["checkout", "-q", "feature/unpushed"]);
    std::fs::write(repo_b.join("local.txt"), "local only\n").unwrap();
    git(&repo_b, &["add", "local.txt"]);
    git(&repo_b, &["commit", "-q", "-m", "local only"]);
    git(&repo_b, &["checkout", "-q", "main"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();

    let refused = project_b.move_in(package).await.err().unwrap();
    assert!(
        refused.contains("has commits that are not pushed"),
        "{refused}"
    );
    service_a.stop().await;
    service_b.stop().await;
}

async fn moving_with_queued(host_a: &remote::Host, repo_a: &Path, texts: &[&str]) -> MovePackage {
    let project_a = host_a.project(repo_a).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    thread.send("wait", false).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
    for text in texts {
        thread.send(text, false).await.unwrap();
    }
    let queued = texts.len();
    wait(&thread, move |s| {
        s.turns
            .iter()
            .filter(|t| t.status == TurnStatus::Queued)
            .count()
            == queued
    })
    .await;
    thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap()
}

fn carried_prompts(snapshot: &Snapshot) -> Vec<String> {
    let mut turns: Vec<_> = snapshot
        .turns
        .iter()
        .filter(|t| t.prompt.starts_with("carry "))
        .collect();
    turns.sort_by_key(|t| t.number);
    turns.iter().map(|t| t.prompt.clone()).collect()
}

fn carried_done(snapshot: &Snapshot) -> usize {
    snapshot
        .turns
        .iter()
        .filter(|t| t.prompt.starts_with("carry ") && t.status == TurnStatus::Completed)
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn carried_messages_reach_the_target_only_after_it_lands() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/carried"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/carried"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_b = host_b.project(&repo_b).await.unwrap();
    let package = moving_with_queued(&host_a, &repo_a, &["carry one"]).await;
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse_record BEFORE INSERT ON move_record \
         BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(project_b.move_in(package.clone()).await.is_err());
    raw.execute_batch("DROP TRIGGER refuse_record").unwrap();
    let listed = project_b.threads().await.unwrap();
    let held = project_b.open(&listed[0].id).await.unwrap();
    let snapshot = wait(&held, completed(1)).await;
    assert!(
        carried_prompts(&snapshot).is_empty(),
        "a carried message reached a session that never landed"
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn carried_messages_run_on_the_landed_session_once_each_and_in_order() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/ordered"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/ordered"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_b = host_b.project(&repo_b).await.unwrap();
    let package = moving_with_queued(&host_a, &repo_a, &["carry one", "carry two"]).await;
    let landed = project_b.move_in(package.clone()).await.unwrap();
    wait(&landed, |s| carried_done(s) == 2).await;
    let snapshot = wait(&landed, |s| carried_done(s) == 2).await;
    assert_eq!(carried_prompts(&snapshot), ["carry one", "carry two"]);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_delivery_resumes_on_move_status_and_a_repeated_move_in() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/resumed"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/resumed"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_b = host_b.project(&repo_b).await.unwrap();
    let package = moving_with_queued(&host_a, &repo_a, &["carry one", "carry two"]).await;
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse_progress BEFORE UPDATE OF delivered ON arrival \
         WHEN NEW.delivered = 1 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(project_b.move_in(package.clone()).await.is_err());
    raw.execute_batch("DROP TRIGGER refuse_progress").unwrap();
    assert!(matches!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed { .. }
    ));
    let listed = project_b.threads().await.unwrap();
    let landed = project_b.open(&listed[0].id).await.unwrap();
    let snapshot = wait(&landed, |s| carried_done(s) == 2).await;
    assert_eq!(carried_prompts(&snapshot), ["carry one", "carry two"]);
    let again = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(again.id(), landed.id());
    let snapshot = wait(&landed, |s| carried_done(s) == 2).await;
    assert_eq!(carried_prompts(&snapshot), ["carry one", "carry two"]);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_landed_move_with_an_undelivered_message_answers_starting_until_it_is_delivered() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/unfinished"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/unfinished"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_b = host_b.project(&repo_b).await.unwrap();
    let package = moving_with_queued(&host_a, &repo_a, &["carry one", "carry two"]).await;
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse_progress BEFORE UPDATE OF delivered ON arrival \
         WHEN NEW.delivered = 1 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(project_b.move_in(package.clone()).await.is_err());
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Starting
    );
    assert_eq!(
        host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Starting,
        "an abort on a landed transfer never refuses it"
    );
    raw.execute_batch("DROP TRIGGER refuse_progress").unwrap();
    assert!(matches!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed { .. }
    ));
    let listed = project_b.threads().await.unwrap();
    let landed = project_b.open(&listed[0].id).await.unwrap();
    let snapshot = wait(&landed, |s| carried_done(s) == 2).await;
    assert_eq!(carried_prompts(&snapshot), ["carry one", "carry two"]);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_move_in_is_refused_while_a_carried_message_is_undelivered() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/refused-early"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/refused-early"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_b = host_b.project(&repo_b).await.unwrap();
    let package = moving_with_queued(&host_a, &repo_a, &["carry one", "carry two"]).await;
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse_progress BEFORE UPDATE OF delivered ON arrival \
         WHEN NEW.delivered = 1 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(project_b.move_in(package.clone()).await.is_err());
    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(
        refused.contains("not all delivered yet"),
        "a repeat does not finish the move early: {refused}"
    );
    raw.execute_batch("DROP TRIGGER refuse_progress").unwrap();
    let landed = project_b.move_in(package.clone()).await.unwrap();
    let snapshot = wait(&landed, |s| carried_done(s) == 2).await;
    assert_eq!(carried_prompts(&snapshot), ["carry one", "carry two"]);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_release_during_a_move_out_waits_for_it_and_loses_no_message() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    git(&repo_a, &["checkout", "-q", "-b", "feature/released-late"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/released-late"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    thread.send("wait", false).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
    thread.send("keep one", false).await.unwrap();
    thread.send("keep two", false).await.unwrap();
    wait(&thread, |s| {
        s.turns
            .iter()
            .filter(|t| t.status == TurnStatus::Queued)
            .count()
            == 2
    })
    .await;
    let transfer = transfer();
    let (moved, released) = tokio::join!(
        thread.move_out(
            &transfer,
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned())
        ),
        async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            thread.move_finish(&transfer, MoveFinish::Release).await
        },
    );
    let package = moved.unwrap();
    released.unwrap();
    assert_eq!(package.queued, ["keep one", "keep two"]);
    wait(&thread, |s| {
        ["keep one", "keep two"]
            .iter()
            .all(|text| s.turns.iter().any(|t| t.prompt == *text))
    })
    .await;
    service_a.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_that_tracks_a_local_branch_is_not_attached_by_a_move() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/local-tracked"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/local-tracked"],
    );
    git(
        &repo_b,
        &["branch", "--track", "feature/local-tracked", "main"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let refused = project_b.move_in(package).await.err().unwrap();
    assert!(refused.contains("tracks another branch"), "{refused}");
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_moving_session_is_not_deleted_or_archived_but_can_be_stopped() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    git(&repo_a, &["checkout", "-q", "-b", "feature/held"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/held"]);
    let (service, host) = start_codex(&dir.path().join("host-a")).await;
    let project = host.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let id = thread.id().to_owned();
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let summary = project
        .threads()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == id)
        .unwrap();

    let deleted = project.delete(&summary, false).await.err().unwrap();
    assert!(deleted.contains("moving to another host"), "{deleted}");
    let archived = project.archive(&id, true).await.err().unwrap();
    assert!(archived.contains("moving to another host"), "{archived}");
    thread.stop().await.expect("stop stays allowed");

    thread
        .move_finish(&package.transfer, MoveFinish::Release)
        .await
        .unwrap();
    project
        .delete(&summary, false)
        .await
        .expect("a released session can be deleted");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_in_repeated_or_concurrent_lands_the_session_once() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/landed"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/landed"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();

    let (first, second) = tokio::join!(
        project_b.move_in(package.clone()),
        project_b.move_in(package.clone())
    );
    let landed = first.unwrap();
    assert_eq!(second.unwrap().id(), landed.id());
    assert_eq!(project_b.threads().await.unwrap().len(), 1);
    let again = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(again.id(), landed.id());
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed {
            thread: landed.id().to_owned()
        }
    );
    assert_eq!(
        host_b.move_status("unknown").await.unwrap(),
        Arrival::Unknown
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_preflight_says_where_a_branch_would_land_and_why_it_cannot() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_b = host_b.project(&repo_b).await.unwrap();
    let root = repo_b.to_string_lossy().into_owned();

    assert_eq!(
        host_b.preflight(&root, "feature/new").await.unwrap(),
        Landing::NewWorktree
    );
    assert_eq!(
        host_b.preflight(&root, "main").await.unwrap(),
        Landing::Continue { path: root.clone() }
    );
    project_b.git().create_branch("feature/side").await.unwrap();
    project_b.git().switch("main").await.unwrap();
    let Landing::Blocked { reason } = host_b.preflight(&root, "feature/side").await.unwrap() else {
        panic!("a branch without an upstream is not attached");
    };
    assert!(reason.contains("has no upstream branch"), "{reason}");
    git(&repo_b, &["push", "-q", "-u", "origin", "feature/side"]);
    let attached = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match host_b.preflight(&root, "feature/side").await.unwrap() {
                Landing::Attach => break,
                _ => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
    })
    .await;
    assert!(attached.is_ok(), "the pushed branch is attached");
    std::fs::write(repo_b.join("README.md"), "changed here\n").unwrap();
    let Landing::Blocked { reason } = host_b.preflight(&root, "main").await.unwrap() else {
        panic!("a dirty checkout blocks the move");
    };
    assert!(reason.contains("uncommitted changes"), "{reason}");
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_lands_in_the_clean_checkout_the_target_has_on_its_branch() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(package.branch, "main");

    std::fs::write(repo_b.join("README.md"), "changed here\n").unwrap();
    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("uncommitted changes"), "{refused}");
    assert!(project_b.threads().await.unwrap().is_empty());
    git(&repo_b, &["checkout", "-q", "--", "README.md"]);

    let landed = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(
        landed.worktree().await.unwrap().as_deref(),
        Some(repo_b.as_path()),
        "the session continues in the checkout"
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_onto_a_local_branch_that_is_not_checked_out_attaches_a_worktree() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/attach"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/attach"]);
    git(&repo_b, &["fetch", "-q", "origin"]);
    git(
        &repo_b,
        &["branch", "feature/attach", "origin/feature/attach"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();

    let landed = project_b.move_in(package.clone()).await.unwrap();
    let worktree = landed
        .worktree()
        .await
        .unwrap()
        .expect("the branch has its own worktree");
    assert_ne!(worktree, repo_b);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sealed_session_can_be_archived_and_deleted() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    git(&repo_a, &["checkout", "-q", "-b", "feature/sealed"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/sealed"]);
    let (service, host) = start_codex(&dir.path().join("host-a")).await;
    let project = host.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let id = thread.id().to_owned();
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let summary = project
        .threads()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == id)
        .unwrap();
    thread
        .move_finish(
            &package.transfer,
            MoveFinish::Seal {
                to_host: "studio".to_owned(),
            },
        )
        .await
        .unwrap();

    project
        .archive(&id, true)
        .await
        .expect("a sealed session can be archived");
    project
        .delete(&summary, false)
        .await
        .expect("a sealed session can be deleted");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_moved_back_to_the_host_it_left_continues_in_its_checkout() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/loop"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/loop"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let first = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let there = project_b.move_in(first.clone()).await.unwrap();
    wait(&there, completed(1)).await;
    thread
        .move_finish(
            &first.transfer,
            MoveFinish::Seal {
                to_host: host_b.environment_name().await.unwrap(),
            },
        )
        .await
        .unwrap();

    let back = there
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append again".to_owned()),
        )
        .await
        .unwrap();
    there
        .move_finish(
            &back.transfer,
            MoveFinish::Seal {
                to_host: host_a.environment_name().await.unwrap(),
            },
        )
        .await
        .unwrap();
    let home = project_a.move_in(back.clone()).await.unwrap();
    assert_eq!(
        home.worktree().await.unwrap().as_deref(),
        Some(repo_a.as_path())
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_whose_reply_was_lost_is_open_on_the_session_and_can_be_released() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let (service, host) = start_codex(&dir.path().join("host-a")).await;
    let project = host.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    thread.send("wait", false).await.unwrap();
    wait(&thread, |s| {
        s.turns.iter().any(|t| t.status == TurnStatus::Running)
    })
    .await;
    thread.send("append one", false).await.unwrap();
    wait(&thread, |s| {
        s.turns
            .iter()
            .any(|t| t.status == TurnStatus::Queued && t.prompt == "append one")
    })
    .await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();

    let open = thread
        .move_check()
        .await
        .unwrap()
        .moving
        .expect("the handed move is open on the session");
    assert_eq!(open.transfer, package.transfer);
    thread
        .move_finish(&open.transfer, MoveFinish::Release)
        .await
        .unwrap();
    let after = thread.move_check().await.unwrap();
    assert_eq!((after.moving, after.blocked), (None, None));
    wait(&thread, |s| {
        s.turns.iter().any(|t| {
            t.prompt == "append one"
                && matches!(t.status, TurnStatus::Running | TurnStatus::Completed)
        })
    })
    .await;
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_move_out_returns_the_stored_package_and_a_second_transfer_is_refused() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let (service, host) = start_codex(&dir.path().join("host-a")).await;
    let project = host.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let first_transfer = transfer();
    let first = thread
        .move_out(
            &first_transfer,
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let again = thread
        .move_out(
            &first_transfer,
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .expect("a repeat of the same transfer returns its package");
    assert_eq!(again, first);
    let second = transfer();
    let other = thread
        .move_out(
            &second,
            "host-b",
            "studio",
            "environment-target",
            true,
            None,
        )
        .await
        .unwrap_err();
    assert!(other.contains("already moving"), "{other}");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_starting_arrival_launches_its_chosen_thread_id_when_no_thread_exists_yet() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/chosen"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/chosen"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    rusqlite::Connection::open(home_b.join("meta-harness.sqlite"))
        .unwrap()
        .execute(
            "INSERT INTO arrival (transfer, state, root, thread, at_ms) \
             VALUES (?1, 'starting', ?2, 'thread-chosen-before-launch', 0)",
            rusqlite::params![package.transfer, repo_b.to_string_lossy()],
        )
        .unwrap();

    let landed = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(landed.id(), "thread-chosen-before-launch");
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed {
            thread: "thread-chosen-before-launch".to_owned()
        }
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_starting_arrival_adopts_the_thread_it_already_launched() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/adopted"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/adopted"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let landed = project_b.move_in(package.clone()).await.unwrap();
    rusqlite::Connection::open(home_b.join("meta-harness.sqlite"))
        .unwrap()
        .execute("UPDATE arrival SET state = 'starting'", [])
        .unwrap();

    let adopted = project_b.move_in(package.clone()).await.unwrap();
    assert_eq!(adopted.id(), landed.id());
    assert_eq!(project_b.threads().await.unwrap().len(), 1);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_abort_removes_the_thread_keeps_the_worktree_and_refuses_the_transfer() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/withdrawn"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/withdrawn"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let landed = project_b.move_in(package.clone()).await.unwrap();
    let worktree = landed.worktree().await.unwrap().expect("its own worktree");
    rusqlite::Connection::open(home_b.join("meta-harness.sqlite"))
        .unwrap()
        .execute("UPDATE arrival SET state = 'starting'", [])
        .unwrap();
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Starting
    );

    assert_eq!(
        host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Refused
    );
    assert_eq!(
        host_b.move_abort(&package.transfer).await.unwrap(),
        Arrival::Refused
    );
    assert!(project_b.threads().await.unwrap().is_empty());
    assert!(worktree.exists(), "a move never removes a worktree");
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Unknown
    );
    assert!(project_b.move_in(package.clone()).await.is_err());
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_abort_racing_a_landing_answers_what_landed() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/raced"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/raced"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();

    let (arrived, withdrawn) = tokio::join!(
        project_b.move_in(package.clone()),
        host_b.move_abort(&package.transfer)
    );
    match (arrived, withdrawn.unwrap()) {
        (Ok(landed), Arrival::Landed { thread }) => assert_eq!(landed.id(), thread),
        (Err(_), Arrival::Refused) => {}
        (arrived, withdrawn) => panic!(
            "landing {} and abort {withdrawn:?} disagree",
            arrived.is_ok()
        ),
    }
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_moving_session_stays_listed_with_its_open_move() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let (service, host) = start_codex(&dir.path().join("host-a")).await;
    let project = host.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let id = thread.id().to_owned();
    let transfer = transfer();
    thread
        .move_out(
            &transfer,
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();

    let summary = project
        .threads()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == id)
        .expect("a moving session stays listed");
    let moving = summary.moving.expect("the summary names the open move");
    assert_eq!(moving.transfer, transfer);
    assert_eq!(moving.phase, MovePhase::Handed);
    assert_eq!(moving.target_address, "host-b");
    assert_eq!(moving.target_name, "studio");
    assert_eq!(
        thread.move_check().await.unwrap().moving.map(|m| m.phase),
        Some(MovePhase::Handed)
    );
    thread
        .move_finish(&transfer, MoveFinish::Release)
        .await
        .unwrap();
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_is_refused_while_a_live_session_uses_the_checkout_it_would_continue_in() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let live = project_b.start(codex_start("work in b")).await.unwrap();
    wait(&live, completed(1)).await;

    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("checked out"), "{refused}");
    assert_eq!(project_b.threads().await.unwrap().len(), 1);
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_is_refused_onto_a_checkout_that_tracks_another_branch() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["push", "-q", "origin", "main:refs/heads/side"]);
    git(&repo_b, &["fetch", "-q", "origin"]);
    git(
        &repo_b,
        &["branch", "--set-upstream-to=origin/side", "main"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();

    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("tracks"), "{refused}");
    assert!(project_b.threads().await.unwrap().is_empty());
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_in_a_project_with_two_remotes_is_blocked_and_an_absent_branch_still_gets_a_worktree()
 {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(
        &repo_b,
        &["remote", "add", "second", &origin.to_string_lossy()],
    );
    git(&repo_b, &["fetch", "-q", "second"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let (service_b, host_b) = start_codex(&dir.path().join("host-b")).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let root = repo_b.to_string_lossy().into_owned();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();

    let Landing::Blocked { reason } = host_b.preflight(&root, "main").await.unwrap() else {
        panic!("a branch that exists in a project with two remotes is blocked");
    };
    assert!(reason.contains("more than one Git remote"), "{reason}");
    assert_eq!(
        host_b.preflight(&root, "feature/absent").await.unwrap(),
        Landing::NewWorktree
    );
    let refused = project_b.move_in(package.clone()).await.err().unwrap();
    assert!(refused.contains("more than one Git remote"), "{refused}");
    assert!(project_b.threads().await.unwrap().is_empty());
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_reports_a_stable_environment_id_and_a_move_keeps_its_target() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let environment = host_b.environment_id().await.unwrap();
    assert_eq!(environment.len(), 36, "{environment}");
    assert_eq!(environment.matches('-').count(), 4, "{environment}");
    service_b.stop().await;
    let (service_b, host_b) = start_codex(&home_b).await;
    assert_eq!(host_b.environment_id().await.unwrap(), environment);
    service_b.stop().await;

    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let project = host_a.project(&repo_a).await.unwrap();
    let thread = project.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            &environment,
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let moving = thread.move_check().await.unwrap().moving.unwrap();
    assert_eq!(moving.target_environment, environment);
    service_a.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_record_store_is_retried_on_the_same_thread_and_lands_listed() {
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/recorded"]);
    git(&repo_a, &["push", "-q", "-u", "origin", "feature/recorded"]);
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER refuse BEFORE INSERT ON move_record BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(project_b.move_in(package.clone()).await.is_err());
    raw.execute_batch("DROP TRIGGER refuse").unwrap();

    let landed = project_b.move_in(package.clone()).await.unwrap();
    let listed = project_b.threads().await.unwrap();
    assert!(listed.iter().any(|t| t.id == landed.id()));
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Landed {
            thread: landed.id().to_owned()
        }
    );
    service_a.stop().await;
    service_b.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_withdrawal_interrupted_after_its_delete_refuses_a_late_move_in_and_never_reports_landed()
{
    let dir = tempdir::Dir::new();
    let (origin, url) = shared_remote(dir.path());
    let repo_a = clone_project(dir.path(), &origin, &url, "a");
    let repo_b = clone_project(dir.path(), &origin, &url, "b");
    git(&repo_a, &["checkout", "-q", "-b", "feature/withdrawn-late"]);
    git(
        &repo_a,
        &["push", "-q", "-u", "origin", "feature/withdrawn-late"],
    );
    let (service_a, host_a) = start_codex(&dir.path().join("host-a")).await;
    let home_b = dir.path().join("host-b");
    let (service_b, host_b) = start_codex(&home_b).await;
    let project_a = host_a.project(&repo_a).await.unwrap();
    let project_b = host_b.project(&repo_b).await.unwrap();
    let thread = project_a.start(codex_start("append first")).await.unwrap();
    wait(&thread, completed(1)).await;
    let package = thread
        .move_out(
            &transfer(),
            "host-b",
            "studio",
            "environment-target",
            true,
            Some("Append".to_owned()),
        )
        .await
        .unwrap();
    let landed = project_b.move_in(package.clone()).await.unwrap();
    let summary = project_b
        .threads()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == landed.id())
        .unwrap();
    project_b.delete(&summary, false).await.unwrap();
    let raw = rusqlite::Connection::open(home_b.join("meta-harness.sqlite")).unwrap();
    raw.execute("UPDATE arrival SET state = 'landed'", [])
        .unwrap();
    assert_eq!(
        host_b.move_status(&package.transfer).await.unwrap(),
        Arrival::Unknown,
        "a deleted thread is never reported as landed"
    );
    raw.execute("UPDATE arrival SET state = 'starting'", [])
        .unwrap();
    let late = project_b.move_in(package.clone()).await;
    assert!(
        late.as_ref().map_or(true, |t| t.id() != landed.id()),
        "a deleted thread is not adopted"
    );
    service_a.stop().await;
    service_b.stop().await;
}
