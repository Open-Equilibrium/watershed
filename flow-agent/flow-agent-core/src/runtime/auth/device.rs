use super::{
    DeviceAuthorization, DevicePoll, MAX_OAUTH_DEVICE_POLL_BODY_BYTES,
    MAX_OAUTH_USER_CODE_BODY_BYTES, protocol, transport,
};
use crate::runtime::{
    deadlines::{AUTH_HTTP_DEADLINES, DEVICE_POLL_OVERALL_DEADLINE, HttpDeadlines},
    oauth_credential::{CredentialRecord, auth_protocol},
    openai_codex::OPENAI_CODEX_CLIENT_ID,
    types::RuntimeError,
};
use std::{
    thread,
    time::{Duration, Instant},
};

const OPENAI_DEVICE_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const OPENAI_DEVICE_POLL_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const OPENAI_DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const DEVICE_REDIRECT_URL: &str = "https://auth.openai.com/deviceauth/callback";

pub(super) fn device_login(
    present: &mut impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<CredentialRecord, RuntimeError> {
    let started = Instant::now();
    run_device_login_with_deadlines_and_clock(
        present,
        || request_device_authorization_body_with_transport(transport::post_json),
        |authorization, deadlines| {
            poll_device_authorization_body_with_transport(
                authorization,
                deadlines,
                transport::post_json_with_deadlines,
            )
        },
        transport::exchange_authorization_code_with_deadlines,
        thread::sleep,
        || started.elapsed(),
    )
}

pub(crate) fn request_device_authorization_body_with_transport(
    transport: impl FnOnce(&str, &serde_json::Value, usize) -> Result<Vec<u8>, RuntimeError>,
) -> Result<Vec<u8>, RuntimeError> {
    transport(
        OPENAI_DEVICE_CODE_URL,
        &serde_json::json!({"client_id": OPENAI_CODEX_CLIENT_ID}),
        MAX_OAUTH_USER_CODE_BODY_BYTES,
    )
}

pub(crate) fn poll_device_authorization_body_with_transport(
    authorization: &DeviceAuthorization,
    deadlines: HttpDeadlines,
    transport: impl FnOnce(
        &str,
        &serde_json::Value,
        usize,
        HttpDeadlines,
    ) -> Result<Vec<u8>, RuntimeError>,
) -> Result<Vec<u8>, RuntimeError> {
    transport(
        OPENAI_DEVICE_POLL_URL,
        &serde_json::json!({
            "device_auth_id": &authorization.device_auth_id,
            "user_code": &authorization.user_code,
        }),
        MAX_OAUTH_DEVICE_POLL_BODY_BYTES,
        deadlines,
    )
}

#[cfg(test)]
pub(crate) fn run_device_login_with_components<P, S, Q, E, W>(
    present: &mut P,
    start: S,
    mut poll: Q,
    mut exchange: E,
    wait: W,
) -> Result<CredentialRecord, RuntimeError>
where
    P: FnMut(&str) -> Result<(), RuntimeError>,
    S: FnMut() -> Result<Vec<u8>, RuntimeError>,
    Q: FnMut(&DeviceAuthorization) -> Result<Vec<u8>, RuntimeError>,
    E: FnMut(&str, &str, &str) -> Result<CredentialRecord, RuntimeError>,
    W: FnMut(Duration),
{
    let started = Instant::now();
    run_device_login_with_deadlines_and_clock(
        present,
        start,
        |authorization, _| poll(authorization),
        |code, verifier, redirect_uri, _| exchange(code, verifier, redirect_uri),
        wait,
        || started.elapsed(),
    )
}

pub(crate) fn run_device_login_with_deadlines_and_clock<P, S, Q, E, W, N>(
    present: &mut P,
    mut start: S,
    mut poll: Q,
    mut exchange: E,
    mut wait: W,
    mut elapsed: N,
) -> Result<CredentialRecord, RuntimeError>
where
    P: FnMut(&str) -> Result<(), RuntimeError>,
    S: FnMut() -> Result<Vec<u8>, RuntimeError>,
    Q: FnMut(&DeviceAuthorization, HttpDeadlines) -> Result<Vec<u8>, RuntimeError>,
    E: FnMut(&str, &str, &str, HttpDeadlines) -> Result<CredentialRecord, RuntimeError>,
    W: FnMut(Duration),
    N: FnMut() -> Duration,
{
    let body = start()?;
    let authorization = protocol::parse_device_authorization_body(&body)?;
    if authorization.verification_uri != OPENAI_DEVICE_VERIFICATION_URL {
        return Err(auth_protocol());
    }
    present(&format!(
        "Open {OPENAI_DEVICE_VERIFICATION_URL} and enter code {}",
        authorization.user_code
    ))?;
    let mut interval = authorization.interval_seconds;
    loop {
        let remaining = device_login_remaining(elapsed())?;
        wait(Duration::from_secs(interval).min(remaining));
        let body = poll(
            &authorization,
            device_login_http_deadlines(device_login_remaining(elapsed())?),
        )?;
        match protocol::parse_device_poll_body(&body)? {
            DevicePoll::Pending => {}
            DevicePoll::SlowDown => interval = protocol::next_poll_interval(interval, true)?,
            DevicePoll::Authorization {
                authorization_code,
                code_verifier,
            } => {
                return exchange(
                    &authorization_code,
                    &code_verifier,
                    DEVICE_REDIRECT_URL,
                    device_login_http_deadlines(device_login_remaining(elapsed())?),
                );
            }
        }
    }
}

pub(crate) fn device_login_remaining(elapsed: Duration) -> Result<Duration, RuntimeError> {
    let remaining = DEVICE_POLL_OVERALL_DEADLINE
        .checked_sub(elapsed)
        .ok_or_else(auth_protocol)?;
    if remaining.is_zero() {
        return Err(auth_protocol());
    }
    Ok(remaining)
}

fn device_login_http_deadlines(remaining: Duration) -> HttpDeadlines {
    HttpDeadlines {
        connect: AUTH_HTTP_DEADLINES.connect.min(remaining),
        header: AUTH_HTTP_DEADLINES.header.min(remaining),
        read: AUTH_HTTP_DEADLINES.read.min(remaining),
        overall: AUTH_HTTP_DEADLINES.overall.min(remaining),
    }
}
