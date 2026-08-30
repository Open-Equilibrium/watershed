mod report;

use flow_agent_core::{
    M12_STARTUP_TOOL_CHILD_ARG, M12DirectRunnerMeasurement, run_m12_direct_runner_startup,
    write_m12_noop_tool_child_report,
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
const MAX_MEASUREMENT_CHILD_BYTES: usize = 256;
const MAX_CHILD_DIAGNOSTIC_BYTES: usize = 1_024;
const MEASUREMENT_CHILD_ARG: &str = "--measure-child";
const MEASUREMENT_CHILD_SCHEMA: &str = "flow-m12-startup-sample-v0";
const FLOW_AGENT_HOME_ENV: &str = "FLOW_AGENT_HOME";
const FLOW_AGENT_HOME_LEAF: &str = ".flow";
type DynError = Box<dyn Error + Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Config {
    warmups: usize,
    samples: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChildMeasurement {
    schema: String,
    runner_elapsed_ns: u64,
    tool_runtime_ns: u64,
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn create() -> Result<Self, DynError> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!("flow-m12-startup-{}-{nonce}", std::process::id()));
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

fn measure_once() -> Result<ChildMeasurement, DynError> {
    let workspace = TempRoot::create()?;
    let M12DirectRunnerMeasurement {
        runner_elapsed,
        tool_runtime,
    } = run_m12_direct_runner_startup(workspace.path()).map_err(io::Error::other)?;
    Ok(ChildMeasurement {
        schema: MEASUREMENT_CHILD_SCHEMA.to_owned(),
        runner_elapsed_ns: duration_ns(runner_elapsed),
        tool_runtime_ns: duration_ns(tool_runtime),
    })
}

fn bounded_diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_CHILD_DIAGNOSTIC_BYTES)])
        .trim()
        .to_owned()
}

fn fresh_child_measurement() -> Result<ChildMeasurement, DynError> {
    let session_root = TempRoot::create()?;
    let output = Command::new(env::current_exe()?)
        .arg(MEASUREMENT_CHILD_ARG)
        .env(
            FLOW_AGENT_HOME_ENV,
            session_root.path().join(FLOW_AGENT_HOME_LEAF),
        )
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "fresh measurement child failed: {}",
            bounded_diagnostic(&output.stderr)
        ))
        .into());
    }
    if output.stdout.is_empty() || output.stdout.len() > MAX_MEASUREMENT_CHILD_BYTES {
        return Err(io::Error::other("fresh measurement child violated its output bound").into());
    }
    let measurement: ChildMeasurement = serde_json::from_slice(&output.stdout)?;
    if measurement.schema != MEASUREMENT_CHILD_SCHEMA {
        return Err(io::Error::other("fresh measurement child schema did not match").into());
    }
    if measurement.tool_runtime_ns > measurement.runner_elapsed_ns {
        return Err(
            io::Error::other("Tool runtime exceeded the enclosing direct-runner interval").into(),
        );
    }
    Ok(measurement)
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
    if args.as_slice() == [M12_STARTUP_TOOL_CHILD_ARG] {
        write_m12_noop_tool_child_report(&mut io::stdout().lock()).map_err(io::Error::other)?;
        return Ok(());
    }
    if args.as_slice() == [MEASUREMENT_CHILD_ARG] {
        write_jsonl(&mut io::stdout().lock(), &measure_once()?)?;
        return Ok(());
    }

    let config = parse_args(args)?;
    let complete = write_report(&mut io::stdout().lock(), config)?;
    if !complete {
        return Err(io::Error::other("M1.2 startup baseline evidence was incomplete").into());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run_main() {
        eprintln!("m12_executor_startup: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{ChildMeasurement, Config, DynError, MEASUREMENT_CHILD_SCHEMA, parse_args};
    use crate::report::{percentile, write_report_with_measurement};
    use serde_json::Value;
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
                samples: 30,
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
    fn aggregate_keeps_runner_and_tool_runtime_distributions_separate() {
        let mut observation = 0_u64;
        let mut measure = || -> Result<ChildMeasurement, DynError> {
            observation += 1;
            Ok(ChildMeasurement {
                schema: MEASUREMENT_CHILD_SCHEMA.to_owned(),
                runner_elapsed_ns: observation * 10,
                tool_runtime_ns: observation,
            })
        };
        let mut writer = Vec::new();

        assert!(
            write_report_with_measurement(
                &mut writer,
                Config {
                    warmups: 1,
                    samples: 3,
                },
                &mut measure,
            )
            .unwrap()
        );

        let records = String::from_utf8(writer)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            records
                .iter()
                .filter(|record| record["kind"] == "sample")
                .count(),
            3
        );
        let aggregate = records
            .iter()
            .find(|record| record["kind"] == "aggregate")
            .unwrap();
        assert_eq!(aggregate["runner_p50_ns"], 30);
        assert_eq!(aggregate["runner_p95_ns"], 40);
        assert_eq!(aggregate["tool_runtime_p50_ns"], 3);
        assert_eq!(aggregate["tool_runtime_p95_ns"], 4);
    }

    #[test]
    fn child_failure_retains_a_complete_failed_report() {
        let mut measure = || -> Result<ChildMeasurement, DynError> {
            Err(io::Error::other("injected child diagnostic").into())
        };
        let mut writer = FlushTrackingWriter::default();

        let complete = write_report_with_measurement(
            &mut writer,
            Config {
                warmups: 1,
                samples: 1,
            },
            &mut measure,
        )
        .unwrap();

        assert!(!complete);
        assert!(writer.flushed);
        let records = String::from_utf8(writer.bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["kind"], "metadata");
        assert_eq!(records[1]["kind"], "workload_failure");
        assert_eq!(records[1]["error"], "injected child diagnostic");
        assert_eq!(records[2]["kind"], "summary");
        assert_eq!(records[2]["complete"], false);
    }
}
