use super::super::load::parse_registry_block;
use super::super::model::{
    BlockIdentity, FlowBlock, FlowValue, MAX_FLOW_VALUE_BYTES, MAX_FLOW_VALUE_DEPTH,
    MAX_FLOW_VALUE_KEY_CHARS, MAX_FLOW_VALUE_MEMBERS, MAX_FLOW_VALUE_NODES, PhaseBlock,
    PhaseTransition, RegistryBlock, ResolvedRegistry, ValueContract, ValueFieldContract,
    ValuePathSegment, ValuePredicate,
};
use super::super::values::{
    predicate_matches, validate_flow_value, validate_flow_value_against_contract,
    validate_predicate_against_contract, validate_value_contract_definition,
};
use super::{test_flow, test_phase};

#[test]
fn m11_runtime_values_are_bounded_typed_and_contract_checked() {
    let contract = ValueContract::Map {
        fields: vec![
            ValueFieldContract {
                name: "approved".to_owned(),
                required: true,
                value_contract: ValueContract::Boolean,
            },
            ValueFieldContract {
                name: "score".to_owned(),
                required: false,
                value_contract: ValueContract::Integer {
                    min: Some(0),
                    max: Some(10),
                },
            },
        ],
    };
    let value = FlowValue::Map(std::collections::BTreeMap::from([
        ("approved".to_owned(), FlowValue::Boolean(true)),
        ("score".to_owned(), FlowValue::Integer("7".to_owned())),
    ]));

    validate_flow_value(&value).expect("bounded value is valid");
    validate_flow_value_against_contract(&value, &contract).expect("contract matches");
    assert!(
        predicate_matches(
            &value,
            &ValuePredicate {
                path: vec![ValuePathSegment::Field {
                    field: "approved".to_owned(),
                }],
                equals: FlowValue::Boolean(true),
            }
        )
        .expect("predicate evaluates")
    );

    let unknown_field = FlowValue::Map(std::collections::BTreeMap::from([
        ("approved".to_owned(), FlowValue::Boolean(true)),
        ("unexpected".to_owned(), FlowValue::String("no".to_owned())),
    ]));
    assert!(
        validate_flow_value_against_contract(&unknown_field, &contract)
            .expect_err("closed map rejects unknown fields")
            .to_string()
            .contains("unexpected")
    );
    assert!(
        validate_flow_value(&FlowValue::Integer("01".to_owned()))
            .expect_err("non-canonical integer is rejected")
            .to_string()
            .contains("canonical")
    );
}

