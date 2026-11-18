use super::{
    flow_command,
    process::{cli_child_watchdog, wait_with_input_and_output_before},
    test_support::{expected_stream, session_home_path, workspace_copy, workspace_session_dir},
};
use std::{
    fs,
    path::Path,
    process::{Output, Stdio},
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
    wait_with_input_and_output_before(child, input, cli_child_watchdog())
}

#[test]
fn chat_preserves_the_registered_hello_flow_reference() {
    let run_workspace = workspace_copy("hello-flow");
    let chat_workspace = workspace_copy("hello-flow");
    let global_registry = session_home_path().join("registry/flows");
    let original = global_registry.join("hello-flow.yaml");
    let renamed = global_registry.join("hello.yaml");
    let definition = fs::read_to_string(&original)
        .expect("hello Flow definition reads")
        .replacen("id: hello-flow", "id: hello", 1);
    fs::write(&renamed, definition).expect("hello Flow definition writes");
    fs::remove_file(original).expect("hello-flow alias definition is absent");

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
    let workspace = workspace_copy("negative");
    fs::write(
        session_home_path().join("registry/flows/hello-flow.yaml"),
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
fn chat_rejects_unusable_input_before_session_mutation() {
    let overlong = |chars: usize| [vec![b'x'; chars], vec![b'\n']].concat();
    let nonblank = "error: flow chat requires one nonblank stdin Flow reference\n";
    for (case, input, code, stderr) in [
        ("blank-only", b"\n   \n".to_vec(), 64, nonblank),
        ("slash-only", b"/\n".to_vec(), 64, nonblank),
        (
            "overlong",
            overlong(core_script::MAX_BLOCK_NAME_CHARS + 1),
            64,
            nonblank,
        ),
        ("overlong-1024", overlong(1_024), 64, nonblank),
        (
            "invalid-utf8",
            b"\xff\n".to_vec(),
            64,
            "error: flow chat stdin must be valid UTF-8\n",
        ),
        (
            "unknown-reference",
            b"/unknown\n".to_vec(),
            65,
            "error: registry root references missing flow unknown\n",
        ),
    ] {
        let workspace = workspace_copy("hello-flow");
        let output = run_chat(&workspace, &input);

        assert_eq!(output.status.code(), Some(code), "{case}");
        assert!(output.stdout.is_empty(), "{case}");
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr should be UTF-8"),
            stderr,
            "{case}"
        );
        assert!(!workspace_session_dir(&workspace).exists(), "{case}");
    }
}
