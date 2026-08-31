mod browser;
mod device;
mod protocol;
mod transport;

use crate::runtime::{
    credential_store::CredentialStore, oauth_credential::CredentialRecord, types::RuntimeError,
};
use std::{thread, time::Instant};

#[cfg(test)]
pub(crate) use browser::{
    BrowserLauncher, build_authorize_url, read_loopback_callback, read_loopback_callback_until,
    run_browser_login_with_components, system_browser_launcher, write_loopback_response,
};
#[cfg(test)]
pub(crate) use device::{
    device_login_remaining, poll_device_authorization_body_with_transport,
    request_device_authorization_body_with_transport, run_device_login_with_components,
    run_device_login_with_deadlines_and_clock,
};
#[cfg(test)]
pub(crate) use protocol::{
    base64url_encode, epoch_milliseconds, next_poll_interval, parse_device_authorization_body,
    parse_device_poll_body, parse_oauth_callback, parse_token_body, percent_encode,
    random_url_token,
};
#[cfg(test)]
pub(crate) use transport::{
    exchange_authorization_code_at, exchange_authorization_code_body_with_transport,
    post_json_with_deadlines, refresh_credential_at, refresh_credential_with_transport,
    send_auth_request_async,
};

pub(crate) const MAX_OAUTH_CALLBACK_HTTP_HEAD_BYTES: usize = 16_384;
pub(crate) const MAX_OAUTH_USER_CODE_BODY_BYTES: usize = 64 * 1024;
pub(crate) const MAX_OAUTH_DEVICE_POLL_BODY_BYTES: usize = 64 * 1024;
pub(crate) const MAX_OAUTH_TOKEN_BODY_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_OAUTH_POLL_INTERVAL_SECONDS: u64 = 60;
pub(crate) const MAX_OAUTH_EXPIRY_SECONDS: u64 = 86_400;

/// Interactive OpenAI Codex authentication route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthLoginMode {
    /// Open the system browser and receive a loopback OAuth callback.
    Browser,
    /// Present a device code and poll until the operator authorizes it.
    Device,
}

/// Redacted local authentication status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthStatus {
    /// Whether Flow Agent currently owns an OpenAI Codex credential record.
    pub authenticated: bool,
    /// Credential expiry as Unix epoch milliseconds, when authenticated.
    pub expires_epoch_milliseconds: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeviceAuthorization {
    pub(crate) device_auth_id: String,
    pub(crate) interval_seconds: u64,
    pub(crate) user_code: String,
    pub(crate) verification_uri: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DevicePoll {
    Authorization {
        authorization_code: String,
        code_verifier: String,
    },
    Pending,
    SlowDown,
}

/// Performs interactive OpenAI Codex authentication and atomically stores the result.
///
/// The callback receives the browser URL or device-code instruction before this function waits
/// for the operator. It must not persist the message as runtime history.
pub fn login_openai_codex(
    mode: AuthLoginMode,
    mut present: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<AuthStatus, RuntimeError> {
    let credential = match mode {
        AuthLoginMode::Browser => browser::browser_login(&mut present)?,
        AuthLoginMode::Device => device::device_login(&mut present)?,
    };
    store_login_credential(&CredentialStore::platform_default()?, credential)
}

pub(crate) fn store_login_credential(
    store: &CredentialStore,
    credential: CredentialRecord,
) -> Result<AuthStatus, RuntimeError> {
    store.replace(&credential)?;
    Ok(AuthStatus {
        authenticated: true,
        expires_epoch_milliseconds: Some(credential.expires),
    })
}

/// Returns redacted OpenAI Codex authentication status from Flow Agent's own store.
pub fn openai_codex_auth_status() -> Result<AuthStatus, RuntimeError> {
    auth_status_from_store(&CredentialStore::platform_default()?)
}

pub(crate) fn auth_status_from_store(store: &CredentialStore) -> Result<AuthStatus, RuntimeError> {
    let credential = store.read()?;
    Ok(AuthStatus {
        authenticated: credential.is_some(),
        expires_epoch_milliseconds: credential.map(|record| record.expires),
    })
}

/// Removes only Flow Agent's local OpenAI Codex credential record.
pub fn logout_openai_codex() -> Result<bool, RuntimeError> {
    logout_from_store(&CredentialStore::platform_default()?)
}

pub(crate) fn logout_from_store(store: &CredentialStore) -> Result<bool, RuntimeError> {
    store.logout()
}

pub(crate) fn resolve_openai_codex_credential() -> Result<CredentialRecord, RuntimeError> {
    let store = CredentialStore::platform_default()?;
    let now_epoch_milliseconds = protocol::epoch_milliseconds()?;
    let started = Instant::now();
    store.resolve_with_clock(
        now_epoch_milliseconds,
        |prior| transport::refresh_credential(prior, now_epoch_milliseconds),
        || started.elapsed(),
        thread::sleep,
    )
}
