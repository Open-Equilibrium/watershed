use super::super::super::{
    helpers::{
        copy_workspace_runtime, empty_workspace, workspace_log_dir, workspace_session_dir,
        workspace_store_dir,
    },
    test_support::{copy_dir, workspace_copy},
};
use super::super::file_tree_bytes;
use crate::runtime::{
    conversations::{conversation_status_page, migrate_legacy_session},
    fixture_effects::{fixture_tool_apply_count, reset_fixture_tool_apply_count},
    live_events::live_event_channel,
    resume::{resume_session, resume_session_with_live_events},
    session::{resume_conversation_run, run_flow},
    session_reading::{SessionEventReader, replay_conversation_run},
    types::EmitMode,
};
use std::fs;
#[test]
fn incomplete_legacy_bundle_remains_readable_and_status_skips_migration() {
    let workspace = workspace_copy("smoke-flow");
    let original =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy fixture run completes");
    let prefix = original
        .stdout
        .lines()
        .next()
        .expect("fixture stream starts")
        .to_owned()
        + "\n";
    let legacy_path = workspace_session_dir(&workspace).join("smoke-flow.jsonl");
    fs::write(&legacy_path, &prefix).expect("legacy stream is made incomplete");

    let replayed = replay_conversation_run(&workspace, "smoke-flow", "smoke-flow", EmitMode::Jsonl)
        .expect("an incomplete legacy stream remains readable by its exact id pair");
    assert_eq!(replayed.stdout, prefix);
    assert!(legacy_path.is_file());
    assert!(
        !workspace_session_dir(&workspace)
            .join("smoke-flow/runs")
            .exists()
    );

    let page = conversation_status_page(&workspace, None)
        .expect("status does not reject or migrate an incomplete legacy stream");
    assert!(page.conversations.is_empty());
    assert!(legacy_path.is_file());
}

#[test]
fn incomplete_legacy_bundle_conflicts_with_published_conversation_before_access() {
    let workspace = workspace_copy("smoke-flow");
    let original =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy fixture run completes");
    let legacy_seed = empty_workspace("incomplete-legacy-authority-seed");
    copy_dir(&workspace, &legacy_seed);
    copy_workspace_runtime(&workspace, &legacy_seed);
    let incomplete_events = original
        .stdout
        .lines()
        .next()
        .expect("fixture stream starts")
        .to_owned()
        + "\n";
    fs::write(
        workspace_session_dir(&legacy_seed).join("smoke-flow.jsonl"),
        &incomplete_events,
    )
    .expect("legacy stream is made incomplete");

    migrate_legacy_session(&workspace, "smoke-flow").expect("legacy bundle migrates");
    for (source, target, events) in [
        (
            workspace_session_dir(&legacy_seed),
            workspace_session_dir(&workspace),
            true,
        ),
        (
            workspace_log_dir(&legacy_seed),
            workspace_log_dir(&workspace),
            false,
        ),
    ] {
        for entry in fs::read_dir(source).expect("legacy source inventory reads") {
            let entry = entry.expect("legacy source entry reads");
            let name = entry.file_name();
            let name_text = name.to_string_lossy();
            let is_numbered_event_segment = name_text
                .strip_prefix("smoke-flow.")
                .and_then(|suffix| suffix.strip_suffix(".jsonl"))
                .is_some_and(|ordinal| {
                    ordinal.len() == 6 && ordinal.bytes().all(|byte| byte.is_ascii_digit())
                });
            if !name_text.starts_with("smoke-flow.") || (events && is_numbered_event_segment) {
                continue;
            }
            fs::copy(entry.path(), target.join(name)).expect("legacy member is republished");
        }
    }
    let authority_before = file_tree_bytes(&workspace_store_dir(&workspace));
    reset_fixture_tool_apply_count();
    let (notifier, receiver) = live_event_channel();

    let conflicts = [
        (
            "single-ID event reader",
            SessionEventReader::open(&workspace, "smoke-flow")
                .map(drop)
                .expect_err("single-ID event reader rejects conflicting legacy authority"),
        ),
        (
            "single-ID Resume",
            resume_session(&workspace, "smoke-flow", EmitMode::Human)
                .map(drop)
                .expect_err("single-ID Resume rejects conflicting legacy authority"),
        ),
        (
            "single-ID live Resume",
            resume_session_with_live_events(&workspace, "smoke-flow", notifier)
                .map(drop)
                .expect_err("single-ID live Resume rejects conflicting legacy authority"),
        ),
        (
            "replay",
            replay_conversation_run(&workspace, "smoke-flow", "smoke-flow", EmitMode::Jsonl)
                .map(drop)
                .expect_err("replay rejects conflicting legacy authority"),
        ),
        (
            "Tail",
            SessionEventReader::open_conversation_run(&workspace, "smoke-flow", "smoke-flow")
                .map(drop)
                .expect_err("Tail open rejects conflicting legacy authority"),
        ),
        (
            "status",
            conversation_status_page(&workspace, None)
                .map(drop)
                .expect_err("status rejects conflicting legacy authority"),
        ),
        (
            "Resume",
            resume_conversation_run(&workspace, "smoke-flow", "smoke-flow", EmitMode::Human)
                .map(drop)
                .expect_err("Resume rejects conflicting legacy authority"),
        ),
    ];
    for (operation, error) in conflicts {
        assert!(
            error
                .to_string()
                .contains("conflicts with a later legacy-format bundle"),
            "{operation} returned the wrong conflict: {error}"
        );
    }
    assert_eq!(fixture_tool_apply_count(), 0, "Resume applied a Tool");
    assert_eq!(
        receiver.highest_committed_sequence(),
        0,
        "live Resume published an event"
    );
    assert_eq!(
        file_tree_bytes(&workspace_store_dir(&workspace)),
        authority_before,
        "rejected access leaves both authorities byte-identical"
    );
}
