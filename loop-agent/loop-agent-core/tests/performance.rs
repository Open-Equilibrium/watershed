use loop_agent_core::{
    run_loop, validate_protocol_jsonl_text, EmitMode, MAX_LOOP_EVENT_STREAM_BYTES,
};
use proto::{EventEnvelope, EventType};
use std::{
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc, Arc, Barrier, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant},
};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);
static PERFORMANCE_TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn near_cap_event_validation_enforces_m1_memory_budget() {
    let _guard = performance_test_guard();
    let stream = near_cap_valid_event_stream();
    assert!(
        stream.len() <= MAX_LOOP_EVENT_STREAM_BYTES,
        "near-cap stream must stay inside the runtime budget"
    );
    assert!(
        stream.len() >= MAX_LOOP_EVENT_STREAM_BYTES - 8 * 1024,
        "near-cap stream should exercise the budget boundary: {} bytes",
        stream.len()
    );

    let started = Instant::now();
    let events = validate_protocol_jsonl_text(Path::new("near-cap.jsonl"), &stream)
        .expect("near-cap stream validates");
    assert_eq!(events.len(), 9);
    assert!(
        started.elapsed() <= Duration::from_secs(5),
        "near-cap validation should remain bounded"
    );

    let oversized = format!("{stream}{}\n", "x".repeat(16 * 1024));
    let err = validate_protocol_jsonl_text(Path::new("oversized-near-cap.jsonl"), &oversized)
        .expect_err("oversized stream must fail the event-stream budget");
    assert!(err.to_string().contains("event stream budget"), "{err}");
}

#[test]
fn ten_successful_fixture_loop_invocations_complete_under_m1_runtime_contract() {
    let _guard = performance_test_guard();
    let peak_rss_sampler = if rss_budget_must_be_enforced() {
        let baseline = current_resident_set_size()
            .expect("RSS measurement must be available on this enforced target before the run");
        Some(PeakRssSampler::start(baseline))
    } else {
        None
    };
    let workspaces = [
        ("smoke-loop", "smoke-loop"),
        ("hello-loop", "hello-loop"),
        ("smoke-loop", "smoke-loop"),
        ("hello-loop", "hello-loop"),
        ("smoke-loop", "smoke-loop"),
        ("hello-loop", "hello-loop"),
        ("smoke-loop", "smoke-loop"),
        ("hello-loop", "hello-loop"),
        ("smoke-loop", "smoke-loop"),
        ("hello-loop", "hello-loop"),
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
        let elapsed = started.elapsed();
        assert!(
            elapsed < timeout,
            "10 concurrent successful fixture loops must complete within {timeout:?}"
        );
        let remaining = timeout - elapsed;
        let (loop_name, result) = rx
            .recv_timeout(remaining)
            .expect("10 concurrent successful fixture loops complete before timeout");
        let (event_count, failed, stdout_bytes, workspace_bytes) =
            result.unwrap_or_else(|err| panic!("{loop_name}: {err}"));
        assert!(event_count > 0, "{loop_name}");
        total_stdout_bytes += stdout_bytes;
        total_workspace_bytes += workspace_bytes;
        assert!(!failed, "{loop_name} should complete successfully");
    }
    for handle in handles {
        handle.join().expect("worker thread joins");
    }
    assert!(
        started.elapsed() <= timeout,
        "10 concurrent successful fixture loops must complete within {timeout:?}"
    );
    assert!(
        total_stdout_bytes < 512 * 1024,
        "concurrent fixture stdout must stay bounded: {total_stdout_bytes} bytes"
    );
    assert!(
        total_workspace_bytes < 64 * 1024 * 1024,
        "concurrent fixture workspaces must stay bounded: {total_workspace_bytes} bytes"
    );
    if let Some(mut sampler) = peak_rss_sampler {
        let baseline = sampler.baseline();
        let peak_growth = sampler.finish().saturating_sub(baseline);
        let per_loop_budget = 10 * 1024 * 1024;
        let budget = per_loop_budget * concurrency as u64;
        assert!(
            peak_growth <= budget,
            "concurrent fixture peak RSS growth must stay <= {per_loop_budget} bytes per active top-level loop ({budget} bytes total): {peak_growth} bytes"
        );
    }
}

