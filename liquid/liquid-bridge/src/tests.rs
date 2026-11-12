mod database;
mod workspace;

use meta_harness_core::{
    AgentCommand, AgentEvent, ChangedLines, CommandError, ItemDetail, ItemId, ItemKind, ItemStatus,
    RequestId, SessionEvent, StoredSession, TurnId, Usage, UserInput,
};

use crate::api::events::{
    BridgeChangeStatus, BridgeEvent, BridgeEventKind, BridgeFileChange, BridgeItemKind,
    BridgeQuestionKind, BridgeQueue,
};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use crate::api::history::{
    BridgeChangesState, BridgeEntryKind, BridgeRecordKind, BridgeRevertConfirmation,
    BridgeRevertFile, BridgeRunningAgent, BridgeTurn, BridgeTurnChanges,
};
use crate::api::session::{
    AgentSession, BridgeCheckpointSession, BridgeModel, BridgeStoredSession, SessionConfig,
};
use crate::convert::describe;
use crate::stream::{Credit, Outbox, WINDOW, forward, gather, push};

/// The extension path the fake agents are started with.
const EXTENSION: &str = "/w/meta-harness/pi-extension/watershed.ts";

/// `text` as one word of a POSIX shell command, so spaces and quotes in a temporary directory
/// stay part of its paths.
fn shell_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Answers the check before a session starts or a message may start a run: Watershed's
/// extension is loaded.
fn gate(id: u32) -> String {
    format!(
        "IFS= read -r line\ncase \"$line\" in *'\"type\":\"get_commands\"'*) ;; *) exit 9 ;; esac\nprintf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_commands\",\"success\":true,\"data\":{{\"commands\":[{{\"name\":\"watershed-branch\",\"source\":\"extension\",\"sourceInfo\":{{\"path\":\"{EXTENSION}\"}}}}]}}}}'\n"
    )
}

/// Answers the check every session starts with, then runs `script`.
fn gated(script: &str) -> String {
    format!("{}{script}", gate(1))
}

/// Points the per-user lock folder at a test home, so no test writes into the real one. The test
/// runner gives each test process its own (`WATERSHED_TEST_USER_HOME`); without it, a folder per
/// process in the temporary folder. Each test runs in its own process under nextest.
fn isolate_user_home() {
    let home = std::env::var_os("WATERSHED_TEST_USER_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("watershed-bridge-test-home-{}", std::process::id()))
        });
    std::fs::create_dir_all(&home).expect("test user home");
    unsafe {
        std::env::set_var("HOME", home);
        std::env::remove_var("XDG_RUNTIME_DIR");
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

/// Starts a scripted stand-in for pi in `cwd`; `sh` takes pi's arguments as `$0` and `$@`.
fn launch(script: String, cwd: String) -> Result<AgentSession, String> {
    isolate_user_home();
    AgentSession::launch(AgentCommand {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        extension: EXTENSION.into(),
        session: None,
        cwd: cwd.into(),
    })
}

/// Answers the `get_state` that starts a first history read, for a session pi keeps in memory.
fn no_session_file(id: u32) -> String {
    format!(
        "IFS= read -r line\ncase \"$line\" in *'\"type\":\"get_state\"'*) ;; *) exit 9 ;; esac\nprintf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_state\",\"success\":true,\"data\":{{\"sessionId\":\"s\",\"isStreaming\":false}}}}'\n"
    )
}

/// Runs an async call as flutter_rust_bridge does: on a runtime of its own, apart from the
/// bridge's.
fn call<F: std::future::Future>(call: F) -> F::Output {
    isolate_user_home();
    tokio::runtime::Runtime::new()
        .expect("runtime starts")
        .block_on(call)
}

fn bridge(event: AgentEvent) -> BridgeEvent {
    SessionEvent::Agent(event).into()
}

#[test]
fn tool_items_keep_their_call_and_outcome() {
    let started = bridge(AgentEvent::ItemStarted {
        turn: TurnId(2),
        item: ItemId(7),
        kind: ItemKind::CommandExecution,
        detail: ItemDetail::Tool {
            name: "bash".to_owned(),
            target: "ls".to_owned(),
        },
    });
    assert_eq!(started.kind, BridgeEventKind::ItemStarted);
    let item = started.item.expect("item");
    assert_eq!(
        (item.id, item.kind),
        (7, Some(BridgeItemKind::CommandExecution))
    );
    assert_eq!(item.tool.as_deref(), Some("bash"));
    assert_eq!(item.target.as_deref(), Some("ls"));

    let completed = bridge(AgentEvent::ItemCompleted {
        turn: TurnId(2),
        item: ItemId(7),
        kind: ItemKind::CommandExecution,
        text: "a.txt".to_owned(),
        status: ItemStatus::Failed,
        duration_ms: Some(12),
        lines: None,
    });
    let item = completed.item.expect("item");
    assert!(item.failed);
    assert_eq!(item.duration_ms, Some(12));
    assert_eq!(completed.text.as_deref(), Some("a.txt"));

    let edited = bridge(AgentEvent::ItemCompleted {
        turn: TurnId(2),
        item: ItemId(8),
        kind: ItemKind::FileChange,
        text: String::new(),
        status: ItemStatus::Completed,
        duration_ms: None,
        lines: Some(ChangedLines {
            added: 9,
            removed: 2,
        }),
    })
    .item
    .expect("item");
    assert_eq!((edited.added, edited.removed), (Some(9), Some(2)));
}

#[test]
fn model_requests_carry_usage_and_how_they_ended() {
    let completed = bridge(AgentEvent::ModelRequestCompleted {
        turn: TurnId(1),
        usage: Usage {
            input: 6140,
            output: 820,
            cache_read: 1,
            cache_write: 2,
            reasoning: 214,
            cost_micro_usd: 23457,
        },
        stop_reason: "error".to_owned(),
        error: Some("529 overloaded".to_owned()),
    });
    let step = completed.step.expect("step");
    assert_eq!(
        (step.stop_reason.as_str(), step.error.as_deref()),
        ("error", Some("529 overloaded"))
    );
    assert_eq!(
        (step.usage.input, step.usage.output, step.usage.reasoning),
        (6140, 820, 214)
    );
    assert_eq!(step.usage.cost_micro_usd, 23457);
}

#[test]
fn a_changed_file_shows_its_name_with_hiding_characters_written_out() {
    let change = BridgeFileChange::from(meta_harness_core::FileChange {
        path: "src/a\u{202e}b.ts".to_owned(),
        status: meta_harness_core::ChangeStatus::Added,
        added: 1,
        removed: 0,
    });
    assert_eq!(
        (change.path.as_str(), change.shown.as_str()),
        ("src/a\u{202e}b.ts", "src/a<U+202E>b.ts"),
        "the path still names the file for its patch"
    );
    // A name stays on one line.
    let change = BridgeFileChange::from(meta_harness_core::FileChange {
        path: "notes.md\n\tsecret.sh".to_owned(),
        status: meta_harness_core::ChangeStatus::Added,
        added: 1,
        removed: 0,
    });
    assert_eq!(change.shown, "notes.md<U+000A><U+0009>secret.sh");
}

#[test]
fn questions_reach_dart_with_their_answers() {
    let asked = bridge(AgentEvent::UserInputRequested {
        request: RequestId("q1".to_owned()),
        input: UserInput::Editor {
            title: "Edit".to_owned(),
            prefill: "draft".to_owned(),
        },
    });
    let question = asked.request.expect("request").question.expect("question");
    assert_eq!(question.kind, BridgeQuestionKind::Editor);
    assert_eq!(
        (question.title.as_str(), question.detail.as_str()),
        ("Edit", "draft")
    );
}

#[test]
fn the_exit_ends_the_stream_and_failures_explain_themselves() {
    let exited: BridgeEvent = SessionEvent::Exited {
        success: false,
        error_output: "Error: no models".to_owned(),
    }
    .into();
    assert_eq!(
        (exited.kind, exited.success, exited.text.as_deref()),
        (
            BridgeEventKind::Exited,
            Some(false),
            Some("Error: no models")
        )
    );
    let failed: BridgeEvent = SessionEvent::Failed("broken".to_owned()).into();
    assert_eq!(
        (failed.kind, failed.text.as_deref()),
        (BridgeEventKind::Failed, Some("broken"))
    );
    assert_eq!(describe(CommandError::Closed), "the agent has exited");
    assert_eq!(
        describe(CommandError::Rejected("busy".to_owned())),
        "the agent rejected the command: busy"
    );
    assert_eq!(
        describe(CommandError::TooLarge),
        "the agent's answer exceeded 16 MiB and was skipped"
    );
}

#[test]
fn drives_an_agent_through_the_bridge() {
    let script = gate(2)
        + r#"IFS= read -r line
printf '%s\n' '{"id":"3","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}'
printf '%s\n' '{"type":"agent_start"}' '{"type":"agent_settled","aborted":false}'
IFS= read -r line || exit 0"#;
    let session =
        launch(gated(&script), std::env::temp_dir().display().to_string()).expect("agent starts");
    let (sender, receiver) = mpsc::channel();
    session
        .forward(move |event| sender.send(event).is_ok())
        .expect("first subscriber");
    assert_eq!(
        session.forward(|_| true),
        Err("the session's events already have a subscriber".to_owned())
    );
    assert_eq!(session.send("Say hi".to_owned(), false), Ok(()));
    let next = || {
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("event arrives")
            .kind
    };
    assert_eq!(next(), BridgeEventKind::TurnStarted);
    assert_eq!(next(), BridgeEventKind::TurnCompleted);
    assert!(call(session.shutdown()));
    assert_eq!(next(), BridgeEventKind::Exited);
    assert_eq!(
        session.send("again".to_owned(), false),
        Err("the session was shut down".to_owned())
    );
}

#[test]
fn answers_a_question_the_agent_asks_before_taking_a_message() {
    // pi accepts a message only after its command handlers ran, and they may ask first.
    let script = gate(2)
        + r#"IFS= read -r line
printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"input","title":"Name","placeholder":""}'
IFS= read -r line
case "$line" in *'"id":"q1"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"3","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}'
IFS= read -r line || exit 0"#;
    let session = Arc::new(
        launch(gated(&script), std::env::temp_dir().display().to_string()).expect("agent starts"),
    );
    let (sender, receiver) = mpsc::channel();
    session
        .forward(move |event| sender.send(event).is_ok())
        .expect("first subscriber");
    let sending = {
        let session = Arc::clone(&session);
        std::thread::spawn(move || session.send("/ask".to_owned(), false))
    };
    let asked = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("question arrives");
    assert_eq!(asked.kind, BridgeEventKind::UserInputRequested);
    let (answered, answer) = mpsc::channel();
    {
        let session = Arc::clone(&session);
        std::thread::spawn(move || {
            answered.send(call(session.answer_text("q1".to_owned(), "Ada".to_owned())))
        });
    }
    assert_eq!(answer.recv_timeout(Duration::from_secs(10)), Ok(Ok(())));
    assert_eq!(sending.join().expect("send returns"), Ok(()));
    assert!(call(session.shutdown()));
}

#[test]
fn stopping_dismisses_the_question_the_agent_waits_for() {
    // pi acknowledges an abort once the agent is idle, which needs its open dialogs answered.
    let script = r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"input","title":"Name","placeholder":""}'
IFS= read -r line
case "$line" in *'"type":"abort"'*) ;; *) exit 9 ;; esac
IFS= read -r line
case "$line" in *'"id":"q1"'*) ;; *) exit 9 ;; esac
case "$line" in *'"cancelled":true'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"2","type":"response","command":"abort","success":true}'
IFS= read -r line || exit 0"#;
    let session = Arc::new(
        launch(gated(script), std::env::temp_dir().display().to_string()).expect("agent starts"),
    );
    let (sender, receiver) = mpsc::channel();
    session
        .forward(move |event| sender.send(event).is_ok())
        .expect("first subscriber");
    let next = || {
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("event arrives")
    };
    assert_eq!(next().kind, BridgeEventKind::UserInputRequested);
    let (stopped, stop) = mpsc::channel();
    {
        let session = Arc::clone(&session);
        std::thread::spawn(move || stopped.send(call(session.abort())));
    }
    assert_eq!(stop.recv_timeout(Duration::from_secs(10)), Ok(Ok(())));
    let resolved = next();
    assert_eq!(
        (resolved.kind, resolved.request.map(|request| request.id)),
        (BridgeEventKind::UserInputResolved, Some("q1".to_owned()))
    );
    assert!(call(session.shutdown()));
}

