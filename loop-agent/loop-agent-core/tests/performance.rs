use loop_agent_core::{EmitMode, run_loop};
use std::{
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[path = "../../tests/support.rs"]
mod test_support;

#[test]
#[ignore = "performance gate"]
fn ten_orchestrating_fixture_loops_complete_under_m1_runtime_contract() {
    let peak_rss_sampler = if rss_budget_must_be_enforced() {
        let baseline = current_resident_set_size()
            .expect("RSS measurement must be available on this enforced target before the run");
        Some(PeakRssSampler::start(baseline))
    } else {
        None
    };
    let workspaces = (0..10)
        .map(|_| workspace_copy("hello-loop"))
        .collect::<Vec<_>>();
    let concurrency = workspaces.len();
    let barrier = Arc::new(Barrier::new(concurrency + 1));
    let (tx, rx) = mpsc::channel();
    let handles = workspaces
        .into_iter()
        .map(|workspace| {
            let barrier = Arc::clone(&barrier);
            let tx = tx.clone();
            thread::spawn(move || {
                barrier.wait();
                let result = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
                    .map(|output| (output.event_count, output.failed))
                    .map_err(|err| err.to_string());
                tx.send(result).expect("result sent");
            })
        })
        .collect::<Vec<_>>();
    drop(tx);

    let started = Instant::now();
    barrier.wait();
    let timeout = Duration::from_secs(30);
    for _ in 0..concurrency {
        let elapsed = started.elapsed();
        assert!(
            elapsed < timeout,
            "10 concurrent orchestrating fixture loops must complete within {timeout:?}"
        );
        let remaining = timeout - elapsed;
        let result = rx
            .recv_timeout(remaining)
            .expect("10 concurrent orchestrating fixture loops complete before timeout");
        let (event_count, failed) = result.unwrap_or_else(|err| panic!("hello-loop: {err}"));
        assert!(event_count > 0, "hello-loop must emit events");
        assert!(!failed, "hello-loop should complete successfully");
    }
    for handle in handles {
        handle.join().expect("worker thread joins");
    }
    assert!(
        started.elapsed() <= timeout,
        "10 concurrent orchestrating fixture loops must complete within {timeout:?}"
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

#[test]
#[ignore = "performance gate"]
fn ten_near_limit_registry_closures_stay_within_the_loop_memory_budget() {
    let (workspace, active_bytes) = near_limit_registry_workspace();
    assert!(active_bytes <= core_script::MAX_ACTIVE_REGISTRY_BYTES);
    assert!(active_bytes >= core_script::MAX_ACTIVE_REGISTRY_BYTES * 9 / 10);

    let baseline = current_resident_set_size()
        .expect("RSS measurement must be available on every enforced release target");
    let mut sampler = PeakRssSampler::start(baseline);
    let concurrency = 10;
    let start = Arc::new(Barrier::new(concurrency + 1));
    let release = Arc::new(Barrier::new(concurrency + 1));
    let (tx, rx) = mpsc::channel();
    let handles = (0..concurrency)
        .map(|_| {
            let workspace = workspace.as_ref().to_path_buf();
            let start = Arc::clone(&start);
            let release = Arc::clone(&release);
            let tx = tx.clone();
            thread::spawn(move || {
                start.wait();
                let registry = core_script::load_loop_registry_from_workspace(
                    &workspace,
                    Path::new("registry"),
                    "smoke-loop",
                );
                tx.send(
                    registry
                        .as_ref()
                        .map(|registry| {
                            registry.instruction_block("near-limit-00").is_some()
                                && registry.instruction_block("unused-0000").is_none()
                        })
                        .map_err(ToString::to_string),
                )
                .expect("load result sent");
                release.wait();
                drop(registry);
            })
        })
        .collect::<Vec<_>>();
    drop(tx);

    start.wait();
    for _ in 0..concurrency {
        assert!(
            rx.recv_timeout(Duration::from_secs(60))
                .expect("near-limit registry loads before timeout")
                .unwrap_or_else(|err| panic!("near-limit registry: {err}")),
            "active definitions load once and unrelated catalog entries are not retained"
        );
    }
    release.wait();
    for handle in handles {
        handle.join().expect("registry loader thread joins");
    }

    let peak_growth = sampler.finish().saturating_sub(baseline);
    let per_loop_budget = 10 * 1024 * 1024;
    let budget = per_loop_budget * concurrency as u64;
    assert!(
        peak_growth <= budget,
        "near-limit registry peak RSS growth must stay <= {per_loop_budget} bytes per active top-level loop ({budget} bytes total): {peak_growth} bytes"
    );
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

struct TempWorkspace {
    path: PathBuf,
}

impl TempWorkspace {
    fn new(path: PathBuf) -> Self {
        Self { path }
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
    TempWorkspace::new(test_support::workspace_copy(fixture))
}

fn near_limit_registry_workspace() -> (TempWorkspace, u64) {
    let workspace = workspace_copy("smoke-loop");
    let mut active_bytes = [
        "instructions/say-smoke.yaml",
        "loops/smoke-loop.yaml",
        "phases/smoke.yaml",
        "tools/echo.yaml",
    ]
    .into_iter()
    .map(|path| {
        fs::metadata(workspace.join("registry").join(path))
            .expect("fixture metadata")
            .len()
    })
    .sum::<u64>();
    let mut references = Vec::new();
    for index in 0..16 {
        let id = format!("near-limit-{index:02}");
        references.push(id.clone());
        let source = format!(
            "instruction:\n  id: {id}\n  name: NearLimit{index:02}\n  prompt: {}\n",
            "x".repeat(60 * 1024)
        );
        active_bytes += u64::try_from(source.len()).expect("source length fits u64");
        fs::write(
            workspace.join(format!("registry/instructions/{id}.yaml")),
            source,
        )
        .expect("near-limit instruction written");
    }
    let phase_path = workspace.join("registry/phases/smoke.yaml");
    let phase = fs::read_to_string(&phase_path).expect("smoke phase readable");
    let updated_phase = phase.replace(
        "instruction_refs: [say-smoke]",
        &format!("instruction_refs: [say-smoke, {}]", references.join(", ")),
    );
    active_bytes = active_bytes - phase.len() as u64 + updated_phase.len() as u64;
    fs::write(phase_path, updated_phase).expect("near-limit references written");
    for index in 0..1000 {
        let id = format!("unused-{index:04}");
        let name_prefix = format!("Unused{index:04}");
        let name = format!(
            "{name_prefix}{}",
            "x".repeat(core_script::MAX_BLOCK_NAME_CHARS - name_prefix.len())
        );
        fs::write(
            workspace.join(format!("registry/instructions/{id}.yaml")),
            format!("instruction:\n  id: {id}\n  name: {name}\n  prompt: Unused\n"),
        )
        .expect("unrelated catalog instruction written");
    }
    (workspace, active_bytes)
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
    unsafe extern "system" {
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

    unsafe extern "C" {
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
