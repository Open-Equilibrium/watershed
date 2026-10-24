use std::time::Duration;

use serde_json::{Map, Value};

use crate::pi::session_tree;
use crate::{
    ContextSection, EntryKind, ItemDetail, ItemId, ItemKind, ItemStatus, NotificationLevel,
    PiAdapter, RequestId, RequestKind, RuntimeEvent, SessionTree, StepId, TreeNode, TurnId, Usage,
    UserInput,
};

const TURN: TurnId = TurnId(1);

fn translate(records: &[&str]) -> Vec<RuntimeEvent> {
    let mut adapter = PiAdapter::new();
    records
        .iter()
        .flat_map(|record| {
            let record: Map<String, Value> =
                serde_json::from_str(record).expect("fixture records are JSON objects");
            adapter.translate(&record, Duration::ZERO)
        })
        .collect()
}

fn started(item: u64, kind: ItemKind) -> RuntimeEvent {
    RuntimeEvent::ItemStarted {
        turn: TURN,
        item: ItemId(item),
        kind,
        detail: ItemDetail::None,
    }
}

fn completed(item: u64, kind: ItemKind, text: &str) -> RuntimeEvent {
    RuntimeEvent::ItemCompleted {
        turn: TURN,
        item: ItemId(item),
        kind,
        text: text.to_owned(),
        status: ItemStatus::Completed,
        duration_ms: None,
    }
}

fn delta(item: u64, text: &str) -> RuntimeEvent {
    RuntimeEvent::ContentDelta {
        item: ItemId(item),
        delta: text.to_owned(),
    }
}

#[test]
fn streams_a_turn_with_reasoning_and_text() {
    let events = translate(&[
        r#"{"type":"agent_start"}"#,
        r#"{"type":"turn_start"}"#,
        r#"{"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"Say hi"}]}}"#,
        r#"{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"Say hi"}]}}"#,
        r#"{"type":"message_start","message":{"role":"assistant","content":[]}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_start","contentIndex":0}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"The user "}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"wants hi."}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_end","contentIndex":0,"content":"The user wants hi."}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":1}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"Hi"}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":" there"}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_end","contentIndex":1,"content":"Hi there"}}"#,
        r#"{"type":"message_end","message":{"role":"assistant","content":[]}}"#,
        r#"{"type":"turn_end","message":{"role":"assistant","content":[]},"toolResults":[]}"#,
        r#"{"type":"agent_end","messages":[],"willRetry":false}"#,
        r#"{"type":"agent_settled","aborted":false}"#,
    ]);
    assert_eq!(
        events,
        [
            RuntimeEvent::TurnStarted { turn: TURN },
            started(1, ItemKind::UserMessage),
            completed(1, ItemKind::UserMessage, "Say hi"),
            RuntimeEvent::ModelRequestStarted {
                turn: TURN,
                step: StepId(1),
            },
            started(2, ItemKind::Reasoning),
            delta(2, "The user "),
            delta(2, "wants hi."),
            completed(2, ItemKind::Reasoning, "The user wants hi."),
            started(3, ItemKind::AssistantMessage),
            delta(3, "Hi"),
            delta(3, " there"),
            completed(3, ItemKind::AssistantMessage, "Hi there"),
            RuntimeEvent::ModelRequestCompleted {
                turn: TURN,
                step: StepId(1),
                model: String::new(),
                usage: Usage::default(),
                stop_reason: String::new(),
                error: None,
                first_token_ms: Some(0),
                duration_ms: 0,
            },
            RuntimeEvent::TurnCompleted { turn: TURN },
        ]
    );
}

#[test]
fn keeps_retries_in_one_turn_and_reports_aborts() {
    let events = translate(&[
        r#"{"type":"agent_start"}"#,
        r#"{"type":"agent_end","messages":[],"willRetry":true}"#,
        r#"{"type":"auto_retry_start","attempt":1,"maxAttempts":3,"delayMs":2000,"errorMessage":"529 overloaded"}"#,
        r#"{"type":"agent_start"}"#,
        r#"{"type":"agent_end","messages":[],"willRetry":false}"#,
        r#"{"type":"agent_settled","aborted":true}"#,
        r#"{"type":"agent_start"}"#,
        r#"{"type":"message_end","message":{"role":"user","content":"Plain string prompt"}}"#,
    ]);
    let second = TurnId(2);
    assert_eq!(
        events,
        [
            RuntimeEvent::TurnStarted { turn: TURN },
            RuntimeEvent::TurnAborted { turn: TURN },
            RuntimeEvent::TurnStarted { turn: second },
            RuntimeEvent::ItemStarted {
                turn: second,
                item: ItemId(1),
                kind: ItemKind::UserMessage,
                detail: ItemDetail::None,
            },
            RuntimeEvent::ItemCompleted {
                turn: second,
                item: ItemId(1),
                kind: ItemKind::UserMessage,
                text: "Plain string prompt".to_owned(),
                status: ItemStatus::Completed,
                duration_ms: None,
            },
        ]
    );
}

