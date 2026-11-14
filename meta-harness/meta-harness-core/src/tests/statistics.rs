use serde_json::Value;

use crate::{CostByKind, History, ModelUsage, Statistics, ToolCalls, TurnStatistics, Usage};

/// A session with checkpoints: a compaction before the first turn, three turns and a steering
/// prompt that stays in turn one. Turn one's end checkpoint comes after its last response, so its
/// unanswered `edit` call was cancelled; turn two's `bash` call was cancelled once turn three
/// started; turn three's `bash` call is still open. Turn three has no prompt.
const CHECKPOINTED: &str = r#"{"entries":[
  {"type":"model_change","id":"m0","parentId":null,"timestamp":"2026-11-01T10:00:00.000Z"},
  {"type":"compaction","id":"c0","parentId":"m0","timestamp":"2026-11-01T10:00:00.500Z","firstKeptEntryId":"m0","summary":"s","usage":{"input":30,"output":1,"cost":{"total":0}}},
  {"type":"custom","id":"k1","parentId":"c0","timestamp":"2026-11-01T10:00:01.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T1","phase":"start"}},
  {"type":"message","id":"u1","parentId":"k1","timestamp":"2026-11-01T10:00:01.100Z","message":{"role":"user","content":"Find why login fails\nin the second test"}},
  {"type":"custom","id":"t1","parentId":"u1","timestamp":"2026-11-01T10:00:01.200Z","customType":"watershed.timing","data":{"kind":"request","durationMs":2000}},
  {"type":"message","id":"a1","parentId":"t1","timestamp":"2026-11-01T10:00:03.200Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4-5","content":[{"type":"toolCall","id":"c1","name":"read","arguments":{"path":"a.ts"}},{"type":"toolCall","id":"c2","name":"bash","arguments":{"command":"npm test"}}],"usage":{"input":100,"output":10,"cacheRead":900,"cacheWrite":0,"cost":{"total":0.01}}}},
  {"type":"custom","id":"t2","parentId":"a1","timestamp":"2026-11-01T10:00:03.700Z","customType":"watershed.timing","data":{"kind":"tool","toolCallId":"c1","durationMs":500}},
  {"type":"custom","id":"t3","parentId":"t2","timestamp":"2026-11-01T10:00:04.200Z","customType":"watershed.timing","data":{"kind":"tool","toolCallId":"c2","durationMs":1500}},
  {"type":"message","id":"r1","parentId":"t3","timestamp":"2026-11-01T10:00:04.300Z","message":{"role":"toolResult","toolCallId":"c1","toolName":"read","isError":false,"content":[]}},
  {"type":"message","id":"r2","parentId":"r1","timestamp":"2026-11-01T10:00:04.400Z","message":{"role":"toolResult","toolCallId":"c2","toolName":"bash","isError":true,"content":[]}},
  {"type":"custom","id":"t4","parentId":"r2","timestamp":"2026-11-01T10:00:04.500Z","customType":"watershed.timing","data":{"kind":"request","durationMs":3000}},
  {"type":"message","id":"a2","parentId":"t4","timestamp":"2026-11-01T10:00:07.500Z","message":{"role":"assistant","provider":"anthropic","model":"claude-haiku-4-5","content":[{"type":"toolCall","id":"c3","name":"edit","arguments":{}}],"usage":{"input":10,"output":5,"cost":{"total":0}}}},
  {"type":"custom","id":"k2","parentId":"a2","timestamp":"2026-11-01T10:00:08.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T1","phase":"end","files":0,"added":0,"removed":0}},
  {"type":"custom","id":"k3","parentId":"k2","timestamp":"2026-11-01T10:00:09.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T2","phase":"start"}},
  {"type":"message","id":"u2","parentId":"k3","timestamp":"2026-11-01T10:00:09.100Z","message":{"role":"user","content":"Explain the retry policy"}},
  {"type":"message","id":"a3","parentId":"u2","timestamp":"2026-11-01T10:00:10.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4-5","content":[{"type":"toolCall","id":"c4","name":"bash","arguments":{}}],"usage":{"input":50,"output":5,"cost":{"total":0.005}}}},
  {"type":"custom","id":"k4","parentId":"a3","timestamp":"2026-11-01T10:00:11.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T3","phase":"start"}},
  {"type":"message","id":"a4","parentId":"k4","timestamp":"2026-11-01T10:00:11.500Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4-5","content":[{"type":"toolCall","id":"c5","name":"bash","arguments":{}}],"usage":{"input":20,"output":2,"cost":{"total":0}}}}
],"leafId":"a4"}"#;

