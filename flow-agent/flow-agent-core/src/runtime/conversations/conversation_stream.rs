mod layout;
mod reader;
mod writer;

pub(in crate::runtime::conversations) use layout::run_segment_leaf;
#[cfg(test)]
pub(crate) use layout::target_segment_count_for_test;
pub(in crate::runtime::conversations) use reader::jsonl_segment_from_open_file;
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use reader::read_jsonl;
#[cfg(test)]
pub(crate) use reader::read_jsonl_quantum;
pub(crate) use reader::{
    open_anchored_jsonl_segment, read_anchored_jsonl, read_anchored_jsonl_quantum,
    validate_jsonl_segment_snapshot,
};
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use writer::append_jsonl;
pub(in crate::runtime::conversations) use writer::{
    append_anchored_canonical_jsonl_batch, append_anchored_canonical_jsonl_batch_with,
    conversation_file_sync_checkpoint, create_anchored_jsonl_file, open_anchored_stream_appender,
    sync_anchored_stream, sync_anchored_stream_with,
};
#[cfg(test)]
pub(crate) use writer::{
    conversation_stream_parent_sync_count_for_path_for_test,
    reset_conversation_stream_parent_sync_count_for_path_for_test,
    set_conversation_batch_append_error_after_commit_for_path_for_test,
    set_conversation_file_sync_error_for_path_for_test,
    set_conversation_out_of_band_append_before_next_append_for_path_for_test,
    set_conversation_stream_parent_sync_error_for_path_for_test,
};
