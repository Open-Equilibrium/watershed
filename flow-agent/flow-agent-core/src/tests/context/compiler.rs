use super::support::{test_profile, tier_zero, total_source_bytes};
use crate::runtime::{
    context::{
        ContextModelProfile, ContextOmissionCounts, bounded_context_array_source, compile_context,
        context_source, context_source_bytes,
    },
    types::RuntimeError,
};

#[test]
fn context_profile_requires_a_usable_input_budget() {
    let profile = |context_limit| ContextModelProfile {
        context_limit,
        id: "profile-v0",
        output_reserve: 8,
        safety_margin: 3,
    };
    assert!(matches!(
        profile(10).input_budget_tokens(),
        Err(RuntimeError::Protocol(message)) if message.contains("reserves more tokens")
    ));
    assert!(matches!(
        profile(11).input_budget_tokens(),
        Err(RuntimeError::Protocol(message)) if message.contains("leaves no input budget")
    ));
    assert!(matches!(profile(12).input_budget_tokens(), Ok(1)));
}

#[test]
fn context_compiler_rejects_mandatory_content_over_budget() {
    let mandatory = tier_zero("large");
    let required = total_source_bytes(&mandatory);
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
