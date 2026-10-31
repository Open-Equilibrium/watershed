use meta_harness_core::{
    AgentCommand, AgentEvent, ChangedLines, CommandError, Decision, EditChange, EditPreview,
    ItemDetail, ItemId, ItemKind, ItemStatus, RequestId, RequestKind, SessionEvent, TurnId, Usage,
    UserInput,
};

use crate::api::events::{
    BridgeApproval, BridgeChangeStatus, BridgeDecision, BridgeEdit, BridgeEditChange, BridgeEvent,
    BridgeEventKind, BridgeFileChange, BridgeItemKind, BridgeQuestionKind, BridgeQueue,
    BridgeRequestKind,
};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use crate::api::history::{BridgeEntryKind, BridgeRecordKind};
use crate::api::session::{
    AgentSession, BridgeApprovalMode, BridgeModel, BridgeStoredSession, SessionConfig,
};
use crate::convert::describe;
use crate::stream::{gather, push};

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

/// Starts a scripted stand-in for pi in `cwd`; `sh` takes pi's arguments as `$0` and `$@`.
fn launch(script: String, cwd: String) -> Result<AgentSession, String> {
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
fn requests_and_questions_reach_dart_with_their_answers() {
    let opened = bridge(AgentEvent::RequestOpened {
        request: RequestId("a1".to_owned()),
        kind: RequestKind::FileChangeApproval,
        tool: "write".to_owned(),
        arguments: "{}".to_owned(),
        text: "{}".to_owned(),
        read_only: false,
        session: None,
        item: Some(ItemId(7)),
        edit: Some(EditPreview {
            path: "a.ts".to_owned(),
            writes_to: Some("/work/a.ts".to_owned()),
            changes: vec![EditChange {
                removed: String::new(),
                added: "one".to_owned(),
            }],
            whole_file: true,
        }),
    });
    let request = opened.request.expect("request");
    assert_eq!(request.id, "a1");
    assert_eq!(
        request.approval,
        Some(BridgeApproval {
            kind: BridgeRequestKind::FileChange,
            tool: "write".to_owned(),
            text: "{}".to_owned(),
            item: Some(7),
            edit: Some(BridgeEdit {
                path: "a.ts".to_owned(),
                writes_to: Some("/work/a.ts".to_owned()),
                changes: vec![BridgeEditChange {
                    removed: String::new(),
                    added: "one".to_owned(),
                }],
                whole_file: true,
            }),
        })
    );
    let resolved = bridge(AgentEvent::RequestResolved {
        request: RequestId("a1".to_owned()),
    });
    assert_eq!(
        (
            resolved.kind,
            resolved.request.expect("request").id.as_str()
        ),
        (BridgeEventKind::RequestResolved, "a1")
    );
    assert_eq!(Decision::from(BridgeDecision::Reject), Decision::Reject);

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
    assert_eq!(
        call(session.set_approval_mode(BridgeApprovalMode::FullAccess)),
        Ok(())
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
fn stopping_withdraws_the_open_approval_the_agent_waits_for() {
    // pi acknowledges an abort once the agent is idle, which needs its open dialogs answered.
    let payload = r#"{\"tool\":\"bash\",\"arguments\":{},\"readOnly\":false}"#;
    let script = format!(
        r#"printf '%s\n' '{{"type":"extension_ui_request","id":"a1","method":"select","title":"watershed.approval {payload}","options":[]}}'
IFS= read -r line
case "$line" in *'"type":"abort"'*) ;; *) exit 9 ;; esac
IFS= read -r line
case "$line" in *'"id":"a1"'*) ;; *) exit 9 ;; esac
case "$line" in *'"cancelled":true'*) ;; *) exit 9 ;; esac
printf '%s\n' '{{"id":"2","type":"response","command":"abort","success":true}}'
IFS= read -r line || exit 0"#
    );
    let session = Arc::new(
        launch(gated(&script), std::env::temp_dir().display().to_string()).expect("agent starts"),
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
    assert_eq!(next().kind, BridgeEventKind::RequestOpened);
    let (stopped, stop) = mpsc::channel();
    {
        let session = Arc::clone(&session);
        std::thread::spawn(move || stopped.send(call(session.abort())));
    }
    assert_eq!(stop.recv_timeout(Duration::from_secs(10)), Ok(Ok(())));
    let resolved = next();
    assert_eq!(
        (resolved.kind, resolved.request.map(|request| request.id)),
        (BridgeEventKind::RequestResolved, Some("a1".to_owned()))
    );
    assert!(call(session.shutdown()));
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
        let mut batch = vec![BridgeEvent::from(delta("0"))];
        gather(&mut batch, &mut receiver, async {
            let _ = window.await;
        })
        .await;
        let text: Vec<_> = batch
            .iter()
            .filter_map(|event| event.text.as_deref())
            .collect();
        assert_eq!(
            text.concat(),
            "0123456789",
            "everything until the window passed"
        );
        assert_eq!(
            batch.len(),
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
        let mut batch = vec![BridgeEvent::from(delta("0"))];
        gather(&mut batch, &mut receiver, std::future::pending()).await;
        assert_eq!(batch.len(), 2);
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

#[test]
fn diffs_what_a_run_changed_in_its_project() {
    let repo = repository("diff");
    // The stand-in agent edits the project only after the prompt arrives, writing a character
    // that reorders the line.
    let script = [
        gate(2),
        r#"IFS= read -r line
printf 'new\342\200\256\n' > a.txt
printf '%s\n' '{"id":"3","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}'
"#
        .to_owned(),
        gate(4),
        r#"IFS= read -r line
printf '%s\n' '{"id":"5","type":"response","command":"prompt","success":false,"error":"busy"}'
"#
        .to_owned(),
        gate(6),
        r#"IFS= read -r line
printf '%s\n' '{"id":"7","type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}'
IFS= read -r line || exit 0"#
            .to_owned(),
    ]
    .concat();
    let session = launch(gated(&script), repo.display().to_string()).expect("agent starts");
    assert_eq!(session.changes(false), Ok(Some(Vec::new())), "no run yet");
    assert_eq!(session.send("edit".to_owned(), false), Ok(()));
    let changes = session.changes(false).expect("changes");
    assert_eq!(
        changes,
        Some(vec![BridgeFileChange {
            path: "a.txt".to_owned(),
            shown: "a.txt".to_owned(),
            status: BridgeChangeStatus::Modified,
            added: 1,
            removed: 1,
        }])
    );
    let patch = session.patch("a.txt".to_owned(), false).expect("patch");
    assert!(
        patch.contains("-old\n") && patch.contains("+new<U+202E>\n"),
        "{patch}"
    );
    // A prompt the agent rejects starts no run, so the last run's changes stay in view.
    assert!(session.send("again".to_owned(), false).is_err());
    assert_eq!(session.changes(false).expect("changes"), changes);
    // Nor does one that pi handles itself, such as a command.
    assert_eq!(session.send("/name tidy".to_owned(), false), Ok(()));
    assert_eq!(session.changes(false).expect("changes"), changes);
    assert!(call(session.shutdown()));
    assert_eq!(
        session.changes(false),
        Ok(Some(Vec::new())),
        "shutting down removes the snapshots"
    );
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn shuts_down_while_a_slow_project_is_captured() {
    let repo = repository("slow");
    // A clean filter runs while the project is captured; this one holds the capture up.
    git(&repo, &["config", "filter.slow.clean", "sleep 10; cat"]);
    std::fs::write(repo.join(".gitattributes"), "*.txt filter=slow\n").expect("attributes");
    std::fs::write(repo.join("a.txt"), "new\n").expect("file");
    let session = Arc::new(
        launch(
            gated("IFS= read -r line || exit 0"),
            repo.display().to_string(),
        )
        .expect("agent starts"),
    );
    {
        let session = Arc::clone(&session);
        std::thread::spawn(move || session.send("edit".to_owned(), false));
    }
    std::thread::sleep(Duration::from_millis(300));
    let (stopped, stop) = mpsc::channel();
    {
        let session = Arc::clone(&session);
        std::thread::spawn(move || stopped.send(call(session.shutdown())));
    }
    assert!(
        stop.recv_timeout(Duration::from_secs(5)).is_ok(),
        "the capture does not hold up shutdown"
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
        // The second read continues after the last entry; nothing was added.
        r#"IFS= read -r line
case "$line" in *'"since":"b1"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"4","type":"response","command":"get_entries","success":true,"data":{"entries":[],"leafId":"b1"}}'
"#
        .to_owned(),
        gate(5),
        r#"IFS= read -r line
case "$line" in *'"message":"/watershed-branch e1"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"6","type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}'
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
            "IFS= read -r line\nprintf '%s\\n' '{{\"id\":\"3\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{entries}}}'\nIFS= read -r line\nprintf '%s\\n' '{{\"id\":\"4\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{{\"entries\":[],\"leafId\":\"a2\"}}}}'\nIFS= read -r line || exit 0"
        );
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
            "IFS= read -r line\nprintf '%s\\n' '{{\"id\":\"3\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{entries}}}'\nIFS= read -r line || exit 0"
        );
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
    let last = std::iter::from_fn(|| receiver.recv_timeout(Duration::from_secs(5)).ok())
        .find(|event| event.text.as_deref() == Some("299"));
    assert!(last.is_some(), "every early event reaches Dart");
    assert!(call(session.shutdown()));
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
