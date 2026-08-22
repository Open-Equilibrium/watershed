#[cfg(target_os = "linux")]
use std::fs;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

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

#[cfg(target_os = "linux")]
#[allow(dead_code)]
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

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
pub(crate) fn current_resident_set_size() -> Option<u64> {
    None
}
