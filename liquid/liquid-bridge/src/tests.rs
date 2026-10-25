use meta_harness_core::{
    CommandError, Decision, ItemDetail, ItemId, ItemKind, ItemStatus, RequestId, RequestKind,
    RuntimeEvent, SessionEvent, StepId, TurnId, Usage, UserInput,
};

use crate::api::events::{
    BridgeApproval, BridgeChangeStatus, BridgeDecision, BridgeEvent, BridgeEventKind,
    BridgeFileChange, BridgeItemKind, BridgeQuestionKind, BridgeQueue, BridgeRequestKind,
};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use crate::api::history::{BridgeEntryKind, BridgeRecordKind};
use crate::api::session::{
    AgentSession, BridgeApprovalMode, BridgeModel, BridgeStoredSession, SessionConfig,
};
use crate::convert::{coalesce, describe};

/// Answers the `get_state` that starts a first history read, for a session pi keeps in memory.
fn no_session_file(id: u32) -> String {
    format!(
        "IFS= read -r line\ncase \"$line\" in *'\"type\":\"get_state\"'*) ;; *) exit 9 ;; esac\nprintf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_state\",\"success\":true,\"data\":{{\"sessionId\":\"s\",\"isStreaming\":false}}}}'\n"
    )
}

fn bridge(event: RuntimeEvent) -> BridgeEvent {
    SessionEvent::Runtime(event).into()
}

