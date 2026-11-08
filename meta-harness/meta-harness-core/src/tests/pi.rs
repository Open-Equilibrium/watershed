use serde_json::{Map, Value};

use crate::{
    AgentEvent, ChangedLines, EntryKind, History, ItemDetail, ItemId, ItemKind, ItemStatus,
    NotificationLevel, PiAdapter, RecordKind, RequestId, SessionTree, TreeNode, TurnId, Usage,
    UserInput,
};

const TURN: TurnId = TurnId(1);

fn translate(records: &[&str]) -> Vec<AgentEvent> {
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

fn started(item: u64, kind: ItemKind) -> AgentEvent {
    AgentEvent::ItemStarted {
        turn: TURN,
        item: ItemId(item),
        kind,
        detail: ItemDetail::None,
    }
}

fn completed(item: u64, kind: ItemKind, text: &str) -> AgentEvent {
    AgentEvent::ItemCompleted {
        turn: TURN,
        item: ItemId(item),
        kind,
        text: text.to_owned(),
        status: ItemStatus::Completed,
        duration_ms: None,
        lines: None,
    }
}

fn delta(item: u64, text: &str) -> AgentEvent {
    AgentEvent::ContentDelta {
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
            AgentEvent::TurnStarted { turn: TURN },
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
            AgentEvent::ModelRequestCompleted {
                turn: TURN,
                usage: Usage::default(),
                stop_reason: String::new(),
                error: None,
            },
            AgentEvent::TurnCompleted { turn: TURN },
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
            AgentEvent::TurnStarted { turn: TURN },
            AgentEvent::TurnAborted { turn: TURN },
            AgentEvent::TurnStarted { turn: second },
            AgentEvent::ItemStarted {
                turn: second,
                item: ItemId(1),
                kind: ItemKind::UserMessage,
                detail: ItemDetail::None,
            },
            AgentEvent::ItemCompleted {
                turn: second,
                item: ItemId(1),
                kind: ItemKind::UserMessage,
                text: "Plain string prompt".to_owned(),
                status: ItemStatus::Completed,
                duration_ms: None,
                lines: None,
            },
        ]
    );
}

