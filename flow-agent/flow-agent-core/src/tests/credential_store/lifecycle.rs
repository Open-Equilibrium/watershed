use super::super::helpers::empty_workspace;
use super::support::{
    credential, endpoint_credential, resolve, set_unix_mode, subscription_document,
};
use crate::runtime::{
    credential_store::{CREDENTIAL_STORE_MAX_BYTES, CredentialStore},
    oauth_credential::{CredentialRecord, MAX_OAUTH_FIELD_BYTES, MAX_OAUTH_SECRET_BYTES},
    types::RuntimeError,
};
use crate::tests::test_support::TempWorkspace;
use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[test]
fn credential_store_replaces_atomically_and_logout_is_local() {
    let workspace = empty_workspace("credential-store-lifecycle");
    let parent = workspace.join("private");
    let store = CredentialStore::at(parent.join("credentials.json"));
    let credential = credential(100);
    store.replace(&credential).expect("replace credential");
    let foreign = parent.join("foreign-client.json");
    fs::write(&foreign, b"foreign").expect("foreign cache");

    assert_eq!(store.read().expect("read credential"), Some(credential));
    assert!(store.logout().expect("logout"));
    assert_eq!(store.read().expect("empty store"), None);
    assert_eq!(fs::read(foreign).expect("foreign unchanged"), b"foreign");
}

#[test]
fn credential_store_keeps_each_provider_slot_through_every_write() {
    let workspace = empty_workspace("credential-store-provider-slots");
    let store = CredentialStore::at(workspace.join("private/credentials.json"));
    let subscription = credential(300_000);
    let endpoint = endpoint_credential("https://models.example.test/v1", "sk-slot-fixture");
    store
        .replace(&subscription)
        .expect("subscription credential stores");
    store
        .replace_endpoint_with_clock(&endpoint, || Duration::ZERO, |_| {})
        .expect("endpoint credential stores beside it");
    assert_eq!(
        store.read().expect("subscription reads"),
        Some(subscription)
    );
    assert_eq!(
        store.read_endpoint().expect("endpoint reads"),
        Some(endpoint.clone())
    );

    resolve(&store, |_| Ok(credential(900_000))).expect("subscription refreshes");
    assert_eq!(
        store.read_endpoint().expect("refresh keeps the endpoint"),
        Some(endpoint.clone())
    );

    assert!(store.logout().expect("subscription logs out"));
    assert_eq!(store.read().expect("subscription is gone"), None);
    assert_eq!(
        store
            .read_endpoint()
            .expect("subscription logout keeps the endpoint"),
        Some(endpoint)
    );

    assert!(
        store
            .logout_endpoint_with_clock(|| Duration::ZERO, |_| {})
            .expect("endpoint logs out")
    );
    assert_eq!(store.read_endpoint().expect("endpoint is gone"), None);
}

const BROKEN_ENDPOINT: &str = r#"{ "type" : "api-key", "baseUrl": "http://models.example.test/v1", "key": "sk-broken-fixture" }"#;
const BROKEN_SUBSCRIPTION: &str =
    r#"{"type":"oauth","access":"a","refresh":"r","expires":1,"accountId":"x"}"#;
const UNKNOWN_SLOT: &str = r#"{"nested": [1, 2, 3], "note": "kept"}"#;
const RECORD_WITHOUT_ROUTING: &[u8] =
    br#"{"openai-codex":{"type":"oauth","access":"a","refresh":"r","expires":1,"accountId":"x"}}"#;

fn assert_document_keeps(path: &Path, raw_entries: &[&str]) {
    let document = fs::read_to_string(path).expect("credential document reads");
    for entry in raw_entries {
        assert!(
            document.contains(entry),
            "{entry} was not kept byte for byte in {document}"
        );
    }
}

