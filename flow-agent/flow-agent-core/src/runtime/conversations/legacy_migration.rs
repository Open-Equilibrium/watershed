use super::storage::{
    CONVERSATION_MIGRATIONS_DIR, migration_transaction_leaf, migration_transaction_stage_leaf,
};
use super::{
    contract::{CONVERSATION_RUNS_DIR, protocol, validate_id},
    lifecycle::reconcile_releases,
    run_log::RunLogRecord,
    session_event_stream::SessionEventReader,
};
use crate::runtime::{
    fs_guards::{
        AnchoredDir, AnchoredWorkspace, DirectoryErrorMode, RuntimeDirs,
        ensure_anchored_runtime_dirs, open_runtime_dir, sync_anchored_directory,
    },
    session_authority::{SessionOwnershipLease, conversation_ownership_key, run_ownership_key},
    stage_results::reconcile_operation_and_cleanup,
    types::{RuntimeError, SESSION_STORAGE_DIR},
};
use proto::EventType;
use std::path::Path;
mod plan;
mod stage;
mod target;

use plan::build_legacy_migration_plan;
use stage::{
    MigrationTransaction, clear_migration_transaction, create_json_file, publish_migration_stage,
    read_migration_transaction, recover_migration_transaction_write,
    remove_published_staging_marker, remove_recoverable_staging, validate_migration_transaction,
};
pub(super) use target::{legacy_log_source_id, legacy_session_source_id};
use target::{
    legacy_source_present, legacy_source_present_in, retire_legacy_sources,
    source_manifest_from_target, validate_migrated_target,
};

