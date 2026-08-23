use super::contract::workload_contract;
use super::{
    ChildMeasurement, Config, DynError, RssMeasurement, duration_ns, fresh_child_measurement,
};
use flow_agent_core::{M11_BUDGET_WORKLOADS, M11BudgetWorkload};
use serde::Serialize;
use serde_json::Value;
use std::{env, error::Error, io::Write, process::Command};

const REPORT_SCHEMA: &str = "flow-m11-budget-v0";
const REPORT_SUITE: &str = "Flow Agent M1.1 optimized budget evidence";
const RSS_METHOD: &str = "fresh-child-linux-vmhwm-minus-vmrss-v0";

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
    p95_limit_ns: Option<u64>,
    peak_rss_growth_max_bytes: Option<u64>,
    retained_rss_growth_max_bytes: Option<u64>,
    max_peak_rss_growth_limit_bytes: Option<u64>,
    min_peak_rss_growth_limit_bytes: Option<u64>,
    timing_passed: bool,
    rss_passed: bool,
    passed: bool,
    inputs: Value,
    exclusions: &'a [&'a str],
}

#[derive(Serialize)]
struct WorkloadFailure<'a> {
    kind: &'static str,
    schema: &'static str,
    benchmark: &'a str,
    passed: bool,
    error: &'a str,
    inputs: Value,
    exclusions: &'a [&'a str],
}

#[derive(Serialize)]
struct Summary<'a> {
    kind: &'static str,
    schema: &'static str,
    passed: bool,
    failing_workloads: &'a [String],
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
            passed: false,
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
    let p95_limit_ns = workload.p95_limit.map(duration_ns);
    let timing_passed = p95_limit_ns.is_none_or(|limit| p95 <= limit);
    let rss_reference = cfg!(target_os = "linux");
    let maximum_rss_passed = if rss_reference {
        workload
            .max_peak_rss_growth_bytes
            .is_none_or(|limit| peak_rss_growth_max_bytes.is_some_and(|actual| actual <= limit))
    } else {
        true
    };
    let minimum_rss_passed = if rss_reference {
        workload
            .min_peak_rss_growth_bytes
            .is_none_or(|limit| peak_rss_growth_max_bytes.is_some_and(|actual| actual >= limit))
    } else {
        true
    };
    let rss_passed = maximum_rss_passed && minimum_rss_passed;
    let passed = timing_passed && rss_passed;
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
            p95_limit_ns,
            peak_rss_growth_max_bytes,
            retained_rss_growth_max_bytes,
            max_peak_rss_growth_limit_bytes: workload.max_peak_rss_growth_bytes,
            min_peak_rss_growth_limit_bytes: workload.min_peak_rss_growth_bytes,
            timing_passed,
            rss_passed,
            passed,
            inputs,
            exclusions,
        },
    )?;
    Ok(passed)
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
            passed: failures.is_empty(),
            failing_workloads: &failures,
        },
    )?;
    writer.flush()?;
    Ok(failures.is_empty())
}
