mod attempts;
mod context;
mod events;
mod productive;
mod stream;
mod validation;
mod workspace;

pub(in crate::tests) use attempts::DiscardAttempts;
pub(super) use context::canonical_context_manifest_line;
pub(super) use events::{
    CollectingEventSink, base_event, event_line, event_line_with_parent, first_event_line,
    flow_completed_line, flow_id_for_definition, flow_started_line, message_completed_line,
    message_delta_line, phase_completed_line, phase_entered_line, phase_failed_line,
    session_event_line, tool_completed_line, tool_failed_line, tool_started_line,
};
#[cfg(unix)]
pub(in crate::tests) use productive::configured_smoke_productive_execution_fixture;
pub(in crate::tests) use productive::{
    ProductiveExecutionFixture, disabled_configured_smoke_productive_execution_fixture,
    disabled_smoke_productive_execution_fixture, load_productive_execution_fixture,
    load_productive_execution_fixture_for_flow, load_productive_execution_fixture_with_credential,
    smoke_productive_execution_fixture,
};
pub(super) use stream::{
    fill_event_segments_to_final_byte, prefix_before_tool_started, prefix_through_tool_progress,
    prefix_through_tool_started, workspace_at_write_summary_progress_with_existing_output,
    write_definition_hash_metadata,
};
pub(super) use validation::{
    assert_invalid_event, assert_invalid_session_log, assert_invalid_stream,
};
#[cfg(windows)]
pub(super) use workspace::create_windows_junction;
pub(super) use workspace::{
    add_bad_write_tool_to_summarize, assert_no_active_session_lock, assert_no_session_artifacts,
    canonical_test_path, copy_workspace_runtime, create_directory_alias, disable_smoke_echo_tool,
    empty_workspace, ensure_workspace_log_dir, ensure_workspace_session_dir,
    fixture_runtime_policy, load_test_registry, remove_directory_alias, replace_registry_text,
    reserve_session_log, reserve_session_log_with_publish_observer, workspace_log_dir,
    workspace_session_dir, workspace_store_dir, workspace_with_later_invalid_own_script_path,
    write_productive_workspace_config,
};
