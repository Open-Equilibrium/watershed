use super::{
    flow_command,
    test_support::{
        empty_workspace, workspace_copy, workspace_session_dir, write_sized_conversation_replay,
    },
};

#[test]
fn replay_jsonl_streams_output_above_the_in_memory_limit() {
    if super::test_support::run_current_test_isolated_session_home() {
        return;
    }

    let workspace = empty_workspace();
    flow_agent_core::conversation_status(&workspace, None, flow_agent_core::EmitMode::Jsonl)
        .expect("session store initializes");
    let conversation_id = "cliconversation001";
    let run_session_id = "cliconversationrun001";
    let total_bytes = 67_108_865usize;
    write_sized_conversation_replay(
        &workspace_session_dir(&workspace),
        conversation_id,
        run_session_id,
        total_bytes,
        |_| {},
    );

    let output = flow_command()
        .current_dir(&workspace)
        .args(["replay", conversation_id, run_session_id, "--emit", "jsonl"])
        .output()
        .expect("large replay starts");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout.len(), total_bytes);
}

#[test]
fn unsafe_session_id_is_rejected_before_filesystem_access() {
    let workspace = workspace_copy("smoke-flow");
    let output = flow_command()
        .current_dir(&workspace)
        .args(["replay", "../smoke001", "../smoke001", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");

    assert_eq!(output.status.code(), Some(64));
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr should be UTF-8")
            .contains("invalid conversation or run session id")
    );
    assert!(output.stdout.is_empty());
}
