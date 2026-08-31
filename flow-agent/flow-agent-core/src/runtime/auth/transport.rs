use super::{MAX_OAUTH_TOKEN_BODY_BYTES, protocol};
use crate::runtime::{
    deadlines::{
        AUTH_HTTP_DEADLINES, HttpDeadlines, await_deadline, block_on_network, build_http_client,
    },
    oauth_credential::{CredentialRecord, auth_protocol},
    openai_codex::OPENAI_CODEX_CLIENT_ID,
    types::RuntimeError,
};
use std::time::Duration;

const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

pub(super) fn exchange_authorization_code(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<CredentialRecord, RuntimeError> {
    exchange_authorization_code_with_deadlines(code, verifier, redirect_uri, AUTH_HTTP_DEADLINES)
}

pub(super) fn exchange_authorization_code_with_deadlines(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    deadlines: HttpDeadlines,
) -> Result<CredentialRecord, RuntimeError> {
    exchange_authorization_code_for_endpoint(
        OPENAI_TOKEN_URL,
        code,
        verifier,
        redirect_uri,
        deadlines,
    )
}

#[cfg(test)]
pub(crate) fn exchange_authorization_code_at(
    endpoint: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    deadlines: HttpDeadlines,
) -> Result<CredentialRecord, RuntimeError> {
    exchange_authorization_code_for_endpoint(endpoint, code, verifier, redirect_uri, deadlines)
}

fn exchange_authorization_code_for_endpoint(
    endpoint: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    deadlines: HttpDeadlines,
) -> Result<CredentialRecord, RuntimeError> {
    let body = exchange_authorization_code_body_with_transport(
        endpoint,
        code,
        verifier,
        redirect_uri,
        deadlines,
        post_form_with_deadlines,
    )?;
    protocol::parse_token_body(&body, protocol::epoch_milliseconds()?)
}

pub(crate) fn exchange_authorization_code_body_with_transport(
    endpoint: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    deadlines: HttpDeadlines,
    transport: impl FnOnce(&str, &[(&str, &str)], usize, HttpDeadlines) -> Result<Vec<u8>, RuntimeError>,
) -> Result<Vec<u8>, RuntimeError> {
    transport(
        endpoint,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", OPENAI_CODEX_CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ],
        MAX_OAUTH_TOKEN_BODY_BYTES,
        deadlines,
    )
}

pub(super) fn refresh_credential(
    prior: &CredentialRecord,
    now_epoch_milliseconds: u64,
) -> Result<CredentialRecord, RuntimeError> {
    refresh_credential_for_endpoint(
        OPENAI_TOKEN_URL,
        prior,
        now_epoch_milliseconds,
        AUTH_HTTP_DEADLINES,
    )
}

#[cfg(test)]
pub(crate) fn refresh_credential_at(
    endpoint: &str,
    prior: &CredentialRecord,
    now_epoch_milliseconds: u64,
    deadlines: HttpDeadlines,
) -> Result<CredentialRecord, RuntimeError> {
    refresh_credential_for_endpoint(endpoint, prior, now_epoch_milliseconds, deadlines)
}

fn refresh_credential_for_endpoint(
    endpoint: &str,
    prior: &CredentialRecord,
    now_epoch_milliseconds: u64,
    deadlines: HttpDeadlines,
) -> Result<CredentialRecord, RuntimeError> {
    refresh_credential_with_transport(
        endpoint,
        prior,
        now_epoch_milliseconds,
        |endpoint, fields, maximum| post_form_with_deadlines(endpoint, fields, maximum, deadlines),
    )
}

pub(crate) fn refresh_credential_with_transport(
    endpoint: &str,
    prior: &CredentialRecord,
    now_epoch_milliseconds: u64,
    transport: impl FnOnce(&str, &[(&str, &str)], usize) -> Result<Vec<u8>, RuntimeError>,
) -> Result<CredentialRecord, RuntimeError> {
    let body = transport(
        endpoint,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", prior.refresh.as_str()),
            ("client_id", OPENAI_CODEX_CLIENT_ID),
        ],
        MAX_OAUTH_TOKEN_BODY_BYTES,
    )?;
    protocol::parse_refresh_token_body(&body, prior, now_epoch_milliseconds)
}

fn post_form_with_deadlines(
    endpoint: &str,
    fields: &[(&str, &str)],
    maximum: usize,
    deadlines: HttpDeadlines,
) -> Result<Vec<u8>, RuntimeError> {
    let client = build_http_client(deadlines).map_err(|_| auth_protocol())?;
    send_auth_request(client.post(endpoint).form(fields), maximum, deadlines)
}

pub(super) fn post_json(
    endpoint: &str,
    body: &serde_json::Value,
    maximum: usize,
) -> Result<Vec<u8>, RuntimeError> {
    post_json_with_deadlines(endpoint, body, maximum, AUTH_HTTP_DEADLINES)
}

pub(crate) fn post_json_with_deadlines(
    endpoint: &str,
    body: &serde_json::Value,
    maximum: usize,
    deadlines: HttpDeadlines,
) -> Result<Vec<u8>, RuntimeError> {
    let client = build_http_client(deadlines).map_err(|_| auth_protocol())?;
    send_auth_request(client.post(endpoint).json(body), maximum, deadlines)
}

fn send_auth_request(
    request: reqwest::RequestBuilder,
    maximum: usize,
    deadlines: HttpDeadlines,
) -> Result<Vec<u8>, RuntimeError> {
    block_on_network(send_auth_request_async(request, maximum, deadlines))?
}

pub(crate) async fn send_auth_request_async(
    request: reqwest::RequestBuilder,
    maximum: usize,
    deadlines: HttpDeadlines,
) -> Result<Vec<u8>, RuntimeError> {
    let response = await_deadline(deadlines.header, request.send())
        .await
        .map_err(|_| auth_protocol())?
        .map_err(|_| auth_protocol())?;
    read_auth_response(response, maximum, deadlines.read).await
}

async fn read_auth_response(
    mut response: reqwest::Response,
    maximum: usize,
    body_deadline: Duration,
) -> Result<Vec<u8>, RuntimeError> {
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > maximum as u64)
    {
        return Err(auth_protocol());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = await_deadline(body_deadline, response.chunk())
        .await
        .map_err(|_| auth_protocol())?
        .map_err(|_| auth_protocol())?
    {
        let next = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(auth_protocol)?;
        if next > maximum {
            return Err(auth_protocol());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
