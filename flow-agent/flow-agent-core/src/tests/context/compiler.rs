use super::support::{test_profile, tier_zero};
use crate::runtime::{
    context::{
        ContextModelProfile, ContextOmissionCounts, bounded_context_array_source, compile_context,
        context_source, context_source_bytes,
    },
    types::RuntimeError,
};

#[test]
fn context_profile_requires_a_usable_input_budget() {
    let invalid_profile = ContextModelProfile {
        context_limit: 10,
        id: "invalid-profile",
        output_reserve: 8,
        safety_margin: 3,
    };
    assert!(matches!(
        invalid_profile.input_budget_tokens(),
        Err(RuntimeError::Protocol(message)) if message.contains("reserves more tokens")
    ));
    let empty_profile = ContextModelProfile {
        context_limit: 11,
        id: "empty-profile",
        output_reserve: 8,
        safety_margin: 3,
    };
    assert!(matches!(
        empty_profile.input_budget_tokens(),
        Err(RuntimeError::Protocol(message)) if message.contains("leaves no input budget")
    ));
    let smallest_valid_profile = ContextModelProfile {
        context_limit: 12,
        id: "smallest-valid-profile",
        output_reserve: 8,
        safety_margin: 3,
    };
    assert!(matches!(
        smallest_valid_profile.input_budget_tokens(),
        Ok(1)
    ));
}

#[test]
fn context_compiler_rejects_mandatory_content_over_budget() {
    let mandatory = tier_zero("large");
    let required = mandatory
        .iter()
        .map(|source| {
            context_source_bytes(source)
                .expect("mandatory source serializes")
                .len()
        })
        .sum::<usize>();
    let err = compile_context(
        &test_profile(required - 1),
        &mandatory,
        None,
        ContextOmissionCounts::default(),
    )
    .expect_err("mandatory context must not be truncated");

    let message = err.to_string();
    assert!(
        message.contains("canonical bytes (one estimated token per byte)"),
        "{message}"
    );
    assert!(matches!(
        err,
        RuntimeError::ContextBudgetExceeded {
            required_bytes,
            input_budget_tokens
        } if required_bytes == required && input_budget_tokens == required - 1
    ));
}

#[test]
fn context_array_source_stops_materializing_repeated_content_at_its_budget() {
    let item = "x".repeat(64 * 1024);
    let source_id = "active-phase-instructions";
    let one_item_source = context_source(
        source_id,
        serde_json::json!([{"id":"repeated","prompt":item.as_str()}]),
    );
    let input_budget_tokens = context_source_bytes(&one_item_source)
        .expect("one-item source serializes")
        .len();
    let mut materialized = 0;

    let result = bounded_context_array_source(
        source_id,
        (0..4_096).map(|_| {
            materialized += 1;
            Ok(Some(serde_json::json!({
                "id": "repeated",
                "prompt": item.as_str(),
            })))
        }),
        input_budget_tokens,
    );
    let err = match result {
        Err(err) => err,
        Ok(_) => panic!("the second repeated item must exceed the source budget"),
    };

    assert_eq!(materialized, 2);
    assert!(matches!(
        err,
        RuntimeError::ContextBudgetExceeded {
            input_budget_tokens: actual_budget,
            required_bytes,
        } if actual_budget == input_budget_tokens && required_bytes > input_budget_tokens
    ));
}

#[test]
fn empty_context_array_source_enforces_its_canonical_wrapper_budget() {
    let source_id = "empty-source";
    let expected = context_source(source_id, serde_json::json!([]));
    let required_bytes = context_source_bytes(&expected)
        .expect("empty source serializes")
        .len();
    let empty_items = || std::iter::empty::<Result<Option<serde_json::Value>, RuntimeError>>();

    let err = match bounded_context_array_source(source_id, empty_items(), required_bytes - 1) {
        Err(err) => err,
        Ok(_) => panic!("the canonical empty wrapper must fit its source budget"),
    };
    assert!(matches!(
        err,
        RuntimeError::ContextBudgetExceeded {
            input_budget_tokens,
            required_bytes: actual_required,
        } if input_budget_tokens == required_bytes - 1 && actual_required == required_bytes
    ));

    let actual = bounded_context_array_source(source_id, empty_items(), required_bytes)
        .expect("the exact empty-wrapper boundary must fit");
    assert_eq!(actual.source_id, expected.source_id);
    assert_eq!(actual.content, expected.content);
}
