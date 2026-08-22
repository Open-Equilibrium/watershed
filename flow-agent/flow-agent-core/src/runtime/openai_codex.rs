mod protocol;
mod transport;

/// Stable provider identifier used by configuration and CLI commands.
pub const OPENAI_CODEX_PROVIDER_ID: &str = "openai-codex";
pub(crate) const OPENAI_CODEX_ORIGINATOR: &str = "flow-agent";
pub(crate) const OPENAI_CODEX_RESPONSES_URL: &str =
    "https://chatgpt.com/backend-api/codex/responses";
pub(crate) const OPENAI_CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

pub(crate) use protocol::{
    ProviderTokenUsage, ProviderToolCall, ProviderTurn, build_responses_request_body,
    derive_prompt_cache_key, output_contract_instruction, provider_arguments_to_flow_value,
    responses_request_input_bytes,
};
pub(crate) use transport::request_responses_at;

#[cfg(test)]
pub(crate) use protocol::decode_responses_turn;
#[cfg(test)]
pub(crate) use transport::{
    request_responses_async, request_responses_at_with_deadlines_and_cancellation,
    request_responses_with_client_async,
};
