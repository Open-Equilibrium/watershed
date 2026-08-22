#![cfg_attr(all(test, not(feature = "m11-budget-evidence")), allow(dead_code))]

mod authoring;
#[cfg(test)]
pub(crate) use authoring::maximum_tool;
mod conversations;
mod runner;
#[cfg(test)]
pub(crate) use conversations::verify_conversation_operation_boundaries_for_test;

use std::{hint::black_box, path::Path, time::Duration};

const MIB: u64 = 1024 * 1024;
const RSS_FIXTURE_BYTES: usize = 4 * 1024 * 1024;
const RSS_TOUCH_STRIDE_BYTES: usize = 4096;
const RSS_ACCOUNTING_TOLERANCE_BYTES: u64 = 512 * 1024;

/// One optimized M1.1 workload and its approved pass criteria.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct M11BudgetWorkload {
    /// Exact workload identifier from the approved evidence matrix.
    pub id: M11BudgetWorkloadId,
    /// Maximum allowed p95 elapsed time, when the workload has a timing gate.
    pub p95_limit: Option<Duration>,
    /// Maximum allowed peak RSS growth, when the workload has a memory gate.
    pub max_peak_rss_growth_bytes: Option<u64>,
    /// Minimum required peak RSS growth for the RSS detection fixture.
    pub min_peak_rss_growth_bytes: Option<u64>,
}

impl M11BudgetWorkload {
    /// Stable external name used by the report and child-process protocol.
    pub const fn name(self) -> &'static str {
        self.id.name()
    }
}

/// The finite identities of the optimized M1.1 workloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum M11BudgetWorkloadId {
    /// Resident-memory accounting fixture.
    RssDetectionFixture,
    /// Four sequential no-op Tool launches.
    RunnerFourNoopLaunches,
    /// Ready process-group termination.
    RunnerTermination,
    /// Ready Tool cancellation.
    RunnerCancellation,
    /// Concurrent stdout and stderr caps.
    RunnerDualStreamCaps,
    /// Maximum Tool-definition transaction.
    AuthoringMaxDefinitionTransaction,
    /// Empty-workspace initialization.
    AuthoringInit,
    /// Maximum registry validation.
    AuthoringMaxRegistryValidate,
    /// Bounded Conversation status page.
    ConversationStatusPage,
    /// Bounded Run Log projection page.
    RunLogProjectionPage,
    /// One Conversation migration quantum.
    ConversationMigrationQuantum,
    /// One Conversation replay quantum.
    ConversationReplayQuantum,
    /// Full Run streaming replay.
    ConversationFullRunStreamingReplay,
    /// One Conversation history validation quantum.
    ConversationHistoryValidationQuantum,
    /// Eight synchronized Run Log appends.
    RunLogEightSyncAppends,
}

impl M11BudgetWorkloadId {
    /// Stable external name from the approved evidence matrix.
    pub const fn name(self) -> &'static str {
        match self {
            Self::RssDetectionFixture => "rss_detection_fixture",
            Self::RunnerFourNoopLaunches => "runner_four_noop_launches",
            Self::RunnerTermination => "runner_termination",
            Self::RunnerCancellation => "runner_cancellation",
            Self::RunnerDualStreamCaps => "runner_dual_stream_caps",
            Self::AuthoringMaxDefinitionTransaction => "authoring_max_definition_transaction",
            Self::AuthoringInit => "authoring_init",
            Self::AuthoringMaxRegistryValidate => "authoring_max_registry_validate",
            Self::ConversationStatusPage => "conversation_status_page",
            Self::RunLogProjectionPage => "run_log_projection_page",
            Self::ConversationMigrationQuantum => "conversation_migration_quantum",
            Self::ConversationReplayQuantum => "conversation_replay_quantum",
            Self::ConversationFullRunStreamingReplay => "conversation_full_run_streaming_replay",
            Self::ConversationHistoryValidationQuantum => "conversation_history_validation_quantum",
            Self::RunLogEightSyncAppends => "run_log_eight_sync_appends",
        }
    }
}

impl TryFrom<&str> for M11BudgetWorkloadId {
    type Error = String;

