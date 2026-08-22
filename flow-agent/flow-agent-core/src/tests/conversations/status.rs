mod run_status_mutation;
mod summary;
mod transaction;

use super::super::helpers::session_event_line;
use proto::EventType;
use std::{fs, path::Path};

fn commit_review_event(workspace: &Path) {
    fs::write(
        crate::tests::helpers::workspace_session_dir(workspace)
            .join("review/runs/review-1/events.jsonl"),
        session_event_line("review-1", "evt-001", EventType::SessionStarted, 1),
    )
    .expect("review event commits");
}