fn legacy_uncertain_attempt_count(records: &[RunLogRecord]) -> Result<u64, RuntimeError> {
    records
        .iter()
        .filter(|record| {
            matches!(
                record,
                RunLogRecord::LegacyToolObservation { outcome, .. } if outcome.is_uncertain()
            )
        })
        .count()
        .try_into()
        .map_err(|_| protocol("legacy uncertain attempt count exceeds u64"))
}
pub(crate) fn migrate_legacy_session(
    workspace: &Path,
    session_id: &str,
) -> Result<(), RuntimeError> {
    validate_id(session_id, "legacy session")?;
    let anchored_workspace = AnchoredWorkspace::open(workspace)?;
    let roots = ensure_anchored_runtime_dirs(&anchored_workspace)?;
    let marker = roots
        .sessions
        .path
        .join(session_id)
        .join(CONVERSATION_RUNS_DIR)
        .join(session_id);
    let legacy = SessionOwnershipLease::acquire_anchored(&anchored_workspace, session_id, &marker)?;
    let conversation_key = conversation_ownership_key(session_id);
    let conversation = match SessionOwnershipLease::acquire_anchored(
        &anchored_workspace,
        &conversation_key,
        &marker,
    ) {
        Ok(lease) => lease,
        Err(error) => {
            return reconcile_operation_and_cleanup(Err(error), legacy.release());
        }
    };
    let run_key = run_ownership_key(session_id, session_id);
    let run = match SessionOwnershipLease::acquire_anchored(&anchored_workspace, &run_key, &marker)
    {
        Ok(lease) => lease,
        Err(error) => {
            let release = reconcile_releases(Ok(()), conversation.release(), legacy.release());
            return reconcile_operation_and_cleanup(Err(error), release);
        }
    };
    #[cfg(test)]
    legacy_migration_roots_checkpoint()?;
    let operation = migrate_legacy_session_locked(&anchored_workspace, &roots, session_id);
    let release = reconcile_releases(run.release(), conversation.release(), legacy.release());
    reconcile_operation_and_cleanup(operation, release)
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LegacyMigrationCrashPoint {
    TransactionRecorded,
    StagePopulated,
    TargetPublished,
    FirstSourceRetired,
    BeforeTransactionCleared,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LegacyMigrationControlFile {
    Transaction,
    IdentityMarker,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LegacyEventScanPoint {
    TerminalCheck,
    MigrationPlan,
    TargetValidation,
}

#[cfg(test)]
type LegacyEventScanObserver = (
    LegacyEventScanPoint,
    Box<dyn FnOnce() -> Result<(), RuntimeError>>,
);

#[cfg(test)]
type LegacyMigrationRootsObserver = Box<dyn FnOnce() -> Result<(), RuntimeError>>;

#[cfg(test)]
type LegacyObjectCopyObserver = Box<dyn FnOnce() -> Result<(), RuntimeError>>;

#[cfg(test)]
std::thread_local! {
    static LEGACY_MIGRATION_CRASH_POINT: std::cell::Cell<Option<LegacyMigrationCrashPoint>> =
        const { std::cell::Cell::new(None) };
    static LEGACY_EVENT_SCAN_OBSERVER: std::cell::RefCell<Option<LegacyEventScanObserver>> =
        const { std::cell::RefCell::new(None) };
    static LEGACY_MIGRATION_ROOTS_OBSERVER: std::cell::RefCell<Option<LegacyMigrationRootsObserver>> =
        const { std::cell::RefCell::new(None) };
    static LEGACY_MIGRATION_CONTROL_WRITE_FAILURE:
        std::cell::Cell<Option<LegacyMigrationControlFile>> = const { std::cell::Cell::new(None) };
    static LEGACY_OBJECT_COPY_OBSERVER: std::cell::RefCell<Option<LegacyObjectCopyObserver>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_legacy_migration_crash_point(point: LegacyMigrationCrashPoint) {
    LEGACY_MIGRATION_CRASH_POINT.set(Some(point));
}

#[cfg(test)]
pub(crate) fn set_legacy_event_scan_observer(
    point: LegacyEventScanPoint,
    observer: impl FnOnce() -> Result<(), RuntimeError> + 'static,
) {
    LEGACY_EVENT_SCAN_OBSERVER.with(|slot| slot.replace(Some((point, Box::new(observer)))));
}

#[cfg(test)]
pub(crate) fn set_legacy_migration_roots_observer(
    observer: impl FnOnce() -> Result<(), RuntimeError> + 'static,
) {
    LEGACY_MIGRATION_ROOTS_OBSERVER.with(|slot| slot.replace(Some(Box::new(observer))));
}

#[cfg(test)]
pub(crate) fn set_legacy_migration_control_write_failure(file: LegacyMigrationControlFile) {
    LEGACY_MIGRATION_CONTROL_WRITE_FAILURE.set(Some(file));
}

#[cfg(test)]
pub(crate) fn set_legacy_object_copy_observer(
    observer: impl FnOnce() -> Result<(), RuntimeError> + 'static,
) {
    LEGACY_OBJECT_COPY_OBSERVER.with(|slot| slot.replace(Some(Box::new(observer))));
}

#[cfg(test)]
pub(super) fn legacy_object_copy_checkpoint() -> Result<(), RuntimeError> {
    LEGACY_OBJECT_COPY_OBSERVER
        .with(|slot| slot.replace(None).map_or(Ok(()), |observer| observer()))
}

#[cfg(test)]
pub(super) fn legacy_migration_control_write_should_fail(file: LegacyMigrationControlFile) -> bool {
    if LEGACY_MIGRATION_CONTROL_WRITE_FAILURE.get() == Some(file) {
        LEGACY_MIGRATION_CONTROL_WRITE_FAILURE.set(None);
        return true;
    }
    false
}

#[cfg(test)]
fn legacy_migration_roots_checkpoint() -> Result<(), RuntimeError> {
    LEGACY_MIGRATION_ROOTS_OBSERVER
        .with(|slot| slot.replace(None).map_or(Ok(()), |observer| observer()))
}

#[cfg(test)]
pub(super) fn legacy_event_scan_checkpoint(
    point: LegacyEventScanPoint,
) -> Result<(), RuntimeError> {
    LEGACY_EVENT_SCAN_OBSERVER.with(|slot| {
        let Some((expected, observer)) = slot.replace(None) else {
            return Ok(());
        };
        if expected == point {
            observer()
        } else {
            slot.replace(Some((expected, observer)));
            Ok(())
        }
    })
}

#[cfg(test)]
pub(super) fn legacy_migration_checkpoint(
    point: LegacyMigrationCrashPoint,
) -> Result<(), RuntimeError> {
    if LEGACY_MIGRATION_CRASH_POINT.get() == Some(point) {
        LEGACY_MIGRATION_CRASH_POINT.set(None);
        return Err(protocol(format!(
            "injected legacy migration crash at {point:?}"
        )));
    }
    Ok(())
}

pub(crate) fn migrate_legacy_session_if_present(
    workspace: &Path,
    session_id: &str,
) -> Result<(), RuntimeError> {
    validate_id(session_id, "legacy session")?;
    if legacy_migration_is_present(workspace, session_id)? {
        migrate_legacy_session(workspace, session_id)?;
    }
    Ok(())
}

fn legacy_migration_is_present(workspace: &Path, session_id: &str) -> Result<bool, RuntimeError> {
    Ok(
        legacy_migration_transaction_is_present(workspace, session_id)?
            || legacy_source_present(workspace, session_id)?,
    )
}

fn legacy_migration_transaction_is_present(
    workspace: &Path,
    session_id: &str,
) -> Result<bool, RuntimeError> {
    let Some(sessions) = open_runtime_dir(workspace, SESSION_STORAGE_DIR)? else {
        return Ok(false);
    };
    let Some(migrations) = sessions.child(
        CONVERSATION_MIGRATIONS_DIR,
        false,
        DirectoryErrorMode::Protocol,
    )?
    else {
        return Ok(false);
    };
    let transaction = migrations.file(migration_transaction_leaf(session_id));
    match transaction.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(protocol("migration transaction must be a real file"))
        }
        Ok(_) => Ok(true),
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(source) => Err(source),
    }
}

pub(crate) fn legacy_flat_compatibility_is_available(
    workspace: &Path,
    session_id: &str,
) -> Result<bool, RuntimeError> {
    if !proto::is_valid_session_id(session_id) {
        return Err(RuntimeError::Usage(format!(
            "invalid session_id {session_id:?}"
        )));
    }
    if !legacy_source_present(workspace, session_id)? {
        return Ok(false);
    }
    let target_present = open_runtime_dir(workspace, SESSION_STORAGE_DIR)?
        .map(|sessions| {
            sessions
                .child(session_id, false, DirectoryErrorMode::Protocol)
                .map(|target| target.is_some())
        })
        .transpose()?
        .unwrap_or(false);
    if target_present || legacy_migration_transaction_is_present(workspace, session_id)? {
        migrate_legacy_session(workspace, session_id)?;
        return Ok(false);
    }
    Ok(true)
}

pub(crate) fn legacy_session_is_terminal(
    workspace: &Path,
    session_id: &str,
) -> Result<Option<bool>, RuntimeError> {
    validate_id(session_id, "legacy session")?;
    if !legacy_flat_compatibility_is_available(workspace, session_id)? {
        return Ok(None);
    }
    let mut reader = match SessionEventReader::open_flat(workspace, session_id) {
        Ok(reader) => reader,
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let mut last_event_type = None;
    reader.visit_verified_after(0, u64::MAX, |event, _line| {
        #[cfg(test)]
        legacy_event_scan_checkpoint(LegacyEventScanPoint::TerminalCheck)?;
        last_event_type = Some(event.event_type);
        Ok(())
    })?;
    Ok(Some(last_event_type.is_some_and(|event_type| {
        matches!(
            event_type,
            EventType::SessionCompleted | EventType::SessionFailed
        )
    })))
}

fn migrate_legacy_session_locked(
    workspace: &AnchoredWorkspace,
    roots: &RuntimeDirs,
    session_id: &str,
) -> Result<(), RuntimeError> {
    let sessions = &roots.sessions;
    let migrations = sessions
        .child(
            CONVERSATION_MIGRATIONS_DIR,
            true,
            DirectoryErrorMode::Protocol,
        )?
        .ok_or_else(|| protocol("migration transaction directory disappeared"))?;
    sync_anchored_directory(sessions)?;
    let transaction_path = migrations.file(migration_transaction_leaf(session_id));
    let transaction_stage_path = migrations.file(migration_transaction_stage_leaf(session_id));
    recover_migration_transaction_write(&migrations, &transaction_path, &transaction_stage_path)?;
    let target = sessions.child(session_id, false, DirectoryErrorMode::Protocol)?;
    let transaction = read_migration_transaction(&transaction_path)?;

    if let Some(transaction) = transaction {
        validate_migration_transaction(&transaction, session_id)?;
        if let Some(target) = target.as_ref() {
            finalize_published_migration(roots, target, &transaction)?;
            clear_migration_transaction(&transaction_path, &migrations)?;
            return Ok(());
        }
        remove_recoverable_staging(sessions, &transaction)?;
        let plan = build_legacy_migration_plan(workspace, sessions, &roots.logs, session_id)?;
        if plan.manifest != transaction.source_manifest {
            return Err(protocol(
                "legacy migration sources changed after the transaction was recorded",
            ));
        }
        publish_migration_stage(sessions, &transaction, &plan)?;
        let target = published_target(sessions, session_id)?;
        finalize_published_migration(roots, &target, &transaction)?;
        clear_migration_transaction(&transaction_path, &migrations)?;
        return Ok(());
    }

    if let Some(target) = target.as_ref() {
        if legacy_source_present_in(sessions, &roots.logs, session_id)? {
            return Err(protocol(
                "published conversation conflicts with a later legacy-format bundle",
            ));
        }
        validate_migrated_target(target, &source_manifest_from_target(target, session_id)?)?;
        return Ok(());
    }

    let plan = build_legacy_migration_plan(workspace, sessions, &roots.logs, session_id)?;
    let transaction = MigrationTransaction::new(session_id, plan.manifest.clone())?;
    create_json_file(
        &migrations,
        &transaction_path,
        &transaction_stage_path,
        &transaction,
    )?;
    #[cfg(test)]
    legacy_migration_checkpoint(LegacyMigrationCrashPoint::TransactionRecorded)?;
    publish_migration_stage(sessions, &transaction, &plan)?;
    let target = published_target(sessions, session_id)?;
    finalize_published_migration(roots, &target, &transaction)?;
    #[cfg(test)]
    legacy_migration_checkpoint(LegacyMigrationCrashPoint::BeforeTransactionCleared)?;
    clear_migration_transaction(&transaction_path, &migrations)
}

fn finalize_published_migration(
    roots: &RuntimeDirs,
    target: &AnchoredDir,
    transaction: &MigrationTransaction,
) -> Result<(), RuntimeError> {
    validate_migrated_target(target, &transaction.source_manifest)?;
    remove_published_staging_marker(target, &transaction.staging_identity)?;
    retire_legacy_sources(
        &roots.sessions,
        &roots.logs,
        target,
        &transaction.source_manifest,
    )
}

fn published_target(sessions: &AnchoredDir, session_id: &str) -> Result<AnchoredDir, RuntimeError> {
    sessions
        .child(session_id, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("published conversation directory disappeared"))
}
