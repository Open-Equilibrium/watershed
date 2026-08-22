use super::super::helpers::empty_workspace;
use super::support::{assert_no_credential_staging_files, credential};
use crate::runtime::{
    credential_store::{
        CREDENTIAL_LOCK_DEADLINE, CredentialStore, StoreLock, credential_staging_path_for_test,
        set_credential_protection_error_for_test,
    },
    fs_guards::{
        set_directory_sync_error_for_path_for_test, start_directory_sync_trace_for_test,
        take_directory_sync_trace_for_test,
    },
    types::RuntimeError,
};
use std::{cell::Cell, fs, io, time::Duration};

#[test]
fn credential_lock_deadline() {
    assert_eq!(CREDENTIAL_LOCK_DEADLINE, Duration::from_secs(5));

    let workspace = empty_workspace("credential-lock-deadline");
    let path = workspace.join("credentials.json");
    let store = CredentialStore::at(path.clone());
    let prior = credential(100);
    store
        .replace_with_clock(&prior, || Duration::ZERO, |_| {})
        .expect("initial credential");
    let lock_path = path.with_extension("lock");
    let held = StoreLock::acquire(lock_path.clone(), false, || Duration::ZERO, |_| {})
        .expect("first mutation lock acquires");

    let calls = Cell::new(0usize);
    let result = store.replace_with_clock(
        &credential(200),
        || {
            let call = calls.get();
            calls.set(call + 1);
            match call {
                0 => Duration::ZERO,
                1 => CREDENTIAL_LOCK_DEADLINE - Duration::from_nanos(1),
                _ => CREDENTIAL_LOCK_DEADLINE,
            }
        },
        |_| {},
    );
    assert!(result.is_err());
    assert_eq!(store.read().expect("prior remains"), Some(prior));
    drop(held);

    fs::write(&lock_path, b"abandoned").expect("abandoned lock file is staged");
    StoreLock::acquire(lock_path, false, || Duration::ZERO, |_| {})
        .expect("a stale lock file does not block authentication");
}

#[test]
fn published_credential_parent_sync_failure_is_distinct_and_reuses_the_replacement() {
    let workspace = empty_workspace("credential-published-parent-sync-failure");
    let path = workspace.join("credentials.json");
    let lock = path.with_extension("lock");
    let store = CredentialStore::at(path);
    store
        .replace(&credential(300_000))
        .expect("near-expiry credential stores");
    let replacement = credential(900_000);
    let refreshes = Cell::new(0usize);

    let error = store
        .resolve_with_clock(
            1,
            |_| {
                refreshes.set(refreshes.get() + 1);
                set_directory_sync_error_for_path_for_test(&workspace, io::ErrorKind::Other);
                Ok(replacement.clone())
            },
            || Duration::ZERO,
            |_| {},
        )
        .expect_err("published credential finalization failure is reported");

    assert!(matches!(
        &error,
        RuntimeError::PublishedCredentialFinalizationFailure { .. }
    ));
    assert_eq!(
        error.to_string(),
        "credential_published_not_finalized: replacement credential was published but finalization failed"
    );
    assert_eq!(refreshes.get(), 1);
    assert_eq!(
        store.read().expect("complete replacement reads"),
        Some(replacement.clone())
    );
    assert_no_credential_staging_files(&workspace);
    StoreLock::acquire(lock, false, || Duration::ZERO, |_| {})
        .expect("published failure releases the credential lock");

    let resolved = store
        .resolve_with_clock(
            1,
            |_| panic!("the published replacement must prevent another provider refresh"),
            || Duration::ZERO,
            |_| {},
        )
        .expect("published replacement resolves deterministically");
    assert_eq!(resolved, replacement);

    start_directory_sync_trace_for_test();
    store
        .replace_with_clock(&replacement, || Duration::ZERO, |_| {})
        .expect("the next locked operation finalizes the published replacement");
    let trace = take_directory_sync_trace_for_test();
    assert!(
        trace.iter().any(|synced| synced == workspace.as_ref()),
        "the credential parent was not finalized: {trace:?}"
    );
    assert_eq!(refreshes.get(), 1);
}