#[test]
fn ends_a_failed_responses_open_text_before_its_retry_reuses_the_index() {
    let events = translate(&[
        r#"{"type":"agent_start"}"#,
        r#"{"type":"message_start","message":{"role":"assistant","content":[]}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":0}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Partial"}}"#,
        r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"connection reset"}}"#,
        r#"{"type":"agent_end","messages":[],"willRetry":true}"#,
        r#"{"type":"agent_start"}"#,
        r#"{"type":"message_start","message":{"role":"assistant","content":[]}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":0}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hi"}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_end","contentIndex":0,"content":"Hi"}}"#,
        r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"stop"}}"#,
        r#"{"type":"agent_settled","aborted":false}"#,
    ]);
    let request = |stop_reason: &str, error: Option<&str>| AgentEvent::ModelRequestCompleted {
        turn: TURN,
        usage: Usage::default(),
        stop_reason: stop_reason.to_owned(),
        error: error.map(str::to_owned),
    };
    assert_eq!(
        events,
        [
            AgentEvent::TurnStarted { turn: TURN },
            started(1, ItemKind::AssistantMessage),
            delta(1, "Partial"),
            AgentEvent::ItemCompleted {
                turn: TURN,
                item: ItemId(1),
                kind: ItemKind::AssistantMessage,
                text: "Partial".to_owned(),
                status: ItemStatus::Failed,
                duration_ms: None,
                lines: None,
            },
            request("error", Some("connection reset")),
            started(2, ItemKind::AssistantMessage),
            delta(2, "Hi"),
            completed(2, ItemKind::AssistantMessage, "Hi"),
            request("stop", None),
            AgentEvent::TurnCompleted { turn: TURN },
        ],
        "the failed response keeps its partial text and the retry's text is its own item"
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
        r#"{"type":"tool_execution_start","toolCallId":"c4","toolName":"edit","args":{"path":"a.txt"}}"#,
        r#"{"type":"tool_execution_end","toolCallId":"c4","toolName":"edit","result":{"content":[{"type":"text","text":"Edited"}],"details":{"patch":"@@ -1,2 +1,2 @@\n-old\n+new\n same\n"}},"isError":false}"#,
        r#"{"type":"tool_execution_start","toolCallId":"c5","toolName":"write","args":{"path":"b.txt","content":"one\ntwo\nthree\n"}}"#,
        r#"{"type":"tool_execution_end","toolCallId":"c5","toolName":"write","result":{"content":[{"type":"text","text":"Wrote"}]},"isError":false}"#,
    ]);
    let tool = |item, kind, name: &str| AgentEvent::ItemStarted {
        turn: TURN,
        item: ItemId(item),
        kind,
        detail: ItemDetail::Tool {
            name: name.to_owned(),
            target: match item {
                1 => "ls",
                2 | 4 => "a.txt",
                3 => "x",
                _ => "b.txt",
            }
            .to_owned(),
        },
    };
    let finished = |item, kind, text: &str, status, duration_ms| AgentEvent::ItemCompleted {
        turn: TURN,
        item: ItemId(item),
        kind,
        text: text.to_owned(),
        status,
        duration_ms,
        lines: None,
    };
    let changed = |item, text: &str, added, removed| AgentEvent::ItemCompleted {
        turn: TURN,
        item: ItemId(item),
        kind: ItemKind::FileChange,
        text: text.to_owned(),
        status: ItemStatus::Completed,
        duration_ms: None,
        lines: Some(ChangedLines { added, removed }),
    };
    use ItemKind::{CommandExecution, DynamicToolCall, FileChange};
    assert_eq!(
        events,
        [
            AgentEvent::TurnStarted { turn: TURN },
            tool(1, CommandExecution, "bash"),
            AgentEvent::ItemUpdated {
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
            tool(2, FileChange, "edit"),
            finished(2, FileChange, "No match", ItemStatus::Failed, Some(3)),
            tool(3, DynamicToolCall, "grep"),
            finished(3, DynamicToolCall, "", ItemStatus::Completed, None),
            tool(4, FileChange, "edit"),
            changed(4, "Edited", 1, 1),
            tool(5, FileChange, "write"),
            changed(5, "Wrote", 3, 0),
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
    let input = |id, input| AgentEvent::UserInputRequested {
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
            AgentEvent::Notification {
                level: NotificationLevel::Warning,
                message: "Blocked".to_owned(),
            },
            AgentEvent::Notification {
                level: NotificationLevel::Info,
                message: "Saved".to_owned(),
            },
            AgentEvent::StatusChanged {
                key: "todo".to_owned(),
                text: Some("2/3".to_owned()),
            },
            AgentEvent::StatusChanged {
                key: "todo".to_owned(),
                text: None,
            },
            AgentEvent::WidgetChanged {
                key: "todo".to_owned(),
                lines: Some(vec!["one".to_owned(), "two".to_owned()]),
            },
            AgentEvent::WidgetChanged {
                key: "todo".to_owned(),
                lines: None,
            },
            // A title an extension sets is dropped.
            AgentEvent::InputTextSet {
                text: "draft".to_owned(),
            },
        ]
    );
}

#[test]
fn reports_what_each_model_request_used_and_how_it_ended() {
    let events = translate(&[
        r#"{"type":"agent_start"}"#,
        r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Hi"}],"model":"faux-1","stopReason":"stop","usage":{"input":6140,"output":820,"cacheRead":100,"cacheWrite":5,"reasoning":214,"totalTokens":6960,"cost":{"input":0.01,"output":0.008,"cacheRead":0,"cacheWrite":0,"total":0.0234567}}}}"#,
        r#"{"type":"message_end","message":{"role":"assistant","content":[],"model":"faux-1","stopReason":"error","errorMessage":"529 overloaded","usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"total":0}}}}"#,
    ]);
    let requests: Vec<_> = events
        .into_iter()
        .filter(|event| matches!(event, AgentEvent::ModelRequestCompleted { .. }))
        .collect();
    assert_eq!(
        requests,
        [
            AgentEvent::ModelRequestCompleted {
                turn: TURN,
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
            },
            AgentEvent::ModelRequestCompleted {
                turn: TURN,
                usage: Usage::default(),
                stop_reason: "error".to_owned(),
                error: Some("529 overloaded".to_owned()),
            },
        ]
    );
}

#[test]
fn builds_the_session_tree_from_flat_entries() {
    let data: Value = serde_json::from_str(
        r#"{"entries":[
          {"type":"message","id":"e1","parentId":null,"timestamp":"2026-10-24T10:00:00.250Z","message":{"role":"user","content":"Add rate limiting to /login\nand tests"}},
          {"type":"message","id":"e2","parentId":"e1","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"I'll look first."}]}},
          {"type":"message","id":"e3","parentId":"e2","message":{"role":"toolResult","toolName":"read","content":[]}},
          {"type":"label","id":"l1","parentId":"e2","targetId":"e2","label":"A"},
          {"type":"custom","id":"e4","parentId":"l1","customType":"acme.note","data":{}},
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
            at_ms: None,
            failed: false,
        };
    assert_eq!(
        History::from_entries(&data).tree(),
        SessionTree {
            nodes: vec![
                TreeNode {
                    at_ms: Some(1_792_836_000_250),
                    ..node(
                        "e1",
                        None,
                        EntryKind::User,
                        "Add rate limiting to /login",
                        None,
                    )
                },
                node(
                    "e2",
                    Some("e1"),
                    EntryKind::Assistant,
                    "I'll look first.",
                    Some("A2")
                ),
                node("e3", Some("e2"), EntryKind::ToolResult, "read", None),
                node("e4", Some("e2"), EntryKind::Custom, "", None),
                node("e5", Some("e4"), EntryKind::Other, "", None),
            ],
            leaf: Some("e4".to_owned()),
        }
    );
}

#[test]
fn names_what_a_tool_ran_on_and_whether_it_failed_in_the_tree() {
    let data: Value = serde_json::from_str(
        r#"{"entries":[
          {"type":"message","id":"a1","parentId":null,"message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"bash","arguments":{"command":"npm test"}},{"type":"toolCall","id":"c2","name":"read","arguments":{"path":"a.ts"}}]}},
          {"type":"message","id":"r1","parentId":"a1","message":{"role":"toolResult","toolCallId":"c1","toolName":"bash","isError":true,"content":[]}},
          {"type":"message","id":"r2","parentId":"r1","message":{"role":"toolResult","toolCallId":"c2","toolName":"read","content":[]}},
          {"type":"message","id":"a2","parentId":"r2","message":{"role":"assistant","stopReason":"error","content":[]}}],
          "leafId":"a2"}"#,
    )
    .expect("JSON");
    let nodes = History::from_entries(&data).tree().nodes;
    assert_eq!(
        nodes
            .iter()
            .map(|node| (node.id.as_str(), node.preview.as_str(), node.failed))
            .collect::<Vec<_>>(),
        [
            ("a1", "", false),
            ("r1", "bash npm test", true),
            ("r2", "read a.ts", false),
            ("a2", "", true),
        ],
        "a tool result names what its call ran on; a failed tool or response is marked"
    );
}

#[test]
fn names_what_each_result_ran_on_along_its_own_branch() {
    // `s2` continues after `r1` and `s1` after the response, so each answers the earliest call
    // its own branch left open; `r4` finds none open.
    let data: Value = serde_json::from_str(
        r#"{"entries":[
          {"type":"message","id":"a1","parentId":null,"message":{"role":"assistant","content":[{"type":"toolCall","id":"","name":"bash","arguments":{"command":"ls"}},{"type":"toolCall","id":"","name":"bash","arguments":{"command":"make"}},{"type":"toolCall","id":"c3","name":"bash","arguments":{"command":"npm test"}}]}},
          {"type":"message","id":"r1","parentId":"a1","message":{"role":"toolResult","toolCallId":"","toolName":"bash","content":[]}},
          {"type":"custom","id":"t1","parentId":"r1","customType":"watershed.timing","data":{"kind":"tool","toolCallId":"c3","durationMs":5}},
          {"type":"message","id":"r2","parentId":"t1","message":{"role":"toolResult","toolCallId":"","toolName":"bash","content":[]}},
          {"type":"message","id":"r3","parentId":"r2","message":{"role":"toolResult","toolCallId":"c3","toolName":"bash","content":[]}},
          {"type":"message","id":"s2","parentId":"r1","message":{"role":"toolResult","toolCallId":"","toolName":"bash","content":[]}},
          {"type":"message","id":"s1","parentId":"a1","message":{"role":"toolResult","toolCallId":"","toolName":"bash","content":[]}},
          {"type":"message","id":"a2","parentId":"r3","message":{"role":"assistant","content":[{"type":"toolCall","id":"","name":"bash","arguments":{"command":"cargo test"}}]}},
          {"type":"message","id":"q1","parentId":"a2","message":{"role":"toolResult","toolCallId":"","toolName":"bash","content":[]}},
          {"type":"message","id":"r4","parentId":"r3","message":{"role":"toolResult","toolCallId":"","toolName":"bash","content":[]}}],
          "leafId":"q1"}"#,
    )
    .expect("JSON");
    let previews: Vec<_> = History::from_entries(&data)
        .tree()
        .nodes
        .into_iter()
        .filter(|node| node.kind == EntryKind::ToolResult)
        .map(|node| (node.id, node.preview))
        .collect();
    let preview = |id: &str, preview: &str| (id.to_owned(), preview.to_owned());
    assert_eq!(
        previews,
        [
            preview("r1", "bash ls"),
            preview("r2", "bash make"),
            preview("r3", "bash npm test"),
            preview("s2", "bash make"),
            preview("s1", "bash ls"),
            preview("q1", "bash cargo test"),
            preview("r4", "bash"),
        ]
    );
}

