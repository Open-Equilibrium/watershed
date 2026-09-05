use super::super::contract::protocol;
use crate::runtime::types::RuntimeError;
use serde::{Deserialize, Serialize};

pub(crate) const CONVERSATION_ENTRY_SCHEMA_V1: &str = "flow-conversation-entry-v1";
pub(crate) const MAX_HISTORY_INDEX_ID_BYTES: usize = proto::MAX_SESSION_ID_BYTES;
pub(super) const INDEX_ID_FIELD_BYTES: usize = MAX_HISTORY_INDEX_ID_BYTES + 1;
pub(super) const INDEX_ENTRY_ID_OFFSET: usize = 0;
pub(super) const INDEX_PARENT_ID_OFFSET: usize = INDEX_ENTRY_ID_OFFSET + INDEX_ID_FIELD_BYTES;
pub(super) const INDEX_RUN_SESSION_ID_OFFSET: usize = INDEX_PARENT_ID_OFFSET + INDEX_ID_FIELD_BYTES;
pub(super) const INDEX_ORDINAL_OFFSET: usize = INDEX_RUN_SESSION_ID_OFFSET + INDEX_ID_FIELD_BYTES;
pub(super) const INDEX_EVENT_SEQUENCE_OFFSET: usize =
    INDEX_ORDINAL_OFFSET + std::mem::size_of::<u64>();
pub(super) const INDEX_RECORD_BYTES: usize =
    INDEX_EVENT_SEQUENCE_OFFSET + std::mem::size_of::<u64>();
pub(super) const EVENT_POINTER_SEQUENCE_OFFSET: usize = INDEX_ID_FIELD_BYTES;
pub(super) const EVENT_POINTER_RECORD_BYTES: usize =
    EVENT_POINTER_SEQUENCE_OFFSET + std::mem::size_of::<u64>();
pub(super) const INDEX_SORT_BYTES: usize = 16 * 1024 * 1024;
pub(super) const INDEX_MERGE_FAN_IN: u64 = 64;

pub(super) type IndexRecord = [u8; INDEX_RECORD_BYTES];
pub(super) type EventPointerRecord = [u8; EVENT_POINTER_RECORD_BYTES];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ConversationEntryType {
    Checkpoint,
    Continuation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConversationEntry {
    pub(crate) schema: String,
    pub(crate) entry_id: String,
    pub(crate) parent_entry_id: Option<String>,
    pub(crate) recovery_snapshot_hash: String,
    pub(crate) run_session_id: String,
    pub(crate) event_sequence: u64,
    pub(crate) entry_type: ConversationEntryType,
    pub(crate) timestamp: String,
}

#[derive(Clone, Copy)]
pub(super) struct WorkBudget {
    pub(super) used: u64,
    pub(super) limit: u64,
}

impl WorkBudget {
    pub(super) fn add(&mut self, amount: u64) -> Result<(), RuntimeError> {
        self.used = self
            .used
            .checked_add(amount)
            .ok_or_else(|| protocol("conversation history work count overflow"))?;
        if self.used > self.limit {
            return Err(protocol(
                "conversation history exceeds its O(n log n) work budget",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct EventPointerMetrics {
    #[cfg(test)]
    pub(super) state_payload_peak: u64,
    #[cfg(test)]
    pub(super) work: u64,
    #[cfg(test)]
    pub(super) work_limit: u64,
}

impl EventPointerMetrics {
    pub(super) fn include(&mut self, _other: Self) {
        #[cfg(test)]
        {
            self.state_payload_peak = self.state_payload_peak.max(_other.state_payload_peak);
            self.work = self.work.max(_other.work);
            self.work_limit = self.work_limit.max(_other.work_limit);
        }
    }
}