/// A session without checkpoints: each prompt starts a turn. Only turn one has timing entries.
const UNCHECKPOINTED: &str = r#"{"entries":[
  {"type":"message","id":"u1","parentId":null,"timestamp":"2026-11-01T10:00:00.000Z","message":{"role":"user","content":"Fix it"}},
  {"type":"custom","id":"t1","parentId":"u1","timestamp":"2026-11-01T10:00:00.100Z","customType":"watershed.timing","data":{"kind":"request","durationMs":2000}},
  {"type":"message","id":"a1","parentId":"t1","timestamp":"2026-11-01T10:00:02.000Z","message":{"role":"assistant","provider":"faux","model":"faux-1","content":[{"type":"toolCall","id":"c1","name":"write","arguments":{}}],"usage":{"input":5,"output":1,"cost":{"total":0.001}}}},
  {"type":"message","id":"r1","parentId":"a1","timestamp":"2026-11-01T10:00:03.000Z","message":{"role":"toolResult","toolCallId":"c1","toolName":"write","isError":true,"content":[]}},
  {"type":"message","id":"u2","parentId":"r1","timestamp":"2026-11-01T10:00:04.000Z","message":{"role":"user","content":"Thanks"}},
  {"type":"message","id":"a2","parentId":"u2","timestamp":"2026-11-01T10:00:05.000Z","message":{"role":"assistant","provider":"faux","model":"faux-1","content":[{"type":"toolCall","id":"c2","name":"grep","arguments":{}}],"usage":{"input":5,"output":1,"cost":{"total":0.001}}}}
],"leafId":"a2"}"#;

fn counted(fixture: &str) -> Statistics {
    let data: Value = serde_json::from_str(fixture).expect("JSON");
    History::from_entries(&data).statistics()
}

fn model(model: &str, usage: Usage, responses: u32, unpriced: u32) -> ModelUsage {
    ModelUsage {
        model: model.to_owned(),
        usage,
        cost_by_kind: CostByKind::default(),
        responses,
        unpriced,
    }
}

#[test]
fn turns_start_at_checkpoints_and_split_figures_by_model_tool_and_time() {
    let statistics = counted(CHECKPOINTED);
    let sonnet = "anthropic/claude-sonnet-4-5";
    let haiku = "anthropic/claude-haiku-4-5";
    assert_eq!(
        (statistics.started_ms, statistics.ended_ms),
        (Some(1_793_527_200_000), Some(1_793_527_211_500)),
        "from the first stored entry to the last"
    );
    assert_eq!(
        statistics.models,
        [
            model(
                sonnet,
                Usage {
                    input: 170,
                    output: 17,
                    cache_read: 900,
                    cost_micro_usd: 15_000,
                    ..Usage::default()
                },
                3,
                1,
            ),
            // The compaction before the first turn has no model and is no response. It costs
            // nothing, so its tokens rank it above the free haiku usage.
            model(
                "",
                Usage {
                    input: 30,
                    output: 1,
                    ..Usage::default()
                },
                0,
                0,
            ),
            model(
                haiku,
                Usage {
                    input: 10,
                    output: 5,
                    ..Usage::default()
                },
                1,
                1,
            ),
        ],
        "the whole session, the preamble included, sorted by cost and then tokens"
    );
    assert_eq!(
        statistics.turns,
        [
            TurnStatistics {
                entry: "u1".to_owned(),
                prompt: "Find why login fails".to_owned(),
                started_ms: Some(1_793_527_201_000),
                ended_ms: Some(1_793_527_208_000),
                models: vec![
                    model(
                        sonnet,
                        Usage {
                            input: 100,
                            output: 10,
                            cache_read: 900,
                            cost_micro_usd: 10_000,
                            ..Usage::default()
                        },
                        1,
                        0,
                    ),
                    model(
                        haiku,
                        Usage {
                            input: 10,
                            output: 5,
                            ..Usage::default()
                        },
                        1,
                        1,
                    ),
                ],
                // Two requests; the tools of the first ran in parallel, so the longer one counts.
                model_ms: Some(5_000),
                tool_ms: Some(1_500),
                tool_calls: 3,
                failed: 1,
            },
            TurnStatistics {
                entry: "u2".to_owned(),
                prompt: "Explain the retry policy".to_owned(),
                started_ms: Some(1_793_527_209_000),
                ended_ms: Some(1_793_527_210_000),
                models: vec![model(
                    sonnet,
                    Usage {
                        input: 50,
                        output: 5,
                        cost_micro_usd: 5_000,
                        ..Usage::default()
                    },
                    1,
                    0,
                )],
                // No timing entry in this turn, so no time is shown rather than a guess.
                model_ms: None,
                tool_ms: None,
                tool_calls: 1,
                failed: 0,
            },
            TurnStatistics {
                // Without a prompt, the turn is named by its start checkpoint.
                entry: "k4".to_owned(),
                prompt: String::new(),
                started_ms: Some(1_793_527_211_000),
                ended_ms: Some(1_793_527_211_500),
                models: vec![model(
                    sonnet,
                    Usage {
                        input: 20,
                        output: 2,
                        ..Usage::default()
                    },
                    1,
                    1,
                )],
                model_ms: None,
                tool_ms: None,
                tool_calls: 1,
                failed: 0,
            },
        ],
        "a turn starts at each start checkpoint; the entries before the first one count only to the session"
    );
    assert_eq!(
        statistics.tools,
        [
            ToolCalls {
                name: "bash".to_owned(),
                succeeded: 0,
                failed: 1,
                cancelled: 1,
                open: 1,
            },
            ToolCalls {
                name: "edit".to_owned(),
                succeeded: 0,
                failed: 0,
                cancelled: 1,
                open: 0,
            },
            ToolCalls {
                name: "read".to_owned(),
                succeeded: 1,
                failed: 0,
                cancelled: 0,
                open: 0,
            },
        ],
        "a call without a result is cancelled once its turn ended or a later turn started, and open otherwise; ties sort by name"
    );
}

