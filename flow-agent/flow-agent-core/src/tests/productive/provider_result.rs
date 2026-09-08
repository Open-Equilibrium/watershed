use super::super::{helpers::load_test_registry, test_support::workspace_copy};
use super::support::{CountingObjectRecovery, ObjectRecovery};
use crate::runtime::{
    digest::sha256_hex,
    openai_codex::{ProviderTokenUsage, ProviderToolCall, ProviderTurn},
    productive::{
        MAX_DURABLE_PROVIDER_OUTPUT_BYTES, durable_provider_output, parse_provider_result,
        provider_turn_from_durable_output, verify_provider_result_session_objects,
    },
    types::MAX_SESSION_OBJECT_BYTES,
};
use std::{cell::Cell, collections::BTreeMap};
#[test]
fn provider_attempt_output_round_trips_and_rejects_ambiguous_recovery_data() {
    let turn = ProviderTurn {
        token_usage: Some(ProviderTokenUsage {
            input_tokens: Some(5),
            output_tokens: Some(6),
            cache_read_tokens: Some(4),
            cache_write_tokens: Some(3),
        }),
        response_id: "response".to_owned(),
        output_text: "{\"type\":\"string\",\"value\":\"done\"}".to_owned(),
        retained_items: vec![serde_json::json!({"id": "message", "type": "message"})],
        tool_calls: vec![ProviderToolCall {
            call_id: "call".to_owned(),
            name: "echo".to_owned(),
            arguments: "{}".to_owned(),
        }],
    };
    let durable = durable_provider_output(&turn).expect("provider output becomes durable");
    assert_eq!(durable.reference["schema"], "flow-provider-output-v2");
    assert_eq!(durable.reference["token_usage"]["input_tokens"], 5);
    assert_eq!(durable.reference["token_usage"]["output_tokens"], 6);
    assert_eq!(durable.reference["token_usage"]["cache_read_tokens"], 4);
    assert_eq!(durable.reference["token_usage"]["cache_write_tokens"], 3);
    let recovery = ObjectRecovery::from_objects(&durable.objects);
    let recovered = provider_turn_from_durable_output(&durable.reference, &recovery)
        .expect("durable provider output recovers");
    assert_eq!(recovered.response_id, turn.response_id);
    assert_eq!(recovered.output_text, turn.output_text);
    assert_eq!(recovered.retained_items, turn.retained_items);
    assert_eq!(recovered.token_usage, turn.token_usage);
    assert_eq!(recovered.tool_calls[0].call_id, turn.tool_calls[0].call_id);
    assert_eq!(recovered.tool_calls[0].name, turn.tool_calls[0].name);
    assert_eq!(
        recovered.tool_calls[0].arguments,
        turn.tool_calls[0].arguments
    );

    let unsupported = serde_json::json!({
        "provider_output_objects": ["session-object:sha256:unused"],
        "schema": "flow-provider-output-v9"
    });
    assert!(provider_turn_from_durable_output(&unsupported, &recovery).is_err());

    for bytes in [
        b"not-json".to_vec(),
        b"{ }".to_vec(),
        br#"{"extra":true,"output_text":"","response_id":"x","retained_items":[],"tool_calls":[]}"#
            .to_vec(),
    ] {
        let digest = sha256_hex(&bytes);
        let reference = serde_json::json!({
            "provider_output_objects": [format!("session-object:sha256:{digest}")],
            "schema": "flow-provider-output-v2"
        });
        let recovery = ObjectRecovery(BTreeMap::from([(
            format!("session-object:sha256:{digest}"),
            bytes,
        )]));
        assert!(provider_turn_from_durable_output(&reference, &recovery).is_err());
    }
}

#[test]
fn recovered_provider_output_objects_must_match_their_digest_uris() {
    let turn = ProviderTurn {
        token_usage: None,
        response_id: "response".to_owned(),
        output_text: "original".to_owned(),
        retained_items: Vec::new(),
        tool_calls: Vec::new(),
    };
    let durable = durable_provider_output(&turn).expect("provider output becomes durable");
    let uri = durable.reference["provider_output_objects"][0]
        .as_str()
        .expect("provider output URI")
        .to_owned();
    let mut tampered: serde_json::Value =
        serde_json::from_slice(&durable.objects[0].bytes).expect("provider snapshot JSON");
    tampered["output_text"] = serde_json::json!("tampered");
    let tampered = proto::canonical_json(&tampered)
        .expect("tampered snapshot remains canonicalizable")
        .into_bytes();
    let recovery = ObjectRecovery(BTreeMap::from([(uri, tampered)]));

    let error = provider_turn_from_durable_output(&durable.reference, &recovery)
        .expect_err("provider output whose bytes do not match its URI must fail closed");

    assert!(error.to_string().contains("does not match its URI digest"));
}