    fn try_from(name: &str) -> Result<Self, Self::Error> {
        M11_BUDGET_WORKLOADS
            .iter()
            .find(|workload| workload.name() == name)
            .map(|workload| workload.id)
            .ok_or_else(|| format!("unknown M1.1 budget workload {name}"))
    }
}

/// Returns the exact input contract used by one optimized workload.
pub fn m11_budget_workload_inputs(id: M11BudgetWorkloadId) -> serde_json::Value {
    use crate::runtime::{
        authoring::{DEFAULT_REGISTRY_ROOT, registry_directory},
        conversations::{
            MAX_CONVERSATION_IO_BUFFER_BYTES, MAX_CONVERSATION_RECORD_BYTES,
            MAX_CONVERSATION_SCAN_BYTES, MAX_CONVERSATION_STATUS_BYTES,
            MAX_CONVERSATION_STATUS_RECORDS, MAX_HISTORY_INDEX_ID_BYTES,
        },
        tool_runner::MAX_TOOL_STREAM_BYTES,
        types::{EVENT_STREAM_LIMITS, MAX_SESSION_EVENT_BYTES, MAX_SESSION_SEGMENT_BYTES},
    };
    use core_script::{
        MAX_REGISTRY_ENTRIES, MAX_REGISTRY_FILE_BYTES, MAX_REGISTRY_TOTAL_BYTES, RegistryBlockKind,
    };
    use serde_json::json;

    match id {
        M11BudgetWorkloadId::RssDetectionFixture => json!({
            "allocation_bytes": RSS_FIXTURE_BYTES,
            "touch_stride_bytes": RSS_TOUCH_STRIDE_BYTES,
            "accounting_tolerance_bytes": RSS_ACCOUNTING_TOLERANCE_BYTES,
        }),
        M11BudgetWorkloadId::RunnerFourNoopLaunches => json!({
            "launches": runner::NOOP_LAUNCHES,
            "executable": runner::NOOP_EXECUTABLE,
            "sequential": true,
            "environment": "empty",
        }),
        M11BudgetWorkloadId::RunnerTermination => json!({
            "ready_children": 1,
            "trigger": "TERM",
            "measurement_start": "post-readiness TERM request",
            "includes": ["process-group termination", "reap", "EOF drain"],
        }),
        M11BudgetWorkloadId::RunnerCancellation => json!({
            "ready_children": 1,
            "trigger": "atomic cancellation",
            "measurement_start": "post-readiness cancellation request",
            "controller": "production Tool controller",
            "includes": ["cancellation observation", "process-group termination", "reap", "EOF drain"],
        }),
        M11BudgetWorkloadId::RunnerDualStreamCaps => json!({
            "stdout_bytes": MAX_TOOL_STREAM_BYTES,
            "stderr_bytes": MAX_TOOL_STREAM_BYTES,
            "concurrent": true,
            "separate_caps": true,
        }),
        M11BudgetWorkloadId::AuthoringMaxDefinitionTransaction => json!({
            "definitions": 1,
            "definition_bytes": MAX_REGISTRY_FILE_BYTES,
            "kind": RegistryBlockKind::Tool.as_str(),
            "stages": ["stage", "sync", "no-replace publish", "reload", "semantic compare"],
        }),
        M11BudgetWorkloadId::AuthoringInit => json!({
            "workspaces": 1,
            "initial_state": "empty",
            "registry_root": DEFAULT_REGISTRY_ROOT,
            "registry_kinds": RegistryBlockKind::ALL.map(registry_directory),
        }),
        M11BudgetWorkloadId::AuthoringMaxRegistryValidate => json!({
            "entries": MAX_REGISTRY_ENTRIES,
            "registry_bytes": MAX_REGISTRY_TOTAL_BYTES,
            "bytes_per_entry": MAX_REGISTRY_TOTAL_BYTES / MAX_REGISTRY_ENTRIES as u64,
        }),
        M11BudgetWorkloadId::ConversationStatusPage => json!({
            "records": MAX_CONVERSATION_STATUS_RECORDS,
            "conversation_id_bytes": proto::MAX_SESSION_ID_BYTES,
            "latest_entry_id_bytes": proto::MAX_SESSION_ID_BYTES,
            "canonical_output_ceiling_bytes": MAX_CONVERSATION_STATUS_BYTES,
        }),
        M11BudgetWorkloadId::RunLogProjectionPage => json!({
            "records": MAX_CONVERSATION_STATUS_RECORDS,
            "canonical_output_bytes": MAX_CONVERSATION_STATUS_BYTES,
            "next_record_behind_cursor": true,
        }),
        M11BudgetWorkloadId::ConversationMigrationQuantum
        | M11BudgetWorkloadId::ConversationReplayQuantum => json!({
            "stored_input_bytes": MAX_CONVERSATION_SCAN_BYTES,
            "records": MAX_CONVERSATION_SCAN_BYTES / MAX_CONVERSATION_RECORD_BYTES as u64,
            "stored_bytes_per_record": MAX_CONVERSATION_RECORD_BYTES,
            "io_buffer_ceiling_bytes": MAX_CONVERSATION_IO_BUFFER_BYTES,
        }),
        M11BudgetWorkloadId::ConversationFullRunStreamingReplay => json!({
            "event_bytes": MAX_SESSION_EVENT_BYTES,
            "segments": EVENT_STREAM_LIMITS.max_segments,
            "segment_bytes": MAX_SESSION_SEGMENT_BYTES,
            "events": MAX_SESSION_EVENT_BYTES / MAX_CONVERSATION_RECORD_BYTES as u64,
            "canonical_bytes_per_event": MAX_CONVERSATION_RECORD_BYTES,
            "output": "callback-streamed canonical JSONL",
            "identity": "SHA-256",
        }),
        M11BudgetWorkloadId::ConversationHistoryValidationQuantum => json!({
            "stored_input_bytes": MAX_CONVERSATION_SCAN_BYTES,
            "records": conversations::HISTORY_VALIDATION_RECORDS,
            "entry_id_bytes": MAX_HISTORY_INDEX_ID_BYTES,
            "distinct_run_pointers": 1,
            "max_event_sequence": 1,
            "io_buffer_ceiling_bytes": MAX_CONVERSATION_IO_BUFFER_BYTES,
        }),
        M11BudgetWorkloadId::RunLogEightSyncAppends => json!({
            "records": conversations::SYNC_APPEND_RECORDS,
            "canonical_bytes_per_record": conversations::SYNC_APPEND_RECORD_BYTES,
            "canonical_synchronized_append_per_record": "append_jsonl",
            "replay_after_append": true,
        }),
    }
}

