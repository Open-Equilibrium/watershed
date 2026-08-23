use crate::runtime::types::RuntimeError;
use serde::Deserialize;

pub(crate) const MAX_OAUTH_FIELD_BYTES: usize = 8 * 1024;
pub(crate) const MAX_OAUTH_SECRET_BYTES: usize = 64 * 1024;
pub(crate) const OAUTH_CREDENTIAL_TYPE: &str = "oauth";

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct CredentialRecord {
    #[serde(rename = "type")]
    pub(crate) credential_type: String,
    pub(crate) access: String,
    pub(crate) refresh: String,
    pub(crate) expires: u64,
    #[serde(rename = "accountId")]
    pub(crate) account_id: String,
    #[serde(rename = "isFedramp")]
    pub(crate) is_fedramp: bool,
}

#[derive(Deserialize)]
struct JwtClaims {
    #[serde(rename = "https://api.openai.com/auth")]
    auth: JwtAuthClaim,
}

#[derive(Deserialize)]
struct AccessJwtClaims {
    exp: u64,
}

#[derive(Deserialize)]
struct JwtAuthClaim {
    chatgpt_account_id: String,
    #[serde(default)]
    chatgpt_account_is_fedramp: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccountRouting {
    pub(crate) account_id: String,
    pub(crate) is_fedramp: bool,
}

pub(crate) fn validate_credential_record(
    credential: &CredentialRecord,
) -> Result<(), RuntimeError> {
    if credential.credential_type != OAUTH_CREDENTIAL_TYPE || credential.expires == 0 {
        return Err(auth_protocol());
    }
    validate_field(&credential.access, MAX_OAUTH_SECRET_BYTES)?;
    validate_field(&credential.refresh, MAX_OAUTH_SECRET_BYTES)?;
    validate_field(&credential.account_id, MAX_OAUTH_FIELD_BYTES)?;
    Ok(())
}

pub(crate) fn account_routing_from_id_token(token: &str) -> Result<AccountRouting, RuntimeError> {
    let payload = jwt_payload(token)?;
    let claims: JwtClaims = serde_json::from_slice(&payload).map_err(|_| auth_protocol())?;
    validate_field(&claims.auth.chatgpt_account_id, MAX_OAUTH_FIELD_BYTES)?;
    Ok(AccountRouting {
        account_id: claims.auth.chatgpt_account_id,
        is_fedramp: claims.auth.chatgpt_account_is_fedramp,
    })
}

pub(crate) fn access_token_expiration_milliseconds(token: &str) -> Result<u64, RuntimeError> {
    let payload = jwt_payload(token)?;
    let claims: AccessJwtClaims = serde_json::from_slice(&payload).map_err(|_| auth_protocol())?;
    claims.exp.checked_mul(1_000).ok_or_else(auth_protocol)
}

fn jwt_payload(token: &str) -> Result<Vec<u8>, RuntimeError> {
    validate_field(token, MAX_OAUTH_SECRET_BYTES)?;
    let mut segments = token.split('.');
    let header = segments.next().ok_or_else(auth_protocol)?;
    let payload = segments.next().ok_or_else(auth_protocol)?;
    let signature = segments.next().ok_or_else(auth_protocol)?;
    if header.is_empty() || signature.is_empty() || segments.next().is_some() {
        return Err(auth_protocol());
    }
    base64url_decode(payload)
}

pub(crate) fn base64url_decode(value: &str) -> Result<Vec<u8>, RuntimeError> {
    if value.is_empty() || value.contains('=') {
        return Err(auth_protocol());
    }
    let mut decoded = Vec::with_capacity(value.len().saturating_mul(3) / 4 + 2);
    let mut accumulator = 0u32;
    let mut bits = 0u8;
    for byte in value.bytes() {
        accumulator = (accumulator << 6) | u32::from(base64url_value(byte)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            decoded.push((accumulator >> bits) as u8);
            accumulator &= (1u32 << bits).saturating_sub(1);
        }
    }
    if bits >= 6 || accumulator != 0 {
        return Err(auth_protocol());
    }
    Ok(decoded)
}

fn base64url_value(byte: u8) -> Result<u8, RuntimeError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'-' => Ok(62),
        b'_' => Ok(63),
        _ => Err(auth_protocol()),
    }
}

pub(crate) fn validate_field(value: &str, maximum: usize) -> Result<(), RuntimeError> {
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        return Err(auth_protocol());
    }
    Ok(())
}

pub(crate) fn auth_protocol() -> RuntimeError {
    RuntimeError::Protocol("authentication protocol failure".to_owned())
}
