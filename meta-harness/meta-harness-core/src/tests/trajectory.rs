use serde_json::Value;

use crate::pi::trajectory;
use crate::{RecordKind, StepContext, Trajectory, TrajectoryRecord, Usage};

/// A session with a branch after the first model response: `e7` continues the original run,
/// `b1` a fork.
const ENTRIES: &str = r#"{"entries":[
  {"type":"model_change","id":"e0","parentId":null,"timestamp":"2026-10-24T10:00:00.000Z"},
  {"type":"message","id":"e1","parentId":"e0","timestamp":"2026-10-24T10:00:00.010Z","message":{"role":"system","content":"","sections":{"preamble":"You are pi.","rules":"Be brief."},"toolsAdded":[{"name":"read"},{"name":"bash"}]}},
  {"type":"message","id":"e2","parentId":"e1","timestamp":"2026-10-24T10:00:00.100Z","message":{"role":"user","content":[{"type":"text","text":"Add rate limiting"}]}},
  {"type":"custom","id":"t1","parentId":"e2","timestamp":"2026-10-24T10:00:03.500Z","customType":"watershed.timing","data":{"kind":"request","firstTokenMs":900,"durationMs":3400}},
  {"type":"message","id":"e3","parentId":"t1","timestamp":"2026-10-24T10:00:03.500Z","message":{"role":"assistant","model":"faux-1","stopReason":"toolUse","usage":{"input":6140,"output":820,"cacheRead":0,"cacheWrite":0,"reasoning":214,"totalTokens":6960,"cost":{"total":0.02}},"content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"I'll run the tests.\nThen fix."},{"type":"toolCall","id":"c1","name":"bash","arguments":{"command":"npm test"}}]}},
  {"type":"custom","id":"a1","parentId":"e3","timestamp":"2026-10-24T10:00:13.000Z","customType":"watershed.approval","data":{"toolCallId":"c1","tool":"bash","decision":"allow-once","waitedMs":9400}},
  {"type":"custom","id":"t2","parentId":"a1","timestamp":"2026-10-24T10:00:19.800Z","customType":"watershed.timing","data":{"kind":"tool","toolCallId":"c1","durationMs":6800}},
  {"type":"message","id":"e4","parentId":"t2","timestamp":"2026-10-24T10:00:19.800Z","message":{"role":"toolResult","toolCallId":"c1","toolName":"bash","isError":true,"content":[{"type":"text","text":"1 failed"}]}},
  {"type":"message","id":"e7","parentId":"e4","timestamp":"2026-10-24T10:00:21.000Z","message":{"role":"assistant","model":"faux-1","stopReason":"stop","usage":{"input":7000,"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":7010,"cost":{"total":0}},"content":[{"type":"text","text":"Done."}]}},
  {"type":"label","id":"l1","parentId":"e3","timestamp":"2026-10-24T10:01:00.000Z","targetId":"e3","label":"B"},
  {"type":"message","id":"b1","parentId":"l1","timestamp":"2026-10-24T10:01:05.000Z","message":{"role":"user","content":"Use a token bucket"}}
],"leafId":"b1"}"#;

#[test]
fn projects_the_selected_branch_with_timing_and_what_the_model_saw() {
    let data: Value = serde_json::from_str(ENTRIES).expect("JSON");
    let record = |entry: &str, turn, kind, label: &str, start_ms, duration_ms| TrajectoryRecord {
        entry: entry.to_owned(),
        turn,
        kind,
        label: label.to_owned(),
        detail: String::new(),
        start_ms,
        duration_ms,
        first_token_ms: None,
        model: String::new(),
        usage: None,
        stop_reason: String::new(),
        failed: false,
        context: None,
    };
    let user = record("e2", 1, RecordKind::User, "Add rate limiting", 100, None);
    let message = TrajectoryRecord {
        first_token_ms: Some(900),
        model: "faux-1".to_owned(),
        usage: Some(Usage {
            input: 6140,
            output: 820,
            cache_read: 0,
            cache_write: 0,
            reasoning: 214,
            cost_micro_usd: 20000,
        }),
        stop_reason: "toolUse".to_owned(),
        context: Some(StepContext {
            prompt_chars: 20,
            tools: vec!["bash".to_owned(), "read".to_owned()],
            messages: 1,
            new_since_previous: vec!["prompt".to_owned()],
        }),
        ..record(
            "e3",
            1,
            RecordKind::Message,
            "I'll run the tests.",
            100,
            Some(3400),
        )
    };
    let approval = TrajectoryRecord {
        detail: "allow-once".to_owned(),
        ..record("a1", 1, RecordKind::Approval, "bash", 3600, Some(9400))
    };
    let tool = TrajectoryRecord {
        detail: r#"{"command":"npm test"}"#.to_owned(),
        failed: true,
        ..record("e4", 1, RecordKind::Tool, "bash", 13000, Some(6800))
    };

    let original = trajectory(&data, Some("e7"));
    let finished = TrajectoryRecord {
        model: "faux-1".to_owned(),
        usage: Some(Usage {
            input: 7000,
            output: 10,
            ..Usage::default()
        }),
        stop_reason: "stop".to_owned(),
        context: Some(StepContext {
            prompt_chars: 20,
            tools: vec!["bash".to_owned(), "read".to_owned()],
            messages: 3,
            new_since_previous: vec!["result of bash".to_owned()],
        }),
        ..record("e7", 1, RecordKind::Message, "Done.", 21000, None)
    };
    assert_eq!(
        original,
        Trajectory {
            records: vec![user.clone(), message.clone(), approval, tool, finished],
            duration_ms: 21000,
        }
    );

    let fork = trajectory(&data, None);
    assert_eq!(
        fork.records,
        [
            user,
            message,
            record("b1", 2, RecordKind::User, "Use a token bucket", 65000, None),
        ]
    );
}
