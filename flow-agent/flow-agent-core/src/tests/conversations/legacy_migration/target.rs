use super::super::super::test_support::workspace_copy;
use crate::runtime::{
    conversations::{
        LegacyMigrationCrashPoint, migrate_legacy_session, reserve_conversation_continuation,
        set_legacy_migration_crash_point,
    },
    session::run_flow,
    session_reading::replay_conversation_run,
    types::EmitMode,
};
use std::fs;

#[test]
fn published_migration_rejects_a_missing_derived_tool_observation() {
    let workspace = workspace_copy("smoke-flow");
    let original =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy fixture run completes");
    set_legacy_migration_crash_point(LegacyMigrationCrashPoint::TargetPublished);
    migrate_legacy_session(&workspace, &original.session_id)
        .expect_err("migration stops after publishing its target");

    let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
    let run_log = sessions
        .join(&original.session_id)
        .join("runs")
        .join(&original.session_id)
        .join("run-log.jsonl");
    let records = fs::read_to_string(&run_log).expect("published run log reads");
    let (definition, observations) = records
        .split_once('\n')
        .expect("published run log has a definition record");
    assert!(!observations.is_empty(), "fixture has a tool observation");
    fs::write(&run_log, format!("{definition}\n")).expect("tool observations are removed");

    let error = migrate_legacy_session(&workspace, &original.session_id)
        .expect_err("migration rejects its damaged published target");
    assert!(
        error
            .to_string()
            .contains("migrated run log observations do not match its event stream"),
        "unexpected migration error: {error}"
    );
    assert!(
        sessions
            .join(format!("{}.jsonl", original.session_id))
            .is_file(),
        "validation failure retains the legacy event source"
    );
    assert!(
        sessions
            .join(".migrations")
            .join(format!("{}.json", original.session_id))
            .is_file(),
        "validation failure retains the migration transaction"
    );
}
#[test]
fn legacy_continuation_accepts_multiple_validated_event_segments() {
    let workspace = workspace_copy("smoke-flow");
    let original =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy fixture run completes");
    let split = original
        .stdout
        .match_indices('\n')
        .nth(4)
        .map(|(index, _)| index + 1)
        .expect("legacy fixture has enough complete records to split");
    let first = &original.stdout[..split];
    let second = &original.stdout[split..];
    assert!(!first.is_empty() && !second.is_empty());
    let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
    fs::write(
        sessions.join(format!("{}.jsonl", original.session_id)),
        first,
    )
    .expect("first legacy event segment writes");
    fs::write(
        sessions.join(format!("{}.000002.jsonl", original.session_id)),
        second,
    )
    .expect("second legacy event segment writes");

    migrate_legacy_session(&workspace, &original.session_id).expect("legacy bundle migrates");
    let reservation = reserve_conversation_continuation(&workspace, &original.session_id, None)
        .expect("migrated conversation reserves for continuation");
    reservation
        .release()
        .expect("continuation reservation releases");
}

