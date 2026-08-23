use super::super::super::{
    helpers::{create_directory_alias, empty_workspace, remove_directory_alias},
    test_support::{copy_dir, workspace_copy},
};
use super::super::file_tree_bytes;
use crate::runtime::{
    conversations::{
        LegacyMigrationControlFile, LegacyMigrationCrashPoint, MAX_CONVERSATION_IO_BUFFER_BYTES,
        conversation_status_page, measure_conversation_operation, migrate_legacy_session,
        set_legacy_migration_control_write_failure, set_legacy_migration_crash_point,
        set_legacy_migration_roots_observer, set_legacy_object_copy_observer,
    },
    digest::sha256_hex,
    fs_guards::{
        set_directory_sync_error_for_path_for_test, start_directory_sync_trace_for_test,
        take_directory_sync_trace_for_test,
    },
    session::run_flow,
    session_reading::replay_conversation_run,
    types::EmitMode,
};

#[test]
fn legacy_migration_stays_bound_to_its_anchored_runtime_roots() {
    let original = workspace_copy("smoke-flow");
    let legacy =
        run_flow(&original, "smoke-flow", EmitMode::Jsonl).expect("original legacy run completes");
    let replacement = empty_workspace("migration-root-replacement");
    copy_dir(&original, &replacement);
    crate::tests::helpers::copy_workspace_runtime(&original, &replacement);
    let replacement_before =
        file_tree_bytes(&crate::tests::helpers::workspace_store_dir(&replacement));

    let alias = empty_workspace("migration-root-alias");
    fs::remove_dir(&*alias).expect("workspace alias starts absent");
    create_directory_alias(&alias, &original);

    let alias_for_observer = alias.to_path_buf();
    let replacement_for_observer = replacement.to_path_buf();
    set_legacy_migration_roots_observer(move || {
        remove_directory_alias(&alias_for_observer);
        create_directory_alias(&alias_for_observer, &replacement_for_observer);
        Ok(())
    });

    migrate_legacy_session(&alias, &legacy.session_id)
        .expect("migration remains bound to its original runtime roots");
    assert!(
        crate::tests::helpers::workspace_session_dir(&original)
            .join(&legacy.session_id)
            .is_dir(),
        "the retained sessions directory receives the migrated conversation"
    );
    assert_eq!(
        file_tree_bytes(&crate::tests::helpers::workspace_store_dir(&replacement)),
        replacement_before,
        "the replacement runtime roots remain untouched"
    );

    remove_directory_alias(&alias);
    fs::create_dir(&*alias).expect("workspace alias cleanup root is restored");
}
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
};
#[test]
fn legacy_migration_recovers_every_durable_transaction_boundary() {
    for point in [
        LegacyMigrationCrashPoint::TransactionRecorded,
        LegacyMigrationCrashPoint::StagePopulated,
        LegacyMigrationCrashPoint::TargetPublished,
        LegacyMigrationCrashPoint::FirstSourceRetired,
        LegacyMigrationCrashPoint::BeforeTransactionCleared,
    ] {
        let workspace = workspace_copy("smoke-flow");
        let original =
            run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy run completes");
        set_legacy_migration_crash_point(point);
        let error = migrate_legacy_session(&workspace, &original.session_id)
            .expect_err("selected durable boundary interrupts migration");
        assert!(
            error
                .to_string()
                .contains("injected legacy migration crash"),
            "{point:?}: {error}"
        );
        assert!(
            crate::tests::helpers::workspace_session_dir(&workspace)
                .join(".migrations")
                .join(format!("{}.json", original.session_id))
                .is_file(),
            "{point:?}: the durable transaction remains"
        );

        migrate_legacy_session(&workspace, &original.session_id)
            .expect("the same migration recovers deterministically");
        let migrated = replay_conversation_run(
            &workspace,
            &original.session_id,
            &original.session_id,
            EmitMode::Jsonl,
        )
        .expect("recovered migration replays");
        assert_eq!(migrated.stdout, original.stdout, "{point:?}");
        assert!(
            !crate::tests::helpers::workspace_session_dir(&workspace)
                .join(".migrations")
                .join(format!("{}.json", original.session_id))
                .exists(),
            "{point:?}: the recovered transaction is cleared"
        );
        assert!(
            fs::read_dir(crate::tests::helpers::workspace_session_dir(&workspace))
                .expect("session inventory")
                .filter_map(Result::ok)
                .filter_map(|entry| entry.file_name().into_string().ok())
                .all(|name| !name.starts_with(".migration-") || name == ".migrations"),
            "{point:?}: no staging directory remains"
        );
        let page = conversation_status_page(&workspace, None).expect("status after recovery");
        assert_eq!(page.conversations.len(), 1, "{point:?}");
    }
}