#[cfg(any(unix, windows))]
#[test]
fn published_credential_protection_failure_is_distinct_and_next_lock_finalizes() {
    let workspace = empty_workspace("credential-published-protection-failure");
    let parent = workspace.join("private");
    let path = parent.join("credentials.json");
    let lock = path.with_extension("lock");
    let store = CredentialStore::protected_at(path);
    store
        .replace(&credential(300_000))
        .expect("near-expiry protected credential stores");
    let replacement = credential(900_000);
    let refreshes = Cell::new(0usize);
    set_credential_protection_error_for_test(io::ErrorKind::PermissionDenied);

    let error = store
        .resolve_with_clock(
            1,
            |_| {
                refreshes.set(refreshes.get() + 1);
                Ok(replacement.clone())
            },
            || Duration::ZERO,
            |_| {},
        )
        .expect_err("published credential protection failure is reported");

    assert!(matches!(
        &error,
        RuntimeError::PublishedCredentialFinalizationFailure { .. }
    ));
    assert_eq!(refreshes.get(), 1);
    assert_eq!(
        store.read().expect("complete protected replacement reads"),
        Some(replacement.clone())
    );
    assert_no_credential_staging_files(&parent);
    StoreLock::acquire(lock, true, || Duration::ZERO, |_| {})
        .expect("published failure releases the protected credential lock");

    start_directory_sync_trace_for_test();
    store
        .replace_with_clock(&replacement, || Duration::ZERO, |_| {})
        .expect("the next locked operation validates and finalizes the replacement");
    let trace = take_directory_sync_trace_for_test();
    assert!(
        trace.iter().any(|synced| synced == &parent),
        "the credential parent was not finalized: {trace:?}"
    );
    assert_eq!(refreshes.get(), 1);
}

#[test]
fn credential_store_creates_missing_unprotected_parents() {
    let workspace = empty_workspace("credential-store-nested-parent");
    let path = workspace.join("nested/config/credentials.json");
    let store = CredentialStore::at(path.clone());
    let current = credential(900_000);

    store.replace(&current).expect("nested credential stores");

    assert!(path.is_file());
    assert_eq!(
        store.read().expect("nested credential reads"),
        Some(current)
    );
}

#[test]
fn credential_store_retries_every_created_parent_sync_before_success() {
    let workspace = empty_workspace("credential-store-parent-sync-retry");
    let workspace_parent = workspace
        .parent()
        .expect("workspace has an externally owned parent")
        .to_owned();
    let nested = workspace.join("nested");
    let parent = nested.join("config");
    let store = CredentialStore::at(parent.join("credentials.json"));
    let current = credential(900_000);
    set_directory_sync_error_for_path_for_test(&workspace, io::ErrorKind::Other);

    store
        .replace(&current)
        .expect_err("the injected created-parent sync failure is reported");
    assert!(parent.is_dir(), "the failed sync leaves created parents");

    start_directory_sync_trace_for_test();
    set_directory_sync_error_for_path_for_test(&workspace_parent, io::ErrorKind::Other);
    store
        .replace(&current)
        .expect("retry stops at the store's fixed durable ancestor");
    let trace = take_directory_sync_trace_for_test();
    for expected in [nested.as_path(), workspace.as_ref(), parent.as_path()] {
        assert!(
            trace.iter().any(|path| path == expected),
            "retry omitted directory synchronization for {}: {trace:?}",
            expected.display()
        );
    }
    assert!(
        !trace.iter().any(|path| path == &workspace_parent),
        "retry crossed into an externally owned ancestor: {trace:?}"
    );
    assert_eq!(
        store.read().expect("retried credential reads"),
        Some(current)
    );
}

#[test]
fn failed_credential_refresh_removes_its_atomic_staging_file() {
    let workspace = empty_workspace("credential-store-staging-cleanup");
    let path = workspace.join("credentials.json");
    let store = CredentialStore::at(path.clone());
    store
        .replace(&credential(300_000))
        .expect("near-expiry credential stores");

    let result = store.resolve_with_clock(
        1,
        |_| {
            fs::remove_file(&path).expect("credential file removes for conflict fixture");
            fs::create_dir(&path).expect("conflicting destination directory creates");
            Ok(credential(900_000))
        },
        || Duration::ZERO,
        |_| {},
    );

    assert!(result.is_err());
    assert_no_credential_staging_files(&workspace);
}

