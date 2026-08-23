use super::super::helpers::empty_workspace;
use crate::runtime::tool_runner::{
    MAX_TOOL_STREAM_BYTES, PrimaryTrigger, READY_CANCELLATION_MARKER, ToolExecutionOutcome,
    ToolInvocation, ToolRunControl, ToolTerminalClassification,
    execute_tool_invocation as execute_anchored_tool_invocation, force_reap_timeout_for_test,
    measure_ready_tool_cancellation, visible_exit_code,
};
use crate::runtime::{fs_guards::AnchoredWorkspace, run_attempts::RunAttemptOutcome};
use std::{
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[cfg(unix)]
fn shell_invocation(body: &str) -> ToolInvocation {
    ToolInvocation {
        executable: "/bin/sh".to_owned(),
        argv: vec![
            "-c".to_owned(),
            body.to_owned(),
            "flow-tool:test".to_owned(),
        ],
    }
}

#[cfg(unix)]
fn execute_tool_invocation(
    invocation: &ToolInvocation,
    workspace: &Path,
    control: ToolRunControl<'_>,
) -> ToolExecutionOutcome {
    let workspace = AnchoredWorkspace::open(workspace).expect("workspace root opens");
    execute_anchored_tool_invocation(invocation, workspace.root(), control)
}

#[cfg(unix)]
fn escaped_output_invocation(fixture: &str) -> ToolInvocation {
    let mut invocation = shell_invocation(
        "\"$1\" --exact \"$2\" --nocapture & printf leader-done; while [ ! -f escaped.filled ]; do /bin/sleep 0.01; done",
    );
    invocation.argv.push(
        std::env::current_exe()
            .expect("the test executable has a path")
            .to_string_lossy()
            .into_owned(),
    );
    invocation.argv.push(fixture.to_owned());
    invocation
}

#[cfg(unix)]
fn run_escaped_output_fixture(marker: &str, write_cap: bool) -> bool {
    use std::io::Write as _;

    if !Path::new(marker).is_file() {
        return false;
    }
    rustix::process::setsid().expect("the background fixture can create a new session");
    if write_cap {
        std::thread::sleep(Duration::from_millis(50));
        let output = vec![0; MAX_TOOL_STREAM_BYTES + 1];
        std::io::stdout()
            .lock()
            .write_all(&output)
            .expect("the escaped fixture writes its retained stream");
    }
    std::fs::write("escaped.filled", b"filled")
        .expect("the escaped fixture publishes completed output");
    std::thread::sleep(Duration::from_secs(5));
    true
}

#[cfg(unix)]
fn contains_bytes(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

#[cfg(unix)]
#[test]
fn unix_runner_captures_both_streams_with_an_empty_environment() {
    let cancelled = AtomicBool::new(false);
    let empty_environment = execute_tool_invocation(
        &ToolInvocation {
            executable: "/usr/bin/env".to_owned(),
            argv: Vec::new(),
        },
        Path::new("."),
        ToolRunControl {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(5),
        },
    );
    assert_eq!(empty_environment.status, RunAttemptOutcome::Completed);
    assert!(empty_environment.stdout.is_empty());

    let outcome = execute_tool_invocation(
        &shell_invocation("printf stdout-value && printf stderr-value >&2"),
        Path::new("."),
        ToolRunControl {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(5),
        },
    );

    assert_eq!(outcome.status, RunAttemptOutcome::Completed);
    assert_eq!(outcome.classification, None);
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(outcome.stdout, b"stdout-value");
    assert_eq!(outcome.stderr, b"stderr-value");
}

#[cfg(unix)]
#[test]
fn runner_pre_cancelled_before_spawn_does_not_launch() {
    let cancelled = AtomicBool::new(true);
    let workspace = empty_workspace("runner-cancelled-before-spawn");
    let outcome = execute_tool_invocation(
        &shell_invocation("printf launched > cancelled-tool-started"),
        &workspace,
        ToolRunControl {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(5),
        },
    );

    assert_eq!(outcome.status, RunAttemptOutcome::Cancelled);
    assert_eq!(
        outcome.classification,
        Some(ToolTerminalClassification::Cancelled)
    );
    assert_eq!(outcome.exit_code, None);
    assert!(
        !workspace.join("cancelled-tool-started").exists(),
        "a pre-cancelled Tool must not start"
    );
}

#[cfg(unix)]
#[test]
fn runner_uses_the_retained_workspace_after_ambient_path_replacement() {
    let workspace = empty_workspace("runner-retained-workspace");
    let replacement = empty_workspace("runner-replacement-workspace");
    let moved = workspace.with_extension("original");
    let retained = AnchoredWorkspace::open(&workspace).expect("workspace root is retained");
    std::fs::rename(&*workspace, &moved).expect("original workspace moves");
    std::fs::rename(&*replacement, &*workspace).expect("replacement workspace is installed");

    let cancelled = AtomicBool::new(false);
    let outcome = execute_anchored_tool_invocation(
        &shell_invocation("printf retained > tool-workspace"),
        retained.root(),
        ToolRunControl {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(5),
        },
    );

    std::fs::rename(&*workspace, &*replacement).expect("replacement workspace restores");
    std::fs::rename(&moved, &*workspace).expect("original workspace restores");
    assert_eq!(outcome.status, RunAttemptOutcome::Completed);
    assert_eq!(
        std::fs::read(workspace.join("tool-workspace")).expect("Tool uses retained workspace"),
        b"retained"
    );
    assert!(
        !replacement.join("tool-workspace").exists(),
        "ambient replacement must receive no Tool side effect"
    );
}

#[cfg(unix)]
#[test]
fn runner_cancellation_lifecycle() {
    let workspace = empty_workspace("runner-cancelled-after-ready");
    let (_elapsed, outcome) = measure_ready_tool_cancellation(&workspace)
        .expect("one ready Tool is cancelled through the production controller");

    assert_eq!(outcome.status, RunAttemptOutcome::Cancelled);
    assert_eq!(
        outcome.classification,
        Some(ToolTerminalClassification::Cancelled)
    );
    assert_eq!(outcome.exit_code, None);
    assert!(outcome.stdout.is_empty());
    assert!(outcome.stderr.is_empty());
    assert_eq!(
        std::fs::read(workspace.join(READY_CANCELLATION_MARKER))
            .expect("the Tool published readiness"),
        b"ready"
    );
}

#[cfg(unix)]
#[test]
fn runner_omits_an_observed_exit_code_for_timeout_or_cancellation() {
    assert_eq!(visible_exit_code(&PrimaryTrigger::TimedOut, Some(0)), None);
    assert_eq!(visible_exit_code(&PrimaryTrigger::Cancelled, Some(0)), None);
    assert_eq!(
        visible_exit_code(&PrimaryTrigger::StdoutCap, Some(0)),
        Some(0)
    );
}

#[cfg(unix)]
fn stream_budget_fixture(body: &str) -> ToolExecutionOutcome {
    let cancelled = AtomicBool::new(false);
    execute_tool_invocation(
        &shell_invocation(body),
        Path::new("."),
        ToolRunControl {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(10),
        },
    )
}

#[cfg(unix)]
#[test]
fn runner_stdout_budget() {
    let exact = stream_budget_fixture("/usr/bin/head -c 4194304 /dev/zero");
    assert_eq!(exact.status, RunAttemptOutcome::Completed);
    assert_eq!(exact.stdout.len(), MAX_TOOL_STREAM_BYTES);
    assert!(exact.stderr.is_empty());

    let excess = stream_budget_fixture("/usr/bin/head -c 4194305 /dev/zero");
    assert_eq!(excess.status, RunAttemptOutcome::Failed);
    assert_eq!(
        excess.classification,
        Some(ToolTerminalClassification::StdoutCapExceeded)
    );
    assert!(matches!(excess.exit_code, None | Some(0)));
    assert_eq!(excess.stdout.len(), MAX_TOOL_STREAM_BYTES);
    assert!(excess.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn runner_stderr_budget() {
    let exact = stream_budget_fixture("/usr/bin/head -c 4194304 /dev/zero >&2");
    assert_eq!(exact.status, RunAttemptOutcome::Completed);
    assert_eq!(exact.stderr.len(), MAX_TOOL_STREAM_BYTES);
    assert!(exact.stdout.is_empty());

    let excess = stream_budget_fixture("/usr/bin/head -c 4194305 /dev/zero >&2");
    assert_eq!(excess.status, RunAttemptOutcome::Failed);
    assert_eq!(
        excess.classification,
        Some(ToolTerminalClassification::StderrCapExceeded)
    );
    assert!(matches!(excess.exit_code, None | Some(0)));
    assert_eq!(excess.stderr.len(), MAX_TOOL_STREAM_BYTES);
    assert!(excess.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn runner_cap_does_not_expose_an_exit_observed_during_cleanup() {
    let outcome =
        stream_budget_fixture("/usr/bin/head -c 4194305 /dev/zero; /bin/sleep 0.05; exit 17");

    assert_eq!(outcome.status, RunAttemptOutcome::Failed);
    assert_eq!(
        outcome.classification,
        Some(ToolTerminalClassification::StdoutCapExceeded)
    );
    assert_eq!(outcome.exit_code, None);
}

#[cfg(unix)]
#[test]
fn runner_noop_lifecycle() {
    let cancelled = AtomicBool::new(false);
    for _ in 0..4 {
        let outcome = execute_tool_invocation(
            &ToolInvocation {
                executable: "/usr/bin/true".to_owned(),
                argv: Vec::new(),
            },
            Path::new("."),
            ToolRunControl {
                cancelled: &cancelled,
                deadline: Instant::now() + Duration::from_secs(5),
            },
        );
        assert_eq!(outcome.status, RunAttemptOutcome::Completed);
        assert_eq!(outcome.exit_code, Some(0));
    }
}

#[cfg(unix)]
#[test]
fn runner_term_grace() {
    let cancelled = AtomicBool::new(false);
    let started = Instant::now();
    let outcome = execute_tool_invocation(
        &shell_invocation("trap '' TERM; printf ready; while :; do /bin/sleep 1; done"),
        Path::new("."),
        ToolRunControl {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_millis(100),
        },
    );

    assert_eq!(outcome.status, RunAttemptOutcome::TimedOut);
    assert_eq!(
        outcome.classification,
        Some(ToolTerminalClassification::ToolTimedOut)
    );
    assert_eq!(outcome.stdout, b"ready");
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[cfg(unix)]
#[test]
fn runner_forced_reap() {
    force_reap_timeout_for_test(true);
    let cancelled = AtomicBool::new(false);
    let started = Instant::now();
    let outcome = execute_tool_invocation(
        &shell_invocation("trap '' TERM; while :; do /bin/sleep 1; done"),
        Path::new("."),
        ToolRunControl {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_millis(100),
        },
    );
    force_reap_timeout_for_test(false);

    assert_eq!(outcome.status, RunAttemptOutcome::Failed);
    assert_eq!(
        outcome.classification,
        Some(ToolTerminalClassification::ProcessReapFailed)
    );
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[cfg(unix)]
#[test]
fn runner_output_drain() {
    const FILTER: &str = "tests::tool_runner::unix_process::runner_output_drain";
    const MARKER: &str = "escaped-output-holder";
    if run_escaped_output_fixture(MARKER, false) {
        return;
    }
    let workspace = empty_workspace("runner-output-drain");
    std::fs::write(workspace.join(MARKER), b"fixture").expect("fixture mode is selected");
    let cancelled = AtomicBool::new(false);
    let started = Instant::now();
    let outcome = execute_tool_invocation(
        &escaped_output_invocation(FILTER),
        &workspace,
        ToolRunControl {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(5),
        },
    );

    assert_eq!(outcome.status, RunAttemptOutcome::Failed);
    assert_eq!(
        outcome.classification,
        Some(ToolTerminalClassification::OutputDrainTimeout)
    );
    assert!(contains_bytes(&outcome.stdout, b"leader-done"));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[cfg(unix)]
#[test]
fn runner_output_drain_rejects_an_escaped_child_that_exceeds_the_stream_cap() {
    const FILTER: &str = "tests::tool_runner::unix_process::runner_output_drain_rejects_an_escaped_child_that_exceeds_the_stream_cap";
    const MARKER: &str = "escaped-output-cap";
    if run_escaped_output_fixture(MARKER, true) {
        return;
    }
    let workspace = empty_workspace("runner-output-drain-cap");
    std::fs::write(workspace.join(MARKER), b"fixture").expect("fixture mode is selected");
    let cancelled = AtomicBool::new(false);
    let started = Instant::now();
    let outcome = execute_tool_invocation(
        &escaped_output_invocation(FILTER),
        &workspace,
        ToolRunControl {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(15),
        },
    );

    assert_eq!(outcome.status, RunAttemptOutcome::Failed);
    assert_eq!(
        outcome.classification,
        Some(ToolTerminalClassification::OutputDrainTimeout)
    );
    assert!(contains_bytes(&outcome.stdout, b"leader-done"));
    assert_eq!(outcome.stdout.len(), MAX_TOOL_STREAM_BYTES);
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert!(started.elapsed() < Duration::from_secs(10));
}
