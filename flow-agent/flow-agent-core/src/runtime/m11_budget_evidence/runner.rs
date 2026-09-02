use super::{M11BudgetOutcome, RSS_FIXTURE_BYTES, RSS_TOUCH_STRIDE_BYTES, outcome};
use std::{hint::black_box, path::Path, time::Instant};

#[cfg(unix)]
use crate::runtime::tool_runner::{
    MAX_TOOL_STREAM_BYTES, ToolInvocation, ToolRunControl, ToolTerminalClassification,
    execute_tool_invocation, measure_ready_process_group_cleanup, measure_ready_tool_cancellation,
};
#[cfg(unix)]
use crate::runtime::{fs_guards::AnchoredWorkspace, run_attempts::RunAttemptOutcome};
#[cfg(unix)]
use std::{sync::atomic::AtomicBool, time::Duration};

pub(super) const NOOP_LAUNCHES: usize = 4;
pub(super) const NOOP_EXECUTABLE: &str = "/usr/bin/true";

pub(super) fn rss_detection_fixture() -> Result<M11BudgetOutcome, String> {
    let started = Instant::now();
    let mut allocation = vec![0_u8; RSS_FIXTURE_BYTES];
    for offset in (0..allocation.len()).step_by(RSS_TOUCH_STRIDE_BYTES) {
        allocation[offset] = 0xa5;
    }
    if let Some(last) = allocation.last_mut() {
        *last = 0x5a;
    }
    let checksum = allocation
        .iter()
        .step_by(RSS_TOUCH_STRIDE_BYTES)
        .fold(0_u64, |sum, byte| sum.wrapping_add(u64::from(*byte)));
    black_box(&allocation);
    drop(allocation);
    Ok(outcome(
        started.elapsed(),
        1,
        RSS_FIXTURE_BYTES as u64,
        0,
        checksum,
    ))
}

#[cfg(unix)]
pub(super) fn runner_four_noop_launches(temp_root: &Path) -> Result<M11BudgetOutcome, String> {
    let cancelled = AtomicBool::new(false);
    let workspace = AnchoredWorkspace::open(temp_root)
        .map_err(|_| "runner workspace did not open".to_owned())?;
    let invocation = ToolInvocation {
        executable: NOOP_EXECUTABLE.to_owned(),
        argv: Vec::new(),
    };
    let started = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..NOOP_LAUNCHES {
        let result = execute_tool_invocation(
            &invocation,
            workspace.root(),
            ToolRunControl {
                cancelled: &cancelled,
                deadline: Instant::now() + Duration::from_secs(5),
            },
        );
        if result.status != RunAttemptOutcome::Completed
            || result.exit_code != Some(0)
            || !result.stdout.is_empty()
            || !result.stderr.is_empty()
        {
            return Err("a no-op runner launch did not complete exactly once".to_owned());
        }
        checksum = checksum.wrapping_add(1);
    }
    Ok(outcome(
        started.elapsed(),
        NOOP_LAUNCHES as u64,
        0,
        0,
        checksum,
    ))
}

#[cfg(not(unix))]
pub(super) fn runner_four_noop_launches(_: &Path) -> Result<M11BudgetOutcome, String> {
    Err("runner workloads require the selected Unix reference platform".to_owned())
}

#[cfg(unix)]
pub(super) fn runner_termination() -> Result<M11BudgetOutcome, String> {
    let elapsed = measure_ready_process_group_cleanup().map_err(str::to_owned)?;
    Ok(outcome(elapsed, 1, 0, 0, 0))
}

#[cfg(not(unix))]
pub(super) fn runner_termination() -> Result<M11BudgetOutcome, String> {
    Err("runner workloads require the selected Unix reference platform".to_owned())
}

#[cfg(unix)]
pub(super) fn runner_cancellation(temp_root: &Path) -> Result<M11BudgetOutcome, String> {
    let (elapsed, result) = measure_ready_tool_cancellation(temp_root).map_err(str::to_owned)?;
    if result.status != RunAttemptOutcome::Cancelled
        || result.classification != Some(ToolTerminalClassification::Cancelled)
        || result.exit_code.is_some()
        || !result.stdout.is_empty()
        || !result.stderr.is_empty()
    {
        return Err("ready Tool cancellation did not preserve its exact outcome".to_owned());
    }
    Ok(outcome(elapsed, 1, 0, 0, 1))
}

#[cfg(not(unix))]
pub(super) fn runner_cancellation(_: &Path) -> Result<M11BudgetOutcome, String> {
    Err("runner workloads require the selected Unix reference platform".to_owned())
}

#[cfg(unix)]
pub(super) fn runner_dual_stream_caps(temp_root: &Path) -> Result<M11BudgetOutcome, String> {
    let cancelled = AtomicBool::new(false);
    let workspace = AnchoredWorkspace::open(temp_root)
        .map_err(|_| "runner workspace did not open".to_owned())?;
    let invocation = ToolInvocation {
        executable: core_policy::OWN_SCRIPT_PRODUCTIVE_EXECUTABLE.to_owned(),
        argv: vec![
            "-c".to_owned(),
            format!(
                "(/usr/bin/head -c {MAX_TOOL_STREAM_BYTES} /dev/zero) & (/usr/bin/head -c {MAX_TOOL_STREAM_BYTES} /dev/zero >&2) & wait"
            ),
            "flow-m11-dual-stream".to_owned(),
        ],
    };
    let started = Instant::now();
    let result = execute_tool_invocation(
        &invocation,
        workspace.root(),
        ToolRunControl {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(10),
        },
    );
    let elapsed = started.elapsed();
    if result.status != RunAttemptOutcome::Completed
        || result.exit_code != Some(0)
        || result.stdout.len() != MAX_TOOL_STREAM_BYTES
        || result.stderr.len() != MAX_TOOL_STREAM_BYTES
    {
        return Err("dual-stream runner fixture did not retain both exact caps".to_owned());
    }
    let checksum = result
        .stdout
        .len()
        .wrapping_add(result.stderr.len())
        .try_into()
        .unwrap_or(u64::MAX);
    Ok(outcome(
        elapsed,
        1,
        (2 * MAX_TOOL_STREAM_BYTES) as u64,
        (result.stdout.len() + result.stderr.len()) as u64,
        checksum,
    ))
}

#[cfg(not(unix))]
pub(super) fn runner_dual_stream_caps(_: &Path) -> Result<M11BudgetOutcome, String> {
    Err("runner workloads require the selected Unix reference platform".to_owned())
}
