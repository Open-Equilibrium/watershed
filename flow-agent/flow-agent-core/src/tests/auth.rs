mod browser;
mod device;
mod protocol;
mod transport;

pub(super) use super::oauth_credential::{jwt_with_account, jwt_with_account_routing};

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