#[test]
fn answers_and_stop_never_wait_for_a_pending_query() {
    // pi never answers the `get_state`, yet the question is answered and the turn stopped.
    let script = r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"confirm","title":"Run?","message":""}'
IFS= read -r line
case "$line" in *'"type":"get_state"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"type":"extension_ui_request","id":"s1","method":"setStatus","statusKey":"k","statusText":"pending"}'
IFS= read -r line
case "$line" in *'"id":"q1"'*) ;; *) exit 9 ;; esac
case "$line" in *'"confirmed":true'*) ;; *) exit 9 ;; esac
IFS= read -r line
case "$line" in *'"type":"abort"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"3","type":"response","command":"abort","success":true}'
IFS= read -r line || exit 0"#;
    let session = Arc::new(
        launch(gated(script), std::env::temp_dir().display().to_string()).expect("agent starts"),
    );
    let (sender, receiver) = mpsc::channel();
    session
        .forward(move |event| sender.send(event).is_ok())
        .expect("first subscriber");
    let mut events = std::iter::from_fn(|| receiver.recv_timeout(Duration::from_secs(10)).ok());
    assert!(events.any(|event| event.kind == BridgeEventKind::UserInputRequested));
    let query = {
        let session = Arc::clone(&session);
        std::thread::spawn(move || session.title())
    };
    assert!(events.any(|event| event.text.as_deref() == Some("pending")));
    let (done, finished) = mpsc::channel();
    {
        let session = Arc::clone(&session);
        std::thread::spawn(move || {
            let _ = done.send(call(session.answer_confirm("q1".to_owned(), true)));
            let _ = done.send(call(session.abort()));
        });
    }
    assert_eq!(finished.recv_timeout(Duration::from_secs(10)), Ok(Ok(())));
    assert_eq!(finished.recv_timeout(Duration::from_secs(10)), Ok(Ok(())));
    call(session.shutdown());
    assert!(query.join().expect("the query returns").is_err());
}

#[test]
fn starts_and_resumes_sessions_and_picks_models() {
    let project = std::env::temp_dir();
    let dir = project.join(format!("watershed-bridge-stored-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("session directory");
    let older = dir.join("older.jsonl");
    std::fs::write(
        &older,
        format!(
            "{{\"type\":\"session\",\"version\":3,\"id\":\"o1\",\"cwd\":\"{}\"}}\n{{\"type\":\"message\",\"id\":\"u1\",\"parentId\":null,\"message\":{{\"role\":\"user\",\"content\":\"Fix login\"}}}}\n",
            project.display()
        ),
    )
    .expect("stored session");
    std::fs::File::options()
        .write(true)
        .open(&older)
        .and_then(|file| file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_000)))
        .expect("file time");
    let current = dir.join("current.jsonl");
    let respond = |id: u32, command: &str, data: &str| {
        let answer = format!(
            r#"{{"id":"{id}","type":"response","command":"{command}","success":true,"data":{data}}}"#
        );
        format!(
            "IFS= read -r line\ncase \"$line\" in *'\"type\":\"{command}\"'*) ;; *) exit 9 ;; esac\nprintf '%s\\n' {}\n",
            shell_word(&answer)
        )
    };
    let state = format!(
        r#"{{"sessionId":"s","isStreaming":false,"sessionFile":"{}","model":{{"provider":"faux","id":"faux-1"}}}}"#,
        current.display()
    );
    let script = [
        respond(2, "get_state", &state),
        respond(3, "new_session", r#"{"cancelled":false}"#),
        respond(4, "switch_session", r#"{"cancelled":false}"#),
        respond(
            5,
            "get_available_models",
            r#"{"models":[{"provider":"faux","id":"faux-1","name":"Faux One","contextWindow":200000}]}"#,
        ),
        respond(6, "get_state", &state),
        respond(7, "set_model", "{}"),
        "IFS= read -r line || exit 0".to_owned(),
    ]
    .concat();
    let session = launch(gated(&script), project.display().to_string()).expect("agent starts");
    let recent = session.recent_sessions(5);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        recent,
        Ok(vec![BridgeStoredSession {
            path: older.display().to_string(),
            title: "Fix login".to_owned(),
            modified_ms: 1_000_000,
        }])
    );
    assert_eq!(session.new_session(), Ok(true));
    assert_eq!(
        session.switch_session(older.display().to_string()),
        Ok(true)
    );
    assert_eq!(
        session.models(),
        Ok(vec![BridgeModel {
            provider: "faux".to_owned(),
            id: "faux-1".to_owned(),
            name: "Faux One".to_owned(),
            context_window: Some(200_000),
        }])
    );
    assert_eq!(session.current_model(), Ok(Some("faux/faux-1".to_owned())));
    assert_eq!(
        session.set_model("faux".to_owned(), "faux-1".to_owned()),
        Ok(())
    );
    assert!(call(session.shutdown()));
}

#[cfg(unix)]
#[test]
fn a_session_path_that_is_not_utf8_is_not_listed() {
    use std::os::unix::ffi::OsStrExt;
    let stored = StoredSession {
        path: std::ffi::OsStr::from_bytes(b"/s/session-\xff.jsonl").into(),
        title: "Fix login".to_owned(),
        modified_ms: 1_000_000,
    };
    assert_eq!(BridgeStoredSession::try_from(stored).ok(), None);
}

#[test]
fn answers_a_question_the_agent_asks_while_selecting_a_model() {
    // pi runs extensions' model selection handlers before it answers, and they may ask first.
    let script = r#"IFS= read -r line
case "$line" in *'"type":"set_model"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"confirm","title":"Switch?","message":"Costs more"}'
IFS= read -r line
case "$line" in *'"id":"q1"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"2","type":"response","command":"set_model","success":true,"data":{}}'
IFS= read -r line || exit 0"#;
    let session = Arc::new(
        launch(gated(script), std::env::temp_dir().display().to_string()).expect("agent starts"),
    );
    let (sender, receiver) = mpsc::channel();
    session
        .forward(move |event| sender.send(event).is_ok())
        .expect("first subscriber");
    let selecting = {
        let session = Arc::clone(&session);
        std::thread::spawn(move || session.set_model("faux".to_owned(), "faux-2".to_owned()))
    };
    let asked = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("question arrives");
    assert_eq!(asked.kind, BridgeEventKind::UserInputRequested);
    let (answered, answer) = mpsc::channel();
    {
        let session = Arc::clone(&session);
        std::thread::spawn(move || {
            answered.send(call(session.answer_confirm("q1".to_owned(), true)))
        });
    }
    assert_eq!(answer.recv_timeout(Duration::from_secs(10)), Ok(Ok(())));
    assert_eq!(selecting.join().expect("selection returns"), Ok(()));
    assert!(call(session.shutdown()));
}

#[test]
fn shuts_down_while_a_query_waits_for_a_hung_agent() {
    // The agent reads the query and never answers or reads again.
    let script = "IFS= read -r line\ntrap '' TERM\nwhile :; do sleep 1; done";
    let session = Arc::new(
        launch(gated(script), std::env::temp_dir().display().to_string()).expect("agent starts"),
    );
    let querying = {
        let session = Arc::clone(&session);
        std::thread::spawn(move || session.models())
    };
    std::thread::sleep(Duration::from_millis(200));
    let (stopped, stop) = mpsc::channel();
    {
        let session = Arc::clone(&session);
        std::thread::spawn(move || stopped.send(call(session.shutdown())));
    }
    assert_eq!(
        stop.recv_timeout(Duration::from_secs(20)),
        Ok(false),
        "killed after the grace period"
    );
    assert!(querying.join().expect("query returns").is_err());
}

#[test]
fn gathers_streaming_text_until_its_window_passes() {
    let delta = |text: &str| {
        SessionEvent::Agent(AgentEvent::ContentDelta {
            item: ItemId(1),
            delta: text.to_owned(),
        })
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime starts");
    runtime.block_on(async {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(16);
        let (pass, window) = tokio::sync::oneshot::channel::<()>();
        // Deltas arrive one by one while the window is open; one more comes after it passed.
        tokio::spawn(async move {
            for index in 1..10 {
                sender.send(delta(&index.to_string())).await.expect("sent");
                tokio::task::yield_now().await;
            }
            pass.send(()).expect("window passes");
            tokio::task::yield_now().await;
            sender.send(delta("late")).await.expect("sent");
        });
        let mut outbox = Outbox::default();
        outbox.admit(BridgeEvent::from(delta("0")));
        gather(&mut outbox, &mut receiver, async {
            let _ = window.await;
        })
        .await;
        let text: Vec<_> = outbox
            .pending
            .iter()
            .filter_map(|event| event.text.as_deref())
            .collect();
        assert_eq!(
            text.concat(),
            "0123456789",
            "everything until the window passed"
        );
        assert_eq!(
            outbox.pending.len(),
            1,
            "deltas merge as they arrive, so the batch stays small"
        );
    });

    // Anything but streaming text ends the gathering at once, without waiting for the window.
    runtime.block_on(async {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(16);
        sender
            .send(SessionEvent::Agent(AgentEvent::TurnCompleted {
                turn: TurnId(1),
            }))
            .await
            .expect("sent");
        let mut outbox = Outbox::default();
        outbox.admit(BridgeEvent::from(delta("0")));
        gather(&mut outbox, &mut receiver, std::future::pending()).await;
        assert_eq!(outbox.pending.len(), 2);
    });
}

#[test]
fn forwards_streaming_text_once_its_window_passed() {
    let delta = |text: &str| {
        SessionEvent::Agent(AgentEvent::ContentDelta {
            item: ItemId(1),
            delta: text.to_owned(),
        })
    };
    let ms = Duration::from_millis;
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime starts")
        .block_on(async {
            let (events, receiver) = tokio::sync::mpsc::channel(16);
            let (delivered, mut deliveries) = tokio::sync::mpsc::unbounded_channel();
            let start = tokio::time::Instant::now();
            tokio::spawn(forward(
                Vec::new(),
                receiver,
                Arc::default(),
                move |event| {
                    delivered
                        .send((start.elapsed(), event.kind, event.text))
                        .is_ok()
                },
            ));
            let streamed =
                |at, text: &str| Some((at, BridgeEventKind::ContentDelta, Some(text.to_owned())));
            events.send(delta("Hel")).await.expect("sent");
            tokio::time::sleep(ms(30)).await;
            events.send(delta("lo")).await.expect("sent");
            assert_eq!(deliveries.recv().await, streamed(ms(50), "Hello"));
            // Anything else ends the window at once, with the text before it.
            tokio::time::sleep(ms(10)).await;
            events.send(delta("!")).await.expect("sent");
            let completed = AgentEvent::TurnCompleted { turn: TurnId(1) };
            events
                .send(SessionEvent::Agent(completed))
                .await
                .expect("sent");
            assert_eq!(deliveries.recv().await, streamed(ms(60), "!"));
            assert_eq!(
                deliveries.recv().await,
                Some((ms(60), BridgeEventKind::TurnCompleted, None))
            );
        });
}

#[test]
fn merges_consecutive_deltas_of_one_item() {
    let delta = |item: u64, text: &str| {
        bridge(AgentEvent::ContentDelta {
            item: ItemId(item),
            delta: text.to_owned(),
        })
    };
    let mut merged = Vec::new();
    for event in [
        delta(1, "He"),
        delta(1, "llo"),
        delta(2, "Hi"),
        bridge(AgentEvent::TurnCompleted { turn: TurnId(1) }),
        delta(2, "!"),
    ] {
        push(&mut merged, event);
    }
    let summary: Vec<_> = merged
        .iter()
        .map(|event| {
            (
                event.kind,
                event.item.as_ref().map(|item| item.id),
                event.text.clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (
                BridgeEventKind::ContentDelta,
                Some(1),
                Some("Hello".to_owned())
            ),
            (
                BridgeEventKind::ContentDelta,
                Some(2),
                Some("Hi".to_owned())
            ),
            (BridgeEventKind::TurnCompleted, None, None),
            (BridgeEventKind::ContentDelta, Some(2), Some("!".to_owned())),
        ]
    );
}

#[test]
fn keeps_only_the_latest_partial_output_of_adjacent_updates() {
    let output = |item: u64, text: &str| {
        bridge(AgentEvent::ItemUpdated {
            item: ItemId(item),
            output: text.to_owned(),
        })
    };
    let mut merged = Vec::new();
    for event in [
        output(1, "a"),
        output(1, "ab"),
        output(2, "x"),
        output(1, "abc"),
        bridge(AgentEvent::TurnCompleted { turn: TurnId(1) }),
        output(1, "b"),
    ] {
        push(&mut merged, event);
    }
    let summary: Vec<_> = merged
        .iter()
        .map(|event| {
            (
                event.kind,
                event.item.as_ref().map(|item| item.id),
                event.text.clone(),
            )
        })
        .collect();
    // Each update carries the whole output so far, so an adjacent one replaces the last; an
    // update after another item or event keeps its place in the order.
    assert_eq!(
        summary,
        [
            (BridgeEventKind::ItemUpdated, Some(1), Some("ab".to_owned())),
            (BridgeEventKind::ItemUpdated, Some(2), Some("x".to_owned())),
            (
                BridgeEventKind::ItemUpdated,
                Some(1),
                Some("abc".to_owned())
            ),
            (BridgeEventKind::TurnCompleted, None, None),
            (BridgeEventKind::ItemUpdated, Some(1), Some("b".to_owned())),
        ]
    );
}

#[test]
fn forwards_partial_output_once_its_window_passed() {
    let output = |text: &str| {
        SessionEvent::Agent(AgentEvent::ItemUpdated {
            item: ItemId(1),
            output: text.to_owned(),
        })
    };
    let ms = Duration::from_millis;
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime starts")
        .block_on(async {
            let (events, receiver) = tokio::sync::mpsc::channel(16);
            let (delivered, mut deliveries) = tokio::sync::mpsc::unbounded_channel();
            let start = tokio::time::Instant::now();
            tokio::spawn(forward(
                Vec::new(),
                receiver,
                Arc::default(),
                move |event| {
                    delivered
                        .send((start.elapsed(), event.kind, event.text))
                        .is_ok()
                },
            ));
            // Two updates within one window reach Dart as the latest output only.
            events.send(output("a")).await.expect("sent");
            tokio::time::sleep(ms(30)).await;
            events.send(output("ab")).await.expect("sent");
            assert_eq!(
                deliveries.recv().await,
                Some((ms(50), BridgeEventKind::ItemUpdated, Some("ab".to_owned())))
            );
            // Anything else ends the window at once, after the output before it.
            tokio::time::sleep(ms(10)).await;
            events.send(output("b")).await.expect("sent");
            events
                .send(SessionEvent::Agent(AgentEvent::TurnCompleted {
                    turn: TurnId(1),
                }))
                .await
                .expect("sent");
            assert_eq!(
                deliveries.recv().await,
                Some((ms(60), BridgeEventKind::ItemUpdated, Some("b".to_owned())))
            );
            assert_eq!(
                deliveries.recv().await,
                Some((ms(60), BridgeEventKind::TurnCompleted, None))
            );
        });
}

/// A session event that starts item `item`, which no later event merges with.
fn started(item: u64) -> SessionEvent {
    SessionEvent::Agent(AgentEvent::ItemStarted {
        turn: TurnId(1),
        item: ItemId(item),
        kind: ItemKind::CommandExecution,
        detail: ItemDetail::None,
    })
}

/// A session event that finishes item `item` with `text`.
fn finished(item: u64, text: &str) -> SessionEvent {
    SessionEvent::Agent(AgentEvent::ItemCompleted {
        turn: TurnId(1),
        item: ItemId(item),
        kind: ItemKind::CommandExecution,
        text: text.to_owned(),
        status: ItemStatus::Completed,
        duration_ms: None,
        lines: None,
    })
}

/// A session event that appends `delta` to item `item`.
fn delta(item: u64, delta: &str) -> SessionEvent {
    SessionEvent::Agent(AgentEvent::ContentDelta {
        item: ItemId(item),
        delta: delta.to_owned(),
    })
}

/// Starts forwarding a session's events under `credit`; the events are sent in, and what the
/// sink receives comes out.
fn start_forwarding(
    credit: Arc<Credit>,
) -> (
    tokio::sync::mpsc::Sender<SessionEvent>,
    tokio::sync::mpsc::UnboundedReceiver<BridgeEvent>,
) {
    let (events, receiver) = tokio::sync::mpsc::channel(4096);
    let (delivered, deliveries) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(forward(Vec::new(), receiver, credit, move |event| {
        delivered.send(event).is_ok()
    }));
    (events, deliveries)
}

/// What the sink received since the last call, in order.
fn taken(deliveries: &mut tokio::sync::mpsc::UnboundedReceiver<BridgeEvent>) -> Vec<BridgeEvent> {
    std::iter::from_fn(|| deliveries.try_recv().ok()).collect()
}

/// Lets the forwarder run until it waits for an event or for Liquid, past any gathering window.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

fn paused_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime starts")
}

#[test]
fn holds_events_beyond_the_window_until_liquid_confirms() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let (events, mut deliveries) = start_forwarding(credit.clone());
        for item in 1..=WINDOW as u64 + 44 {
            events.send(started(item)).await.expect("sent");
        }
        settle().await;
        assert_eq!(
            taken(&mut deliveries).len(),
            WINDOW,
            "the sink never holds more than the window unconfirmed"
        );
        credit.confirm(WINDOW as u64);
        settle().await;
        assert_eq!(
            taken(&mut deliveries).len(),
            44,
            "a confirmation reopens the window for what waits"
        );
    });
}