/// The exact finite set of optimized workloads approved for M1.1.
pub const M11_BUDGET_WORKLOADS: [M11BudgetWorkload; 15] = [
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::RssDetectionFixture,
        p95_limit: None,
        max_peak_rss_growth_bytes: None,
        min_peak_rss_growth_bytes: Some(RSS_FIXTURE_BYTES as u64 - RSS_ACCOUNTING_TOLERANCE_BYTES),
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::RunnerFourNoopLaunches,
        p95_limit: Some(Duration::from_millis(10)),
        max_peak_rss_growth_bytes: None,
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::RunnerTermination,
        p95_limit: Some(Duration::from_millis(5)),
        max_peak_rss_growth_bytes: None,
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::RunnerCancellation,
        p95_limit: Some(Duration::from_millis(5)),
        max_peak_rss_growth_bytes: None,
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::RunnerDualStreamCaps,
        p95_limit: Some(Duration::from_millis(100)),
        max_peak_rss_growth_bytes: Some(12 * MIB),
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::AuthoringMaxDefinitionTransaction,
        p95_limit: Some(Duration::from_millis(25)),
        max_peak_rss_growth_bytes: Some(4 * MIB),
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::AuthoringInit,
        p95_limit: Some(Duration::from_millis(50)),
        max_peak_rss_growth_bytes: Some(4 * MIB),
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::AuthoringMaxRegistryValidate,
        p95_limit: Some(Duration::from_secs(2)),
        max_peak_rss_growth_bytes: Some(32 * MIB),
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::ConversationStatusPage,
        p95_limit: Some(Duration::from_millis(100)),
        max_peak_rss_growth_bytes: Some(16 * MIB),
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::RunLogProjectionPage,
        p95_limit: Some(Duration::from_millis(100)),
        max_peak_rss_growth_bytes: Some(16 * MIB),
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::ConversationMigrationQuantum,
        p95_limit: Some(Duration::from_millis(1_250)),
        max_peak_rss_growth_bytes: Some(64 * MIB),
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::ConversationReplayQuantum,
        p95_limit: Some(Duration::from_secs(1)),
        max_peak_rss_growth_bytes: Some(64 * MIB),
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::ConversationFullRunStreamingReplay,
        p95_limit: Some(Duration::from_secs(13)),
        max_peak_rss_growth_bytes: Some(256 * MIB),
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::ConversationHistoryValidationQuantum,
        p95_limit: Some(Duration::from_secs(1)),
        max_peak_rss_growth_bytes: Some(64 * MIB),
        min_peak_rss_growth_bytes: None,
    },
    M11BudgetWorkload {
        id: M11BudgetWorkloadId::RunLogEightSyncAppends,
        p95_limit: Some(Duration::from_millis(10)),
        max_peak_rss_growth_bytes: None,
        min_peak_rss_growth_bytes: None,
    },
];