#[test]
fn unprotected_credential_mutation_recovers_only_exact_abandoned_stages() {
    let workspace = empty_workspace("credential-store-abandoned-stage-recovery");
    let path = workspace.join("credentials.json");
    let store = CredentialStore::at(path.clone());
    store
        .replace(&credential(900_000))
        .expect("credential stores");
    let abandoned = credential_staging_path_for_test(&path, 7, 9);
    let lookalike = workspace.join(".credentials-7-nine.staged");
    fs::write(&abandoned, b"abandoned secret").expect("abandoned stage writes");
    fs::write(&lookalike, b"unrelated").expect("lookalike writes");

    let peer_path = workspace.join("peer.json");
    let peer_store = CredentialStore::at(peer_path.clone());
    let peer_credential = credential(1_000_000);
    let peer_stage = credential_staging_path_for_test(&peer_path, 8, 10);
    assert_ne!(abandoned, peer_stage);
    fs::write(
        &peer_stage,
        serde_json::to_vec(&serde_json::json!({ "openai-codex": peer_credential }))
            .expect("peer staged credential serializes"),
    )
    .expect("peer live stage writes");

    assert!(store.logout().expect("credential logs out"));

    assert!(!abandoned.exists(), "abandoned stage must be recovered");
    assert!(
        peer_stage.exists(),
        "another destination's live stage must not be recovered"
    );
    assert_eq!(
        fs::read(&lookalike).expect("lookalike remains readable"),
        b"unrelated"
    );
    fs::rename(&peer_stage, &peer_path).expect("peer live transaction resumes");
    assert_eq!(
        peer_store.read().expect("peer credential reads"),
        Some(peer_credential)
    );
}

#[cfg(any(unix, windows))]
#[test]
fn protected_credential_mutation_recovers_an_abandoned_stage() {
    let workspace = empty_workspace("protected-credential-abandoned-stage-recovery");
    let parent = workspace.join("private");
    let path = parent.join("credentials.json");
    let store = CredentialStore::protected_at(path.clone());
    let peer_path = parent.join("peer.json");
    let peer_store = CredentialStore::protected_at(peer_path.clone());
    let peer_credential = credential(1_000_000);
    let peer_stage = credential_staging_path_for_test(&peer_path, 8, 10);
    peer_store
        .replace(&peer_credential)
        .expect("peer protected credential stores");
    drop(peer_store);
    fs::rename(&peer_path, &peer_stage).expect("peer protected transaction starts");

    store
        .replace(&credential(900_000))
        .expect("protected credential stores");
    let abandoned = credential_staging_path_for_test(&path, 7, 9);
    assert_ne!(abandoned, peer_stage);
    fs::write(&abandoned, b"abandoned secret").expect("abandoned protected stage writes");

    assert!(store.logout().expect("protected credential logs out"));

    assert!(!abandoned.exists(), "abandoned stage must be recovered");
    assert!(
        peer_stage.exists(),
        "another protected destination's live stage must not be recovered"
    );
    drop(store);
    fs::rename(&peer_stage, &peer_path).expect("peer protected live transaction resumes");
    assert_eq!(
        CredentialStore::protected_at(peer_path)
            .read()
            .expect("peer protected credential reads"),
        Some(peer_credential)
    );
}

#[test]
fn credential_store_convenience_methods_and_error_paths_release_lock_ownership() {
    let workspace = empty_workspace("credential-store-lock-cleanup");
    let path = workspace.join("credentials.json");
    let lock = path.with_extension("lock");
    let store = CredentialStore::at(path.clone());
    let current = credential(900_000);
    store.replace(&current).expect("credential stores");
    assert_eq!(store.read().expect("credential reads"), Some(current));
    assert!(lock.exists());
    StoreLock::acquire(lock.clone(), false, || Duration::ZERO, |_| {})
        .expect("completed mutation releases its lock");
    assert!(store.logout().expect("credential logs out"));
    assert!(!store.logout().expect("empty logout succeeds"));
    assert!(lock.exists());
    StoreLock::acquire(lock.clone(), false, || Duration::ZERO, |_| {})
        .expect("logout releases its lock");

    fs::write(&path, b"not-json").expect("malformed fixture write");
    assert!(
        store
            .replace_with_clock(&credential(1_000_000), || Duration::ZERO, |_| {})
            .is_err()
    );
    assert!(lock.exists());
    StoreLock::acquire(lock, false, || Duration::ZERO, |_| {})
        .expect("failed mutation releases its lock");
}
