use serde_json::Value;

use crate::{ChangedLines, History, ItemKind, Transcript, TranscriptItem};

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
        lines: None,
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

#[test]
fn shows_a_call_that_never_finished_as_failed() {
    let data: Value = serde_json::from_str(
        r#"{"entries":[
          {"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"Deploy"}},
          {"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"bash","arguments":{"command":"./deploy.sh"}}]}}
        ],"leafId":"a1"}"#,
    )
    .expect("JSON");
    let items = History::from_entries(&data).transcript(None, 10).items;
    let call = items.last().expect("the call");
    assert_eq!(
        (call.tool.as_deref(), call.failed, call.duration_ms),
        (Some("bash"), true, None),
        "no result means it did not succeed, for example after a crash"
    );
}

#[test]
fn counts_the_lines_of_file_changes() {
    let session = r#"{"entries":[
      {"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"Edit"}},
      {"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"edit","arguments":{"path":"a.txt"}},{"type":"toolCall","id":"c2","name":"write","arguments":{"path":"b.txt","content":"one\ntwo\n"}},{"type":"toolCall","id":"c3","name":"edit","arguments":{"path":"c.txt"}}]}},
      {"type":"message","id":"r1","parentId":"a1","message":{"role":"toolResult","toolCallId":"c1","toolName":"edit","isError":false,"content":[],"details":{"patch":"@@ -1,2 +1,2 @@\n-old\n+new\n same\n"}}},
      {"type":"message","id":"r2","parentId":"r1","message":{"role":"toolResult","toolCallId":"c2","toolName":"write","isError":false,"content":[]}},
      {"type":"message","id":"r3","parentId":"r2","message":{"role":"toolResult","toolCallId":"c3","toolName":"edit","isError":true,"content":[]}}
    ],"leafId":"r3"}"#;
    let history = History::from_entries(&serde_json::from_str::<Value>(session).expect("JSON"));
    let lines: Vec<_> = history
        .transcript(None, 100)
        .items
        .into_iter()
        .filter(|item| item.tool.is_some())
        .map(|item| item.lines)
        .collect();
    assert_eq!(
        lines,
        [
            Some(ChangedLines {
                added: 1,
                removed: 1
            }),
            Some(ChangedLines {
                added: 2,
                removed: 0
            }),
            None,
        ],
        "an edit's patch, the lines written, and nothing for a failed edit"
    );
}
