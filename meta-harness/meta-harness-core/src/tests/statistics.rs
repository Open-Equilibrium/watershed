//! A thread's exact statistics and its context window, from T3's records (ADR-0279, ADR-0293).

use serde_json::{Value, json};

use crate::statistics::{context, statistics};
use crate::t3::Projection;
use crate::{Context, ModelUsage, ToolCalls, TurnStatistics, Usage};

/// T3's records as `statistics` reads them: the given collections, the rest empty.
fn projection(records: Value) -> Projection {
    let mut snapshot = json!({"thread": {}, "runs": [], "attempts": [], "providerTurns": [],
        "turnItems": [], "messages": []});
    for (key, value) in records.as_object().into_iter().flatten() {
        snapshot[key] = value.clone();
    }
    Projection::from_snapshot(&snapshot)
}

fn run(id: &str, ordinal: u64, status: &str, completed: Value) -> Value {
    json!({"id": id, "ordinal": ordinal, "status": status, "userMessageId": format!("m-{id}"),
        "startedAt": "2026-11-15T10:00:00.000Z", "completedAt": completed})
}

fn turn(id: &str, attempt: &str, model: Value, usage: Value) -> Value {
    json!({"id": id, "runAttemptId": attempt, "model": model, "status": "completed",
        "turnTokenUsage": usage})
}

#[test]
fn statistics_count_tokens_tools_and_turns_as_t3_records_them() {
    let projection = projection(json!({
        "runs": [
            run("r1", 1, "completed", json!("2026-11-15T10:00:10.000Z")),
            run("r2", 2, "running", Value::Null),
            run("r3", 3, "rolled_back", json!("2026-11-15T10:02:00.000Z")),
        ],
        "attempts": [{"id": "a1", "runId": "r1"}, {"id": "a2", "runId": "r2"}, {"id": "a3", "runId": "r3"}],
        "messages": [{"id": "m-r1", "text": "build the thing\nin detail"}, {"id": "m-r2", "text": "second"}],
        "providerTurns": [
            turn("t1", "a1", json!("claude/opus"), json!({"usageStatus": "complete", "inputTokens": 100,
                "cachedInputTokens": 40, "cacheCreationTokens": 10, "outputTokens": 20, "reasoningTokens": 5})),
            turn("t2", "a1", json!("claude/opus"), json!({"usageStatus": "complete", "inputTokens": 50,
                "outputTokens": 5})),
            // Counted by neither: partial usage, no input count, no attempt, a rolled-back run.
            turn("t3", "a1", json!("claude/opus"), json!({"usageStatus": "partial", "inputTokens": 999,
                "outputTokens": 999})),
            turn("t4", "a1", json!("claude/opus"), json!({"usageStatus": "complete", "outputTokens": 9})),
            turn("t5", "gone", json!("claude/opus"), json!({"usageStatus": "complete", "inputTokens": 9,
                "outputTokens": 9})),
            turn("t6", "a3", json!("claude/opus"), json!({"usageStatus": "complete", "inputTokens": 1000,
                "outputTokens": 1000})),
            // Turns of r2, the smaller model listed first: each turn lists its models largest first.
            turn("t7", "a2", Value::Null, json!({"usageStatus": "complete", "inputTokens": 2, "outputTokens": 1})),
            turn("t8", "a2", json!("codex/gpt"), json!({"usageStatus": "complete", "inputTokens": 7,
                "outputTokens": 3})),
        ],
        "turnItems": [
            {"id": "u1", "runId": "r1", "type": "user_message", "text": "build the thing"},
            {"id": "c1", "runId": "r1", "type": "command_execution", "status": "completed", "exitCode": 0,
             "input": "ls", "startedAt": "2026-11-15T10:00:01.000Z", "completedAt": "2026-11-15T10:00:02.000Z"},
            {"id": "c2", "runId": "r1", "type": "command_execution", "status": "completed", "exitCode": 1,
             "input": "make", "startedAt": "2026-11-15T10:00:02.000Z", "completedAt": "2026-11-15T10:00:05.000Z"},
            {"id": "c3", "runId": "r1", "type": "command_execution", "status": "interrupted", "input": "sleep 9"},
            {"id": "c4", "runId": "r2", "type": "command_execution", "status": "running", "input": "npm test"},
            {"id": "f1", "runId": "r1", "type": "file_change", "status": "failed", "fileName": "a.rs"},
            {"id": "f2", "runId": "r1", "type": "file_change", "status": "completed", "fileName": "b.rs"},
            {"id": "d1", "runId": "r1", "type": "dynamic_tool", "title": "Read", "status": "completed"},
            {"id": "d2", "runId": "r3", "type": "dynamic_tool", "title": "Read", "status": "completed"},
        ],
    }));

    let statistics = statistics(&projection);
    let opus = ModelUsage {
        model: "claude/opus".to_owned(),
        usage: Usage {
            input: 100,
            output: 25,
            cache_read: 40,
            cache_write: 10,
            reasoning: 5,
        },
        responses: 2,
    };
    let codex = ModelUsage {
        model: "codex/gpt".to_owned(),
        usage: Usage {
            input: 7,
            output: 3,
            ..Usage::default()
        },
        responses: 1,
    };
    let unnamed = ModelUsage {
        model: String::new(),
        usage: Usage {
            input: 2,
            output: 1,
            ..Usage::default()
        },
        responses: 1,
    };
    assert_eq!(
        statistics.models,
        vec![opus.clone(), codex.clone(), unnamed.clone()]
    );
    let tool = |name: &str, succeeded, failed, cancelled, open, duration_ms| ToolCalls {
        name: name.to_owned(),
        succeeded,
        failed,
        cancelled,
        open,
        duration_ms,
    };
    assert_eq!(
        statistics.tools,
        vec![
            tool("bash", 1, 1, 1, 1, Some(4000)),
            tool("edit", 1, 1, 0, 0, None),
            tool("Read", 1, 0, 0, 0, None),
        ]
    );
    assert_eq!(
        statistics.turns,
        vec![
            TurnStatistics {
                entry: "1".to_owned(),
                prompt: "build the thing".to_owned(),
                started_ms: Some(1_794_736_800_000),
                ended_ms: Some(1_794_736_810_000),
                models: vec![opus],
                tool_ms: Some(4000),
                failed: 2,
            },
            TurnStatistics {
                entry: "2".to_owned(),
                prompt: "second".to_owned(),
                started_ms: Some(1_794_736_800_000),
                ended_ms: None,
                models: vec![codex, unnamed],
                tool_ms: None,
                failed: 0,
            },
        ]
    );
    assert_eq!(statistics.started_ms, Some(1_794_736_800_000));
    assert_eq!(statistics.ended_ms, Some(1_794_736_800_000));
}

