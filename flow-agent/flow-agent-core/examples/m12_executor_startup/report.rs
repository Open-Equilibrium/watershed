use super::{ChildMeasurement, Config, fresh_child_measurement};
use crate::evidence_support::{
    DynError, Environment, current_environment as common_environment, percentile, write_jsonl,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{error::Error, io::Write};

const REPORT_SCHEMA: &str = "flow-m12-executor-startup-v0";
const REPORT_SUITE: &str = "Flow Agent M1.2 Executor startup evidence";
const BENCHMARK: &str = "prepared_selected_executor_single_noop_tool";

#[derive(Serialize)]
struct Metadata {
    kind: &'static str,
    schema: &'static str,
    benchmark_suite: &'static str,
    warmup_samples: usize,
    measured_samples: usize,
    tool_executions_per_fresh_child: usize,
    self_protection_required: bool,
    environment: Environment,
}

#[derive(Serialize)]
struct RawSample {
    kind: &'static str,
    schema: &'static str,
    benchmark: &'static str,
    sample: usize,
    executor_elapsed_ns: u64,
    self_protection_active: bool,
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
        "self_protection_required": true,
        "executor_interval": [
            "selected Executor preparation and readiness",
            "canonical invocation and own-file protection preparation",
            "one-shot Executor and Tool lifecycle",
            "validated terminal Tool result and enforcement receipt"
        ],
        "distribution": "executor_elapsed_ns"
    })
}

fn current_environment() -> Environment {
    let mut environment = common_environment();
    environment.reference_platform = cfg!(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    ));
    environment
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
            self_protection_required: true,
            environment: current_environment(),
        },
    )?;

    for _ in 0..config.warmups {
        if let Err(error) = measure().and_then(ChildMeasurement::validate) {
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
        let measurement = match measure().and_then(ChildMeasurement::validate) {
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
                self_protection_active: measurement.self_protection_active,
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
