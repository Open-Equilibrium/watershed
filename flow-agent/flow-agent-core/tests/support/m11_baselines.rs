use serde_json::{Value, json};

pub(crate) struct BaselineReport {
    environment: Value,
    exclusions: Vec<&'static str>,
    inputs: Value,
    memory: Value,
    metrics: Value,
    operation: &'static str,
}

impl BaselineReport {
    pub(crate) fn fixture() -> Self {
        Self {
            environment: environment(),
            exclusions: vec!["fixture-only"],
            inputs: json!({"samples": 1}),
            memory: json!({"boundary": "not_applicable"}),
            metrics: json!({"sample_count": 1}),
            operation: "fixture",
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn operation(&self) -> &'static str {
        self.operation
    }

    pub(crate) fn canonical_json(&self) -> String {
        serde_json::to_string(&json!({
            "environment": self.environment,
            "exclusions": self.exclusions,
            "inputs": self.inputs,
            "memory": self.memory,
            "metrics": self.metrics,
            "operation": self.operation,
            "schema": "m1.1-baseline-v0",
        }))
        .expect("baseline report serializes")
    }
}

fn environment() -> Value {
    json!({
        "architecture": std::env::consts::ARCH,
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "logical_processors": std::thread::available_parallelism().map(|value| value.get()).ok(),
        "operating_system": std::env::consts::OS,
        "reference_gate": "ubuntu-24.04-x64-release",
    })
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{BaselineReport, environment};
    use crate::test_support::{PeakRssSampler, current_resident_set_size};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::{
        fs::{self, File},
        io::{Read, Write},
        path::{Path, PathBuf},
        process::{Command, Stdio},
        time::Instant,
    };

    const FILE_COUNT: usize = 64;
    const FILE_BYTES: usize = 64 * 1024;
    const FILESYSTEM_SAMPLES: usize = 20;
    const FILESYSTEM_WARMUPS: usize = 3;
    const PROCESS_SAMPLES: usize = 100;
    const PROCESS_WARMUPS: usize = 10;

    pub(crate) fn filesystem_baseline() -> BaselineReport {
        let root = TempBaselineDir::new("filesystem");
        let source = root.path().join("source");
        fs::create_dir(&source).expect("baseline source directory created");
        create_source_bundle(&source);
        let bundle_bytes = FILE_COUNT * FILE_BYTES;

        for index in 0..FILESYSTEM_WARMUPS {
            let destination = root.path().join(format!("warmup-{index:02}"));
            let (files, bytes) = inventory_bundle(&source);
            assert_eq!((files, bytes), (FILE_COUNT, bundle_bytes));
            copy_bundle(&source, &destination);
            fs::remove_dir_all(destination).expect("warm-up destination removed");
        }

        let mut rss_sampler =
            PeakRssSampler::start().expect("Linux baseline requires RSS sampling");
        let rss_before = rss_sampler.baseline();
        let mut inventory_nanos = Vec::with_capacity(FILESYSTEM_SAMPLES);
        let mut copy_nanos = Vec::with_capacity(FILESYSTEM_SAMPLES);
        let mut delete_nanos = Vec::with_capacity(FILESYSTEM_SAMPLES);
        for index in 0..FILESYSTEM_SAMPLES {
            let inventory_started = Instant::now();
            let (files, bytes) = inventory_bundle(&source);
            inventory_nanos.push(inventory_started.elapsed().as_nanos());
            assert_eq!((files, bytes), (FILE_COUNT, bundle_bytes));

            let destination = root.path().join(format!("sample-{index:02}"));
            let copy_started = Instant::now();
            copy_bundle(&source, &destination);
            copy_nanos.push(copy_started.elapsed().as_nanos());

            let delete_started = Instant::now();
            fs::remove_dir_all(destination).expect("baseline destination removed");
            delete_nanos.push(delete_started.elapsed().as_nanos());
        }
        let rss_peak = rss_sampler.finish();
        let rss_after = current_resident_set_size().expect("Linux baseline requires final RSS");

        BaselineReport {
            environment: environment(),
            exclusions: vec![
                "source creation and warm-up",
                "cold-cache behavior",
                "fsync and crash-durability guarantees",
                "manifest serialization and product policy",
                "concurrent filesystem activity",
                "Flow Agent export, delete, prune, and quota implementation",
            ],
            inputs: json!({
                "bundle_bytes": bundle_bytes,
                "bytes_per_file": FILE_BYTES,
                "file_count": FILE_COUNT,
                "samples": FILESYSTEM_SAMPLES,
                "storage_root": "std::env::temp_dir",
                "warmups": FILESYSTEM_WARMUPS,
                "working_set": "warm",
            }),
            memory: memory_report(
                rss_before,
                rss_peak,
                rss_after,
                "peak_and_retained_parent_process_rss",
            ),
            metrics: json!({
                "copy_p50_nanos": percentile(&copy_nanos, 50),
                "copy_p95_nanos": percentile(&copy_nanos, 95),
                "copy_throughput_bytes_per_second": throughput(bundle_bytes, &copy_nanos),
                "delete_p50_nanos": percentile(&delete_nanos, 50),
                "delete_p95_nanos": percentile(&delete_nanos, 95),
                "inventory_sha256_p50_nanos": percentile(&inventory_nanos, 50),
                "inventory_sha256_p95_nanos": percentile(&inventory_nanos, 95),
                "inventory_throughput_bytes_per_second": throughput(bundle_bytes, &inventory_nanos),
                "sample_count": FILESYSTEM_SAMPLES,
            }),
            operation: "filesystem_inventory_copy_delete",
        }
    }

    pub(crate) fn process_baseline() -> BaselineReport {
        let shell = Path::new("/bin/sh");
        assert!(shell.is_file(), "reference runner requires /bin/sh");
        let root = TempBaselineDir::new("process");

        for _ in 0..PROCESS_WARMUPS {
            run_noop_shell(shell, root.path());
        }

        let mut rss_sampler =
            PeakRssSampler::start().expect("Linux baseline requires RSS sampling");
        let rss_before = rss_sampler.baseline();
        let mut spawn_wait_nanos = Vec::with_capacity(PROCESS_SAMPLES);
        for _ in 0..PROCESS_SAMPLES {
            let started = Instant::now();
            run_noop_shell(shell, root.path());
            spawn_wait_nanos.push(started.elapsed().as_nanos());
        }
        let rss_peak = rss_sampler.finish();
        let rss_after = current_resident_set_size().expect("Linux baseline requires final RSS");

        BaselineReport {
            environment: environment(),
            exclusions: vec![
                "shell discovery and PATH lookup",
                "script parsing beyond the POSIX no-op builtin",
                "stdout and stderr capture",
                "timeouts, cancellation, signals, and process groups",
                "child-process RSS",
                "policy validation and Flow Agent runner implementation",
            ],
            inputs: json!({
                "arguments": ["-c", ":"],
                "environment": "empty",
                "samples": PROCESS_SAMPLES,
                "shell": "/bin/sh",
                "stdio": "null",
                "warmups": PROCESS_WARMUPS,
            }),
            memory: memory_report(
                rss_before,
                rss_peak,
                rss_after,
                "peak_and_retained_parent_process_rss",
            ),
            metrics: json!({
                "sample_count": PROCESS_SAMPLES,
                "spawn_wait_max_nanos": spawn_wait_nanos.iter().copied().max().unwrap_or(0),
                "spawn_wait_min_nanos": spawn_wait_nanos.iter().copied().min().unwrap_or(0),
                "spawn_wait_p50_nanos": percentile(&spawn_wait_nanos, 50),
                "spawn_wait_p95_nanos": percentile(&spawn_wait_nanos, 95),
                "spawn_wait_throughput_per_second": operations_per_second(&spawn_wait_nanos),
            }),
            operation: "posix_shell_spawn_wait",
        }
    }

    fn create_source_bundle(source: &Path) {
        for index in 0..FILE_COUNT {
            let mut file = File::create(source.join(format!("object-{index:03}.bin")))
                .expect("baseline source file created");
            file.write_all(&vec![index as u8; FILE_BYTES])
                .expect("baseline source bytes written");
        }
    }

    fn sorted_files(directory: &Path) -> Vec<PathBuf> {
        let mut files = fs::read_dir(directory)
            .expect("baseline directory readable")
            .map(|entry| entry.expect("baseline entry readable").path())
            .collect::<Vec<_>>();
        files.sort();
        files
    }

    fn inventory_bundle(source: &Path) -> (usize, usize) {
        let files = sorted_files(source);
        let mut total_bytes = 0;
        let mut bundle_hash = Sha256::new();
        for path in &files {
            let name = path
                .file_name()
                .expect("baseline path has a file name")
                .to_string_lossy();
            bundle_hash.update(name.as_bytes());
            let mut file = File::open(path).expect("baseline source file opens");
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .expect("baseline source file reads");
            total_bytes += bytes.len();
            bundle_hash.update(&bytes);
        }
        std::hint::black_box(bundle_hash.finalize());
        (files.len(), total_bytes)
    }

    fn copy_bundle(source: &Path, destination: &Path) {
        fs::create_dir(destination).expect("baseline destination created");
        for path in sorted_files(source) {
            let target = destination.join(
                path.file_name()
                    .expect("baseline source path has a file name"),
            );
            fs::copy(path, target).expect("baseline source file copied");
        }
    }

    fn run_noop_shell(shell: &Path, working_directory: &Path) {
        let status = Command::new(shell)
            .args(["-c", ":"])
            .current_dir(working_directory)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("baseline shell starts and waits");
        assert!(status.success(), "baseline shell exits successfully");
    }

    fn percentile(samples: &[u128], percentile: usize) -> u128 {
        assert!(!samples.is_empty(), "percentile requires samples");
        assert!((1..=100).contains(&percentile));
        let mut ordered = samples.to_vec();
        ordered.sort_unstable();
        let index = (ordered.len() * percentile).div_ceil(100).saturating_sub(1);
        ordered[index]
    }

    fn throughput(bytes_per_operation: usize, samples: &[u128]) -> u128 {
        let elapsed_nanos = samples.iter().sum::<u128>();
        (bytes_per_operation as u128 * samples.len() as u128 * 1_000_000_000) / elapsed_nanos.max(1)
    }

    fn operations_per_second(samples: &[u128]) -> u128 {
        samples.len() as u128 * 1_000_000_000 / samples.iter().sum::<u128>().max(1)
    }

    fn memory_report(before: u64, peak: u64, after: u64, boundary: &str) -> Value {
        json!({
            "after_bytes": after,
            "before_bytes": before,
            "boundary": boundary,
            "peak_bytes": peak,
            "peak_growth_bytes": peak.saturating_sub(before),
            "retained_growth_bytes": after.saturating_sub(before),
        })
    }

    struct TempBaselineDir(PathBuf);

    impl TempBaselineDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "watershed-m11-baseline-{label}-{}",
                std::process::id()
            ));
            if path.exists() {
                fs::remove_dir_all(&path).expect("stale baseline directory removed");
            }
            fs::create_dir(&path).expect("baseline directory created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempBaselineDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::{filesystem_baseline, process_baseline};
