use flow_agent_core::validate_protocol_jsonl_text;
use proto::EventType;
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

fn assert_append_only_resume<'a>(
    prefix: &str,
    resumed: &'a str,
    expected_terminal: EventType,
) -> &'a str {
    let suffix = resumed
        .strip_prefix(prefix)
        .expect("resume must preserve the exact seeded prefix");
    let events = validate_protocol_jsonl_text(Path::new("resumed-session.jsonl"), resumed)
        .expect("resumed session history must be valid");
    let prefix_event_count = prefix.lines().count();
    let appended_events = &events[prefix_event_count..];
    assert_eq!(
        appended_events.first().map(|event| event.event_type),
        Some(EventType::SessionResumed),
        "resume marker must be the first appended event"
    );
    assert_eq!(
        appended_events.last().map(|event| event.event_type),
        Some(expected_terminal),
        "expected terminal event must end the resumed history"
    );
    assert_eq!(
        appended_events
            .iter()
            .filter(|event| event.event_type == EventType::SessionResumed)
            .count(),
        1,
        "resume suffix must contain one resume marker"
    );
    assert_eq!(
        appended_events
            .iter()
            .filter(|event| event.event_type == expected_terminal)
            .count(),
        1,
        "resume suffix must contain one expected terminal event"
    );
    for (offset, event) in appended_events.iter().enumerate() {
        assert_eq!(
            event.sequence,
            prefix_event_count as u64 + offset as u64 + 1,
            "appended resume events must continue the prefix sequence"
        );
    }
    suffix
}

#[test]
fn append_only_resume_check_rejects_a_rewritten_prefix() {
    let result = std::panic::catch_unwind(|| {
        assert_append_only_resume("original\n", "rewritten\n", EventType::SessionCompleted);
    });

    assert!(
        result.is_err(),
        "rewritten history must fail the resume check"
    );
}

fn flow_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flow"))
}

fn closed_pipe_stdout() -> Stdio {
    let mut reader = flow_command()
        .arg("--version")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("pipe reader should spawn");
    let writer = reader.stdin.take().expect("pipe writer is available");
    assert!(
        reader.wait().expect("pipe reader should exit").success(),
        "pipe reader should close its stdin"
    );
    Stdio::from(writer)
}

fn run_chat(workspace: &Path, input: &[u8]) -> Output {
    let mut child = flow_command()
        .current_dir(workspace)
        .arg("chat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("flow binary should spawn");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(input)
        .expect("stdin write");
    child.wait_with_output().expect("flow binary should exit")
}

fn wait_with_output_before(mut child: Child, timeout: Duration) -> Output {
    let stdout_reader =
        read_to_end_in_thread(child.stdout.take().expect("stdout is piped"), "stdout");
    let stderr_reader =
        read_to_end_in_thread(child.stderr.take().expect("stderr is piped"), "stderr");
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
    Output {
        status,
        stdout: stdout_reader.join().expect("stdout reader should finish"),
        stderr: stderr_reader.join().expect("stderr reader should finish"),
    }
}

fn read_to_end_in_thread(
    mut reader: impl Read + Send + 'static,
    stream: &'static str,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut output = Vec::new();
        reader
            .read_to_end(&mut output)
            .unwrap_or_else(|error| panic!("{stream} should be readable: {error}"));
        output
    })
}

#[test]
fn large_output_child() {
    if std::env::var_os("WATERSHED_FLOW_CLI_LARGE_OUTPUT_CHILD").is_none() {
        return;
    }
    let output = vec![b'x'; 512 * 1024];
    std::io::stdout()
        .write_all(&output)
        .expect("large stdout should write");
    std::io::stderr()
        .write_all(&output)
        .expect("large stderr should write");
}