#[test]
fn a_turns_tool_time_counts_the_longest_call_of_each_response() {
    let timed = projection(json!({
        "runs": [run("r1", 1, "completed", json!("2026-11-15T10:00:10.000Z"))],
        "turnItems": [
            // Two calls of one response ran in parallel: the response counts the longer one.
            {"id": "p1", "runId": "r1", "providerTurnId": "t1", "type": "command_execution", "status": "completed",
             "exitCode": 0, "input": "a", "startedAt": "2026-11-15T10:00:01.000Z", "completedAt": "2026-11-15T10:00:02.000Z"},
            {"id": "p2", "runId": "r1", "providerTurnId": "t1", "type": "command_execution", "status": "completed",
             "exitCode": 0, "input": "b", "startedAt": "2026-11-15T10:00:01.000Z", "completedAt": "2026-11-15T10:00:04.000Z"},
            // A later response's call adds its own time.
            {"id": "p3", "runId": "r1", "providerTurnId": "t2", "type": "command_execution", "status": "completed",
             "exitCode": 0, "input": "c", "startedAt": "2026-11-15T10:00:05.000Z", "completedAt": "2026-11-15T10:00:07.000Z"},
        ],
    }));
    let statistics = statistics(&timed);
    assert_eq!(statistics.turns[0].tool_ms, Some(5000));
    // Each tool's own time is the sum of its calls.
    assert_eq!(statistics.tools[0].duration_ms, Some(6000));

    // A turn without tool calls took no tool time, which is known, not missing.
    let chat_only = projection(json!({
        "runs": [run("r1", 1, "completed", json!("2026-11-15T10:00:10.000Z"))],
        "turnItems": [],
    }));
    assert_eq!(
        crate::statistics::statistics(&chat_only).turns[0].tool_ms,
        Some(0)
    );
}

#[test]
fn an_unfinished_call_is_open_only_while_its_turn_is_the_last_and_still_runs() {
    // (the last turn's status, the call's status, whether the call is open)
    let cases = [
        ("running", "running", true),
        ("waiting", "pending", true),
        ("running", "interrupted", false),
        ("completed", "running", false),
        ("interrupted", "running", false),
        ("failed", "pending", false),
    ];
    for (turn_status, call_status, open) in cases {
        let projection = projection(json!({
            "runs": [
                run("r1", 1, "completed", json!("2026-11-15T10:00:10.000Z")),
                run("r2", 2, turn_status, Value::Null),
            ],
            // The call in the earlier turn never got a result either: its turn was followed by another.
            "turnItems": [
                {"id": "c1", "runId": "r1", "type": "command_execution", "status": "running", "input": "a"},
                {"id": "c2", "runId": "r2", "type": "command_execution", "status": call_status, "input": "b"},
            ],
        }));
        let bash = &statistics(&projection).tools[0];
        assert_eq!(
            (bash.cancelled, bash.open),
            if open { (1, 1) } else { (2, 0) },
            "last turn {turn_status}, call {call_status}"
        );
    }
}

#[test]
fn the_context_window_is_the_latest_one_t3_reported() {
    let window = |turns: Value| context(&projection(json!({"providerTurns": turns})));
    assert_eq!(
        window(json!([
            {"tokenUsage": {"usedTokens": 10, "maxTokens": 100, "autoCompactThreshold": 80}},
            {"tokenUsage": {"usedTokens": 20}},
        ])),
        Some(Context {
            tokens: Some(10),
            window: 100,
            compacts_at: Some(80)
        })
    );
    // After a compaction T3 reports the window without the tokens it holds.
    assert_eq!(
        window(json!([{"tokenUsage": {"maxTokens": 200}}])),
        Some(Context {
            tokens: None,
            window: 200,
            compacts_at: None
        })
    );
    assert_eq!(window(json!([{"tokenUsage": {"usedTokens": 5}}])), None);
}

#[test]
fn a_queued_turn_behind_the_running_one_leaves_its_unfinished_calls_open() {
    let queued = projection(json!({
        "runs": [
            run("r1", 1, "running", Value::Null),
            run("r2", 2, "queued", Value::Null),
        ],
        "turnItems": [
            {"id": "c1", "runId": "r1", "type": "command_execution", "status": "running", "input": "a"},
        ],
    }));
    let tools = statistics(&queued).tools;
    assert_eq!((tools[0].open, tools[0].cancelled), (1, 0));
}
