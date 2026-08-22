use super::OPENAI_CODEX_PROVIDER_ID;
use crate::runtime::{digest::sha256_hex, types::RuntimeError};
use proto::parse_unique_json;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderToolCall {
    pub(crate) call_id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderTurn {
    pub(crate) response_id: String,
    pub(crate) output_text: String,
    pub(crate) retained_items: Vec<serde_json::Value>,
    pub(crate) tool_calls: Vec<ProviderToolCall>,
    pub(crate) token_usage: Option<ProviderTokenUsage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderTokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cache_write_tokens: Option<u64>,
}

pub(crate) fn derive_prompt_cache_key(conversation_id: &str, model: &str) -> String {
    let mut material = Vec::new();
    for component in [
        b"flow-prompt-cache-key-v0".as_slice(),
        OPENAI_CODEX_PROVIDER_ID.as_bytes(),
        conversation_id.as_bytes(),
        model.as_bytes(),
    ] {
        material.extend_from_slice(&(component.len() as u64).to_be_bytes());
        material.extend_from_slice(component);
    }
    sha256_hex(&material)
}

pub(crate) fn build_responses_request_body(
    model: &str,
    prompt_cache_key: &str,
    instructions: &str,
    input: &[serde_json::Value],
    tools: &[&core_script::ToolBlock],
) -> Result<serde_json::Value, RuntimeError> {
    let tools = tools
        .iter()
        .map(|tool| provider_tool_schema(tool))
        .collect::<Result<Vec<_>, _>>()?;
    let mut body = serde_json::json!({
        "include": ["reasoning.encrypted_content"],
        "input": input,
        "instructions": instructions,
        "model": model,
        "parallel_tool_calls": true,
        "prompt_cache_key": prompt_cache_key,
        "store": false,
        "stream": true,
        "text": {"verbosity": "low"},
        "tool_choice": "auto",
    });
    if !tools.is_empty() {
        body.as_object_mut()
            .expect("request body is an object")
            .insert("tools".to_owned(), serde_json::Value::Array(tools));
    }
    Ok(body)
}

pub(crate) fn responses_request_input_bytes(
    body: &serde_json::Value,
) -> Result<usize, RuntimeError> {
    let object = body.as_object().ok_or_else(|| {
        RuntimeError::Protocol("Responses request body must be an object".to_owned())
    })?;
    let input = object
        .get("input")
        .ok_or_else(|| RuntimeError::Protocol("Responses request body lacks input".to_owned()))?;
    let instructions = object.get("instructions").ok_or_else(|| {
        RuntimeError::Protocol("Responses request body lacks instructions".to_owned())
    })?;
    let mut context = serde_json::Map::from_iter([
        ("input".to_owned(), input.clone()),
        ("instructions".to_owned(), instructions.clone()),
    ]);
    if let Some(tools) = object.get("tools") {
        context.insert("tools".to_owned(), tools.clone());
    }
    proto::canonical_json(&serde_json::Value::Object(context))
        .map(|context| context.len())
        .map_err(|error| {
            RuntimeError::Protocol(format!(
                "Responses request model input cannot be serialized: {error}"
            ))
        })
}

pub(crate) fn decode_responses_turn(
    events: &[serde_json::Value],
) -> Result<ProviderTurn, RuntimeError> {
    let mut created_id = None;
    let mut completed_id = None;
    let mut completed = 0usize;
    let mut output_text = String::new();
    let mut retained_items = Vec::new();
    let mut tool_calls = Vec::new();
    let mut tool_call_ids = BTreeSet::new();
    let mut token_usage = None;

    for event in events {
        if completed != 0 {
            return Err(provider_protocol(
                "response contains data after its successful terminal event",
            ));
        }
        let event_type = required_string(event, "type", "event type")?;
        match event_type {
            "response.created" => {
                let id = response_id(event)?;
                if created_id.replace(id.to_owned()).is_some() {
                    return Err(provider_protocol(
                        "response contains multiple created events",
                    ));
                }
            }
            "response.output_text.delta" => {
                let delta = required_string(event, "delta", "output text delta")?;
                output_text.push_str(delta);
            }
            "response.output_item.done" => {
                let item = event
                    .get("item")
                    .ok_or_else(|| provider_protocol("output_item.done lacks item"))?
                    .clone();
                if item.get("type").and_then(serde_json::Value::as_str) == Some("function_call") {
                    let call = decode_tool_call(&item)?;
                    if !tool_call_ids.insert(call.call_id.clone()) {
                        return Err(provider_protocol(format!(
                            "response repeated function call id {}",
                            call.call_id
                        )));
                    }
                    tool_calls.push(call);
                }
                retained_items.push(item);
            }
            "response.completed" => {
                completed += 1;
                completed_id = Some(response_id(event)?.to_owned());
                token_usage = decode_token_usage(event)?;
            }
            "response.error" | "response.failed" | "response.incomplete" | "error" => {
                return Err(RuntimeError::definitive_provider_error(
                    None,
                    provider_error_message(event)
                        .unwrap_or_else(|| format!("provider ended with {event_type}")),
                ));
            }
            _ => {}
        }
    }
    if completed != 1 {
        return Err(provider_protocol(format!(
            "response requires exactly one successful terminal event; observed {completed}"
        )));
    }
    let response_id = completed_id.expect("one completed event has an id");
    if let Some(created_id) = created_id
        && created_id != response_id
    {
        return Err(provider_protocol(
            "created and completed response ids do not match",
        ));
    }
    Ok(ProviderTurn {
        response_id,
        output_text,
        retained_items,
        tool_calls,
        token_usage,
    })
}

pub(crate) fn provider_arguments_to_flow_value(
    tool: &core_script::ToolBlock,
    arguments: &str,
) -> Result<core_script::FlowValue, RuntimeError> {
    let value = parse_unique_json(arguments).map_err(|error| {
        provider_protocol(format!(
            "tool arguments are not duplicate-free JSON: {error}"
        ))
    })?;
    let object = value
        .as_object()
        .ok_or_else(|| provider_protocol("tool arguments must be a JSON object"))?;
    let mut values = BTreeMap::new();
    for (name, value) in object {
        let parameter = tool
            .allowed_parameters
            .iter()
            .find(|parameter| parameter.name == *name)
            .ok_or_else(|| {
                provider_protocol(format!(
                    "tool {} received undeclared parameter {name}",
                    tool.identity.id
                ))
            })?;
        if value.is_null() && !parameter.required {
            continue;
        }
        values.insert(name.clone(), provider_parameter_value(parameter, value)?);
    }
    for parameter in &tool.allowed_parameters {
        if parameter.required && !values.contains_key(&parameter.name) {
            return Err(provider_protocol(format!(
                "tool {} lacks required parameter {}",
                tool.identity.id, parameter.name
            )));
        }
    }
    let value = core_script::FlowValue::Map(values);
    core_script::validate_flow_value(&value)
        .map_err(|error| provider_protocol(format!("tool arguments are invalid: {error}")))?;
    Ok(value)
}

pub(crate) fn output_contract_instruction(
    contract: &core_script::ValueContract,
) -> Result<String, RuntimeError> {
    let contract = serde_json::to_value(contract).map_err(RuntimeError::Json)?;
    let contract = proto::canonical_json(&contract).map_err(|error| {
        provider_protocol(format!(
            "Phase output contract cannot be serialized: {error}"
        ))
    })?;
    Ok(format!(
        "When no tool call remains, return exactly one canonical JSON flow-value-v0 value matching this closed contract; return no prose or Markdown: {contract}"
    ))
}

pub(super) fn provider_error_message(value: &serde_json::Value) -> Option<String> {
    [
        "/error/message",
        "/response/error/message",
        "/message",
        "/response/message",
        "/error",
    ]
    .into_iter()
    .find_map(|pointer| value.pointer(pointer).and_then(serde_json::Value::as_str))
    .map(str::to_owned)
}

fn provider_tool_schema(tool: &core_script::ToolBlock) -> Result<serde_json::Value, RuntimeError> {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for parameter in &tool.allowed_parameters {
        let mut schema = serde_json::Map::new();
        match parameter.value_type {
            core_script::ParameterValueType::None => {
                schema.insert("const".to_owned(), serde_json::Value::Bool(true));
                schema.insert("type".to_owned(), serde_json::json!("boolean"));
            }
            core_script::ParameterValueType::String
            | core_script::ParameterValueType::WorkspaceRelativePath => {
                schema.insert("type".to_owned(), serde_json::json!("string"));
                if let Some(maximum) = parameter.max_length {
                    schema.insert("maxLength".to_owned(), serde_json::json!(maximum));
                }
            }
            core_script::ParameterValueType::Integer => {
                schema.insert("type".to_owned(), serde_json::json!("integer"));
                if let Some(minimum) = parameter.min {
                    schema.insert("minimum".to_owned(), serde_json::json!(minimum));
                }
                if let Some(maximum) = parameter.max {
                    schema.insert("maximum".to_owned(), serde_json::json!(maximum));
                }
            }
            core_script::ParameterValueType::Enum => {
                schema.insert("type".to_owned(), serde_json::json!("string"));
                schema.insert(
                    "enum".to_owned(),
                    serde_json::json!(parameter.allowed_values),
                );
            }
        }
        if !parameter.required {
            make_provider_schema_nullable(&mut schema);
        }
        required.push(parameter.name.clone());
        properties.insert(parameter.name.clone(), serde_json::Value::Object(schema));
    }
    Ok(serde_json::json!({
        "description": format!("Watershed Tool: {}", tool.identity.name),
        "name": tool.identity.id,
        "parameters": {
            "additionalProperties": false,
            "properties": properties,
            "required": required,
            "type": "object"
        },
        "strict": true,
        "type": "function"
    }))
}

fn make_provider_schema_nullable(schema: &mut serde_json::Map<String, serde_json::Value>) {
    let value_type = schema
        .remove("type")
        .expect("every provider parameter schema has a type");
    schema.insert(
        "type".to_owned(),
        serde_json::Value::Array(vec![value_type, serde_json::json!("null")]),
    );
    if let Some(constant) = schema.remove("const") {
        schema.insert(
            "enum".to_owned(),
            serde_json::Value::Array(vec![constant, serde_json::Value::Null]),
        );
    } else if let Some(serde_json::Value::Array(values)) = schema.get_mut("enum") {
        values.push(serde_json::Value::Null);
    }
}

fn provider_parameter_value(
    parameter: &core_script::AllowedParameter,
    value: &serde_json::Value,
) -> Result<core_script::FlowValue, RuntimeError> {
    let value = match parameter.value_type {
        core_script::ParameterValueType::None if value.as_bool() == Some(true) => {
            Ok(core_script::FlowValue::Boolean(true))
        }
        core_script::ParameterValueType::String
        | core_script::ParameterValueType::WorkspaceRelativePath
        | core_script::ParameterValueType::Enum => value
            .as_str()
            .map(|value| core_script::FlowValue::String(value.to_owned()))
            .ok_or_else(|| {
                provider_protocol(format!("parameter {} must be a string", parameter.name))
            }),
        core_script::ParameterValueType::Integer => value
            .as_i64()
            .map(|value| core_script::FlowValue::Integer(value.to_string()))
            .ok_or_else(|| {
                provider_protocol(format!(
                    "parameter {} must be an i64 integer",
                    parameter.name
                ))
            }),
        core_script::ParameterValueType::None => Err(provider_protocol(format!(
            "flag parameter {} must be true",
            parameter.name
        ))),
    }?;
    crate::runtime::tool_runner::validate_parameter_value(parameter, &value).map_err(|error| {
        provider_protocol(format!(
            "parameter {} violates its declared contract: {error:?}",
            parameter.name
        ))
    })?;
    Ok(value)
}

fn decode_tool_call(item: &serde_json::Value) -> Result<ProviderToolCall, RuntimeError> {
    Ok(ProviderToolCall {
        call_id: required_non_empty_string(item, "call_id", "function call id")?.to_owned(),
        name: required_non_empty_string(item, "name", "function name")?.to_owned(),
        arguments: required_string(item, "arguments", "function arguments")?.to_owned(),
    })
}

fn response_id(event: &serde_json::Value) -> Result<&str, RuntimeError> {
    event
        .get("response")
        .and_then(|response| response.get("id"))
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| provider_protocol("response event lacks a non-empty response id"))
}