#[test]
fn legacy_migration_atomically_recovers_partial_control_file_writes() {
    for (label, control_file) in [
        (
            "migration-partial-transaction",
            LegacyMigrationControlFile::Transaction,
        ),
        (
            "migration-partial-identity-marker",
            LegacyMigrationControlFile::IdentityMarker,
        ),
    ] {
        let workspace = workspace_copy("smoke-flow");
        let original =
            run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy run completes");
        let legacy_before =
            file_tree_bytes(&crate::tests::helpers::workspace_store_dir(&workspace));
        set_legacy_migration_control_write_failure(control_file);

        let error = migrate_legacy_session(&workspace, &original.session_id)
            .expect_err("the selected partial control-file write fails");
        assert!(
            error.to_string().contains("injected migration"),
            "{label}: {error}"
        );
        let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
        let transaction = sessions
            .join(".migrations")
            .join(format!("{}.json", original.session_id));
        match control_file {
            LegacyMigrationControlFile::Transaction => {
                assert!(
                    !transaction.exists(),
                    "{label}: no partial final transaction is visible"
                );
                assert_eq!(
                    fs::metadata(
                        sessions
                            .join(".migrations")
                            .join(format!(".{}.json.staged", original.session_id)),
                    )
                    .expect("partial staged transaction has metadata")
                    .len(),
                    1,
                    "{label}: the bounded staged write is inventory-visible"
                );
            }
            LegacyMigrationControlFile::IdentityMarker => {
                let transaction_json: serde_json::Value = serde_json::from_slice(
                    &fs::read(&transaction).expect("published transaction reads"),
                )
                .expect("published transaction remains valid");
                let stage = sessions.join(
                    transaction_json["staging_name"]
                        .as_str()
                        .expect("transaction names its stage"),
                );
                assert!(
                    !stage.join(".migration-identity").exists(),
                    "{label}: no partial final identity marker is visible"
                );
                assert_eq!(
                    fs::metadata(stage.join(".migration-identity.staged"))
                        .expect("partial staged identity marker has metadata")
                        .len(),
                    1,
                    "{label}: the bounded staged write is inventory-visible"
                );
            }
        }
        assert_eq!(
            fs::read(sessions.join(format!("{}.jsonl", original.session_id)))
                .expect("legacy event source remains readable"),
            legacy_before
                .get(&Path::new("sessions").join(format!("{}.jsonl", original.session_id)))
                .expect("legacy event source was captured")
                .as_slice(),
            "{label}: source remains unchanged until publication"
        );

        migrate_legacy_session(&workspace, &original.session_id)
            .expect("retry removes the bounded staged artifact and completes");
        assert_eq!(
            replay_conversation_run(
                &workspace,
                &original.session_id,
                &original.session_id,
                EmitMode::Jsonl,
            )
            .expect("recovered migration replays")
            .stdout,
            original.stdout,
            "{label}"
        );
        assert!(
            fs::read_dir(sessions.join(".migrations"))
                .expect("migration transaction directory reads")
                .next()
                .is_none(),
            "{label}: no transaction artifact remains"
        );
        assert!(
            fs::read_dir(&sessions)
                .expect("session inventory reads")
                .filter_map(Result::ok)
                .filter_map(|entry| entry.file_name().into_string().ok())
                .all(|name| !name.starts_with(".migration-") || name == ".migrations"),
            "{label}: no migration staging artifact remains"
        );
    }
}

#[test]
fn legacy_migration_rejects_a_changed_object_before_target_publication() {
    let workspace = workspace_copy("smoke-flow");
    let original =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy run completes");
    let original_bytes = b"original legacy object";
    let digest = sha256_hex(original_bytes);
    let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
    let object = sessions.join(format!("{}.object.sha256-{digest}", original.session_id));
    fs::write(&object, original_bytes).expect("legacy object writes");
    let object_for_observer = object.clone();
    set_legacy_object_copy_observer(move || {
        fs::write(&object_for_observer, b"changed legacy object").map_err(|source| {
            crate::runtime::fs_guards::path_io_error(&object_for_observer, source)
        })
    });

    let error = migrate_legacy_session(&workspace, &original.session_id)
        .expect_err("changed object rejects migration");
    assert!(error.to_string().contains("object"), "{error}");
    assert!(
        !sessions.join(&original.session_id).exists(),
        "an invalid canonical target must not be published"
    );

    fs::write(&object, original_bytes).expect("legacy object restores");
    migrate_legacy_session(&workspace, &original.session_id)
        .expect("retry rebuilds the recoverable stage and completes");
    assert_eq!(
        fs::read(
            sessions
                .join(&original.session_id)
                .join("runs")
                .join(&original.session_id)
                .join("objects")
                .join(&digest)
        )
        .expect("migrated object reads"),
        original_bytes
    );
}