#[test]
fn tool_items_keep_their_call_and_outcome() {
    let started = bridge(RuntimeEvent::ItemStarted {
        turn: TurnId(2),
        item: ItemId(7),
        kind: ItemKind::CommandExecution,
        detail: ItemDetail::Tool {
            name: "bash".to_owned(),
            arguments: r#"{"command":"ls"}"#.to_owned(),
        },
    });
    assert_eq!(started.kind, BridgeEventKind::ItemStarted);
    assert_eq!(started.turn, Some(2));
    let item = started.item.expect("item");
    assert_eq!(
        (item.id, item.kind),
        (7, Some(BridgeItemKind::CommandExecution))
    );
    assert_eq!(item.tool.as_deref(), Some("bash"));
    assert_eq!(item.arguments.as_deref(), Some(r#"{"command":"ls"}"#));

    let completed = bridge(RuntimeEvent::ItemCompleted {
        turn: TurnId(2),
        item: ItemId(7),
        kind: ItemKind::CommandExecution,
        text: "a.txt".to_owned(),
        status: ItemStatus::Failed,
        duration_ms: Some(12),
    });
    let item = completed.item.expect("item");
    assert!(item.failed);
    assert_eq!(item.duration_ms, Some(12));
    assert_eq!(completed.text.as_deref(), Some("a.txt"));
}

#[test]
fn model_requests_carry_usage_and_timing() {
    let completed = bridge(RuntimeEvent::ModelRequestCompleted {
        turn: TurnId(1),
        step: StepId(3),
        model: "faux/faux-1".to_owned(),
        usage: Usage {
            input: 6140,
            output: 820,
            cache_read: 1,
            cache_write: 2,
            reasoning: 214,
            cost_micro_usd: 23457,
        },
        stop_reason: "toolUse".to_owned(),
        error: None,
        first_token_ms: Some(900),
        duration_ms: 3400,
    });
    let step = completed.step.expect("step");
    assert_eq!((step.id, step.model.as_str()), (3, "faux/faux-1"));
    assert_eq!(
        (step.usage.input, step.usage.output, step.usage.reasoning),
        (6140, 820, 214)
    );
    assert_eq!(step.usage.cost_micro_usd, 23457);
    assert_eq!((step.first_token_ms, step.duration_ms), (Some(900), 3400));
}

#[test]
fn requests_and_questions_reach_dart_with_their_answers() {
    let opened = bridge(RuntimeEvent::RequestOpened {
        request: RequestId("a1".to_owned()),
        kind: RequestKind::FileChangeApproval,
        tool: "write".to_owned(),
        arguments: "{}".to_owned(),
        read_only: false,
    });
    let request = opened.request.expect("request");
    assert_eq!(request.id, "a1");
    assert_eq!(
        request.approval,
        Some(BridgeApproval {
            kind: BridgeRequestKind::FileChange,
            tool: "write".to_owned(),
            arguments: "{}".to_owned(),
            read_only: false,
        })
    );
    let resolved = bridge(RuntimeEvent::RequestResolved {
        request: RequestId("a1".to_owned()),
        decision: Decision::AllowSession,
    });
    assert_eq!(
        resolved.request.expect("request").decision,
        Some(BridgeDecision::AllowSession)
    );
    assert_eq!(Decision::from(BridgeDecision::Reject), Decision::Reject);

    let asked = bridge(RuntimeEvent::UserInputRequested {
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
    let exited: BridgeEvent = SessionEvent::Exited { success: false }.into();
    assert_eq!(
        (exited.kind, exited.success),
        (BridgeEventKind::Exited, Some(false))
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
    let script = r#"IFS= read -r line
printf '%s\n' '{"id":"1","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}'
printf '%s\n' '{"type":"agent_start"}' '{"type":"agent_settled","aborted":false}'
IFS= read -r line || exit 0"#;
    let session = AgentSession::start(SessionConfig {
        program: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), script.to_owned()],
        cwd: std::env::temp_dir().display().to_string(),
    })
    .expect("agent starts");
    let (sender, receiver) = mpsc::channel();
    session
        .forward(move |event| sender.send(event).is_ok())
        .expect("first subscriber");
    assert_eq!(
        session.forward(|_| true),
        Err("the session's events already have a subscriber".to_owned())
    );
    assert_eq!(
        session.set_approval_mode(BridgeApprovalMode::FullAccess),
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
    assert!(session.shutdown());
    assert_eq!(next(), BridgeEventKind::Exited);
    assert_eq!(
        session.send("again".to_owned(), false),
        Err("the session was shut down".to_owned())
    );
}

#[test]
fn answers_a_question_the_agent_asks_before_taking_a_message() {
    // pi accepts a message only after its command handlers ran, and they may ask first.
    let script = r#"IFS= read -r line
printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"input","title":"Name","placeholder":""}'
IFS= read -r line
case "$line" in *'"id":"q1"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"1","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}'
IFS= read -r line || exit 0"#;
    let session = Arc::new(
        AgentSession::start(SessionConfig {
            program: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script.to_owned()],
            cwd: std::env::temp_dir().display().to_string(),
        })
        .expect("agent starts"),
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
            answered.send(session.answer_text("q1".to_owned(), "Ada".to_owned()))
        });
    }
    assert_eq!(answer.recv_timeout(Duration::from_secs(10)), Ok(Ok(())));
    assert_eq!(sending.join().expect("send returns"), Ok(()));
    assert!(session.shutdown());
}

