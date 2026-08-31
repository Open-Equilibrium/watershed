use super::super::helpers::empty_workspace;
use super::support::credential;
use crate::runtime::{
    credential_store::{CREDENTIAL_STORE_MAX_BYTES, CredentialStore},
    oauth_credential::{CredentialRecord, MAX_OAUTH_FIELD_BYTES, MAX_OAUTH_SECRET_BYTES},
    types::RuntimeError,
};
use std::{
    cell::{Cell, RefCell},
    fs,
    time::Duration,
};

#[test]
fn credential_store_replaces_atomically_and_logout_is_local() {
    let workspace = empty_workspace("credential-store-lifecycle");
    let path = workspace.join("credentials.json");
    let foreign = workspace.join("foreign-client.json");
    fs::write(&foreign, b"foreign").expect("foreign cache");
    let store = CredentialStore::at(path);
    let credential = credential(100);

    store
        .replace_with_clock(&credential, || Duration::ZERO, |_| {})
        .expect("replace credential");
    assert_eq!(store.read().expect("read credential"), Some(credential));
    assert!(
        store
            .logout_with_clock(|| Duration::ZERO, |_| {})
            .expect("logout")
    );
    assert_eq!(store.read().expect("empty store"), None);
    assert_eq!(fs::read(foreign).expect("foreign unchanged"), b"foreign");
}

#[test]
fn credential_resolution_refreshes_near_expiry_under_the_store_lock() {
    let workspace = empty_workspace("credential-resolution-refresh");
    let store = CredentialStore::at(workspace.join("credentials.json"));
    store
        .replace_with_clock(&credential(300_000), || Duration::ZERO, |_| {})
        .expect("near-expiry credential");
    let refreshes = Cell::new(0usize);

    let resolved = store
        .resolve_with_clock(
            1,
            |prior| {
                refreshes.set(refreshes.get() + 1);
                assert_eq!(prior.expires, 300_000);
                Ok(credential(900_000))
            },
            || Duration::ZERO,
            |_| {},
        )
        .expect("credential refreshes");

    assert_eq!(refreshes.get(), 1);
    assert_eq!(resolved.expires, 900_000);
    assert_eq!(store.read().expect("stored refresh"), Some(resolved));
}

#[cfg(any(unix, windows))]
#[test]
fn protected_credential_store_coordinates_the_end_to_end_lifecycle() {
    let workspace = empty_workspace("protected-credential-lifecycle");
    let store = CredentialStore::protected_at(workspace.join("private/credentials.json"));
    let prior = credential(300_000);
    store
        .replace_with_clock(&prior, || Duration::ZERO, |_| {})
        .expect("protected credential stores");
    assert_eq!(
        store.read().expect("protected credential reads"),
        Some(prior.clone())
    );

    assert!(
        store
            .resolve_with_clock(
                1,
                |_| Err(RuntimeError::Protocol("refresh rejected".to_owned())),
                || Duration::ZERO,
                |_| {},
            )
            .is_err()
    );
    assert_eq!(
        store.read().expect("failed refresh preserves credential"),
        Some(prior)
    );

    let replacement = credential(900_000);
    let resolved = store
        .resolve_with_clock(1, |_| Ok(replacement.clone()), || Duration::ZERO, |_| {})
        .expect("protected credential refreshes");
    assert_eq!(resolved, replacement);
    assert_eq!(
        store.read().expect("protected refresh reads"),
        Some(replacement)
    );

    assert!(store.logout().expect("protected credential logs out"));
    assert_eq!(store.read().expect("protected store is empty"), None);
}