#[test]
fn conversation_status_recovers_an_interrupted_legacy_migration() {
    let workspace = workspace_copy("smoke-flow");
    let original =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy run completes");
    set_legacy_migration_crash_point(LegacyMigrationCrashPoint::TransactionRecorded);
    migrate_legacy_session(&workspace, &original.session_id)
        .expect_err("migration stops after recording its durable transaction");
    let transaction = crate::tests::helpers::workspace_session_dir(&workspace)
        .join(".migrations")
        .join(format!("{}.json", original.session_id));
    assert!(transaction.is_file());

    let page = conversation_status_page(&workspace, None)
        .expect("status recovers the interrupted migration");
    assert_eq!(page.conversations.len(), 1);
    assert_eq!(page.conversations[0].conversation_id, original.session_id);
    assert!(!transaction.exists());
    assert_eq!(
        replay_conversation_run(
            &workspace,
            &original.session_id,
            &original.session_id,
            EmitMode::Jsonl,
        )
        .expect("status-recovered run replays")
        .stdout,
        original.stdout
    );
}

#[test]
fn legacy_migration_recovers_only_authorized_stage_cleanup_tails() {
    for (label, remove_non_marker, remove_marker) in [
        ("migration-cleanup-partial-with-marker", true, false),
        ("migration-cleanup-empty-without-marker", true, true),
    ] {
        let workspace = workspace_copy("smoke-flow");
        let original =
            run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy run completes");
        set_legacy_migration_crash_point(LegacyMigrationCrashPoint::StagePopulated);
        migrate_legacy_session(&workspace, &original.session_id)
            .expect_err("migration stops with a complete authenticated stage");
        let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
        let transaction: serde_json::Value = serde_json::from_slice(
            &fs::read(
                sessions
                    .join(".migrations")
                    .join(format!("{}.json", original.session_id)),
            )
            .expect("migration transaction reads"),
        )
        .expect("migration transaction parses");
        let stage = sessions.join(
            transaction["staging_name"]
                .as_str()
                .expect("transaction names its stage"),
        );
        let marker = stage.join(".migration-identity");

        if remove_non_marker && !remove_marker {
            fs::remove_file(stage.join("history.jsonl"))
                .expect("one non-marker cleanup mutation succeeds");
        } else if remove_non_marker {
            for entry in fs::read_dir(&stage).expect("migration stage reads") {
                let entry = entry.expect("migration stage entry reads");
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).expect("stage metadata reads");
                if metadata.is_dir() {
                    fs::remove_dir_all(&path).expect("stage child directory removes");
                } else {
                    fs::remove_file(&path).expect("stage child file removes");
                }
            }
        }
        assert_eq!(marker.exists(), !remove_marker, "{label}");
        assert!(stage.is_dir(), "{label}: stage root remains");

        start_directory_sync_trace_for_test();
        migrate_legacy_session(&workspace, &original.session_id)
            .expect("the same transaction finishes authorized cleanup and migration");
        let trace = take_directory_sync_trace_for_test();
        let sessions_key = crate::tests::helpers::canonical_test_path(&sessions);
        assert!(
            trace.iter().filter(|path| *path == &sessions_key).count() >= 2,
            "{label}: cleanup and publication must synchronize sessions: {trace:?}"
        );
        assert!(!stage.exists(), "{label}: old migration stage is gone");
        assert_eq!(
            replay_conversation_run(
                &workspace,
                &original.session_id,
                &original.session_id,
                EmitMode::Jsonl,
            )
            .expect("recovered migration replays")
            .stdout,
            original.stdout,
            "{label}"
        );
    }
}

