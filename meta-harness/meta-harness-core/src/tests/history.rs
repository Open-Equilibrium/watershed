use serde_json::Value;

use crate::History;

#[test]
fn sums_the_usage_of_a_session_read_from_its_file() {
    let file = std::env::temp_dir().join(format!("watershed-usage-{}.jsonl", std::process::id()));
    std::fs::write(
        &file,
        r#"{"type":"session","version":3,"id":"s1","cwd":"/tmp"}
{"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"Go"}}
{"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[],"usage":{"input":7,"output":2,"cost":{"total":0.001}}}}
{"type":"compaction","id":"c1","parentId":"a1","firstKeptEntryId":"a1","summary":"s","usage":{"input":30,"output":1,"cost":{"total":0}}}
"#,
    )
    .expect("session file");
    let history = History::from_file(&file, 16 * 1024 * 1024);
    let _ = std::fs::remove_file(&file);
    let usage = history.expect("history").usage();
    assert_eq!((usage.input, usage.output), (37, 3));
}

#[test]
fn an_entry_too_large_to_read_keeps_its_place_in_the_branch() {
    let file = std::env::temp_dir().join(format!("watershed-gap-{}.jsonl", std::process::id()));
    let huge = "x".repeat(17_000_000);
    std::fs::write(
        &file,
        format!(
            r#"{{"type":"session","version":3,"id":"s1","cwd":"/tmp"}}
{{"type":"message","id":"u1","parentId":null,"message":{{"role":"user","content":"Look"}}}}
{{"type":"message","id":"r1","parentId":"u1","message":{{"role":"toolResult","toolCallId":"c1","toolName":"read","content":[{{"type":"image","data":"{huge}"}}]}}}}
{{"type":"message","id":"a1","parentId":"r1","message":{{"role":"assistant","content":[{{"type":"text","text":"Seen."}}]}}}}
"#
        ),
    )
    .expect("session file");
    let history = History::from_file(&file, 16 * 1024 * 1024);
    let _ = std::fs::remove_file(&file);
    assert_eq!(
        nodes(history.expect("history")),
        [
            node("u1", None),
            node("r1", Some("u1")),
            node("a1", Some("r1"))
        ],
        "the skipped entry keeps its id and parent, so the entries after it reach the prompt"
    );
}

#[test]
fn a_custom_entry_too_large_to_read_keeps_its_place_in_the_branch() {
    // pi writes a custom entry's data before its id and parent.
    let file =
        std::env::temp_dir().join(format!("watershed-custom-gap-{}.jsonl", std::process::id()));
    let huge = "x".repeat(17_000_000);
    std::fs::write(
        &file,
        format!(
            r#"{{"type":"session","version":3,"id":"s1","cwd":"/tmp"}}
{{"type":"message","id":"u1","parentId":null,"message":{{"role":"user","content":"Look"}}}}
{{"type":"custom","customType":"big","data":{{"id":"inner","blob":"{huge}"}},"id":"c1","parentId":"u1","timestamp":"2026-10-24T10:00:00.000Z"}}
{{"type":"message","id":"a1","parentId":"c1","message":{{"role":"assistant","content":[{{"type":"text","text":"Seen."}}]}}}}
"#
        ),
    )
    .expect("session file");
    let history = History::from_file(&file, 16 * 1024 * 1024);
    let _ = std::fs::remove_file(&file);
    assert_eq!(
        nodes(history.expect("history")),
        [
            node("u1", None),
            node("c1", Some("u1")),
            node("a1", Some("c1"))
        ],
        "the skipped custom entry keeps its id and parent, so the entries after it reach the prompt"
    );
}

#[test]
fn an_oversized_custom_message_keeps_its_place_in_the_branch() {
    // A custom message starts like a custom entry, but its id and parent come first.
    let file = std::env::temp_dir().join(format!(
        "watershed-custom-message-gap-{}.jsonl",
        std::process::id()
    ));
    let huge = "x".repeat(17_000_000);
    std::fs::write(
        &file,
        format!(
            r#"{{"type":"session","version":3,"id":"s1","cwd":"/tmp"}}
{{"type":"message","id":"u1","parentId":null,"message":{{"role":"user","content":"Look"}}}}
{{"type":"custom_message","id":"cm","parentId":"u1","timestamp":"2026-10-24T10:00:00.000Z","customType":"reminder","content":"{huge}","display":true}}
{{"type":"message","id":"a1","parentId":"cm","message":{{"role":"assistant","content":[{{"type":"text","text":"Seen."}}]}}}}
"#
        ),
    )
    .expect("session file");
    let history = History::from_file(&file, 16 * 1024 * 1024);
    let _ = std::fs::remove_file(&file);
    assert_eq!(
        nodes(history.expect("history")),
        [
            node("u1", None),
            node("cm", Some("u1")),
            node("a1", Some("cm"))
        ],
        "the skipped custom message keeps its id and parent, so the entries after it reach the \
         prompt"
    );
}

#[test]
fn sums_usage_pi_records_outside_responses() {
    let usage = |input: u64| format!(r#"{{"input":{input},"output":1,"cost":{{"total":0.001}}}}"#);
    let data: Value = serde_json::from_str(&format!(
        r#"{{"entries":[
          {{"type":"message","id":"u1","parentId":null,"message":{{"role":"user","content":"Go"}}}},
          {{"type":"message","id":"a1","parentId":"u1","message":{{"role":"assistant","content":[],"usage":{}}}}},
          {{"type":"compaction","id":"c1","parentId":"a1","firstKeptEntryId":"a1","summary":"s","usage":{}}},
          {{"type":"branch_summary","id":"b1","parentId":"c1","summary":"s","usage":{}}},
          {{"type":"usage","id":"w1","parentId":"b1","kind":"cache_warm","usage":{}}},
          {{"type":"message","id":"r1","parentId":"w1","message":{{"role":"toolResult","toolCallId":"t","toolName":"task","content":[],"usage":{}}}}}
        ],"leafId":"r1"}}"#,
        usage(1),
        usage(10),
        usage(100),
        usage(1000),
        usage(10000)
    ))
    .expect("JSON");
    let total = History::from_entries(&data).usage();
    assert_eq!(
        (total.input, total.output, total.cost_micro_usd),
        (11111, 5, 5000),
        "summaries, cache warming and tools cost too"
    );
}

