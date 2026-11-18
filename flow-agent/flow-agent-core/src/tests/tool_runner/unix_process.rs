use super::super::helpers::empty_workspace;
use crate::runtime::tool_runner::{
    MAX_TOOL_STREAM_BYTES, PrimaryTrigger, READY_CANCELLATION_MARKER, ToolExecutionOutcome,
    ToolInvocation, ToolRunControl, ToolTerminalClassification,
    execute_tool_invocation as execute_anchored_tool_invocation, force_reap_timeout_for_test,
    measure_ready_tool_cancellation, observe_group_signals_for_test, visible_exit_code,
};
use crate::runtime::{fs_guards::AnchoredWorkspace, run_attempts::RunAttemptOutcome};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn shell_invocation(body: &str) -> ToolInvocation {
    ToolInvocation {
        executable: proto::EXECUTOR_OWN_SCRIPT_EXECUTABLE_V0.to_owned(),
        argv: vec![
            "-c".to_owned(),
            body.to_owned(),
            "flow-tool:test".to_owned(),
        ],
    }
}

fn execute_tool_invocation(
    invocation: &ToolInvocation,
    workspace: &Path,
    control: ToolRunControl<'_>,
) -> ToolExecutionOutcome {
    let workspace = AnchoredWorkspace::open(workspace).expect("workspace root opens");
    execute_anchored_tool_invocation(invocation, workspace.root(), control)
}

fn run_tool(
    invocation: &ToolInvocation,
    workspace: &Path,
    timeout: Duration,
) -> ToolExecutionOutcome {
    let cancelled = AtomicBool::new(false);
    execute_tool_invocation(
        invocation,
        workspace,
        ToolRunControl {
            cancelled: &cancelled,
            deadline: Instant::now() + timeout,
        },
    )
}

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

