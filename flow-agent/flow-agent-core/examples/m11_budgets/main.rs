mod contract;
mod report;

use flow_agent_core::{
    M11_BUDGET_WORKLOADS, M11BudgetOutcome, M11BudgetWorkload, run_m11_budget_workload,
};
use report::{write_jsonl, write_report};
use serde::{Deserialize, Serialize};
use std::{
    env,
    error::Error,
    fs,
    io::{self},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DEFAULT_WARMUPS: usize = 5;
const DEFAULT_SAMPLES: usize = 30;
const MAX_SAMPLE_COUNT: usize = 1_000;
const FLOW_AGENT_HOME_ENV: &str = "FLOW_AGENT_HOME";
const FLOW_AGENT_HOME_LEAF: &str = ".flow";
type DynError = Box<dyn Error + Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Config {
    warmups: usize,
    samples: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct RssMeasurement {
    supported: bool,
    peak_growth_bytes: Option<u64>,
    retained_growth_bytes: Option<u64>,
}

#[derive(Deserialize, Serialize)]
struct ChildMeasurement {
    elapsed_ns: u64,
    operations: u64,
    input_bytes: u64,
    output_bytes: u64,
    checksum: u64,
    rss: RssMeasurement,
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn create() -> Result<Self, DynError> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!("flow-m11-budget-{}-{nonce}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn unsupported_rss() -> RssMeasurement {
    RssMeasurement {
        supported: false,
        peak_growth_bytes: None,
        retained_growth_bytes: None,
    }
}

#[cfg(target_os = "linux")]
fn current_rss_observation() -> (Option<u64>, Option<u64>) {
    let Ok(status) = fs::read_to_string("/proc/self/status") else {
        return (None, None);
    };
    let field_bytes = |field| {
        status.lines().find_map(|line| {
            let value = line.strip_prefix(field)?.split_whitespace().next()?;
            value.parse::<u64>().ok()?.checked_mul(1024)
        })
    };
    (field_bytes("VmHWM:"), field_bytes("VmRSS:"))
}

#[cfg(not(target_os = "linux"))]
fn current_rss_observation() -> (Option<u64>, Option<u64>) {
    (None, None)
}

fn measure_workload(
    workload: &M11BudgetWorkload,
    iteration: usize,
) -> Result<ChildMeasurement, DynError> {
    let temp = TempRoot::create()?;
    let (_, baseline_retained) = current_rss_observation();
    let outcome =
        run_m11_budget_workload(workload.id, temp.path(), iteration).map_err(io::Error::other)?;
    let (post_high_water, post_retained) = current_rss_observation();
    let rss = match (baseline_retained, post_high_water) {
        (Some(baseline), Some(high_water)) if high_water >= baseline => RssMeasurement {
            supported: true,
            peak_growth_bytes: Some(high_water - baseline),
            retained_growth_bytes: post_retained.map(|retained| retained.saturating_sub(baseline)),
        },
        _ => unsupported_rss(),
    };
    Ok(child_measurement(outcome, rss))
}

fn child_measurement(outcome: M11BudgetOutcome, rss: RssMeasurement) -> ChildMeasurement {
    ChildMeasurement {
        elapsed_ns: duration_ns(outcome.elapsed),
        operations: outcome.operations,
        input_bytes: outcome.input_bytes,
        output_bytes: outcome.output_bytes,
        checksum: outcome.checksum,
        rss,
    }
}

fn fresh_child_measurement(
    workload: &M11BudgetWorkload,
    iteration: usize,
) -> Result<ChildMeasurement, DynError> {
    let session_root = TempRoot::create()?;
    let output = Command::new(env::current_exe()?)
        .args(["--measure-child", workload.name(), &iteration.to_string()])
        .env(
            FLOW_AGENT_HOME_ENV,
            session_root.path().join(FLOW_AGENT_HOME_LEAF),
        )
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "fresh child for {} failed: {}",
            workload.name(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
        .into());
    }
    serde_json::from_slice(&output.stdout).map_err(Into::into)
}

fn parse_positive(value: &str, flag: &str) -> Result<usize, DynError> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| io::Error::other(format!("{flag} must be an integer")))?;
    if parsed == 0 || parsed > MAX_SAMPLE_COUNT {
        return Err(
            io::Error::other(format!("{flag} must be between 1 and {MAX_SAMPLE_COUNT}")).into(),
        );
    }
    Ok(parsed)
}

fn parse_args<I, S>(args: I) -> Result<Config, DynError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut config = Config {
        warmups: DEFAULT_WARMUPS,
        samples: DEFAULT_SAMPLES,
    };
    let mut args = args.into_iter().map(Into::into);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| io::Error::other(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--warmup" => config.warmups = parse_positive(&value, "--warmup")?,
            "--samples" => config.samples = parse_positive(&value, "--samples")?,
            _ => return Err(io::Error::other(format!("unknown argument {flag}")).into()),
        }
    }
    Ok(config)
}

