use super::{
    helpers::{
        assert_no_active_session_lock, empty_workspace, event_line, first_event_line,
        prefix_before_tool_started, prefix_through_tool_progress, prefix_through_tool_started,
        reserve_session_log, workspace_with_later_invalid_own_script_path,
        write_definition_hash_metadata,
    },
    support::{assert_active_session, assert_denied, write_initial_session_log},
    test_support::{expected_stream, stream_prefix, workspace_copy},
};
use crate::runtime::{
    fixture_effects::{fixture_tool_applied_ids, reset_fixture_tool_apply_count},
    fs_guards::{
        replacement_temp_path, segmented_jsonl_path, set_directory_sync_error_for_path_for_test,
    },
    resume::resume_session,
    resume_inspection::checked_resume_event_count,
    segmented_appender::{EventLogAppender, SessionLogAppender},
    session::run_flow,
    types::{EmitMode, MAX_FLOW_EVENTS, RuntimeError, human_failure_status},
    validate::{stream_is_completed, validate_session_log_text},
};
use proto::{EventEnvelope, EventType};
use std::{fs, io};

#[test]
fn resume_event_capacity_counts_prior_markers_and_the_new_marker() {
    let max = usize::try_from(MAX_FLOW_EVENTS).expect("event limit fits usize");

    assert_eq!(
        checked_resume_event_count(max - 2, 1).expect("exact limit is accepted"),
        max
    );
    let err = checked_resume_event_count(max - 1, 1)
        .expect_err("one event beyond the cumulative limit is rejected");
    assert!(err.to_string().contains("runtime event budget exceeded"));
}

#[test]
fn resume_resyncs_a_rotated_checkpoint_parent_before_accepting_the_prefix() {
    let workspace = workspace_copy("hello-flow");
    let reservation = reserve_session_log(&workspace, "hello-flow").expect("Run bundle reserves");
    let prefix = prefix_before_tool_started(
        &expected_stream("hello-flow", "hello-flow.jsonl"),
        "write-summary",
    );
    fs::write(reservation.session_path.diagnostic_path(), &prefix)
        .expect("complete resumable prefix writes");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");
    let last_record_start = prefix[..prefix.len() - 1]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let (base_prefix, rotated_checkpoint) = prefix.split_at(last_record_start);
    assert!(rotated_checkpoint.contains("\"event_type\":\"message.completed\""));
    fs::write(reservation.session_path.diagnostic_path(), base_prefix)
        .expect("base segment writes");
    let rotated =
        segmented_jsonl_path(&reservation.session_path, 2).expect("rotated segment path is valid");
    fs::write(rotated.diagnostic_path(), rotated_checkpoint).expect("rotated checkpoint writes");

    let mut appender =
        SessionLogAppender::open(&reservation.session_path).expect("rotated event appender opens");
    set_directory_sync_error_for_path_for_test(
        &reservation.session_path.parent.path,
        io::ErrorKind::Other,
    );
    let first_error = appender
        .sync(reservation.session_path.diagnostic_path())
        .expect_err("checkpoint parent synchronization fails");
    assert!(
        first_error
            .to_string()
            .contains("injected directory synchronization failure"),
        "{first_error}"
    );
    drop(appender);
    reservation.activate().expect("session activates");
    reservation
        .release_lock()
        .expect("session ownership releases");
    let base_before =
        fs::read(reservation.session_path.diagnostic_path()).expect("base prefix remains readable");
    let rotated_before =
        fs::read(rotated.diagnostic_path()).expect("rotated checkpoint remains readable");

    set_directory_sync_error_for_path_for_test(
        &reservation.session_path.parent.path,
        io::ErrorKind::Other,
    );
    let retry_error = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("Resume must re-synchronize the visible checkpoint before accepting it");
    assert!(
        retry_error
            .to_string()
            .contains("injected directory synchronization failure"),
        "{retry_error}"
    );
    assert_eq!(
        fs::read(reservation.session_path.diagnostic_path()).expect("base prefix remains readable"),
        base_before
    );
    assert_eq!(
        fs::read(rotated.diagnostic_path()).expect("rotated checkpoint remains readable"),
        rotated_before
    );
    assert!(!workspace.join("out/summary.txt").exists());

    let resumed = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("final retry synchronizes and resumes");
    assert!(!resumed.failed);
    let stream = format!(
        "{}{}",
        fs::read_to_string(reservation.session_path.diagnostic_path()).expect("base stream reads"),
        fs::read_to_string(rotated.diagnostic_path()).expect("rotated stream reads")
    );
    assert!(stream.starts_with(&prefix));
    assert_eq!(
        stream
            .lines()
            .filter(|line| {
                line.contains("\"event_type\":\"tool.completed\"")
                    && line.contains("\"tool_id\":\"write-summary\"")
            })
            .count(),
        1
    );
    assert!(workspace.join("out/summary.txt").is_file());
}

