use super::{
    flow_command,
    process::wait_with_input_and_output_before,
    test_support::{expected_stream, workspace_copy, workspace_session_dir},
};
use std::{
    fs,
    path::Path,
    process::{Output, Stdio},
    time::Duration,
};

fn run_chat(workspace: &Path, input: &[u8]) -> Output {
    let child = flow_command()
        .current_dir(workspace)
        .arg("chat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("flow binary should spawn");
    wait_with_input_and_output_before(child, input, chat_child_watchdog())
}

fn chat_child_watchdog() -> Duration {
    if cfg!(windows) || std::env::var_os("CARGO_LLVM_COV").is_some() {
        Duration::from_secs(30)
    } else {
        Duration::from_secs(10)
    }
}

#[test]
fn chat_preserves_the_registered_hello_flow_reference() {
    let run_workspace = workspace_copy("hello-flow");
    let chat_workspace = workspace_copy("hello-flow");
    for workspace in [&run_workspace, &chat_workspace] {
        let original = workspace.join("registry/flows/hello-flow.yaml");
        let renamed = workspace.join("registry/flows/hello.yaml");
        let definition = fs::read_to_string(&original)
            .expect("hello Flow definition reads")
            .replacen("id: hello-flow", "id: hello", 1);
        fs::write(&renamed, definition).expect("hello Flow definition writes");
        fs::remove_file(original).expect("hello-flow alias definition is absent");
    }

    let run = flow_command()
        .current_dir(&run_workspace)
        .args(["run", "hello", "--emit", "jsonl"])
        .output()
        .expect("registered hello Flow runs");
    let chat = run_chat(&chat_workspace, b"hello\n");

    assert!(run.status.success());
    assert!(chat.status.success());
    assert!(run.stderr.is_empty());
    assert!(chat.stderr.is_empty());
    assert_eq!(chat.stdout, run.stdout);
    assert!(
        String::from_utf8(chat.stdout)
            .expect("stdout should be UTF-8")
            .contains("\"flow_definition_id\":\"hello\"")
    );
}

#[test]
fn chat_skips_blank_lines_before_a_registered_slash_reference() {
    let workspace = workspace_copy("smoke-flow");
    let output = run_chat(&workspace, b"\n   \n/smoke-flow\n");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        expected_stream("smoke-flow", "smoke-flow.jsonl")
    );
}

#[test]
fn chat_failed_flow_exits_with_failed_status() {
    let workspace = workspace_copy("sandbox-negative");
    fs::write(
        workspace.join("registry/flows/hello-flow.yaml"),
        "flow:\n  id: hello-flow\n  name: HelloFlow\n  phase_refs: [negative-write]\n  subflow_refs: []\n",
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
fn chat_requires_a_nonblank_reference_before_eof() {
    for (case, input) in [
        ("blank-only", b"\n   \n".as_slice()),
        ("slash-only", b"/\n".as_slice()),
    ] {
        let workspace = workspace_copy("hello-flow");
        let output = run_chat(&workspace, input);

        assert_eq!(output.status.code(), Some(64), "{case}");
        assert!(output.stdout.is_empty(), "{case}");
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr should be UTF-8"),
            "error: flow chat requires one nonblank stdin Flow reference\n",
            "{case}"
        );
        assert!(!workspace_session_dir(&workspace).exists(), "{case}");
    }
}

#[test]
fn chat_rejects_increasing_overlong_references_before_session_mutation() {
    let workspace = workspace_copy("hello-flow");
    for input_chars in [core_script::MAX_BLOCK_NAME_CHARS + 1, 1_024] {
        let mut input = vec![b'x'; input_chars];
        input.push(b'\n');
        let output = run_chat(&workspace, &input);

        assert_eq!(output.status.code(), Some(64), "{input_chars}");
        assert!(output.stdout.is_empty(), "{input_chars}");
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr should be UTF-8"),
            "error: flow chat requires one nonblank stdin Flow reference\n",
            "{input_chars}"
        );
        assert!(!workspace_session_dir(&workspace).exists(), "{input_chars}");
    }
}

#[test]
fn chat_unknown_reference_fails_before_session_mutation() {
    let workspace = workspace_copy("hello-flow");
    let output = run_chat(&workspace, b"/unknown\n");

    assert_eq!(output.status.code(), Some(65));
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr should be UTF-8"),
        "error: registry root references missing flow unknown\n"
    );
    assert!(output.stdout.is_empty());
    assert!(!workspace_session_dir(&workspace).exists());
}
