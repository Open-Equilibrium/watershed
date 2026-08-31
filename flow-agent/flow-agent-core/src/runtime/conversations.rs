mod contract;
#[cfg(test)]
pub(crate) use contract::CONVERSATION_STATUS_PAGE_SCHEMA;
#[cfg(test)]
pub(crate) use contract::MAX_CONVERSATION_SEGMENT_BYTES;
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use contract::{
    CONVERSATION_HISTORY_LEAF, CONVERSATION_RUNS_DIR, CONVERSATION_STATUS_LEAF,
    MAX_CONVERSATION_IO_BUFFER_BYTES, MAX_CONVERSATION_SCAN_BYTES, MAX_CONVERSATION_SCAN_RECORDS,
    MAX_CONVERSATION_STATUS_BYTES, MAX_CONVERSATION_STATUS_RECORDS, RUN_EVENTS_STEM, RUN_LOG_LEAF,
    RUN_LOG_RECORD_SCHEMA_V1, TOOL_RUN_LOG_PAGE_SCHEMA,
};
pub(crate) use contract::{MAX_CONVERSATION_RECORD_BYTES, RUN_EVENTS_LEAF};

mod event_reader;
mod history_index;
mod session_event_stream;
pub use event_reader::SessionEventReader;
pub(crate) use event_reader::ensure_in_memory_replay_output_limit;
#[cfg(test)]
pub(crate) use history_index::append_conversation_entry;
pub(crate) use history_index::append_productive_run_checkpoint;
#[cfg(test)]
pub(crate) use history_index::read_conversation_history;
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use history_index::{
    CONVERSATION_ENTRY_SCHEMA_V1, ConversationEntry, ConversationEntryType,
};
#[cfg(test)]
pub(crate) use history_index::{
    HistoryScratchFault, HistoryScratchMemberStage, HistoryScratchStage,
    abandon_history_index_scratch_for_test, abandon_history_index_scratches_for_test,
    complete_history_index_scratch_for_test, history_index_limits_for_test,
    history_validation_dir_path_for_test, set_event_pointer_sort_record_limit_for_test,
    set_history_index_available_space_for_test, set_history_index_sort_record_limit_for_test,
    set_history_scratch_fault_for_test, take_history_index_metrics_for_test,
    with_event_identifier_digest_collision_for_test, with_history_scratch_member_observer_for_test,
    with_history_scratch_stage_observer_for_test,
};
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use history_index::{
    MAX_HISTORY_INDEX_ID_BYTES, validate_conversation_history_for_budget,
};

mod event_persistence;

mod attempt_log;
pub(crate) use attempt_log::ConversationAttemptLog;

mod conversation_writer;
pub(crate) use conversation_writer::ConversationEventWriter;

mod lifecycle;
#[cfg(all(test, unix))]
pub(crate) use lifecycle::set_run_creation_stage_observer;
#[cfg(test)]
pub(crate) use lifecycle::{
    create_conversation_run, create_conversation_run_with_model_profile,
    create_unpublished_productive_conversation_run, set_conversation_lifecycle_cleanup_observer,
    set_conversation_root_cleanup_observer, set_partial_run_cleanup_observer,
    set_productive_run_creation_observer, set_run_sibling_scan_observer,
};
pub(crate) use lifecycle::{
    create_unpublished_productive_conversation_run_with_model_profile,
    reclaim_productive_run_creation, reclaim_unpublished_productive_run,
};

mod prefix_reader;

mod query;
pub use query::conversation_status;
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use query::conversation_status_page;
#[cfg(test)]
pub(crate) use query::{ConversationStatus, ConversationStatusPage};

mod run_log;
pub use run_log::project_tool_run_log;
pub(crate) use run_log::{RunAttemptLedger, inspect_run_attempts};
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use run_log::{RunLogProjectionPage, RunLogRecord, project_tool_run_log_page};
#[cfg(test)]
pub(crate) use run_log::{append_run_attempt_intent, append_run_attempt_result};

mod run_objects;
pub(crate) use run_objects::RunObjectStore;
#[cfg(test)]
pub(crate) use run_objects::RunObjectUsageSnapshot;

mod recovery_record;
#[cfg(test)]
pub(crate) use recovery_record::ProductiveRecoveryRecord;

mod recovery;
pub(crate) use recovery::{
    ProductiveConversationReservation, ProductiveRecoveryWriter,
    read_conversation_continuation_definition, read_conversation_recovery_definition,
    reserve_conversation_continuation, reserve_conversation_run_recovery,
    reserve_new_conversation_run, with_conversation_run_ownership,
};

mod status;
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use status::{ConversationStatusSummary, STATUS_SUMMARY_SCHEMA};
mod conversation_stream;
mod productive_storage;
mod storage;
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use conversation_stream::{append_jsonl, read_jsonl};
#[cfg(test)]
pub(crate) use conversation_stream::{
    conversation_stream_parent_sync_count_for_path_for_test,
    reset_conversation_stream_parent_sync_count_for_path_for_test,
    set_conversation_batch_append_error_after_commit_for_path_for_test,
    set_conversation_file_sync_error_for_path_for_test,
    set_conversation_out_of_band_append_before_next_append_for_path_for_test,
    set_conversation_stream_parent_sync_error_for_path_for_test, target_segment_count_for_test,
};
#[cfg(test)]
pub(crate) use conversation_stream::{read_anchored_jsonl, read_jsonl_quantum};
#[cfg(test)]
pub(crate) use status::{StatusTransactionCrashPoint, set_status_transaction_crash_point};
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use storage::canonical_json;
pub(crate) use storage::existing_anchored_run;
#[cfg(test)]
pub(crate) use storage::real_child_file_names;
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use storage::{ConversationOperationMetrics, measure_conversation_operation};