#[test]
fn legacy_migration_stage_cleanup_rejects_unowned_shapes_without_mutation() {
    for shape in [
        "unknown-file",
        "known-file-as-directory",
        "markerless-nonempty",
    ] {
        let workspace = workspace_copy("smoke-flow");
        let original =
            run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy run completes");
        set_legacy_migration_crash_point(LegacyMigrationCrashPoint::StagePopulated);
        migrate_legacy_session(&workspace, &original.session_id)
            .expect_err("migration stops with a complete authenticated stage");
        let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
        let transaction_path = sessions
            .join(".migrations")
            .join(format!("{}.json", original.session_id));
        let transaction: serde_json::Value = serde_json::from_slice(
            &fs::read(&transaction_path).expect("migration transaction reads"),
        )
        .expect("migration transaction parses");
        let stage = sessions.join(
            transaction["staging_name"]
                .as_str()
                .expect("transaction names its stage"),
        );
        let witness = match shape {
            "unknown-file" => {
                let path = stage.join("foreign");
                fs::write(&path, b"foreign").expect("unknown stage file writes");
                path
            }
            "known-file-as-directory" => {
                let path = stage.join("history.jsonl");
                fs::remove_file(&path).expect("known stage file removes");
                fs::create_dir(&path).expect("known stage file becomes a directory");
                path
            }
            "markerless-nonempty" => {
                fs::remove_file(stage.join(".migration-identity"))
                    .expect("migration marker removes");
                stage.join("history.jsonl")
            }
            _ => unreachable!(),
        };
        let before = file_tree_bytes(&stage);

        migrate_legacy_session(&workspace, &original.session_id)
            .expect_err("unowned migration stage shape fails closed");

        assert!(stage.is_dir(), "{shape}: stage root remains");
        assert!(witness.exists(), "{shape}: unauthorized witness remains");
        assert_eq!(
            file_tree_bytes(&stage),
            before,
            "{shape}: stage is untouched"
        );
        assert!(
            transaction_path.is_file(),
            "{shape}: migration transaction remains"
        );
    }
}

#[test]
fn legacy_migration_anchors_migrations_before_mutating_recoverable_state() {
    let workspace = workspace_copy("smoke-flow");
    let original =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy run completes");
    let session_id = original.session_id;
    let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
    let legacy_events = sessions.join(format!("{session_id}.jsonl"));
    let legacy_events_before = fs::read(&legacy_events).expect("legacy events read");
    set_directory_sync_error_for_path_for_test(&sessions, io::ErrorKind::Other);

    migrate_legacy_session(&workspace, &session_id)
        .expect_err("the injected migrations-parent sync failure is reported");
    assert_eq!(
        fs::read(&legacy_events).expect("legacy events remain readable"),
        legacy_events_before
    );
    assert!(!sessions.join(&session_id).exists(), "target was mutated");
    assert!(
        !sessions
            .join(".migrations")
            .join(format!("{session_id}.json"))
            .exists(),
        "migration transaction was mutated"
    );
    assert!(
        fs::read_dir(&sessions)
            .expect("sessions reads")
            .all(|entry| !entry
                .expect("sessions entry reads")
                .file_name()
                .to_string_lossy()
                .starts_with(".migration-")),
        "migration stage was mutated"
    );

    start_directory_sync_trace_for_test();
    migrate_legacy_session(&workspace, &session_id)
        .expect("retry anchors migrations and completes");
    let trace = take_directory_sync_trace_for_test();
    let sessions = crate::tests::helpers::canonical_test_path(&sessions);
    assert!(
        trace.iter().any(|path| path == &sessions),
        "retry omitted sessions synchronization: {trace:?}"
    );
}

#[test]
fn legacy_migration_rejects_tampered_canonical_stage_identifiers() {
    let seed = workspace_copy("smoke-flow");
    let original =
        run_flow(&seed, "smoke-flow", EmitMode::Jsonl).expect("legacy seed run completes");

    for (label, field, replacement, expected_error) in [
        (
            "migration-tampered-stage-identity",
            "staging_identity",
            "0".repeat(64),
            "migration transaction identity is invalid",
        ),
        (
            "migration-tampered-stage-name",
            "staging_name",
            format!(
                ".migration-{}-{}.staged",
                original.session_id,
                "0".repeat(64)
            ),
            "migration staging name is invalid",
        ),
    ] {
        let workspace = empty_workspace(label);
        copy_dir(&seed, &workspace);
        crate::tests::helpers::copy_workspace_runtime(&seed, &workspace);
        set_legacy_migration_crash_point(LegacyMigrationCrashPoint::TransactionRecorded);
        migrate_legacy_session(&workspace, &original.session_id)
            .expect_err("migration stops after recording its transaction");
        let transaction_path = crate::tests::helpers::workspace_session_dir(&workspace)
            .join(".migrations")
            .join(format!("{}.json", original.session_id));
        let mut transaction: serde_json::Value = serde_json::from_str(
            fs::read_to_string(&transaction_path)
                .expect("transaction reads")
                .trim_end(),
        )
        .expect("transaction parses");
        transaction[field] = serde_json::Value::String(replacement);
        fs::write(
            &transaction_path,
            format!(
                "{}\n",
                proto::canonical_json(&transaction).expect("transaction canonicalizes")
            ),
        )
        .expect("tampered transaction writes");

        let error = migrate_legacy_session(&workspace, &original.session_id)
            .expect_err("tampered canonical stage identifier rejects recovery");
        assert!(
            error.to_string().contains(expected_error),
            "{label}: {error}"
        );
        assert!(transaction_path.is_file(), "{label}");
    }
}