fn performance_test_guard() -> MutexGuard<'static, ()> {
    PERFORMANCE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct PeakRssSampler {
    baseline: u64,
    peak: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl PeakRssSampler {
    fn start(baseline: u64) -> Self {
        let peak = Arc::new(AtomicU64::new(baseline));
        let running = Arc::new(AtomicBool::new(true));
        let sampler_peak = Arc::clone(&peak);
        let sampler_running = Arc::clone(&running);
        let handle = thread::spawn(move || {
            // Sample while workers are live: post-join RSS deltas can miss transient peaks.
            while sampler_running.load(Ordering::Acquire) {
                if let Some(current) = current_resident_set_size() {
                    sampler_peak.fetch_max(current, Ordering::AcqRel);
                }
                thread::sleep(Duration::from_millis(1));
            }
            if let Some(current) = current_resident_set_size() {
                sampler_peak.fetch_max(current, Ordering::AcqRel);
            }
        });

        Self {
            baseline,
            peak,
            running,
            handle: Some(handle),
        }
    }

    fn baseline(&self) -> u64 {
        self.baseline
    }

    fn finish(&mut self) -> u64 {
        self.stop();
        self.peak.load(Ordering::Acquire)
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("RSS sampler joins");
        }
    }
}

impl Drop for PeakRssSampler {
    fn drop(&mut self) {
        self.stop();
    }
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

fn near_cap_valid_event_stream() -> String {
    let base = event_stream_with_message_content("");
    let target_len = MAX_LOOP_EVENT_STREAM_BYTES - 4 * 1024;
    let padding_len = target_len.saturating_sub(base.len());
    event_stream_with_message_content(&"x".repeat(padding_len))
}

fn event_stream_with_message_content(content: &str) -> String {
    [
        perf_event_line(
            1,
            EventType::SessionStarted,
            None,
            serde_json::json!({"reason":"fixture-start"}),
        ),
        perf_event_line(
            2,
            EventType::LoopStarted,
            Some("loop-001"),
            serde_json::json!({"loop_definition_id":"near-cap-loop","loop_name":"NearCap"}),
        ),
        perf_event_line(
            3,
            EventType::PhaseEntered,
            Some("loop-001"),
            serde_json::json!({
                "instruction_ids": [],
                "phase_id": "phase",
                "phase_name": "Phase",
                "tool_ids": [],
            }),
        ),
        perf_event_line(
            4,
            EventType::StepStarted,
            Some("loop-001"),
            serde_json::json!({
                "phase_id": "phase",
                "step_id": "step",
                "step_name": "Step",
            }),
        ),
        perf_event_line(
            5,
            EventType::MessageDelta,
            Some("loop-001"),
            serde_json::json!({
                "content_delta": content,
                "message_id": "msg-001",
                "role": "assistant",
            }),
        ),
        perf_event_line(
            6,
            EventType::MessageCompleted,
            Some("loop-001"),
            serde_json::json!({
                "message_id": "msg-001",
                "role": "assistant",
            }),
        ),
        perf_event_line(
            7,
            EventType::StepCompleted,
            Some("loop-001"),
            serde_json::json!({
                "phase_id": "phase",
                "step_id": "step",
                "step_name": "Step",
            }),
        ),
        perf_event_line(
            8,
            EventType::LoopCompleted,
            Some("loop-001"),
            serde_json::json!({"loop_definition_id":"near-cap-loop","loop_name":"NearCap"}),
        ),
        perf_event_line(9, EventType::SessionCompleted, None, serde_json::json!({})),
    ]
    .join("")
}

fn perf_event_line(
    sequence: u64,
    event_type: EventType,
    loop_id: Option<&str>,
    payload: serde_json::Value,
) -> String {
    let mut event = EventEnvelope::new(
        format!("evt-{sequence:03}"),
        event_type,
        "nearcap001",
        sequence,
        format!("2026-01-01T00:00:{:02}Z", sequence - 1),
        "loop-agent-cli",
        payload,
    );
    event.loop_id = loop_id.map(str::to_owned);
    event.canonical_jsonl().expect("event serializes")
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
    copy_fixture_workspace(&fixture_dir(fixture), &target);
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
        if source_path.is_dir() && entry.file_name() == ".loop" {
            continue;
        }
        if source_path.is_dir() && entry.file_name() == "out" {
            fs::create_dir_all(&target_path).expect("output directory shape copied");
            continue;
        }
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            fs::copy(&source_path, &target_path).expect("fixture file copied");
        }
    }
}

