use super::contract::workload_contract;
use super::{ChildMeasurement, Config, RssMeasurement, fresh_child_measurement};
use crate::evidence_support::{
    DynError, Environment, current_environment, percentile, write_jsonl,
};
use flow_agent_core::{M11_BUDGET_WORKLOADS, M11BudgetWorkload, validate_m11_rss_measurement};
use serde::Serialize;
use serde_json::Value;
use std::{error::Error, io::Write};

const REPORT_SCHEMA: &str = "flow-m11-performance-evidence-v0";
const REPORT_SUITE: &str = "Flow Agent M1.1 performance evidence";
const RSS_METHOD: &str = "fresh-child-linux-vmhwm-minus-vmrss-v0";

#[derive(Serialize)]
struct Metadata {
    kind: &'static str,
    schema: &'static str,
    benchmark_suite: &'static str,
    warmup_samples: usize,
    measured_samples_per_workload: usize,
    workload_count: usize,
    rss_method: &'static str,
    environment: Environment,
}

#[derive(Serialize)]
struct RawSample<'a> {
    kind: &'static str,
    schema: &'static str,
    benchmark: &'a str,
    sample: usize,
    elapsed_ns: u64,
    operations: u64,
    input_bytes: u64,
    output_bytes: u64,
    checksum: u64,
    rss: RssMeasurement,
}

#[derive(Serialize)]
struct Aggregate<'a> {
    kind: &'static str,
    schema: &'static str,
    benchmark: &'a str,
    count: usize,
    p50_ns: u64,
    p95_ns: u64,
    max_ns: u64,
    peak_rss_growth_max_bytes: Option<u64>,
    retained_rss_growth_max_bytes: Option<u64>,
    inputs: Value,
    exclusions: &'a [&'a str],
}

#[derive(Serialize)]
struct WorkloadFailure<'a> {
    kind: &'static str,
    schema: &'static str,
    benchmark: &'a str,
    error: &'a str,
    inputs: Value,
    exclusions: &'a [&'a str],
}

#[derive(Serialize)]
struct Summary<'a> {
    kind: &'static str,
    schema: &'static str,
    complete: bool,
    failing_workloads: &'a [String],
}

fn write_workload_failure(
    writer: &mut impl Write,
    workload: &M11BudgetWorkload,
    error: &dyn Error,
) -> Result<(), DynError> {
    let error = error.to_string();
    let (inputs, exclusions) = workload_contract(workload.id);
    write_jsonl(
        writer,
        &WorkloadFailure {
            kind: "workload_failure",
            schema: REPORT_SCHEMA,
            benchmark: workload.name(),
            error: &error,
            inputs,
            exclusions,
        },
    )
}

fn write_workload_report(
    writer: &mut impl Write,
    workload: &M11BudgetWorkload,
    config: Config,
    measure: &mut impl FnMut(&M11BudgetWorkload, usize) -> Result<ChildMeasurement, DynError>,
) -> Result<bool, DynError> {
    for iteration in 0..config.warmups {
        if let Err(error) = measure(workload, iteration) {
            write_workload_failure(writer, workload, error.as_ref())?;
            return Ok(false);
        }
    }

    let mut rss_measurements = Vec::with_capacity(config.samples);
    let mut samples = Vec::with_capacity(config.samples);
    for sample in 0..config.samples {
        let measurement = match measure(workload, config.warmups + sample) {
            Ok(measurement) => measurement,
            Err(error) => {
                write_workload_failure(writer, workload, error.as_ref())?;
                return Ok(false);
            }
        };
        if let Err(error) =
            validate_m11_rss_measurement(workload.id, measurement.rss.peak_growth_bytes)
        {
            write_workload_failure(writer, workload, &std::io::Error::other(error))?;
            return Ok(false);
        }
        rss_measurements.push(measurement.rss);
        write_jsonl(
            writer,
            &RawSample {
                kind: "sample",
                schema: REPORT_SCHEMA,
                benchmark: workload.name(),
                sample,
                elapsed_ns: measurement.elapsed_ns,
                operations: measurement.operations,
                input_bytes: measurement.input_bytes,
                output_bytes: measurement.output_bytes,
                checksum: measurement.checksum,
                rss: measurement.rss,
            },
        )?;
        samples.push(measurement.elapsed_ns);
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 50, 100);
    let p95 = percentile(&samples, 95, 100);
    let max = *samples.last().expect("sample count is nonzero");
    let peak_rss_growth_max_bytes = rss_measurements
        .iter()
        .filter_map(|measurement| measurement.peak_growth_bytes)
        .max();
    let retained_rss_growth_max_bytes = rss_measurements
        .iter()
        .filter_map(|measurement| measurement.retained_growth_bytes)
        .max();
    let (inputs, exclusions) = workload_contract(workload.id);
    write_jsonl(
        writer,
        &Aggregate {
            kind: "aggregate",
            schema: REPORT_SCHEMA,
            benchmark: workload.name(),
            count: config.samples,
            p50_ns: p50,
            p95_ns: p95,
            max_ns: max,
            peak_rss_growth_max_bytes,
            retained_rss_growth_max_bytes,
            inputs,
            exclusions,
        },
    )?;
    Ok(true)
}

pub(super) fn write_report(writer: &mut impl Write, config: Config) -> Result<bool, DynError> {
    write_report_with_measurement(writer, config, &mut fresh_child_measurement)
}

pub(super) fn write_report_with_measurement(
    writer: &mut impl Write,
    config: Config,
    measure: &mut impl FnMut(&M11BudgetWorkload, usize) -> Result<ChildMeasurement, DynError>,
) -> Result<bool, DynError> {
    write_jsonl(
        writer,
        &Metadata {
            kind: "metadata",
            schema: REPORT_SCHEMA,
            benchmark_suite: REPORT_SUITE,
            warmup_samples: config.warmups,
            measured_samples_per_workload: config.samples,
            workload_count: M11_BUDGET_WORKLOADS.len(),
            rss_method: RSS_METHOD,
            environment: current_environment(),
        },
    )?;
    let mut failures = Vec::new();
    for workload in &M11_BUDGET_WORKLOADS {
        if !write_workload_report(writer, workload, config, measure)? {
            failures.push(workload.name().to_owned());
        }
    }
    write_jsonl(
        writer,
        &Summary {
            kind: "summary",
            schema: REPORT_SCHEMA,
            complete: failures.is_empty(),
            failing_workloads: &failures,
        },
    )?;
    writer.flush()?;
    Ok(failures.is_empty())
}
