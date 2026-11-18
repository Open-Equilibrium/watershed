mod attempt_reconciliation;
mod budgets;
mod context_pairing;
mod event_prefix;
mod productive_replay;
mod reservation;
mod selection;

use super::REQUEST_HASH;
use crate::runtime::{
    conversations::{
        ConversationEventWriter, ProductiveRecoveryWriter, append_run_attempt_intent,
        append_run_attempt_result,
    },
    run_attempts::{
        RunAttemptIntent, RunAttemptKind, RunAttemptResult, ToolEnforcementExpectation,
    },
    session_reading::SessionEventReader,
};
use std::path::{Path, PathBuf};

fn review_run_dir(workspace: &Path, run_session_id: &str) -> PathBuf {
    crate::tests::helpers::workspace_session_dir(workspace)
        .join("review/runs")
        .join(run_session_id)
}

fn provider_intent(attempt_id: &str) -> RunAttemptIntent {
    RunAttemptIntent {
        attempt_id: attempt_id.to_owned(),
        attempt_kind: RunAttemptKind::Provider,
        expected_enforcement: None,
        request_hash: REQUEST_HASH.to_owned(),
        tool_id: None,
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
    }
}

fn tool_intent(attempt_id: &str, tool_id: &str) -> RunAttemptIntent {
    RunAttemptIntent {
        attempt_id: attempt_id.to_owned(),
        attempt_kind: RunAttemptKind::Tool,
        expected_enforcement: Some(ToolEnforcementExpectation {
            applied_policy_digest: "0".repeat(64),
        }),
        request_hash: REQUEST_HASH.to_owned(),
        tool_id: Some(tool_id.to_owned()),
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
    }
}

fn commit_intent(workspace: &Path, intent: &RunAttemptIntent) {
    append_run_attempt_intent(workspace, "review", "review-1", intent)
        .expect("attempt intent commits");
}

fn commit_result(workspace: &Path, result: &RunAttemptResult) {
    append_run_attempt_result(workspace, "review", "review-1", result)
        .expect("attempt result commits");
}

fn open_recovery_snapshot(workspace: &Path) -> ProductiveRecoveryWriter {
    ProductiveRecoveryWriter::open_for_resume(workspace, "review", "review-1")
        .expect("recovery snapshot opens")
}

fn open_event_recovery(workspace: &Path) -> ConversationEventWriter {
    ConversationEventWriter::open_for_recovery(workspace, "review", "review-1", false, None)
        .expect("recovery writer opens")
}

fn catch_up_after(workspace: &Path, after: u64, highest: u64) -> String {
    let mut reader = SessionEventReader::open_conversation_run(workspace, "review", "review-1")
        .expect("final catch-up reader opens");
    let mut delivered = String::new();
    reader
        .visit_verified_after(after, highest, |_, line| {
            delivered.push_str(line);
            Ok(())
        })
        .expect("authoritative log verifies");
    delivered
}
