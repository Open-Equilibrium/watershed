mod report;

use flow_agent_core::{
    M12ExecutorStartupMeasurement, configure_executor_path, run_m12_executor_startup,
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
const MEASUREMENT_CHILD_SCHEMA: &str = "flow-m12-executor-startup-sample-v0";
const FLOW_AGENT_HOME_ENV: &str = "FLOW_AGENT_HOME";
const FLOW_AGENT_HOME_LEAF: &str = ".flow";
const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";
const XDG_CONFIG_HOME_LEAF: &str = ".config";
type DynError = Box<dyn Error + Send + Sync>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Config {
    executor: PathBuf,
    warmups: usize,
    samples: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChildMeasurement {
    schema: String,
    executor_elapsed_ns: u64,
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
    let M12ExecutorStartupMeasurement { executor_elapsed } =
        run_m12_executor_startup(workspace.path()).map_err(io::Error::other)?;
    Ok(ChildMeasurement {
        schema: MEASUREMENT_CHILD_SCHEMA.to_owned(),
        executor_elapsed_ns: duration_ns(executor_elapsed),
    })
}

fn bounded_diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_CHILD_DIAGNOSTIC_BYTES)])
        .trim()
        .to_owned()
}

fn fresh_child_measurement(executor: &Path) -> Result<ChildMeasurement, DynError> {
    let session_root = TempRoot::create()?;
    let output = Command::new(env::current_exe()?)
        .arg(MEASUREMENT_CHILD_ARG)
        .arg(executor)
        .env(
            FLOW_AGENT_HOME_ENV,
            session_root.path().join(FLOW_AGENT_HOME_LEAF),
        )
        .env(
            XDG_CONFIG_HOME_ENV,
            session_root.path().join(XDG_CONFIG_HOME_LEAF),
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
    let mut executor = None;
    let mut warmups = DEFAULT_WARMUPS;
    let mut samples = DEFAULT_SAMPLES;
    let mut args = args.into_iter().map(Into::into);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| io::Error::other(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--executor" => {
                if executor.is_some() {
                    return Err(io::Error::other("--executor may only be provided once").into());
                }
                let path = PathBuf::from(value);
                if !path.is_absolute() {
                    return Err(io::Error::other("--executor must be an absolute path").into());
                }
                executor = Some(path);
            }
            "--warmup" => warmups = parse_positive(&value, "--warmup")?,
            "--samples" => samples = parse_positive(&value, "--samples")?,
            _ => return Err(io::Error::other(format!("unknown argument {flag}")).into()),
        }
    }
    Ok(Config {
        executor: executor.ok_or_else(|| io::Error::other("--executor is required"))?,
        warmups,
        samples,
    })
}

fn run_main() -> Result<(), DynError> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if let [measurement_child, executor] = args.as_slice()
        && measurement_child == MEASUREMENT_CHILD_ARG
    {
        configure_executor_path(Path::new(executor)).map_err(io::Error::other)?;
        write_jsonl(&mut io::stdout().lock(), &measure_once()?)?;
        return Ok(());
    }

    let config = parse_args(args)?;
    let complete = write_report(&mut io::stdout().lock(), config)?;
    if !complete {
        return Err(io::Error::other("M1.2 Executor startup evidence was incomplete").into());
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
    use std::{
        io::{self, Write},
        path::PathBuf,
    };

    fn executor_path() -> PathBuf {
        PathBuf::from(if cfg!(windows) {
            r"C:\flow-executor.exe"
        } else {
            "/flow-executor"
        })
    }

    fn config(warmups: usize, samples: usize) -> Config {
        Config {
            executor: executor_path(),
            warmups,
            samples,
        }
    }

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
    fn defaults_require_one_absolute_executor_path() {
        assert_eq!(
            parse_args(["--executor", executor_path().to_str().unwrap()]).unwrap(),
            Config {
                executor: executor_path(),
                warmups: 5,
                samples: 30,
            }
        );
        assert!(parse_args(Vec::<String>::new()).is_err());
        assert!(parse_args(["--executor", "relative/flow-executor"]).is_err());
        assert!(
            parse_args([
                "--executor",
                executor_path().to_str().unwrap(),
                "--executor",
                executor_path().to_str().unwrap(),
            ])
            .is_err()
        );
    }

    #[test]
    fn nearest_rank_percentile_is_deterministic() {
        let samples = (1..=30).collect::<Vec<_>>();
        assert_eq!(percentile(&samples, 50, 100), 15);
        assert_eq!(percentile(&samples, 95, 100), 29);
    }

    #[test]
    fn aggregate_retains_the_executor_distribution() {
        let mut observation = 0_u64;
        let mut measure = || -> Result<ChildMeasurement, DynError> {
            observation += 1;
            Ok(ChildMeasurement {
                schema: MEASUREMENT_CHILD_SCHEMA.to_owned(),
                executor_elapsed_ns: observation * 10,
            })
        };
        let mut writer = Vec::new();

        assert!(write_report_with_measurement(&mut writer, config(1, 3), &mut measure,).unwrap());

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
        assert_eq!(aggregate["executor_p50_ns"], 30);
        assert_eq!(aggregate["executor_p95_ns"], 40);
    }

    #[test]
    fn report_identifies_the_real_executor_boundary() {
        let mut measure = || -> Result<ChildMeasurement, DynError> {
            Ok(ChildMeasurement {
                schema: MEASUREMENT_CHILD_SCHEMA.to_owned(),
                executor_elapsed_ns: 1,
            })
        };
        let mut writer = Vec::new();

        assert!(write_report_with_measurement(&mut writer, config(1, 1), &mut measure,).unwrap());

        let records = String::from_utf8(writer)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let metadata = &records[0];
        assert_eq!(metadata["schema"], "flow-m12-executor-startup-v0");
        assert!(metadata["environment"].get("contract_image").is_some());
        let sample = records
            .iter()
            .find(|record| record["kind"] == "sample")
            .unwrap();
        assert_eq!(sample["executor_elapsed_ns"], 1);
        let aggregate = records
            .iter()
            .find(|record| record["kind"] == "aggregate")
            .unwrap();
        assert_eq!(
            aggregate["inputs"]["boundary"],
            "prepared_selected_executor"
        );
        assert_eq!(aggregate["inputs"]["tool"], "/bin/echo");
        assert_eq!(aggregate["inputs"]["tool_arguments"], Value::Array(vec![]));
        assert_eq!(aggregate["inputs"]["tool_environment"], "empty");
        assert_eq!(aggregate["inputs"]["runtime_profile"], "exact");
        assert_eq!(aggregate["inputs"]["tool_executions_per_child"], 1);
    }

    #[test]
    fn child_failure_retains_a_complete_failed_report() {
        let mut measure = || -> Result<ChildMeasurement, DynError> {
            Err(io::Error::other("injected child diagnostic").into())
        };
        let mut writer = FlushTrackingWriter::default();

        let complete =
            write_report_with_measurement(&mut writer, config(1, 1), &mut measure).unwrap();

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
