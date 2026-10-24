use serde_json::{Map, Value};

use crate::{
    ItemDetail, ItemId, ItemKind, ItemStatus, NotificationLevel, PiAdapter, RequestId, RequestKind,
    RuntimeEvent, TurnId, UserInput,
};

const TURN: TurnId = TurnId(1);

fn translate(records: &[&str]) -> Vec<RuntimeEvent> {
    let mut adapter = PiAdapter::new();
    records
        .iter()
        .flat_map(|record| {
            let record: Map<String, Value> =
                serde_json::from_str(record).expect("fixture records are JSON objects");
            adapter.translate(&record)
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
            started(2, ItemKind::Reasoning),
            delta(2, "The user "),
            delta(2, "wants hi."),
            completed(2, ItemKind::Reasoning, "The user wants hi."),
            started(3, ItemKind::AssistantMessage),
            delta(3, "Hi"),
            delta(3, " there"),
            completed(3, ItemKind::AssistantMessage, "Hi there"),
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