#[test]
fn m11_runtime_value_contract_covers_every_recursive_shape_and_bound() {
    let object_uri = format!("session-object:sha256:{}", "a".repeat(64));
    for value in [
        FlowValue::Boolean(false),
        FlowValue::Integer(i64::MIN.to_string()),
        FlowValue::String("café".to_owned()),
        FlowValue::SessionObject(object_uri.clone()),
        FlowValue::List(vec![FlowValue::Boolean(true)]),
        FlowValue::Map(std::collections::BTreeMap::from([(
            "nested".to_owned(),
            FlowValue::List(vec![FlowValue::String("value".to_owned())]),
        )])),
    ] {
        validate_flow_value(&value).expect("every runtime value shape is valid");
    }

    for (value, expected) in [
        (
            FlowValue::SessionObject("object:sha256:bad".to_owned()),
            "canonical URI",
        ),
        (
            FlowValue::SessionObject(format!("session-object:sha256:{}", "A".repeat(64))),
            "lowercase SHA-256",
        ),
        (
            FlowValue::SessionObject(format!("session-object:sha256:{}", "a".repeat(63))),
            "lowercase SHA-256",
        ),
        (FlowValue::Integer("+1".to_owned()), "canonical decimal"),
        (
            FlowValue::Integer("9223372036854775808".to_owned()),
            "signed 64-bit",
        ),
        (FlowValue::String("e\u{301}".to_owned()), "NFC"),
        (
            FlowValue::Map(std::collections::BTreeMap::from([(
                String::new(),
                FlowValue::Boolean(true),
            )])),
            "must contain 1 to",
        ),
        (
            FlowValue::Map(std::collections::BTreeMap::from([(
                "e\u{301}".to_owned(),
                FlowValue::Boolean(true),
            )])),
            "NFC",
        ),
    ] {
        assert!(
            validate_flow_value(&value)
                .expect_err("invalid runtime value is rejected")
                .to_string()
                .contains(expected),
            "expected {expected} for {value:?}"
        );
    }

    let exact_depth = (1..MAX_FLOW_VALUE_DEPTH).fold(FlowValue::Boolean(true), |value, _| {
        FlowValue::List(vec![value])
    });
    validate_flow_value(&exact_depth).expect("exact depth limit is valid");
    assert!(
        validate_flow_value(&FlowValue::List(vec![exact_depth]))
            .expect_err("one-past depth limit is rejected")
            .to_string()
            .contains("depth")
    );

    validate_flow_value(&FlowValue::List(vec![
        FlowValue::Boolean(true);
        MAX_FLOW_VALUE_MEMBERS
    ]))
    .expect("exact list member limit is valid");
    assert!(
        validate_flow_value(&FlowValue::List(vec![
            FlowValue::Boolean(true);
            MAX_FLOW_VALUE_MEMBERS + 1
        ]))
        .expect_err("one-past list member limit is rejected")
        .to_string()
        .contains("member count")
    );
    let mut map_members = (0..MAX_FLOW_VALUE_MEMBERS)
        .map(|index| (format!("field-{index}"), FlowValue::Boolean(true)))
        .collect::<std::collections::BTreeMap<_, _>>();
    validate_flow_value(&FlowValue::Map(map_members.clone()))
        .expect("exact map member limit is valid");
    map_members.insert("one-past".to_owned(), FlowValue::Boolean(true));
    assert!(
        validate_flow_value(&FlowValue::Map(map_members))
            .expect_err("one-past map member limit is rejected")
            .to_string()
            .contains("member count")
    );

    let remaining_nodes = MAX_FLOW_VALUE_NODES - 1;
    let container_count = remaining_nodes.div_ceil(MAX_FLOW_VALUE_MEMBERS + 1);
    assert!(container_count <= MAX_FLOW_VALUE_MEMBERS);
    let mut leaf_count = remaining_nodes - container_count;
    let mut exact_nodes = (0..container_count)
        .map(|_| {
            let current_leaf_count = leaf_count.min(MAX_FLOW_VALUE_MEMBERS);
            leaf_count -= current_leaf_count;
            FlowValue::List(vec![FlowValue::Boolean(true); current_leaf_count])
        })
        .collect::<Vec<_>>();
    assert_eq!(leaf_count, 0);
    let exact_node_error = validate_flow_value(&FlowValue::List(exact_nodes.clone()))
        .expect_err("the tighter canonical byte limit rejects this exact-node fixture");
    assert!(!exact_node_error.to_string().contains("node count"));
    assert!(exact_node_error.to_string().contains("canonical JSON"));
    let Some(FlowValue::List(final_nodes)) = exact_nodes.iter_mut().find(
        |value| matches!(value, FlowValue::List(nodes) if nodes.len() < MAX_FLOW_VALUE_MEMBERS),
    ) else {
        panic!("node fixture has capacity for one more leaf");
    };
    final_nodes.push(FlowValue::Boolean(true));
    assert!(
        validate_flow_value(&FlowValue::List(exact_nodes))
            .expect_err("one-past node limit is rejected")
            .to_string()
            .contains("node count")
    );

    let empty_string_bytes = serde_json::to_vec(&FlowValue::String(String::new()))
        .expect("empty string fixture serializes")
        .len();
    validate_flow_value(&FlowValue::String(
        "x".repeat(MAX_FLOW_VALUE_BYTES - empty_string_bytes),
    ))
    .expect("exact canonical byte limit is valid");
    assert!(
        validate_flow_value(&FlowValue::String(
            "x".repeat(MAX_FLOW_VALUE_BYTES - empty_string_bytes + 1)
        ))
        .expect_err("one-past canonical byte limit is rejected")
        .to_string()
        .contains("canonical JSON")
    );

    validate_flow_value(&FlowValue::Integer(i64::MAX.to_string()))
        .expect("exact signed integer limit is valid");
    validate_flow_value(&FlowValue::Map(std::collections::BTreeMap::from([(
        "x".repeat(MAX_FLOW_VALUE_KEY_CHARS),
        FlowValue::Boolean(true),
    )])))
    .expect("exact key character limit is valid");
    assert!(
        validate_flow_value(&FlowValue::Map(std::collections::BTreeMap::from([(
            "x".repeat(MAX_FLOW_VALUE_KEY_CHARS + 1),
            FlowValue::Boolean(true),
        )])))
        .expect_err("one-past key character limit is rejected")
        .to_string()
        .contains("must contain 1 to")
    );

    let contract = ValueContract::Map {
        fields: vec![
            ValueFieldContract {
                name: "required".to_owned(),
                required: true,
                value_contract: ValueContract::Integer {
                    min: Some(-2),
                    max: Some(2),
                },
            },
            ValueFieldContract {
                name: "optional".to_owned(),
                required: false,
                value_contract: ValueContract::List {
                    items: Box::new(ValueContract::String {
                        max_length: Some(3),
                    }),
                    max_items: Some(2),
                },
            },
            ValueFieldContract {
                name: "object".to_owned(),
                required: true,
                value_contract: ValueContract::SessionObject,
            },
        ],
    };
    validate_flow_value_against_contract(
        &FlowValue::Map(std::collections::BTreeMap::from([
            ("required".to_owned(), FlowValue::Integer("2".to_owned())),
            (
                "optional".to_owned(),
                FlowValue::List(vec![FlowValue::String("yes".to_owned())]),
            ),
            ("object".to_owned(), FlowValue::SessionObject(object_uri)),
        ])),
        &contract,
    )
    .expect("recursive closed contract accepts matching value");

    for (value, expected) in [
        (
            FlowValue::Map(std::collections::BTreeMap::from([(
                "object".to_owned(),
                FlowValue::SessionObject(format!("session-object:sha256:{}", "b".repeat(64))),
            )])),
            "missing required field",
        ),
        (
            FlowValue::Map(std::collections::BTreeMap::from([
                ("required".to_owned(), FlowValue::Integer("3".to_owned())),
                (
                    "object".to_owned(),
                    FlowValue::SessionObject(format!("session-object:sha256:{}", "b".repeat(64))),
                ),
            ])),
            "outside",
        ),
        (
            FlowValue::Map(std::collections::BTreeMap::from([
                ("required".to_owned(), FlowValue::Integer("1".to_owned())),
                (
                    "optional".to_owned(),
                    FlowValue::List(vec![
                        FlowValue::String("one".to_owned()),
                        FlowValue::String("two".to_owned()),
                        FlowValue::String("tri".to_owned()),
                    ]),
                ),
                (
                    "object".to_owned(),
                    FlowValue::SessionObject(format!("session-object:sha256:{}", "b".repeat(64))),
                ),
            ])),
            "list length",
        ),
        (
            FlowValue::Map(std::collections::BTreeMap::from([
                ("required".to_owned(), FlowValue::Integer("1".to_owned())),
                (
                    "optional".to_owned(),
                    FlowValue::List(vec![FlowValue::String("long".to_owned())]),
                ),
                (
                    "object".to_owned(),
                    FlowValue::SessionObject(format!("session-object:sha256:{}", "b".repeat(64))),
                ),
            ])),
            "string length",
        ),
        (FlowValue::Boolean(true), "value type"),
    ] {
        assert!(
            validate_flow_value_against_contract(&value, &contract)
                .expect_err("contract mismatch is rejected")
                .to_string()
                .contains(expected),
            "expected {expected} for {value:?}"
        );
    }
}