#[test]
fn m1_legacy_bundle_migrates_and_replays_without_rewriting_events() {
    const M1_LEGACY_EVENTS: &str = concat!(
        r#"{"event_id":"evt-001","event_type":"session.started","payload":{"reason":"fixture-start"},"protocol_version":"0","sequence":1,"session_id":"smoke-flow","source":"flow-agent-cli","timestamp":"2026-01-01T00:00:00Z"}"#,
        "\n",
        r#"{"event_id":"evt-002","event_type":"flow.started","flow_id":"flow-001","payload":{"flow_definition_id":"smoke-flow","flow_name":"SmokeFlow"},"protocol_version":"0","sequence":2,"session_id":"smoke-flow","source":"flow-agent-cli","timestamp":"2026-01-01T00:00:01Z"}"#,
        "\n",
        r#"{"event_id":"evt-003","event_type":"phase.entered","flow_id":"flow-001","payload":{"instruction_ids":["say-smoke"],"phase_id":"smoke","phase_name":"Smoke","tool_ids":["echo"]},"protocol_version":"0","sequence":3,"session_id":"smoke-flow","source":"flow-agent-cli","timestamp":"2026-01-01T00:00:02Z"}"#,
        "\n",
        r#"{"event_id":"evt-004","event_type":"step.started","flow_id":"flow-001","payload":{"phase_id":"smoke","step_id":"say","step_name":"Say"},"protocol_version":"0","sequence":4,"session_id":"smoke-flow","source":"flow-agent-cli","timestamp":"2026-01-01T00:00:03Z"}"#,
        "\n",
        r#"{"event_id":"evt-005","event_type":"message.delta","flow_id":"flow-001","payload":{"content_delta":"smoke","message_id":"msg-001","role":"assistant"},"protocol_version":"0","sequence":5,"session_id":"smoke-flow","source":"flow-agent-cli","timestamp":"2026-01-01T00:00:04Z"}"#,
        "\n",
        r#"{"event_id":"evt-006","event_type":"message.completed","flow_id":"flow-001","payload":{"message_id":"msg-001","role":"assistant"},"protocol_version":"0","sequence":6,"session_id":"smoke-flow","source":"flow-agent-cli","timestamp":"2026-01-01T00:00:05Z"}"#,
        "\n",
        r#"{"event_id":"evt-007","event_type":"tool.started","flow_id":"flow-001","payload":{"allowed_parameters":[],"network_access":"deny","read_scope":["workspace"],"tool_id":"echo","tool_kind":"predefined-command","tool_name":"Echo","write_scope":[]},"protocol_version":"0","sequence":7,"session_id":"smoke-flow","source":"flow-agent-cli","timestamp":"2026-01-01T00:00:06Z"}"#,
        "\n",
        r#"{"event_id":"evt-008","event_type":"tool.completed","flow_id":"flow-001","payload":{"exit_code":0,"tool_id":"echo"},"protocol_version":"0","sequence":8,"session_id":"smoke-flow","source":"flow-agent-cli","timestamp":"2026-01-01T00:00:07Z"}"#,
        "\n",
        r#"{"event_id":"evt-009","event_type":"step.completed","flow_id":"flow-001","payload":{"phase_id":"smoke","step_id":"say","step_name":"Say"},"protocol_version":"0","sequence":9,"session_id":"smoke-flow","source":"flow-agent-cli","timestamp":"2026-01-01T00:00:08Z"}"#,
        "\n",
        r#"{"event_id":"evt-010","event_type":"flow.completed","flow_id":"flow-001","payload":{"flow_definition_id":"smoke-flow","flow_name":"SmokeFlow"},"protocol_version":"0","sequence":10,"session_id":"smoke-flow","source":"flow-agent-cli","timestamp":"2026-01-01T00:00:09Z"}"#,
        "\n",
        r#"{"event_id":"evt-011","event_type":"session.completed","payload":{},"protocol_version":"0","sequence":11,"session_id":"smoke-flow","source":"flow-agent-cli","timestamp":"2026-01-01T00:00:10Z"}"#,
        "\n",
    );
    let workspace = workspace_copy("smoke-flow");
    let original =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy fixture run completes");
    assert_eq!(original.session_id, "smoke-flow");
    fs::write(
        crate::tests::helpers::workspace_session_dir(&workspace).join("smoke-flow.jsonl"),
        M1_LEGACY_EVENTS,
    )
    .expect("M1 legacy event bytes replace the generated stream");

    migrate_legacy_session(&workspace, "smoke-flow").expect("M1 legacy bundle migrates");
    let migrated = replay_conversation_run(&workspace, "smoke-flow", "smoke-flow", EmitMode::Jsonl)
        .expect("M1 legacy run replays");

    assert_eq!(migrated.stdout, M1_LEGACY_EVENTS);
    let reservation = reserve_conversation_continuation(&workspace, "smoke-flow", None)
        .expect("M1 legacy conversation reserves for continuation");
    reservation
        .release()
        .expect("M1 legacy continuation reservation releases");
}