#[test]
fn matches_each_result_to_its_own_call_when_ids_repeat() {
    // A server that sends no call ids leaves every call with the same empty one; a result
    // answers the earliest call of the response above it that no result answered yet.
    let data: Value = serde_json::from_str(
        r#"{"entries":[
          {"type":"message","id":"a1","parentId":null,"message":{"role":"assistant","content":[{"type":"toolCall","id":"","name":"bash","arguments":{"command":"ls"}}]}},
          {"type":"message","id":"r1","parentId":"a1","message":{"role":"toolResult","toolCallId":"","toolName":"bash","content":[]}},
          {"type":"message","id":"a2","parentId":"r1","message":{"role":"assistant","content":[{"type":"toolCall","id":"","name":"bash","arguments":{"command":"rm -rf build"}},{"type":"toolCall","id":"","name":"bash","arguments":{"command":"make"}}]}},
          {"type":"message","id":"r2","parentId":"a2","message":{"role":"toolResult","toolCallId":"","toolName":"bash","content":[],"isError":true}},
          {"type":"message","id":"r3","parentId":"r2","message":{"role":"toolResult","toolCallId":"","toolName":"bash","content":[]}}],
          "leafId":"r3"}"#,
    )
    .expect("JSON");
    let history = History::from_entries(&data);
    let previews: Vec<_> = history
        .tree()
        .nodes
        .into_iter()
        .filter(|node| node.kind == EntryKind::ToolResult)
        .map(|node| node.preview)
        .collect();
    assert_eq!(previews, ["bash ls", "bash rm -rf build", "bash make"]);
    let steps: Vec<_> = history
        .trajectory(None)
        .records
        .into_iter()
        .filter(|record| record.kind == RecordKind::Tool)
        .map(|record| (record.target, record.failed))
        .collect();
    assert_eq!(
        steps,
        [
            ("ls".to_owned(), false),
            ("rm -rf build".to_owned(), true),
            ("make".to_owned(), false)
        ]
    );
    let chat: Vec<_> = history
        .transcript(None, 10)
        .items
        .into_iter()
        .filter_map(|item| Some((item.target?, item.failed)))
        .collect();
    assert_eq!(chat, steps, "the chat tells the same as the Trajectory");
}

