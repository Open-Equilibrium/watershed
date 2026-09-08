use super::{
    native::{apply_inner_status, checked_output},
    protection,
    supervision::{
        PrimaryTrigger, ProcessOutcome, classify_exit, reportable_status, run_bounded,
        select_primary, terminate_and_reap, tool_result,
    },
};
use proto::ExecutorToolClassificationV0 as Classification;
use rustix::fd::AsRawFd;
use std::{
    fs::File,
    os::unix::process::ExitStatusExt,
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

fn natural_outcome(raw_status: i32) -> ProcessOutcome {
    let status = Some(ExitStatus::from_raw(raw_status));
    ProcessOutcome {
        status,
        classification: classify_exit(status),
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}

#[test]
fn inner_status_requires_one_exact_root_record() {
    for bytes in [
        Vec::new(),
        vec![1],
        vec![1, 2],
        vec![1, 2, 3],
        vec![1, 2, 3, 4, 5],
        vec![0; 8],
    ] {
        assert!(
            apply_inner_status(&mut natural_outcome(0), &bytes).is_err(),
            "{} status bytes cannot prove one reaped Tool root",
            bytes.len()
        );
    }
    apply_inner_status(&mut natural_outcome(0), &0_i32.to_ne_bytes())
        .expect("one complete status record is accepted");
}

#[test]
fn natural_inner_exit_requires_successful_supervision_and_exact_tool_status() {
    let recorded = (7_i32 << 8).to_ne_bytes();
    assert!(apply_inner_status(&mut natural_outcome(65_i32 << 8), &recorded).is_err());
    let mut missing_supervisor = natural_outcome(0);
    missing_supervisor.status = None;
    assert!(apply_inner_status(&mut missing_supervisor, &recorded).is_err());

    for raw_status in [0, 7_i32 << 8, 15] {
        let mut outcome = natural_outcome(0);
        apply_inner_status(&mut outcome, &raw_status.to_ne_bytes())
            .expect("reaped Tool status is accepted");
        assert_eq!(outcome.status, Some(ExitStatus::from_raw(raw_status)));
        assert_eq!(outcome.classification, classify_exit(outcome.status));
    }
}

#[test]
fn bounded_failures_retain_only_a_prior_tool_code_not_the_wrapper_code() {
    for classification in [
        Classification::StdoutCapExceeded,
        Classification::StderrCapExceeded,
        Classification::StdoutStderrCapExceeded,
        Classification::OutputCollectorFailed,
        Classification::OutputDrainTimeout,
    ] {
        let mut outcome = natural_outcome(0);
        outcome.classification = Some(classification);
        apply_inner_status(&mut outcome, &(7_i32 << 8).to_ne_bytes())
            .expect("prior Tool status is accepted");
        assert_eq!(outcome.status.and_then(|status| status.code()), Some(7));
        assert_eq!(outcome.classification, Some(classification));
        assert_eq!(tool_result(&outcome).exit_code, Some(7));

        let mut cleanup_induced = natural_outcome(0);
        cleanup_induced.status = None;
        cleanup_induced.classification = Some(classification);
        apply_inner_status(&mut cleanup_induced, &15_i32.to_ne_bytes())
            .expect("root-reaping evidence does not invent a prior exit code");
        assert!(cleanup_induced.status.is_none());
        assert_eq!(tool_result(&cleanup_induced).exit_code, None);
        assert_eq!(cleanup_induced.classification, Some(classification));
    }
}

#[test]
fn cancellation_and_timeout_require_root_evidence_without_claiming_an_exit_code() {
    for classification in [Classification::Cancelled, Classification::ToolTimedOut] {
        let mut outcome = natural_outcome(0);
        outcome.classification = Some(classification);
        assert!(apply_inner_status(&mut outcome, &[]).is_err());
        apply_inner_status(&mut outcome, &15_i32.to_ne_bytes()).expect("root reaping is proven");
        assert_eq!(outcome.classification, Some(classification));
        assert_eq!(tool_result(&outcome).exit_code, None);
    }
}

#[test]
fn collector_failure_replaces_an_established_terminal_cause() {
    for established in [
        PrimaryTrigger::Cancelled,
        PrimaryTrigger::TimedOut,
        PrimaryTrigger::StdoutCap,
        PrimaryTrigger::StderrCap,
        PrimaryTrigger::Exit,
    ] {
        let mut primary = Some(established);
        assert!(!select_primary(
            &mut primary,
            PrimaryTrigger::CollectorFailed
        ));
        assert_eq!(primary, Some(PrimaryTrigger::CollectorFailed));
    }
}

#[test]
fn only_an_exit_observed_before_cleanup_is_reportable() {
    let natural = ExitStatus::from_raw(7_i32 << 8);
    let cleanup_induced = ExitStatus::from_raw(15);
    assert_eq!(
        reportable_status(natural, false).and_then(|status| status.code()),
        Some(7)
    );
    assert!(reportable_status(cleanup_induced, true).is_none());
}

#[test]
fn forced_cleanup_reaps_the_direct_child() {
    let mut child = Command::new("/bin/sh")
        .args(["-c", "while :; do :; done"])
        .spawn()
        .expect("test child launches");
    let status = terminate_and_reap(&mut child);
    assert!(!status.success(), "forced cleanup observes a killed child");
    assert!(
        child
            .try_wait()
            .expect("reaped child remains observable")
            .is_some(),
        "forced cleanup must not leave an unreaped direct child"
    );
}

#[test]
fn undeclared_descriptors_are_close_on_exec_before_native_launch() {
    if crate::tests::run_isolated("WATERSHED_EXECUTOR_CLOEXEC_CHILD") {
        return;
    }
    let ambient = protection::retain_descriptor(File::open("/dev/null").unwrap())
        .expect("ambient descriptor is retained above the protected slots");
    let declared = protection::retain_descriptor(File::open("/dev/null").unwrap())
        .expect("declared descriptor is retained above the protected slots");
    rustix::io::fcntl_setfd(&ambient, rustix::io::FdFlags::empty())
        .expect("ambient fixture is inheritable");
    protection::inherit_only(&[declared.as_raw_fd()]).expect("inherited descriptors are isolated");
    assert!(
        rustix::io::fcntl_getfd(&ambient)
            .unwrap()
            .contains(rustix::io::FdFlags::CLOEXEC)
    );
    assert!(
        !rustix::io::fcntl_getfd(&declared)
            .unwrap()
            .contains(rustix::io::FdFlags::CLOEXEC)
    );

    #[cfg(target_os = "linux")]
    let directory = "/proc/self/fd";
    #[cfg(target_os = "macos")]
    let directory = "/dev/fd";
    let status = Command::new("/bin/sh")
        .args([
            "-c",
            &format!(
                "test ! -e {directory}/{} && test -e {directory}/{}",
                ambient.as_raw_fd(),
                declared.as_raw_fd(),
            ),
        ])
        .status()
        .expect("descriptor-check child launches");
    assert!(status.success(), "only declared descriptors survive exec");

    protection::inherit_only(&[])
        .expect("the Tool inherits no internal status or protection handles");
    assert!(
        rustix::io::fcntl_getfd(&declared)
            .unwrap()
            .contains(rustix::io::FdFlags::CLOEXEC)
    );
}

#[test]
fn bounded_output_classifies_each_stream_and_preserves_its_exact_prefix() {
    if crate::tests::run_isolated("WATERSHED_EXECUTOR_STREAM_BOUNDS_CHILD") {
        return;
    }
    for (script, classification, stdout, stderr) in [
        (
            "printf abcd; printf wxyz >&2",
            None,
            b"abcd".as_slice(),
            b"wxyz".as_slice(),
        ),
        (
            "printf abcde; while :; do :; done",
            Some(Classification::StdoutCapExceeded),
            b"abcd".as_slice(),
            b"".as_slice(),
        ),
        (
            "printf vwxyz >&2; while :; do :; done",
            Some(Classification::StderrCapExceeded),
            b"".as_slice(),
            b"vwxy".as_slice(),
        ),
        (
            "trap '' TERM; printf abcde; printf vwxyz >&2; while :; do :; done",
            Some(Classification::StdoutStderrCapExceeded),
            b"abcd".as_slice(),
            b"vwxy".as_slice(),
        ),
    ] {
        let outcome = run_bounded(shell(script), 2_000, 4, 4, Vec::new(), Vec::new())
            .expect("bounded direct-child supervision completes");
        assert_eq!(outcome.classification, classification, "{script}");
        assert_eq!(outcome.stdout, stdout, "{script}");
        assert_eq!(outcome.stderr, stderr, "{script}");
        let result = tool_result(&outcome);
        assert_eq!(result.classification, classification);
        assert_eq!(
            proto::decode_executor_stream_v0(&result.stdout_base64).unwrap(),
            stdout
        );
        assert_eq!(
            proto::decode_executor_stream_v0(&result.stderr_base64).unwrap(),
            stderr
        );
    }
}

#[test]
fn timeout_reaps_a_noncooperative_root_without_reporting_a_cleanup_exit_code() {
    if crate::tests::run_isolated("WATERSHED_EXECUTOR_ROOT_TIMEOUT_CHILD") {
        return;
    }
    let started = Instant::now();
    let outcome = run_bounded(
        shell("trap '' TERM; while :; do :; done"),
        100,
        4,
        4,
        Vec::new(),
        Vec::new(),
    )
    .expect("bounded cleanup reaps the direct child");
    assert_eq!(outcome.classification, Some(Classification::ToolTimedOut));
    assert_eq!(tool_result(&outcome).exit_code, None);
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "root supervision must remain bounded"
    );
}

#[test]
fn readiness_failure_retains_exit_code_and_bounded_diagnostic() {
    if crate::tests::run_isolated("WATERSHED_EXECUTOR_READINESS_DIAGNOSTIC_CHILD") {
        return;
    }
    let error = checked_output(shell("printf fixture-diagnostic >&2; exit 7"))
        .expect_err("readiness command failure is rejected");
    assert_eq!(error.code, proto::ExecutorErrorCodeV0::Unavailable);
    assert!(error.message.contains("exit code 7"), "{}", error.message);
    assert!(
        error.message.contains("fixture-diagnostic"),
        "{}",
        error.message
    );
    assert!(error.message.len() <= 1024 + 96);

    let started = Instant::now();
    let error = checked_output(shell("while :; do printf xxxxxxxxxxxxxxxx >&2; done"))
        .expect_err("unbounded readiness diagnostics are rejected");
    assert!(error.message.len() <= 1024 + 96);
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "diagnostic collection must remain bounded"
    );
}

fn shell(script: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", script])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}
