use crate::{
    FLOW_VALUE_MAX_BYTES_V0, canonical_json, flow_value::canonical_flow_value_fits_byte_limit,
    validate_flow_value_v0,
};
use serde_json::json;

#[test]
fn flow_values_accept_closed_nested_values() {
    let value = json!({
        "type": "map",
        "value": {
            "complete": { "type": "boolean", "value": true },
            "count": { "type": "integer", "value": "42" },
            "labels": {
                "type": "list",
                "value": [{ "type": "string", "value": "ready" }]
            }
        }
    });

    validate_flow_value_v0(&value).expect("closed nested flow value must validate");
}

#[test]
fn flow_values_reject_noncanonical_or_open_wrappers() {
    for value in [
        json!({ "type": "integer", "value": "01" }),
        json!({ "type": "string", "value": "e\u{301}" }),
        json!({ "type": "boolean", "value": true, "extra": false }),
        json!({ "type": "unknown", "value": null }),
    ] {
        assert!(
            validate_flow_value_v0(&value).is_err(),
            "invalid flow value must be rejected: {value}"
        );
    }
}

#[test]
fn flow_value_canonical_byte_counter_enforces_exact_boundary() {
    let empty = json!({ "type": "string", "value": "" });
    let overhead = canonical_json(&empty).unwrap().len();
    let exact = json!({
        "type": "string",
        "value": "x".repeat(FLOW_VALUE_MAX_BYTES_V0 - overhead)
    });
    let next = json!({
        "type": "string",
        "value": "x".repeat(FLOW_VALUE_MAX_BYTES_V0 - overhead + 1)
    });
    let escaping_expands_over_limit = json!({
        "type": "string",
        "value": "\"".repeat(FLOW_VALUE_MAX_BYTES_V0)
    });

    assert!(canonical_flow_value_fits_byte_limit(&exact));
    assert!(!canonical_flow_value_fits_byte_limit(&next));
    assert!(!canonical_flow_value_fits_byte_limit(
        &escaping_expands_over_limit
    ));
    validate_flow_value_v0(&exact).expect("exact canonical byte limit must validate");
    for oversized in [next, escaping_expands_over_limit] {
        assert!(
            validate_flow_value_v0(&oversized)
                .expect_err("over-limit canonical JSON must fail")
                .to_string()
                .contains(&FLOW_VALUE_MAX_BYTES_V0.to_string())
        );
    }
}