#[test]
fn legacy_migration_rejects_changed_sources_and_foreign_recovery_state() {
    let seed = workspace_copy("smoke-flow");
    let original =
        run_flow(&seed, "smoke-flow", EmitMode::Jsonl).expect("legacy seed run completes");
    let transaction_path = |workspace: &Path| {
        crate::tests::helpers::workspace_session_dir(workspace)
            .join(".migrations")
            .join(format!("{}.json", original.session_id))
    };
    let interrupt = |workspace: &Path, point| {
        set_legacy_migration_crash_point(point);
        migrate_legacy_session(workspace, &original.session_id)
            .expect_err("selected migration boundary interrupts")
    };
    let clone_seed = |label| {
        let workspace = empty_workspace(label);
        copy_dir(&seed, &workspace);
        crate::tests::helpers::copy_workspace_runtime(&seed, &workspace);
        workspace
    };

    let changed_source = clone_seed("migration-changed-source");
    interrupt(
        &changed_source,
        LegacyMigrationCrashPoint::TransactionRecorded,
    );
    let metadata_path = crate::tests::helpers::workspace_log_dir(&changed_source)
        .join(format!("{}.log", original.session_id));
    OpenOptions::new()
        .append(true)
        .open(&metadata_path)
        .expect("legacy metadata opens")
        .write_all(b"tamper=1\n")
        .expect("valid but different metadata writes");
    let changed_error = migrate_legacy_session(&changed_source, &original.session_id)
        .expect_err("changed source rejects transaction recovery");
    assert!(
        changed_error
            .to_string()
            .contains("sources changed after the transaction was recorded")
    );
    assert!(metadata_path.is_file(), "changed source is retained");

    let invalid_transaction = clone_seed("migration-invalid-transaction");
    interrupt(
        &invalid_transaction,
        LegacyMigrationCrashPoint::TransactionRecorded,
    );
    let invalid_transaction_path = transaction_path(&invalid_transaction);
    let mut transaction: serde_json::Value = serde_json::from_str(
        fs::read_to_string(&invalid_transaction_path)
            .expect("transaction reads")
            .trim_end(),
    )
    .expect("transaction parses");
    transaction["schema"] = serde_json::json!("foreign-migration-v0");
    fs::write(
        &invalid_transaction_path,
        format!(
            "{}\n",
            proto::canonical_json(&transaction).expect("transaction canonicalizes")
        ),
    )
    .expect("foreign transaction writes");
    let transaction_error = migrate_legacy_session(&invalid_transaction, &original.session_id)
        .expect_err("foreign transaction rejects recovery");
    assert!(
        transaction_error
            .to_string()
            .contains("migration transaction identity is invalid")
    );
    assert!(invalid_transaction_path.is_file());

    let foreign_stage = clone_seed("migration-foreign-stage");
    interrupt(&foreign_stage, LegacyMigrationCrashPoint::StagePopulated);
    let stage_transaction: serde_json::Value = serde_json::from_str(
        fs::read_to_string(transaction_path(&foreign_stage))
            .expect("transaction reads")
            .trim_end(),
    )
    .expect("transaction parses");
    let stage = crate::tests::helpers::workspace_session_dir(&foreign_stage).join(
        stage_transaction["staging_name"]
            .as_str()
            .expect("stage name"),
    );
    let stage_marker = stage.join(".migration-identity");
    fs::write(&stage_marker, "foreign\n").expect("foreign stage marker writes");
    let stage_error = migrate_legacy_session(&foreign_stage, &original.session_id)
        .expect_err("foreign staging directory rejects recovery");
    assert!(
        stage_error
            .to_string()
            .contains("staging identity does not match its transaction")
    );
    assert_eq!(
        fs::read_to_string(&stage_marker).expect("foreign marker remains"),
        "foreign\n"
    );

    let foreign_target = clone_seed("migration-foreign-target");
    interrupt(&foreign_target, LegacyMigrationCrashPoint::TargetPublished);
    let target_marker = crate::tests::helpers::workspace_session_dir(&foreign_target)
        .join(&original.session_id)
        .join(".migration-identity");
    fs::write(&target_marker, "foreign\n").expect("foreign target marker writes");
    let target_error = migrate_legacy_session(&foreign_target, &original.session_id)
        .expect_err("foreign published target rejects recovery");
    assert!(
        target_error
            .to_string()
            .contains("published migration marker identity is invalid")
    );
    assert_eq!(
        fs::read_to_string(&target_marker).expect("foreign marker remains"),
        "foreign\n"
    );

    let reappeared = clone_seed("migration-reappeared-source");
    migrate_legacy_session(&reappeared, &original.session_id).expect("migration completes");
    let reappeared_source = crate::tests::helpers::workspace_session_dir(&reappeared)
        .join(format!("{}.jsonl", original.session_id));
    fs::write(&reappeared_source, &original.stdout).expect("later legacy source writes");
    let reappeared_error = migrate_legacy_session(&reappeared, &original.session_id)
        .expect_err("later legacy bundle conflicts with published target");
    assert!(
        reappeared_error
            .to_string()
            .contains("conflicts with a later legacy-format bundle")
    );
    assert_eq!(
        fs::read_to_string(&reappeared_source).expect("later source remains"),
        original.stdout
    );
}