#[test]
fn wait_with_output_drains_large_captured_streams() {
    let child = Command::new(std::env::current_exe().expect("test executable should resolve"))
        .args(["--exact", "large_output_child", "--nocapture"])
        .env("WATERSHED_FLOW_CLI_LARGE_OUTPUT_CHILD", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("large-output child should spawn");

    let output = wait_with_output_before(child, Duration::from_secs(2));

    assert!(output.status.success());
    assert!(output.stdout.len() >= 512 * 1024);
    assert!(output.stderr.len() >= 512 * 1024);
}

fn replace_seeded_session_with_prefix(workspace: &Path, session_id: &str, prefix: &str) {
    fs::write(
        workspace
            .join(".flow/sessions")
            .join(format!("{session_id}.jsonl")),
        prefix,
    )
    .expect("partial session log written");
    let context_path = workspace
        .join(".flow/logs")
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
    for flag in ["--help", "-h"] {
        let output = flow_command()
            .arg(flag)
            .output()
            .expect("flow binary should run");

        assert!(output.status.success(), "{flag}");
        assert!(
            String::from_utf8(output.stdout)
                .expect("stdout should be UTF-8")
                .starts_with("usage: flow run <flow>"),
            "{flag}"
        );
        assert!(output.stderr.is_empty(), "{flag}");
    }
}

#[test]
fn no_arguments_and_unknown_commands_print_usage_errors() {
    for args in [Vec::<&str>::new(), vec!["unknown"]] {
        let output = flow_command()
            .args(args)
            .output()
            .expect("flow binary should run");

        assert_eq!(output.status.code(), Some(64));
        assert!(
            String::from_utf8(output.stderr)
                .expect("stderr should be UTF-8")
                .contains("usage: flow run <flow>")
        );
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn invalid_command_arguments_print_usage_errors() {
    let workspace = workspace_copy("smoke-flow");
    for (args, expected) in [
        (
            vec!["run", "smoke-flow", "--emit", "human"],
            "unsupported emit mode",
        ),
        (
            vec!["run", "smoke-flow", "--emit"],
            "missing value for --emit",
        ),
        (vec!["run", "smoke-flow", "--bogus"], "unknown argument"),
        (vec!["sessions", "--bogus"], "unknown argument"),
        (vec!["replay"], "missing session_id"),
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
    let hostile_flow_ref = "missing\u{1b}]0;owned\u{7}";

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
        stderr.contains("missing\\u{1b}]0;owned\\u{7}"),
        "{stderr:?}"
    );
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
}

#[test]
fn invalid_tail_arguments_print_usage_errors() {
    let workspace = workspace_copy("smoke-flow");
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
fn tail_timeout_exits_after_current_non_terminal_prefix() {
    let fixture = workspace_copy("smoke-flow");
    let session_dir = fixture.join(".flow/sessions");
    fs::create_dir_all(&session_dir).expect("session dir created");
    let prefix = expected_stream("smoke-flow", "smoke-flow.jsonl")
        .lines()
        .next()
        .expect("golden has session.started")
        .to_owned()
        + "\n";
    let session_path = session_dir.join("smoke-flow.jsonl");
    fs::write(&session_path, &prefix).expect("partial session written");

    let child = flow_command()
        .current_dir(&fixture)
        .args([
            "tail",
            "smoke-flow",
            "--emit",
            "jsonl",
            "--timeout-ms",
            "25",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("flow binary should spawn");
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
}

#[test]
fn run_flow_emits_golden_jsonl_and_persists_session_log() {
    let workspace = workspace_copy("smoke-flow");
    let output = flow_command()
        .current_dir(&workspace)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let expected = expected_stream("smoke-flow", "smoke-flow.jsonl");
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        expected
    );
    assert_eq!(
        fs::read_to_string(workspace.join(".flow/sessions/smoke-flow.jsonl"))
            .expect("session log is written"),
        expected
    );
    assert!(
        workspace.join(".flow/logs/smoke-flow.log").is_file(),
        "structured run log should be written"
    );
}

#[test]
fn closed_stdout_pipe_does_not_panic_for_jsonl_run() {
    let workspace = workspace_copy("smoke-flow");
    let output = flow_command()
        .current_dir(&workspace)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .stdout(closed_pipe_stdout())
        .stderr(Stdio::piped())
        .output()
        .expect("flow binary should run");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
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
        .write_all(b"unsupported\n")
        .expect("chat input writes");

    assert_eq!(
        child.wait().expect("flow binary should exit").code(),
        Some(64)
    );
}

#[test]
fn replay_tail_and_sessions_read_persisted_event_log() {
    let fixture = workspace_copy("smoke-flow");
    let run = flow_command()
        .current_dir(&fixture)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");
    assert!(run.status.success());

    for command in ["replay", "tail"] {
        let output = flow_command()
            .current_dir(&fixture)
            .args([command, "smoke-flow", "--emit", "jsonl"])
            .output()
            .expect("flow binary should run");

        assert!(output.status.success(), "{command}");
        assert!(output.stderr.is_empty(), "{command}");
        assert_eq!(
            String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
            expected_stream("smoke-flow", "smoke-flow.jsonl"),
            "{command}"
        );
    }

    let sessions = flow_command()
        .current_dir(&fixture)
        .arg("sessions")
        .output()
        .expect("flow binary should run");

    assert!(sessions.status.success());
    assert_eq!(
        String::from_utf8(sessions.stdout).expect("stdout should be UTF-8"),
        "smoke-flow\n"
    );
}

#[test]
fn tail_no_follow_exits_after_current_non_terminal_prefix() {
    let fixture = workspace_copy("smoke-flow");
    let session_dir = fixture.join(".flow/sessions");
    fs::create_dir_all(&session_dir).expect("session dir created");
    let prefix = expected_stream("smoke-flow", "smoke-flow.jsonl")
        .lines()
        .next()
        .expect("golden has session.started")
        .to_owned()
        + "\n";
    fs::write(session_dir.join("smoke-flow.jsonl"), &prefix).expect("partial session written");

    let child = flow_command()
        .current_dir(&fixture)
        .args(["tail", "smoke-flow", "--emit", "jsonl", "--no-follow"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("flow binary should spawn");
    let output = wait_with_output_before(child, Duration::from_secs(2));

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        prefix
    );
}

#[test]
fn tail_follows_a_growing_session_through_its_terminal_event() {
    let fixture = workspace_copy("smoke-flow");
    let session_dir = fixture.join(".flow/sessions");
    fs::create_dir_all(&session_dir).expect("session dir created");
    let expected = expected_stream("smoke-flow", "smoke-flow.jsonl");
    let split = expected.find('\n').expect("golden has a first event") + 1;
    let session_path = session_dir.join("smoke-flow.jsonl");
    fs::write(&session_path, &expected[..split]).expect("initial prefix written");
    let mut child = flow_command()
        .current_dir(&fixture)
        .args([
            "tail",
            "smoke-flow",
            "--emit",
            "jsonl",
            "--timeout-ms",
            "5000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("flow binary should spawn");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let stderr_reader =
        read_to_end_in_thread(child.stderr.take().expect("stderr is piped"), "stderr");
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
    stdout
        .read_to_string(&mut actual)
        .expect("tail emits appended events");
    let status = child.wait().expect("flow binary should exit");
    let stderr = String::from_utf8(stderr_reader.join().expect("stderr reader should finish"))
        .expect("stderr should be UTF-8");

    assert!(status.success(), "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(actual, expected);
}

#[test]
fn closed_stdout_pipe_does_not_fail_for_jsonl_tail() {
    let workspace = workspace_copy("smoke-flow");
    let run = flow_command()
        .current_dir(&workspace)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");
    assert!(run.status.success());

    let output = flow_command()
        .current_dir(&workspace)
        .args(["tail", "smoke-flow", "--emit", "jsonl"])
        .stdout(closed_pipe_stdout())
        .stderr(Stdio::piped())
        .output()
        .expect("flow binary should run");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(output.status.success(), "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn resume_rejects_terminal_sessions_without_rewriting_log() {
    let fixture = workspace_copy("smoke-flow");
    let run = flow_command()
        .current_dir(&fixture)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");
    assert!(run.status.success());
    let before = fs::read_to_string(fixture.join(".flow/sessions/smoke-flow.jsonl"))
        .expect("session log exists");

    let output = flow_command()
        .current_dir(&fixture)
        .args(["resume", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");

    assert_eq!(output.status.code(), Some(65));
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr should be UTF-8")
            .contains("terminal session")
    );
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(fixture.join(".flow/sessions/smoke-flow.jsonl"))
            .expect("session log exists"),
        before
    );
}

#[test]
fn resume_partial_session_prints_human_status() {
    let workspace = workspace_copy("smoke-flow");
    let seed = flow_command()
        .current_dir(&workspace)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("flow binary should seed metadata");
    assert!(seed.status.success());
    assert!(seed.stderr.is_empty());

    let session_dir = workspace.join(".flow/sessions");
    fs::create_dir_all(&session_dir).expect("session dir created");
    let prefix = expected_stream("smoke-flow", "smoke-flow.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    replace_seeded_session_with_prefix(&workspace, "smoke-flow", &prefix);

    let output = flow_command()
        .current_dir(&workspace)
        .args(["resume", "smoke-flow"])
        .output()
        .expect("flow binary should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        "session smoke-flow resumed\n"
    );
    let resumed =
        fs::read_to_string(session_dir.join("smoke-flow.jsonl")).expect("resumed log readable");
    assert_append_only_resume(&prefix, &resumed, EventType::SessionCompleted);
}

#[test]
fn failed_jsonl_resume_exits_with_failed_status() {
    let workspace = workspace_copy("sandbox-negative");
    let seed = flow_command()
        .current_dir(&workspace)
        .args(["run", "sandbox-negative-write", "--emit", "jsonl"])
        .output()
        .expect("flow binary should seed metadata");
    assert_eq!(seed.status.code(), Some(65));
    assert!(seed.stderr.is_empty());

    let session_dir = workspace.join(".flow/sessions");
    fs::create_dir_all(&session_dir).expect("session dir created");
    let prefix = expected_stream("sandbox-negative", "sandbox-negative-write.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    replace_seeded_session_with_prefix(&workspace, "sandbox-negative-write", &prefix);

    let output = flow_command()
        .current_dir(&workspace)
        .args(["resume", "sandbox-negative-write", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");

    assert_eq!(output.status.code(), Some(65));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("\"event_type\":\"session.resumed\""));
    assert!(stdout.contains("\"event_type\":\"session.failed\""));
    let resumed = fs::read_to_string(session_dir.join("sandbox-negative-write.jsonl"))
        .expect("failed resumed log readable");
    let suffix = assert_append_only_resume(&prefix, &resumed, EventType::SessionFailed);
    assert_eq!(stdout, suffix);
    assert!(!workspace.join("out/forbidden.txt").exists());
}

#[test]
fn unsafe_session_id_is_rejected_before_filesystem_access() {
    let workspace = workspace_copy("smoke-flow");
    let output = flow_command()
        .current_dir(&workspace)
        .args(["replay", "../smoke001", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");

    assert_eq!(output.status.code(), Some(64));
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr should be UTF-8")
            .contains("invalid session_id")
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn chat_hello_command_runs_hello_flow() {
    let workspace = workspace_copy("hello-flow");
    let output = run_chat(&workspace, b"/hello-flow\n");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        expected_stream("hello-flow", "hello-flow.jsonl")
    );
}

#[test]
fn chat_failed_flow_exits_with_failed_status() {
    let workspace = workspace_copy("sandbox-negative");
    fs::write(
        workspace.join("registry/flows/hello-flow.yaml"),
        "flow:\n  id: hello-flow\n  name: HelloFlow\n  phase_refs: [negative-write]\n  subflow_refs: []\n  connection_refs: []\n",
    )
    .expect("chat flow fixture written");
    let output = run_chat(&workspace, b"/hello-flow\n");

    assert_eq!(output.status.code(), Some(65));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("\"flow_definition_id\":\"hello-flow\""));
    assert!(stdout.contains("\"event_type\":\"session.failed\""));
    assert!(!workspace.join("out/forbidden.txt").exists());
}

#[test]
fn chat_ignores_blank_input_until_eof() {
    let workspace = workspace_copy("hello-flow");
    let output = run_chat(&workspace, b"\n   \n");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn chat_rejects_unsupported_commands() {
    let workspace = workspace_copy("hello-flow");
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
            .args([command, "sandbox-negative-write", "--emit", "jsonl"])
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
            "session sandbox-negative-write replayed: failed (write_denied): write outside declared roots denied\n",
        ),
        (
            "tail",
            "session sandbox-negative-write tailed: failed (write_denied): write outside declared roots denied\n",
        ),
    ] {
        let output = flow_command()
            .current_dir(&workspace)
            .args([command, "sandbox-negative-write"])
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