#[test]
fn m11_runtime_value_rejects_adversarial_depth_before_serialization() {
    let mut value = FlowValue::Boolean(true);
    for _ in 0..16_384 {
        value = FlowValue::List(vec![value]);
    }

    let error = validate_flow_value(&value).expect_err("adversarial depth is rejected");
    assert!(error.to_string().contains("depth"));
    std::mem::forget(value);
}

#[test]
fn m11_value_contract_and_predicate_definitions_are_closed_and_finite() {
    for (contract, expected) in [
        (
            ValueContract::Integer {
                min: Some(2),
                max: Some(1),
            },
            "min must be <= max",
        ),
        (
            ValueContract::List {
                items: Box::new(ValueContract::Boolean),
                max_items: Some(
                    u16::try_from(MAX_FLOW_VALUE_MEMBERS + 1).expect("fixture fits u16"),
                ),
            },
            "max_items",
        ),
        (
            ValueContract::Map {
                fields: vec![
                    ValueFieldContract {
                        name: "duplicate".to_owned(),
                        required: true,
                        value_contract: ValueContract::Boolean,
                    };
                    2
                ],
            },
            "declared more than once",
        ),
        (
            ValueContract::Map {
                fields: (0..=MAX_FLOW_VALUE_MEMBERS)
                    .map(|index| ValueFieldContract {
                        name: format!("field-{index}"),
                        required: false,
                        value_contract: ValueContract::Boolean,
                    })
                    .collect(),
            },
            "field count",
        ),
        (
            ValueContract::Map {
                fields: vec![ValueFieldContract {
                    name: String::new(),
                    required: false,
                    value_contract: ValueContract::Boolean,
                }],
            },
            "must contain 1 to",
        ),
    ] {
        assert!(
            validate_value_contract_definition(&contract)
                .expect_err("invalid contract definition is rejected")
                .to_string()
                .contains(expected),
            "expected {expected} for {contract:?}"
        );
    }
    let mut too_deep = ValueContract::Boolean;
    for _ in 0..MAX_FLOW_VALUE_DEPTH {
        too_deep = ValueContract::List {
            items: Box::new(too_deep),
            max_items: None,
        };
    }
    assert!(
        validate_value_contract_definition(&too_deep)
            .expect_err("contract depth is bounded")
            .to_string()
            .contains("contract depth")
    );

    let contract = ValueContract::Map {
        fields: vec![ValueFieldContract {
            name: "items".to_owned(),
            required: true,
            value_contract: ValueContract::List {
                items: Box::new(ValueContract::Map {
                    fields: vec![ValueFieldContract {
                        name: "approved".to_owned(),
                        required: true,
                        value_contract: ValueContract::Boolean,
                    }],
                }),
                max_items: Some(2),
            },
        }],
    };
    let predicate = ValuePredicate {
        path: vec![
            ValuePathSegment::Field {
                field: "items".to_owned(),
            },
            ValuePathSegment::Index { index: 0 },
            ValuePathSegment::Field {
                field: "approved".to_owned(),
            },
        ],
        equals: FlowValue::Boolean(true),
    };
    validate_predicate_against_contract(&predicate, &contract)
        .expect("nested predicate matches its output contract");
    for index in [2, u16::MAX] {
        let impossible = ValuePredicate {
            path: vec![
                ValuePathSegment::Field {
                    field: "items".to_owned(),
                },
                ValuePathSegment::Index { index },
                ValuePathSegment::Field {
                    field: "approved".to_owned(),
                },
            ],
            equals: FlowValue::Boolean(true),
        };
        assert!(
            validate_predicate_against_contract(&impossible, &contract)
                .expect_err("predicate cannot select beyond the declared list bound")
                .to_string()
                .contains("cannot exist"),
            "index {index}"
        );
    }
    let value = FlowValue::Map(std::collections::BTreeMap::from([(
        "items".to_owned(),
        FlowValue::List(vec![FlowValue::Map(std::collections::BTreeMap::from([(
            "approved".to_owned(),
            FlowValue::Boolean(true),
        )]))]),
    )]));
    assert!(predicate_matches(&value, &predicate).expect("nested predicate evaluates"));

    for (value, predicate) in [
        (
            value.clone(),
            ValuePredicate {
                path: vec![ValuePathSegment::Field {
                    field: "missing".to_owned(),
                }],
                equals: FlowValue::Boolean(true),
            },
        ),
        (
            value.clone(),
            ValuePredicate {
                path: vec![
                    ValuePathSegment::Field {
                        field: "items".to_owned(),
                    },
                    ValuePathSegment::Index { index: 1 },
                ],
                equals: FlowValue::Boolean(true),
            },
        ),
        (
            FlowValue::Boolean(true),
            ValuePredicate {
                path: vec![ValuePathSegment::Field {
                    field: "items".to_owned(),
                }],
                equals: FlowValue::Boolean(true),
            },
        ),
    ] {
        assert!(!predicate_matches(&value, &predicate).expect("missing path does not match"));
    }
    for (predicate, expected) in [
        (
            ValuePredicate {
                path: vec![ValuePathSegment::Field {
                    field: "missing".to_owned(),
                }],
                equals: FlowValue::Boolean(true),
            },
            "undeclared field",
        ),
        (
            ValuePredicate {
                path: vec![ValuePathSegment::Index { index: 0 }],
                equals: FlowValue::Boolean(true),
            },
            "does not match",
        ),
        (
            ValuePredicate {
                path: vec![ValuePathSegment::Field {
                    field: "items".to_owned(),
                }],
                equals: FlowValue::Boolean(true),
            },
            "equality does not match",
        ),
    ] {
        assert!(
            validate_predicate_against_contract(&predicate, &contract)
                .expect_err("invalid predicate definition is rejected")
                .to_string()
                .contains(expected)
        );
    }
}

