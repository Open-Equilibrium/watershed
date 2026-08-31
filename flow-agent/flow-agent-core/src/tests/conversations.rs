use crate::runtime::{
    conversations::{
        ConversationEntry, ConversationEntryType, ConversationEventWriter,
        append_run_attempt_intent, create_conversation_run,
    },
    live_events::{LiveEventReceiver, live_event_channel},
    run_attempts::{RunAttemptIntent, RunAttemptKind},
};
use proto::{EventEnvelope, EventType};
use std::{
    collections::BTreeMap,
    fs::{self},
    path::{Path, PathBuf},
};

mod conversation_stream;
mod conversation_writer;
mod history_index;
mod history_scratch;
mod history_support;
mod lifecycle;
mod query;
mod recovery;
mod recovery_fixtures;
mod replay;
mod run_log;
mod run_objects;
mod session_event_stream;
mod status;
mod storage;

pub(super) use recovery_fixtures::write_terminal_recovery_snapshot;

const REGISTRY_HASH: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FLOW_HASH: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const REQUEST_HASH: &str =
    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn entry(id: &str, parent: Option<&str>, run_id: &str, sequence: u64) -> ConversationEntry {
    ConversationEntry {
        schema: "flow-conversation-entry-v1".to_owned(),
        entry_id: id.to_owned(),
        parent_entry_id: parent.map(str::to_owned),
        recovery_snapshot_hash: "d".repeat(64),
        run_session_id: run_id.to_owned(),
        event_sequence: sequence,
        entry_type: if parent.is_some() {
            ConversationEntryType::Continuation
        } else {
            ConversationEntryType::Checkpoint
        },
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
    }
}

fn create_review_run(workspace: &Path) {
    create_conversation_run(
        workspace,
        "review",
        "review-1",
        "review-flow",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("conversation run is created");
}

fn open_notified_review_writer(workspace: &Path) -> (ConversationEventWriter, LiveEventReceiver) {
    create_review_run(workspace);
    let (notifier, receiver) = live_event_channel();
    let writer = ConversationEventWriter::open_with_notifier(
        workspace,
        "review",
        "review-1",
        false,
        Some(notifier),
    )
    .expect("conversation writer opens");
    (writer, receiver)
}

fn create_terminal_review_run(workspace: &Path) {
    create_review_run(workspace);
    write_terminal_run(workspace, "review", "review-1");
}

fn append_uncertain_provider_intent(workspace: &Path) {
    append_run_attempt_intent(
        workspace,
        "review",
        "review-1",
        &RunAttemptIntent {
            attempt_id: "provider-001".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            request_hash: REQUEST_HASH.to_owned(),
            tool_id: None,
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
        },
    )
    .expect("uncertain provider intent is durable");
}

fn file_tree_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).expect("authority directory reads") {
            let entry = entry.expect("authority entry reads");
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).expect("authority metadata reads");
            assert!(!metadata.file_type().is_symlink(), "authority is link-free");
            if metadata.is_dir() {
                visit(root, &path, files);
            } else {
                assert!(
                    metadata.is_file(),
                    "authority entries are files or directories"
                );
                files.insert(
                    path.strip_prefix(root)
                        .expect("authority entry remains below its root")
                        .to_owned(),
                    fs::read(&path).expect("authority file reads"),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

pub(super) fn write_terminal_run(workspace: &Path, conversation_id: &str, run_session_id: &str) {
    let events = [
        EventEnvelope::new(
            "evt-001",
            EventType::SessionStarted,
            run_session_id,
            1,
            "2026-07-30T12:00:00Z",
            "flow-agent-cli",
            serde_json::json!({}),
        ),
        EventEnvelope::new(
            "evt-002",
            EventType::SessionCompleted,
            run_session_id,
            2,
            "2026-07-30T12:00:01Z",
            "flow-agent-cli",
            serde_json::json!({}),
        ),
    ]
    .into_iter()
    .map(|event| event.canonical_jsonl().expect("event serializes"))
    .collect::<String>();
    fs::write(
        crate::tests::helpers::workspace_session_dir(workspace)
            .join(conversation_id)
            .join("runs")
            .join(run_session_id)
            .join("events.jsonl"),
        events,
    )
    .expect("terminal event prefix writes");
}
