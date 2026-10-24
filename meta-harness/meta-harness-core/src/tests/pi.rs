use crate::{
    AdapterError, ItemDetail, ItemId, ItemKind, ItemStatus, PiAdapter, RuntimeEvent, TurnId,
};

const TURN: TurnId = TurnId(1);

fn translate(records: &[&str]) -> Vec<RuntimeEvent> {
    let mut adapter = PiAdapter::new();
    records
        .iter()
        .flat_map(|record| {
            adapter
                .translate(record.as_bytes())
                .expect("valid pi record")
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
fn rejects_records_that_are_not_json_objects() {
    let mut adapter = PiAdapter::new();
    for record in [&b"not json"[..], b"[1]"] {
        assert_eq!(adapter.translate(record), Err(AdapterError::InvalidRecord));
    }
    assert_eq!(
        AdapterError::InvalidRecord.to_string(),
        "pi record is not a JSON object"
    );
}
