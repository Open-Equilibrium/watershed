use crate::runtime::{
    openai_codex::{ProviderToolCall, ProviderTurn},
    types::RuntimeError,
};
use std::{env, ffi::OsStr};

const ACCEPTANCE_ENV: &str = "FLOW_AGENT_M12_INSTALL_ACCEPTANCE";
const ACCEPTANCE_MODEL: &str = "gpt-m12-install-acceptance";
const TOOL_CALL_ID: &str = "m12-install-acceptance-tool";

pub(super) fn maybe_provider_turn(
    body: &serde_json::Value,
) -> Result<Option<ProviderTurn>, RuntimeError> {
    if !acceptance_enabled(env::var_os(ACCEPTANCE_ENV).as_deref())? {
        return Ok(None);
    }
    acceptance_provider_turn(body).map(Some)
}

fn acceptance_enabled(value: Option<&OsStr>) -> Result<bool, RuntimeError> {
    match value {
        None => Ok(false),
        Some(value) if value == OsStr::new("1") => Ok(true),
        Some(_) => Err(protocol("acceptance provider switch must equal 1")),
    }
}

fn acceptance_provider_turn(body: &serde_json::Value) -> Result<ProviderTurn, RuntimeError> {
    if body.get("model").and_then(serde_json::Value::as_str) != Some(ACCEPTANCE_MODEL) {
        return Err(protocol("acceptance provider requires its dedicated model"));
    }
    if !body
        .get("tools")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool.get("name").and_then(serde_json::Value::as_str) == Some("echo"))
        })
    {
        return Err(protocol("acceptance Flow did not expose the echo Tool"));
    }
    let input = body
        .get("input")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| protocol("acceptance provider input must be an array"))?;
    let outputs = input
        .iter()
        .filter(|item| {
            item.get("type").and_then(serde_json::Value::as_str) == Some("function_call_output")
        })
        .collect::<Vec<_>>();
    if outputs.is_empty() {
        let retained = serde_json::json!({
            "arguments": "{}",
            "call_id": TOOL_CALL_ID,
            "name": "echo",
            "type": "function_call",
        });
        return Ok(ProviderTurn {
            token_usage: None,
            response_id: "m12-install-acceptance-tool-response".to_owned(),
            output_text: String::new(),
            retained_items: vec![retained],
            tool_calls: vec![ProviderToolCall {
                call_id: TOOL_CALL_ID.to_owned(),
                name: "echo".to_owned(),
                arguments: "{}".to_owned(),
            }],
        });
    }
    if outputs.len() != 1
        || outputs[0]
            .get("call_id")
            .and_then(serde_json::Value::as_str)
            != Some(TOOL_CALL_ID)
        || !outputs[0]
            .get("output")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|output| output.contains("flow-tool-result-v0"))
    {
        return Err(protocol(
            "acceptance provider requires the completed echo Tool result",
        ));
    }
    Ok(ProviderTurn {
        token_usage: None,
        response_id: "m12-install-acceptance-final-response".to_owned(),
        output_text: "{\"type\":\"string\",\"value\":\"after-tool\"}".to_owned(),
        retained_items: vec![serde_json::json!({
            "content": [],
            "id": "m12-install-acceptance-final-message",
            "role": "assistant",
            "type": "message",
        })],
        tool_calls: Vec::new(),
    })
}

fn protocol(message: &str) -> RuntimeError {
    RuntimeError::Protocol(format!("M1.2 install acceptance: {message}"))
}

#[cfg(test)]
mod tests {
    use super::{acceptance_enabled, acceptance_provider_turn};
    use std::ffi::OsStr;

    fn request(input: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "input": input,
            "model": "gpt-m12-install-acceptance",
            "tools": [{"name": "echo", "type": "function"}],
        })
    }

    #[test]
    fn provider_completes_only_after_the_productive_tool_result() {
        let tool_turn = acceptance_provider_turn(&request(serde_json::json!([])))
            .expect("initial acceptance turn");
        assert!(tool_turn.output_text.is_empty());
        assert_eq!(tool_turn.tool_calls.len(), 1);
        assert_eq!(tool_turn.tool_calls[0].name, "echo");

        let final_turn = acceptance_provider_turn(&request(serde_json::json!([
            {
                "arguments": "{}",
                "call_id": "m12-install-acceptance-tool",
                "name": "echo",
                "type": "function_call"
            },
            {
                "call_id": "m12-install-acceptance-tool",
                "output": "{\"schema\":\"flow-tool-result-v0\"}",
                "type": "function_call_output"
            }
        ])))
        .expect("final acceptance turn");
        assert!(final_turn.tool_calls.is_empty());
        assert_eq!(
            final_turn.output_text,
            "{\"type\":\"string\",\"value\":\"after-tool\"}"
        );
    }

    #[test]
    fn provider_gate_requires_the_explicit_switch_and_model() {
        assert!(!acceptance_enabled(None).expect("absent switch"));
        assert!(acceptance_enabled(Some(OsStr::new("0"))).is_err());
        assert!(acceptance_enabled(Some(OsStr::new("1"))).expect("explicit switch"));

        let mut wrong_model = request(serde_json::json!([]));
        wrong_model["model"] = serde_json::json!("gpt-production");
        assert!(acceptance_provider_turn(&wrong_model).is_err());
    }
}
