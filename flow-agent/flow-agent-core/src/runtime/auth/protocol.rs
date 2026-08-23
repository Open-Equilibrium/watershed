use super::{
    DeviceAuthorization, DevicePoll, MAX_OAUTH_DEVICE_POLL_BODY_BYTES, MAX_OAUTH_EXPIRY_SECONDS,
    MAX_OAUTH_POLL_INTERVAL_SECONDS, MAX_OAUTH_TOKEN_BODY_BYTES, MAX_OAUTH_USER_CODE_BODY_BYTES,
};
use crate::runtime::{
    oauth_credential::{
        CredentialRecord, MAX_OAUTH_FIELD_BYTES, MAX_OAUTH_SECRET_BYTES, OAUTH_CREDENTIAL_TYPE,
        access_token_expiration_milliseconds, account_routing_from_id_token, auth_protocol,
        validate_field,
    },
    types::RuntimeError,
};
use serde::Deserialize;
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn random_url_token(bytes: usize) -> Result<String, RuntimeError> {
    let mut random = vec![0_u8; bytes];
    fill_random(&mut random)?;
    Ok(base64url_encode(&random))
}

#[cfg(unix)]
fn fill_random(bytes: &mut [u8]) -> Result<(), RuntimeError> {
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(bytes))
        .map_err(|_| auth_protocol())
}

#[cfg(windows)]
fn fill_random(bytes: &mut [u8]) -> Result<(), RuntimeError> {
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };
    let length = u32::try_from(bytes.len()).map_err(|_| auth_protocol())?;
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            length,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(auth_protocol())
    }
}

pub(crate) fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

pub(crate) fn epoch_milliseconds() -> Result<u64, RuntimeError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .ok_or_else(auth_protocol)
}

pub(crate) fn parse_oauth_callback(
    query: &str,
    expected_state: &str,
) -> Result<String, RuntimeError> {
    let mut state = None;
    let mut code = None;
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').ok_or_else(auth_protocol)?;
        let name = percent_decode(name)?;
        let value = percent_decode(value)?;
        match name.as_str() {
            "state" => set_once(&mut state, value)?,
            "code" => set_once(&mut code, value)?,
            _ => {}
        }
    }
    let state = state.ok_or_else(auth_protocol)?;
    if !constant_time_equal(state.as_bytes(), expected_state.as_bytes()) {
        return Err(auth_protocol());
    }
    let code = code.ok_or_else(auth_protocol)?;
    if code.is_empty() || code.contains('\0') {
        return Err(auth_protocol());
    }
    Ok(code)
}

pub(crate) fn parse_device_authorization_body(
    body: &[u8],
) -> Result<DeviceAuthorization, RuntimeError> {
    let source: DeviceAuthorizationSource =
        parse_bounded_body(body, MAX_OAUTH_USER_CODE_BODY_BYTES)?;
    validate_field(&source.device_auth_id, MAX_OAUTH_FIELD_BYTES)?;
    validate_field(&source.user_code, MAX_OAUTH_FIELD_BYTES)?;
    validate_field(&source.verification_uri, MAX_OAUTH_FIELD_BYTES)?;
    Ok(DeviceAuthorization {
        device_auth_id: source.device_auth_id,
        interval_seconds: bounded_positive_seconds(
            &source.interval,
            MAX_OAUTH_POLL_INTERVAL_SECONDS,
        )?,
        user_code: source.user_code,
        verification_uri: source.verification_uri,
    })
}

pub(crate) fn parse_device_poll_body(body: &[u8]) -> Result<DevicePoll, RuntimeError> {
    let source: DevicePollSource = parse_bounded_body(body, MAX_OAUTH_DEVICE_POLL_BODY_BYTES)?;
    match (
        source.error.as_deref(),
        source.authorization_code,
        source.code_verifier,
    ) {
        (Some("authorization_pending"), None, None) => Ok(DevicePoll::Pending),
        (Some("slow_down"), None, None) => Ok(DevicePoll::SlowDown),
        (None, Some(authorization_code), Some(code_verifier)) => {
            validate_field(&authorization_code, MAX_OAUTH_FIELD_BYTES)?;
            validate_field(&code_verifier, MAX_OAUTH_FIELD_BYTES)?;
            Ok(DevicePoll::Authorization {
                authorization_code,
                code_verifier,
            })
        }
        _ => Err(auth_protocol()),
    }
}

pub(crate) fn parse_token_body(
    body: &[u8],
    now_epoch_milliseconds: u64,
) -> Result<CredentialRecord, RuntimeError> {
    let source: TokenSource = parse_bounded_body(body, MAX_OAUTH_TOKEN_BODY_BYTES)?;
    validate_field(&source.access_token, MAX_OAUTH_SECRET_BYTES)?;
    validate_field(&source.id_token, MAX_OAUTH_SECRET_BYTES)?;
    validate_field(&source.refresh_token, MAX_OAUTH_SECRET_BYTES)?;
    let expires = expiration_from_duration(&source.expires_in, now_epoch_milliseconds)?;
    let routing = account_routing_from_id_token(&source.id_token)?;
    Ok(CredentialRecord {
        credential_type: OAUTH_CREDENTIAL_TYPE.to_owned(),
        access: source.access_token,
        refresh: source.refresh_token,
        expires,
        account_id: routing.account_id,
        is_fedramp: routing.is_fedramp,
    })
}

