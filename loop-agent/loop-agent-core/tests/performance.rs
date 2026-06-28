use loop_agent_core::{
    run_loop, validate_protocol_jsonl_text, EmitMode, LOCAL_LOG_DIR, LOCAL_SESSION_DIR,
};
use std::{
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Barrier,
    },
    thread,
    time::{Duration, Instant},
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
    let workspace = workspace_copy("hello-loop");
    let mut nanos_per_event = Vec::new();

    for _ in 0..100 {
        clear_runtime_state(&workspace);
        let started = Instant::now();
        let output =
            run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("hello-loop succeeds");
        nanos_per_event.push(started.elapsed().as_nanos() / event_count);
        assert!(!output.failed);
        assert_eq!(output.event_count as u128, event_count);
        assert!(output.session_path.exists());
        assert!(workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
    }

    assert!(
        p95(nanos_per_event) <= 5_000_000,
        "hello-loop stream/log append p95 must stay <= 5 ms per event"
    );
}

#[test]
fn ten_fixture_loop_invocations_complete_under_m1_runtime_contract() {
    let workspaces = [
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
    ]
    .into_iter()
    .map(|(fixture, loop_name)| (workspace_copy(fixture), loop_name))
    .collect::<Vec<_>>();
    let concurrency = workspaces.len();
    let barrier = Arc::new(Barrier::new(concurrency + 1));
    let (tx, rx) = mpsc::channel();
    let handles = workspaces
        .into_iter()
        .map(|(workspace, loop_name)| {
            let barrier = Arc::clone(&barrier);
            let tx = tx.clone();
            thread::spawn(move || {
                barrier.wait();
                let result = run_loop(&workspace, loop_name, EmitMode::Jsonl)
                    .map(|output| {
                        (
                            output.event_count,
                            output.failed,
                            output.stdout.len(),
                            dir_size(&workspace),
                        )
                    })
                    .map_err(|err| err.to_string());
                tx.send((loop_name, result)).expect("result sent");
            })
        })
        .collect::<Vec<_>>();
    drop(tx);

    let started = Instant::now();
    barrier.wait();
    let timeout = Duration::from_secs(30);
    let mut total_stdout_bytes = 0usize;
    let mut total_workspace_bytes = 0u64;
    for _ in 0..concurrency {
        let remaining = timeout.saturating_sub(started.elapsed());
        let (loop_name, result) = rx
            .recv_timeout(remaining)
            .expect("10 concurrent fixture loops complete before timeout");
        let (event_count, failed, stdout_bytes, workspace_bytes) =
            result.unwrap_or_else(|err| panic!("{loop_name}: {err}"));
        assert!(event_count > 0, "{loop_name}");
        total_stdout_bytes += stdout_bytes;
        total_workspace_bytes += workspace_bytes;
        assert!(
            failed == loop_name.contains("sandbox-negative"),
            "{loop_name} failure state must match fixture kind"
        );
    }
    for handle in handles {
        handle.join().expect("worker thread joins");
    }
    assert!(
        started.elapsed() <= timeout,
        "10 concurrent fixture loops must complete within {timeout:?}"
    );
    assert!(
        total_stdout_bytes < 512 * 1024,
        "concurrent fixture stdout must stay bounded: {total_stdout_bytes} bytes"
    );
    assert!(
        total_workspace_bytes < 64 * 1024 * 1024,
        "concurrent fixture workspaces must stay bounded: {total_workspace_bytes} bytes"
    );
}

#[test]
fn temp_workspace_guard_removes_directory_on_drop() {
    let path = {
        let workspace = empty_workspace("cleanup");
        let path = workspace.path().to_path_buf();
        assert!(path.exists());
        path
    };

    assert!(!path.exists(), "temporary workspace should be removed");
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

struct TempWorkspace {
    path: PathBuf,
}

impl TempWorkspace {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Deref for TempWorkspace {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for TempWorkspace {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn workspace_copy(fixture: &str) -> TempWorkspace {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target = std::env::temp_dir().join(format!(
        "watershed-loop-agent-core-perf-{}-{id}",
        std::process::id()
    ));
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    copy_dir(&fixture_dir(fixture), &target);
    TempWorkspace::new(target)
}

fn empty_workspace(label: &str) -> TempWorkspace {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target = std::env::temp_dir().join(format!(
        "watershed-loop-agent-core-perf-{label}-{}-{id}",
        std::process::id()
    ));
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    fs::create_dir_all(&target).expect("empty workspace created");
    TempWorkspace::new(target)
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

fn dir_size(path: &Path) -> u64 {
    fs::read_dir(path)
        .expect("directory readable")
        .map(|entry| {
            let path = entry.expect("directory entry readable").path();
            if path.is_dir() {
                dir_size(&path)
            } else {
                fs::metadata(&path).expect("file metadata readable").len()
            }
        })
        .sum()
}

fn clear_runtime_state(workspace: &Path) {
    let _ = fs::remove_dir_all(workspace.join(LOCAL_SESSION_DIR));
    let _ = fs::remove_dir_all(workspace.join(LOCAL_LOG_DIR));
}