#[test]
fn times_results_with_repeated_ids_only_when_their_calls_can_be_told_apart() {
    // Timing entries name only the call id. pi runs `a1`'s tools one after another, each
    // followed by its result, and `a2`'s at once, with both results after both timing entries;
    // `a3`'s call never got its result.
    let tool = |id: &str, parent: &str, millis: u64| {
        format!(
            r#"{{"type":"custom","id":"{id}","parentId":"{parent}","customType":"watershed.timing","data":{{"kind":"tool","toolCallId":"","durationMs":{millis}}}}}"#
        )
    };
    let result = |id: &str, parent: &str| {
        format!(
            r#"{{"type":"message","id":"{id}","parentId":"{parent}","message":{{"role":"toolResult","toolCallId":"","toolName":"bash","content":[]}}}}"#
        )
    };
    let response = |id: &str, parent: &str, commands: &[&str]| {
        let calls: Vec<String> = commands
            .iter()
            .map(|command| {
                format!(
                    r#"{{"type":"toolCall","id":"","name":"bash","arguments":{{"command":"{command}"}}}}"#
                )
            })
            .collect();
        format!(
            r#"{{"type":"message","id":"{id}","parentId":"{parent}","message":{{"role":"assistant","content":[{}]}}}}"#,
            calls.join(",")
        )
    };
    let entries = [
        r#"{"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"Go"}}"#
            .to_owned(),
        response("a1", "u1", &["ls", "make"]),
        tool("t1", "a1", 10),
        result("r1", "t1"),
        tool("t2", "r1", 20),
        result("r2", "t2"),
        response("a2", "r2", &["npm test", "npm run lint"]),
        tool("t3", "a2", 30),
        tool("t4", "t3", 40),
        result("r3", "t4"),
        result("r4", "r3"),
        response("a3", "r4", &["./deploy.sh"]),
        tool("t5", "a3", 50),
        r#"{"type":"message","id":"u2","parentId":"t5","message":{"role":"user","content":"Again"}}"#
            .to_owned(),
        response("a4", "u2", &["git status"]),
        tool("t6", "a4", 60),
        result("r6", "t6"),
    ];
    let data: Value = serde_json::from_str(&format!(
        r#"{{"entries":[{}],"leafId":"r6"}}"#,
        entries.join(",")
    ))
    .expect("JSON");
    let history = History::from_entries(&data);
    let steps: Vec<_> = history
        .trajectory(None)
        .records
        .into_iter()
        .filter(|record| record.kind == RecordKind::Tool)
        .map(|record| (record.target, record.duration_ms))
        .collect();
    let step = |target: &str, duration: Option<u64>| (target.to_owned(), duration);
    assert_eq!(
        steps,
        [
            step("ls", Some(10)),
            step("make", Some(20)),
            step("npm test", None),
            step("npm run lint", None),
            step("git status", Some(60)),
        ],
        "a parallel call's timing could be another's, so it is unknown"
    );
    let chat: Vec<_> = history
        .transcript(None, 100)
        .items
        .into_iter()
        .filter_map(|item| Some((item.target?, item.duration_ms)))
        .filter(|(target, _)| target != "./deploy.sh")
        .collect();
    assert_eq!(chat, steps, "the chat tells the same as the Trajectory");
}