fn run_main() -> Result<(), DynError> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg| arg == "--measure-child") {
        if args.len() != 3 {
            return Err(io::Error::other(
                "--measure-child requires one workload and one iteration",
            )
            .into());
        }
        let workload = M11_BUDGET_WORKLOADS
            .iter()
            .find(|workload| workload.name() == args[1])
            .ok_or_else(|| io::Error::other("unknown child workload"))?;
        let iteration = args[2]
            .parse::<usize>()
            .map_err(|_| io::Error::other("child iteration must be an integer"))?;
        let measurement = measure_workload(workload, iteration)?;
        write_jsonl(&mut io::stdout().lock(), &measurement)?;
        return Ok(());
    }

    let config = parse_args(args)?;
    let passed = write_report(&mut io::stdout().lock(), config)?;
    if !passed {
        return Err(io::Error::other("one or more M1.1 optimized budgets failed").into());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run_main() {
        eprintln!("m11_budgets: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChildMeasurement, Config, DynError, M11_BUDGET_WORKLOADS, M11BudgetWorkload,
        RssMeasurement,
        contract::workload_contract,
        parse_args,
        report::{percentile, write_report_with_measurement},
    };
    use flow_agent_core::M11BudgetWorkloadId;
    use serde_json::{Value, json};
    use std::io::{self, Write};

    #[derive(Default)]
    struct FlushTrackingWriter {
        bytes: Vec<u8>,
        flushed: bool,
    }

    impl Write for FlushTrackingWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushed = true;
            Ok(())
        }
    }

    #[test]
    fn defaults_match_the_approved_sampling_contract() {
        assert_eq!(
            parse_args(Vec::<String>::new()).unwrap(),
            Config {
                warmups: 5,
                samples: 30
            }
        );
    }

    #[test]
    fn nearest_rank_percentile_is_deterministic() {
        let samples = (1..=30).collect::<Vec<_>>();
        assert_eq!(percentile(&samples, 50, 100), 15);
        assert_eq!(percentile(&samples, 95, 100), 29);
    }

    #[test]
    fn every_workload_has_non_null_exact_inputs() {
        for workload in M11_BUDGET_WORKLOADS {
            assert_ne!(workload_contract(workload.id).0, Value::Null);
        }
        assert_eq!(
            workload_contract(M11BudgetWorkloadId::ConversationHistoryValidationQuantum).0["records"],
            38_481
        );
    }

    #[test]
    fn cancellation_contract_is_distinct_from_termination() {
        let termination = workload_contract(M11BudgetWorkloadId::RunnerTermination).0;
        let cancellation = workload_contract(M11BudgetWorkloadId::RunnerCancellation).0;

        assert_eq!(termination["trigger"], "TERM");
        assert_eq!(cancellation["trigger"], "atomic cancellation");
        assert_ne!(cancellation, termination);
    }

    #[test]
    fn child_measurement_failure_retains_a_complete_failed_report() {
        let failed_workload = M11_BUDGET_WORKLOADS[1].name();
        let mut measured_workloads = Vec::new();
        let mut measure = |workload: &M11BudgetWorkload,
                           _iteration: usize|
         -> Result<ChildMeasurement, DynError> {
            measured_workloads.push(workload.name());
            if workload.name() == failed_workload {
                return Err(io::Error::other("injected child diagnostic").into());
            }
            Ok(ChildMeasurement {
                elapsed_ns: 1,
                operations: 1,
                input_bytes: 1,
                output_bytes: 1,
                checksum: 1,
                rss: RssMeasurement {
                    supported: true,
                    peak_growth_bytes: Some(4 * 1024 * 1024),
                    retained_growth_bytes: Some(0),
                },
            })
        };
        let mut writer = FlushTrackingWriter::default();

        let passed = write_report_with_measurement(
            &mut writer,
            Config {
                warmups: 1,
                samples: 1,
            },
            &mut measure,
        )
        .expect("measurement failure is retained in the completed report");

        assert!(!passed);
        assert!(writer.flushed);
        for workload in M11_BUDGET_WORKLOADS {
            assert!(
                measured_workloads.contains(&workload.name()),
                "{} was not attempted",
                workload.name()
            );
        }
        let records = String::from_utf8(writer.bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        for workload in M11_BUDGET_WORKLOADS {
            assert!(records.iter().any(|record| {
                record["benchmark"] == workload.name()
                    && matches!(
                        record["kind"].as_str(),
                        Some("aggregate" | "workload_failure")
                    )
            }));
        }
        let failure = records
            .iter()
            .find(|record| record["kind"] == "workload_failure")
            .expect("failed workload has a terminal record");
        assert_eq!(failure["benchmark"], failed_workload);
        assert_eq!(failure["passed"], false);
        assert_eq!(failure["error"], "injected child diagnostic");
        let summary = records.last().expect("summary is present");
        assert_eq!(summary["kind"], "summary");
        assert_eq!(summary["passed"], false);
        assert_eq!(summary["failing_workloads"], json!([failed_workload]));
    }

    #[test]
    fn aggregate_rss_uses_only_emitted_measured_samples() {
        let mut measure = |_workload: &M11BudgetWorkload,
                           iteration: usize|
         -> Result<ChildMeasurement, DynError> {
            Ok(ChildMeasurement {
                elapsed_ns: 1,
                operations: 1,
                input_bytes: 1,
                output_bytes: 1,
                checksum: 1,
                rss: RssMeasurement {
                    supported: true,
                    peak_growth_bytes: Some(if iteration == 0 { 9 } else { 1 }),
                    retained_growth_bytes: Some(if iteration == 0 { 8 } else { 2 }),
                },
            })
        };
        let mut writer = Vec::new();

        write_report_with_measurement(
            &mut writer,
            Config {
                warmups: 1,
                samples: 1,
            },
            &mut measure,
        )
        .expect("report writes");

        let aggregate = String::from_utf8(writer)
            .expect("report is UTF-8")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("report line parses"))
            .find(|record| record["kind"] == "aggregate")
            .expect("aggregate exists");
        assert_eq!(aggregate["peak_rss_growth_max_bytes"], 1);
        assert_eq!(aggregate["retained_rss_growth_max_bytes"], 2);
    }
}
