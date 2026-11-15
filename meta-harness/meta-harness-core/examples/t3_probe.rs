//! Spike probe: one T3 thread end to end through Meta-Harness's T3 client.
//! Usage: t3_probe <t3-binary> <t3-home> <project-dir> <instance-id> <model> <prompt>

use meta_harness_core::t3::{T3Client, T3Project, T3Server};
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let [_, binary, home, project, instance, model, prompt] = args.as_slice() else {
        return Err("usage: t3_probe <t3> <home> <project> <instance> <model> <prompt>".into());
    };
    let server = T3Server::start(Path::new(binary), Path::new(home), Path::new(project)).await?;
    println!("server {}", server.origin);
    let client = T3Client::connect(&server.origin, server.token()).await?;
    let t3 = T3Project::open(client, project).await?;
    println!("project {}", t3.project_id);
    let thread = t3
        .launch_thread(
            instance,
            model,
            prompt,
            std::env::var("T3_WORKTREE_BASE").ok().as_deref(),
        )
        .await?;
    let mut stream = t3.subscribe(&thread).await?;
    let mut last = 0u64;
    let (mut events, mut deltas) = (0, 0);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(240);
    while let Ok(Some(item)) = tokio::time::timeout_at(deadline, stream.recv()).await {
        match item["kind"].as_str() {
            Some("snapshot") => {
                last = item["snapshotSequence"].as_u64().unwrap_or_default();
                println!("snapshot at sequence {last}");
            }
            Some("synchronized") => println!("synchronized"),
            Some("event") => {
                let sequence = item["sequence"].as_u64().unwrap_or_default();
                if sequence <= last {
                    continue; // already in the snapshot (ADR-0290)
                }
                last = sequence;
                events += 1;
                let event = &item["event"];
                if let Ok(path) = std::env::var("T3_PROBE_DUMP") {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                    {
                        let _ = writeln!(f, "{item}");
                    }
                }
                let kind = event["type"].as_str().unwrap_or("?");
                if kind == "turn-item.updated" || kind == "message.updated" {
                    deltas += 1;
                }
                if kind == "run.updated" {
                    let status = event["payload"]["status"].as_str().unwrap_or("?");
                    println!("run {status} at sequence {sequence}");
                    if matches!(status, "completed" | "failed" | "interrupted" | "cancelled") {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    println!("{events} live events, {deltas} message or item updates");
    let (sequence, projection) = t3.projection(&thread).await?;
    let reply = projection["messages"]
        .as_array()
        .and_then(|m| m.iter().rev().find(|m| m["role"] == "assistant"))
        .map(|m| m["text"].to_string())
        .unwrap_or_default();
    println!("reply {reply}");
    for turn in projection["providerTurns"].as_array().into_iter().flatten() {
        println!(
            "provider turn {} {}: usage {} context {}",
            turn["ordinal"], turn["status"], turn["turnTokenUsage"], turn["tokenUsage"]
        );
    }
    println!("checkpoints at sequence {sequence}:");
    for c in projection["checkpoints"].as_array().into_iter().flatten() {
        println!(
            "  run {} ordinal {} status {} files {}",
            c["appRunOrdinal"], c["ordinalWithinScope"], c["status"], c["files"]
        );
    }
    let diff = t3.turn_diff(&thread, 0, 1).await?;
    println!("turn 1 diff:\n{diff}");
    t3.revert_to(&thread, &projection, 0).await?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let (_, after) = t3.projection(&thread).await?;
    println!(
        "reverted to before turn 1 in {}; diff now: {:?}",
        after["thread"]["worktreePath"],
        t3.turn_diff(&thread, 0, 1).await.map(|d| d.len())
    );
    server.stop().await;
    Ok(())
}
