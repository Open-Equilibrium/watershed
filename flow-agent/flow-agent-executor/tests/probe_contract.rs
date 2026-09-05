use proto::{
    EXECUTOR_BACKEND_V0, EXECUTOR_NAME_V0, EXECUTOR_PLATFORM_V0, EXECUTOR_PROTOCOL_VERSION_V0,
    parse_executor_probe_v0,
};
use std::process::Command;

#[test]
fn sibling_probe_uses_the_canonical_protocol_identity() {
    let output = Command::new(env!("CARGO_BIN_EXE_flow-executor"))
        .arg("--probe")
        .output()
        .expect("sibling Executor probe launches");

    assert!(output.status.success(), "probe stderr: {:?}", output.stderr);
    let probe = parse_executor_probe_v0(&output.stdout).expect("probe is canonical");
    assert_eq!(probe.executor, EXECUTOR_NAME_V0);
    assert_eq!(probe.executor_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(probe.backend, EXECUTOR_BACKEND_V0);
    assert_eq!(probe.platform, EXECUTOR_PLATFORM_V0);
    assert_eq!(
        probe.protocol_versions,
        [EXECUTOR_PROTOCOL_VERSION_V0.to_owned()]
    );
}

#[test]
fn sibling_self_test_succeeds_without_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_flow-executor"))
        .arg("--inner-self-test")
        .output()
        .expect("sibling Executor self-test launches");

    assert!(
        output.status.success(),
        "self-test stderr: {:?}",
        output.stderr
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[test]
fn sibling_inner_mode_fails_closed_on_unsupported_platform() {
    let output = Command::new(env!("CARGO_BIN_EXE_flow-executor"))
        .args(["--inner", "3", "4", "5"])
        .output()
        .expect("sibling Executor inner mode launches");

    assert_eq!(output.status.code(), Some(65), "{output:?}");
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        "inner Executor mode is unavailable on this platform\n"
    );
}

#[test]
fn sibling_rejects_invalid_arguments_without_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_flow-executor"))
        .arg("--invalid")
        .output()
        .expect("sibling Executor launches");

    assert_eq!(output.status.code(), Some(65), "{output:?}");
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        "usage: flow-executor [--probe]\n"
    );
}
