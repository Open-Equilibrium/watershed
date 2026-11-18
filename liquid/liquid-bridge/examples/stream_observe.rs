use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use liquid_bridge::api::events::BridgeEventKind;
use liquid_bridge::api::session::AgentSession;
use meta_harness_core::remote::Host;
use meta_harness_core::{
    AcpAgent, ModelChoice, Permissions, Service, ServiceConfig, StartThread, Workspace,
};

const CHUNKS: usize = 20_000;
const CHUNK_BYTES: usize = 16;
const SAMPLES: usize = 20;

fn main() {
    let dir = std::env::temp_dir().join(format!("liquid-stream-{}", std::process::id()));
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).expect("temporary directory");
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .is_ok_and(|status| status.success());
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&[
        "-c",
        "user.name=o",
        "-c",
        "user.email=o@o",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "start",
    ]);
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../meta-harness/meta-harness-core/tests/fixtures/scripted_acp_agent.py");
    let runtime = tokio::runtime::Runtime::new().expect("runtime starts");
    let home = dir.join("meta-harness");
    let service = runtime
        .block_on(Service::start(ServiceConfig {
            home: home.clone(),
            t3: PathBuf::from(std::env::var_os("META_HARNESS_T3").expect("META_HARNESS_T3")),
            acp_agents: vec![AcpAgent {
                id: "scripted".to_owned(),
                command: PathBuf::from("/usr/bin/python3"),
                args: vec![script.to_string_lossy().into_owned()],
                models: vec!["scripted".to_owned()],
            }],
        }))
        .expect("the service starts");
    let host = runtime
        .block_on(Host::connect(&home, "stream_observe"))
        .expect("the bridge connects");
    let project = runtime.block_on(host.project(&repo)).expect("project");
    let mut elapsed = Vec::new();
    for sample in 0..SAMPLES {
        let started = Instant::now();
        let thread = runtime
            .block_on(project.start(StartThread {
                text: format!("stream {CHUNKS}"),
                model: ModelChoice {
                    instance: "scripted".to_owned(),
                    model: "scripted".to_owned(),
                    options: Vec::new(),
                },
                workspace: Workspace::Checkout,
                permissions: Permissions::FullAccess,
                message_id: None,
            }))
            .expect("thread starts");
        let session = AgentSession::follow(thread, repo.to_string_lossy().into_owned());
        let (sender, receiver) = mpsc::channel();
        let snapshot = session
            .forward(move |event| sender.send(event).is_ok())
            .expect("the session follows");
        let mut text: usize = snapshot
            .transcript
            .iter()
            .filter(|item| {
                item.item.kind == liquid_bridge::api::events::BridgeItemKind::AssistantMessage
            })
            .map(|item| item.text.len())
            .sum();
        let (mut events, mut deltas, mut largest) = (0, 0, 0);
        let mut done = snapshot.turns.iter().any(|turn| turn.ended_ms.is_some());
        while !done {
            let event = receiver
                .recv_timeout(Duration::from_secs(120))
                .expect("event arrives");
            events += 1;
            session.confirm_applied(events);
            match event.kind {
                BridgeEventKind::ContentDelta => {
                    let length = event.text.map_or(0, |text| text.len());
                    deltas += 1;
                    largest = largest.max(length);
                    text += length;
                }
                BridgeEventKind::TurnCompleted => done = true,
                _ => {}
            }
        }
        let millis = started.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(text, CHUNKS * CHUNK_BYTES, "the whole stream arrived");
        println!(
            r#"{{"sample":{sample},"elapsed_ms":{millis:.3},"events":{events},"delta_events":{deltas},"largest_delta_bytes":{largest}}}"#
        );
        elapsed.push(millis);
    }
    runtime.block_on(service.stop());
    let _ = std::fs::remove_dir_all(&dir);
    elapsed.sort_by(f64::total_cmp);
    println!(
        r#"{{"summary":"stream","chunks_in":{CHUNKS},"chunk_bytes":{CHUNK_BYTES},"samples":{SAMPLES},"p50_ms":{:.3},"p95_ms":{:.3},"max_ms":{:.3}}}"#,
        rank(&elapsed, 0.50),
        rank(&elapsed, 0.95),
        elapsed[elapsed.len() - 1],
    );
}

fn rank(sorted: &[f64], quantile: f64) -> f64 {
    let index = ((quantile * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[index - 1]
}