#[test]
fn credential_store_keeps_an_undecodable_endpoint_slot_through_subscription_writes() {
    let workspace = empty_workspace("credential-store-keep-endpoint-slot");
    let path = workspace.join("private/credentials.json");
    let store = CredentialStore::at(path.clone());
    store
        .replace(&credential(900_000))
        .expect("private document initializes");
    let subscription =
        serde_json::to_string(&credential(900_000)).expect("subscription record serializes");
    fs::write(
        &path,
        format!(
            r#"{{"openai-codex":{subscription},"openai-compatible":{BROKEN_ENDPOINT},"future-provider":{UNKNOWN_SLOT}}}"#
        ),
    )
    .expect("mixed document written");
    assert_eq!(
        store
            .read()
            .expect("a broken endpoint slot leaves the subscription"),
        Some(credential(900_000))
    );
    assert!(store.read_endpoint().is_err());

    store
        .replace(&credential(1_200_000))
        .expect("subscription login stores");
    assert_document_keeps(&path, &[BROKEN_ENDPOINT, UNKNOWN_SLOT]);
    assert!(store.read_endpoint().is_err());

    store
        .replace(&credential(300_000))
        .expect("near-expiry subscription stores");
    resolve(&store, |_| Ok(credential(900_000))).expect("subscription refreshes");
    assert_document_keeps(&path, &[BROKEN_ENDPOINT, UNKNOWN_SLOT]);

    assert!(store.logout().expect("subscription logs out"));
    assert_document_keeps(&path, &[BROKEN_ENDPOINT, UNKNOWN_SLOT]);

    store
        .replace_endpoint_with_clock(
            &endpoint_credential("https://models.example.test/v1", "sk-slot-fixture"),
            || Duration::ZERO,
            |_| {},
        )
        .expect("endpoint login replaces its own broken slot");
    assert!(
        store
            .read_endpoint()
            .expect("replaced slot reads")
            .is_some()
    );
    assert_document_keeps(&path, &[UNKNOWN_SLOT]);
}