#[test]
fn reports_extension_errors_as_notifications() {
    assert_eq!(
        translate(&[
            r#"{"type":"extension_error","extensionPath":"/home/me/.pi/extensions/lint.ts","event":"tool_call","error":"boom"}"#
        ]),
        [AgentEvent::Notification {
            level: NotificationLevel::Error,
            message: "lint.ts failed in tool_call: boom".to_owned(),
        }]
    );
}

#[test]
fn shows_invisible_and_reordering_characters_in_what_a_call_shows() {
    use crate::pi::{call_target, changed_path};
    // Right-to-left override, zero-width space and carriage return; the target is the first
    // line.
    let command = r#"{"command":"rm -rf /\u202egnp.txt\u200b\r\n\techo\u0000"}"#;
    assert_eq!(
        call_target(command),
        "rm -rf /<U+202E>gnp.txt<U+200B><U+000D>"
    );
    // As the adapter receives them: parsed, with the characters themselves.
    let edit = serde_json::json!({"path": "a\u{2066}b.rs", "oldText": "x\u{2028}y"}).to_string();
    assert_eq!(
        changed_path("edit", &edit).as_deref(),
        Some("a<U+2066>b.rs")
    );
    // Characters Unicode lets renderers ignore: variation selectors, fillers, joiners and tags.
    let ignorable = r#"{"command":"rm\ufe0f \u3164-\u115f\u034f\u17b4\u180b\udb40\udd00"}"#;
    assert_eq!(
        call_target(ignorable),
        "rm<U+FE0F> <U+3164>-<U+115F><U+034F><U+17B4><U+180B><U+E0100>"
    );
}

