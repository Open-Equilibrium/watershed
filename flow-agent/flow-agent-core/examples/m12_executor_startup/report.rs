use super::{
    ChildMeasurement, Config, M12_EXECUTOR_STARTUP_PROCESS_CAPACITY, fresh_child_measurement,
};
use crate::evidence_support::{
    DynError, Environment as CommonEnvironment, bounded_environment_value,
    current_environment as common_environment, percentile, write_jsonl,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{error::Error, io::Write};

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
    contract_image: Option<String>,
    logical_cpus: usize,
    cpu_model: Option<String>,
    total_memory_bytes: Option<u64>,
    host_isolation: HostIsolation,
}

#[derive(Serialize)]
struct HostIsolation {
    systemd_version: Option<String>,
    cgroup_version: Option<u8>,
    cgroup_path: Option<String>,
    pids_controller_available: bool,
    pids_events_available: bool,
}

#[derive(Serialize)]
struct Metadata {
    kind: &'static str,
    schema: &'static str,
    benchmark_suite: &'static str,
    warmup_samples: usize,
    measured_samples: usize,
    tool_executions_per_fresh_child: usize,
    max_concurrent_processes_and_threads: u32,
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
        "max_concurrent_processes_and_threads": M12_EXECUTOR_STARTUP_PROCESS_CAPACITY,
        "executor_interval": [
            "selected Executor preparation and readiness",
            "canonical request and capability preparation",
            "one-shot Executor and Sandbox lifecycle",
            "validated terminal Tool result and enforcement receipt"
        ],
        "distribution": "executor_elapsed_ns"
    })
}

#[cfg(target_os = "linux")]
fn host_isolation_metadata() -> HostIsolation {
    let cgroup_root = std::path::Path::new("/sys/fs/cgroup");
    let cgroup_v2 = cgroup_root.join("cgroup.controllers");
    let cgroup_path = std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|source| {
            source
                .lines()
                .find_map(|line| line.strip_prefix("0::"))
                .map(str::to_owned)
        });
    let pids_controller_available = std::fs::read_to_string(&cgroup_v2)
        .ok()
        .is_some_and(|controllers| controllers.split_whitespace().any(|name| name == "pids"));
    let pids_events_available = cgroup_path.as_deref().is_some_and(|path| {
        cgroup_root
            .join(path.trim_start_matches('/'))
            .join("pids.events")
            .is_file()
    });
    let systemd_version = std::process::Command::new("systemd")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .map(|line| line.chars().take(256).collect())
        });
    HostIsolation {
        systemd_version,
        cgroup_version: cgroup_v2.is_file().then_some(2),
        cgroup_path,
        pids_controller_available,
        pids_events_available,
    }
}

#[cfg(not(target_os = "linux"))]
fn host_isolation_metadata() -> HostIsolation {
    HostIsolation {
        systemd_version: None,
        cgroup_version: None,
        cgroup_path: None,
        pids_controller_available: false,
        pids_events_available: false,
    }
}

fn current_environment() -> Environment {
    let CommonEnvironment {
        os,
        arch,
        rustc,
        reference_platform,
        commit_sha,
        runner_image,
        runner_image_version,
        logical_cpus,
        cpu_model,
        total_memory_bytes,
    } = common_environment();
    Environment {
        os,
        arch,
        rustc,
        reference_platform,
        commit_sha,
        runner_image,
        runner_image_version,
        contract_image: bounded_environment_value("M12_CONTRACT_IMAGE", 256),
        logical_cpus,
        cpu_model,
        total_memory_bytes,
        host_isolation: host_isolation_metadata(),
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
            max_concurrent_processes_and_threads: M12_EXECUTOR_STARTUP_PROCESS_CAPACITY,
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
