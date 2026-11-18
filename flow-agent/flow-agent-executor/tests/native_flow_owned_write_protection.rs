#![cfg(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64")
))]
#![allow(clippy::unwrap_used)]

#[path = "native_support/artifact.rs"]
mod artifact;

use std::process::Command;

#[test]
fn executor_artifact_selects_debug_or_explicit_override() {
    const EXPECTED: &str = "FLOW_TEST_EXPECTED_EXECUTOR_ARTIFACT";
    if let Some(expected) = std::env::var_os(EXPECTED) {
        assert_eq!(
            artifact::executor_artifact(),
            std::path::PathBuf::from(expected)
        );
        return;
    }
    for explicit in [None, Some("installed release executor")] {
        let expected = explicit.unwrap_or(env!("CARGO_BIN_EXE_flow-executor"));
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "executor_artifact_selects_debug_or_explicit_override",
                "--nocapture",
            ])
            .env(EXPECTED, expected)
            .env_remove("FLOW_EXECUTOR_UNDER_TEST");
        if let Some(path) = explicit {
            child.env("FLOW_EXECUTOR_UNDER_TEST", path);
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "selector case {explicit:?}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
