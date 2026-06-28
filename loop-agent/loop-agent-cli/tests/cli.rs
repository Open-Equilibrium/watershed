use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn loop_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_loop"))
}

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join(name)
}

fn expected_stream(fixture: &str, stream: &str) -> String {
    fs::read_to_string(fixture_dir(fixture).join("expected").join(stream))
        .expect("expected stream is readable")
}

fn workspace_copy(fixture: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target = std::env::temp_dir().join(format!(
        "watershed-loop-agent-cli-{}-{id}",
        std::process::id()
    ));
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    copy_dir(&fixture_dir(fixture), &target);
    target
}

fn copy_dir(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("target directory created");
    for entry in fs::read_dir(source).expect("source directory readable") {
        let entry = entry.expect("source entry readable");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            fs::copy(&source_path, &target_path).expect("fixture file copied");
        }
    }
}

#[test]
fn version_flag_prints_package_version() {
    let output = loop_command()
        .arg("--version")
        .output()
        .expect("loop binary should run");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        format!("loop {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn short_version_flag_prints_package_version() {
    let output = loop_command()
        .arg("-V")
        .output()
        .expect("loop binary should run");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        format!("loop {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
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
        fs::read_to_string(workspace.join(".loop/sessions/smoke001.jsonl"))
            .expect("session log is written"),
        expected
    );
    assert!(
        workspace.join(".loop/logs/smoke001.log").is_file(),
        "structured run log should be written"
    );
}

#[test]
fn run_loop_can_be_repeated_in_same_workspace() {
    let workspace = workspace_copy("smoke-loop");
    let first = loop_command()
        .current_dir(&workspace)
        .args(["run", "smoke-loop", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");
    assert!(first.status.success());
    assert_eq!(
        String::from_utf8(first.stdout).expect("stdout should be UTF-8"),
        expected_stream("smoke-loop", "smoke-loop.jsonl")
    );

    let second = loop_command()
        .current_dir(&workspace)
        .args(["run", "smoke-loop", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");

    assert!(second.status.success());
    assert!(second.stderr.is_empty());
    let stdout = String::from_utf8(second.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("\"session_id\":\"smoke001-2\""));
    assert!(workspace.join(".loop/sessions/smoke001-2.jsonl").is_file());
}

#[test]
fn run_hello_loop_emits_multi_phase_subloop_golden_stream() {
    let workspace = workspace_copy("hello-loop");
    let output = loop_command()
        .current_dir(&workspace)
        .args(["run", "hello-loop", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        expected_stream("hello-loop", "hello-loop.jsonl")
    );
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt"))
            .expect("own-script stub writes summary"),
        "hello\n"
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
            .args([command, "smoke001", "--emit", "jsonl"])
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
        "smoke001\n"
    );
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
    let before = fs::read_to_string(fixture.join(".loop/sessions/smoke001.jsonl"))
        .expect("session log exists");

    let output = loop_command()
        .current_dir(&fixture)
        .args(["resume", "smoke001", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");

    assert_eq!(output.status.code(), Some(65));
    assert!(String::from_utf8(output.stderr)
        .expect("stderr should be UTF-8")
        .contains("terminal session"));
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(fixture.join(".loop/sessions/smoke001.jsonl"))
            .expect("session log exists"),
        before
    );
}

#[test]
fn unsafe_session_id_is_rejected_before_filesystem_access() {
    let workspace = workspace_copy("smoke-loop");
    let output = loop_command()
        .current_dir(workspace)
        .args(["replay", "../smoke001", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");

    assert_eq!(output.status.code(), Some(64));
    assert!(String::from_utf8(output.stderr)
        .expect("stderr should be UTF-8")
        .contains("invalid session_id"));
    assert!(output.stdout.is_empty());
}

#[test]
fn chat_hello_command_runs_hello_loop() {
    let workspace = workspace_copy("hello-loop");
    let mut child = loop_command()
        .current_dir(workspace)
        .arg("chat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("loop binary should spawn");

    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("stdin is piped");
        stdin.write_all(b"/hello-loop\n").expect("stdin write");
    }

    let output = child.wait_with_output().expect("loop binary should exit");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        expected_stream("hello-loop", "hello-loop.jsonl")
    );
}

#[test]
fn sandbox_negative_streams_fail_without_side_effects() {
    for (loop_name, stream_name) in [
        ("sandbox-negative-write", "sandbox-negative-write.jsonl"),
        ("sandbox-negative-network", "sandbox-negative-network.jsonl"),
        (
            "sandbox-negative-environment",
            "sandbox-negative-environment.jsonl",
        ),
        (
            "sandbox-negative-interpreter",
            "sandbox-negative-interpreter.jsonl",
        ),
        (
            "sandbox-negative-protected-path",
            "sandbox-negative-protected-path.jsonl",
        ),
        ("sandbox-negative-symlink", "sandbox-negative-symlink.jsonl"),
        (
            "sandbox-negative-tool-out-of-phase",
            "sandbox-negative-tool-out-of-phase.jsonl",
        ),
    ] {
        let workspace = workspace_copy("sandbox-negative");
        let output = loop_command()
            .current_dir(&workspace)
            .args(["run", loop_name, "--emit", "jsonl"])
            .output()
            .expect("loop binary should run");

        assert_eq!(output.status.code(), Some(65), "{loop_name}");
        assert!(output.stderr.is_empty(), "{loop_name}");
        assert_eq!(
            String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
            expected_stream("sandbox-negative", stream_name),
            "{loop_name}"
        );
        assert!(
            !workspace.join("out/forbidden.txt").exists(),
            "{loop_name} must not create side effects after rejection"
        );
    }
}

#[test]
fn failed_human_run_does_not_report_completion() {
    let workspace = workspace_copy("sandbox-negative");
    let output = loop_command()
        .current_dir(&workspace)
        .args(["run", "sandbox-negative-write"])
        .output()
        .expect("loop binary should run");

    assert_eq!(output.status.code(), Some(65));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert_eq!(stdout, "loop sandbox-negative-write failed\n");
    assert!(!stdout.contains("completed"));
    assert!(
        !workspace.join("out/forbidden.txt").exists(),
        "failed human run must not create side effects after rejection"
    );
}

#[test]
fn non_unicode_argument_exits_with_usage_error() {
    let output = loop_command()
        .arg(non_unicode_argument())
        .output()
        .expect("loop binary should run");

    assert_eq!(output.status.code(), Some(64));
    assert!(String::from_utf8(output.stderr)
        .expect("stderr should be UTF-8")
        .contains("arguments must be valid UTF-8"));
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