#[test]
fn sums_what_every_stored_response_used() {
    let data: Value = serde_json::from_str(
        r#"{"entries":[
          {"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"Go"}},
          {"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[],"usage":{"input":100,"output":20,"cacheRead":900,"cacheWrite":10,"cost":{"total":0.01}}}},
          {"type":"message","id":"a2","parentId":"u1","message":{"role":"assistant","content":[],"usage":{"input":5,"output":2,"cacheRead":0,"cacheWrite":0,"cost":{"total":0.002}}}}
        ],"leafId":"a2"}"#,
    )
    .expect("JSON");
    let usage = History::from_entries(&data).usage();
    assert_eq!(
        (
            usage.input,
            usage.output,
            usage.cache_read,
            usage.cache_write,
            usage.cost_micro_usd
        ),
        (105, 22, 900, 10, 12_000),
        "abandoned branches cost too"
    );
}

#[test]
fn a_parent_cycle_ends_the_branch_instead_of_hanging() {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let data: Value = serde_json::from_str(
            r#"{"entries":[
          {"type":"message","id":"u1","parentId":"a1","message":{"role":"user","content":"Go"}},
          {"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[]}},
          {"type":"label","id":"l1","parentId":"l2","targetId":"a1","label":"A"},
          {"type":"label","id":"l2","parentId":"l1","targetId":"u1","label":"B"},
          {"type":"message","id":"m1","parentId":"l2","message":{"role":"user","content":"More"}}
        ],"leafId":"a1"}"#,
        )
        .expect("JSON");
        let history = History::from_entries(&data);
        let chat: Vec<String> = history
            .transcript(None, 10)
            .items
            .into_iter()
            .map(|item| item.entry)
            .collect();
        let _ = done.send((chat, nodes(history)));
    });
    let (chat, tree) = finished
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("a stored cycle must not hang the reader");
    assert_eq!(chat, ["u1"], "the branch's prompt is shown once");
    assert_eq!(
        tree,
        [
            node("u1", Some("a1")),
            node("a1", Some("u1")),
            node("m1", None)
        ],
        "every entry but the labels is a node, and labels that lead back to each other lead \
         nowhere"
    );
}

#[test]
fn reads_numbers_out_of_range_without_overflowing() {
    let max = u64::MAX;
    let response = |id: &str, parent: &str, timestamp: &str| {
        format!(
            r#"{{"type":"message","id":"{id}","parentId":{parent},"timestamp":"{timestamp}","message":{{"role":"assistant","model":"m","stopReason":"stop","usage":{{"input":{max},"output":{max},"cacheRead":{max},"cacheWrite":{max},"reasoning":{max},"totalTokens":0,"cost":{{"total":1e300}}}},"content":[{{"type":"text","text":"x"}}]}}}}"#
        )
    };
    let session = format!(
        r#"{{"entries":[{},{}],"leafId":"a2"}}"#,
        response("a1", "null", "99999999999999-13-99T99:99:99Z"),
        response("a2", r#""a1""#, "2026-10-24T10:00:00Z"),
    );
    let history = History::from_entries(&serde_json::from_str::<Value>(&session).expect("JSON"));
    assert_eq!(
        history.usage().input,
        u64::MAX,
        "the sum stops at the largest number"
    );
    assert_eq!(history.usage().cost_micro_usd, u64::MAX);
    assert_eq!(history.tree().nodes.len(), 2);
}

/// A branch-tree node as `(id, parent)`.
fn node(id: &str, parent: Option<&str>) -> (String, Option<String>) {
    (id.to_owned(), parent.map(str::to_owned))
}

/// Every node of the session's tree as `(id, parent)`.
fn nodes(history: History) -> Vec<(String, Option<String>)> {
    history
        .tree()
        .nodes
        .into_iter()
        .map(|node| (node.id, node.parent))
        .collect()
}