#[test]
fn resume_rejects_events_after_terminal_without_rewriting_log() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let path = session_dir.join("terminal-plus.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "terminal-plus",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "terminal-plus",
        2,
        "2026-01-01T00:00:01Z",
        "flow-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    let appended = EventEnvelope::new(
        "evt-003",
        EventType::SessionPaused,
        "terminal-plus",
        3,
        "2026-01-01T00:00:02Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"external-append"}),
    )
    .canonical_jsonl()
    .expect("appended event serializes");
    let before = format!("{started}{completed}{appended}");
    fs::write(&path, &before).expect("malformed terminal log written");

    let err = resume_session(&workspace, "terminal-plus", EmitMode::Jsonl)
        .expect_err("terminal-plus log must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("after terminal")));
    assert_eq!(
        fs::read_to_string(&path).expect("malformed terminal log remains readable"),
        before
    );
}

#[test]
fn resume_rejects_placeholder_prefix_without_rerunning_tool() {
    let workspace = workspace_copy("hello-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "hello001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("event serializes");
    let path = session_dir.join("hello001.jsonl");
    fs::write(&path, &event).expect("partial log written");
    fs::create_dir_all(workspace.join("out")).expect("output dir created");
    fs::write(workspace.join("out/summary.txt"), "already-written\n")
        .expect("committed side effect written");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("placeholder prefix must fail closed");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing definition metadata")
    ));
    assert_eq!(
        fs::read_to_string(&path).expect("placeholder log remains readable"),
        event
    );
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary remains readable"),
        "already-written\n"
    );
}

#[test]
fn resume_recovers_session_started_only_crash_prefix_from_metadata() {
    let workspace = workspace_copy("smoke-flow");
    let completed =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("seed session completes");
    let prefix = completed
        .stdout
        .lines()
        .next()
        .map(|line| format!("{line}\n"))
        .expect("seed stream has session.started");
    fs::write(&completed.session_path, &prefix).expect("crash prefix replaces completed log");
    fs::write(
        crate::tests::helpers::workspace_log_dir(&workspace)
            .join(format!("{}.contexts.jsonl", completed.session_id)),
        "",
    )
    .expect("crash precedes the first context checkpoint");

    let resumed = resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
        .expect("definition metadata identifies the selected flow");

    assert!(
        resumed
            .stdout
            .contains("\"event_type\":\"session.resumed\"")
    );
    let stream = fs::read_to_string(&completed.session_path).expect("resumed log is readable");
    let events = validate_session_log_text(&completed.session_path, &completed.session_id, &stream)
        .expect("resumed crash prefix remains canonical");
    assert_eq!(events[0].event_type, EventType::SessionStarted);
    assert_eq!(
        events.last().map(|event| event.event_type),
        Some(EventType::SessionCompleted)
    );
}