#[test]
fn credential_store_keeps_an_undecodable_subscription_slot_through_endpoint_writes() {
    let workspace = empty_workspace("credential-store-keep-subscription-slot");
    let path = workspace.join("private/credentials.json");
    let store = CredentialStore::at(path.clone());
    let endpoint = endpoint_credential("https://models.example.test/v1", "sk-slot-fixture");
    store
        .replace_endpoint_with_clock(&endpoint, || Duration::ZERO, |_| {})
        .expect("private document initializes");
    let endpoint_entry = serde_json::to_string(&endpoint).expect("endpoint record serializes");
    fs::write(
        &path,
        format!(r#"{{"openai-codex":{BROKEN_SUBSCRIPTION},"openai-compatible":{endpoint_entry}}}"#),
    )
    .expect("mixed document written");
    assert!(store.read().is_err());

    store
        .replace_endpoint_with_clock(
            &endpoint_credential("https://models.example.test/v1", "sk-next-fixture"),
            || Duration::ZERO,
            |_| {},
        )
        .expect("endpoint login stores beside the broken subscription slot");
    assert_document_keeps(&path, &[BROKEN_SUBSCRIPTION]);
    assert!(store.read().is_err());

    assert!(
        store
            .logout_endpoint_with_clock(|| Duration::ZERO, |_| {})
            .expect("endpoint logs out")
    );
    assert_document_keeps(&path, &[BROKEN_SUBSCRIPTION]);
    assert!(store.read().is_err());
}

#[test]
fn credential_store_reads_absent_and_subscription_only_documents() {
    let workspace = empty_workspace("credential-store-readable-documents");
    let path = workspace.join("private/credentials.json");
    let store = CredentialStore::at(path.clone());
    assert_eq!(
        CredentialStore::at(workspace.join("missing/credentials.json"))
            .read()
            .expect("missing store is empty"),
        None
    );
    let subscription = credential(900_000);
    store
        .replace(&subscription)
        .expect("private document initializes");

    for (case, document, expected) in [
        ("empty object", b"{}".to_vec(), None),
        (
            "null subscription",
            br#"{"openai-codex":null}"#.to_vec(),
            None,
        ),
        ("unknown slot", br#"{"unknown":true}"#.to_vec(), None),
        (
            "subscription-only document",
            subscription_document(&subscription),
            Some(subscription.clone()),
        ),
    ] {
        fs::write(&path, document).expect("readable fixture written");
        assert_eq!(store.read().expect(case), expected, "{case}: subscription");
        assert_eq!(store.read_endpoint().expect(case), None, "{case}: endpoint");
    }
}

#[test]
fn credential_resolution_refreshes_only_a_near_expiry_record() {
    for (case, expires, expected_refreshes) in
        [("current", 900_000, 0), ("near-expiry", 300_000, 1)]
    {
        let workspace = empty_workspace(&format!("credential-resolution-{case}"));
        let store = CredentialStore::at(workspace.join("private/credentials.json"));
        let prior = credential(expires);
        store.replace(&prior).expect("prior credential stores");
        let refreshes = Cell::new(0usize);

        let resolved = resolve(&store, |refreshing| {
            refreshes.set(refreshes.get() + 1);
            assert_eq!(refreshing, &prior, "refresh receives the stored record");
            Ok(credential(900_000))
        })
        .expect("credential resolves");

        assert_eq!(refreshes.get(), expected_refreshes, "{case}");
        assert_eq!(resolved, credential(900_000), "{case}");
        assert_eq!(
            store.read().expect("resolved record reads"),
            Some(resolved),
            "{case}"
        );
    }
}

#[test]
fn credential_resolution_requires_reauthentication_without_refreshing() {
    let absent = empty_workspace("credential-resolution-absent");
    let absent_store = CredentialStore::at(absent.join("private/credentials.json"));
    let (_workspace, undecodable_store, _) =
        store_with_undecodable_record("credential-resolution-undecodable");

    for (case, store) in [
        ("absent", &absent_store),
        ("undecodable", &undecodable_store),
    ] {
        let refreshes = Cell::new(0usize);
        let error = resolve(store, |_| {
            refreshes.set(refreshes.get() + 1);
            Ok(credential(900_000))
        })
        .expect_err("the record requires authentication");

        assert!(
            matches!(error, RuntimeError::AuthenticationRequired(_)),
            "{case}"
        );
        assert_eq!(error.exit_code(), 65, "{case}");
        assert_eq!(refreshes.get(), 0, "{case}");
    }
}

#[test]
fn credential_refresh_failure_preserves_the_prior_record() {
    let workspace = empty_workspace("credential-refresh-failure");
    let store = CredentialStore::at(workspace.join("private/credentials.json"));
    let prior = credential(100);
    store.replace(&prior).expect("prior credential stores");

    assert!(
        resolve(&store, |_| Err(RuntimeError::Protocol(
            "refresh rejected".to_owned()
        )))
        .is_err()
    );
    assert_eq!(store.read().expect("prior remains"), Some(prior.clone()));

    assert!(
        resolve(&store, |_| {
            let mut invalid = credential(900_000);
            invalid.credential_type = "api-key".to_owned();
            Ok(invalid)
        })
        .is_err()
    );
    assert_eq!(store.read().expect("prior still remains"), Some(prior));
}

#[test]
fn credential_refresh_reuses_a_winner_after_lock_contention() {
    let workspace = empty_workspace("credential-refresh-race-winner");
    let path = workspace.join("private/credentials.json");
    let store = CredentialStore::at(path.clone());
    store
        .replace(&credential(300_000))
        .expect("near-expiry credential stores");
    let held = RefCell::new(Some(
        store
            .acquire_lock_for_test(|| Duration::ZERO, |_| {})
            .expect("competing refresh lock acquires"),
    ));
    let winner = credential(900_000);
    let waits = Cell::new(0usize);

    let resolved = store
        .resolve_with_clock(
            || Ok(1),
            |_| panic!("the losing refresh must reuse the winner's credential"),
            || Duration::ZERO,
            |_| {
                if waits.get() == 0 {
                    drop(held.borrow_mut().take());
                    fs::write(&path, subscription_document(&winner))
                        .expect("winner credential commits");
                }
                waits.set(waits.get() + 1);
            },
        )
        .expect("winner credential resolves");

    assert_eq!(resolved, winner);
    assert_eq!(waits.get(), 1);
}

fn store_with_undecodable_record(name: &str) -> (TempWorkspace, CredentialStore, PathBuf) {
    let workspace = empty_workspace(name);
    let path = workspace.join("private/credentials.json");
    let store = CredentialStore::at(path.clone());
    store.logout().expect("private empty document initializes");
    fs::write(&path, RECORD_WITHOUT_ROUTING).expect("undecodable record writes");
    assert!(store.read().is_err());
    (workspace, store, path)
}

#[test]
fn reauthentication_and_logout_replace_an_undecodable_record() {
    let (_workspace, store, _) = store_with_undecodable_record("credential-undecodable-replace");
    let replacement = credential(900_000);
    store
        .replace(&replacement)
        .expect("login replaces an undecodable record");
    assert_eq!(store.read().expect("replacement reads"), Some(replacement));

    let (_workspace, store, _) = store_with_undecodable_record("credential-undecodable-logout");
    assert!(
        store
            .logout()
            .expect("logout removes an undecodable record")
    );
    assert_eq!(store.read().expect("logged-out store reads"), None);
}

#[test]
fn credential_store_rejects_malformed_and_oversized_documents() {
    let workspace = empty_workspace("credential-store-bounds");
    let path = workspace.join("private/credentials.json");
    let store = CredentialStore::at(path.clone());
    store.logout().expect("private empty document initializes");
    let mut oversized = b"{}".to_vec();
    oversized.resize(CREDENTIAL_STORE_MAX_BYTES as usize + 1, b' ');

    for document in [
        b"not-json".to_vec(),
        br#"{"openai-codex":{"type":"oauth","access":"a","refresh":"r","expires":1,"accountId":"x"}}"#
            .to_vec(),
        br#"{"openai-codex":{"type":"api-key","access":"x","refresh":"x","expires":1,"accountId":"x"}}"#
            .to_vec(),
        oversized,
    ] {
        fs::write(&path, document).expect("invalid fixture write");
        assert!(store.read().is_err());
    }

    fs::remove_file(&path).expect("invalid fixture removed");
    fs::create_dir(&path).expect("directory fixture created");
    assert!(store.read().is_err());
}

#[test]
fn credential_store_does_not_report_metadata_errors_as_absence() {
    let workspace = empty_workspace("credential-store-metadata-error");
    let store = CredentialStore::at(workspace.join("a".repeat(256)).join("credentials.json"));

    assert!(store.read().is_err());
}

#[test]
fn credential_store_enforces_the_on_disk_size_limit_before_replacement() {
    let workspace = empty_workspace("credential-store-write-boundary");
    let path = workspace.join("private/credentials.json");
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
    let baseline = subscription_document(&value).len();
    let growth = target
        .checked_sub(baseline)
        .expect("target fits credential fields");
    let escaped_controls = growth / 5;
    let escaped_quotes = growth % 5;
    assert!(escaped_controls + escaped_quotes <= MAX_OAUTH_SECRET_BYTES);
    value.refresh = format!(
        "{}{}{}",
        "\u{1}".repeat(escaped_controls),
        "\"".repeat(escaped_quotes),
        "r".repeat(MAX_OAUTH_SECRET_BYTES - escaped_controls - escaped_quotes)
    );
    assert_eq!(subscription_document(&value).len(), target);
    value
}

#[test]
fn credential_mutation_fails_closed_for_an_unsafe_document() {
    use std::os::unix::fs::symlink;

    let (_workspace, store, path) = store_with_undecodable_record("credential-unsafe-mode");
    set_unix_mode(&path, 0o644);
    assert!(store.replace(&credential(900_000)).is_err());
    assert!(store.logout().is_err());
    assert_eq!(
        fs::read(&path).expect("unsafe document remains"),
        RECORD_WITHOUT_ROUTING
    );
    store
        .acquire_lock_for_test(|| Duration::ZERO, |_| {})
        .expect("failed mutations release the credential lock");

    let (_workspace, store, path) = store_with_undecodable_record("credential-unsafe-symlink");
    let target = path.with_file_name("target.json");
    fs::rename(&path, &target).expect("symlink target moves");
    symlink(&target, &path).expect("credential symlink creates");
    assert!(store.replace(&credential(900_000)).is_err());
    assert!(store.logout().is_err());
    assert_eq!(
        fs::read(&target).expect("symlink target remains"),
        RECORD_WITHOUT_ROUTING
    );
    assert!(
        fs::symlink_metadata(&path)
            .expect("credential symlink remains")
            .file_type()
            .is_symlink()
    );
}
