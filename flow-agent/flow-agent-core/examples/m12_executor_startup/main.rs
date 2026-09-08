#[path = "../evidence_support/mod.rs"]
mod evidence_support;
#[path = "../evidence_support/installed_controller.rs"]
mod installed_controller;
mod report;

use evidence_support::{
    DynError, FLOW_AGENT_HOME, TempRoot, duration_ns, launch_measurement_child, parse_positive,
    write_jsonl,
};
use flow_agent_core::{
    M12ExecutorStartupMeasurement, configure_executor_path, run_m12_executor_startup,
};
use report::write_report;
use serde::{Deserialize, Serialize};
use std::{
    env,
    ffi::OsStr,
    io::{self},
    path::{Path, PathBuf},
};

const DEFAULT_WARMUPS: usize = 5;
const DEFAULT_SAMPLES: usize = 30;
const MAX_MEASUREMENT_CHILD_BYTES: usize = 256;
const MAX_CHILD_DIAGNOSTIC_BYTES: usize = 1_024;
const MEASUREMENT_CHILD_ARG: &str = "--measure-child";
const MEASUREMENT_CHILD_SCHEMA: &str = "flow-m12-executor-startup-sample-v0";
const XDG_CONFIG_HOME: (&str, &str) = ("XDG_CONFIG_HOME", ".config");

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
    self_protection_active: bool,
}

impl ChildMeasurement {
    fn validate(self) -> Result<Self, DynError> {
        if self.schema != MEASUREMENT_CHILD_SCHEMA {
            return Err(io::Error::other("fresh measurement child schema did not match").into());
        }
        if !self.self_protection_active {
            return Err(io::Error::other(
                "fresh measurement child did not confirm active self-protection",
            )
            .into());
        }
        Ok(self)
    }
}

fn measure_once() -> Result<ChildMeasurement, DynError> {
    let workspace = TempRoot::create("flow-m12-startup")?;
    let M12ExecutorStartupMeasurement {
        executor_elapsed,
        self_protection_active,
    } = run_m12_executor_startup(workspace.path()).map_err(io::Error::other)?;
    Ok(ChildMeasurement {
        schema: MEASUREMENT_CHILD_SCHEMA.to_owned(),
        executor_elapsed_ns: duration_ns(executor_elapsed),
        self_protection_active,
    })
}

fn bounded_diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_CHILD_DIAGNOSTIC_BYTES)])
        .trim()
        .to_owned()
}

