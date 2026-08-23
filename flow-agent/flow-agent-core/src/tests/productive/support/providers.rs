use crate::runtime::{
    oauth_credential::CredentialRecord,
    openai_codex::{ProviderToolCall, ProviderTurn},
    productive::ProductiveProvider,
    types::RuntimeError,
};
use std::collections::VecDeque;
#[derive(Default)]
pub(in super::super) struct FakeProvider {
    pub(in super::super) bodies: Vec<serde_json::Value>,
    pub(in super::super) cancel: bool,
    pub(in super::super) cancel_after_response: bool,
    pub(in super::super) error_after_interrupt: bool,
}

pub(in super::super) struct ScriptedProvider {
    pub(in super::super) bodies: Vec<serde_json::Value>,
    pub(in super::super) turns: VecDeque<ProviderTurn>,
}

pub(in super::super) struct DefinitiveFailureProvider {
    pub(in super::super) bodies: Vec<serde_json::Value>,
}

pub(in super::super) fn single_tool_provider_turn(
    response_id: impl Into<String>,
    call_id: impl Into<String>,
) -> ProviderTurn {
    ProviderTurn {
        token_usage: None,
        response_id: response_id.into(),
        output_text: String::new(),
        retained_items: Vec::new(),
        tool_calls: vec![ProviderToolCall {
            call_id: call_id.into(),
            name: "echo".to_owned(),
            arguments: "{}".to_owned(),
        }],
    }
}

impl ProductiveProvider for DefinitiveFailureProvider {
    fn turn(
        &mut self,
        _credential: &CredentialRecord,
        body: &serde_json::Value,
    ) -> Result<ProviderTurn, RuntimeError> {
        self.bodies.push(body.clone());
        Err(RuntimeError::definitive_provider_error(
            Some(429),
            "provider capacity exhausted",
        ))
    }
}

impl ProductiveProvider for ScriptedProvider {
    fn turn(
        &mut self,
        _credential: &CredentialRecord,
        body: &serde_json::Value,
    ) -> Result<ProviderTurn, RuntimeError> {
        self.bodies.push(body.clone());
        self.turns
            .pop_front()
            .ok_or_else(|| RuntimeError::Protocol("scripted provider exhausted".to_owned()))
    }
}

impl ProductiveProvider for FakeProvider {
    fn turn(
        &mut self,
        _credential: &CredentialRecord,
        body: &serde_json::Value,
    ) -> Result<ProviderTurn, RuntimeError> {
        self.bodies.push(body.clone());
        if self.cancel {
            return Err(RuntimeError::Cancelled);
        }
        if self.error_after_interrupt {
            assert_eq!(
                crate::request_productive_interrupt(),
                crate::ProductiveInterruptAction::Cancel
            );
            return Err(RuntimeError::Protocol(
                "provider failed after cancellation won".to_owned(),
            ));
        }
        let turn = ProviderTurn {
            token_usage: None,
            response_id: "response-fixture".to_owned(),
            output_text: "{\"type\":\"string\",\"value\":\"productive\"}".to_owned(),
            retained_items: vec![serde_json::json!({
                "content": [],
                "id": "message-fixture",
                "role": "assistant",
                "type": "message"
            })],
            tool_calls: Vec::new(),
        };
        if self.cancel_after_response {
            assert_eq!(
                crate::request_productive_interrupt(),
                crate::ProductiveInterruptAction::Cancel
            );
        }
        Ok(turn)
    }
}
