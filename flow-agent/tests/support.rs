use std::{
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join(name)
}

#[derive(Clone)]
pub(crate) struct TempWorkspace(Arc<OwnedTempWorkspace>);

struct OwnedTempWorkspace(PathBuf);

impl TempWorkspace {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self(Arc::new(OwnedTempWorkspace(path)))
    }
}

impl Deref for TempWorkspace {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0.0
    }
}

impl AsRef<Path> for TempWorkspace {
    fn as_ref(&self) -> &Path {
        &self.0.0
    }
}

impl Drop for OwnedTempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn workspace_copy(fixture: &str) -> TempWorkspace {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target = TempWorkspace::new(
        std::env::temp_dir().join(format!("watershed-flow-agent-{}-{id}", std::process::id())),
    );
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    copy_fixture_workspace(&fixture_dir(fixture), &target);
    target
}

#[allow(dead_code)]
pub(crate) fn expected_stream(fixture: &str, stream: &str) -> String {
    fs::read_to_string(fixture_dir(fixture).join("expected").join(stream))
        .expect("expected stream is readable")
}

pub(crate) fn copy_fixture_workspace(source: &Path, target: &Path) {
    copy_dir(&source.join("registry"), &target.join("registry"));
    let source_config = source.join(".flow/config.yaml");
    if source_config.exists() {
        let target_config = target.join(".flow/config.yaml");
        fs::create_dir_all(target_config.parent().expect("config path has parent"))
            .expect("workspace config directory created");
        fs::copy(source_config, target_config).expect("workspace config copied");
    }
    if source.join("out").is_dir() {
        fs::create_dir_all(target.join("out")).expect("output directory shape copied");
    }
}

pub(crate) fn copy_dir(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("target directory created");
    for entry in fs::read_dir(source).expect("source directory readable") {
        let entry = entry.expect("source entry readable");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            fs::copy(source_path, target_path).expect("fixture file copied");
        }
    }
}

#[allow(dead_code)]
pub(crate) struct PeakRssSampler {
    baseline: u64,
    peak: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

#[allow(dead_code)]
impl PeakRssSampler {
    pub(crate) fn start() -> Option<Self> {
        let baseline = match current_resident_set_size() {
            Some(baseline) => baseline,
            None if cfg!(target_os = "linux") => {
                panic!("Linux RSS performance gates require readable /proc/self/status")
            }
            None => return None,
        };
        let peak = Arc::new(AtomicU64::new(baseline));
        let running = Arc::new(AtomicBool::new(true));
        let sampler_peak = Arc::clone(&peak);
        let sampler_running = Arc::clone(&running);
        let handle = thread::spawn(move || {
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
        Some(Self {
            baseline,
            peak,
            running,
            handle: Some(handle),
        })
    }

    pub(crate) fn baseline(&self) -> u64 {
        self.baseline
    }

    pub(crate) fn finish(&mut self) -> u64 {
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

#[allow(dead_code)]
#[cfg(target_os = "linux")]
pub(crate) fn current_resident_set_size() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status")
        .expect("Linux RSS performance gates require readable /proc/self/status");
    let rss_bytes = status
        .lines()
        .find_map(|line| {
            let value = line.strip_prefix("VmRSS:")?.trim();
            let kilobytes = value.strip_suffix(" kB")?.trim().parse::<u64>().ok()?;
            kilobytes.checked_mul(1024)
        })
        .expect("Linux RSS performance gates require a valid VmRSS byte value");
    Some(rss_bytes)
}

#[allow(dead_code)]
#[cfg(not(target_os = "linux"))]
pub(crate) fn current_resident_set_size() -> Option<u64> {
    None
}