#[test]
fn stopping_rejects_the_open_approval_the_agent_waits_for() {
    // pi acknowledges an abort once the agent is idle, which needs its open dialogs answered.
    let payload = r#"{\"tool\":\"bash\",\"arguments\":{},\"readOnly\":false}"#;
    let script = format!(
        r#"printf '%s\n' '{{"type":"extension_ui_request","id":"a1","method":"select","title":"watershed.approval {payload}","options":[]}}'
IFS= read -r line
case "$line" in *'"type":"abort"'*) ;; *) exit 9 ;; esac
IFS= read -r line
case "$line" in *'"id":"a1"'*) ;; *) exit 9 ;; esac
case "$line" in *'"value":"reject"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{{"id":"1","type":"response","command":"abort","success":true}}'
IFS= read -r line || exit 0"#
    );
    let session = Arc::new(
        AgentSession::start(SessionConfig {
            program: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script],
            cwd: std::env::temp_dir().display().to_string(),
        })
        .expect("agent starts"),
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
        std::thread::spawn(move || stopped.send(session.abort()));
    }
    assert_eq!(stop.recv_timeout(Duration::from_secs(10)), Ok(Ok(())));
    let resolved = next();
    assert_eq!(
        (
            resolved.kind,
            resolved.request.and_then(|request| request.decision)
        ),
        (
            BridgeEventKind::RequestResolved,
            Some(BridgeDecision::Reject)
        )
    );
    assert!(session.shutdown());
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
        format!(
            "IFS= read -r line\ncase \"$line\" in *'\"type\":\"{command}\"'*) ;; *) exit 9 ;; esac\nprintf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"{command}\",\"success\":true,\"data\":{data}}}'\n"
        )
    };
    let state = format!(
        r#"{{"sessionId":"s","isStreaming":false,"sessionFile":"{}","model":{{"provider":"faux","id":"faux-1"}}}}"#,
        current.display()
    );
    let script = [
        respond(1, "get_state", &state),
        respond(2, "new_session", r#"{"cancelled":false}"#),
        respond(3, "switch_session", r#"{"cancelled":false}"#),
        respond(
            4,
            "get_available_models",
            r#"{"models":[{"provider":"faux","id":"faux-1","name":"Faux One"}]}"#,
        ),
        respond(5, "get_state", &state),
        respond(6, "set_model", "{}"),
        "IFS= read -r line || exit 0".to_owned(),
    ]
    .concat();
    let session = AgentSession::start(SessionConfig {
        program: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), script],
        cwd: project.display().to_string(),
    })
    .expect("agent starts");
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
        }])
    );
    assert_eq!(session.current_model(), Ok(Some("faux/faux-1".to_owned())));
    assert_eq!(
        session.set_model("faux".to_owned(), "faux-1".to_owned()),
        Ok(())
    );
    assert!(session.shutdown());
}

#[test]
fn answers_a_question_the_agent_asks_while_selecting_a_model() {
    // pi runs extensions' model selection handlers before it answers, and they may ask first.
    let script = r#"IFS= read -r line
case "$line" in *'"type":"set_model"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"confirm","title":"Switch?","message":"Costs more"}'
IFS= read -r line
case "$line" in *'"id":"q1"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"1","type":"response","command":"set_model","success":true,"data":{}}'
IFS= read -r line || exit 0"#;
    let session = Arc::new(
        AgentSession::start(SessionConfig {
            program: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script.to_owned()],
            cwd: std::env::temp_dir().display().to_string(),
        })
        .expect("agent starts"),
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
        std::thread::spawn(move || answered.send(session.answer_confirm("q1".to_owned(), true)));
    }
    assert_eq!(answer.recv_timeout(Duration::from_secs(10)), Ok(Ok(())));
    assert_eq!(selecting.join().expect("selection returns"), Ok(()));
    assert!(session.shutdown());
}

#[test]
fn coalesces_consecutive_deltas_of_one_item() {
    let delta = |item: u64, text: &str| {
        bridge(RuntimeEvent::ContentDelta {
            item: ItemId(item),
            delta: text.to_owned(),
        })
    };
    let merged = coalesce(vec![
        delta(1, "He"),
        delta(1, "llo"),
        delta(2, "Hi"),
        bridge(RuntimeEvent::TurnCompleted { turn: TurnId(1) }),
        delta(2, "!"),
    ]);
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
fn diffs_what_a_run_changed_in_its_project() {
    let repo = std::env::temp_dir().join(format!("liquid-bridge-diff-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).expect("repository");
    let git = |args: &[&str]| {
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
            .current_dir(&repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("a.txt"), "old\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    // The stand-in agent edits the project only after the prompt arrives.
    let script = r#"IFS= read -r line
printf 'new\n' > a.txt
printf '%s\n' '{"id":"1","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}'
IFS= read -r line || exit 0"#;
    let session = AgentSession::start(SessionConfig {
        program: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), script.to_owned()],
        cwd: repo.display().to_string(),
    })
    .expect("agent starts");
    assert_eq!(session.changes(), Ok(Vec::new()), "no run yet");
    assert_eq!(session.send("edit".to_owned(), false), Ok(()));
    let changes = session.changes().expect("changes");
    assert_eq!(
        changes,
        [BridgeFileChange {
            path: "a.txt".to_owned(),
            status: BridgeChangeStatus::Modified,
            added: 1,
            removed: 1,
        }]
    );
    let patch = session.patch("a.txt".to_owned()).expect("patch");
    assert!(
        patch.contains("-old\n") && patch.contains("+new\n"),
        "{patch}"
    );
    assert!(session.shutdown());
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
        no_session_file(1),
        respond("2"),
        // The second read continues after the last entry; nothing was added.
        r#"IFS= read -r line
case "$line" in *'"since":"b1"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"3","type":"response","command":"get_entries","success":true,"data":{"entries":[],"leafId":"b1"}}'
"#
        .to_owned(),
        r#"IFS= read -r line
case "$line" in *'"message":"/watershed-branch e1"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"4","type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}'
IFS= read -r line || exit 0"#
            .to_owned(),
    ]
    .concat();
    let session = AgentSession::start(SessionConfig {
        program: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), script],
        cwd: std::env::temp_dir().display().to_string(),
    })
    .expect("agent starts");

    let current = session.history(Vec::new()).expect("history");
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
    assert!(session.shutdown());
}

