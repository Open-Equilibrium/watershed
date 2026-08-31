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
