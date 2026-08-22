use super::super::load::parse_registry_block;
use super::super::model::{
    BlockIdentity, FlowValue, InstructionBlock, InstructionParameter,
    MAX_REGISTRY_DEFINITION_BYTES, RegistryBlock, ValueContract,
};
use super::super::semantics::validate_registry_block_semantics;
use super::super::values::render_instruction;

#[test]
fn m11_instruction_parameters_and_closed_value_contracts_parse() {
    let block = parse_registry_block(
        "instruction.yaml",
        r#"instruction:
  id: review-project
  name: Review Project
  prompt: Review {{project}} and return the requested result.
  parameters:
    - name: project
      value_contract:
        type: string
        max_length: 256
"#,
    )
    .expect("M1.1 typed Instruction parses");

    let RegistryBlock::Instruction(instruction) = block else {
        panic!("expected Instruction block");
    };
    let value = serde_json::to_value(instruction).expect("Instruction serializes");
    assert_eq!(value["parameters"][0]["name"], "project");
    assert_eq!(value["parameters"][0]["value_contract"]["type"], "string");
}

#[test]
fn m11_instruction_rendering_requires_exact_typed_parameters() {
    let instruction = InstructionBlock {
        identity: BlockIdentity {
            id: "review".to_owned(),
            name: "Review".to_owned(),
        },
        prompt: "Review {{project}} with strict={{strict}}.".to_owned(),
        parameters: vec![
            InstructionParameter {
                name: "project".to_owned(),
                value_contract: ValueContract::String {
                    max_length: Some(32),
                },
            },
            InstructionParameter {
                name: "strict".to_owned(),
                value_contract: ValueContract::Boolean,
            },
        ],
    };
    let parameters = std::collections::BTreeMap::from([
        (
            "project".to_owned(),
            FlowValue::String("watershed".to_owned()),
        ),
        ("strict".to_owned(), FlowValue::Boolean(true)),
    ]);

    assert_eq!(
        render_instruction(&instruction, &parameters, MAX_REGISTRY_DEFINITION_BYTES)
            .expect("parameters render"),
        "Review {\"type\":\"string\",\"value\":\"watershed\"} with strict={\"type\":\"boolean\",\"value\":true}."
    );

    let placeholder_value = std::collections::BTreeMap::from([
        (
            "project".to_owned(),
            FlowValue::String("{{strict}}".to_owned()),
        ),
        ("strict".to_owned(), FlowValue::Boolean(true)),
    ]);
    assert_eq!(
        render_instruction(
            &instruction,
            &placeholder_value,
            MAX_REGISTRY_DEFINITION_BYTES,
        )
        .expect("inserted values are not interpolated again"),
        "Review {\"type\":\"string\",\"value\":\"{{strict}}\"} with strict={\"type\":\"boolean\",\"value\":true}."
    );

    let missing = std::collections::BTreeMap::from([(
        "project".to_owned(),
        FlowValue::String("watershed".to_owned()),
    )]);
    assert!(
        render_instruction(&instruction, &missing, MAX_REGISTRY_DEFINITION_BYTES)
            .expect_err("missing parameter is rejected")
            .to_string()
            .contains("strict")
    );

    let wrong_type = std::collections::BTreeMap::from([
        (
            "project".to_owned(),
            FlowValue::String("watershed".to_owned()),
        ),
        ("strict".to_owned(), FlowValue::String("yes".to_owned())),
    ]);
    assert!(
        render_instruction(&instruction, &wrong_type, MAX_REGISTRY_DEFINITION_BYTES)
            .expect_err("typed Instruction parameter is enforced")
            .to_string()
            .contains("strict")
    );

    let extra = std::collections::BTreeMap::from([
        (
            "project".to_owned(),
            FlowValue::String("watershed".to_owned()),
        ),
        ("strict".to_owned(), FlowValue::Boolean(true)),
        ("extra".to_owned(), FlowValue::Boolean(true)),
    ]);
    assert!(
        render_instruction(&instruction, &extra, MAX_REGISTRY_DEFINITION_BYTES)
            .expect_err("undeclared Instruction parameter is rejected")
            .to_string()
            .contains("undeclared")
    );
}

