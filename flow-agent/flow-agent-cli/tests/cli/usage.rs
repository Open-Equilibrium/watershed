use super::{
    flow_command,
    test_support::{session_home_path, workspace_copy},
};
use std::{io::Write, process::Stdio};

#[test]
fn version_flags_print_package_version() {
    for flag in ["--version", "-V"] {
        let output = flow_command()
            .arg(flag)
            .output()
            .expect("flow binary should run");

        assert!(output.status.success(), "{flag}");
        assert_eq!(
            String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
            format!("flow {}\n", env!("CARGO_PKG_VERSION")),
            "{flag}"
        );
        assert!(output.stderr.is_empty(), "{flag}");
    }
}

#[test]
fn help_flags_print_usage() {
    for args in [
        vec!["--help"],
        vec!["-h"],
        vec!["run", "--help"],
        vec!["tail", "-h"],
    ] {
        let output = flow_command()
            .args(&args)
            .output()
            .expect("flow binary should run");

        assert!(output.status.success(), "{args:?}");
        assert!(
            String::from_utf8(output.stdout)
                .expect("stdout should be UTF-8")
                .starts_with("Usage:\n  flow run <flow>"),
            "{args:?}"
        );
        assert!(output.stderr.is_empty(), "{args:?}");
    }
}

#[test]
fn no_arguments_print_readable_global_usage_error() {
    assert_global_usage_error(&[]);
}

#[test]
fn unknown_commands_print_readable_global_usage_error() {
    assert_global_usage_error(&["unknown"]);
}

fn assert_global_usage_error(args: &[&str]) {
    let help = flow_command()
        .arg("--help")
        .output()
        .expect("flow help should run");
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
    let usage = String::from_utf8(help.stdout).expect("help should be UTF-8");
    assert_eq!(usage.lines().count(), 19);

    let output = flow_command()
        .args(args)
        .output()
        .expect("flow binary should run");

    assert_eq!(output.status.code(), Some(64));
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr should be UTF-8"),
        format!("error: {usage}")
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn invalid_command_arguments_print_usage_errors() {
    let workspace = workspace_copy("smoke-flow");
    for (args, expected) in [
        (vec!["run", "--emit"], "unknown argument"),
        (
            vec!["run", "smoke-flow", "--emit", "human"],
            "unsupported emit mode",
        ),
        (
            vec!["run", "smoke-flow", "--emit"],
            "missing value for --emit",
        ),
        (vec!["run", "smoke-flow", "--bogus"], "unknown argument"),
        (vec!["sessions", "--bogus"], "usage: flow sessions"),
        (vec!["replay"], "usage: flow replay"),
    ] {
        let output = flow_command()
            .current_dir(&workspace)
            .args(args)
            .output()
            .expect("flow binary should run");

        assert_eq!(output.status.code(), Some(64));
        let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
        assert!(stderr.contains(expected), "{stderr}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn registry_diagnostics_escape_terminal_controls() {
    let workspace = workspace_copy("smoke-flow");
    let hostile_flow_ref = "missing\u{1b}]0;owned\u{7}\n";

    let output = flow_command()
        .current_dir(&workspace)
        .args(["run", hostile_flow_ref])
        .output()
        .expect("flow binary should run");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert_eq!(output.status.code(), Some(65));
    assert!(output.stdout.is_empty());
    assert!(!stderr.contains('\u{1b}'), "{stderr:?}");
    assert!(!stderr.contains('\u{7}'), "{stderr:?}");
    assert!(
        stderr.contains("missing\\u{1b}]0;owned\\u{7}\\n"),
        "{stderr:?}"
    );
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
}

#[test]
fn usage_diagnostics_escape_terminal_controls() {
    let workspace = workspace_copy("smoke-flow");
    std::fs::write(
        session_home_path().join("config.yaml"),
        "\"hostile\\n\\u001b]0;owned\\u0007\": true\n",
    )
    .expect("synthetic invalid config writes");

    let output = flow_command()
        .current_dir(&workspace)
        .arg("validate")
        .output()
        .expect("flow binary should run");

    assert_eq!(output.status.code(), Some(64));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr should be UTF-8"),
        "error: FLOW_AGENT_HOME/config.yaml: unknown field: hostile\\n\\u{1b}]0;owned\\u{7}\n"
    );
}

#[test]
fn invalid_tail_arguments_print_usage_errors() {
    let workspace = workspace_copy("smoke-flow");
    for (args, expected) in [
        (
            vec!["tail", "smoke001", "smoke001", "--emit", "human"],
            "unsupported emit mode",
        ),
        (
            vec!["tail", "smoke001", "smoke001", "--emit"],
            "missing value for --emit",
        ),
        (
            vec!["tail", "smoke001", "smoke001", "--timeout-ms"],
            "missing value for --timeout-ms",
        ),
        (
            vec!["tail", "smoke001", "smoke001", "--timeout-ms", "slow"],
            "invalid --timeout-ms value",
        ),
        (
            vec!["tail", "smoke001", "smoke001", "--bogus"],
            "unknown argument",
        ),
    ] {
        let output = flow_command()
            .current_dir(&workspace)
            .args(args)
            .output()
            .expect("flow binary should run");

        assert_eq!(output.status.code(), Some(64), "{expected}");
        assert!(
            String::from_utf8(output.stderr)
                .expect("stderr should be UTF-8")
                .contains(expected),
            "{expected}"
        );
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn closed_stderr_pipe_preserves_the_usage_exit_code() {
    let workspace = workspace_copy("smoke-flow");
    let mut child = flow_command()
        .current_dir(&workspace)
        .arg("chat")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("flow binary should spawn");
    drop(child.stderr.take().expect("stderr is piped"));
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"/\n")
        .expect("chat input writes");

    assert_eq!(
        child.wait().expect("flow binary should exit").code(),
        Some(64)
    );
}
