use crate::runtime::{
    oauth_credential::CredentialRecord, openai_codex::ProviderTurn, productive::ProductiveProvider,
    types::RuntimeError,
};

mod continuation;
mod entrypoints;
mod productive_resume;

#[derive(Default)]
pub(super) struct SessionProvider {
    pub(super) calls: usize,
}

impl ProductiveProvider for SessionProvider {
    fn turn(
        &mut self,
        _credential: &CredentialRecord,
        _body: &serde_json::Value,
    ) -> Result<ProviderTurn, RuntimeError> {
        self.calls = self.calls.saturating_add(1);
        Ok(ProviderTurn {
            token_usage: None,
            response_id: format!("session-response-{}", self.calls),
            output_text: "{\"type\":\"string\",\"value\":\"continued\"}".to_owned(),
            retained_items: vec![serde_json::json!({
                "content": [],
                "id": format!("session-message-{}", self.calls),
                "role": "assistant",
                "type": "message"
            })],
            tool_calls: Vec::new(),
        })
    }
}

pub(super) fn session_credential() -> CredentialRecord {
    CredentialRecord {
        credential_type: "oauth".to_owned(),
        access: "fixture-access".to_owned(),
        refresh: "fixture-refresh".to_owned(),
        expires: u64::MAX,
        account_id: "fixture-account".to_owned(),
        is_fedramp: false,
    }
}