#[test]
fn m11_instruction_semantics_reject_every_placeholder_mismatch() {
    let valid = InstructionBlock {
        identity: BlockIdentity {
            id: "review".to_owned(),
            name: "Review".to_owned(),
        },
        prompt: "Review {{project}}.".to_owned(),
        parameters: vec![InstructionParameter {
            name: "project".to_owned(),
            value_contract: ValueContract::String {
                max_length: Some(32),
            },
        }],
    };
    let mut cases = Vec::new();

    let mut invalid_name = valid.clone();
    invalid_name.parameters[0].name = "Bad Name".to_owned();
    cases.push(invalid_name);

    let mut duplicate = valid.clone();
    duplicate.parameters.push(duplicate.parameters[0].clone());
    cases.push(duplicate);

    let mut invalid_contract = valid.clone();
    invalid_contract.parameters[0].value_contract = ValueContract::Integer {
        min: Some(2),
        max: Some(1),
    };
    cases.push(invalid_contract);

    let mut missing_placeholder = valid.clone();
    missing_placeholder.prompt = "Review the project.".to_owned();
    cases.push(missing_placeholder);

    for prompt in [
        "Review {{other}}.",
        "Review {{project.",
        "Review {{Bad Name}}.",
        "Review project}}.",
    ] {
        let mut malformed = valid.clone();
        malformed.prompt = prompt.to_owned();
        cases.push(malformed);
    }

    for instruction in cases {
        assert!(
            validate_registry_block_semantics(&RegistryBlock::Instruction(instruction)).is_err()
        );
    }
    for prompt in [
        "Review {{other}}.",
        "Review {{project.",
        "Review {{Bad Name}}.",
        "Review project}}.",
    ] {
        let instruction = InstructionBlock {
            prompt: prompt.to_owned(),
            parameters: Vec::new(),
            ..valid.clone()
        };
        assert!(
            validate_registry_block_semantics(&RegistryBlock::Instruction(instruction)).is_err()
        );
    }
    validate_registry_block_semantics(&RegistryBlock::Instruction(valid))
        .expect("matching typed placeholder is valid");
}

#[test]
fn m11_instruction_semantics_rejects_stray_terminators_before_placeholders() {
    let instruction = InstructionBlock {
        identity: BlockIdentity {
            id: "review".to_owned(),
            name: "Review".to_owned(),
        },
        prompt: String::new(),
        parameters: vec![InstructionParameter {
            name: "project".to_owned(),
            value_contract: ValueContract::String {
                max_length: Some(32),
            },
        }],
    };

    for prompt in [
        "}} Review {{project}}.",
        "Review {{project}} stray }} then {{project}}.",
    ] {
        let error =
            validate_registry_block_semantics(&RegistryBlock::Instruction(InstructionBlock {
                prompt: prompt.to_owned(),
                ..instruction.clone()
            }))
            .expect_err("a terminator before the next placeholder must be rejected");
        assert!(
            error
                .to_string()
                .contains("unmatched placeholder terminator"),
            "{error}"
        );
    }
}

#[test]
fn m11_instruction_rendering_enforces_its_cumulative_byte_limit() {
    let instruction = InstructionBlock {
        identity: BlockIdentity {
            id: "bounded".to_owned(),
            name: "Bounded".to_owned(),
        },
        prompt: "{{value}}".repeat(8),
        parameters: vec![InstructionParameter {
            name: "value".to_owned(),
            value_contract: ValueContract::String {
                max_length: Some(1_024),
            },
        }],
    };
    let value = FlowValue::String("x".repeat(1_024));
    let canonical = proto::canonical_json(&serde_json::to_value(&value).unwrap()).unwrap();
    let parameters = std::collections::BTreeMap::from([("value".to_owned(), value)]);
    let exact_bytes = canonical.len() * 8;

    assert_eq!(
        render_instruction(&instruction, &parameters, exact_bytes)
            .expect("exact rendered byte limit must succeed")
            .len(),
        exact_bytes
    );
    assert!(
        render_instruction(&instruction, &parameters, exact_bytes - 1)
            .expect_err("rendering beyond the byte limit must fail")
            .to_string()
            .contains(&exact_bytes.saturating_sub(1).to_string())
    );
}
