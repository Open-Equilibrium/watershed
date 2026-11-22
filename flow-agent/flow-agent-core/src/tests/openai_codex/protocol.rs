use crate::runtime::openai_codex::{
    ProviderToolCall, build_responses_request_body, decode_responses_turn, derive_prompt_cache_key,
    output_contract_instruction, provider_arguments_to_flow_value,
};
use std::collections::BTreeMap;

fn parameter(
    name: &str,
    value_type: core_script::ParameterValueType,
) -> core_script::AllowedParameter {
    core_script::AllowedParameter {
        name: name.to_owned(),
        value_type,
        required: false,
        allowed_values: Vec::new(),
        value_pattern: None,
        max_length: None,
        min: None,
        max: None,
    }
}

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
                required: true,
                value_pattern: Some("^[a-z]+(?:/[a-z]+)*$".to_owned()),
                max_length: Some(128),
                ..parameter(
                    "--path",
                    core_script::ParameterValueType::WorkspaceRelativePath,
                )
            },
            core_script::AllowedParameter {
                min: Some(1),
                max: Some(10),
                ..parameter("--limit", core_script::ParameterValueType::Integer)
            },
        ],
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
        &[],
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
    assert!(
        body["tools"][0]["parameters"]["properties"]["--path"]
            .get("pattern")
            .is_none()
    );
    assert!(body.get("previous_response_id").is_none());
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
        allowed_values: if name == "--mode" {
            vec!["safe".to_owned()]
        } else {
            Vec::new()
        },
        ..parameter(name, value_type)
    })
    .collect();
    let body = build_responses_request_body(
        "gpt-fixture",
        "cache-key",
        "Follow the Phase instructions.",
        &[],
        &[&tool],
        &[],
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
fn provider_reply_that_breaks_the_responses_format_is_reply_invalid() {
    let call = |call_id: &str, name: &str| {
        serde_json::json!({
            "type":"response.output_item.done",
            "item":{"type":"function_call","call_id":call_id,"name":name,"arguments":"{}"}
        })
    };
    let completed = serde_json::json!({"type":"response.completed","response":{"id":"resp_1"}});
    let cases = [
        (
            "empty call id",
            vec![call("", "inspect"), completed.clone()],
            "OpenAI Codex provider: function call id must not be empty",
        ),
        (
            "empty function name",
            vec![call("call_1", ""), completed.clone()],
            "OpenAI Codex provider: function name must not be empty",
        ),
        (
            "repeated call id",
            vec![
                call("call_1", "inspect"),
                call("call_1", "inspect"),
                completed.clone(),
            ],
            "response repeated function call id call_1",
        ),
        (
            "output item without content",
            vec![
                serde_json::json!({
                    "type":"response.output_item.done",
                    "item":{"type":"message","id":"msg_1","role":"assistant"}
                }),
                completed.clone(),
            ],
            "output item without content",
        ),
        (
            "data after the end",
            vec![
                completed.clone(),
                serde_json::json!({"type":"response.output_text.delta","delta":"late"}),
            ],
            "response contains data after its successful terminal event",
        ),
    ];
    for (case, events, detail) in cases {
        let error = decode_responses_turn(&events).expect_err(case);
        let failure = error
            .provider_failure()
            .unwrap_or_else(|| panic!("{case} is a provider reply failure: {error}"));
        assert!(failure.is_reply_invalid(), "{case}");
        assert_eq!(
            failure.message(),
            format!("Flow Agent rejected the provider reply: {detail}"),
            "{case}"
        );
    }
}

#[test]
fn productive_response_decodes_optional_bounded_cache_usage() {
    for (payload, expected) in [
        (
            serde_json::json!({
                "input_tokens":12,
                "input_tokens_details":{"cached_tokens":4,"cache_write_tokens":3},
                "output_tokens":6
            }),
            (Some(5), Some(6), Some(4), Some(3)),
        ),
        (
            serde_json::json!({
                "input_tokens":0,
                "input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0},
                "output_tokens":0
            }),
            (Some(0), Some(0), Some(0), Some(0)),
        ),
        (
            serde_json::json!({"input_tokens":u64::MAX,"output_tokens":u64::MAX}),
            (Some(u64::MAX), Some(u64::MAX), None, None),
        ),
    ] {
        let turn = decode_responses_turn(&[serde_json::json!({
            "type":"response.completed",
            "response":{"id":"resp_1","usage":payload}
        })])
        .expect("completed turn with usage");
        let usage = turn.token_usage.expect("usage is decoded");
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_write_tokens,
            ),
            expected
        );
    }
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
        core_script::FlowValue::Map(BTreeMap::from([
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
        core_script::FlowValue::Map(BTreeMap::from([(
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
            allowed_values: vec!["safe".to_owned()],
            ..parameter("--mode", core_script::ParameterValueType::Enum)
        },
        core_script::AllowedParameter {
            max_length: Some(3),
            ..parameter("--label", core_script::ParameterValueType::String)
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
    let instruction = output_contract_instruction(&core_script::ValueContract::Boolean {})
        .expect("output instruction");
    assert!(instruction.contains("flow-value-v0"));
    assert!(instruction.contains("boolean"));
}