#[test]
fn shows_tool_names_with_hiding_characters_written_out() {
    // An extension's tool name that reorders what follows it or breaks its line, live and as
    // stored.
    let events = translate(&[
        r#"{"type":"agent_start"}"#,
        r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"lint\u202e\n","args":{}}"#,
    ]);
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::ItemStarted { detail: ItemDetail::Tool { name, .. }, .. }
                if name == "lint<U+202E><U+000A>"
        )),
        "{events:?}"
    );
    let session: Value = serde_json::from_str(
        r#"{"entries":[
          {"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"Lint"}},
          {"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"lint\u202e\n","arguments":{}}]}},
          {"type":"message","id":"r1","parentId":"a1","message":{"role":"toolResult","toolCallId":"c1","toolName":"lint\u202e\n","content":[]}}
        ],"leafId":"r1"}"#,
    )
    .expect("JSON");
    let history = History::from_entries(&session);
    let chat: Vec<_> = history
        .transcript(None, 10)
        .items
        .into_iter()
        .filter_map(|item| item.tool)
        .collect();
    assert_eq!(chat, ["lint<U+202E><U+000A>"]);
    let steps: Vec<_> = history
        .trajectory(None)
        .records
        .into_iter()
        .filter(|record| record.label.starts_with("lint"))
        .map(|record| record.label)
        .collect();
    assert_eq!(steps, ["lint<U+202E><U+000A>"]);
}

#[test]
fn ends_the_items_an_abort_leaves_open() {
    let events = translate(&[
        r#"{"type":"agent_start"}"#,
        r#"{"type":"message_start","message":{"role":"assistant","content":[]}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":0}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Run"}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"ning"}}"#,
        r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{"command":"sleep 9"}}"#,
        r#"{"type":"agent_settled","aborted":true}"#,
    ]);
    let ended: Vec<_> = events
        .iter()
        .skip_while(|event| {
            !matches!(
                event,
                AgentEvent::ItemStarted {
                    item: ItemId(2),
                    ..
                }
            )
        })
        .skip(1)
        .cloned()
        .collect();
    let interrupted = |item, kind, text: &str| AgentEvent::ItemCompleted {
        turn: TURN,
        item: ItemId(item),
        kind,
        text: text.to_owned(),
        status: ItemStatus::Failed,
        duration_ms: None,
        lines: None,
    };
    assert_eq!(
        ended,
        [
            interrupted(1, ItemKind::AssistantMessage, "Running"),
            interrupted(2, ItemKind::CommandExecution, ""),
            AgentEvent::TurnAborted { turn: TURN },
        ],
        "nothing stays running after the abort"
    );
}