#[test]
fn m11_predicates_and_composite_results_match_declared_output_contracts() {
    let invalid_loop = parse_registry_block(
        "invalid-loop-path.yaml",
        r#"phase:
  id: retry-review
  name: Retry Review
  instruction_refs: []
  tool_refs: []
  output:
    type: boolean
  loop:
    max_iterations: 2
    until:
      path:
        - field: approved
      equals:
        type: boolean
        value: true
"#,
    )
    .expect_err("loop path must exist in the Phase output contract");
    assert!(invalid_loop.to_string().contains("loop.until"));

    let source = PhaseBlock {
        identity: BlockIdentity {
            id: "source".to_owned(),
            name: "Source".to_owned(),
        },
        output: ValueContract::Boolean,
        ..test_phase()
    };
    let child = PhaseBlock {
        identity: BlockIdentity {
            id: "child".to_owned(),
            name: "Child".to_owned(),
        },
        output: ValueContract::Boolean,
        ..test_phase()
    };
    let composite = PhaseBlock {
        identity: BlockIdentity {
            id: "composite".to_owned(),
            name: "Composite".to_owned(),
        },
        phase_refs: vec!["child".to_owned()],
        output: ValueContract::Boolean,
        result_from: Some("child".to_owned()),
        ..test_phase()
    };
    let flow = FlowBlock {
        phase_refs: vec!["source".to_owned(), "composite".to_owned()],
        transitions: vec![PhaseTransition {
            from_phase_ref: "source".to_owned(),
            to_phase_ref: "composite".to_owned(),
            when: ValuePredicate {
                path: Vec::new(),
                equals: FlowValue::String("wrong-type".to_owned()),
            },
        }],
        ..test_flow()
    };

    let error = ResolvedRegistry::from_blocks([
        RegistryBlock::Phase(source.clone()),
        RegistryBlock::Phase(child.clone()),
        RegistryBlock::Phase(composite.clone()),
        RegistryBlock::Flow(flow),
    ])
    .expect_err("Transition predicate must match its source output");
    assert!(error.to_string().contains("Transition predicate"));

    let mismatched_composite = PhaseBlock {
        output: ValueContract::String { max_length: None },
        ..composite
    };
    let error = ResolvedRegistry::from_blocks([
        RegistryBlock::Phase(source),
        RegistryBlock::Phase(child),
        RegistryBlock::Phase(mismatched_composite),
        RegistryBlock::Flow(test_flow()),
    ])
    .expect_err("composite output must match its selected child result");
    assert!(error.to_string().contains("output contract"));
}