#[test]
fn legacy_migration_bounds_identity_marker_reads_during_recovery() {
    const OVERSIZED_MARKER_BYTES: usize = MAX_CONVERSATION_IO_BUFFER_BYTES + 1;
    let seed = workspace_copy("smoke-flow");
    let original =
        run_flow(&seed, "smoke-flow", EmitMode::Jsonl).expect("legacy seed run completes");

    for (label, point) in [
        (
            "migration-oversized-stage-marker",
            LegacyMigrationCrashPoint::StagePopulated,
        ),
        (
            "migration-oversized-target-marker",
            LegacyMigrationCrashPoint::TargetPublished,
        ),
    ] {
        let workspace = empty_workspace(label);
        copy_dir(&seed, &workspace);
        crate::tests::helpers::copy_workspace_runtime(&seed, &workspace);
        set_legacy_migration_crash_point(point);
        migrate_legacy_session(&workspace, &original.session_id)
            .expect_err("selected migration boundary interrupts");

        let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
        let transaction_path = sessions
            .join(".migrations")
            .join(format!("{}.json", original.session_id));
        let transaction: serde_json::Value = serde_json::from_slice(
            &fs::read(&transaction_path).expect("migration transaction reads"),
        )
        .expect("migration transaction parses");
        let parent = match point {
            LegacyMigrationCrashPoint::StagePopulated => sessions.join(
                transaction["staging_name"]
                    .as_str()
                    .expect("transaction has a staging name"),
            ),
            LegacyMigrationCrashPoint::TargetPublished => sessions.join(&original.session_id),
            _ => unreachable!("the test covers marker-bearing recovery states"),
        };
        fs::write(
            parent.join(".migration-identity"),
            vec![b'x'; OVERSIZED_MARKER_BYTES],
        )
        .expect("oversized migration marker writes");
        let authority_before =
            file_tree_bytes(&crate::tests::helpers::workspace_store_dir(&workspace));

        let (recovery, metrics) = measure_conversation_operation(|| {
            Ok(migrate_legacy_session(&workspace, &original.session_id))
        })
        .expect("migration failure is measured");
        let error = recovery.expect_err("oversized migration marker rejects recovery");
        assert!(
            error.to_string().contains(&format!(
                "read size {OVERSIZED_MARKER_BYTES} bytes exceeds max 65"
            )),
            "{point:?}: {error}"
        );
        assert!(
            metrics.max_read_request_bytes <= MAX_CONVERSATION_IO_BUFFER_BYTES,
            "{point:?}: observed a {}-byte read request",
            metrics.max_read_request_bytes
        );
        assert_eq!(
            file_tree_bytes(&crate::tests::helpers::workspace_store_dir(&workspace)),
            authority_before,
            "{point:?}: rejected recovery leaves legacy, transaction, staging and target bytes unchanged"
        );
    }
}