#[test]
fn classifies_tool_calls_and_reports_their_outcome() {
    let events = translate(&[
        r#"{"type":"agent_start"}"#,
        r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{"command":"ls"}}"#,
        r#"{"type":"tool_execution_update","toolCallId":"c1","toolName":"bash","args":{"command":"ls"},"partialResult":{"content":[{"type":"text","text":"a.txt"}],"details":{}}}"#,
        r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[{"type":"text","text":"a.txt\nb.txt"}],"details":{}},"isError":false,"durationMs":12}"#,
        r#"{"type":"tool_execution_start","toolCallId":"c2","toolName":"edit","args":{"path":"a.txt"}}"#,
        r#"{"type":"tool_execution_end","toolCallId":"c2","toolName":"edit","result":{"content":[{"type":"text","text":"No match"}]},"isError":true,"durationMs":3}"#,
        r#"{"type":"tool_execution_start","toolCallId":"c3","toolName":"grep","args":{"pattern":"x"}}"#,
        r#"{"type":"tool_execution_end","toolCallId":"c3","toolName":"grep","result":{"content":[]},"isError":false}"#,
    ]);
    let tool = |item, kind, name: &str, arguments: &str| RuntimeEvent::ItemStarted {
        turn: TURN,
        item: ItemId(item),
        kind,
        detail: ItemDetail::Tool {
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        },
    };
    let finished = |item, kind, text: &str, status, duration_ms| RuntimeEvent::ItemCompleted {
        turn: TURN,
        item: ItemId(item),
        kind,
        text: text.to_owned(),
        status,
        duration_ms,
    };
    use ItemKind::{CommandExecution, DynamicToolCall, FileChange};
    assert_eq!(
        events,
        [
            RuntimeEvent::TurnStarted { turn: TURN },
            tool(1, CommandExecution, "bash", r#"{"command":"ls"}"#),
            RuntimeEvent::ItemUpdated {
                item: ItemId(1),
                output: "a.txt".to_owned(),
            },
            finished(
                1,
                CommandExecution,
                "a.txt\nb.txt",
                ItemStatus::Completed,
                Some(12)
            ),
            tool(2, FileChange, "edit", r#"{"path":"a.txt"}"#),
            finished(2, FileChange, "No match", ItemStatus::Failed, Some(3)),
            tool(3, DynamicToolCall, "grep", r#"{"pattern":"x"}"#),
            finished(3, DynamicToolCall, "", ItemStatus::Completed, None),
        ]
    );
}

#[test]
fn translates_extension_questions_and_display_updates() {
    let events = translate(&[
        r#"{"type":"extension_ui_request","id":"q1","method":"select","title":"Pick one","options":["A","B"],"timeout":5000}"#,
        r#"{"type":"extension_ui_request","id":"q2","method":"confirm","title":"Sure?","message":"This cannot be undone."}"#,
        r#"{"type":"extension_ui_request","id":"q3","method":"input","title":"Name","placeholder":"type a name"}"#,
        r#"{"type":"extension_ui_request","id":"q4","method":"editor","title":"Edit","prefill":"a\nb"}"#,
        r#"{"type":"extension_ui_request","id":"n1","method":"notify","message":"Blocked","notifyType":"warning"}"#,
        r#"{"type":"extension_ui_request","id":"n2","method":"notify","message":"Saved"}"#,
        r#"{"type":"extension_ui_request","id":"s1","method":"setStatus","statusKey":"todo","statusText":"2/3"}"#,
        r#"{"type":"extension_ui_request","id":"s2","method":"setStatus","statusKey":"todo"}"#,
        r#"{"type":"extension_ui_request","id":"w1","method":"setWidget","widgetKey":"todo","widgetLines":["one","two"]}"#,
        r#"{"type":"extension_ui_request","id":"w2","method":"setWidget","widgetKey":"todo"}"#,
        r#"{"type":"extension_ui_request","id":"t1","method":"setTitle","title":"pi - acme"}"#,
        r#"{"type":"extension_ui_request","id":"e1","method":"set_editor_text","text":"draft"}"#,
    ]);
    let request = |id: &str| RequestId(id.to_owned());
    let input = |id, input| RuntimeEvent::UserInputRequested {
        request: request(id),
        input,
    };
    assert_eq!(
        events,
        [
            input(
                "q1",
                UserInput::Select {
                    title: "Pick one".to_owned(),
                    options: vec!["A".to_owned(), "B".to_owned()],
                }
            ),
            input(
                "q2",
                UserInput::Confirm {
                    title: "Sure?".to_owned(),
                    message: "This cannot be undone.".to_owned(),
                }
            ),
            input(
                "q3",
                UserInput::Input {
                    title: "Name".to_owned(),
                    placeholder: "type a name".to_owned(),
                }
            ),
            input(
                "q4",
                UserInput::Editor {
                    title: "Edit".to_owned(),
                    prefill: "a\nb".to_owned(),
                }
            ),
            RuntimeEvent::Notification {
                level: NotificationLevel::Warning,
                message: "Blocked".to_owned(),
            },
            RuntimeEvent::Notification {
                level: NotificationLevel::Info,
                message: "Saved".to_owned(),
            },
            RuntimeEvent::StatusChanged {
                key: "todo".to_owned(),
                text: Some("2/3".to_owned()),
            },
            RuntimeEvent::StatusChanged {
                key: "todo".to_owned(),
                text: None,
            },
            RuntimeEvent::WidgetChanged {
                key: "todo".to_owned(),
                lines: Some(vec!["one".to_owned(), "two".to_owned()]),
            },
            RuntimeEvent::WidgetChanged {
                key: "todo".to_owned(),
                lines: None,
            },
            RuntimeEvent::TitleChanged {
                title: "pi - acme".to_owned(),
            },
            RuntimeEvent::InputTextSet {
                text: "draft".to_owned(),
            },
        ]
    );
}

#[test]
fn turns_watershed_approval_questions_into_requests() {
    let events = translate(&[
        r#"{"type":"extension_ui_request","id":"a1","method":"select","title":"watershed.approval {\"tool\":\"bash\",\"arguments\":{\"command\":\"npm test\"},\"readOnly\":false}","options":["allow-once","allow-session","reject","allow-auto"]}"#,
        r#"{"type":"extension_ui_request","id":"a2","method":"select","title":"watershed.approval {\"tool\":\"write\",\"arguments\":{},\"readOnly\":false}","options":[]}"#,
        r#"{"type":"extension_ui_request","id":"a3","method":"select","title":"watershed.approval {\"tool\":\"read\",\"arguments\":{\"path\":\"a\"},\"readOnly\":true}","options":[]}"#,
    ]);
    let opened =
        |id: &str, kind, tool: &str, arguments: &str, read_only| RuntimeEvent::RequestOpened {
            request: RequestId(id.to_owned()),
            kind,
            tool: tool.to_owned(),
            arguments: arguments.to_owned(),
            read_only,
        };
    assert_eq!(
        events,
        [
            opened(
                "a1",
                RequestKind::CommandExecutionApproval,
                "bash",
                r#"{"command":"npm test"}"#,
                false
            ),
            opened("a2", RequestKind::FileChangeApproval, "write", "{}", false),
            opened(
                "a3",
                RequestKind::ToolApproval,
                "read",
                r#"{"path":"a"}"#,
                true
            ),
        ]
    );
}

#[test]
fn measures_model_requests_and_context_changes() {
    let mut adapter = PiAdapter::new();
    let records = [
        (0, r#"{"type":"agent_start"}"#),
        (
            10,
            r#"{"type":"message_end","message":{"role":"system","content":"Also be kind.","sections":{"preamble":"You are pi.","rules":"Be brief.","docs":null},"toolsAdded":[{"name":"read"},{"name":"bash"}],"toolsRemoved":[{"name":"grep"}]}}"#,
        ),
        (
            100,
            r#"{"type":"message_start","message":{"role":"assistant","content":[],"model":"faux-1","stopReason":"pending"}}"#,
        ),
        (
            400,
            r#"{"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":0}}"#,
        ),
        (
            450,
            r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hi"}}"#,
        ),
        (
            1100,
            r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Hi"}],"model":"faux-1","stopReason":"stop","usage":{"input":6140,"output":820,"cacheRead":100,"cacheWrite":5,"reasoning":214,"totalTokens":6960,"cost":{"input":0.01,"output":0.008,"cacheRead":0,"cacheWrite":0,"total":0.0234567}}}}"#,
        ),
        (
            1200,
            r#"{"type":"message_start","message":{"role":"assistant","content":[],"model":"faux-1"}}"#,
        ),
        (
            1300,
            r#"{"type":"message_end","message":{"role":"assistant","content":[],"model":"faux-1","stopReason":"error","errorMessage":"529 overloaded","usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"total":0}}}}"#,
        ),
    ];
    let events: Vec<RuntimeEvent> = records
        .iter()
        .flat_map(|(at, record)| {
            let record: Map<String, Value> = serde_json::from_str(record).expect("JSON object");
            adapter.translate(&record, Duration::from_millis(*at))
        })
        .filter(|event| {
            matches!(
                event,
                RuntimeEvent::ContextChanged { .. }
                    | RuntimeEvent::ModelRequestStarted { .. }
                    | RuntimeEvent::ModelRequestCompleted { .. }
            )
        })
        .collect();
    let section = |name: &str, chars| ContextSection {
        name: name.to_owned(),
        chars,
    };
    assert_eq!(
        events,
        [
            RuntimeEvent::ContextChanged {
                turn: TURN,
                sections: vec![
                    section("docs", None),
                    section("preamble", Some(11)),
                    section("rules", Some(9)),
                ],
                appended_chars: 13,
                tools_added: vec!["read".to_owned(), "bash".to_owned()],
                tools_removed: vec!["grep".to_owned()],
            },
            RuntimeEvent::ModelRequestStarted {
                turn: TURN,
                step: StepId(1),
            },
            RuntimeEvent::ModelRequestCompleted {
                turn: TURN,
                step: StepId(1),
                model: "faux-1".to_owned(),
                usage: Usage {
                    input: 6140,
                    output: 820,
                    cache_read: 100,
                    cache_write: 5,
                    reasoning: 214,
                    cost_micro_usd: 23457,
                },
                stop_reason: "stop".to_owned(),
                error: None,
                first_token_ms: Some(300),
                duration_ms: 1000,
            },
            RuntimeEvent::ModelRequestStarted {
                turn: TURN,
                step: StepId(2),
            },
            RuntimeEvent::ModelRequestCompleted {
                turn: TURN,
                step: StepId(2),
                model: "faux-1".to_owned(),
                usage: Usage::default(),
                stop_reason: "error".to_owned(),
                error: Some("529 overloaded".to_owned()),
                first_token_ms: None,
                duration_ms: 100,
            },
        ]
    );
}

#[test]
fn builds_the_session_tree_from_flat_entries() {
    let data: Value = serde_json::from_str(
        r#"{"entries":[
          {"type":"message","id":"e1","parentId":null,"message":{"role":"user","content":"Add rate limiting to /login\nand tests"}},
          {"type":"message","id":"e2","parentId":"e1","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"I'll look first."}]}},
          {"type":"message","id":"e3","parentId":"e2","message":{"role":"toolResult","toolName":"read","content":[]}},
          {"type":"label","id":"l1","parentId":"e2","targetId":"e2","label":"A"},
          {"type":"custom","id":"e4","parentId":"l1","customType":"watershed.approval"},
          {"type":"label","id":"l2","parentId":"e4","targetId":"e2","label":"A2"},
          {"type":"model_change","id":"e5","parentId":"l2"}],
          "leafId":"l2"}"#,
    )
    .expect("JSON");
    let node =
        |id: &str, parent: Option<&str>, kind, preview: &str, label: Option<&str>| TreeNode {
            id: id.to_owned(),
            parent: parent.map(str::to_owned),
            kind,
            preview: preview.to_owned(),
            label: label.map(str::to_owned),
        };
    assert_eq!(
        session_tree(&data),
        SessionTree {
            nodes: vec![
                node(
                    "e1",
                    None,
                    EntryKind::User,
                    "Add rate limiting to /login",
                    None
                ),
                node(
                    "e2",
                    Some("e1"),
                    EntryKind::Assistant,
                    "I'll look first.",
                    Some("A2")
                ),
                node("e3", Some("e2"), EntryKind::ToolResult, "read", None),
                node(
                    "e4",
                    Some("e2"),
                    EntryKind::Custom("watershed.approval".to_owned()),
                    "",
                    None
                ),
                node(
                    "e5",
                    Some("e4"),
                    EntryKind::Other("model_change".to_owned()),
                    "",
                    None
                ),
            ],
            leaf: Some("e4".to_owned()),
        }
    );
}
