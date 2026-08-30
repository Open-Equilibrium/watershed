//! Feature-gated direct-runner evidence for the M1.2 startup baseline.

use serde::{Deserialize, Serialize};
use std::{
    hint::black_box,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

#[cfg(unix)]
use crate::runtime::{
    fs_guards::AnchoredWorkspace,
    run_attempts::RunAttemptOutcome,
    tool_runner::{ToolInvocation, ToolRunControl, execute_tool_invocation},
};
#[cfg(unix)]
use std::{env, sync::atomic::AtomicBool};

const TOOL_REPORT_SCHEMA: &str = "flow-m12-noop-tool-v0";
const MAX_TOOL_REPORT_BYTES: usize = 128;
#[cfg(unix)]
const TOOL_DEADLINE: Duration = Duration::from_secs(5);

/// The sole internal argument that selects the fixed no-op Tool child.
pub const M12_STARTUP_TOOL_CHILD_ARG: &str = "--m12-noop-tool-child";

/// One unadjusted M1.2 direct-runner startup observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct M12DirectRunnerMeasurement {
    /// Direct-runner handoff through terminal classification, reap and output drain.
    pub runner_elapsed: Duration,
    /// Runtime independently observed inside the fixed no-op Tool child.
    pub tool_runtime: Duration,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NoopToolReport {
    schema: String,
    tool_runtime_ns: u64,
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Writes the bounded, schema-tagged result of the exact fixed no-op Tool work.
pub fn write_m12_noop_tool_child_report(writer: &mut impl Write) -> Result<(), String> {
    let started = Instant::now();
    let _ = black_box(0_u8);
    let report = NoopToolReport {
        schema: TOOL_REPORT_SCHEMA.to_owned(),
        tool_runtime_ns: duration_ns(started.elapsed()),
    };
    let encoded = serde_json::to_vec(&report).map_err(|_| "no-op Tool report did not encode")?;
    if encoded.len() + 1 > MAX_TOOL_REPORT_BYTES {
        return Err("no-op Tool report exceeded its byte bound".to_owned());
    }
    writer
        .write_all(&encoded)
        .and_then(|()| writer.write_all(b"\n"))
        .map_err(|_| "no-op Tool report could not be written".to_owned())
}

#[cfg(any(unix, test))]
fn parse_noop_tool_report(bytes: &[u8], runner_elapsed: Duration) -> Result<Duration, String> {
    if bytes.is_empty() || bytes.len() > MAX_TOOL_REPORT_BYTES {
        return Err("no-op Tool report violated its byte bound".to_owned());
    }
    let report: NoopToolReport =
        serde_json::from_slice(bytes).map_err(|_| "no-op Tool report was not exact JSON")?;
    if report.schema != TOOL_REPORT_SCHEMA {
        return Err("no-op Tool report schema did not match".to_owned());
    }
    let runtime = Duration::from_nanos(report.tool_runtime_ns);
    if runtime > runner_elapsed {
        return Err("no-op Tool runtime exceeded the enclosing runner interval".to_owned());
    }
    Ok(runtime)
}

/// Measures one fixed no-op Tool invocation through the M1.1 direct runner.
#[cfg(unix)]
pub fn run_m12_direct_runner_startup(
    workspace: &Path,
) -> Result<M12DirectRunnerMeasurement, String> {
    let executable = env::current_exe()
        .map_err(|_| "measurement child executable did not resolve")?
        .into_os_string()
        .into_string()
        .map_err(|_| "measurement child executable was not UTF-8")?;
    let workspace =
        AnchoredWorkspace::open(workspace).map_err(|_| "direct-runner workspace did not open")?;
    let cancelled = AtomicBool::new(false);
    let invocation = ToolInvocation {
        executable,
        argv: vec![M12_STARTUP_TOOL_CHILD_ARG.to_owned()],
    };

    let started = Instant::now();
    let outcome = execute_tool_invocation(
        &invocation,
        workspace.root(),
        ToolRunControl {
            cancelled: &cancelled,
            deadline: Instant::now() + TOOL_DEADLINE,
        },
    );
    let runner_elapsed = started.elapsed();
    if outcome.status != RunAttemptOutcome::Completed
        || outcome.classification.is_some()
        || outcome.exit_code != Some(0)
        || !outcome.stderr.is_empty()
    {
        return Err("direct runner did not return one exact successful Tool result".to_owned());
    }
    let tool_runtime = parse_noop_tool_report(&outcome.stdout, runner_elapsed)?;
    Ok(M12DirectRunnerMeasurement {
        runner_elapsed,
        tool_runtime,
    })
}

/// Reports that the selected direct runner is unavailable off Unix.
#[cfg(not(unix))]
pub fn run_m12_direct_runner_startup(_: &Path) -> Result<M12DirectRunnerMeasurement, String> {
    Err("M1.2 startup evidence requires the selected Ubuntu reference platform".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_TOOL_REPORT_BYTES, TOOL_REPORT_SCHEMA, parse_noop_tool_report,
        write_m12_noop_tool_child_report,
    };
    use std::time::Duration;

    #[test]
    fn fixed_tool_report_is_bounded_and_schema_tagged() {
        let mut report = Vec::new();
        write_m12_noop_tool_child_report(&mut report).unwrap();

        assert!(report.len() <= MAX_TOOL_REPORT_BYTES);
        let runtime = parse_noop_tool_report(&report, Duration::from_secs(1)).unwrap();
        assert!(runtime <= Duration::from_secs(1));
        assert!(
            String::from_utf8(report)
                .unwrap()
                .contains(TOOL_REPORT_SCHEMA)
        );
    }

    #[test]
    fn tool_report_rejects_schema_drift_and_unbounded_output() {
        assert!(
            parse_noop_tool_report(
                br#"{"schema":"other","tool_runtime_ns":0}"#,
                Duration::from_secs(1)
            )
            .is_err()
        );
        assert!(
            parse_noop_tool_report(&[b' '; MAX_TOOL_REPORT_BYTES + 1], Duration::from_secs(1))
                .is_err()
        );
    }
}