#[test]
fn falling_behind_keeps_finished_items_and_sends_one_fell_behind_event() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let (events, mut deliveries) = start_forwarding(credit.clone());
        for item in 1..=WINDOW as u64 {
            events.send(started(item)).await.expect("sent");
        }
        // Liquid applies nothing: finished items queue, then more partial output than the bound
        // allows, and events after that overflow come behind it.
        for item in 1000..1100 {
            events.send(finished(item, "done")).await.expect("sent");
        }
        for item in 3000..3200 {
            events.send(delta(item, "partial")).await.expect("sent");
        }
        for item in 5000..5010 {
            events.send(started(item)).await.expect("sent");
        }
        settle().await;
        assert_eq!(taken(&mut deliveries).len(), WINDOW);
        credit.confirm(WINDOW as u64);
        settle().await;
        let sent: Vec<_> = taken(&mut deliveries)
            .iter()
            .map(|event| (event.kind, event.item.as_ref().map(|item| item.id)))
            .collect();
        let finished_items =
            (1000i64..1100).map(|item| (BridgeEventKind::ItemCompleted, Some(item)));
        // The partial output queued before the overflow is gone; what came after the fell-behind
        // event is live and stays.
        let live_after = (3157i64..3200).map(|item| (BridgeEventKind::ContentDelta, Some(item)));
        let started_after = (5000i64..5010).map(|item| (BridgeEventKind::ItemStarted, Some(item)));
        let expected: Vec<_> = finished_items
            .chain([(BridgeEventKind::FellBehind, None)])
            .chain(live_after)
            .chain(started_after)
            .collect();
        assert_eq!(
            sent, expected,
            "the finished items stay in order, one fell-behind event follows the partial output that went, and what comes after is live"
        );
    });
}

#[test]
fn events_other_than_partial_output_wait_for_liquid_and_are_never_dropped() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let (events, mut deliveries) = start_forwarding(credit.clone());
        for item in 1..=1000 {
            events.send(started(item)).await.expect("sent");
        }
        settle().await;
        let mut delivered = taken(&mut deliveries);
        for applied in [256u64, 512, 768] {
            credit.confirm(applied);
            settle().await;
            delivered.extend(taken(&mut deliveries));
        }
        let items: Vec<_> = delivered
            .iter()
            .map(|event| (event.kind, event.item.as_ref().map(|item| item.id)))
            .collect();
        let expected: Vec<_> = (1..=1000i64)
            .map(|item| (BridgeEventKind::ItemStarted, Some(item)))
            .collect();
        assert_eq!(
            items, expected,
            "a queue past its bound waits for confirmations, in order, and drops nothing"
        );
    });
}

#[test]
fn a_second_overflow_while_the_fell_behind_event_waits_adds_no_second_one() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let (events, mut deliveries) = start_forwarding(credit.clone());
        for item in 1..=WINDOW as u64 {
            events.send(started(item)).await.expect("sent");
        }
        // Two overflows before Liquid confirms anything.
        for item in 3000..3700 {
            events.send(delta(item, "partial")).await.expect("sent");
        }
        settle().await;
        let mut delivered = taken(&mut deliveries);
        for applied in [512u64, 768, 1024, 1280] {
            credit.confirm(applied);
            settle().await;
            delivered.extend(taken(&mut deliveries));
        }
        let fell_behind = delivered
            .iter()
            .filter(|event| event.kind == BridgeEventKind::FellBehind)
            .count();
        assert_eq!(fell_behind, 1, "one fell-behind event per stall");
    });
}

#[test]
fn merges_text_and_partial_output_while_the_window_is_full() {
    paused_runtime().block_on(async {
        let credit = Arc::new(Credit::default());
        let (events, mut deliveries) = start_forwarding(credit.clone());
        for item in 1..=WINDOW as u64 {
            events.send(started(item)).await.expect("sent");
        }
        settle().await;
        assert_eq!(taken(&mut deliveries).len(), WINDOW);
        events.send(delta(1, "He")).await.expect("sent");
        events.send(delta(1, "llo")).await.expect("sent");
        events
            .send(SessionEvent::Agent(AgentEvent::ItemUpdated {
                item: ItemId(2),
                output: "a".to_owned(),
            }))
            .await
            .expect("sent");
        events
            .send(SessionEvent::Agent(AgentEvent::ItemUpdated {
                item: ItemId(2),
                output: "ab".to_owned(),
            }))
            .await
            .expect("sent");
        events.send(delta(1, "!")).await.expect("sent");
        settle().await;
        assert!(taken(&mut deliveries).is_empty(), "the window is full");
        credit.confirm(WINDOW as u64);
        settle().await;
        let texts: Vec<_> = taken(&mut deliveries)
            .into_iter()
            .map(|event| (event.kind, event.text))
            .collect();
        assert_eq!(
            texts,
            [
                (BridgeEventKind::ContentDelta, Some("Hello".to_owned())),
                (BridgeEventKind::ItemUpdated, Some("ab".to_owned())),
                (BridgeEventKind::ContentDelta, Some("!".to_owned())),
            ],
            "deltas concatenate and the latest partial output wins, in order"
        );
    });
}

/// A new Git repository with `a.txt` committed as `old`.
fn repository(name: &str) -> std::path::PathBuf {
    let repo = std::env::temp_dir().join(format!("liquid-bridge-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).expect("repository");
    std::fs::write(repo.join("a.txt"), "old\n").expect("file");
    for args in [
        &["init", "-q"][..],
        &["add", "."],
        &["commit", "-q", "-m", "base"],
    ] {
        git(&repo, args);
    }
    repo
}

fn git(repo: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?}");
}

/// Asks for a checkpoint as Watershed's extension does and checks that the core took it.
fn checkpoint(id: &str, turn: &str, phase: &str, answer: &str) -> String {
    format!(
        r#"printf '%s\n' '{{"type":"extension_ui_request","id":"{id}","method":"input","title":"watershed.checkpoint {{\"session\":\"s1\",\"turn\":\"{turn}\",\"phase\":\"{phase}\"}}","placeholder":""}}'
IFS= read -r line
case "$line" in *'"id":"{id}"'*'{answer}'*) ;; *) exit 9 ;; esac
"#
    )
}

/// Answers a history read with `entries`, the first time after its `get_state`.
fn entries(id: u32, entries: &str) -> String {
    entries_at(id, entries, "c2")
}

/// Answers a history read with `entries` and the session's leaf at `leaf`.
fn entries_at(id: u32, entries: &str, leaf: &str) -> String {
    format!(
        "IFS= read -r line\ncase \"$line\" in *'\"type\":\"get_entries\"'*) ;; *) exit 9 ;; esac\nprintf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{{\"entries\":[{entries}],\"leafId\":\"{leaf}\"}}}}'\n"
    )
}