fn fresh_child_measurement(executor: &Path) -> Result<ChildMeasurement, DynError> {
    let session_root = TempRoot::create("flow-m12-startup")?;
    let output = launch_measurement_child(
        &session_root,
        &installed_controller::stage_controller(session_root.path())?,
        [OsStr::new(MEASUREMENT_CHILD_ARG), executor.as_os_str()],
        &[FLOW_AGENT_HOME, XDG_CONFIG_HOME],
    )?;
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
    Ok(serde_json::from_slice(&output.stdout)?)
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
    use crate::{
        evidence_support::test::FlushTrackingWriter, report::write_report_with_measurement,
    };
    use serde_json::Value;
    use std::{io, path::PathBuf};

    fn executor_path() -> PathBuf {
        PathBuf::from("/flow-executor")
    }

    fn config(warmups: usize, samples: usize) -> Config {
        Config {
            executor: executor_path(),
            warmups,
            samples,
        }
    }

    fn measurement(elapsed_ns: u64) -> ChildMeasurement {
        serde_json::from_value(serde_json::json!({
            "schema": MEASUREMENT_CHILD_SCHEMA,
            "executor_elapsed_ns": elapsed_ns,
            "self_protection_active": true,
        }))
        .expect("child evidence binds the elapsed observation to active own-file protection")
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
    fn aggregate_retains_the_executor_distribution() {
        let mut observation = 0_u64;
        let mut measure = || -> Result<ChildMeasurement, DynError> {
            observation += 1;
            Ok(measurement(observation * 10))
        };
        let mut writer = Vec::new();

        assert!(write_report_with_measurement(&mut writer, config(1, 3), &mut measure,).unwrap());
        assert_eq!(observation, 4);

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
        assert_eq!(aggregate["executor_max_ns"], 40);
        assert_eq!(aggregate["count"], 3);
        assert_eq!(
            records
                .iter()
                .filter(|record| record["kind"] == "sample")
                .map(|record| record["executor_elapsed_ns"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [20, 30, 40]
        );
        assert_eq!(records.last().unwrap()["complete"], true);
    }

    #[test]
    fn report_identifies_the_real_executor_boundary() {
        let mut measure = || -> Result<ChildMeasurement, DynError> { Ok(measurement(1)) };
        let mut writer = Vec::new();

        assert!(write_report_with_measurement(&mut writer, config(1, 1), &mut measure,).unwrap());

        let records = String::from_utf8(writer)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let metadata = &records[0];
        assert_eq!(metadata["schema"], "flow-m12-executor-startup-v0");
        assert_eq!(metadata["environment"]["os"], std::env::consts::OS);
        assert_eq!(metadata["environment"]["arch"], std::env::consts::ARCH);
        assert_eq!(
            metadata["environment"]["reference_platform"],
            cfg!(any(
                all(target_os = "linux", target_arch = "x86_64"),
                all(target_os = "macos", target_arch = "aarch64")
            ))
        );
        assert_eq!(metadata["self_protection_required"], true);
        let sample = records
            .iter()
            .find(|record| record["kind"] == "sample")
            .unwrap();
        assert_eq!(sample["executor_elapsed_ns"], 1);
        assert_eq!(sample["self_protection_active"], true);
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
        assert_eq!(aggregate["inputs"]["self_protection_required"], true);
        assert_eq!(aggregate["inputs"]["tool_executions_per_child"], 1);
    }

    #[test]
    fn report_rejects_unverified_warmups_and_samples_without_losing_the_prefix() {
        for fail_at in [0_usize, 2] {
            for invalid_schema in [false, true] {
                let mut calls = 0;
                let mut measure = || -> Result<ChildMeasurement, DynError> {
                    let mut value = serde_json::to_value(measurement(1)).unwrap();
                    if calls == fail_at {
                        if invalid_schema {
                            value["schema"] = "wrong-schema".into();
                        } else {
                            value["self_protection_active"] = false.into();
                        }
                    }
                    calls += 1;
                    Ok(serde_json::from_value(value).unwrap())
                };
                let mut writer = FlushTrackingWriter::default();

                assert!(
                    !write_report_with_measurement(&mut writer, config(1, 3), &mut measure)
                        .unwrap()
                );
                assert_eq!(calls, fail_at + 1);
                assert!(writer.flushed);
                let records = String::from_utf8(writer.bytes)
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str::<Value>(line).unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(records[0]["kind"], "metadata");
                assert_eq!(
                    records
                        .iter()
                        .filter(|record| record["kind"] == "sample")
                        .count(),
                    fail_at.saturating_sub(1)
                );
                let failure = &records[records.len() - 2];
                assert_eq!(failure["kind"], "workload_failure");
                assert!(
                    failure["error"]
                        .as_str()
                        .unwrap()
                        .contains(if invalid_schema {
                            "schema"
                        } else {
                            "self-protection"
                        })
                );
                assert_eq!(records.last().unwrap()["kind"], "summary");
                assert_eq!(records.last().unwrap()["complete"], false);
                assert!(!records.iter().any(|record| record["kind"] == "aggregate"));
            }
        }
    }

    #[test]
    fn child_evidence_requires_the_guard_and_rejects_legacy_fields() {
        let valid = serde_json::to_value(measurement(1)).unwrap();
        let mut missing = valid.clone();
        missing
            .as_object_mut()
            .unwrap()
            .remove("self_protection_active");
        assert!(serde_json::from_value::<ChildMeasurement>(missing).is_err());
        for (field, value) in [
            ("max_concurrent_processes_and_threads", serde_json::json!(8)),
            ("runtime_profile", serde_json::json!("exact")),
            ("isolation_active", serde_json::json!(true)),
        ] {
            let mut legacy = valid.clone();
            legacy[field] = value;
            assert!(serde_json::from_value::<ChildMeasurement>(legacy).is_err());
        }
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
