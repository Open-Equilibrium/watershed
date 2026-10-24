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

/// One response with two tools that pi runs in parallel and stores together, an injected
/// message and a compaction, five minutes after the session started.
const PARALLEL: &str = r#"{"entries":[
  {"type":"model_change","id":"s0","parentId":null,"timestamp":"2026-10-24T10:00:00.000Z"},
  {"type":"message","id":"u1","parentId":"s0","timestamp":"2026-10-24T10:05:00.000Z","message":{"role":"user","content":"Read and test"}},
  {"type":"custom","id":"tr","parentId":"u1","timestamp":"2026-10-24T10:05:02.000Z","customType":"watershed.timing","data":{"kind":"request","firstTokenMs":500,"durationMs":2000}},
  {"type":"message","id":"a1","parentId":"tr","timestamp":"2026-10-24T10:05:02.000Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"read","arguments":{"path":"a.ts"}},{"type":"toolCall","id":"c2","name":"bash","arguments":{"command":"npm test"}}]}},
  {"type":"custom","id":"x1","parentId":"a1","timestamp":"2026-10-24T10:05:02.030Z","customType":"watershed.timing","data":{"kind":"tool","toolCallId":"c1","durationMs":20}},
  {"type":"custom","id":"x2","parentId":"x1","timestamp":"2026-10-24T10:05:08.000Z","customType":"watershed.timing","data":{"kind":"tool","toolCallId":"c2","durationMs":6000}},
  {"type":"message","id":"r1","parentId":"x2","timestamp":"2026-10-24T10:05:08.010Z","message":{"role":"toolResult","toolCallId":"c1","toolName":"read","content":[]}},
  {"type":"message","id":"r2","parentId":"r1","timestamp":"2026-10-24T10:05:08.020Z","message":{"role":"toolResult","toolCallId":"c2","toolName":"bash","content":[]}},
  {"type":"custom_message","id":"cm","parentId":"r2","timestamp":"2026-10-24T10:05:08.030Z","customType":"reminder","content":"Mind the style guide","display":true},
  {"type":"compaction","id":"k1","parentId":"cm","timestamp":"2026-10-24T10:05:09.000Z","summary":"Read a.ts and ran the tests.","firstKeptEntryId":"r2","tokensBefore":9000},
  {"type":"custom","id":"tq","parentId":"k1","timestamp":"2026-10-24T10:05:10.000Z","customType":"watershed.timing","data":{"kind":"request","firstTokenMs":300,"durationMs":1000}},
  {"type":"message","id":"a2","parentId":"tq","timestamp":"2026-10-24T10:05:10.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Done."}]}},
  {"type":"compaction","id":"k2","parentId":"a2","timestamp":"2026-10-24T10:05:11.000Z","summary":"Everything so far.","firstKeptEntryId":"k1","tokensBefore":4000},
  {"type":"context_edit","id":"ce","parentId":"k2","timestamp":"2026-10-24T10:05:12.000Z","targetId":"a2","replacement":null},
  {"type":"message","id":"bx","parentId":"ce","timestamp":"2026-10-24T10:05:13.000Z","message":{"role":"bashExecution","command":"ls","output":"","excludeFromContext":true}},
  {"type":"message","id":"a3","parentId":"bx","timestamp":"2026-10-24T10:05:14.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Again."}]}}
],"leafId":"a3"}"#;

#[test]
fn times_parallel_tools_from_their_own_end_and_follows_compaction() {
    let data: Value = serde_json::from_str(PARALLEL).expect("JSON");
    let records = trajectory(&data, None).records;
    let timing: Vec<_> = records
        .iter()
        .map(|record| (record.entry.as_str(), record.start_ms, record.duration_ms))
        .collect();
    assert_eq!(
        timing,
        [
            ("u1", 0, None),
            ("a1", 0, Some(2000)),
            ("r1", 2010, Some(20)),
            ("r2", 2000, Some(6000)),
            ("a2", 9000, Some(1000)),
            ("a3", 14000, None),
        ],
        "times count from the first prompt; each tool ends when its timing entry was stored"
    );
    let context = records[4].context.clone().expect("context");
    assert_eq!(
        (context.messages, context.new_since_previous),
        (
            3,
            vec![
                "result of bash".to_owned(),
                "message from reminder".to_owned(),
                "compaction summary".to_owned(),
            ]
        ),
        "after the compaction the model saw its summary, the kept tool result and the injected \
         message, but not the summarized read"
    );
    let context = records[5].context.clone().expect("context");
    assert_eq!(
        (context.messages, context.new_since_previous),
        (
            1,
            vec![
                "compaction summary".to_owned(),
                "removed message".to_owned(),
            ]
        ),
        "a retained older compaction adds no message, an edit removed the kept response and an \
         excluded shell command stays out"
    );
}

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
    let user = record("e2", 1, RecordKind::User, "Add rate limiting", 0, None);
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
            0,
            Some(3400),
        )
    };
    let approval = TrajectoryRecord {
        detail: "allow-once".to_owned(),
        ..record("a1", 1, RecordKind::Approval, "bash", 3500, Some(9400))
    };
    let tool = TrajectoryRecord {
        detail: r#"{"command":"npm test"}"#.to_owned(),
        failed: true,
        ..record("e4", 1, RecordKind::Tool, "bash", 12900, Some(6800))
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
        ..record("e7", 1, RecordKind::Message, "Done.", 20900, None)
    };
    assert_eq!(
        original,
        Trajectory {
            records: vec![user.clone(), message.clone(), approval, tool, finished],
            duration_ms: 20900,
        }
    );

    let fork = trajectory(&data, None);
    assert_eq!(
        fork.records,
        [
            user,
            message,
            record("b1", 2, RecordKind::User, "Use a token bucket", 64900, None),
        ]
    );
}

#[test]
fn a_later_edit_brings_a_removed_message_back() {
    let data: Value = serde_json::from_str(
        r#"{"entries":[
          {"type":"message","id":"u1","parentId":null,"timestamp":"2026-10-24T10:00:00.000Z","message":{"role":"user","content":"Read it"}},
          {"type":"message","id":"a1","parentId":"u1","timestamp":"2026-10-24T10:00:01.000Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"read","arguments":{}}]}},
          {"type":"message","id":"r1","parentId":"a1","timestamp":"2026-10-24T10:00:02.000Z","message":{"role":"toolResult","toolCallId":"c1","toolName":"read","content":[]}},
          {"type":"context_edit","id":"x1","parentId":"r1","timestamp":"2026-10-24T10:00:03.000Z","targetId":"r1","replacement":null},
          {"type":"context_edit","id":"x2","parentId":"x1","timestamp":"2026-10-24T10:00:04.000Z","targetId":"r1","replacement":"shorter"},
          {"type":"message","id":"a2","parentId":"x2","timestamp":"2026-10-24T10:00:05.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Done."}]}}
        ],"leafId":"a2"}"#,
    )
    .expect("JSON");
    let context = trajectory(&data, None).records[3]
        .context
        .clone()
        .expect("context");
    assert_eq!(
        (context.messages, context.new_since_previous),
        (
            3,
            vec![
                "result of read".to_owned(),
                "removed message".to_owned(),
                "edited message".to_owned(),
            ]
        ),
        "the latest edit wins, so the replaced tool result is back in the context"
    );
}
