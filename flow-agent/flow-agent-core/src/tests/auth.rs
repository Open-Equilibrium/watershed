mod browser;
mod device;
mod protocol;
mod transport;

use crate::{
    runtime::{
        auth::{endpoint_auth_status_from_store, store_endpoint_credential},
        credential_store::CredentialStore,
        endpoint_credential::EndpointCredential,
    },
    tests::helpers::empty_workspace,
};

pub(super) use super::oauth_credential::{
    credential_record, jwt_with_account, jwt_with_account_routing,
};

#[test]
fn endpoint_login_status_names_the_stored_base_url() {
    const BASE_URL: &str = "https://models.example.test/v1";
    let workspace = empty_workspace("endpoint-login-status");
    let store = CredentialStore::at(workspace.join("private/credentials.json"));
    let credential =
        EndpointCredential::new(BASE_URL, "sk-status-fixture").expect("endpoint fixture");

    let status = store_endpoint_credential(&store, credential).expect("endpoint stores");

    assert_eq!(status.base_url.as_deref(), Some(BASE_URL));
    assert_eq!(
        endpoint_auth_status_from_store(&store).expect("stored status"),
        status
    );
}

pub(super) fn token_body(access: &str, refresh: &str, expires_in: serde_json::Value) -> Vec<u8> {
    token_body_with_id(access, access, refresh, expires_in)
}

pub(super) fn token_body_with_id(
    access: &str,
    id: &str,
    refresh: &str,
    expires_in: serde_json::Value,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "access_token": access,
        "id_token": id,
        "expires_in": expires_in,
        "refresh_token": refresh,
    }))
    .expect("token JSON")
}
