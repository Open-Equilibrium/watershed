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
    copy_fixture_workspace(&fixture_dir(fixture), &target);
    target
}

fn copy_fixture_workspace(source: &Path, target: &Path) {
    copy_dir(source, target);
    copy_workspace_config(source, target);
}

fn copy_dir(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("target directory created");
    for entry in fs::read_dir(source).expect("source directory readable") {
        let entry = entry.expect("source entry readable");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() && entry.file_name() == ".loop" {
            continue;
        }
        if source_path.is_dir() && entry.file_name() == "out" {
            fs::create_dir_all(&target_path).expect("output directory shape copied");
            continue;
        }
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            fs::copy(&source_path, &target_path).expect("fixture file copied");
        }
    }
}

fn copy_workspace_config(source: &Path, target: &Path) {
    let source_config = source.join(".loop/config.yaml");
    if !source_config.exists() {
        return;
    }
    let target_config = target.join(".loop/config.yaml");
    fs::create_dir_all(target_config.parent().expect("config path has parent"))
        .expect("workspace config directory created");
    fs::copy(source_config, target_config).expect("workspace config copied");
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
fn workspace_copy_skips_fixture_runtime_state() {
    let fixture = fixture_dir("hello-loop");
    let stale_session = fixture.join(".loop/sessions/stale.jsonl");
    let stale_output = fixture.join("out/summary.txt");
    let _guard = FixtureRuntimeStateGuard::new([stale_session.clone(), stale_output.clone()]);
    fs::create_dir_all(stale_session.parent().expect("session path has parent"))
        .expect("stale session parent created");
    fs::write(&stale_session, "{}\n").expect("stale session created");
    fs::write(&stale_output, "stale\n").expect("stale output created");

    let workspace = workspace_copy("hello-loop");

    assert!(
        workspace.join(".loop/config.yaml").exists(),
        "workspace config must still be copied"
    );
    assert!(
        !workspace.join(".loop/sessions/stale.jsonl").exists(),
        "fixture runtime session state must not be copied"
    );
    assert!(
        !workspace.join("out/summary.txt").exists(),
        "fixture output state must not be copied"
    );
}

struct FixtureRuntimeStateGuard {
    paths: Vec<PathBuf>,
}

impl FixtureRuntimeStateGuard {
    fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            paths: paths.into_iter().collect(),
        }
    }
}

impl Drop for FixtureRuntimeStateGuard {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
        for path in &self.paths {
            if let Some(parent) = path.parent() {
                let _ = fs::remove_dir(parent);
            }
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
fn no_arguments_and_unknown_commands_print_usage_errors() {
    for args in [Vec::<&str>::new(), vec!["unknown"]] {
        let output = loop_command()
            .args(args)
            .output()
            .expect("loop binary should run");

        assert_eq!(output.status.code(), Some(64));
        assert!(String::from_utf8(output.stderr)
            .expect("stderr should be UTF-8")
            .contains("usage: loop run <loop>"));
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
fn tail_timeout_argument_is_accepted() {
    let workspace = workspace_copy("smoke-loop");
    let run = loop_command()
        .current_dir(&workspace)
        .args(["run", "smoke-loop", "--emit", "jsonl"])
        .output()
        .expect("loop binary should run");
    assert!(run.status.success());

    let output = loop_command()
        .current_dir(&workspace)
        .args(["tail", "smoke001", "--timeout-ms", "1"])
        .output()
        .expect("loop binary should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        "session smoke001 tailed\n"
    );
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
    fs::write(session_dir.join("smoke001.jsonl"), &prefix).expect("partial session written");

    let output = loop_command()
        .current_dir(&fixture)
        .args([
            "tail",
            "smoke001",
            "--emit",
            "jsonl",
            "--no-follow",
            "--timeout-ms",
            "25",
        ])
        .output()
        .expect("loop binary should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        prefix
    );
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
        .args(["tail", "smoke001", "--emit", "jsonl"])
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
    replace_seeded_session_with_prefix(&workspace, "smoke001", &prefix);

    let output = loop_command()
        .current_dir(&workspace)
        .args(["resume", "smoke001"])
        .output()
        .expect("loop binary should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        "session smoke001 resumed\n"
    );
    assert!(fs::read_to_string(session_dir.join("smoke001.jsonl"))
        .expect("resumed log readable")
        .contains("\"event_type\":\"session.completed\""));
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
    replace_seeded_session_with_prefix(&workspace, "negwrite001", &prefix);

    let output = loop_command()
        .current_dir(&workspace)
        .args(["resume", "negwrite001", "--emit", "jsonl"])
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
fn chat_failed_loop_exits_with_failed_status() {
    let workspace = workspace_copy("sandbox-negative");
    fs::write(
        workspace.join("registry/loops/hello-loop.yaml"),
        "loop:\n  id: hello-loop\n  name: HelloLoop\n  phase_refs: [negative-write]\n  subloop_refs: []\n  connection_refs: []\n",
    )
    .expect("chat loop fixture written");
    let mut child = loop_command()
        .current_dir(&workspace)
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
        stdin.write_all(b"\n   \n").expect("stdin write");
    }

    let output = child.wait_with_output().expect("loop binary should exit");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn chat_rejects_unsupported_commands() {
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
        stdin.write_all(b"/unknown\n").expect("stdin write");
    }

    let output = child.wait_with_output().expect("loop binary should exit");

    assert_eq!(output.status.code(), Some(64));
    assert!(String::from_utf8(output.stderr)
        .expect("stderr should be UTF-8")
        .contains("unsupported chat command"));
    assert!(output.stdout.is_empty());
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
            .args([command, "negwrite001", "--emit", "jsonl"])
            .output()
            .expect("loop binary should run");

        assert_eq!(output.status.code(), Some(65), "{command}");
        assert!(output.stderr.is_empty(), "{command}");
        assert!(String::from_utf8(output.stdout)
            .expect("stdout should be UTF-8")
            .contains("\"event_type\":\"session.failed\""));
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
    assert_eq!(stdout, "loop sandbox-negative-write failed: write_denied\n");
    assert!(!stdout.contains("completed"));
    assert!(
        !workspace.join("out/forbidden.txt").exists(),
        "failed human run must not create side effects after rejection"
    );

    for (command, expected) in [
        (
            "replay",
            "session negwrite001 replayed: failed (write_denied)\n",
        ),
        (
            "tail",
            "session negwrite001 tailed: failed (write_denied)\n",
        ),
    ] {
        let output = loop_command()
            .current_dir(&workspace)
            .args([command, "negwrite001"])
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
