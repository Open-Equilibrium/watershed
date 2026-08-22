use super::{
    flow_command,
    test_support::{expected_stream, workspace_copy},
};
use std::ffi::OsString;

#[test]
fn sandbox_negative_cli_fails_without_side_effects() {
    let workspace = workspace_copy("sandbox-negative");
    let output = flow_command()
        .current_dir(&workspace)
        .args(["run", "sandbox-negative-write", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");

    assert_eq!(output.status.code(), Some(65));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        expected_stream("sandbox-negative", "sandbox-negative-write.jsonl")
    );
    assert!(!workspace.join("out/forbidden.txt").exists());
}

#[test]
fn replay_and_tail_failed_sessions_exit_with_failed_status() {
    let workspace = workspace_copy("sandbox-negative");
    let run_output = flow_command()
        .current_dir(&workspace)
        .args(["run", "sandbox-negative-write", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");
    assert_eq!(run_output.status.code(), Some(65));
    assert!(run_output.stderr.is_empty());

    for command in ["replay", "tail"] {
        let output = flow_command()
            .current_dir(&workspace)
            .args([
                command,
                "sandbox-negative-write",
                "sandbox-negative-write",
                "--emit",
                "jsonl",
            ])
            .output()
            .expect("flow binary should run");

        assert_eq!(output.status.code(), Some(65), "{command}");
        assert!(output.stderr.is_empty(), "{command}");
        assert!(
            String::from_utf8(output.stdout)
                .expect("stdout should be UTF-8")
                .contains("\"event_type\":\"session.failed\"")
        );
    }
}

#[test]
fn failed_human_commands_report_the_terminal_reason() {
    let workspace = workspace_copy("sandbox-negative");
    let output = flow_command()
        .current_dir(&workspace)
        .args(["run", "sandbox-negative-write"])
        .output()
        .expect("flow binary should run");

    assert_eq!(output.status.code(), Some(65));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert_eq!(
        stdout,
        "flow sandbox-negative-write (session sandbox-negative-write) failed (write_denied): write outside declared roots denied\n"
    );
    assert!(!stdout.contains("completed"));
    assert!(
        !workspace.join("out/forbidden.txt").exists(),
        "failed human run must not create side effects after rejection"
    );

    for (command, expected) in [
        (
            "replay",
            "run sandbox-negative-write replayed: failed (write_denied): write outside declared roots denied\n",
        ),
        (
            "tail",
            "run sandbox-negative-write tailed: failed (write_denied): write outside declared roots denied\n",
        ),
    ] {
        let output = flow_command()
            .current_dir(&workspace)
            .args([command, "sandbox-negative-write", "sandbox-negative-write"])
            .output()
            .expect("flow binary should run");

        assert_eq!(output.status.code(), Some(65), "{command}");
        assert!(output.stderr.is_empty(), "{command}");
        assert_eq!(
            String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
            expected,
            "{command}"
        );
    }
}

#[test]
fn non_unicode_argument_exits_with_usage_error() {
    let output = flow_command()
        .arg(non_unicode_argument())
        .output()
        .expect("flow binary should run");

    assert_eq!(output.status.code(), Some(64));
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr should be UTF-8")
            .contains("arguments must be valid UTF-8")
    );
    assert!(output.stdout.is_empty());
}

#[cfg(unix)]
fn non_unicode_argument() -> OsString {
    use std::os::unix::ffi::OsStringExt;

    OsString::from_vec(vec![0xff])
}

#[cfg(windows)]
fn non_unicode_argument() -> OsString {
    use std::os::windows::ffi::OsStringExt;

    OsString::from_wide(&[0xd800])
}
