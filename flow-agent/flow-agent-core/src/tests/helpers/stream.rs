use super::workspace::load_test_registry;
use crate::runtime::{
    config_io::load_global_config,
    context_persistence::SessionObjectWriter,
    digest::sha256_hex,
    execution_plan::{
        FlowExecutionAction, FlowExecutionOptions, ToolSideEffectMode, runtime_policy_target,
    },
    fs_guards::{AnchoredFile, ensure_runtime_dirs, segmented_jsonl_path},
    planning::plan_flow,
    types::{EVENT_STREAM_LIMITS, MAX_SESSION_SEGMENT_BYTES},
};
use crate::tests::test_support::{TempWorkspace, expected_stream, workspace_copy};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(in crate::tests) fn workspace_at_write_summary_progress_with_existing_output()
-> (TempWorkspace, PathBuf) {
    let workspace = workspace_copy("hello-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let prefix = prefix_through_tool_progress(
        &expected_stream("hello-flow", "hello-flow.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-flow.jsonl");
    fs::write(&path, prefix).expect("progress prefix written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");
    fs::create_dir_all(workspace.join("out")).expect("output dir created");
    fs::write(workspace.join("out/summary.txt"), "already-written\n")
        .expect("sentinel summary written");
    (workspace, path)
}

pub(in crate::tests) fn fill_event_segments_to_final_byte(base: &AnchoredFile) {
    for ordinal in 1..=EVENT_STREAM_LIMITS.max_segments {
        let path = segmented_jsonl_path(base, ordinal).expect("segment path resolves");
        let byte_count = if ordinal == EVENT_STREAM_LIMITS.max_segments {
            usize::try_from(MAX_SESSION_SEGMENT_BYTES - 1).expect("segment size fits")
        } else {
            1
        };
        let mut bytes = vec![b'x'; byte_count];
        *bytes.last_mut().expect("segment is nonempty") = b'\n';
        fs::write(path.diagnostic_path(), bytes).expect("saturated segment writes");
    }
}

pub(in crate::tests) fn prefix_through_tool_progress(stream: &str, tool_id: &str) -> String {
    prefix_through_tool_event(stream, "tool.progress", tool_id)
}

pub(in crate::tests) fn prefix_through_tool_started(stream: &str, tool_id: &str) -> String {
    prefix_through_tool_event(stream, "tool.started", tool_id)
}

pub(in crate::tests) fn prefix_before_tool_started(stream: &str, tool_id: &str) -> String {
    let event_marker = "\"event_type\":\"tool.started\"";
    let tool_marker = format!("\"tool_id\":\"{tool_id}\"");
    let mut prefix = String::new();
    for line in stream.lines() {
        if line.contains(event_marker) && line.contains(&tool_marker) {
            return prefix;
        }
        prefix.push_str(line);
        prefix.push('\n');
    }
    panic!("missing tool.started for {tool_id}");
}

fn prefix_through_tool_event(stream: &str, event_type: &str, tool_id: &str) -> String {
    let event_marker = format!("\"event_type\":\"{event_type}\"");
    let tool_marker = format!("\"tool_id\":\"{tool_id}\"");
    let mut prefix = String::new();
    for line in stream.lines() {
        prefix.push_str(line);
        prefix.push('\n');
        if line.contains(&event_marker) && line.contains(&tool_marker) {
            return prefix;
        }
    }
    panic!("missing {event_type} for {tool_id}");
}

pub(in crate::tests) fn write_definition_hash_metadata(
    workspace: &Path,
    session_id: &str,
    flow_ref: &str,
) {
    let registry = load_test_registry(workspace, flow_ref);
    let flow_block = registry.flow_block(flow_ref).expect("flow exists");
    let registry_json = registry.canonical_json().expect("registry serializes");
    let flow_json = proto::canonical_json(
        &serde_json::to_value(flow_block).expect("flow definition converts to JSON"),
    )
    .expect("flow definition serializes");
    let log_dir = crate::tests::helpers::ensure_workspace_log_dir(workspace);
    fs::write(
        log_dir.join(format!("{session_id}.log")),
        format!(
            "registry_hash=sha256:{}\nflow_definition_hash=sha256:{}\nflow_definition_id={flow_ref}\n",
            sha256_hex(registry_json.as_bytes()),
            sha256_hex(flow_json.as_bytes())
        ),
    )
    .expect("definition hash metadata written");

    let session_text = fs::read_to_string(
        crate::tests::helpers::workspace_session_dir(workspace).join(format!("{session_id}.jsonl")),
    )
    .expect("session prefix reads for context fixture");
    let completed_turns = session_text
        .lines()
        .filter(|line| line.contains("\"event_type\":\"message.completed\""))
        .count();
    let config = load_global_config().expect("global config loads");
    let policy = core_policy::compile_policy_artifact(&registry, flow_ref, runtime_policy_target())
        .expect("runtime policy compiles");
    let plan = plan_flow(
        workspace,
        &registry,
        &policy,
        flow_block,
        session_id,
        FlowExecutionOptions::with_stub_model_fixture_profile(
            config.event_clock,
            ToolSideEffectMode::Plan,
            config.stub_model_fixture_profile,
        ),
    )
    .expect("context fixture replay plans");
    assert!(completed_turns <= plan.execution.context_manifests.record_count);
    let checkpoints = plan
        .actions
        .iter()
        .filter_map(|action| match action {
            FlowExecutionAction::Event(action) => action.context_checkpoint.as_ref(),
            FlowExecutionAction::Fixture(_) => None,
        })
        .take(completed_turns)
        .collect::<Vec<_>>();
    let context_stream = checkpoints
        .iter()
        .map(|checkpoint| checkpoint.manifest.line.as_str())
        .collect::<String>();
    fs::write(
        log_dir.join(format!("{session_id}.contexts.jsonl")),
        context_stream,
    )
    .expect("context fixture manifests written");
    let mut object_writer = SessionObjectWriter::open(
        ensure_runtime_dirs(workspace)
            .expect("runtime dirs remain available")
            .sessions,
        session_id,
    )
    .expect("context fixture object writer opens");
    for checkpoint in checkpoints {
        object_writer
            .persist_all(&checkpoint.objects)
            .expect("context fixture objects written");
    }
}
