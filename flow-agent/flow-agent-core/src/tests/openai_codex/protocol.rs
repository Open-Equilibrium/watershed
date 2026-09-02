use crate::runtime::openai_codex::{
    ProviderTokenUsage, ProviderToolCall, build_responses_request_body, decode_responses_turn,
    derive_prompt_cache_key, output_contract_instruction, provider_arguments_to_flow_value,
};

fn test_tool() -> core_script::ToolBlock {
    core_script::ToolBlock {
        identity: core_script::BlockIdentity {
            id: "inspect".to_owned(),
            name: "Inspect".to_owned(),
        },
        tool_kind: core_script::ToolKind::PredefinedCommand,
        command: core_script::ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv: Vec::new(),
        },
        script_runtime: None,
        script_body: None,
        allowed_parameters: vec![
            core_script::AllowedParameter {
                name: "--path".to_owned(),
                value_type: core_script::ParameterValueType::WorkspaceRelativePath,
                required: true,
                allowed_values: Vec::new(),
                value_pattern: Some("^[a-z]+(?:/[a-z]+)*$".to_owned()),
                max_length: Some(128),
                min: None,
                max: None,
            },
            core_script::AllowedParameter {
                name: "--limit".to_owned(),
                value_type: core_script::ParameterValueType::Integer,
                required: false,
                allowed_values: Vec::new(),
                value_pattern: None,
                max_length: None,
                min: Some(1),
                max: Some(10),
            },
        ],
        max_concurrent_processes_and_threads: 16,
        runtime_profile: core_script::ToolRuntimeProfile::Exact,
        read_only_mounts: vec!["workspace".to_owned()],
        writable_mounts: Vec::new(),
        network: core_script::NetworkPolicy::Deny(core_script::NetworkDeny),
    }
}

#[test]
fn productive_request_is_stateless_and_exposes_only_declared_tools() {
    let tool = test_tool();
    let body = build_responses_request_body(
        "gpt-fixture",
        "cache-key",
        "Follow the Phase instructions.",
        &[serde_json::json!({"role":"user","content":"inspect"})],
        &[&tool],
    )
    .expect("request body");

    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["model"], "gpt-fixture");
    assert_eq!(body["prompt_cache_key"], "cache-key");
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["parallel_tool_calls"], true);
    assert_eq!(body["tools"].as_array().expect("tools").len(), 1);
    assert_eq!(body["tools"][0]["name"], "inspect");
    assert_eq!(
        body["tools"][0]["parameters"]["additionalProperties"],
        false
    );
    assert_eq!(
        body["tools"][0]["parameters"]["required"],
        serde_json::json!(["--path", "--limit"])
    );
    assert_eq!(
        body["tools"][0]["parameters"]["properties"]["--limit"]["type"],
        serde_json::json!(["integer", "null"])
    );
    assert!(body.get("previous_response_id").is_none());
}

#[test]
fn provider_tool_schema_does_not_advertise_local_rust_regexes() {
    let body = build_responses_request_body(
        "gpt-fixture",
        "cache-key",
        "Follow the Phase instructions.",
        &[],
        &[&test_tool()],
    )
    .expect("request body");

    assert!(
        body["tools"][0]["parameters"]["properties"]["--path"]
            .get("pattern")
            .is_none()
    );
}

#[test]
fn every_optional_provider_parameter_kind_is_strict_and_nullable() {
    let mut tool = test_tool();
    tool.allowed_parameters = [
        ("--flag", core_script::ParameterValueType::None),
        ("--label", core_script::ParameterValueType::String),
        ("--mode", core_script::ParameterValueType::Enum),
        (
            "--path",
            core_script::ParameterValueType::WorkspaceRelativePath,
        ),
        ("--limit", core_script::ParameterValueType::Integer),
    ]
    .into_iter()
    .map(|(name, value_type)| core_script::AllowedParameter {
        name: name.to_owned(),
        value_type,
        required: false,
        allowed_values: if name == "--mode" {
            vec!["safe".to_owned()]
        } else {
            Vec::new()
        },
        value_pattern: None,
        max_length: None,
        min: None,
        max: None,
    })
    .collect();
    let body = build_responses_request_body(
        "gpt-fixture",
        "cache-key",
        "Follow the Phase instructions.",
        &[],
        &[&tool],
    )
    .expect("request body");
    let parameters = &body["tools"][0]["parameters"];

    assert_eq!(
        parameters["required"],
        serde_json::json!(["--flag", "--label", "--mode", "--path", "--limit"])
    );
    for (name, value_type) in [
        ("--flag", "boolean"),
        ("--label", "string"),
        ("--mode", "string"),
        ("--path", "string"),
        ("--limit", "integer"),
    ] {
        assert_eq!(
            parameters["properties"][name]["type"],
            serde_json::json!([value_type, "null"])
        );
    }
    assert_eq!(
        parameters["properties"]["--flag"]["enum"],
        serde_json::json!([true, null])
    );
    assert_eq!(
        parameters["properties"]["--mode"]["enum"],
        serde_json::json!(["safe", null])
    );
}