fn contains_bytes(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

const TERM_READY: &str = "term-ready";

thread_local! {
    static TERM_READY_WORKSPACE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

fn await_term_ready(signal: rustix::process::Signal) {
    let Some(workspace) = TERM_READY_WORKSPACE.with_borrow(Clone::clone) else {
        return;
    };
    let ready_deadline = Instant::now() + Duration::from_secs(5);
    while signal == rustix::process::Signal::TERM
        && !workspace.join(TERM_READY).is_file()
        && Instant::now() < ready_deadline
    {
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn hold_term_until_ready(_: rustix::process::Pid, signal: rustix::process::Signal) {
    await_term_ready(signal);
}

fn execute_timed_out_after_term_ready(label: &str, body: &str) -> ToolExecutionOutcome {
    let workspace = empty_workspace(label);
    TERM_READY_WORKSPACE.set(Some(workspace.to_path_buf()));
    observe_group_signals_for_test(Some(hold_term_until_ready));
    let outcome = run_tool(
        &shell_invocation(body),
        &workspace,
        Duration::from_millis(100),
    );
    observe_group_signals_for_test(None);
    TERM_READY_WORKSPACE.set(None);
    outcome
}

#[test]
fn unix_runner_captures_both_streams_with_an_empty_environment() {
    let environment = run_tool(
        &ToolInvocation {
            executable: "/usr/bin/env".to_owned(),
            argv: Vec::new(),
        },
        Path::new("."),
        Duration::from_secs(5),
    );
    assert_eq!(environment.status, RunAttemptOutcome::Completed);
    assert!(environment.stdout.is_empty());

    let outcome = run_tool(
        &shell_invocation("printf stdout-value && printf stderr-value >&2"),
        Path::new("."),
        Duration::from_secs(5),
    );

    assert_eq!(outcome.status, RunAttemptOutcome::Completed);
    assert_eq!(outcome.classification, None);
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(outcome.stdout, b"stdout-value");
    assert_eq!(outcome.stderr, b"stderr-value");
}

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

#[test]
fn runner_omits_an_observed_exit_code_for_timeout_or_cancellation() {
    assert_eq!(visible_exit_code(&PrimaryTrigger::TimedOut, Some(0)), None);
    assert_eq!(visible_exit_code(&PrimaryTrigger::Cancelled, Some(0)), None);
    assert_eq!(
        visible_exit_code(&PrimaryTrigger::StdoutCap, Some(0)),
        Some(0)
    );
}

#[test]
fn runner_stream_budgets_accept_the_cap_and_fail_one_byte_beyond() {
    for (label, redirect, retained, classification) in [
        (
            "stdout",
            "",
            (MAX_TOOL_STREAM_BYTES, 0),
            ToolTerminalClassification::StdoutCapExceeded,
        ),
        (
            "stderr",
            " >&2",
            (0, MAX_TOOL_STREAM_BYTES),
            ToolTerminalClassification::StderrCapExceeded,
        ),
    ] {
        let exact = run_tool(
            &shell_invocation(&format!("/usr/bin/head -c 4194304 /dev/zero{redirect}")),
            Path::new("."),
            Duration::from_secs(10),
        );
        assert_eq!(exact.status, RunAttemptOutcome::Completed, "{label}");
        assert_eq!(
            (exact.stdout.len(), exact.stderr.len()),
            retained,
            "{label}"
        );

        let excess = run_tool(
            &shell_invocation(&format!("/usr/bin/head -c 4194305 /dev/zero{redirect}")),
            Path::new("."),
            Duration::from_secs(10),
        );
        assert_eq!(excess.status, RunAttemptOutcome::Failed, "{label}");
        assert_eq!(excess.classification, Some(classification), "{label}");
        assert!(matches!(excess.exit_code, None | Some(0)), "{label}");
        assert_eq!(
            (excess.stdout.len(), excess.stderr.len()),
            retained,
            "{label}"
        );
    }
}

#[test]
fn runner_cap_does_not_expose_an_exit_observed_during_cleanup() {
    let outcome = run_tool(
        &shell_invocation("/usr/bin/head -c 4194305 /dev/zero; /bin/sleep 0.05; exit 17"),
        Path::new("."),
        Duration::from_secs(10),
    );

    assert_eq!(outcome.status, RunAttemptOutcome::Failed);
    assert_eq!(
        outcome.classification,
        Some(ToolTerminalClassification::StdoutCapExceeded)
    );
    assert_eq!(outcome.exit_code, None);
}

#[test]
fn runner_owns_completed_leader_until_final_group_signal() {
    #[derive(Clone, Copy)]
    struct Observation {
        lost_ownership: bool,
        completed: bool,
        exit: Option<i32>,
        signal: rustix::process::Signal,
    }

    thread_local! {
        static OBSERVATIONS: RefCell<Vec<Observation>> = const { RefCell::new(Vec::new()) };
    }
    fn observe_owned_leader(pid: rustix::process::Pid, signal: rustix::process::Signal) {
        await_term_ready(signal);
        let status = rustix::process::waitid(
            rustix::process::WaitId::Pid(pid),
            rustix::process::WaitIdOptions::EXITED
                | rustix::process::WaitIdOptions::NOHANG
                | rustix::process::WaitIdOptions::NOWAIT,
        );
        let observation = Observation {
            lost_ownership: status.is_err(),
            completed: status.as_ref().is_ok_and(Option::is_some),
            exit: status
                .ok()
                .flatten()
                .and_then(|status| status.exit_status()),
            signal,
        };
        OBSERVATIONS.with_borrow_mut(|observations| observations.push(observation));
    }

    for (body, deadline, classification, expected_exit, last_signal_exit, needs_kill) in [
        (
            "exit 17",
            Duration::from_secs(5),
            ToolTerminalClassification::NonzeroExit,
            Some(17),
            Some(17),
            false,
        ),
        (
            "trap '' TERM; (while :; do :; done) & exit 17",
            Duration::from_secs(5),
            ToolTerminalClassification::NonzeroExit,
            Some(17),
            Some(17),
            true,
        ),
        (
            "trap '' TERM; (while :; do :; done) & trap - TERM; kill -TERM $$",
            Duration::from_secs(5),
            ToolTerminalClassification::SignalTermination,
            None,
            None,
            true,
        ),
        (
            "trap '' TERM; (while :; do :; done) & trap 'exit 17' TERM; : > term-ready; while :; do :; done",
            Duration::from_millis(100),
            ToolTerminalClassification::ToolTimedOut,
            None,
            Some(17),
            true,
        ),
    ] {
        let workspace = empty_workspace("runner-owns-completed-leader");
        TERM_READY_WORKSPACE.set(
            (classification == ToolTerminalClassification::ToolTimedOut)
                .then(|| workspace.to_path_buf()),
        );
        OBSERVATIONS.with_borrow_mut(Vec::clear);
        observe_group_signals_for_test(Some(observe_owned_leader));
        let outcome = run_tool(&shell_invocation(body), &workspace, deadline);
        observe_group_signals_for_test(None);
        TERM_READY_WORKSPACE.set(None);

        let observations = OBSERVATIONS.with_borrow(Clone::clone);
        assert!(
            observations
                .iter()
                .all(|observation| !observation.lost_ownership),
            "a group signal must retain ownership of its original leader"
        );
        let last = observations
            .last()
            .expect("a group signal observed the leader");
        assert!(
            last.completed,
            "the completed leader must remain waitable at the final signal"
        );
        assert_eq!(
            last.exit, last_signal_exit,
            "the retained leader must preserve its actual terminal status"
        );
        assert_eq!(outcome.classification, Some(classification));
        assert_eq!(outcome.exit_code, expected_exit);
        if needs_kill {
            assert_eq!(last.signal, rustix::process::Signal::KILL);
        }
    }
}

#[test]
fn runner_term_grace() {
    let started = Instant::now();
    let outcome = execute_timed_out_after_term_ready(
        "runner-term-grace",
        "trap '' TERM; : > term-ready; printf ready; while :; do /bin/sleep 1; done",
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

#[test]
fn forced_reap_fails_the_run_and_hands_the_leader_to_a_reaper() {
    force_reap_timeout_for_test(true);
    let started = Instant::now();
    let outcome = execute_timed_out_after_term_ready(
        "runner-forced-reap",
        "trap '' TERM; printf %s $$; : > term-ready; while :; do /bin/sleep 1; done",
    );
    force_reap_timeout_for_test(false);

    assert_eq!(outcome.status, RunAttemptOutcome::Failed);
    assert_eq!(
        outcome.classification,
        Some(ToolTerminalClassification::ProcessReapFailed)
    );
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert!(started.elapsed() < Duration::from_secs(4));

    let leader = std::str::from_utf8(&outcome.stdout)
        .expect("leader pid is UTF-8")
        .parse::<i32>()
        .expect("the Tool reports its leader pid");
    let leader = rustix::process::Pid::from_raw(leader).expect("leader pid is positive");
    let reaping = Instant::now();
    loop {
        match rustix::process::waitid(
            rustix::process::WaitId::Pid(leader),
            rustix::process::WaitIdOptions::EXITED
                | rustix::process::WaitIdOptions::NOHANG
                | rustix::process::WaitIdOptions::NOWAIT,
        ) {
            Err(rustix::io::Errno::CHILD) => break,
            Ok(_) => {}
            Err(error) => panic!("leader observation failed: {error}"),
        }
        assert!(
            reaping.elapsed() < Duration::from_secs(5),
            "the leader outlived its forced reap without being reaped"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn runner_output_drain() {
    const FILTER: &str = "tests::tool_runner::unix_process::runner_output_drain";
    const MARKER: &str = "escaped-output-holder";
    if run_escaped_output_fixture(MARKER, false) {
        return;
    }
    let workspace = empty_workspace("runner-output-drain");
    std::fs::write(workspace.join(MARKER), b"fixture").expect("fixture mode is selected");
    let started = Instant::now();
    let outcome = run_tool(
        &escaped_output_invocation(FILTER),
        &workspace,
        Duration::from_secs(5),
    );

    assert_eq!(outcome.status, RunAttemptOutcome::Failed);
    assert_eq!(
        outcome.classification,
        Some(ToolTerminalClassification::OutputDrainTimeout)
    );
    assert!(contains_bytes(&outcome.stdout, b"leader-done"));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn runner_output_drain_rejects_an_escaped_child_that_exceeds_the_stream_cap() {
    const FILTER: &str = "tests::tool_runner::unix_process::runner_output_drain_rejects_an_escaped_child_that_exceeds_the_stream_cap";
    const MARKER: &str = "escaped-output-cap";
    if run_escaped_output_fixture(MARKER, true) {
        return;
    }
    let workspace = empty_workspace("runner-output-drain-cap");
    std::fs::write(workspace.join(MARKER), b"fixture").expect("fixture mode is selected");
    let started = Instant::now();
    let outcome = run_tool(
        &escaped_output_invocation(FILTER),
        &workspace,
        Duration::from_secs(15),
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
