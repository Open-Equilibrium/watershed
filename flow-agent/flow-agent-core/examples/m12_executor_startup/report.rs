use super::{ChildMeasurement, Config, DynError, fresh_child_measurement};
use serde::Serialize;
use serde_json::{Value, json};
use std::{env, error::Error, io::Write, process::Command};

const REPORT_SCHEMA: &str = "flow-m12-executor-startup-v0";
const REPORT_SUITE: &str = "Flow Agent M1.2 Executor startup evidence";
const BENCHMARK: &str = "prepared_selected_executor_single_noop_tool";

#[derive(Serialize)]
struct Environment {
    os: &'static str,
    arch: &'static str,
    rustc: String,
    reference_platform: bool,
    commit_sha: Option<String>,
    runner_image: Option<String>,
    runner_image_version: Option<String>,
    logical_cpus: usize,
    cpu_model: Option<String>,
    total_memory_bytes: Option<u64>,
}

#[derive(Serialize)]
struct Metadata {
    kind: &'static str,
    schema: &'static str,
    benchmark_suite: &'static str,
    warmup_samples: usize,
    measured_samples: usize,
    tool_executions_per_fresh_child: usize,
    environment: Environment,
}

#[derive(Serialize)]
struct RawSample {
    kind: &'static str,
    schema: &'static str,
    benchmark: &'static str,
    sample: usize,
    executor_elapsed_ns: u64,
}

#[derive(Serialize)]
struct Aggregate {
    kind: &'static str,
    schema: &'static str,
    benchmark: &'static str,
    count: usize,
    executor_p50_ns: u64,
    executor_p95_ns: u64,
    executor_max_ns: u64,
    inputs: Value,
}

#[derive(Serialize)]
struct WorkloadFailure<'a> {
    kind: &'static str,
    schema: &'static str,
    benchmark: &'static str,
    error: &'a str,
    inputs: Value,
}

#[derive(Serialize)]
struct Summary {
    kind: &'static str,
    schema: &'static str,
    complete: bool,
}

fn inputs() -> Value {
    json!({
        "boundary": "prepared_selected_executor",
        "fresh_measurement_child": true,
        "executor_selection": "explicit absolute path configured before interval",
        "tool_executions_per_child": 1,
        "tool": "/bin/echo",
        "tool_arguments": [],
        "tool_environment": "empty",
        "runtime_profile": "exact",
        "executor_interval": [
            "selected Executor preparation and readiness",
            "canonical request and capability preparation",
            "one-shot Executor and Sandbox lifecycle",
            "validated terminal Tool result and enforcement receipt"
        ],
        "distribution": "executor_elapsed_ns"
    })
}

pub(super) fn percentile(sorted: &[u64], numerator: usize, denominator: usize) -> u64 {
    let rank = sorted
        .len()
        .saturating_mul(numerator)
        .div_ceil(denominator)
        .saturating_sub(1);
    sorted[rank.min(sorted.len().saturating_sub(1))]
}

pub(super) fn write_jsonl(writer: &mut impl Write, value: &impl Serialize) -> Result<(), DynError> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    Ok(())
}

fn bounded_environment_value(name: &str, maximum_bytes: usize) -> Option<String> {
    let value = env::var(name).ok()?;
    (value.len() <= maximum_bytes).then_some(value)
}

#[cfg(target_os = "linux")]
fn hardware_metadata() -> (Option<String>, Option<u64>) {
    let cpu_model = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|source| {
            source.lines().find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|line| line.split_once(':'))
                    .map(|(_, value)| value.trim().chars().take(256).collect())
            })
        });
    let memory = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|source| {
            source.lines().find_map(|line| {
                let value = line.strip_prefix("MemTotal:")?.split_whitespace().next()?;
                value.parse::<u64>().ok()?.checked_mul(1024)
            })
        });
    (cpu_model, memory)
}

#[cfg(not(target_os = "linux"))]
fn hardware_metadata() -> (Option<String>, Option<u64>) {
    (None, None)
}

fn current_environment() -> Environment {
    let rustc = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unavailable".to_owned());
    let (cpu_model, total_memory_bytes) = hardware_metadata();
    Environment {
        os: env::consts::OS,
        arch: env::consts::ARCH,
        rustc,
        reference_platform: cfg!(all(target_os = "linux", target_arch = "x86_64")),
        commit_sha: bounded_environment_value("GITHUB_SHA", 128),
        runner_image: bounded_environment_value("ImageOS", 128),
        runner_image_version: bounded_environment_value("ImageVersion", 128),
        logical_cpus: std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
        cpu_model,
        total_memory_bytes,
    }
}

fn write_failure(writer: &mut impl Write, error: &dyn Error) -> Result<(), DynError> {
    let error = error.to_string();
    write_jsonl(
        writer,
        &WorkloadFailure {
            kind: "workload_failure",
            schema: REPORT_SCHEMA,
            benchmark: BENCHMARK,
            error: &error,
            inputs: inputs(),
        },
    )
}

pub(super) fn write_report(writer: &mut impl Write, config: Config) -> Result<bool, DynError> {
    let executor = config.executor.clone();
    write_report_with_measurement(writer, config, &mut || fresh_child_measurement(&executor))
}

pub(super) fn write_report_with_measurement(
    writer: &mut impl Write,
    config: Config,
    measure: &mut impl FnMut() -> Result<ChildMeasurement, DynError>,
) -> Result<bool, DynError> {
    write_jsonl(
        writer,
        &Metadata {
            kind: "metadata",
            schema: REPORT_SCHEMA,
            benchmark_suite: REPORT_SUITE,
            warmup_samples: config.warmups,
            measured_samples: config.samples,
            tool_executions_per_fresh_child: 1,
            environment: current_environment(),
        },
    )?;

    for _ in 0..config.warmups {
        if let Err(error) = measure() {
            write_failure(writer, error.as_ref())?;
            write_jsonl(
                writer,
                &Summary {
                    kind: "summary",
                    schema: REPORT_SCHEMA,
                    complete: false,
                },
            )?;
            writer.flush()?;
            return Ok(false);
        }
    }

    let mut executor_samples = Vec::with_capacity(config.samples);
    for sample in 0..config.samples {
        let measurement = match measure() {
            Ok(measurement) => measurement,
            Err(error) => {
                write_failure(writer, error.as_ref())?;
                write_jsonl(
                    writer,
                    &Summary {
                        kind: "summary",
                        schema: REPORT_SCHEMA,
                        complete: false,
                    },
                )?;
                writer.flush()?;
                return Ok(false);
            }
        };
        write_jsonl(
            writer,
            &RawSample {
                kind: "sample",
                schema: REPORT_SCHEMA,
                benchmark: BENCHMARK,
                sample,
                executor_elapsed_ns: measurement.executor_elapsed_ns,
            },
        )?;
        executor_samples.push(measurement.executor_elapsed_ns);
    }

    executor_samples.sort_unstable();
    write_jsonl(
        writer,
        &Aggregate {
            kind: "aggregate",
            schema: REPORT_SCHEMA,
            benchmark: BENCHMARK,
            count: config.samples,
            executor_p50_ns: percentile(&executor_samples, 50, 100),
            executor_p95_ns: percentile(&executor_samples, 95, 100),
            executor_max_ns: *executor_samples.last().expect("sample count is nonzero"),
            inputs: inputs(),
        },
    )?;
    write_jsonl(
        writer,
        &Summary {
            kind: "summary",
            schema: REPORT_SCHEMA,
            complete: true,
        },
    )?;
    writer.flush()?;
    Ok(true)
}