#[test]
fn prompt_cache_key_is_stable_opaque_and_conversation_model_scoped() {
    let first = derive_prompt_cache_key("review", "gpt-fixture");
    let repeated = derive_prompt_cache_key("review", "gpt-fixture");
    let other_conversation = derive_prompt_cache_key("other", "gpt-fixture");
    let other_model = derive_prompt_cache_key("review", "gpt-different");

    assert_eq!(first, repeated);
    assert_eq!(first.len(), 64);
    assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(!first.contains("review"));
    assert!(!first.contains("gpt-fixture"));
    assert_ne!(first, other_conversation);
    assert_ne!(first, other_model);
}

#[test]
fn productive_response_retains_order_and_decodes_text_and_tool_calls() {
    let first = serde_json::json!({"type":"message","id":"msg_1","role":"assistant","content":[]});
    let second = serde_json::json!({
        "type":"function_call",
        "id":"fc_1",
        "call_id":"call_1",
        "name":"inspect",
        "arguments":"{\"--path\":\"src\"}"
    });
    let turn = decode_responses_turn(&[
        serde_json::json!({"type":"response.created","response":{"id":"resp_1"}}),
        serde_json::json!({"type":"response.output_text.delta","delta":"done"}),
        serde_json::json!({"type":"response.output_item.done","item":first.clone()}),
        serde_json::json!({"type":"response.output_item.done","item":second.clone()}),
        serde_json::json!({"type":"response.completed","response":{"id":"resp_1"}}),
    ])
    .expect("completed turn");

    assert_eq!(turn.response_id, "resp_1");
    assert_eq!(turn.output_text, "done");
    assert_eq!(turn.token_usage, None);
    assert_eq!(turn.retained_items, vec![first, second]);
    assert_eq!(
        turn.tool_calls,
        vec![ProviderToolCall {
            call_id: "call_1".to_owned(),
            name: "inspect".to_owned(),
            arguments: "{\"--path\":\"src\"}".to_owned(),
        }]
    );
}

#[test]
fn productive_response_rejects_empty_or_duplicate_tool_call_identities() {
    let function_call = |call_id: &str, name: &str, arguments: &str| {
        serde_json::json!({
            "type":"response.output_item.done",
            "item":{
                "type":"function_call",
                "call_id":call_id,
                "name":name,
                "arguments":arguments
            }
        })
    };
    for tool_calls in [
        vec![function_call("", "inspect", "{}")],
        vec![function_call("call_1", "", "{}")],
        vec![
            function_call("call_1", "inspect", "{}"),
            function_call("call_1", "inspect", "{}"),
        ],
        vec![
            function_call("call_1", "inspect", "{}"),
            function_call("call_1", "inspect", "{\"--path\":\"src\"}"),
        ],
    ] {
        let events = tool_calls
            .into_iter()
            .chain([serde_json::json!({
                "type":"response.completed",
                "response":{"id":"resp_1"}
            })])
            .collect::<Vec<_>>();
        assert!(decode_responses_turn(&events).is_err());
    }
}

#[test]
fn productive_response_decodes_optional_bounded_cache_usage() {
    let turn = decode_responses_turn(&[serde_json::json!({
        "type":"response.completed",
        "response":{
            "id":"resp_1",
            "usage":{
                "input_tokens":12,
                "input_tokens_details":{
                    "cached_tokens":4,
                    "cache_write_tokens":3
                },
                "output_tokens":6
            }
        }
    })])
    .expect("completed turn with usage");

    assert_eq!(
        turn.token_usage,
        Some(ProviderTokenUsage {
            input_tokens: Some(5),
            output_tokens: Some(6),
            cache_read_tokens: Some(4),
            cache_write_tokens: Some(3),
        })
    );

    let zero = decode_responses_turn(&[serde_json::json!({
        "type":"response.completed",
        "response":{
            "id":"resp_2",
            "usage":{
                "input_tokens":0,
                "input_tokens_details":{
                    "cached_tokens":0,
                    "cache_write_tokens":0
                },
                "output_tokens":0
            }
        }
    })])
    .expect("zero usage");
    assert_eq!(
        zero.token_usage,
        Some(ProviderTokenUsage {
            input_tokens: Some(0),
            output_tokens: Some(0),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
        })
    );

    let bounded = decode_responses_turn(&[serde_json::json!({
        "type":"response.completed",
        "response":{
            "id":"resp_3",
            "usage":{
                "input_tokens":u64::MAX,
                "output_tokens":u64::MAX
            }
        }
    })])
    .expect("bounded u64 usage");
    assert_eq!(
        bounded
            .token_usage
            .as_ref()
            .and_then(|usage| usage.input_tokens),
        Some(u64::MAX)
    );
    assert_eq!(
        bounded
            .token_usage
            .as_ref()
            .and_then(|usage| usage.output_tokens),
        Some(u64::MAX)
    );
}

