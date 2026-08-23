use super::{
    flow_command,
    process::{closed_pipe_stdout, read_to_end_in_thread, wait_with_output_before},
    test_support::{expected_stream, workspace_copy, workspace_session_dir},
};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

fn write_non_terminal_session_prefix(fixture: &Path) -> (PathBuf, String) {
    flow_agent_core::conversation_status(fixture, None, flow_agent_core::EmitMode::Jsonl)
        .expect("session store initializes");
    let session_dir = workspace_session_dir(fixture);
    fs::create_dir_all(&session_dir).expect("session dir created");
    let prefix = expected_stream("smoke-flow", "smoke-flow.jsonl")
        .lines()
        .next()
        .expect("golden has session.started")
        .to_owned()
        + "\n";
    let session_path = session_dir.join("smoke-flow.jsonl");
    fs::write(&session_path, &prefix).expect("partial session written");
    (session_path, prefix)
}

#[test]
fn tail_timeout_exits_after_current_non_terminal_prefix() {
    if super::test_support::run_current_test_isolated_session_home() {
        return;
    }

    let fixture = workspace_copy("smoke-flow");
    let (session_path, prefix) = write_non_terminal_session_prefix(&fixture);

    let child = flow_command()
        .current_dir(&fixture)
        .args([
            "tail",
            "smoke-flow",
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
fn tail_no_follow_exits_after_current_non_terminal_prefix() {
    if super::test_support::run_current_test_isolated_session_home() {
        return;
    }

    let fixture = workspace_copy("smoke-flow");
    let (_, prefix) = write_non_terminal_session_prefix(&fixture);

    let child = flow_command()
        .current_dir(&fixture)
        .args([
            "tail",
            "smoke-flow",
            "smoke-flow",
            "--emit",
            "jsonl",
            "--no-follow",
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
}

#[test]
fn tail_follows_a_growing_session_through_its_terminal_event() {
    if super::test_support::run_current_test_isolated_session_home() {
        return;
    }

    let fixture = workspace_copy("smoke-flow");
    flow_agent_core::conversation_status(&fixture, None, flow_agent_core::EmitMode::Jsonl)
        .expect("session store initializes");
    let session_dir = workspace_session_dir(&fixture);
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
        .args(["tail", "smoke-flow", "smoke-flow", "--emit", "jsonl"])
        .stdout(closed_pipe_stdout())
        .stderr(Stdio::piped())
        .output()
        .expect("flow binary should run");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(output.status.success(), "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
}