#[test]
fn credential_store_rejects_malformed_and_oversized_documents() {
    let workspace = empty_workspace("credential-store-bounds");
    let path = workspace.join("credentials.json");
    let store = CredentialStore::at(path.clone());
    assert_eq!(store.read().expect("missing store is empty"), None);

    for document in [
        b"not-json".as_slice(),
        br#"{"unknown":true}"#.as_slice(),
        br#"{"openai-codex":{"type":"oauth","access":"a","refresh":"r","expires":1,"accountId":"x"}}"#
            .as_slice(),
        br#"{"openai-codex":{"type":"api-key","access":"x","refresh":"x","expires":1,"accountId":"x"}}"#
            .as_slice(),
    ] {
        fs::write(&path, document).expect("invalid fixture write");
        assert!(store.read().is_err());
    }

    fs::write(&path, b"{}").expect("empty document write");
    assert_eq!(store.read().expect("empty document is valid"), None);
    fs::write(&path, br#"{"openai-codex":null}"#).expect("null credential write");
    assert_eq!(store.read().expect("null credential is absent"), None);
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("oversized fixture opens");
    file.set_len(CREDENTIAL_STORE_MAX_BYTES + 1)
        .expect("oversized fixture grows");
    drop(file);
    assert!(store.read().is_err());

    fs::remove_file(&path).expect("oversized fixture removed");
    fs::create_dir(&path).expect("directory fixture created");
    assert!(store.read().is_err());
}

#[test]
fn credential_store_does_not_report_metadata_errors_as_absence() {
    let workspace = empty_workspace("credential-store-metadata-error");
    let invalid_component = "a".repeat(256);
    let store = CredentialStore::at(workspace.join(invalid_component).join("credentials.json"));

    assert!(store.read().is_err());
}

#[test]
fn credential_store_enforces_the_on_disk_size_limit_before_replacement() {
    let workspace = empty_workspace("credential-store-write-boundary");
    let path = workspace.join("credentials.json");
    let store = CredentialStore::at(path.clone());
    let boundary = credential_with_document_len(CREDENTIAL_STORE_MAX_BYTES as usize - 1);

    store
        .replace(&boundary)
        .expect("maximum on-disk credential stores");
    assert_eq!(
        fs::metadata(&path).expect("credential metadata").len(),
        CREDENTIAL_STORE_MAX_BYTES
    );
    assert_eq!(
        store.read().expect("boundary credential reads"),
        Some(boundary.clone())
    );

    let oversized = credential_with_document_len(CREDENTIAL_STORE_MAX_BYTES as usize);
    assert!(store.replace(&oversized).is_err());
    assert_eq!(
        store.read().expect("prior credential remains"),
        Some(boundary)
    );
}

fn credential_with_document_len(target: usize) -> CredentialRecord {
    let mut value = credential(900_000);
    value.access = "a".repeat(MAX_OAUTH_SECRET_BYTES);
    value.refresh = "r".repeat(MAX_OAUTH_SECRET_BYTES);
    value.account_id = "c".repeat(MAX_OAUTH_FIELD_BYTES);
    let baseline = serialized_credential_document_len(&value);
    let growth = target
        .checked_sub(baseline)
        .expect("target fits credential fields");
    let escaped_controls = growth / 5;
    let escaped_quotes = growth % 5;
    assert!(escaped_controls + escaped_quotes <= MAX_OAUTH_SECRET_BYTES);
    value.access = format!(
        "{}{}{}",
        "\u{1}".repeat(escaped_controls),
        "\"".repeat(escaped_quotes),
        "a".repeat(MAX_OAUTH_SECRET_BYTES - escaped_controls - escaped_quotes)
    );
    assert_eq!(serialized_credential_document_len(&value), target);
    value
}

fn serialized_credential_document_len(credential: &CredentialRecord) -> usize {
    serde_json::to_vec(&serde_json::json!({
        "openai-codex": credential,
    }))
    .expect("credential serializes")
    .len()
}

#[test]
fn credential_resolution_requires_or_reuses_a_current_credential() {
    let workspace = empty_workspace("credential-resolution-current");
    let store = CredentialStore::at(workspace.join("credentials.json"));
    let error = store
        .resolve_with_clock(
            1,
            |_| panic!("an absent credential cannot refresh"),
            || Duration::ZERO,
            |_| {},
        )
        .expect_err("an absent credential is rejected");
    assert_eq!(error.exit_code(), 65);
    assert!(matches!(error, RuntimeError::AuthenticationRequired(_)));

    let current = credential(900_000);
    store
        .replace_with_clock(&current, || Duration::ZERO, |_| {})
        .expect("current credential stores");
    let resolved = store
        .resolve_with_clock(
            1,
            |_| panic!("a current credential must not refresh"),
            || Duration::ZERO,
            |_| {},
        )
        .expect("current credential resolves");
    assert_eq!(resolved, current);
}

#[test]
fn credential_refresh_failure_preserves_the_prior_record() {
    let workspace = empty_workspace("credential-refresh-failure");
    let store = CredentialStore::at(workspace.join("credentials.json"));
    let prior = credential(100);
    store
        .replace_with_clock(&prior, || Duration::ZERO, |_| {})
        .expect("prior credential stores");

    assert!(
        store
            .resolve_with_clock(
                1,
                |_| Err(RuntimeError::Protocol("refresh rejected".to_owned())),
                || Duration::ZERO,
                |_| {},
            )
            .is_err()
    );
    assert_eq!(store.read().expect("prior remains"), Some(prior.clone()));

    assert!(
        store
            .resolve_with_clock(
                1,
                |_| {
                    let mut invalid = credential(900_000);
                    invalid.credential_type = "api-key".to_owned();
                    Ok(invalid)
                },
                || Duration::ZERO,
                |_| {},
            )
            .is_err()
    );
    assert_eq!(store.read().expect("prior still remains"), Some(prior));
}

#[test]
fn credential_refresh_reuses_a_winner_after_lock_contention() {
    assert_credential_refresh_reuses_a_winner_after_lock_contention(false);
}

#[cfg(any(unix, windows))]
#[test]
fn protected_credential_refresh_reuses_a_winner_after_lock_contention() {
    assert_credential_refresh_reuses_a_winner_after_lock_contention(true);
}

fn assert_credential_refresh_reuses_a_winner_after_lock_contention(protected: bool) {
    let workspace = empty_workspace("credential-refresh-race-winner");
    let path = if protected {
        workspace.join("private/credentials.json")
    } else {
        workspace.join("credentials.json")
    };
    let store = if protected {
        CredentialStore::protected_at(path.clone())
    } else {
        CredentialStore::at(path.clone())
    };
    store
        .replace_with_clock(&credential(300_000), || Duration::ZERO, |_| {})
        .expect("near-expiry credential stores");
    let held = RefCell::new(Some(
        store
            .acquire_lock_for_test(|| Duration::ZERO, |_| {})
            .expect("competing refresh lock acquires"),
    ));
    let winner = credential(900_000);
    let winner_document = serde_json::to_vec(&serde_json::json!({
        "openai-codex": winner,
    }))
    .expect("winner credential serializes");
    let waits = Cell::new(0usize);

    let resolved = store
        .resolve_with_clock(
            1,
            |_| panic!("the losing refresh must reuse the winner's credential"),
            || Duration::ZERO,
            |_| {
                if waits.get() == 0 {
                    drop(held.borrow_mut().take());
                    fs::write(&path, &winner_document).expect("winner credential commits");
                }
                waits.set(waits.get() + 1);
            },
        )
        .expect("winner credential resolves");

    assert_eq!(resolved, winner);
    assert_eq!(waits.get(), 1);
}