#[test]
fn without_checkpoints_each_prompt_starts_a_turn_and_missing_timing_stays_none() {
    let statistics = counted(UNCHECKPOINTED);
    let faux = "faux/faux-1";
    assert_eq!(
        statistics.turns,
        [
            TurnStatistics {
                entry: "u1".to_owned(),
                prompt: "Fix it".to_owned(),
                started_ms: Some(1_793_527_200_000),
                ended_ms: Some(1_793_527_203_000),
                models: vec![model(
                    faux,
                    Usage {
                        input: 5,
                        output: 1,
                        cost_micro_usd: 1_000,
                        ..Usage::default()
                    },
                    1,
                    0,
                )],
                // Its request was timed, and its one tool was not: the tools took no time.
                model_ms: Some(2_000),
                tool_ms: Some(0),
                tool_calls: 1,
                failed: 1,
            },
            TurnStatistics {
                entry: "u2".to_owned(),
                prompt: "Thanks".to_owned(),
                started_ms: Some(1_793_527_204_000),
                ended_ms: Some(1_793_527_205_000),
                models: vec![model(
                    faux,
                    Usage {
                        input: 5,
                        output: 1,
                        cost_micro_usd: 1_000,
                        ..Usage::default()
                    },
                    1,
                    0,
                )],
                model_ms: None,
                tool_ms: None,
                tool_calls: 1,
                failed: 0,
            },
        ],
    );
    assert_eq!(
        statistics.models,
        [model(
            faux,
            Usage {
                input: 10,
                output: 2,
                cost_micro_usd: 2_000,
                ..Usage::default()
            },
            2,
            0,
        )]
    );
    assert_eq!(
        statistics.tools,
        [
            // The last turn has no end checkpoint, so its call is still running.
            ToolCalls {
                name: "grep".to_owned(),
                succeeded: 0,
                failed: 0,
                cancelled: 0,
                open: 1,
            },
            ToolCalls {
                name: "write".to_owned(),
                succeeded: 0,
                failed: 1,
                cancelled: 0,
                open: 0,
            },
        ]
    );
}

#[test]
fn splits_each_model_cost_by_kind_of_tokens() {
    let statistics = counted(
        r#"{"entries":[
          {"type":"message","id":"u1","parentId":null,"timestamp":"2026-11-01T10:00:00.000Z","message":{"role":"user","content":"Hi"}},
          {"type":"message","id":"a1","parentId":"u1","timestamp":"2026-11-01T10:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4-5","content":[],"usage":{"input":100,"output":10,"cacheRead":900,"cacheWrite":5,"cost":{"input":0.0005,"output":0.0003,"cacheRead":0.0002,"cacheWrite":0.00001,"total":0.00101}}}},
          {"type":"message","id":"a2","parentId":"a1","timestamp":"2026-11-01T10:00:02.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4-5","content":[],"usage":{"input":50,"output":5,"cost":{"input":0.00025,"output":0.00015,"total":0.0004}}}}
        ],"leafId":"a2"}"#,
    );
    assert_eq!(
        statistics.models[0].cost_by_kind,
        CostByKind {
            input: 750,
            output: 450,
            cache_read: 200,
            cache_write: 10,
        },
        "each kind's cost is summed over the model's responses, in micro-dollars"
    );
    assert_eq!(
        statistics.models[0].usage.cost_micro_usd, 1_410,
        "the kinds add up to the model's cost"
    );
}