#[test]
fn resume_rejects_active_session_lock_without_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let reservation = reserve_session_log(&workspace, "hello001").expect("reservation succeeds");
    write_initial_session_log(&reservation, "hello001").expect("initial log writes");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("active session must not resume concurrently");

    assert_active_session(err, "hello001", "hello001.lock");
    assert!(!workspace.join("out/summary.txt").exists());
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn resume_rejects_case_aliased_session_lock_without_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let reservation = reserve_session_log(&workspace, "hello001").expect("reservation succeeds");
    write_initial_session_log(&reservation, "hello001").expect("initial log writes");
    reservation.activate().expect("session marker activates");
    let alias = crate::tests::helpers::workspace_session_dir(&workspace).join("HELLO001.LOCK");
    fs::rename(reservation.lock_path.diagnostic_path(), &alias).expect("lock alias installed");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("a case-aliased lock must preserve active ownership");

    assert!(
        matches!(err, RuntimeError::ActiveSession { ref session_id, .. } if session_id == "hello001"),
        "{err}"
    );
    assert!(!workspace.join("out/summary.txt").exists());
    fs::rename(&alias, reservation.lock_path.diagnostic_path()).expect("canonical lock restored");
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn resume_does_not_rerun_tool_after_progress_prefix() {
    let workspace = workspace_copy("hello-flow");
    reset_fixture_tool_apply_count();
    let completed =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("initial run completes");
    let prefix = prefix_through_tool_progress(&completed.stdout, "write-summary");
    fs::write(&completed.session_path, &prefix).expect("progress prefix remains durable");
    write_definition_hash_metadata(&workspace, &completed.session_id, "hello-flow");
    let summary_path = workspace.join("out/summary.txt");
    assert_eq!(
        fs::read_to_string(&summary_path).expect("initial summary remains readable"),
        "hello\n"
    );
    fs::write(&summary_path, "sentinel\n").expect("sentinel summary replaces first output");
    assert_eq!(
        fixture_tool_applied_ids()
            .iter()
            .filter(|tool_id| tool_id.as_str() == "write-summary")
            .count(),
        1,
        "the initial write side effect must occur exactly once"
    );

    let output = resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
        .expect("session resumes after the durable progress checkpoint");

    assert_no_active_session_lock(&workspace, &completed.session_id);
    assert_eq!(
        fixture_tool_applied_ids()
            .iter()
            .filter(|tool_id| tool_id.as_str() == "write-summary")
            .count(),
        1,
        "resume must not apply write-summary after its durable progress checkpoint"
    );
    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert_eq!(
        fs::read_to_string(&summary_path).expect("sentinel summary remains readable"),
        "sentinel\n"
    );
    let resumed = fs::read_to_string(&completed.session_path).expect("resumed log readable");
    let events =
        validate_session_log_text(&completed.session_path, &completed.session_id, &resumed)
            .expect("resumed log remains valid");
    assert_eq!(
        events[prefix.lines().count()..]
            .iter()
            .filter(|event| {
                event
                    .payload
                    .get("tool_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("write-summary")
            })
            .map(|event| event.event_type)
            .collect::<Vec<_>>(),
        vec![EventType::ToolCompleted],
        "resume may append only the missing terminal event for write-summary"
    );
    assert!(stream_is_completed(&events));
}

#[cfg(any(unix, windows))]
#[test]
fn resume_rejects_hardlinked_session_log_before_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let outside = empty_workspace("outside-resume-hardlink-reject");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let event = first_event_line("hello-flow", "hello-flow.jsonl");
    let outside_target = outside.join("hello-flow.jsonl");
    fs::write(&outside_target, &event).expect("outside log written");
    let session_path = session_dir.join("hello-flow.jsonl");
    fs::hard_link(&outside_target, &session_path).expect("session hard link");

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("hard-linked session log must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("hard-linked")));
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside log readable"),
        event
    );
    assert!(!workspace.join("out/summary.txt").exists());
}

#[test]
fn resume_rejects_tool_started_prefix_without_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let prefix = prefix_through_tool_started(
        &expected_stream("hello-flow", "hello-flow.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-flow.jsonl");
    fs::write(&path, &prefix).expect("started prefix written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("tool.started prefix is ambiguous and must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("in-flight tool")));
    assert!(!workspace.join("out/summary.txt").exists());
}

