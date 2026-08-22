use super::super::super::{
    helpers::{empty_workspace, session_event_line},
    test_support::workspace_copy,
};
use crate::runtime::{
    conversations::{
        LegacyEventScanPoint, legacy_session_is_terminal, migrate_legacy_session,
        set_legacy_event_scan_observer,
    },
    session::run_flow,
    session_authority::{SessionOwnershipLease, session_ownership_is_active},
    session_reading::replay_conversation_run,
    types::{EmitMode, RuntimeError},
};
use proto::EventType;
use std::{fs, path::Path};
#[test]
fn complete_legacy_bundle_migrates_once_to_the_conversation_tree() {
    let workspace = workspace_copy("smoke-flow");
    let original =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy fixture run completes");

    migrate_legacy_session(&workspace, &original.session_id).expect("legacy bundle migrates");
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join(format!("{}.jsonl", original.session_id))
            .exists()
    );
    let migrated = replay_conversation_run(
        &workspace,
        &original.session_id,
        &original.session_id,
        EmitMode::Jsonl,
    )
    .expect("migrated run replays");
    assert_eq!(migrated.stdout, original.stdout);

    migrate_legacy_session(&workspace, &original.session_id)
        .expect("completed migration recovery is idempotent");
}

#[test]
fn legacy_migration_releases_each_partial_lease_chain() {
    let workspace = workspace_copy("smoke-flow");
    let original =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy fixture run completes");
    let session_id = original.session_id;
    let marker =
        crate::tests::helpers::workspace_session_dir(&workspace).join(format!("{session_id}.lock"));
    let conversation_key = format!("conversation:{session_id}");
    let run_key = format!("run:{session_id}:{session_id}");

    let conversation = SessionOwnershipLease::acquire(&workspace, &conversation_key, &marker)
        .expect("Conversation contention fixture acquires");
    let error = migrate_legacy_session(&workspace, &session_id)
        .expect_err("migration cannot pass an active Conversation lease");
    assert!(matches!(error, RuntimeError::ActiveSession { .. }));
    assert!(!session_ownership_is_active(&workspace, &session_id).expect("legacy lease inspects"));
    conversation
        .release()
        .expect("Conversation fixture releases");

    let run = SessionOwnershipLease::acquire(&workspace, &run_key, &marker)
        .expect("Run contention fixture acquires");
    let error = migrate_legacy_session(&workspace, &session_id)
        .expect_err("migration cannot pass an active Run lease");
    assert!(matches!(error, RuntimeError::ActiveSession { .. }));
    assert!(!session_ownership_is_active(&workspace, &session_id).expect("legacy lease inspects"));
    assert!(
        !session_ownership_is_active(&workspace, &conversation_key)
            .expect("Conversation lease inspects")
    );
    run.release().expect("Run fixture releases");

    migrate_legacy_session(&workspace, &session_id)
        .expect("migration succeeds after reverse-order lease cleanup");
}

#[test]
fn legacy_terminal_plan_and_target_validation_scan_events_incrementally() {
    let write_bundle = |workspace: &Path, session_id: &str, event_stream: &str| {
        let sessions = crate::tests::helpers::ensure_workspace_session_dir(workspace);
        let logs = crate::tests::helpers::ensure_workspace_log_dir(workspace);
        fs::write(sessions.join(format!("{session_id}.jsonl")), event_stream)
            .expect("legacy events write");
        fs::write(logs.join(format!("{session_id}.contexts.jsonl")), "")
            .expect("empty context stream writes");
        fs::write(
            logs.join(format!("{session_id}.log")),
            format!(
                "registry_hash=sha256:{}\nflow_definition_hash=sha256:{}\nflow_definition_id=budget-flow\n",
                "a".repeat(64),
                "b".repeat(64)
            ),
        )
        .expect("legacy metadata writes");
    };
    let stop_at = |point, message: &'static str| {
        set_legacy_event_scan_observer(point, move || Err(RuntimeError::Usage(message.to_owned())));
    };

    let terminal = empty_workspace("legacy-terminal-incremental");
    let terminal_id = "legacyterminalincremental001";
    let terminal_start =
        session_event_line(terminal_id, "evt-started", EventType::SessionStarted, 1);
    write_bundle(
        &terminal,
        terminal_id,
        &format!("{terminal_start}not-json\n"),
    );
    stop_at(LegacyEventScanPoint::TerminalCheck, "terminal stopped");
    let error = legacy_session_is_terminal(&terminal, terminal_id)
        .expect_err("terminal callback stops before the later invalid record");
    assert!(matches!(error, RuntimeError::Usage(message) if message == "terminal stopped"));

    let plan = empty_workspace("legacy-plan-incremental");
    let plan_id = "legacyplanincremental001";
    let plan_start = session_event_line(plan_id, "evt-started", EventType::SessionStarted, 1);
    write_bundle(&plan, plan_id, &format!("{plan_start}not-json\n"));
    stop_at(LegacyEventScanPoint::MigrationPlan, "plan stopped");
    let error = migrate_legacy_session(&plan, plan_id)
        .expect_err("plan callback stops before the later invalid record");
    assert!(matches!(error, RuntimeError::Usage(message) if message == "plan stopped"));

    let target = empty_workspace("legacy-target-incremental");
    let target_id = "legacytargetincremental001";
    let target_start = session_event_line(target_id, "evt-started", EventType::SessionStarted, 1);
    let target_completed =
        session_event_line(target_id, "evt-completed", EventType::SessionCompleted, 2);
    write_bundle(
        &target,
        target_id,
        &format!("{target_start}{target_completed}"),
    );
    stop_at(LegacyEventScanPoint::TargetValidation, "target stopped");
    let error = migrate_legacy_session(&target, target_id)
        .expect_err("target callback stops during segmentwise validation");
    assert!(matches!(error, RuntimeError::Usage(message) if message == "target stopped"));
}
