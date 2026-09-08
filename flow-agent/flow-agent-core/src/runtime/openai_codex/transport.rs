use super::OPENAI_CODEX_ORIGINATOR;
use super::protocol::{ProviderTurn, decode_responses_turn, provider_error_message};
use crate::runtime::{
    deadlines::{
        AwaitInterruption, HttpDeadlines, RESPONSES_HTTP_DEADLINES, await_deadline_or_cancellation,
        block_on_network, build_http_client,
    },
    responses::SseDecoder,
    types::RuntimeError,
};
use std::sync::atomic::AtomicBool;

const MAX_PROVIDER_ERROR_BODY_BYTES: usize = 64 * 1024;

pub(crate) fn request_responses_at(
    endpoint: &str,
    credential: &crate::runtime::oauth_credential::CredentialRecord,
    body: &serde_json::Value,
) -> Result<ProviderTurn, RuntimeError> {
    request_responses_at_with_cancellation(
        endpoint,
        credential,
        body,
        crate::runtime::cancellation::productive_cancellation(),
    )
}

fn request_responses_at_with_cancellation(
    endpoint: &str,
    credential: &crate::runtime::oauth_credential::CredentialRecord,
    body: &serde_json::Value,
    cancelled: &AtomicBool,
) -> Result<ProviderTurn, RuntimeError> {
    request_responses_at_with_deadlines_and_cancellation(
        endpoint,
        credential,
        body,
        RESPONSES_HTTP_DEADLINES,
        cancelled,
    )
}

pub(crate) fn request_responses_at_with_deadlines_and_cancellation(
    endpoint: &str,
    credential: &crate::runtime::oauth_credential::CredentialRecord,
    body: &serde_json::Value,
    deadlines: HttpDeadlines,
    cancelled: &AtomicBool,
) -> Result<ProviderTurn, RuntimeError> {
    block_on_network(request_responses_async(
        endpoint, credential, body, deadlines, cancelled,
    ))?
}

pub(crate) async fn request_responses_async(
    endpoint: &str,
    credential: &crate::runtime::oauth_credential::CredentialRecord,
    body: &serde_json::Value,
    deadlines: HttpDeadlines,
    cancelled: &AtomicBool,
) -> Result<ProviderTurn, RuntimeError> {
    let client = build_http_client(deadlines).map_err(|_| {
        RuntimeError::definitive_provider_error(None, "HTTP client construction failed")
    })?;
    request_responses_with_client_async(client, endpoint, credential, body, deadlines, cancelled)
        .await
}

pub(crate) async fn request_responses_with_client_async(
    client: reqwest::Client,
    endpoint: &str,
    credential: &crate::runtime::oauth_credential::CredentialRecord,
    body: &serde_json::Value,
    deadlines: HttpDeadlines,
    cancelled: &AtomicBool,
) -> Result<ProviderTurn, RuntimeError> {
    let mut response = await_deadline_or_cancellation(
        deadlines.header,
        cancelled,
        routed_request(
            client
                .post(endpoint)
                .bearer_auth(&credential.access)
                .header("chatgpt-account-id", &credential.account_id)
                .header("originator", OPENAI_CODEX_ORIGINATOR)
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .header(
                    reqwest::header::USER_AGENT,
                    concat!("flow-agent/", env!("CARGO_PKG_VERSION")),
                )
                .json(body),
            credential.is_fedramp,
        )
        .send(),
    )
    .await
    .map_err(|interruption| match interruption {
        AwaitInterruption::Cancelled => RuntimeError::Cancelled,
        AwaitInterruption::DeadlineElapsed => {
            RuntimeError::uncertain_provider_error("Responses header deadline elapsed")
        }
    })?
    .map_err(|_| RuntimeError::uncertain_provider_error("Responses request failed"))?;
    let status = response.status();
    if !status.is_success() {
        let message = read_provider_error_message(&mut response, deadlines, cancelled).await;
        return Err(RuntimeError::definitive_provider_error(
            Some(status.as_u16()),
            message,
        ));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
    {
        return Err(RuntimeError::definitive_provider_error(
            Some(status.as_u16()),
            "Responses reply is not text/event-stream",
        ));
    }
    let mut decoder = SseDecoder::default();
    while let Some(chunk) =
        await_deadline_or_cancellation(deadlines.read, cancelled, response.chunk())
            .await
            .map_err(|interruption| match interruption {
                AwaitInterruption::Cancelled => RuntimeError::Cancelled,
                AwaitInterruption::DeadlineElapsed => {
                    RuntimeError::uncertain_provider_error("Responses stream failed")
                }
            })?
            .map_err(|_| RuntimeError::uncertain_provider_error("Responses stream failed"))?
    {
        decoder
            .push(&chunk)
            .map_err(|error| RuntimeError::uncertain_provider_error(error.to_string()))?;
        if decoder.terminal_sentinel() {
            break;
        }
    }
    let parsed = decoder
        .finish()
        .map_err(|error| RuntimeError::uncertain_provider_error(error.to_string()))?;
    if !parsed.terminal_sentinel {
        return Err(RuntimeError::uncertain_provider_error(
            "Responses stream ended without terminal sentinel",
        ));
    }
    decode_responses_turn(&parsed.values).map_err(|error| {
        if error.provider_failure().is_some() {
            error
        } else {
            RuntimeError::definitive_provider_error(None, error.to_string())
        }
    })
}

fn routed_request(request: reqwest::RequestBuilder, is_fedramp: bool) -> reqwest::RequestBuilder {
    if is_fedramp {
        request.header("X-OpenAI-Fedramp", "true")
    } else {
        request
    }
}

async fn read_provider_error_message(
    response: &mut reqwest::Response,
    deadlines: HttpDeadlines,
    cancelled: &AtomicBool,
) -> String {
    let mut body = Vec::new();
    while body.len() < MAX_PROVIDER_ERROR_BODY_BYTES {
        let chunk =
            match await_deadline_or_cancellation(deadlines.read, cancelled, response.chunk()).await
            {
                Ok(Ok(Some(chunk))) => chunk,
                _ => break,
            };
        let remaining = MAX_PROVIDER_ERROR_BODY_BYTES - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() > remaining {
            break;
        }
    }
    direct_provider_message(&body)
}

fn direct_provider_message(body: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body)
        && let Some(message) = provider_error_message(&value)
    {
        return message;
    }
    String::from_utf8_lossy(body).trim().to_owned()
}