#[test]
fn productive_response_rejects_invalid_or_contradictory_cache_usage() {
    for usage in [
        serde_json::json!({"input_tokens":-1}),
        serde_json::json!({"output_tokens":1.5}),
        serde_json::json!({"input_tokens_details":"invalid"}),
        serde_json::json!({
            "input_tokens":1,
            "input_tokens_details":{"cached_tokens":2}
        }),
    ] {
        assert!(
            decode_responses_turn(&[serde_json::json!({
                "type":"response.completed",
                "response":{"id":"resp_1","usage":usage}
            })])
            .is_err()
        );
    }
}

#[test]
fn productive_response_requires_exactly_one_successful_terminal_event() {
    assert!(decode_responses_turn(&[]).is_err());
    assert!(
        decode_responses_turn(&[
            serde_json::json!({"type":"response.completed","response":{"id":"one"}}),
            serde_json::json!({"type":"response.completed","response":{"id":"two"}}),
        ])
        .is_err()
    );
    assert!(
        decode_responses_turn(&[serde_json::json!({
            "type":"response.incomplete",
            "response":{"id":"one"}
        })])
        .is_err()
    );
}

#[test]
fn provider_arguments_become_a_complete_typed_tool_map() {
    let tool = test_tool();
    assert_eq!(
        provider_arguments_to_flow_value(&tool, "{\"--path\":\"src\",\"--limit\":3}")
            .expect("typed arguments"),
        core_script::FlowValue::Map(std::collections::BTreeMap::from([
            (
                "--limit".to_owned(),
                core_script::FlowValue::Integer("3".to_owned()),
            ),
            (
                "--path".to_owned(),
                core_script::FlowValue::String("src".to_owned()),
            ),
        ]))
    );
    assert!(
        provider_arguments_to_flow_value(&tool, "{\"--path\":\"src\",\"--path\":\"lib\"}").is_err()
    );
    assert!(
        provider_arguments_to_flow_value(&tool, "{\"--path\":\"src\",\"--limit\":1.5}").is_err()
    );
    assert_eq!(
        provider_arguments_to_flow_value(&tool, "{\"--path\":\"src\",\"--limit\":null}")
            .expect("strict nullable optional parameter is treated as absent"),
        core_script::FlowValue::Map(std::collections::BTreeMap::from([(
            "--path".to_owned(),
            core_script::FlowValue::String("src".to_owned()),
        )]))
    );
}

#[test]
fn provider_arguments_enforce_declared_tool_parameter_contracts() {
    let mut tool = test_tool();
    tool.allowed_parameters[0].max_length = Some(3);
    tool.allowed_parameters.extend([
        core_script::AllowedParameter {
            name: "--mode".to_owned(),
            value_type: core_script::ParameterValueType::Enum,
            required: false,
            allowed_values: vec!["safe".to_owned()],
            value_pattern: None,
            max_length: None,
            min: None,
            max: None,
        },
        core_script::AllowedParameter {
            name: "--label".to_owned(),
            value_type: core_script::ParameterValueType::String,
            required: false,
            allowed_values: Vec::new(),
            value_pattern: None,
            max_length: Some(3),
            min: None,
            max: None,
        },
    ]);

    for (label, arguments) in [
        ("integer minimum", r#"{"--path":"src","--limit":0}"#),
        ("integer maximum", r#"{"--path":"src","--limit":11}"#),
        ("path traversal", r#"{"--path":"../src"}"#),
        ("path pattern", r#"{"--path":"SRC"}"#),
        ("path length", r#"{"--path":"tool"}"#),
        ("enum", r#"{"--path":"src","--mode":"fast"}"#),
        ("string length", r#"{"--path":"src","--label":"long"}"#),
    ] {
        assert!(
            provider_arguments_to_flow_value(&tool, arguments).is_err(),
            "{label} violation must be rejected"
        );
    }
    assert!(
        provider_arguments_to_flow_value(
            &tool,
            r#"{"--path":"src","--limit":10,"--mode":"safe","--label":"abc"}"#,
        )
        .is_ok()
    );
}

#[test]
fn output_contract_instruction_requires_the_tagged_flow_value() {
    let instruction = output_contract_instruction(&core_script::ValueContract::Boolean)
        .expect("output instruction");
    assert!(instruction.contains("flow-value-v0"));
    assert!(instruction.contains("boolean"));
}
