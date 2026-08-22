use super::super::{
    helpers::empty_workspace,
    test_support::{self},
};
use super::{create_review_run, create_terminal_review_run};
use crate::runtime::{
    session_authority::SessionOwnershipLease,
    session_reading::{
        SessionEventReader, replay_conversation_run, replay_conversation_run_streaming,
    },
    stream_signature::{EVENT_PLAN_DOMAIN, RuntimeStreamSignatureBuilder},
    types::{EmitMode, MAX_CANONICAL_EVENT_BYTES, RuntimeError},
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
};

const IN_MEMORY_REPLAY_TEST_LIMIT_BYTES: usize = 67_108_864;

#[test]
fn human_conversation_replay_uses_run_terminology() {
    let workspace = empty_workspace("human-conversation-replay");
    create_terminal_review_run(&workspace);

    let output = replay_conversation_run(&workspace, "review", "review-1", EmitMode::Human)
        .expect("terminal conversation Run replays");

    assert_eq!(output.stdout, "run review-1 replayed\n");
}

#[test]
fn in_memory_replay_accepts_exact_output_limit_and_rejects_one_byte_over() {
    let workspace = empty_workspace("bounded-conversation-replay");
    let conversation_id = "boundedconversation001";
    let run_session_id = "boundedconversationrun001";
    test_support::write_sized_conversation_replay(
        &crate::tests::helpers::ensure_workspace_session_dir(&workspace),
        conversation_id,
        run_session_id,
        IN_MEMORY_REPLAY_TEST_LIMIT_BYTES,
        |_| {},
    );

    let exact =
        replay_conversation_run(&workspace, conversation_id, run_session_id, EmitMode::Jsonl)
            .expect("the exact in-memory replay limit succeeds");
    assert_eq!(exact.stdout.len(), IN_MEMORY_REPLAY_TEST_LIMIT_BYTES);

    test_support::write_sized_conversation_replay(
        &crate::tests::helpers::ensure_workspace_session_dir(&workspace),
        conversation_id,
        run_session_id,
        IN_MEMORY_REPLAY_TEST_LIMIT_BYTES + 1,
        |_| {},
    );
    let error =
        replay_conversation_run(&workspace, conversation_id, run_session_id, EmitMode::Jsonl)
            .expect_err("one byte beyond the in-memory replay limit is rejected");
    assert!(matches!(
        error,
        RuntimeError::ReplayOutputLimitExceeded {
            limit_bytes: IN_MEMORY_REPLAY_TEST_LIMIT_BYTES
        }
    ));
    let mut reader =
        SessionEventReader::open_conversation_run(&workspace, conversation_id, run_session_id)
            .expect("the oversized Run reader opens");
    assert!(matches!(
        reader.read_after(0),
        Err(RuntimeError::ReplayOutputLimitExceeded {
            limit_bytes: IN_MEMORY_REPLAY_TEST_LIMIT_BYTES
        })
    ));
}

#[test]
fn streaming_replay_emits_large_segmented_jsonl_without_returning_it() {
    let workspace = empty_workspace("streaming-conversation-replay");
    let conversation_id = "streamingconversation001";
    let run_session_id = "streamingconversationrun001";
    let total_bytes = IN_MEMORY_REPLAY_TEST_LIMIT_BYTES + 1;
    let mut expected = RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN);
    test_support::write_sized_conversation_replay(
        &crate::tests::helpers::ensure_workspace_session_dir(&workspace),
        conversation_id,
        run_session_id,
        total_bytes,
        |line| expected.push(line.as_bytes()),
    );
    let mut observed = RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN);
    let mut observed_bytes = 0usize;
    let mut largest_chunk = 0usize;

    let output =
        replay_conversation_run_streaming(&workspace, conversation_id, run_session_id, |line| {
            observed.push(line.as_bytes());
            observed_bytes += line.len();
            largest_chunk = largest_chunk.max(line.len());
            Ok(())
        })
        .expect("streaming replay accepts output above the in-memory limit");

    assert_eq!(observed_bytes, total_bytes);
    assert_eq!(observed.signature(), expected.signature());
    assert!(largest_chunk <= MAX_CANONICAL_EVENT_BYTES);
    assert_eq!(output.event_count, 256);
    assert!(!output.failed);
    assert!(output.stdout.is_empty());
}

#[test]
fn conversation_replay_rejects_inactive_and_malformed_event_boundaries() {
    for (label, bytes, expected) in [
        (
            "empty",
            Vec::new(),
            "is empty without active session ownership",
        ),
        (
            "partial",
            b"{}".to_vec(),
            "incomplete final JSONL line without active session ownership",
        ),
        ("invalid-utf8", vec![0xff, b'\n'], "is not valid UTF-8"),
    ] {
        let workspace = empty_workspace(&format!("conversation-replay-{label}"));
        create_review_run(&workspace);
        fs::write(
            crate::tests::helpers::workspace_session_dir(&workspace)
                .join("review/runs/review-1/events.jsonl"),
            bytes,
        )
        .expect("event boundary fixture writes");

        let error = replay_conversation_run(&workspace, "review", "review-1", EmitMode::Jsonl)
            .expect_err("malformed conversation replay must fail closed");
        assert!(error.to_string().contains(expected), "{label}: {error}");
    }

    let workspace = empty_workspace("conversation-replay-non-final-partial");
    create_review_run(&workspace);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    fs::write(run.join("events.jsonl"), b"{}").expect("partial non-final segment writes");
    fs::write(run.join("events.000002.jsonl"), b"{}\n").expect("complete final segment writes");
    let error = replay_conversation_run(&workspace, "review", "review-1", EmitMode::Jsonl)
        .expect_err("partial non-final segment must fail closed");
    assert!(
        error
            .to_string()
            .contains("non-final segment must end with LF")
    );

    let workspace = empty_workspace("conversation-replay-terminal-partial");
    create_terminal_review_run(&workspace);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    OpenOptions::new()
        .append(true)
        .open(run.join("events.jsonl"))
        .expect("terminal event stream opens")
        .write_all(b"partial")
        .expect("partial terminal suffix writes");
    let _ownership = SessionOwnershipLease::acquire(
        &workspace,
        "run:review:review-1",
        &run.join("replay-test.lock"),
    )
    .expect("conversation Run ownership is active");
    let error = replay_conversation_run(&workspace, "review", "review-1", EmitMode::Jsonl)
        .expect_err("a partial line after a terminal event must fail closed");
    assert!(
        error
            .to_string()
            .contains("partial line after a terminal event")
    );
}