fn decode_token_usage(
    event: &serde_json::Value,
) -> Result<Option<ProviderTokenUsage>, RuntimeError> {
    let response = event
        .get("response")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| provider_protocol("response event lacks a response object"))?;
    let Some(usage) = response.get("usage") else {
        return Ok(None);
    };
    if usage.is_null() {
        return Ok(None);
    }
    let usage = usage
        .as_object()
        .ok_or_else(|| provider_protocol("response usage must be an object"))?;
    let total_input_tokens = optional_u64(usage, "input_tokens", "input token count")?;
    let output_tokens = optional_u64(usage, "output_tokens", "output token count")?;
    let details = match usage.get("input_tokens_details") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_object()
                .ok_or_else(|| provider_protocol("input token details must be an object"))?,
        ),
    };
    let cache_read_tokens = details
        .map(|details| optional_u64(details, "cached_tokens", "cache-read token count"))
        .transpose()?
        .flatten();
    let cache_write_tokens = details
        .map(|details| optional_u64(details, "cache_write_tokens", "cache-write token count"))
        .transpose()?
        .flatten();
    let cached_tokens = cache_read_tokens
        .unwrap_or(0)
        .checked_add(cache_write_tokens.unwrap_or(0))
        .ok_or_else(|| provider_protocol("cached input token count overflowed"))?;
    let input_tokens = total_input_tokens
        .map(|total| {
            total
                .checked_sub(cached_tokens)
                .ok_or_else(|| provider_protocol("cached input tokens exceed total input tokens"))
        })
        .transpose()?;
    Ok(Some(ProviderTokenUsage {
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
    }))
}

fn optional_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    label: &str,
) -> Result<Option<u64>, RuntimeError> {
    match object.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| provider_protocol(format!("{label} must be a u64 integer"))),
    }
}

fn required_string<'a>(
    value: &'a serde_json::Value,
    field: &str,
    label: &str,
) -> Result<&'a str, RuntimeError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| provider_protocol(format!("{label} must be a string")))
}

fn required_non_empty_string<'a>(
    value: &'a serde_json::Value,
    field: &str,
    label: &str,
) -> Result<&'a str, RuntimeError> {
    let string = required_string(value, field, label)?;
    if string.is_empty() {
        return Err(provider_protocol(format!("{label} must not be empty")));
    }
    Ok(string)
}

fn provider_protocol(message: impl Into<String>) -> RuntimeError {
    RuntimeError::Protocol(format!("OpenAI Codex provider: {}", message.into()))
}
