use crate::runtime::{
    fs_guards::{AnchoredFile, read_anchored_file_with_limit},
    types::{MAX_SESSION_METADATA_BYTES, RuntimeError},
};

mod reservation;
mod selection;
mod writer;

fn read_productive_recovery_snapshot(path: &AnchoredFile) -> Result<Vec<u8>, RuntimeError> {
    read_anchored_file_with_limit(path, MAX_SESSION_METADATA_BYTES)
}

pub(crate) use reservation::{
    ProductiveConversationReservation, reserve_conversation_continuation,
    reserve_conversation_run_recovery, reserve_new_conversation_run,
    with_conversation_run_ownership,
};
pub(crate) use writer::ProductiveRecoveryWriter;