/// One measured workload result returned to the benchmark-only report harness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct M11BudgetOutcome {
    /// Product work elapsed time; fixture preparation is excluded.
    pub elapsed: Duration,
    /// Fixed number of logical operations performed.
    pub operations: u64,
    /// Exact product input bytes represented by the fixture.
    pub input_bytes: u64,
    /// Exact canonical or raw output bytes produced by the workload.
    pub output_bytes: u64,
    /// Optimizer-resistant result checksum.
    pub checksum: u64,
}

/// Runs one exact M1.1 optimized workload in an otherwise empty temporary root.
pub fn run_m11_budget_workload(
    id: M11BudgetWorkloadId,
    temp_root: &Path,
    iteration: usize,
) -> Result<M11BudgetOutcome, String> {
    match id {
        M11BudgetWorkloadId::RssDetectionFixture => runner::rss_detection_fixture(),
        M11BudgetWorkloadId::RunnerFourNoopLaunches => runner::runner_four_noop_launches(temp_root),
        M11BudgetWorkloadId::RunnerTermination => runner::runner_termination(),
        M11BudgetWorkloadId::RunnerCancellation => runner::runner_cancellation(temp_root),
        M11BudgetWorkloadId::RunnerDualStreamCaps => runner::runner_dual_stream_caps(temp_root),
        M11BudgetWorkloadId::AuthoringMaxDefinitionTransaction => {
            authoring::authoring_max_definition_transaction(temp_root)
        }
        M11BudgetWorkloadId::AuthoringInit => authoring::authoring_init(temp_root),
        M11BudgetWorkloadId::AuthoringMaxRegistryValidate => {
            authoring::authoring_max_registry_validate(temp_root)
        }
        M11BudgetWorkloadId::ConversationStatusPage => {
            conversations::conversation_status_page_workload(temp_root)
        }
        M11BudgetWorkloadId::RunLogProjectionPage => {
            conversations::run_log_projection_page(temp_root)
        }
        M11BudgetWorkloadId::ConversationMigrationQuantum => {
            conversations::conversation_operation_quantum(
                temp_root,
                conversations::QuantumKind::Migration,
            )
        }
        M11BudgetWorkloadId::ConversationReplayQuantum => {
            conversations::conversation_operation_quantum(
                temp_root,
                conversations::QuantumKind::Replay,
            )
        }
        M11BudgetWorkloadId::ConversationFullRunStreamingReplay => {
            conversations::conversation_full_run_streaming_replay(temp_root)
        }
        M11BudgetWorkloadId::ConversationHistoryValidationQuantum => {
            conversations::conversation_history_validation_quantum(temp_root)
        }
        M11BudgetWorkloadId::RunLogEightSyncAppends => {
            conversations::run_log_eight_sync_appends(temp_root, iteration)
        }
    }
}

fn outcome(
    elapsed: Duration,
    operations: u64,
    input_bytes: u64,
    output_bytes: u64,
    checksum: u64,
) -> M11BudgetOutcome {
    black_box(checksum);
    M11BudgetOutcome {
        elapsed,
        operations,
        input_bytes,
        output_bytes,
        checksum,
    }
}
