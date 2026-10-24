use serde_json::Value;

use crate::{History, ItemKind, Transcript, TranscriptItem};

/// A branch with a prompt, a response that thinks and runs a command, the failed result and a
/// final answer; `b1` forks after the prompt.
const SESSION: &str = r#"{"entries":[
  {"type":"model_change","id":"s0","parentId":null},
  {"type":"message","id":"u1","parentId":"s0","message":{"role":"user","content":"Run the tests"}},
  {"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Plan."},{"type":"text","text":"Running them."},{"type":"toolCall","id":"c1","name":"bash","arguments":{"command":"npm test"}}]}},
  {"type":"custom","id":"p1","parentId":"a1","customType":"watershed.approval","data":{"toolCallId":"c1","tool":"bash","decision":"allow-once","waitedMs":10}},
  {"type":"custom","id":"t1","parentId":"p1","customType":"watershed.timing","data":{"kind":"tool","toolCallId":"c1","durationMs":900}},
  {"type":"message","id":"r1","parentId":"t1","message":{"role":"toolResult","toolCallId":"c1","toolName":"bash","isError":true,"content":[{"type":"text","text":"1 failed"}]}},
  {"type":"message","id":"a2","parentId":"r1","message":{"role":"assistant","content":[{"type":"text","text":"One test fails."}]}},
  {"type":"message","id":"b1","parentId":"u1","message":{"role":"user","content":"Only lint"}}
],"leafId":"a2"}"#;

fn item(entry: &str, kind: ItemKind, text: &str) -> TranscriptItem {
    TranscriptItem {
        entry: entry.to_owned(),
        kind,
        text: text.to_owned(),
        tool: None,
        arguments: None,
        failed: false,
        duration_ms: None,
    }
}

#[test]
fn rebuilds_the_chat_of_the_current_branch_page_by_page() {
    let history = History::from_entries(&serde_json::from_str::<Value>(SESSION).expect("JSON"));
    let command = TranscriptItem {
        tool: Some("bash".to_owned()),
        arguments: Some(r#"{"command":"npm test"}"#.to_owned()),
        failed: true,
        duration_ms: Some(900),
        ..item("a1", ItemKind::CommandExecution, "")
    };
    assert_eq!(
        history.transcript(None, 100),
        Transcript {
            items: vec![
                item("u1", ItemKind::UserMessage, "Run the tests"),
                item("a1", ItemKind::Reasoning, ""),
                item("a1", ItemKind::AssistantMessage, "Running them."),
                command.clone(),
                item("a2", ItemKind::AssistantMessage, "One test fails."),
            ],
            more: false,
            start: Some("s0".to_owned()),
        },
        "approvals, timing and other branches stay out"
    );

    let latest = history.transcript(None, 3);
    assert_eq!(
        (latest.items.len(), latest.more),
        (1, true),
        "the newest three entries hold only the final answer"
    );
    let earlier = history.transcript(Some("p1"), 3);
    assert_eq!(
        earlier,
        Transcript {
            items: vec![
                item("u1", ItemKind::UserMessage, "Run the tests"),
                item("a1", ItemKind::Reasoning, ""),
                item("a1", ItemKind::AssistantMessage, "Running them."),
                command,
            ],
            more: false,
            start: Some("s0".to_owned()),
        },
        "an earlier page still knows how its tool calls ended"
    );

    let hidden = history.transcript(Some("a2"), 2);
    assert_eq!(
        (hidden.items.len(), hidden.more, hidden.start.as_deref()),
        (0, true, Some("t1")),
        "a page without chat items still tells where the next one ends"
    );
    assert_eq!(history.transcript(Some("t1"), 2).items.len(), 3);
}
