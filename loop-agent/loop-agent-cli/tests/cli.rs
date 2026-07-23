use std::{
    ffi::OsString,
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

#[path = "../../tests/support.rs"]
mod test_support;
use test_support::{expected_stream, workspace_copy};

fn loop_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_loop"))
}

fn run_chat(workspace: &Path, input: &[u8]) -> Output {
    let mut child = loop_command()
        .current_dir(workspace)
        .arg("chat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("loop binary should spawn");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(input)
        .expect("stdin write");
    child.wait_with_output().expect("loop binary should exit")
}

fn wait_with_output_before(mut child: Child, timeout: Duration) -> Output {
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("child status should be readable") {
            break status;
        }
        if started.elapsed() >= timeout {
            child.kill().expect("timed-out child should stop");
            child.wait().expect("timed-out child should be reaped");
            panic!("child did not exit within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .expect("stdout is piped")
        .read_to_end(&mut stdout)
        .expect("stdout should be readable");
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .expect("stderr is piped")
        .read_to_end(&mut stderr)
        .expect("stderr should be readable");
    Output {
        status,
        stdout,
        stderr,
    }
}

fn replace_seeded_session_with_prefix(workspace: &Path, session_id: &str, prefix: &str) {
    fs::write(
        workspace
            .join(".loop/sessions")
            .join(format!("{session_id}.jsonl")),
        prefix,
    )
    .expect("partial session log written");
    let context_path = workspace
        .join(".loop/logs")
        .join(format!("{session_id}.contexts.jsonl"));
    let manifests = fs::read_to_string(&context_path).expect("context manifests readable");
    let completed_turns = prefix
        .lines()
        .filter(|line| line.contains("\"event_type\":\"message.completed\""))
        .count();
    let mut manifest_prefix = manifests
        .lines()
        .take(completed_turns)
        .collect::<Vec<_>>()
        .join("\n");
    if !manifest_prefix.is_empty() {
        manifest_prefix.push('\n');
    }
    fs::write(context_path, manifest_prefix).expect("context manifest prefix written");
}

#[test]
fn version_flags_print_package_version() {
    for flag in ["--version", "-V"] {
        let output = loop_command()
            .arg(flag)
            .output()
            .expect("loop binary should run");

        assert!(output.status.success(), "{flag}");
        assert_eq!(
            String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
            format!("loop {}\n", env!("CARGO_PKG_VERSION")),
            "{flag}"
        );
        assert!(output.stderr.is_empty(), "{flag}");
    }
}

#[test]
fn help_flags_print_usage() {
    for flag in ["--help", "-h"] {
        let output = loop_command()
            .arg(flag)
            .output()
            .expect("loop binary should run");

        assert!(output.status.success(), "{flag}");
        assert!(
            String::from_utf8(output.stdout)
                .expect("stdout should be UTF-8")
                .starts_with("usage: loop run <loop>"),
            "{flag}"
        );
        assert!(output.stderr.is_empty(), "{flag}");
    }
}

#[test]
fn no_arguments_and_unknown_commands_print_usage_errors() {
    for args in [Vec::<&str>::new(), vec!["unknown"]] {
        let output = loop_command()
            .args(args)
            .output()
            .expect("loop binary should run");

        assert_eq!(output.status.code(), Some(64));
        assert!(
            String::from_utf8(output.stderr)
                .expect("stderr should be UTF-8")
                .contains("usage: loop run <loop>")
        );
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn invalid_command_arguments_print_usage_errors() {
    let workspace = workspace_copy("smoke-loop");
    for (args, expected) in [
        (
            vec!["run", "smoke-loop", "--emit", "human"],
            "unsupported emit mode",
        ),
        (
            vec!["run", "smoke-loop", "--emit"],
            "missing value for --emit",
        ),
        (vec!["run", "smoke-loop", "--bogus"], "unknown argument"),
        (vec!["sessions", "--bogus"], "unknown argument"),
        (vec!["replay"], "missing session_id"),
    ] {
        let output = loop_command()
            .current_dir(&workspace)
            .args(args)
            .output()
            .expect("loop binary should run");

        assert_eq!(output.status.code(), Some(64));
        let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
        assert!(stderr.contains(expected), "{stderr}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn registry_diagnostics_escape_terminal_controls() {
    let workspace = workspace_copy("smoke-loop");
    let hostile_loop_ref = "missing\u{1b}]0;owned\u{7}";

    let output = loop_command()
        .current_dir(&workspace)
        .args(["run", hostile_loop_ref])
        .output()
        .expect("loop binary should run");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert_eq!(output.status.code(), Some(65));
    assert!(output.stdout.is_empty());
    assert!(!stderr.contains('\u{1b}'), "{stderr:?}");
    assert!(!stderr.contains('\u{7}'), "{stderr:?}");
    assert!(
        stderr.contains("missing\\u{1b}]0;owned\\u{7}"),
        "{stderr:?}"
    );
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
}

#[test]
fn invalid_tail_arguments_print_usage_errors() {
    let workspace = workspace_copy("smoke-loop");
    for (args, expected) in [
        (
            vec!["tail", "smoke001", "--emit", "human"],
            "unsupported emit mode",
        ),
        (
            vec!["tail", "smoke001", "--emit"],
            "missing value for --emit",
        ),
        (
            vec!["tail", "smoke001", "--timeout-ms"],
            "missing value for --timeout-ms",
        ),
        (
            vec!["tail", "smoke001", "--timeout-ms", "slow"],
            "invalid --timeout-ms value",
        ),
        (vec!["tail", "smoke001", "--bogus"], "unknown argument"),
    ] {
        let output = loop_command()
            .current_dir(&workspace)
            .args(args)
            .output()
            .expect("loop binary should run");

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
fn tail_timeout_exits_after_current_non_terminal_prefix() {
    let fixture = workspace_copy("smoke-loop");
    let session_dir = fixture.join(".loop/sessions");
    fs::create_dir_all(&session_dir).expect("session dir created");
    let prefix = expected_stream("smoke-loop", "smoke-loop.jsonl")
        .lines()
        .next()
        .expect("golden has session.started")
        .to_owned()
        + "\n";
    let session_path = session_dir.join("smoke-loop.jsonl");
    let lock_path = session_dir.join("smoke-loop.lock");
    fs::write(&session_path, &prefix).expect("partial session written");
    fs::write(&lock_path, "").expect("active-session lock written");

    let child = loop_command()
        .current_dir(&fixture)
        .args([
            "tail",
            "smoke-loop",
            "--emit",
            "jsonl",
            "--timeout-ms",
            "25",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("loop binary should spawn");
    let output = wait_with_output_before(child, Duration::from_secs(2));

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        prefix
    );
    assert_eq!(
        fs::read_to_string(session_path).expect("partial session remains readable"),
        prefix
    );
    assert!(lock_path.is_file(), "tail must not own the session lock");
}

#[test]
fn run_loop_emits_golden_jsonl_and_persists_session_log() {
    let workspace = workspace_copy("smoke-loop");
    let output = loop_command()
        .current_dir(&workspace)
        .args(["run", "smoke-loop", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let expected = expected_stream("smoke-loop", "smoke-loop.jsonl");
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        expected
    );
    assert_eq!(
        fs::read_to_string(workspace.join(".loop/sessions/smoke-loop.jsonl"))
            .expect("session log is written"),
        expected
    );
    assert!(
        workspace.join(".loop/logs/smoke-loop.log").is_file(),
        "structured run log should be written"
    );
}

#[test]
fn closed_stdout_pipe_does_not_panic_for_jsonl_run() {
    let workspace = workspace_copy("smoke-loop");
    let mut child = loop_command()
        .current_dir(&workspace)
        .args(["run", "smoke-loop", "--emit", "jsonl"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("loop binary should spawn");

    drop(child.stdout.take().expect("stdout is piped"));

    let output = child.wait_with_output().expect("loop binary should exit");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn closed_stderr_pipe_preserves_the_usage_exit_code() {
    let workspace = workspace_copy("smoke-loop");
    let mut child = loop_command()
        .current_dir(&workspace)
        .arg("chat")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("loop binary should spawn");
    drop(child.stderr.take().expect("stderr is piped"));
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"unsupported\n")
        .expect("chat input writes");

    assert_eq!(
        child.wait().expect("loop binary should exit").code(),
        Some(64)
    );
}

#[test]
fn replay_tail_and_sessions_read_persisted_event_log() {
    let fixture = workspace_copy("smoke-loop");
    let run = loop_command()
        .current_dir(&fixture)
        .args(["run", "smoke-loop", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");
    assert!(run.status.success());

    for command in ["replay", "tail"] {
        let output = loop_command()
            .current_dir(&fixture)
            .args([command, "smoke-loop", "--emit", "jsonl"])
            .output()
            .expect("loop binary should run");

        assert!(output.status.success(), "{command}");
        assert!(output.stderr.is_empty(), "{command}");
        assert_eq!(
            String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
            expected_stream("smoke-loop", "smoke-loop.jsonl"),
            "{command}"
        );
    }

    let sessions = loop_command()
        .current_dir(&fixture)
        .arg("sessions")
        .output()
        .expect("loop binary should run");

    assert!(sessions.status.success());
    assert_eq!(
        String::from_utf8(sessions.stdout).expect("stdout should be UTF-8"),
        "smoke-loop\n"
    );
}

#[test]
fn tail_no_follow_exits_after_current_non_terminal_prefix() {
    let fixture = workspace_copy("smoke-loop");
    let session_dir = fixture.join(".loop/sessions");
    fs::create_dir_all(&session_dir).expect("session dir created");
    let prefix = expected_stream("smoke-loop", "smoke-loop.jsonl")
        .lines()
        .next()
        .expect("golden has session.started")
        .to_owned()
        + "\n";
    let lock_path = session_dir.join("smoke-loop.lock");
    fs::write(session_dir.join("smoke-loop.jsonl"), &prefix).expect("partial session written");
    fs::write(&lock_path, "").expect("active-session lock written");

    let child = loop_command()
        .current_dir(&fixture)
        .args(["tail", "smoke-loop", "--emit", "jsonl", "--no-follow"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("loop binary should spawn");
    let output = wait_with_output_before(child, Duration::from_secs(2));

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        prefix
    );
    assert!(lock_path.is_file(), "tail must not own the session lock");
}

#[test]
fn tail_follows_a_growing_session_through_its_terminal_event() {
    let fixture = workspace_copy("smoke-loop");
    let session_dir = fixture.join(".loop/sessions");
    fs::create_dir_all(&session_dir).expect("session dir created");
    let expected = expected_stream("smoke-loop", "smoke-loop.jsonl");
    let split = expected.find('\n').expect("golden has a first event") + 1;
    let session_path = session_dir.join("smoke-loop.jsonl");
    let lock_path = session_dir.join("smoke-loop.lock");
    fs::write(&session_path, &expected[..split]).expect("initial prefix written");
    fs::write(&lock_path, "").expect("active-session lock written");
    let mut child = loop_command()
        .current_dir(&fixture)
        .args([
            "tail",
            "smoke-loop",
            "--emit",
            "jsonl",
            "--timeout-ms",
            "5000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("loop binary should spawn");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let mut actual = String::new();
    stdout
        .read_line(&mut actual)
        .expect("tail emits the committed prefix");

    let mut session = fs::OpenOptions::new()
        .append(true)
        .open(&session_path)
        .expect("session opens for append");
    session
        .write_all(&expected.as_bytes()[split..])
        .expect("remaining events appended");
    session.flush().expect("remaining events flushed");
    drop(session);
    fs::remove_file(lock_path).expect("active-session lock removed");
    stdout
        .read_to_string(&mut actual)
        .expect("tail emits appended events");
    let status = child.wait().expect("loop binary should exit");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr is piped")
        .read_to_string(&mut stderr)
        .expect("stderr reads");

    assert!(status.success(), "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(actual, expected);
}

#[test]
fn closed_stdout_pipe_does_not_fail_for_jsonl_tail() {
    let workspace = workspace_copy("smoke-loop");
    let run = loop_command()
        .current_dir(&workspace)
        .args(["run", "smoke-loop", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");
    assert!(run.status.success());

    let mut child = loop_command()
        .current_dir(&workspace)
        .args(["tail", "smoke-loop", "--emit", "jsonl"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("loop binary should spawn");

    drop(child.stdout.take().expect("stdout is piped"));

    let output = child.wait_with_output().expect("loop binary should exit");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(output.status.success(), "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn resume_rejects_terminal_sessions_without_rewriting_log() {
    let fixture = workspace_copy("smoke-loop");
    let run = loop_command()
        .current_dir(&fixture)
        .args(["run", "smoke-loop", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");
    assert!(run.status.success());
    let before = fs::read_to_string(fixture.join(".loop/sessions/smoke-loop.jsonl"))
        .expect("session log exists");

    let output = loop_command()
        .current_dir(&fixture)
        .args(["resume", "smoke-loop", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");

    assert_eq!(output.status.code(), Some(65));
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr should be UTF-8")
            .contains("terminal session")
    );
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(fixture.join(".loop/sessions/smoke-loop.jsonl"))
            .expect("session log exists"),
        before
    );
}

#[test]
fn resume_partial_session_prints_human_status() {
    let workspace = workspace_copy("smoke-loop");
    let seed = loop_command()
        .current_dir(&workspace)
        .args(["run", "smoke-loop", "--emit", "jsonl"])
        .output()
        .expect("loop binary should seed metadata");
    assert!(seed.status.success());
    assert!(seed.stderr.is_empty());

    let session_dir = workspace.join(".loop/sessions");
    fs::create_dir_all(&session_dir).expect("session dir created");
    let prefix = expected_stream("smoke-loop", "smoke-loop.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    replace_seeded_session_with_prefix(&workspace, "smoke-loop", &prefix);

    let output = loop_command()
        .current_dir(&workspace)
        .args(["resume", "smoke-loop"])
        .output()
        .expect("loop binary should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        "session smoke-loop resumed\n"
    );
    assert!(
        fs::read_to_string(session_dir.join("smoke-loop.jsonl"))
            .expect("resumed log readable")
            .contains("\"event_type\":\"session.completed\"")
    );
}

#[test]
fn failed_jsonl_resume_exits_with_failed_status() {
    let workspace = workspace_copy("sandbox-negative");
    let seed = loop_command()
        .current_dir(&workspace)
        .args(["run", "sandbox-negative-write", "--emit", "jsonl"])
        .output()
        .expect("loop binary should seed metadata");
    assert_eq!(seed.status.code(), Some(65));
    assert!(seed.stderr.is_empty());

    let session_dir = workspace.join(".loop/sessions");
    fs::create_dir_all(&session_dir).expect("session dir created");
    let prefix = expected_stream("sandbox-negative", "sandbox-negative-write.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    replace_seeded_session_with_prefix(&workspace, "sandbox-negative-write", &prefix);

    let output = loop_command()
        .current_dir(&workspace)
        .args(["resume", "sandbox-negative-write", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");

    assert_eq!(output.status.code(), Some(65));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("\"event_type\":\"session.resumed\""));
    assert!(stdout.contains("\"event_type\":\"session.failed\""));
    assert!(!workspace.join("out/forbidden.txt").exists());
}

#[test]
fn unsafe_session_id_is_rejected_before_filesystem_access() {
    let workspace = workspace_copy("smoke-loop");
    let output = loop_command()
        .current_dir(&workspace)
        .args(["replay", "../smoke001", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");

    assert_eq!(output.status.code(), Some(64));
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr should be UTF-8")
            .contains("invalid session_id")
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn chat_hello_command_runs_hello_loop() {
    let workspace = workspace_copy("hello-loop");
    let output = run_chat(&workspace, b"/hello-loop\n");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        expected_stream("hello-loop", "hello-loop.jsonl")
    );
}

#[test]
fn chat_failed_loop_exits_with_failed_status() {
    let workspace = workspace_copy("sandbox-negative");
    fs::write(
        workspace.join("registry/loops/hello-loop.yaml"),
        "loop:\n  id: hello-loop\n  name: HelloLoop\n  phase_refs: [negative-write]\n  subloop_refs: []\n  connection_refs: []\n",
    )
    .expect("chat loop fixture written");
    let output = run_chat(&workspace, b"/hello-loop\n");

    assert_eq!(output.status.code(), Some(65));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("\"loop_definition_id\":\"hello-loop\""));
    assert!(stdout.contains("\"event_type\":\"session.failed\""));
    assert!(!workspace.join("out/forbidden.txt").exists());
}

#[test]
fn chat_ignores_blank_input_until_eof() {
    let workspace = workspace_copy("hello-loop");
    let output = run_chat(&workspace, b"\n   \n");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn chat_rejects_unsupported_commands() {
    let workspace = workspace_copy("hello-loop");
    let output = run_chat(&workspace, b"/unknown\n");

    assert_eq!(output.status.code(), Some(64));
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr should be UTF-8")
            .contains("unsupported chat command")
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn sandbox_negative_cli_fails_without_side_effects() {
    let workspace = workspace_copy("sandbox-negative");
    let output = loop_command()
        .current_dir(&workspace)
        .args(["run", "sandbox-negative-write", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");

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
    let run_output = loop_command()
        .current_dir(&workspace)
        .args(["run", "sandbox-negative-write", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");
    assert_eq!(run_output.status.code(), Some(65));
    assert!(run_output.stderr.is_empty());

    for command in ["replay", "tail"] {
        let output = loop_command()
            .current_dir(&workspace)
            .args([command, "sandbox-negative-write", "--emit", "jsonl"])
            .output()
            .expect("loop binary should run");

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
    let output = loop_command()
        .current_dir(&workspace)
        .args(["run", "sandbox-negative-write"])
        .output()
        .expect("loop binary should run");

    assert_eq!(output.status.code(), Some(65));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert_eq!(
        stdout,
        "loop sandbox-negative-write (session sandbox-negative-write) failed (write_denied): write outside declared roots denied\n"
    );
    assert!(!stdout.contains("completed"));
    assert!(
        !workspace.join("out/forbidden.txt").exists(),
        "failed human run must not create side effects after rejection"
    );

    for (command, expected) in [
        (
            "replay",
            "session sandbox-negative-write replayed: failed (write_denied): write outside declared roots denied\n",
        ),
        (
            "tail",
            "session sandbox-negative-write tailed: failed (write_denied): write outside declared roots denied\n",
        ),
    ] {
        let output = loop_command()
            .current_dir(&workspace)
            .args([command, "sandbox-negative-write"])
            .output()
            .expect("loop binary should run");

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
    let output = loop_command()
        .arg(non_unicode_argument())
        .output()
        .expect("loop binary should run");

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
