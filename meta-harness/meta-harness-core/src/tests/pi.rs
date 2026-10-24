use crate::{AdapterError, ItemId, ItemKind, PiAdapter, RuntimeEvent, TurnId};

fn translate(records: &str) -> Vec<RuntimeEvent> {
    let mut adapter = PiAdapter::new();
    records
        .lines()
        .flat_map(|record| {
            adapter
                .translate(record.as_bytes())
                .expect("valid pi record")
        })
        .collect()
}

#[test]
fn streams_a_turn_with_reasoning_and_text() {
    let events = translate(concat!(
        r#"{"type":"agent_start"}"#,
        "\n",
        r#"{"type":"turn_start"}"#,
        "\n",
        r#"{"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"Say hi"}]}}"#,
        "\n",
        r#"{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"Say hi"}]}}"#,
        "\n",
        r#"{"type":"message_start","message":{"role":"assistant","content":[]}}"#,
        "\n",
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_start","contentIndex":0}}"#,
        "\n",
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"The user "}}"#,
        "\n",
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"wants hi."}}"#,
        "\n",
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_end","contentIndex":0,"content":"The user wants hi."}}"#,
        "\n",
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":1}}"#,
        "\n",
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"Hi"}}"#,
        "\n",
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":" there"}}"#,
        "\n",
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_end","contentIndex":1,"content":"Hi there"}}"#,
        "\n",
        r#"{"type":"message_end","message":{"role":"assistant","content":[]}}"#,
        "\n",
        r#"{"type":"turn_end","message":{"role":"assistant","content":[]},"toolResults":[]}"#,
        "\n",
        r#"{"type":"agent_end","messages":[],"willRetry":false}"#,
        "\n",
        r#"{"type":"agent_settled","aborted":false}"#,
    ));
    let turn = TurnId(1);
    let (user, reasoning, reply) = (ItemId(1), ItemId(2), ItemId(3));
    let delta = |item, text: &str| RuntimeEvent::ContentDelta {
        item,
        delta: text.to_owned(),
    };
    let started = |item, kind| RuntimeEvent::ItemStarted { turn, item, kind };
    let completed = |item, kind, text: &str| RuntimeEvent::ItemCompleted {
        turn,
        item,
        kind,
        text: text.to_owned(),
    };
    assert_eq!(
        events,
        [
            RuntimeEvent::TurnStarted { turn },
            started(user, ItemKind::UserMessage),
            completed(user, ItemKind::UserMessage, "Say hi"),
            started(reasoning, ItemKind::Reasoning),
            delta(reasoning, "The user "),
            delta(reasoning, "wants hi."),
            completed(reasoning, ItemKind::Reasoning, "The user wants hi."),
            started(reply, ItemKind::AssistantMessage),
            delta(reply, "Hi"),
            delta(reply, " there"),
            completed(reply, ItemKind::AssistantMessage, "Hi there"),
            RuntimeEvent::TurnCompleted { turn },
        ]
    );
}

#[test]
fn keeps_retries_in_one_turn_and_reports_aborts() {
    let events = translate(concat!(
        r#"{"type":"agent_start"}"#,
        "\n",
        r#"{"type":"agent_end","messages":[],"willRetry":true}"#,
        "\n",
        r#"{"type":"auto_retry_start","attempt":1,"maxAttempts":3,"delayMs":2000,"errorMessage":"529 overloaded"}"#,
        "\n",
        r#"{"type":"agent_start"}"#,
        "\n",
        r#"{"type":"agent_end","messages":[],"willRetry":false}"#,
        "\n",
        r#"{"type":"agent_settled","aborted":true}"#,
        "\n",
        r#"{"type":"agent_start"}"#,
        "\n",
        r#"{"type":"message_end","message":{"role":"user","content":"Plain string prompt"}}"#,
    ));
    let (first, second) = (TurnId(1), TurnId(2));
    assert_eq!(
        events,
        [
            RuntimeEvent::TurnStarted { turn: first },
            RuntimeEvent::TurnAborted { turn: first },
            RuntimeEvent::TurnStarted { turn: second },
            RuntimeEvent::ItemStarted {
                turn: second,
                item: ItemId(1),
                kind: ItemKind::UserMessage
            },
            RuntimeEvent::ItemCompleted {
                turn: second,
                item: ItemId(1),
                kind: ItemKind::UserMessage,
                text: "Plain string prompt".to_owned(),
            },
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