/// The entries of turn `T1`: its start checkpoint, its prompt and its end checkpoint.
const TURN: &str = r#"{"type":"custom","id":"c1","parentId":null,"timestamp":"2026-11-01T10:00:00.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T1","phase":"start"}},{"type":"message","id":"u1","parentId":"c1","timestamp":"2026-11-01T10:00:00.100Z","message":{"role":"user","content":"edit"}},{"type":"custom","id":"c2","parentId":"u1","timestamp":"2026-11-01T10:00:05.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T1","phase":"end","files":1,"added":1,"removed":1}}"#;

/// The confirmation of reverting `repo` to before turn `T1`, as the dialog reads it now.
fn confirmed(repo: &std::path::Path) -> BridgeRevertConfirmation {
    let checkpoints = meta_harness_core::Checkpoints::new(repo);
    let start = meta_harness_core::Checkpoint {
        session: "s1".to_owned(),
        turn: "T1".to_owned(),
        phase: meta_harness_core::Phase::Start,
    };
    let plan = call(checkpoints.revert_plan(&start, None)).expect("plan");
    call(checkpoints.close());
    plan.confirmation.into()
}

/// Waits until the stand-in agent created `marker`.
fn reached(marker: &std::path::Path) {
    let started = std::time::Instant::now();
    while !marker.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the agent got there"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn diffs_each_turn_between_its_checkpoints() {
    let repo = repository("diff");
    // The stand-in agent asks for its turn's checkpoints and edits the project in between,
    // writing a character that reorders the line.
    let script = [
        checkpoint("q1", "T1", "start", r#"\"ok\":true"#),
        "printf 'new\\342\\200\\256\\n' > a.txt\n".to_owned(),
        checkpoint("q2", "T1", "end", r#"\"files\":1"#),
        "touch .git/checkpointed\n".to_owned(),
        no_session_file(2),
        entries(3, TURN),
        no_session_file(4),
        no_session_file(5),
        entries(6, ""),
        no_session_file(7),
        no_session_file(8),
        entries(9, ""),
        no_session_file(10),
        no_session_file(11),
        entries(12, ""),
        no_session_file(13),
        "IFS= read -r line || exit 0".to_owned(),
    ]
    .concat();
    let session = launch(gated(&script), repo.display().to_string()).expect("agent starts");
    reached(&repo.join(".git/checkpointed"));
    assert_eq!(session.in_repository(), Ok(true));
    assert_eq!(
        session.turns(),
        Ok(vec![BridgeTurn {
            id: "T1".to_owned(),
            start_entry: "c1".to_owned(),
            end_entry: Some("c2".to_owned()),
            number: 1,
            prompt_entry: Some("u1".to_owned()),
            prompt: "edit".to_owned(),
            started_ms: Some(1_793_527_200_000),
            ended_ms: Some(1_793_527_205_000),
            ended: true,
            failure: None,
            files: Some(1),
            added: Some(1),
            removed: Some(1),
        }])
    );
    assert_eq!(
        session.turn_changes("T1".to_owned(), "T1".to_owned(), false),
        Ok(BridgeTurnChanges {
            state: BridgeChangesState::Shown,
            files: vec![BridgeFileChange {
                path: "a.txt".to_owned(),
                shown: "a.txt".to_owned(),
                status: BridgeChangeStatus::Modified,
                added: 1,
                removed: 1,
            }],
        })
    );
    let patch = session
        .turn_patch("T1".to_owned(), "T1".to_owned(), "a.txt".to_owned(), false)
        .expect("patch");
    assert!(
        patch.contains("-old\n") && patch.contains("+new<U+202E>\n"),
        "{patch}"
    );
    // Removed checkpoints say so instead of showing no changes.
    git(
        &repo,
        &["update-ref", "-d", "refs/watershed/checkpoints/s1/T1/end"],
    );
    assert_eq!(
        session
            .turn_changes("T1".to_owned(), "T1".to_owned(), false)
            .map(|changes| changes.state),
        Ok(BridgeChangesState::Removed)
    );
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn reverts_a_turns_files_and_continues_from_before_its_prompt() {
    let repo = repository("revert");
    let script = [
        checkpoint("q1", "T1", "start", r#"\"ok\":true"#),
        "printf 'new\\n' > a.txt\n".to_owned(),
        checkpoint("q2", "T1", "end", r#"\"files\":1"#),
        "touch .git/checkpointed\n".to_owned(),
        no_session_file(2),
        entries(3, TURN),
        no_session_file(4),
        // The plan looks for pi's session directory to name the other agent's session.
        no_session_file(5),
        no_session_file(6),
        entries(7, ""),
        no_session_file(8),
        no_session_file(9),
        entries(10, ""),
        no_session_file(11),
        gate(12),
        r#"IFS= read -r line
case "$line" in *'"message":"/watershed-branch u1"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"13","type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}'
"#
        .to_owned(),
        // pi now continues before the turn's prompt.
        no_session_file(14),
        entries_at(15, "", "c1"),
        no_session_file(16),
        "IFS= read -r line || exit 0".to_owned(),
    ]
    .concat();
    let session = launch(gated(&script), repo.display().to_string()).expect("agent starts");
    reached(&repo.join(".git/checkpointed"));
    // Another window's agent runs a turn in the same checkout.
    let other = meta_harness_core::Checkpoints::new(&repo);
    let theirs = |phase| meta_harness_core::Checkpoint {
        session: "s2".to_owned(),
        turn: "u1".to_owned(),
        phase,
    };
    call(other.capture(&theirs(meta_harness_core::Phase::Start))).expect("started");

    let plan = session.revert_plan("T1".to_owned()).expect("plan");
    assert_eq!(plan.state, BridgeChangesState::Shown);
    assert_eq!(
        plan.files,
        [BridgeRevertFile {
            path: "a.txt".to_owned(),
            shown: "a.txt".to_owned(),
            status: BridgeChangeStatus::Modified,
            edited: false,
            modified_ms: plan.files[0].modified_ms,
            ignored: false,
            kept: false,
        }]
    );
    assert!(plan.files[0].modified_ms.is_some());
    assert_eq!(
        plan.running,
        [BridgeRunningAgent {
            own: false,
            title: None,
        }]
    );
    assert!(plan.branch.is_some());
    assert!(
        matches!(session.revert_to_turn("T1".to_owned(), plan.confirmation.clone()), Err(error) if error.contains("an agent runs in this checkout")),
        "refused"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("a.txt")).expect("a"),
        "new\n"
    );

    call(other.capture(&theirs(meta_harness_core::Phase::End))).expect("ended");
    assert_eq!(
        session.revert_to_turn("T1".to_owned(), plan.confirmation.clone()),
        Ok(())
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("a.txt")).expect("a"),
        "old\n"
    );
    call(other.close());
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn says_when_pi_handled_the_branch_but_stayed_on_the_turn() {
    let repo = repository("revert-stayed");
    let script = [
        checkpoint("q1", "T1", "start", r#"\"ok\":true"#),
        "printf 'new\\n' > a.txt\n".to_owned(),
        checkpoint("q2", "T1", "end", r#"\"files\":1"#),
        "touch .git/checkpointed\n".to_owned(),
        no_session_file(2),
        entries(3, TURN),
        no_session_file(4),
        gate(5),
        // An extension cancelled the navigation; pi still reports the command handled.
        r#"IFS= read -r line
printf '%s\n' '{"id":"6","type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}'
"#
        .to_owned(),
        no_session_file(7),
        entries(8, ""),
        no_session_file(9),
        "IFS= read -r line || exit 0".to_owned(),
    ]
    .concat();
    let session = launch(gated(&script), repo.display().to_string()).expect("agent starts");
    reached(&repo.join(".git/checkpointed"));
    let reverted = session.revert_to_turn("T1".to_owned(), confirmed(&repo));
    assert!(
        matches!(&reverted, Err(error) if error.starts_with("the files were reverted, but the session could not continue")),
        "{reverted:?}"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("a.txt")).expect("a"),
        "old\n"
    );
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn refuses_a_revert_once_the_files_left_the_confirmed_plan() {
    let repo = repository("revert-stale");
    let script = [
        checkpoint("q1", "T1", "start", r#"\"ok\":true"#),
        "printf 'new\\n' > a.txt\n".to_owned(),
        checkpoint("q2", "T1", "end", r#"\"files\":1"#),
        "touch .git/checkpointed\n".to_owned(),
        no_session_file(2),
        entries(3, TURN),
        no_session_file(4),
        "IFS= read -r line || exit 0".to_owned(),
    ]
    .concat();
    let session = launch(gated(&script), repo.display().to_string()).expect("agent starts");
    reached(&repo.join(".git/checkpointed"));
    let confirmed = confirmed(&repo);
    std::fs::write(repo.join("b.txt"), "unlisted\n").expect("file");

    assert_eq!(
        session.revert_to_turn("T1".to_owned(), confirmed),
        Err(
            "nothing was reverted: files changed since you confirmed. Open Revert again to see \
             the current list."
                .to_owned()
        )
    );
    let read = |path: &str| std::fs::read_to_string(repo.join(path)).expect(path);
    assert_eq!(
        (read("a.txt"), read("b.txt")),
        ("new\n".to_owned(), "unlisted\n".to_owned())
    );
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn a_prompt_waits_until_the_revert_moved_the_session() {
    let repo = repository("revert-serialized");
    let script = [
        checkpoint("q1", "T1", "start", r#"\"ok\":true"#),
        "printf 'new\\n' > a.txt\n".to_owned(),
        checkpoint("q2", "T1", "end", r#"\"files\":1"#),
        "touch .git/checkpointed\n".to_owned(),
        no_session_file(2),
        entries(3, TURN),
        no_session_file(4),
        gate(5),
        // An extension asks before the move; the next line must be its answer, not a prompt.
        r#"IFS= read -r line
case "$line" in *'"type":"prompt"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"type":"extension_ui_request","id":"x1","method":"confirm","title":"Leave?","message":""}'
touch .git/asked
IFS= read -r line
case "$line" in *'"id":"x1"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"6","type":"response","command":"prompt","success":true}'
"#
        .to_owned(),
        no_session_file(7),
        entries_at(8, "", "c1"),
        no_session_file(9),
        gate(10),
        r#"IFS= read -r line
case "$line" in *'"message":"next"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"11","type":"response","command":"prompt","success":true}'
IFS= read -r line || exit 0"#
            .to_owned(),
    ]
    .concat();
    let session =
        Arc::new(launch(gated(&script), repo.display().to_string()).expect("agent starts"));
    reached(&repo.join(".git/checkpointed"));
    let reverting = {
        let session = Arc::clone(&session);
        let confirmed = confirmed(&repo);
        std::thread::spawn(move || session.revert_to_turn("T1".to_owned(), confirmed))
    };
    reached(&repo.join(".git/asked"));
    let sending = {
        let session = Arc::clone(&session);
        std::thread::spawn(move || session.send("next".to_owned(), false))
    };
    // Time for the prompt to overtake the move, were it free to.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(call(session.answer_confirm("x1".to_owned(), true)), Ok(()));

    assert_eq!(reverting.join().expect("revert returns"), Ok(()));
    assert_eq!(sending.join().expect("send returns"), Ok(()));
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn says_when_the_files_were_reverted_but_the_session_did_not_branch() {
    let repo = repository("revert-unbranched");
    let script = [
        checkpoint("q1", "T1", "start", r#"\"ok\":true"#),
        "printf 'new\\n' > a.txt\n".to_owned(),
        checkpoint("q2", "T1", "end", r#"\"files\":1"#),
        "touch .git/checkpointed\n".to_owned(),
        no_session_file(2),
        entries(3, TURN),
        no_session_file(4),
        gate(5),
        r#"IFS= read -r line
printf '%s\n' '{"id":"6","type":"response","command":"prompt","success":false,"error":"Agent is busy"}'
IFS= read -r line || exit 0"#
            .to_owned(),
    ]
    .concat();
    let session = launch(gated(&script), repo.display().to_string()).expect("agent starts");
    reached(&repo.join(".git/checkpointed"));
    let reverted = session.revert_to_turn("T1".to_owned(), confirmed(&repo));
    assert!(
        matches!(&reverted, Err(error) if error.starts_with("the files were reverted, but the session could not continue")),
        "{reverted:?}"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("a.txt")).expect("a"),
        "old\n"
    );
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn names_a_few_files_git_left_unreverted_as_liquid_shows_them() {
    let paths: Vec<String> = ["a\u{202e}.txt", "b.txt", "c.txt", "d.txt", "e.txt"]
        .map(str::to_owned)
        .to_vec();
    assert_eq!(
        crate::api::session::left_unreverted(&paths),
        "the other files were reverted and the session stays on its branch; Git could not \
         revert a<U+202E>.txt, b.txt, c.txt, 2 more"
    );
}

#[test]
fn writes_out_what_hides_in_gits_errors() {
    assert_eq!(
        crate::api::session::unread(meta_harness_core::DiffError::Git(
            "cannot read 'a\u{202e}txt.exe'".to_owned()
        )),
        Err("git failed: cannot read 'a<U+202E>txt.exe'".to_owned())
    );
    assert_eq!(
        crate::api::session::unread(meta_harness_core::DiffError::NoCheckpoint),
        Ok(BridgeChangesState::Removed)
    );
}

/// Answers a `get_state` with the session stored in `file`.
fn state_with(id: u32, file: &std::path::Path) -> String {
    format!(
        "IFS= read -r line\ncase \"$line\" in *'\"type\":\"get_state\"'*) ;; *) exit 9 ;; esac\nprintf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_state\",\"success\":true,\"data\":{{\"sessionId\":\"s1\",\"isStreaming\":false,\"sessionFile\":\"{}\"}}}}'\n",
        file.display()
    )
}

#[test]
fn deletes_a_stored_session_with_its_checkpoints_and_removes_all_checkpoints() {
    let repo = repository("delete");
    let sessions = repo.join(".git/pi-sessions");
    std::fs::create_dir_all(&sessions).expect("sessions");
    let header = |id: &str| {
        format!(
            r#"{{"type":"session","version":3,"id":"{id}","cwd":"{}"}}"#,
            repo.display()
        )
    };
    let current = sessions.join("current.jsonl");
    let old = sessions.join("old.jsonl");
    std::fs::write(&current, header("s1") + "\n").expect("current");
    std::fs::write(
        &old,
        header("s9")
            + "\n"
            + r#"{"type":"session_info","id":"i1","parentId":null,"name":"Old work"}"#
            + "\n",
    )
    .expect("old");
    let checkpoints = meta_harness_core::Checkpoints::new(&repo);
    for session in ["s1", "s9"] {
        for phase in [
            meta_harness_core::Phase::Start,
            meta_harness_core::Phase::End,
        ] {
            call(checkpoints.capture(&meta_harness_core::Checkpoint {
                session: session.to_owned(),
                turn: "t1".to_owned(),
                phase,
            }))
            .expect("captured");
        }
    }
    // The window that took these checkpoints moved on, as a switch or a new session does.
    call(checkpoints.show_session(None)).expect("moved on");
    let script = [
        (2..=10).map(|id| state_with(id, &current)).collect(),
        "IFS= read -r line || exit 0".to_owned(),
    ]
    .concat();
    let session = launch(gated(&script), repo.display().to_string()).expect("agent starts");
    assert_eq!(
        session.checkpoint_sessions(),
        Ok(vec![
            BridgeCheckpointSession {
                title: "Untitled session".to_owned(),
                turns: 1,
            },
            BridgeCheckpointSession {
                title: "Old work".to_owned(),
                turns: 1,
            },
        ])
    );
    assert_eq!(session.stored_turns(old.display().to_string()), Ok(1));
    assert_eq!(session.stored_turns(current.display().to_string()), Ok(1));
    assert!(
        session
            .delete_session(current.display().to_string())
            .is_err(),
        "the open session is switched away from first"
    );
    let outside = repo.join("a.txt");
    assert!(
        session
            .delete_session(outside.display().to_string())
            .is_err()
    );
    assert!(outside.exists());
    // A copy of the open session under another name is the open session.
    let twin = sessions.join("twin.jsonl");
    std::fs::copy(&current, &twin).expect("twin");
    assert!(session.delete_session(twin.display().to_string()).is_err());
    assert!(twin.exists());
    let notes = sessions.join("notes.txt");
    std::fs::copy(&old, &notes).expect("notes");
    assert!(session.delete_session(notes.display().to_string()).is_err());
    assert!(notes.exists());
    // A session pi never stored leaves nothing to delete.
    assert_eq!(
        session.delete_session(sessions.join("never.jsonl").display().to_string()),
        Ok(())
    );
    assert_eq!(session.delete_session(old.display().to_string()), Ok(()));
    assert!(!old.exists());
    let refs = std::process::Command::new("git")
        .args(["for-each-ref", "--format=%(refname)", "refs/watershed"])
        .current_dir(&repo)
        .output()
        .expect("git runs");
    assert_eq!(
        String::from_utf8_lossy(&refs.stdout),
        "refs/watershed/checkpoints/s1/t1/end\nrefs/watershed/checkpoints/s1/t1/start\n"
    );
    assert_eq!(session.remove_all_checkpoints(), Ok(()));
    assert_eq!(call(checkpoints.sessions()).expect("listed"), []);
    call(checkpoints.close());
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn a_session_a_window_shows_is_not_deleted_by_another_window() {
    isolate_user_home();
    let repo = repository("shown");
    let sessions = repo.join(".git/pi-sessions");
    std::fs::create_dir_all(&sessions).expect("sessions");
    let header = |id: &str| {
        format!(
            r#"{{"type":"session","version":3,"id":"{id}","cwd":"{}"}}"#,
            repo.display()
        ) + "\n"
    };
    let current = sessions.join("current.jsonl");
    let old = sessions.join("old.jsonl");
    std::fs::write(&current, header("s1")).expect("current");
    std::fs::write(&old, header("s9")).expect("old");
    // This window resumes `old`, so it shows that session.
    let window = AgentSession::launch(AgentCommand {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), gated("IFS= read -r line || exit 0").into()],
        extension: EXTENSION.into(),
        session: Some(old.clone()),
        cwd: repo.clone(),
    })
    .expect("agent starts");
    let script = [
        state_with(2, &current),
        state_with(3, &current),
        "IFS= read -r line || exit 0".to_owned(),
    ]
    .concat();
    let other = launch(gated(&script), repo.display().to_string()).expect("agent starts");
    assert!(other.delete_session(old.display().to_string()).is_err());
    assert!(old.exists(), "kept while another window shows it");
    assert!(call(window.shutdown()));
    assert_eq!(other.delete_session(old.display().to_string()), Ok(()));
    assert!(!old.exists());
    assert!(call(other.shutdown()));
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn deletes_a_session_of_a_project_outside_git() {
    let dir = std::env::temp_dir().join(format!(
        "liquid-bridge-delete-no-git-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&sessions).expect("sessions");
    let header = |id: &str| {
        format!(
            r#"{{"type":"session","version":3,"id":"{id}","cwd":"{}"}}"#,
            dir.display()
        ) + "\n"
    };
    let current = sessions.join("current.jsonl");
    let old = sessions.join("old.jsonl");
    std::fs::write(&current, header("s1")).expect("current");
    std::fs::write(&old, header("s9")).expect("old");
    let script = [
        state_with(2, &current),
        state_with(3, &current),
        "IFS= read -r line || exit 0".to_owned(),
    ]
    .concat();
    let session = launch(gated(&script), dir.display().to_string()).expect("agent starts");
    assert_eq!(session.checkpoint_sessions(), Ok(vec![]));
    assert_eq!(session.stored_turns(old.display().to_string()), Ok(0));
    assert_eq!(session.delete_session(old.display().to_string()), Ok(()));
    assert!(!old.exists());
    assert_eq!(session.remove_all_checkpoints(), Ok(()));
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_project_outside_git_has_no_checkpoints_and_says_so() {
    let dir = std::env::temp_dir().join(format!("liquid-bridge-no-git-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("directory");
    let script = [
        checkpoint("q1", "T1", "start", "not inside a Git working tree"),
        "touch checkpointed\n".to_owned(),
        no_session_file(2),
        entries(3, TURN),
        no_session_file(4),
        no_session_file(5),
        entries(6, ""),
        no_session_file(7),
        "IFS= read -r line || exit 0".to_owned(),
    ]
    .concat();
    let session = launch(gated(&script), dir.display().to_string()).expect("agent starts");
    reached(&dir.join("checkpointed"));
    assert_eq!(session.in_repository(), Ok(false));
    assert_eq!(session.turns().map(|turns| turns.len()), Ok(1));
    assert_eq!(
        session
            .turn_changes("T1".to_owned(), "T1".to_owned(), false)
            .map(|changes| changes.state),
        Ok(BridgeChangesState::OutsideGit)
    );
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shuts_down_while_a_slow_project_is_checkpointed() {
    let repo = repository("slow");
    // A clean filter runs while the project is checkpointed; this one holds the capture up.
    git(&repo, &["config", "filter.slow.clean", "sleep 10; cat"]);
    std::fs::write(repo.join(".gitattributes"), "*.txt filter=slow\n").expect("attributes");
    std::fs::write(repo.join("a.txt"), "new\n").expect("file");
    let script = r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"input","title":"watershed.checkpoint {\"session\":\"s1\",\"turn\":\"T1\",\"phase\":\"start\"}","placeholder":""}'
IFS= read -r line || exit 0"#;
    let session =
        Arc::new(launch(gated(script), repo.display().to_string()).expect("agent starts"));
    std::thread::sleep(Duration::from_millis(300));
    let (stopped, stop) = mpsc::channel();
    {
        let session = Arc::clone(&session);
        std::thread::spawn(move || stopped.send(call(session.shutdown())));
    }
    assert!(
        stop.recv_timeout(Duration::from_secs(5)).is_ok(),
        "the checkpoint does not hold up shutdown"
    );
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn reads_the_history_of_each_branch_and_branches_from_a_step() {
    // A session forked after its first response: `e2` ends the original branch, `b1` the fork.
    let entries = r#"{"entries":[{"type":"message","id":"e1","parentId":null,"timestamp":"2026-10-24T10:00:00.000Z","message":{"role":"user","content":"Add rate limiting"}},{"type":"message","id":"e2","parentId":"e1","timestamp":"2026-10-24T10:00:02.000Z","message":{"role":"assistant","model":"faux-1","usage":{"input":10,"output":2,"cost":{"total":0.001}},"content":[{"type":"text","text":"Done."}]}},{"type":"message","id":"b1","parentId":"e1","timestamp":"2026-10-24T10:01:00.000Z","message":{"role":"user","content":"Use a token bucket"}}],"leafId":"b1"}"#;
    let respond = |id: &str| {
        format!(
            "IFS= read -r line\nprintf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{entries}}}'\n"
        )
    };
    let script = [
        no_session_file(2),
        respond("3"),
        no_session_file(4),
        no_session_file(5),
        // The second read continues after the last entry; nothing was added.
        r#"IFS= read -r line
case "$line" in *'"since":"b1"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"6","type":"response","command":"get_entries","success":true,"data":{"entries":[],"leafId":"b1"}}'
"#
        .to_owned(),
        no_session_file(7),
        gate(8),
        r#"IFS= read -r line
case "$line" in *'"message":"/watershed-branch e1"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"9","type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}'
IFS= read -r line || exit 0"#
            .to_owned(),
    ]
    .concat();
    let session =
        launch(gated(&script), std::env::temp_dir().display().to_string()).expect("agent starts");

    let current = session.history(Vec::new()).expect("history");
    assert_eq!(
        (
            current.usage.input,
            current.usage.output,
            current.usage.cost_micro_usd
        ),
        (10, 2, 1_000),
        "what the stored responses used"
    );
    assert_eq!(current.leaf.as_deref(), Some("b1"));
    let nodes: Vec<_> = current
        .tree
        .iter()
        .map(|node| {
            (
                node.id.as_str(),
                node.parent.as_deref(),
                node.kind,
                node.at_ms,
            )
        })
        .collect();
    assert_eq!(
        nodes,
        [
            ("e1", None, BridgeEntryKind::User, Some(1_792_836_000_000)),
            (
                "e2",
                Some("e1"),
                BridgeEntryKind::Assistant,
                Some(1_792_836_002_000)
            ),
            (
                "b1",
                Some("e1"),
                BridgeEntryKind::User,
                Some(1_792_836_060_000)
            ),
        ]
    );
    let steps =
        |history: &crate::api::history::BridgeHistory| -> Vec<Vec<(BridgeRecordKind, String)>> {
            history
                .trajectories
                .iter()
                .map(|trajectory| {
                    trajectory
                        .records
                        .iter()
                        .map(|record| (record.kind, record.label.clone()))
                        .collect()
                })
                .collect()
        };
    assert_eq!(
        steps(&current),
        [vec![
            (BridgeRecordKind::User, "Add rate limiting".to_owned()),
            (BridgeRecordKind::User, "Use a token bucket".to_owned()),
        ]]
    );

    let both = session
        .history(vec!["e2".to_owned(), "b1".to_owned()])
        .expect("history");
    assert_eq!(
        steps(&both)[0][1],
        (BridgeRecordKind::Message, "Done.".to_owned())
    );
    assert_eq!(both.trajectories[0].duration_ms, 2000);
    let response = &both.trajectories[0].records[1];
    assert_eq!(
        (
            response.duration_ms,
            response.usage.map(|usage| usage.cost_micro_usd)
        ),
        (None, Some(1000))
    );
    assert_eq!(steps(&both)[1], steps(&current)[0]);

    assert_eq!(session.branch("e1".to_owned()), Ok(()));
    assert!(call(session.shutdown()));
}

#[test]
fn reads_tool_outcomes_and_what_the_model_saw() {
    let entries = r#"{"entries":[{"type":"message","id":"s1","parentId":null,"message":{"role":"system","content":"","sections":{"preamble":"You are pi."},"toolsAdded":[{"name":"write"}]}},{"type":"message","id":"u1","parentId":"s1","message":{"role":"user","content":"Write it"}},{"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"write","arguments":{"path":"a.ts","content":"x\ny\n"}}]}},{"type":"message","id":"r1","parentId":"a1","message":{"role":"toolResult","toolCallId":"c1","toolName":"write","content":[{"type":"text","text":"ok"}]}},{"type":"message","id":"a2","parentId":"r1","message":{"role":"assistant","content":[{"type":"text","text":"Done."}]}}],"leafId":"a2"}"#;
    let script = no_session_file(2)
        + &format!(
            "IFS= read -r line\nprintf '%s\\n' '{{\"id\":\"3\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{entries}}}'\n"
        )
        + &no_session_file(4)
        + &no_session_file(5)
        + "IFS= read -r line\nprintf '%s\\n' '{\"id\":\"6\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{\"entries\":[],\"leafId\":\"a2\"}}'\n"
        + &no_session_file(7)
        + "IFS= read -r line || exit 0";
    let session =
        launch(gated(&script), std::env::temp_dir().display().to_string()).expect("agent starts");
    let history = session.history(Vec::new()).expect("history");
    let records = &history.trajectories[0].records;
    assert_eq!(records[2].kind, BridgeRecordKind::Tool);
    assert_eq!(records[2].outcome.added, Some(2), "lines the write wrote");
    let result = history.tree.iter().find(|node| node.id == "r1");
    assert_eq!(
        result.map(|node| (node.preview.as_str(), node.failed)),
        Some(("write a.ts", false)),
        "the tree names what the write wrote to"
    );
    assert_eq!(
        records
            .iter()
            .map(|record| record.added_chars)
            .collect::<Vec<_>>(),
        [8, 34, 2, 5],
        "what the prompt, the call, the result and the answer add to the context"
    );
    let context = records[3].context.as_ref().expect("a2 saw context");
    assert_eq!(
        (
            context.prompt_chars,
            context.sections.clone(),
            context.tool_chars > 0,
            context.messages
        ),
        (11, vec!["preamble".to_owned()], true, 3)
    );
    let detail = session
        .context("a2".to_owned())
        .expect("context")
        .expect("a2 is a response");
    assert_eq!(
        detail
            .messages
            .iter()
            .map(|message| (message.label.as_str(), message.fresh))
            .collect::<Vec<_>>(),
        [
            ("prompt · Write it", false),
            ("response · (tool calls)", false),
            ("result of write · a.ts", true),
        ]
    );
    assert_eq!(detail.tools[0].name, "write");
    assert!(call(session.shutdown()));
}

#[test]
fn rebuilds_the_chat_of_the_current_branch() {
    let entries = r#"{"entries":[{"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"Fix it"}},{"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[{"type":"text","text":"Editing."},{"type":"toolCall","id":"c1","name":"edit","arguments":{"path":"a.ts"}}]}},{"type":"message","id":"r1","parentId":"a1","message":{"role":"toolResult","toolCallId":"c1","toolName":"edit","content":[]}}],"leafId":"r1"}"#;
    let script = no_session_file(2)
        + &format!(
            "IFS= read -r line\nprintf '%s\\n' '{{\"id\":\"3\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{entries}}}'\n"
        )
        + &no_session_file(4)
        + "IFS= read -r line || exit 0";
    let session =
        launch(gated(&script), std::env::temp_dir().display().to_string()).expect("agent starts");
    let transcript = session.transcript(None, 50).expect("transcript");
    let items: Vec<_> = transcript
        .items
        .iter()
        .map(|item| {
            (
                item.entry.as_str(),
                item.kind,
                item.text.as_str(),
                item.tool.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        items,
        [
            ("u1", BridgeItemKind::UserMessage, "Fix it", None),
            ("a1", BridgeItemKind::AssistantMessage, "Editing.", None),
            ("a1", BridgeItemKind::FileChange, "", Some("edit")),
        ]
    );
    assert_eq!(
        (transcript.more, transcript.start.as_deref()),
        (false, Some("u1"))
    );
    assert!(call(session.shutdown()));
}

#[test]
fn passes_the_waiting_messages_to_dart() {
    let changed = bridge(AgentEvent::QueueChanged(meta_harness_core::Queue {
        steering: vec!["Use Redis".to_owned()],
        follow_up: vec!["Also docs".to_owned()],
    }));
    assert_eq!(changed.kind, BridgeEventKind::QueueChanged);
    assert_eq!(
        changed.queue,
        Some(BridgeQueue {
            steering: vec!["Use Redis".to_owned()],
            follow_up: vec!["Also docs".to_owned()],
        })
    );
}

#[test]
fn starts_no_agent_without_the_extension_that_asks_first() {
    // Another extension loads fine, but none would ask before a tool call.
    let script = r#"IFS= read -r line
printf '%s\n' '{"id":"1","type":"response","command":"get_commands","success":true,"data":{"commands":[]}}'
IFS= read -r line || exit 0"#;
    let started = launch(
        script.to_owned(),
        std::env::temp_dir().display().to_string(),
    );
    let error = started.err().expect("no session starts");
    assert!(error.contains("Watershed"), "{error}");
}

#[test]
fn says_why_the_agent_did_not_start() {
    let script = r#"printf 'Error: Failed to load extension\nHint: Start without extensions.\n' >&2
exit 1"#;
    let started = launch(
        script.to_owned(),
        std::env::temp_dir().display().to_string(),
    );
    let error = started.err().expect("no session starts");
    assert!(
        error.starts_with("Error: Failed to load extension"),
        "{error}"
    );

    let missing = AgentSession::start(SessionConfig {
        program: "/nonexistent/pi".to_owned(),
        extension_path: EXTENSION.to_owned(),
        session: None,
        cwd: std::env::temp_dir().display().to_string(),
    });
    let error = missing.err().expect("no session starts");
    assert!(
        error.starts_with("could not run /nonexistent/pi: "),
        "{error}"
    );
}

#[test]
fn starts_after_an_agent_says_much_before_it_is_ready() {
    // More events than the session's channel holds, before the answer to the start check.
    let script = r#"i=0; while [ $i -lt 300 ]; do printf '%s\n' "{\"type\":\"extension_ui_request\",\"id\":\"s$i\",\"method\":\"setStatus\",\"statusKey\":\"k\",\"statusText\":\"$i\"}"; i=$((i+1)); done
"#
    .to_owned()
        + &gated("IFS= read -r line || exit 0");
    let (done, started) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(launch(
            script.to_owned(),
            std::env::temp_dir().display().to_string(),
        ));
    });
    let session = started
        .recv_timeout(Duration::from_secs(10))
        .expect("the start ends")
        .expect("agent starts");
    let (sender, receiver) = mpsc::channel();
    session
        .forward(move |event| sender.send(event).is_ok())
        .expect("first subscriber");
    // The consumer confirms each event it applied, as Liquid does; the window waits otherwise.
    let mut applied = 0;
    let statuses = std::iter::from_fn(|| receiver.recv_timeout(Duration::from_secs(5)).ok())
        .inspect(|_| {
            applied += 1;
            session.confirm_applied(applied);
        })
        .take(300)
        .filter(|event| event.kind == BridgeEventKind::StatusChanged)
        .count();
    assert_eq!(statuses, 300, "every early event reaches Dart");
    assert!(call(session.shutdown()));
}

#[test]
fn refuses_an_agent_that_says_too_much_before_it_is_ready() {
    // One event more than a starting agent may send, then the answer to the start check.
    let script = r#"i=0; while [ $i -le 4096 ]; do printf '%s\n' "{\"type\":\"extension_ui_request\",\"id\":\"s$i\",\"method\":\"setStatus\",\"statusKey\":\"k\",\"statusText\":\"$i\"}"; i=$((i+1)); done
"#
    .to_owned()
        + &gated("IFS= read -r line || exit 0");
    let error = launch(script, std::env::temp_dir().display().to_string())
        .err()
        .expect("no session starts");
    assert!(error.contains("4096 events"), "{error}");
}

#[test]
fn gives_up_on_an_agent_that_never_says_it_is_ready() {
    let (done, started) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(launch(
            "while :; do sleep 1; done".to_owned(),
            std::env::temp_dir().display().to_string(),
        ));
    });
    let error = started
        .recv_timeout(Duration::from_secs(10))
        .expect("the start ends")
        .err()
        .expect("no session starts");
    assert!(error.contains("did not answer"), "{error}");
}

#[test]
fn commit_bridge_exposes_attribution_whole_files_and_safe_notices() {
    use crate::api::git::BridgeGitOutcomeKind;
    let repo = repository("commit-bridge");
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    let script = [
        checkpoint("q1", "T1", "start", r#"\"ok\":true"#),
        "printf 'new\\n' > a.txt\n".to_owned(),
        checkpoint("q2", "T1", "end", r#"\"files\":1"#),
        "touch .git/ready\n".to_owned(),
        no_session_file(2),
        entries(3, TURN),
        no_session_file(4),
        no_session_file(5),
        entries(6, TURN),
        no_session_file(7),
        no_session_file(8),
        entries(9, TURN),
        no_session_file(10),
        "IFS= read -r line || exit 0".to_owned(),
    ]
    .concat();
    let session = launch(gated(&script), repo.display().to_string()).unwrap();
    reached(&repo.join(".git/ready"));
    let plan = session.commit_plan().unwrap();
    assert_eq!(plan.files.len(), 1);
    assert!(plan.files[0].attribution_available);
    assert_eq!(plan.files[0].turns, [1]);
    assert!(!plan.files[0].edited);
    assert!(plan.running.is_empty());
    assert_eq!(
        session.uncommitted_changes(false).unwrap().1[0].path,
        "a.txt"
    );
    assert!(
        session
            .uncommitted_patch("a.txt".into(), false)
            .unwrap()
            .contains("+new")
    );
    let result = session
        .commit_files(
            plan.snapshot,
            vec!["a.txt".into()],
            "Subject\u{202e}".into(),
            None,
        )
        .unwrap();
    assert_eq!(result.kind, BridgeGitOutcomeKind::Done);
    assert_eq!(
        result.commit.as_ref().unwrap().subject.as_deref(),
        Some("Subject<U+202E>")
    );
    assert_eq!(result.commit.as_ref().unwrap().count, 1);
    assert!(session.uncommitted_changes(false).unwrap().1.is_empty());
    use std::os::unix::fs::PermissionsExt;
    let hook = repo.join(".git/hooks/pre-commit");
    std::fs::write(repo.join("a.txt"), "next\n").unwrap();
    std::fs::write(&hook, "#!/bin/sh\nprintf 'hook\\033output\\n'\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let result = session
        .commit_files(
            session.commit_plan().unwrap().snapshot,
            vec!["a.txt".into()],
            "Next".into(),
            None,
        )
        .unwrap();
    assert_eq!(result.kind, BridgeGitOutcomeKind::CommitFailed);
    assert!(result.detail.contains("hook<U+001B>output"), "{result:?}");
    std::fs::write(&hook, "#!/bin/sh\nprintf 'formatted\\n' > a.txt\n").unwrap();
    let result = session
        .commit_files(
            session.commit_plan().unwrap().snapshot,
            vec!["a.txt".into()],
            "Next".into(),
            None,
        )
        .unwrap();
    assert_eq!(result.kind, BridgeGitOutcomeKind::HookChanged);
    assert_eq!(result.files, ["a.txt"]);
    assert_eq!(session.uncommitted_changes(false).unwrap().1.len(), 1);
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn git_remote_status_redacts_synthetic_url_credentials() {
    let repo = repository("git-remote-url");
    git(&repo, &["branch", "-M", "main"]);
    git(
        &repo,
        &[
            "config",
            "branch.main.remote",
            "https://fixture:placeholder@example.invalid/repo.git",
        ],
    );
    git(&repo, &["config", "branch.main.merge", "refs/heads/main"]);
    let session = launch(
        gated("IFS= read -r line || exit 0"),
        repo.display().to_string(),
    )
    .unwrap();
    let remote = session.git_status().unwrap().unwrap().remote;
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(repo);
    assert_eq!(remote.as_deref(), Some("https://example.invalid/repo.git"));
}

#[test]
fn git_status_keeps_the_raw_branch_behind_its_displayed_name() {
    let repo = repository("git-raw-branch");
    git(&repo, &["switch", "-c", "feature/\u{202e}"]);
    let session = launch(
        gated("IFS= read -r line || exit 0"),
        repo.display().to_string(),
    )
    .unwrap();
    let status = session.git_status().unwrap().unwrap();
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(repo);
    assert_eq!(status.branch.as_deref(), Some("feature/<U+202E>"));
    assert_eq!(status.raw_branch.as_deref(), Some("feature/\u{202e}"));
}

#[test]
fn git_remote_transfer_display_redacts_synthetic_url_credentials() {
    use crate::api::git::BridgeGitTransfer;
    let remote = "https://fixture:placeholder@example.invalid/repo.git";
    let transfer = BridgeGitTransfer::from(meta_harness_core::GitTransfer {
        branch: "topic".into(),
        remote: remote.into(),
        upstream: format!("{remote}/main"),
        ahead: 1,
        behind: 0,
        first_push: false,
        gone: false,
        confirmation: meta_harness_core::PushConfirmation {
            branch: "topic".into(),
            remote: remote.into(),
            reference: "refs/heads/main".into(),
            commit: "a".repeat(40),
            repository: "https://example.invalid/repo.git".into(),
        },
    });
    assert_eq!(transfer.remote, "https://example.invalid/repo.git");
    assert_eq!(transfer.upstream, "https://example.invalid/repo.git/main");
    assert_eq!(transfer.confirmation.remote, remote);
}

#[test]
fn git_transfer_bridge_confirms_push_tracks_and_pulls() {
    use crate::api::git::BridgeGitOutcomeKind;
    let repo = repository("sync-bridge");
    git(&repo, &["branch", "-M", "main"]);
    let bare = repo.with_extension("bare");
    let other = repo.with_extension("other");
    let _ = std::fs::remove_dir_all(&bare);
    let _ = std::fs::remove_dir_all(&other);
    git(&repo, &["clone", "--bare", ".", bare.to_str().unwrap()]);
    git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&repo, &["fetch", "origin"]);
    git(&repo, &["branch", "--set-upstream-to=origin/main"]);
    git(&repo, &["remote", "set-head", "origin", "main"]);
    let session = launch(
        gated("IFS= read -r line || exit 0"),
        repo.display().to_string(),
    )
    .unwrap();
    assert_eq!(
        session.git_status().unwrap().unwrap().remote.as_deref(),
        Some("origin")
    );
    std::fs::write(repo.join("a.txt"), "local change\n").unwrap();
    git(&repo, &["commit", "-am", "Local"]);
    git(&repo, &["config", "branch.main.merge", "main"]);
    let invalid = session.push(None).unwrap();
    assert_eq!(invalid.kind, BridgeGitOutcomeKind::PreflightFailed);
    assert!(invalid.transfer.is_none());
    assert!(invalid.files.is_empty());
    git(&repo, &["config", "branch.main.merge", "refs/heads/main"]);
    git(
        &repo,
        &[
            "config",
            "remote.origin.pushurl",
            repo.with_extension("publication").to_str().unwrap(),
        ],
    );
    let unsupported = session.push(None).unwrap();
    assert_eq!(unsupported.kind, BridgeGitOutcomeKind::PreflightFailed);
    assert_eq!(
        unsupported.detail,
        "This remote pushes to other or several repositories. Push it with Git."
    );
    assert!(unsupported.transfer.is_none());
    git(&repo, &["config", "--unset", "remote.origin.pushurl"]);
    let confirmation = session.push(None).unwrap();
    assert_eq!(confirmation.kind, BridgeGitOutcomeKind::PushDefault);
    assert!(confirmation.files.is_empty());
    assert!(confirmation.detail.is_empty());
    let transfer = confirmation.transfer.unwrap();
    assert_eq!(transfer.ahead, 1);
    assert_eq!(transfer.confirmation.remote, "origin");
    assert_eq!(transfer.confirmation.reference, "refs/heads/main");
    assert!(transfer.confirmation.commit.len() >= 40);
    std::fs::write(repo.join("a.txt"), "later change\n").unwrap();
    git(&repo, &["commit", "-am", "Later"]);
    let renewed = session.push(Some(transfer.confirmation)).unwrap();
    assert_eq!(renewed.kind, BridgeGitOutcomeKind::PushDefault);
    let pushed = session
        .push(Some(renewed.transfer.unwrap().confirmation))
        .unwrap();
    assert_eq!(pushed.kind, BridgeGitOutcomeKind::Done);
    assert_eq!(pushed.transfer.unwrap().ahead, 2);
    git(
        &repo,
        &[
            "config",
            "remote.origin.uploadpack",
            "sh -c 'echo offline >&2; exit 1'",
        ],
    );
    let failed = session.push(None).unwrap();
    assert_eq!(failed.kind, BridgeGitOutcomeKind::FetchFailed);
    assert_eq!(failed.transfer.unwrap().remote, "origin");
    assert!(failed.files.is_empty());
    git(&repo, &["config", "--unset", "remote.origin.uploadpack"]);
    git(&repo, &["switch", "-c", "feature/bridge"]);
    let pushed = session.push(None).unwrap();
    assert_eq!(pushed.kind, BridgeGitOutcomeKind::Done);
    assert!(pushed.transfer.unwrap().first_push);
    git(
        &repo,
        &["clone", bare.to_str().unwrap(), other.to_str().unwrap()],
    );
    git(&other, &["switch", "feature/bridge"]);
    std::fs::write(other.join("a.txt"), "remote change\n").unwrap();
    git(&other, &["commit", "-am", "Remote"]);
    git(&other, &["push"]);
    assert!(session.fetch().unwrap());
    assert_eq!(session.git_status().unwrap().unwrap().behind, 1);
    assert_eq!(
        session.push(None).unwrap().kind,
        BridgeGitOutcomeKind::Diverged
    );
    let pulled = session.pull().unwrap();
    assert_eq!(pulled.kind, BridgeGitOutcomeKind::Done);
    assert_eq!(pulled.transfer.unwrap().behind, 1);
    assert_eq!(
        std::fs::read_to_string(repo.join("a.txt")).unwrap(),
        "remote change\n"
    );
    std::fs::write(other.join("a.txt"), "remote next\n").unwrap();
    git(&other, &["commit", "-am", "Remote next"]);
    git(&other, &["push"]);
    std::fs::write(repo.join("a.txt"), "unsaved\n").unwrap();
    let refused = session.pull().unwrap();
    assert_eq!(refused.kind, BridgeGitOutcomeKind::WouldOverwrite);
    assert_eq!(refused.files, ["a.txt"]);
    assert!(call(session.shutdown()));
    for dir in [repo, bare, other] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn commit_bridge_distinguishes_unavailable_attribution_from_no_session_changes() {
    use crate::api::git::BridgeCommitFile;
    for available in [false, true] {
        let file = BridgeCommitFile::from(meta_harness_core::CommitFile {
            change: meta_harness_core::FileChange {
                path: "outside.txt".into(),
                status: meta_harness_core::ChangeStatus::Added,
                added: 6,
                removed: 0,
            },
            attribution_available: available,
            turns: Vec::new(),
            edited: false,
            staged_partly: false,
        });
        assert_eq!(file.attribution_available, available);
        assert!(file.turns.is_empty());
        assert_eq!(file.change.path, "outside.txt");
        assert_eq!(file.change.added, 6);
    }
}

#[test]
fn initialize_bridge_exposes_the_new_repository_without_backfilled_checkpoints() {
    let repo = repository("initialize-bridge");
    std::fs::remove_dir_all(repo.join(".git")).unwrap();
    let session = launch(
        gated("IFS= read -r line || exit 0"),
        repo.display().to_string(),
    )
    .unwrap();
    assert_eq!(session.git_status(), Ok(None));
    assert_eq!(
        session.initialize_git().unwrap().kind,
        crate::api::git::BridgeGitOutcomeKind::Done
    );
    assert!(session.git_status().unwrap().is_some());
    assert!(session.checkpoint_sessions().unwrap().is_empty());
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn commit_bridge_names_the_created_branch_when_either_hook_refuses() {
    use crate::api::git::BridgeGitOutcomeKind;
    use std::os::unix::fs::PermissionsExt;
    for hook_name in ["pre-commit", "post-checkout"] {
        let repo = repository(&format!("commit-branch-{hook_name}"));
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("a.txt"), "changed\n").unwrap();
        let hook = repo.join(".git/hooks").join(hook_name);
        std::fs::write(&hook, "#!/bin/sh\nprintf 'refused\\033hook\n'\nexit 1\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let script = [
            no_session_file(2),
            entries(3, ""),
            no_session_file(4),
            "IFS= read -r line || exit 0".to_owned(),
        ]
        .concat();
        let session = launch(gated(&script), repo.display().to_string()).unwrap();
        let head = session.git_status().unwrap().unwrap().commit;
        let plan = session.commit_plan().unwrap();
        let result = session
            .commit_files(
                plan.snapshot,
                vec!["a.txt".into()],
                "Subject".into(),
                Some("feat".into()),
            )
            .unwrap();
        assert_eq!(result.kind, BridgeGitOutcomeKind::CommitBranchFailed);
        let commit = result.commit.unwrap();
        assert_eq!(commit.branch, "feat");
        assert_eq!(commit.subject, None);
        assert_eq!(commit.count, 0);
        assert!(
            result.detail.contains("refused<U+001B>hook"),
            "{}",
            result.detail
        );
        let status = session.git_status().unwrap().unwrap();
        assert_eq!(status.branch.as_deref(), Some("feat"));
        assert_eq!(status.commit, head);
        assert!(call(session.shutdown()));
        let _ = std::fs::remove_dir_all(repo);
    }
}

#[test]
fn a_committed_bridge_outcome_survives_failed_inspection() {
    use crate::api::git::BridgeGitOutcomeKind;
    use std::os::unix::fs::PermissionsExt;
    let repo = repository("commit-inspection");
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join("a.txt"), "committed\n").unwrap();
    let hook = repo.join(".git/hooks/post-commit");
    std::fs::write(&hook, "#!/bin/sh\nprintf 'bad index' > .git/index\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let script = [
        no_session_file(2),
        entries(3, TURN),
        no_session_file(4),
        "IFS= read -r line || exit 0".into(),
    ]
    .concat();
    let session = launch(gated(&script), repo.display().to_string()).unwrap();
    let result = session
        .commit_files(
            session.commit_plan().unwrap().snapshot,
            vec!["a.txt".into()],
            "Committed".into(),
            None,
        )
        .unwrap();
    assert_eq!(result.kind, BridgeGitOutcomeKind::Done);
    assert_eq!(
        result.commit.as_ref().unwrap().subject.as_deref(),
        Some("Committed")
    );
    assert!(!result.detail.is_empty());
    let output = std::process::Command::new("git")
        .args(["show", "HEAD:a.txt"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"committed\n");
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn pull_request_outcomes_say_what_blocks_creation_and_write_out_hidden_text() {
    use crate::api::git::{
        BridgePullRequest, BridgePullRequestOutcome, BridgePullRequestOutcomeKind,
        BridgePullRequestPlan, BridgePullRequestState,
    };
    use meta_harness_core::PullRequestError;
    let local = BridgePullRequestOutcome::failed(PullRequestError::NotGitHub {
        remote: "origin".into(),
        host: None,
        page: None,
    });
    assert_eq!(local.kind, BridgePullRequestOutcomeKind::NotGitHub);
    assert_eq!((local.remote.as_str(), local.host.as_str()), ("origin", ""));
    let ahead = BridgePullRequestOutcome::failed(PullRequestError::NeedsPush {
        branch: "fix/\u{202e}gnp".into(),
        remote: "origin".into(),
        ahead: 1,
        first: false,
    });
    assert_eq!(ahead.kind, BridgePullRequestOutcomeKind::NeedsPush);
    assert_eq!((ahead.ahead, ahead.first), (1, false));
    assert_eq!(ahead.branch, "fix/<U+202E>gnp");

    let pull = meta_harness_core::PullRequest {
        number: 42,
        title: "Fix \u{202e}gnp.exe".into(),
        url: "https://github.com/acme/api/pull/42".into(),
        state: meta_harness_core::PullRequestState::Draft,
        repository: "acme/api".into(),
        base: "main".into(),
        head: "bw:feature/x".into(),
        checks_passed: 1,
        checks_failed: 0,
        checks_pending: 2,
        review: None,
    };
    let shown = BridgePullRequest::from(pull.clone());
    assert_eq!(shown.state, BridgePullRequestState::Draft);
    assert!(!shown.title.contains('\u{202e}'), "{}", shown.title);
    assert_eq!((shown.checks_passed, shown.checks_pending), (1, 2));

    // The editable title and description start from the commit as Git has it.
    let plan = BridgePullRequestPlan::from(meta_harness_core::PullRequestPlan {
        host: "github.com".into(),
        head_repository: "bw/api".into(),
        head_branch: "feature/\u{202e}x".into(),
        head: "bw:feature/\u{202e}x".into(),
        head_commit: "c".into(),
        targets: vec!["acme/api".into(), "bw/api".into()],
        parent_unknown: false,
        target: Some("acme/api".into()),
        default_branch: Some("main".into()),
        base: Some("main".into()),
        comparison: None,
        comparison_failed: false,
        base_missing: false,
        open: Some(pull),
        uncommitted: 0,
        subject: "\u{267b}\u{fe0f} Refactor".into(),
        body: "Joins \u{1f469}\u{200d}\u{1f4bb} and <U+ text.".into(),
    });
    assert_eq!(plan.subject, "\u{267b}\u{fe0f} Refactor");
    assert_eq!(plan.body, "Joins \u{1f469}\u{200d}\u{1f4bb} and <U+ text.");
    assert_eq!(plan.head, "bw:feature/<U+202E>x");
    assert_eq!(plan.identity.head_branch, "feature/\u{202e}x");
}

#[test]
fn the_chats_model_writes_pull_request_text_until_stopped() {
    let repo = repository("pr-text");
    let commit = |repo: &std::path::Path| {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let base = commit(&repo);
    std::fs::write(repo.join("b.txt"), "retry\n").unwrap();
    git(&repo, &["add", "b.txt"]);
    git(&repo, &["commit", "-q", "-m", "Retry after"]);
    let head = commit(&repo);
    let slow = repo.with_extension("slow");
    let _ = std::fs::remove_file(&slow);
    // pi's one answer, as `sh -c` takes its first argument as `$0`; else the session's pi.
    let once = format!(
        "if [ \"$0\" = --print ]; then cat > /dev/null; if [ -e {0} ]; then : > {0}.started; sleep 12; fi; printf 'Retry after\\n\\nWaits.\\n'; exit 0; fi\n",
        shell_word(&slow.display().to_string())
    );
    let script = format!("{once}{}", gated("IFS= read -r line || exit 0\n"));
    let session = launch(script, repo.display().to_string()).unwrap();
    let write = || {
        session.write_pull_request_text("anthropic/claude-test".into(), base.clone(), head.clone())
    };

    let text = write().unwrap();
    assert_eq!(
        (text.title.as_str(), text.body.as_str()),
        ("Retry after", "Waits.")
    );

    std::fs::write(&slow, "").unwrap();
    let started = std::time::Instant::now();
    let deadline = started + Duration::from_secs(10);
    std::thread::scope(|scope| {
        let writing = scope.spawn(write);
        let started = slow.with_extension("slow.started");
        while !started.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        session.stop_pull_request_text();
        assert!(writing.join().unwrap().is_err());
    });
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_file(slow.with_extension("slow.started"));
    let _ = std::fs::remove_file(slow);
    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn a_reviewed_worktree_is_removed_as_the_user_saw_it() {
    use crate::api::worktrees::BridgeWorktreeOutcomeKind;
    let repo = repository("worktrees");
    let linked = repo.with_extension("linked");
    let _ = std::fs::remove_dir_all(&linked);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ],
    );
    let session = launch(
        gated("IFS= read -r line || exit 0\n"),
        repo.display().to_string(),
    )
    .unwrap();

    let listed = session.worktrees().unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed[0].main && listed[0].current);
    let path = listed[1].path.clone();
    assert_eq!(listed[1].branch.as_ref().unwrap().name, "linked");
    let review = session.worktree_removal(path.clone()).unwrap();
    assert!(review.removable);

    // A file that appears after the review keeps the worktree and asks for a new review.
    std::fs::write(linked.join("late.txt"), "late\n").unwrap();
    let changed = session.remove_worktree(review).unwrap();
    assert_eq!(changed.kind, BridgeWorktreeOutcomeKind::Changed);
    assert!(!changed.removal.as_ref().unwrap().removable);
    std::fs::remove_file(linked.join("late.txt")).unwrap();

    let review = session.worktree_removal(path).unwrap();
    let removed = session.remove_worktree(review).unwrap();
    assert_eq!(removed.kind, BridgeWorktreeOutcomeKind::Done);
    assert!(!linked.exists());
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn exports_a_sessions_record_as_a_new_folder_in_downloads_and_never_overwrites() {
    isolate_user_home();
    let repo = repository("export");
    let sessions = repo.join(".git/pi-sessions");
    std::fs::create_dir_all(&sessions).expect("sessions");
    let header = |id: &str| {
        format!(
            r#"{{"type":"session","version":3,"id":"{id}","cwd":"{}"}}"#,
            repo.display()
        )
    };
    let current = sessions.join("current.jsonl");
    std::fs::write(&current, header("s1") + "\n").expect("current");
    let old = sessions.join("old.jsonl");
    std::fs::write(
        &old,
        header("s9")
            + "\n"
            + r#"{"type":"custom","id":"c1","parentId":null,"timestamp":"2026-11-01T10:00:00.000Z","customType":"watershed.checkpoint","data":{"session":"s9","turn":"T1","phase":"start"}}"#
            + "\n"
            + r#"{"type":"message","id":"u1","parentId":"c1","timestamp":"2026-11-01T10:00:00.100Z","message":{"role":"user","content":"edit"}}"#
            + "\n"
            + r#"{"type":"custom","id":"c2","parentId":"u1","timestamp":"2026-11-01T10:00:05.000Z","customType":"watershed.checkpoint","data":{"session":"s9","turn":"T1","phase":"end","files":1,"added":1,"removed":1}}"#
            + "\n",
    )
    .expect("old");
    let checkpoints = meta_harness_core::Checkpoints::new(&repo);
    call(checkpoints.capture(&meta_harness_core::Checkpoint {
        session: "s9".to_owned(),
        turn: "T1".to_owned(),
        phase: meta_harness_core::Phase::Start,
    }))
    .expect("start captured");
    std::fs::write(repo.join("a.txt"), "new\n").expect("edit");
    call(checkpoints.capture(&meta_harness_core::Checkpoint {
        session: "s9".to_owned(),
        turn: "T1".to_owned(),
        phase: meta_harness_core::Phase::End,
    }))
    .expect("end captured");
    call(checkpoints.close());
    let script = [
        (2..=4).map(|id| state_with(id, &current)).collect(),
        "IFS= read -r line || exit 0".to_owned(),
    ]
    .concat();
    let session = launch(gated(&script), repo.display().to_string()).expect("agent starts");
    let downloads =
        std::env::temp_dir().join(format!("liquid-bridge-downloads-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&downloads);
    std::fs::create_dir_all(&downloads).expect("downloads");
    let export = |session: &AgentSession| {
        session.export_into(
            &downloads,
            &old.display().to_string(),
            "Rate limit for /login",
            "test-host",
            "2026-11-11",
        )
    };
    let folder = export(&session).expect("exported");
    assert_eq!(
        folder,
        downloads.join("Rate limit for login – record 2026-11-11")
    );
    assert_eq!(
        std::fs::read(folder.join("session.jsonl")).expect("session copy"),
        std::fs::read(&old).expect("session file"),
        "pi's file is copied byte for byte"
    );
    let record = std::fs::read_to_string(folder.join("record.json")).expect("record");
    assert!(record.contains("\"turn\": \"T1\"") && record.contains("\"path\": \"a.txt\""));
    assert!(folder.join("README.txt").is_file());
    // Another export never takes an existing folder's name.
    assert_eq!(
        export(&session).expect("second export"),
        downloads.join("Rate limit for login – record 2026-11-11 2")
    );
    assert_eq!(
        export(&session).expect("third export"),
        downloads.join("Rate limit for login – record 2026-11-11 3")
    );
    let names = std::fs::read_dir(&downloads).expect("listed").count();
    assert_eq!(names, 3, "nothing but the three folders");
    assert!(call(session.shutdown()));
    let _ = std::fs::remove_dir_all(&downloads);
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn a_failed_export_leaves_no_folder_behind() {
    let downloads =
        std::env::temp_dir().join(format!("liquid-bridge-failed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&downloads);
    std::fs::create_dir_all(&downloads).expect("downloads");
    let failed = crate::api::record::write_folder(&downloads, "Broken", |folder| {
        std::fs::write(folder.join("session.jsonl"), "partial").expect("partial copy");
        Err("disk full".to_owned())
    });
    assert_eq!(failed, Err("disk full".to_owned()));
    assert_eq!(
        std::fs::read_dir(&downloads).expect("listed").count(),
        0,
        "neither the folder nor its temporary copy stays"
    );
    let _ = std::fs::remove_dir_all(&downloads);
}

#[test]
fn a_long_multibyte_title_keeps_its_record_name_within_a_file_name() {
    // File systems such as ext4 count bytes, and the temporary folder adds `.` and `.partial`.
    let title = "語".repeat(80);
    let name = crate::api::record::record_name(&title, "2026-11-11");
    assert!(format!(".{name}.partial").len() <= 255, "{name}");
}

#[test]
fn downloads_go_to_the_folder_the_system_names_or_to_downloads() {
    let home = std::path::Path::new("/home/ana");
    assert_eq!(
        crate::api::record::download_dir_in(home, None),
        home.join("Downloads")
    );
    assert_eq!(
        crate::api::record::download_dir_in(
            home,
            Some("XDG_DESKTOP_DIR=\"$HOME/Desk\"\nXDG_DOWNLOAD_DIR=\"$HOME/Transfers\"\n")
        ),
        home.join("Transfers")
    );
    assert_eq!(
        crate::api::record::download_dir_in(home, Some("XDG_DOWNLOAD_DIR=\"/srv/incoming\"")),
        std::path::PathBuf::from("/srv/incoming")
    );
}
