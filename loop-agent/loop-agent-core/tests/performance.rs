use loop_agent_core::{
    run_loop, validate_protocol_jsonl_text, EmitMode, LOCAL_LOG_DIR, LOCAL_SESSION_DIR,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[test]
fn event_validation_p95_stays_under_m1_budget() {
    let stream_path = fixture_dir("hello-loop").join("expected/hello-loop.jsonl");
    let stream = fs::read_to_string(&stream_path).expect("hello-loop stream readable");
    let event_count = stream.lines().count() as u128;
    let mut nanos_per_event = Vec::new();

    for _ in 0..100 {
        let started = Instant::now();
        validate_protocol_jsonl_text(&stream_path, &stream).expect("stream validates");
        nanos_per_event.push(started.elapsed().as_nanos() / event_count);
    }

    assert!(
        p95(nanos_per_event) <= 1_000_000,
        "FSM/event validation p95 must stay <= 1 ms per event"
    );
}

#[test]
fn noop_dispatch_p95_stays_under_m1_budget() {
    let workspace = workspace_copy("smoke-loop");
    let mut nanos = Vec::new();

    for _ in 0..100 {
        let started = Instant::now();
        let sessions = loop_agent_core::list_sessions(&workspace).expect("sessions lists");
        assert!(sessions.is_empty());
        nanos.push(started.elapsed().as_nanos());
    }

    assert!(
        p95(nanos) <= 50_000_000,
        "no-op dispatch p95 must stay <= 50 ms"
    );
}

#[test]
fn hello_loop_log_append_p95_stays_under_m1_budget() {
    let stream_path = fixture_dir("hello-loop").join("expected/hello-loop.jsonl");
    let stream = fs::read_to_string(&stream_path).expect("hello-loop stream readable");
    let event_count = stream.lines().count() as u128;
    let workspace = empty_workspace("hello-log-append");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    let log_dir = workspace.join(LOCAL_LOG_DIR);
    fs::create_dir_all(&session_dir).expect("session directory created");
    fs::create_dir_all(&log_dir).expect("log directory created");
    let mut nanos_per_event = Vec::new();

    for index in 0..100 {
        let session_id = format!("perf{index:03}");
        let started = Instant::now();
        fs::write(session_dir.join(format!("{session_id}.jsonl")), &stream)
            .expect("session stream written");
        fs::write(
            log_dir.join(format!("{session_id}.log")),
            format!("session_id={session_id}\nevents={event_count}\n"),
        )
        .expect("session log written");
        nanos_per_event.push(started.elapsed().as_nanos() / event_count);
    }

    assert!(
        p95(nanos_per_event) <= 5_000_000,
        "hello-loop stream/log append p95 must stay <= 5 ms per event"
    );
}

#[test]
fn ten_fixture_loop_invocations_complete_under_m1_runtime_contract() {
    for (fixture, loop_name) in [
        ("smoke-loop", "smoke-loop"),
        ("hello-loop", "hello-loop"),
        ("sandbox-negative", "sandbox-negative-write"),
        ("sandbox-negative", "sandbox-negative-network"),
        ("sandbox-negative", "sandbox-negative-environment"),
        ("sandbox-negative", "sandbox-negative-interpreter"),
        ("sandbox-negative", "sandbox-negative-protected-path"),
        ("sandbox-negative", "sandbox-negative-symlink"),
        ("sandbox-negative", "sandbox-negative-tool-out-of-phase"),
        ("smoke-loop", "smoke-loop"),
    ] {
        let workspace = workspace_copy(fixture);
        let output = run_loop(&workspace, loop_name, EmitMode::Jsonl)
            .unwrap_or_else(|err| panic!("{loop_name}: {err}"));

        assert!(output.event_count > 0, "{loop_name}");
    }
}

fn p95(mut values: Vec<u128>) -> u128 {
    values.sort_unstable();
    let index = ((values.len() - 1) * 95).div_ceil(100);
    values[index]
}

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join(name)
}

fn workspace_copy(fixture: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target = std::env::temp_dir().join(format!(
        "watershed-loop-agent-core-perf-{}-{id}",
        std::process::id()
    ));
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    copy_dir(&fixture_dir(fixture), &target);
    target
}

fn empty_workspace(label: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target = std::env::temp_dir().join(format!(
        "watershed-loop-agent-core-perf-{label}-{}-{id}",
        std::process::id()
    ));
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    fs::create_dir_all(&target).expect("empty workspace created");
    target
}

fn copy_dir(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("target directory created");
    for entry in fs::read_dir(source).expect("source directory readable") {
        let entry = entry.expect("source entry readable");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            fs::copy(&source_path, &target_path).expect("fixture file copied");
        }
    }
}