#[test]
fn provider_output_capacity_fails_closed_before_live_or_recovered_use() {
    let oversized = ProviderTurn {
        token_usage: None,
        response_id: "response-oversized".to_owned(),
        output_text: "x".repeat(MAX_DURABLE_PROVIDER_OUTPUT_BYTES),
        retained_items: Vec::new(),
        tool_calls: Vec::new(),
    };
    let error = match durable_provider_output(&oversized) {
        Err(error) => error,
        Ok(_) => panic!("snapshot envelope must push maximum text beyond the durable byte limit"),
    };
    assert!(error.to_string().contains("durable recovery byte limit"));

    let unused_uri = format!("session-object:sha256:{}", "a".repeat(64));
    let max_objects = MAX_DURABLE_PROVIDER_OUTPUT_BYTES.div_ceil(MAX_SESSION_OBJECT_BYTES as usize);
    for (name, reference, expected) in [
        (
            "empty",
            serde_json::json!({
                "provider_output_objects": [],
                "schema": "flow-provider-output-v2"
            }),
            "unsupported schema",
        ),
        (
            "too-many",
            serde_json::json!({
                "provider_output_objects": vec![unused_uri; max_objects + 1],
                "schema": "flow-provider-output-v2"
            }),
            "too many objects",
        ),
    ] {
        let error = provider_turn_from_durable_output(&reference, &ObjectRecovery::default())
            .expect_err("invalid recovered capacity metadata must fail closed");
        assert!(error.to_string().contains(expected), "{name}: {error}");
    }
}

#[test]
fn provider_results_enforce_their_durable_value_contracts() {
    let workspace = workspace_copy("smoke-flow");
    let registry = load_test_registry(&workspace, "smoke-flow");
    let phase = registry.phase_block("smoke").expect("smoke Phase");
    assert_eq!(
        parse_provider_result(phase, "{\"type\":\"string\",\"value\":\"done\"}")
            .expect("typed provider result"),
        core_script::FlowValue::String("done".to_owned())
    );
    for output in [
        "not-json",
        "{\"type\":\"string\",\"type\":\"boolean\",\"value\":true}",
        "{\"extra\":true,\"type\":\"string\",\"value\":\"done\"}",
        "{\"value\":\"missing type\"}",
        "{\"type\":\"integer\",\"value\":\"01\"}",
    ] {
        assert!(parse_provider_result(phase, output).is_err());
    }
}

#[test]
fn provider_result_session_objects_must_resolve_to_matching_run_objects() {
    let bytes = b"provider result object".to_vec();
    let uri = format!(
        "session-object:sha256:{}",
        crate::runtime::digest::sha256_hex(&bytes)
    );
    let result = core_script::FlowValue::Map(BTreeMap::from([(
        "nested".to_owned(),
        core_script::FlowValue::List(vec![core_script::FlowValue::SessionObject(uri.clone())]),
    )]));

    assert!(verify_provider_result_session_objects(&result, &ObjectRecovery::default()).is_err());
    assert!(
        verify_provider_result_session_objects(
            &result,
            &ObjectRecovery(BTreeMap::from([(uri.clone(), bytes)])),
        )
        .is_ok()
    );
    assert!(
        verify_provider_result_session_objects(
            &result,
            &ObjectRecovery(BTreeMap::from([(uri, b"wrong object".to_vec())])),
        )
        .is_err()
    );
}

#[test]
fn provider_result_session_objects_are_verified_once_per_distinct_uri() {
    let bytes = b"repeated provider result object".to_vec();
    let uri = format!(
        "session-object:sha256:{}",
        crate::runtime::digest::sha256_hex(&bytes)
    );
    let result = core_script::FlowValue::Map(BTreeMap::from([(
        "nested".to_owned(),
        core_script::FlowValue::List(vec![
            core_script::FlowValue::SessionObject(uri.clone()),
            core_script::FlowValue::Map(BTreeMap::from([(
                "repeated".to_owned(),
                core_script::FlowValue::SessionObject(uri.clone()),
            )])),
        ]),
    )]));
    core_script::validate_flow_value(&result).expect("repeated object references remain valid");
    let recovery = CountingObjectRecovery {
        objects: ObjectRecovery(BTreeMap::from([(uri, bytes)])),
        reads: Cell::new(0),
    };

    verify_provider_result_session_objects(&result, &recovery)
        .expect("matching repeated object references verify");

    assert_eq!(recovery.reads.get(), 1);
}