fn copy_fixture_workspace(source: &Path, target: &Path) {
    copy_dir(source, target);
    copy_workspace_config(source, target);
}

fn copy_workspace_config(source: &Path, target: &Path) {
    let source_config = source.join(".loop/config.yaml");
    if !source_config.exists() {
        return;
    }
    let target_config = target.join(".loop/config.yaml");
    fs::create_dir_all(target_config.parent().expect("config path has parent"))
        .expect("workspace config directory created");
    fs::copy(source_config, target_config).expect("workspace config copied");
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

fn rss_budget_must_be_enforced() -> bool {
    cfg!(any(target_os = "linux", target_os = "macos", windows))
}

#[cfg(target_os = "linux")]
fn current_resident_set_size() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?.trim();
        let kilobytes = value.strip_suffix(" kB")?.trim().parse::<u64>().ok()?;
        Some(kilobytes * 1024)
    })
}

#[cfg(windows)]
fn current_resident_set_size() -> Option<u64> {
    use std::ffi::c_void;
    use std::mem;

    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "psapi")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut ProcessMemoryCounters,
            size: u32,
        ) -> i32;
    }

    let mut counters = ProcessMemoryCounters {
        cb: mem::size_of::<ProcessMemoryCounters>() as u32,
        page_fault_count: 0,
        peak_working_set_size: 0,
        working_set_size: 0,
        quota_peak_paged_pool_usage: 0,
        quota_paged_pool_usage: 0,
        quota_peak_non_paged_pool_usage: 0,
        quota_non_paged_pool_usage: 0,
        pagefile_usage: 0,
        peak_pagefile_usage: 0,
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            mem::size_of::<ProcessMemoryCounters>() as u32,
        )
    };
    (ok != 0).then_some(counters.working_set_size as u64)
}

#[cfg(target_os = "macos")]
fn current_resident_set_size() -> Option<u64> {
    use std::mem;

    type MachMsgTypeNumber = u32;
    type MachPort = u32;

    #[repr(C)]
    struct TimeValue {
        seconds: i32,
        microseconds: i32,
    }

    #[repr(C)]
    struct MachTaskBasicInfo {
        virtual_size: u64,
        resident_size: u64,
        resident_size_max: u64,
        user_time: TimeValue,
        system_time: TimeValue,
        policy: i32,
        suspend_count: i32,
    }

    const KERN_SUCCESS: i32 = 0;
    const MACH_TASK_BASIC_INFO: i32 = 20;

    extern "C" {
        fn mach_task_self() -> MachPort;
        fn task_info(
            target_task: MachPort,
            flavor: i32,
            task_info_out: *mut i32,
            task_info_out_count: *mut MachMsgTypeNumber,
        ) -> i32;
    }

    let mut info = MachTaskBasicInfo {
        virtual_size: 0,
        resident_size: 0,
        resident_size_max: 0,
        user_time: TimeValue {
            seconds: 0,
            microseconds: 0,
        },
        system_time: TimeValue {
            seconds: 0,
            microseconds: 0,
        },
        policy: 0,
        suspend_count: 0,
    };
    let mut count =
        (mem::size_of::<MachTaskBasicInfo>() / mem::size_of::<i32>()) as MachMsgTypeNumber;
    let ok = unsafe {
        task_info(
            mach_task_self(),
            MACH_TASK_BASIC_INFO,
            (&mut info as *mut MachTaskBasicInfo).cast::<i32>(),
            &mut count,
        )
    };
    (ok == KERN_SUCCESS).then_some(info.resident_size)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn current_resident_set_size() -> Option<u64> {
    None
}
