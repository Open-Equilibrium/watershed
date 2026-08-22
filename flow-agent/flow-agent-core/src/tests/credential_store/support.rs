use super::super::oauth_credential::jwt_with_account;
use crate::runtime::oauth_credential::CredentialRecord;
use std::{fs, path::Path};

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
