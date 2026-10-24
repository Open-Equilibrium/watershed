//! Observation workload for long Trajectories (PERFORMANCE.md, Liquid): a scripted agent answers
//! `get_entries` with a session of 1,000 turns of eight entries each and eight forks of ten turns,
//! and the core builds its tree and two Trajectories. It prints one JSON line per sample and a
//! summary and never compares with a threshold. Run from the repository root:
//!
//! ```sh
//! cargo build --release -p meta-harness-core --example trajectory_observe
//! /usr/bin/time -l target/release/examples/trajectory_observe
//! ```

use std::fmt::Write as _;
use std::time::Instant;

use meta_harness_core::{AgentCommand, Session};
use serde_json::{Value, json};

const TURNS: usize = 1000;
const FORKS: usize = 8;
const FORK_TURNS: usize = 10;
const SAMPLES: usize = 20;

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime starts");
    runtime.block_on(observe());
}

async fn observe() {
    let (entries, fork_leaf) = session();
    let data = serde_json::to_string(&entries).expect("JSON");
    // Only the agent's copy and the core's own allocations remain in the measured process.
    drop(entries);
    let dir = std::env::temp_dir().join(format!("trajectory-observe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temporary directory");
    let file = dir.join("entries.json");
    std::fs::write(&file, &data).expect("entries file");
    // Answers request `i` with the same stored entries.
    let script = format!(
        "i=1\nwhile IFS= read -r line; do\nprintf '%s' '{{\"id\":\"'$i'\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":'\ncat '{}'\nprintf '}}\\n'\ni=$((i+1))\ndone",
        file.display()
    );
    let command = AgentCommand {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: dir.clone(),
    };
    let (mut session, _events) = Session::start(&command).expect("agent starts");
    let mut totals = Vec::new();
    for sample in 0..SAMPLES {
        let started = Instant::now();
        let history = session.history().await.expect("history");
        let read = started.elapsed();
        let tree = history.tree();
        let built = started.elapsed();
        let current = history.trajectory(None);
        let projected = started.elapsed();
        let fork = history.trajectory(Some(&fork_leaf));
        let done = started.elapsed();
        let ms = |duration: std::time::Duration| duration.as_secs_f64() * 1000.0;
        println!(
            r#"{{"sample":{sample},"read_ms":{:.3},"tree_ms":{:.3},"trajectory_ms":{:.3},"fork_trajectory_ms":{:.3},"total_ms":{:.3},"nodes":{},"records":{},"fork_records":{}}}"#,
            ms(read),
            ms(built - read),
            ms(projected - built),
            ms(done - projected),
            ms(done),
            tree.nodes.len(),
            current.records.len(),
            fork.records.len(),
        );
        totals.push(ms(done));
    }
    session.shutdown(std::time::Duration::from_secs(5)).await;
    let _ = std::fs::remove_dir_all(&dir);
    totals.sort_by(f64::total_cmp);
    println!(
        r#"{{"summary":"trajectory","entries_bytes":{},"samples":{SAMPLES},"p50_ms":{:.3},"p95_ms":{:.3},"max_ms":{:.3}}}"#,
        data.len(),
        rank(&totals, 0.50),
        rank(&totals, 0.95),
        totals[totals.len() - 1],
    );
}

/// The nearest-rank percentile of sorted samples.
fn rank(sorted: &[f64], quantile: f64) -> f64 {
    let index = ((quantile * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[index - 1]
}

/// A `get_entries` result: the main branch of `TURNS` turns and `FORKS` forks, plus the leaf of
/// the last fork.
fn session() -> (Value, String) {
    let mut entries = Vec::new();
    let mut clock = 0;
    let mut fork_points = Vec::new();
    let mut parent = None;
    for turn in 0..TURNS {
        parent = Some(push_turn(
            &mut entries,
            &format!("m{turn}"),
            parent,
            &mut clock,
        ));
        if (turn + 1) % (TURNS / (FORKS + 1)) == 0 && fork_points.len() < FORKS {
            fork_points.push(parent.clone().expect("parent"));
        }
    }
    let leaf = parent.expect("main leaf");
    let mut fork_leaf = String::new();
    for (fork, point) in fork_points.into_iter().enumerate() {
        let mut parent = Some(point);
        for turn in 0..FORK_TURNS {
            parent = Some(push_turn(
                &mut entries,
                &format!("f{fork}t{turn}"),
                parent,
                &mut clock,
            ));
        }
        fork_leaf = parent.expect("fork leaf");
    }
    (json!({ "entries": entries, "leafId": leaf }), fork_leaf)
}

/// Appends one turn as pi and Watershed's extension store it: a prompt, a response with a tool
/// call, an approval, the tool result and a final response, with their timing entries.
fn push_turn(
    entries: &mut Vec<Value>,
    id: &str,
    parent: Option<String>,
    clock: &mut u64,
) -> String {
    let mut parent = parent;
    let mut push = |suffix: &str, body: Value| {
        *clock += 500;
        let entry_id = format!("{id}{suffix}");
        let mut entry = body;
        entry["id"] = json!(entry_id);
        entry["parentId"] = json!(parent);
        entry["timestamp"] = json!(timestamp(*clock));
        entries.push(entry);
        parent = Some(entry_id.clone());
        entry_id
    };
    let mut prompt = String::from("Add rate limiting to the login endpoint ");
    let _ = write!(prompt, "{id}");
    push(
        "u",
        json!({"type":"message","message":{"role":"user","content":[{"type":"text","text":prompt}]}}),
    );
    push(
        "ta",
        json!({"type":"custom","customType":"watershed.timing","data":{"kind":"request","firstTokenMs":700,"durationMs":2400}}),
    );
    push(
        "a",
        json!({"type":"message","message":{"role":"assistant","model":"faux-1","stopReason":"toolUse","usage":{"input":6140,"output":820,"cacheRead":0,"cacheWrite":0,"reasoning":214,"cost":{"total":0.02}},"content":[{"type":"text","text":"I'll run the tests."},{"type":"toolCall","id":format!("{id}c"),"name":"bash","arguments":{"command":"npm test -- login"}}]}}),
    );
    push(
        "p",
        json!({"type":"custom","customType":"watershed.approval","data":{"toolCallId":format!("{id}c"),"tool":"bash","decision":"allow-once","waitedMs":1200}}),
    );
    push(
        "tt",
        json!({"type":"custom","customType":"watershed.timing","data":{"kind":"tool","toolCallId":format!("{id}c"),"durationMs":900}}),
    );
    push(
        "r",
        json!({"type":"message","message":{"role":"toolResult","toolCallId":format!("{id}c"),"toolName":"bash","isError":false,"content":[{"type":"text","text":"14 passed"}]}}),
    );
    push(
        "tb",
        json!({"type":"custom","customType":"watershed.timing","data":{"kind":"request","firstTokenMs":500,"durationMs":1500}}),
    );
    push(
        "d",
        json!({"type":"message","message":{"role":"assistant","model":"faux-1","stopReason":"stop","usage":{"input":7410,"output":96,"cacheRead":6000,"cacheWrite":0,"reasoning":0,"cost":{"total":0.01}},"content":[{"type":"text","text":"Done."}]}}),
    )
}

/// An ISO 8601 UTC timestamp `millis` after 2026-10-24T00:00:00Z, within that day.
fn timestamp(millis: u64) -> String {
    let (seconds, millis) = (millis / 1000, millis % 1000);
    format!(
        "2026-10-24T{:02}:{:02}:{:02}.{millis:03}Z",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
