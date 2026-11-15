use crate::runtime::{
    oauth_credential::{MAX_OAUTH_FIELD_BYTES, MAX_OAUTH_SECRET_BYTES},
    types::RuntimeError,
};
use std::{fmt, net::IpAddr};

/// Stable provider identifier of the OpenAI-compatible endpoint provider (ADR-0263).
pub const OPENAI_COMPATIBLE_PROVIDER_ID: &str = "openai-compatible";
pub(crate) const ENDPOINT_CREDENTIAL_TYPE: &str = "api-key";

/// An endpoint key kept with the base URL it was issued for, so it only ever goes to that host.
#[derive(Clone, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct EndpointCredential {
    #[serde(rename = "type")]
    pub(crate) credential_type: String,
    #[serde(rename = "baseUrl")]
    pub(crate) base_url: String,
    pub(crate) key: String,
}

impl EndpointCredential {
    pub(crate) fn new(base_url: &str, key: &str) -> Result<Self, RuntimeError> {
        let credential = Self {
            credential_type: ENDPOINT_CREDENTIAL_TYPE.to_owned(),
            base_url: base_url.to_owned(),
            key: key.to_owned(),
        };
        validate_endpoint_credential(&credential)?;
        Ok(credential)
    }

    /// The Responses endpoint, `<base URL>/responses`.
    pub(crate) fn responses_url(&self) -> Result<String, RuntimeError> {
        let mut url = parse_base_url(&self.base_url)?;
        url.path_segments_mut()
            .map_err(|()| base_url_rule())?
            .pop_if_empty()
            .push("responses");
        Ok(url.to_string())
    }
}

impl fmt::Debug for EndpointCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndpointCredential")
            .field("credential_type", &self.credential_type)
            .field("base_url", &self.base_url)
            .field("key", &"<redacted>")
            .finish()
    }
}

pub(crate) fn validate_endpoint_credential(
    credential: &EndpointCredential,
) -> Result<(), RuntimeError> {
    if credential.credential_type != ENDPOINT_CREDENTIAL_TYPE {
        return Err(RuntimeError::Usage(
            "endpoint credential has an unsupported type".to_owned(),
        ));
    }
    parse_base_url(&credential.base_url)?;
    let key = &credential.key;
    if key.is_empty()
        || key.len() > MAX_OAUTH_SECRET_BYTES
        || !key.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(RuntimeError::Usage(
            "API key must be a nonempty visible ASCII value".to_owned(),
        ));
    }
    Ok(())
}

/// Accepts `https` URLs, and `http` only for a loopback IP literal, without credentials,
/// query or fragment; loopback names such as `localhost` are not accepted.
fn parse_base_url(raw: &str) -> Result<reqwest::Url, RuntimeError> {
    if raw.is_empty() || raw.len() > MAX_OAUTH_FIELD_BYTES {
        return Err(base_url_rule());
    }
    let url = reqwest::Url::parse(raw).map_err(|_| base_url_rule())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(base_url_rule());
    }
    let loopback_ip = url
        .host_str()
        .map(|host| host.trim_start_matches('[').trim_end_matches(']'))
        .and_then(|host| host.parse::<IpAddr>().ok())
        .is_some_and(|ip| ip.is_loopback());
    match url.scheme() {
        "https" => Ok(url),
        "http" if loopback_ip => Ok(url),
        _ => Err(base_url_rule()),
    }
}

fn base_url_rule() -> RuntimeError {
    RuntimeError::Usage(
        "base URL must be https, or http on a loopback IP address, without credentials, query or fragment"
            .to_owned(),
    )
}
