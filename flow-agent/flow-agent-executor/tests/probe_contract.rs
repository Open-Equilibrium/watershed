use proto::{
    EXECUTOR_FEATURE_SELF_PROTECTION_V0, EXECUTOR_NAME_V0, EXECUTOR_PROTOCOL_VERSION_V0,
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
    #[cfg(target_os = "linux")]
    let identity = ("bubblewrap-seccomp", "ubuntu-24.04-x86_64");
    #[cfg(target_os = "macos")]
    let identity = ("seatbelt", "macos-26-aarch64");
    assert_eq!(probe.backend, identity.0);
    assert_eq!(probe.platform, identity.1);
    assert_eq!(
        output.stdout,
        proto::canonical_executor_probe_v0(&probe).expect("probe canonicalizes")
    );
    assert_eq!(output.stderr.is_empty(), probe.ready);
    assert_eq!(
        probe.supported_policy_features,
        if probe.ready {
            vec![EXECUTOR_FEATURE_SELF_PROTECTION_V0.to_owned()]
        } else {
            Vec::new()
        }
    );
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

#[test]
fn sibling_inner_mode_rejects_invalid_status_descriptors() {
    for descriptor in ["not-an-integer", "-1", "0", "1", "2"] {
        let output = Command::new(env!("CARGO_BIN_EXE_flow-executor"))
            .args(["--inner", descriptor])
            .output()
            .expect("sibling Executor inner mode launches");

        assert_eq!(output.status.code(), Some(65), "{descriptor}: {output:?}");
        assert!(output.stdout.is_empty());
        let diagnostic = String::from_utf8(output.stderr).expect("stderr is UTF-8");
        assert!(
            diagnostic == "invalid inner status descriptor\n"
                || diagnostic == "invalid inherited descriptor\n",
            "{descriptor}: {diagnostic}"
        );
    }
}

#[test]
fn sibling_rejects_invalid_arguments_without_output() {
    for arguments in [
        vec!["--invalid"],
        vec!["--inner"],
        vec!["--inner", "3", "4", "5"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_flow-executor"))
            .args(&arguments)
            .output()
            .expect("sibling Executor launches");

        assert_eq!(output.status.code(), Some(65), "{arguments:?}: {output:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            "usage: flow-executor [--probe]\n"
        );
    }
}