#[test]
fn resume_commits_resume_marker_before_apply_side_effects_fail() {
    let workspace = workspace_copy("hello-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let prefix = prefix_before_tool_started(
        &expected_stream("hello-flow", "hello-flow.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-flow.jsonl");
    fs::write(&path, &prefix).expect("prefix written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");

    let summary_path = workspace.join("out/summary.txt");
    for attempt in 0..100 {
        let temp_path =
            replacement_temp_path(&summary_path, attempt).expect("replacement temp path is valid");
        fs::write(temp_path, b"collision").expect("replacement temp collision written");
    }

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("apply-time side effect failure must fail the resume");

    assert_no_active_session_lock(&workspace, "hello-flow");
    let RuntimeError::SessionFailed { session_id, source } = err else {
        panic!("expected identified session failure, got {err:?}");
    };
    assert_eq!(session_id, "hello-flow");
    assert_denied(
        *source,
        core_policy::DenyReasonCode::WriteDenied,
        "temporary replacement path",
    );
    assert!(!summary_path.exists());
    let resumed = fs::read_to_string(&path).expect("resume marker log readable");
    assert!(resumed.starts_with(&prefix));
    assert!(resumed.contains("\"event_type\":\"session.resumed\""));
    assert!(!resumed.lines().any(|line| {
        line.contains("\"event_type\":\"tool.completed\"")
            && line.contains("\"tool_id\":\"write-summary\"")
    }));
    assert!(!resumed.contains("\"event_type\":\"session.completed\""));
    let events =
        validate_session_log_text(&path, "hello-flow", &resumed).expect("marker log remains valid");
    let denial = core_policy::DenyReasonCode::WriteDenied.as_str();
    for (event_type, field) in [
        (EventType::Error, "code"),
        (EventType::FlowFailed, "error"),
        (EventType::SessionFailed, "reason"),
    ] {
        assert!(events.iter().any(|event| {
            event.event_type == event_type
                && event.payload.get(field).and_then(serde_json::Value::as_str) == Some(denial)
        }));
    }
    assert_eq!(
        human_failure_status(&events).as_deref(),
        Some("failed (write_denied): write outside declared roots denied")
    );
}

#[test]
fn resume_retries_prior_resume_marker_tail_without_duplicate_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let prefix = prefix_before_tool_started(
        &expected_stream("hello-flow", "hello-flow.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-flow.jsonl");
    let event_count = prefix.lines().count();
    let resume_sequence = event_count as u64 + 1;
    let resume_marker = event_line(
        &format!("evt-{resume_sequence:03}"),
        EventType::SessionResumed,
        "hello-flow",
        resume_sequence,
        None,
        serde_json::json!({"reason":"resume"}),
    );
    let before = format!("{prefix}{resume_marker}");
    fs::write(&path, &before).expect("prior resume marker written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");

    let output = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("marker-only resume tail retries from the durable prefix");

    assert!(!output.failed);
    let resumed = fs::read_to_string(&path).expect("resumed log remains readable");
    let events = validate_session_log_text(&path, "hello-flow", &resumed)
        .expect("resumed log remains valid");
    assert_eq!(output.event_count, events.len());
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::SessionResumed)
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                event.event_type == EventType::ToolStarted
                    && event
                        .payload
                        .get("tool_id")
                        .and_then(serde_json::Value::as_str)
                        == Some("write-summary")
            })
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary written once"),
        "hello\n"
    );
    assert!(stream_is_completed(&events));
}

#[test]
fn resume_preflights_later_own_script_path_before_earlier_side_effects() {
    let workspace = workspace_with_later_invalid_own_script_path();
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let path = session_dir.join("hello-flow.jsonl");
    let prefix = stream_prefix(&expected_stream("hello-flow", "hello-flow.jsonl"), 2);
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("later invalid own-script path must reject before earlier write");

    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "must be a file",
    );
    assert!(!workspace.join("out/partial.txt").exists());
    assert_eq!(
        fs::read_to_string(&path).expect("unchanged log readable"),
        prefix
    );
}

#[cfg(not(any(unix, windows)))]
#[test]
fn resume_replaces_hardlinked_session_log_when_link_count_unverified() {
    let workspace = workspace_copy("smoke-flow");
    let outside = empty_workspace("outside-resume-hardlink");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let prefix = stream_prefix(&expected_stream("smoke-flow", "smoke-flow.jsonl"), 2);
    let outside_target = outside.join("smoke-flow.jsonl");
    fs::write(&outside_target, &prefix).expect("outside log written");
    let session_path = session_dir.join("smoke-flow.jsonl");
    fs::hard_link(&outside_target, &session_path).expect("session hard link");
    write_definition_hash_metadata(&workspace, "smoke-flow", "smoke-flow");

    let output =
        resume_session(&workspace, "smoke-flow", EmitMode::Jsonl).expect("session resumes");

    assert!(output.event_count > 2);
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        prefix
    );
    assert!(
        fs::read_to_string(&session_path)
            .expect("workspace session log readable")
            .contains("\"event_type\":\"session.completed\"")
    );
}

#[test]
fn resume_rejects_noncanonical_resume_marker_without_rewriting_log() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let mut prefix = stream_prefix(&expected_stream("smoke-flow", "smoke-flow.jsonl"), 2);
    prefix.push_str(&event_line(
        "evt-016",
        EventType::SessionResumed,
        "smoke-flow",
        3,
        None,
        serde_json::json!({"reason":"resume"}),
    ));
    let path = session_dir.join("smoke-flow.jsonl");
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "smoke-flow", "smoke-flow");

    let err = resume_session(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("noncanonical resume marker must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("valid prefix")));
    assert_eq!(
        fs::read_to_string(&path).expect("session log readable"),
        prefix
    );
}