pub(crate) fn parse_refresh_token_body(
    body: &[u8],
    prior: &CredentialRecord,
    now_epoch_milliseconds: u64,
) -> Result<CredentialRecord, RuntimeError> {
    let source: RefreshTokenSource = parse_bounded_body(body, MAX_OAUTH_TOKEN_BODY_BYTES)?;
    let mut credential = prior.clone();

    let access_token = source.access_token.ok_or_else(auth_protocol)?;
    validate_field(&access_token, MAX_OAUTH_SECRET_BYTES)?;
    credential.expires = match source.expires_in.as_ref() {
        Some(expires_in) => expiration_from_duration(expires_in, now_epoch_milliseconds)?,
        None => {
            let expires = access_token_expiration_milliseconds(&access_token)?;
            let maximum = MAX_OAUTH_EXPIRY_SECONDS
                .checked_mul(1_000)
                .and_then(|duration| now_epoch_milliseconds.checked_add(duration))
                .ok_or_else(auth_protocol)?;
            if expires <= now_epoch_milliseconds || expires > maximum {
                return Err(auth_protocol());
            }
            expires
        }
    };
    credential.access = access_token;

    if let Some(refresh_token) = source.refresh_token {
        validate_field(&refresh_token, MAX_OAUTH_SECRET_BYTES)?;
        credential.refresh = refresh_token;
    }
    if let Some(id_token) = source.id_token {
        validate_field(&id_token, MAX_OAUTH_SECRET_BYTES)?;
        let routing = account_routing_from_id_token(&id_token)?;
        if routing.account_id != prior.account_id {
            return Err(auth_protocol());
        }
        credential.is_fedramp = routing.is_fedramp;
    }
    Ok(credential)
}

pub(crate) fn next_poll_interval(
    current_seconds: u64,
    slow_down: bool,
) -> Result<u64, RuntimeError> {
    if current_seconds == 0 || current_seconds > MAX_OAUTH_POLL_INTERVAL_SECONDS {
        return Err(auth_protocol());
    }
    if !slow_down {
        return Ok(current_seconds);
    }
    current_seconds
        .checked_add(5)
        .filter(|interval| *interval <= MAX_OAUTH_POLL_INTERVAL_SECONDS)
        .ok_or_else(auth_protocol)
}

pub(crate) fn base64url_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        encoded.push(char::from(ALPHABET[usize::from(first >> 2)]));
        let second_prefix = (first & 0b0000_0011) << 4;
        match chunk {
            [_] => encoded.push(char::from(ALPHABET[usize::from(second_prefix)])),
            [_, second] => {
                encoded.push(char::from(
                    ALPHABET[usize::from(second_prefix | (second >> 4))],
                ));
                encoded.push(char::from(
                    ALPHABET[usize::from((second & 0b0000_1111) << 2)],
                ));
            }
            [_, second, third] => {
                encoded.push(char::from(
                    ALPHABET[usize::from(second_prefix | (second >> 4))],
                ));
                encoded.push(char::from(
                    ALPHABET[usize::from(((second & 0b0000_1111) << 2) | (third >> 6))],
                ));
                encoded.push(char::from(ALPHABET[usize::from(third & 0b0011_1111)]));
            }
            _ => unreachable!("chunks are one to three bytes"),
        }
    }
    encoded
}

#[derive(Deserialize)]
struct DeviceAuthorizationSource {
    device_auth_id: String,
    interval: serde_json::Value,
    user_code: String,
    verification_uri: String,
}

#[derive(Deserialize)]
struct DevicePollSource {
    #[serde(default)]
    authorization_code: Option<String>,
    #[serde(default)]
    code_verifier: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct TokenSource {
    access_token: String,
    id_token: String,
    expires_in: serde_json::Value,
    refresh_token: String,
}

#[derive(Deserialize)]
struct RefreshTokenSource {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<serde_json::Value>,
    #[serde(default)]
    refresh_token: Option<String>,
}

fn parse_bounded_body<T>(body: &[u8], maximum: usize) -> Result<T, RuntimeError>
where
    T: for<'de> Deserialize<'de>,
{
    if body.len() > maximum {
        return Err(auth_protocol());
    }
    serde_json::from_slice(body).map_err(|_| auth_protocol())
}

fn bounded_positive_seconds(value: &serde_json::Value, maximum: u64) -> Result<u64, RuntimeError> {
    let seconds = match value {
        serde_json::Value::Number(number) => number.as_u64().ok_or_else(auth_protocol)?,
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() || !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(auth_protocol());
            }
            trimmed.parse::<u64>().map_err(|_| auth_protocol())?
        }
        _ => return Err(auth_protocol()),
    };
    if seconds == 0 || seconds > maximum {
        return Err(auth_protocol());
    }
    Ok(seconds)
}

fn expiration_from_duration(
    expires_in: &serde_json::Value,
    now_epoch_milliseconds: u64,
) -> Result<u64, RuntimeError> {
    bounded_positive_seconds(expires_in, MAX_OAUTH_EXPIRY_SECONDS)?
        .checked_mul(1_000)
        .and_then(|duration| now_epoch_milliseconds.checked_add(duration))
        .ok_or_else(auth_protocol)
}

fn set_once(target: &mut Option<String>, value: String) -> Result<(), RuntimeError> {
    if target.replace(value).is_some() {
        return Err(auth_protocol());
    }
    Ok(())
}

fn percent_decode(value: &str) -> Result<String, RuntimeError> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' => {
                let high = *bytes.get(index + 1).ok_or_else(auth_protocol)?;
                let low = *bytes.get(index + 2).ok_or_else(auth_protocol)?;
                decoded.push((hex_nibble(high)? << 4) | hex_nibble(low)?);
                index += 2;
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8(decoded).map_err(|_| auth_protocol())
}

fn hex_nibble(byte: u8) -> Result<u8, RuntimeError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(auth_protocol()),
    }
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}
