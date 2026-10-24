use meta_harness_core::{
    CommandError, Decision, ItemDetail, ItemId, ItemKind, ItemStatus, RequestId, RequestKind,
    RuntimeEvent, SessionEvent, StepId, TurnId, Usage, UserInput,
};

use crate::api::events::{
    BridgeApproval, BridgeChangeStatus, BridgeDecision, BridgeEvent, BridgeEventKind,
    BridgeFileChange, BridgeItemKind, BridgeQuestionKind, BridgeRequestKind,
};
use std::sync::mpsc;
use std::time::Duration;

use crate::api::session::{AgentSession, BridgeApprovalMode, SessionConfig};
use crate::convert::{coalesce, describe};

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
    assert_eq!(session.prompt("Say hi".to_owned()), Ok(()));
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
        session.prompt("again".to_owned()),
        Err("the session was shut down".to_owned())
    );
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
    assert_eq!(session.prompt("edit".to_owned()), Ok(()));
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