#[test]
fn reads_tool_outcomes_and_what_the_model_saw() {
    let entries = r#"{"entries":[{"type":"message","id":"s1","parentId":null,"message":{"role":"system","content":"","sections":{"preamble":"You are pi."},"toolsAdded":[{"name":"write"}]}},{"type":"message","id":"u1","parentId":"s1","message":{"role":"user","content":"Write it"}},{"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"write","arguments":{"path":"a.ts","content":"x\ny\n"}}]}},{"type":"message","id":"r1","parentId":"a1","message":{"role":"toolResult","toolCallId":"c1","toolName":"write","content":[{"type":"text","text":"ok"}]}},{"type":"message","id":"a2","parentId":"r1","message":{"role":"assistant","content":[{"type":"text","text":"Done."}]}}],"leafId":"a2"}"#;
    let script = no_session_file(1)
        + &format!(
            "IFS= read -r line\nprintf '%s\\n' '{{\"id\":\"2\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{entries}}}'\nIFS= read -r line\nprintf '%s\\n' '{{\"id\":\"3\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{{\"entries\":[],\"leafId\":\"a2\"}}}}'\nIFS= read -r line || exit 0"
        );
    let session = AgentSession::start(SessionConfig {
        program: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), script],
        cwd: std::env::temp_dir().display().to_string(),
    })
    .expect("agent starts");
    let history = session.history(Vec::new()).expect("history");
    let records = &history.trajectories[0].records;
    assert_eq!(records[2].kind, BridgeRecordKind::Tool);
    assert_eq!(records[2].outcome.added, Some(2), "lines the write wrote");
    let context = records[3].context.as_ref().expect("a2 saw context");
    assert_eq!(
        (
            context.prompt_chars,
            context.tool_chars > 0,
            context.messages
        ),
        (11, true, 3)
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
    assert!(session.shutdown());
}

#[test]
fn rebuilds_the_chat_of_the_current_branch() {
    let entries = r#"{"entries":[{"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"Fix it"}},{"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[{"type":"text","text":"Editing."},{"type":"toolCall","id":"c1","name":"edit","arguments":{"path":"a.ts"}}]}},{"type":"message","id":"r1","parentId":"a1","message":{"role":"toolResult","toolCallId":"c1","toolName":"edit","content":[]}}],"leafId":"r1"}"#;
    let script = no_session_file(1)
        + &format!(
            "IFS= read -r line\nprintf '%s\\n' '{{\"id\":\"2\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{entries}}}'\nIFS= read -r line || exit 0"
        );
    let session = AgentSession::start(SessionConfig {
        program: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), script],
        cwd: std::env::temp_dir().display().to_string(),
    })
    .expect("agent starts");
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
    assert!(session.shutdown());
}

#[test]
fn passes_the_waiting_messages_to_dart() {
    let changed = bridge(RuntimeEvent::QueueChanged(meta_harness_core::Queue {
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
