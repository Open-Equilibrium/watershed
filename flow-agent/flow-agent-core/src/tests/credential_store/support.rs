use super::super::oauth_credential::jwt_with_account;
use crate::runtime::{
    credential_store::CredentialStore, endpoint_credential::EndpointCredential,
    oauth_credential::CredentialRecord, types::RuntimeError,
};
use std::{fs, os::unix::fs::PermissionsExt as _, path::Path, time::Duration};

pub(super) fn endpoint_credential(base_url: &str, key: &str) -> EndpointCredential {
    EndpointCredential::new(base_url, key).expect("fixture endpoint credential is valid")
}

pub(super) fn credential(expires: u64) -> CredentialRecord {
    CredentialRecord {
        credential_type: "oauth".to_owned(),
        access: jwt_with_account("account"),
        refresh: "refresh".to_owned(),
        expires,
        account_id: "account".to_owned(),
        is_fedramp: false,
    }
}

pub(super) fn subscription_document(credential: &CredentialRecord) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "openai-codex": credential }))
        .expect("subscription document serializes")
}

pub(super) fn resolve(
    store: &CredentialStore,
    refresh: impl FnMut(&CredentialRecord) -> Result<CredentialRecord, RuntimeError>,
) -> Result<CredentialRecord, RuntimeError> {
    store.resolve_with_clock(|| Ok(1), refresh, || Duration::ZERO, |_| {})
}

pub(super) fn unix_mode(path: &Path) -> u32 {
    fs::metadata(path)
        .expect("credential metadata reads")
        .permissions()
        .mode()
        & 0o777
}

pub(super) fn set_unix_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("credential mode changes");
}

pub(super) fn assert_no_credential_staging_files(parent: &Path) {
    assert!(
        fs::read_dir(parent)
            .expect("credential parent reads")
            .all(|entry| !entry
                .expect("credential parent entry reads")
                .file_name()
                .to_string_lossy()
                .starts_with(".credentials-"))
    );
}
