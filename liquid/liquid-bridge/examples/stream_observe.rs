//! Observation workload for Liquid's token streaming (PERFORMANCE.md, Liquid): a scripted agent
//! streams one response of 20,000 text deltas through the Meta-Harness core and the bridge to a
//! consumer that takes each event at once. It prints one JSON line per sample and a summary and
//! never compares with a threshold. Run from the repository root:
//!
//! ```sh
//! cargo build --release -p liquid_bridge --example stream_observe
//! /usr/bin/time -l target/release/examples/stream_observe
//! ```

use std::fmt::Write as _;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use liquid_bridge::api::events::BridgeEventKind;
use liquid_bridge::api::session::{AgentSession, SessionConfig};

const DELTAS: usize = 20_000;
const DELTA: &str = "sixteen bytes ok";
const SAMPLES: usize = 20;

fn main() {
    let dir = std::env::temp_dir().join(format!("liquid-stream-observe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temporary directory");
    let stream = dir.join("stream.jsonl");
    std::fs::write(&stream, agent_output()).expect("stream file");
    let script = format!(
        "IFS= read -r line\ncat '{}'\nIFS= read -r line || exit 0",
        stream.display()
    );
    let mut elapsed = Vec::new();
    for sample in 0..SAMPLES {
        let session = AgentSession::start(SessionConfig {
            program: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script.clone()],
            cwd: dir.display().to_string(),
        })
        .expect("agent starts");
        let (sender, receiver) = mpsc::channel();
        session
            .forward(move |event| sender.send(event).is_ok())
            .expect("subscriber");
        let started = Instant::now();
        session.send("stream".to_owned(), false).expect("send");
        let (mut events, mut deltas, mut largest, mut text) = (0, 0, 0, 0);
        loop {
            let event = receiver
                .recv_timeout(Duration::from_secs(60))
                .expect("event arrives");
            events += 1;
            match event.kind {
                BridgeEventKind::ContentDelta => {
                    let length = event.text.map_or(0, |text| text.len());
                    deltas += 1;
                    largest = largest.max(length);
                    text += length;
                }
                BridgeEventKind::TurnCompleted => break,
                _ => {}
            }
        }
        let millis = started.elapsed().as_secs_f64() * 1000.0;
        session.shutdown();
        assert_eq!(text, DELTAS * DELTA.len(), "the whole stream arrived");
        println!(
            r#"{{"sample":{sample},"elapsed_ms":{millis:.3},"events":{events},"delta_events":{deltas},"largest_delta_bytes":{largest}}}"#
        );
        elapsed.push(millis);
    }
    let _ = std::fs::remove_dir_all(&dir);
    elapsed.sort_by(f64::total_cmp);
    println!(
        r#"{{"summary":"stream","deltas_in":{DELTAS},"delta_bytes":{},"samples":{SAMPLES},"p50_ms":{:.3},"p95_ms":{:.3},"max_ms":{:.3}}}"#,
        DELTA.len(),
        rank(&elapsed, 0.50),
        rank(&elapsed, 0.95),
        elapsed[elapsed.len() - 1],
    );
}

/// The nearest-rank percentile of sorted samples.
fn rank(sorted: &[f64], quantile: f64) -> f64 {
    let index = ((quantile * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[index - 1]
}

/// pi's RPC output for one prompt that streams `DELTAS` text deltas.
fn agent_output() -> String {
    let mut out = String::new();
    for line in [
        r#"{"id":"1","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}"#,
        r#"{"type":"agent_start"}"#,
        r#"{"type":"turn_start"}"#,
        r#"{"type":"message_start","message":{"role":"assistant","content":[]}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":0}}"#,
    ] {
        out.push_str(line);
        out.push('\n');
    }
    for _ in 0..DELTAS {
        let _ = writeln!(
            out,
            r#"{{"type":"message_update","assistantMessageEvent":{{"type":"text_delta","contentIndex":0,"delta":"{DELTA}"}}}}"#
        );
    }
    for line in [
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_end","contentIndex":0,"content":""}}"#,
        r#"{"type":"message_end","message":{"role":"assistant","content":[]}}"#,
        r#"{"type":"turn_end","message":{"role":"assistant","content":[]},"toolResults":[]}"#,
        r#"{"type":"agent_end","messages":[],"willRetry":false}"#,
        r#"{"type":"agent_settled","aborted":false}"#,
    ] {
        out.push_str(line);
        out.push('\n');
    }
    out
}
