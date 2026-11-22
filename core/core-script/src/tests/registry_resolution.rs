use super::super::error::{RegistryError, SemanticValidationError};
use super::super::load::{
    parse_registry_block, validate_registry_from_root, validate_registry_from_root_dir,
};
use super::super::model::{
    AllowedParameter, BlockIdentity, FlowBlock, FlowValue, InstructionBlock, MAX_BLOCK_NAME_CHARS,
    MAX_FLOW_FANOUT, MAX_FLOW_NESTING_DEPTH, MAX_PHASE_FANOUT, MAX_PHASE_NESTING_DEPTH,
    ParameterValueType, PhaseBlock, PhaseLoop, PhaseTransition, RegistryBlock, ResolvedRegistry,
    SubflowStart, ToolBlock, ToolCommand, ToolKind, ValueContract, ValueFieldContract,
    ValuePathSegment, ValuePredicate,
};
use super::{
    duplicated_subflow_tail_blocks, flow_chain_blocks, own_script_tool, phase_chain_block,
    phase_chain_blocks, registry_location, simple_phase_block, starting_phase, temp_registry_dir,
    test_phase, true_predicate,
};

fn identity(id: &str, name: &str) -> BlockIdentity {
    BlockIdentity {
        id: id.to_owned(),
        name: name.to_owned(),
    }
}

fn instruction(id: &str, name: &str, prompt: &str) -> RegistryBlock {
    RegistryBlock::Instruction(InstructionBlock {
        identity: identity(id, name),
        prompt: prompt.to_owned(),
        parameters: Vec::new(),
    })
}

#[test]
fn phase_transitions_reject_non_forward_references() {
    let error = ResolvedRegistry::from_blocks([
        simple_phase_block("draft"),
        simple_phase_block("review"),
        RegistryBlock::Phase(PhaseBlock {
            identity: identity("delivery", "Delivery"),
            phase_refs: vec!["draft".to_owned(), "review".to_owned()],
            result_from: Some("review".to_owned()),
            transitions: vec![PhaseTransition {
                from_phase_ref: "review".to_owned(),
                to_phase_ref: "draft".to_owned(),
                when: true_predicate(),
            }],
            ..test_phase()
        }),
    ])
    .expect_err("backward transitions are rejected");

    assert!(matches!(error, RegistryError::InvalidTransition { .. }));
}

#[test]
fn registry_reference_validation_rejects_flow_cycles() {
    let err = ResolvedRegistry::from_blocks([
        RegistryBlock::Phase(starting_phase("start-b", vec!["b".to_owned()])),
        RegistryBlock::Phase(starting_phase("start-a", vec!["a".to_owned()])),
        RegistryBlock::Flow(FlowBlock {
            identity: identity("a", "A"),
            phase_refs: vec!["start-b".to_owned()],
            transitions: Vec::new(),
        }),
        RegistryBlock::Flow(FlowBlock {
            identity: identity("b", "B"),
            phase_refs: vec!["start-a".to_owned()],
            transitions: Vec::new(),
        }),
    ])
    .expect_err("cycle rejected");

    assert!(err.to_string().contains("flow cycle"));
    assert!(matches!(err, RegistryError::FlowCycle { .. }));
}

#[test]
fn registry_reference_validation_rejects_deep_flow_chains() {
    assert_eq!(MAX_FLOW_NESTING_DEPTH, 16);
    ResolvedRegistry::from_blocks(flow_chain_blocks(MAX_FLOW_NESTING_DEPTH))
        .expect("max flow nesting depth is accepted");

    let err = ResolvedRegistry::from_blocks(flow_chain_blocks(MAX_FLOW_NESTING_DEPTH + 1))
        .expect_err("flow nesting above the max is rejected");

    assert!(std::error::Error::source(&err).is_none());
    assert!(err.to_string().contains("flow nesting depth"));
    assert!(matches!(
        err,
        RegistryError::FlowDepthExceeded {
            flow_id,
            depth,
            max,
        } if flow_id == format!("flow-{MAX_FLOW_NESTING_DEPTH:03}")
            && depth == MAX_FLOW_NESTING_DEPTH + 1
            && max == MAX_FLOW_NESTING_DEPTH
    ));
}

#[test]
fn a_phase_declares_at_most_32_subflows() {
    let blocks = |count| {
        let mut blocks = flow_chain_blocks(2);
        blocks[0] = RegistryBlock::Phase(starting_phase(
            "chain-phase-000",
            vec!["flow-001".to_owned(); count],
        ));
        blocks
    };
    ResolvedRegistry::from_blocks(blocks(MAX_FLOW_FANOUT)).expect("32 subflows are accepted");

    let err = ResolvedRegistry::from_blocks(blocks(MAX_FLOW_FANOUT + 1))
        .expect_err("more than 32 subflows are rejected");

    assert!(err.to_string().contains("subflow fan-out"));
}

#[test]
fn a_phase_offers_each_llm_subflow_once_under_a_free_function_name() {
    let refused = |phase: PhaseBlock, flow: &str, tool: Option<ToolBlock>| {
        let mut blocks = flow_chain_blocks(2);
        blocks[0] = RegistryBlock::Phase(phase);
        if let RegistryBlock::Flow(child) = &mut blocks[3] {
            child.identity.id = flow.to_owned();
        }
        blocks.extend(tool.map(RegistryBlock::Tool));
        ResolvedRegistry::from_blocks(blocks)
            .expect_err("the provider would see an ambiguous or invalid function name")
            .to_string()
    };
    let llm = |phase: &mut PhaseBlock| {
        for subflow in &mut phase.subflows {
            subflow.start = SubflowStart::Llm;
        }
    };

    let mut twice = starting_phase("chain-phase-000", vec!["flow-001".to_owned(); 2]);
    llm(&mut twice);
    assert!(refused(twice, "flow-001", None).contains("more than once"));

    let mut shadowed = starting_phase("chain-phase-000", vec!["flow-001".to_owned()]);
    llm(&mut shadowed);
    shadowed.tool_refs = vec!["subflow-flow-001".to_owned()];
    let tool = own_script_tool("subflow-flow-001", "script:subflow-flow-001");
    assert!(refused(shadowed, "flow-001", Some(tool)).contains("Tool"));

    let long = "f".repeat(57);
    let mut too_long = starting_phase("chain-phase-000", vec![long.clone()]);
    llm(&mut too_long);
    assert!(refused(too_long, &long, None).contains("64"));
}

#[test]
fn a_composite_phase_starts_no_subflows() {
    let mut phase = starting_phase("chain-phase-000", vec!["flow-001".to_owned()]);
    phase.phase_refs = vec!["child".to_owned()];
    phase.result_from = Some("child".to_owned());
    let mut blocks = flow_chain_blocks(2);
    blocks[0] = RegistryBlock::Phase(phase);
    blocks.push(simple_phase_block("child"));

    let err = ResolvedRegistry::from_blocks(blocks)
        .expect_err("a composite Phase makes no provider turn to start subflows");

    assert!(err.to_string().contains("composite Phase must not declare"));
    assert!(err.to_string().contains("subflows"));
}

#[test]
fn a_subflow_started_per_item_names_a_field() {
    let mut phase = starting_phase("chain-phase-000", vec!["flow-001".to_owned()]);
    phase.subflows[0].start = SubflowStart::Each(String::new());
    let mut blocks = flow_chain_blocks(2);
    blocks[0] = RegistryBlock::Phase(phase);

    let err = ResolvedRegistry::from_blocks(blocks).expect_err("an empty field name is rejected");

    assert!(err.to_string().contains("field name"), "{err}");
}

#[test]
fn registry_reference_validation_counts_shared_subflow_tails_per_path() {
    let mut blocks = flow_chain_blocks(MAX_FLOW_NESTING_DEPTH);
    blocks.push(RegistryBlock::Phase(starting_phase(
        "zz-parent-phase",
        vec!["flow-000".to_owned()],
    )));
    blocks.push(RegistryBlock::Flow(FlowBlock {
        identity: identity("zz-parent", "Parent"),
        phase_refs: vec!["zz-parent-phase".to_owned()],
        transitions: Vec::new(),
    }));

    let err = ResolvedRegistry::from_blocks(blocks)
        .expect_err("shared subflow tail still counts against parent depth");

    assert!(matches!(
        err,
        RegistryError::FlowDepthExceeded {
            depth,
            max,
            ..
        } if depth == MAX_FLOW_NESTING_DEPTH + 1 && max == MAX_FLOW_NESTING_DEPTH
    ));
}

#[test]
fn registry_accepts_duplicate_subflow_tails_within_depth() {
    ResolvedRegistry::from_blocks(duplicated_subflow_tail_blocks(MAX_FLOW_NESTING_DEPTH))
        .expect("duplicated acyclic subflow tail validates");
}

#[test]
fn registry_rejects_ambiguous_same_kind_id_name_references() {
    let err = ResolvedRegistry::from_blocks([
        instruction("alpha", "Alpha", "first"),
        instruction("beta", "alpha", "second"),
    ])
    .expect_err("ambiguous same-kind id/name reference rejected");

    assert!(err.to_string().contains("ambiguous instruction reference"));
    assert!(matches!(
        err,
        RegistryError::AmbiguousReference {
            kind: "instruction",
            reference,
        } if reference == "alpha"
    ));
}

#[test]
fn registry_rejects_duplicate_ids_and_ids_that_shadow_names() {
    let duplicate_id = ResolvedRegistry::from_blocks([
        instruction("same", "First", "first"),
        instruction("same", "Second", "second"),
    ])
    .expect_err("duplicate block ids must fail");

    assert!(duplicate_id.to_string().contains("duplicate instruction"));
    assert!(matches!(
        duplicate_id,
        RegistryError::DuplicateId {
            kind: "instruction",
            id,
        } if id == "same"
    ));

    let id_shadowing_name = ResolvedRegistry::from_blocks([
        instruction("first", "alias", "first"),
        instruction("alias", "Second", "second"),
    ])
    .expect_err("ids must not shadow existing names");

    assert!(matches!(
        id_shadowing_name,
        RegistryError::AmbiguousReference {
            kind: "instruction",
            reference,
        } if reference == "alias"
    ));
}

#[test]
fn registry_rejects_normalized_duplicate_names() {
    let err = ResolvedRegistry::from_blocks([
        instruction("composed", "Café", "Inspect"),
        instruction("decomposed", "Cafe\u{301}", "Inspect"),
    ])
    .expect_err("canonically equivalent names are duplicates");

    assert!(matches!(
        &err,
        RegistryError::DuplicateName {
            kind: "instruction",
            name,
        } if name == "Café"
    ));
    assert_eq!(err.to_string(), "duplicate instruction name: Café");
}

#[test]
fn registry_refuses_non_nfc_definition_text_instead_of_converting_it() {
    let decomposed = "Cafe\u{301}";
    let predefined = |argv: Vec<String>, allowed_values: Vec<String>| {
        RegistryBlock::Tool(ToolBlock {
            allowed_parameters: vec![AllowedParameter {
                name: "--mode".to_owned(),
                value_type: ParameterValueType::Enum,
                required: false,
                allowed_values,
                value_pattern: None,
                max_length: None,
                min: None,
                max: None,
            }],
            command: ToolCommand::Predefined {
                command_id: "agent-echo".to_owned(),
                argv,
            },
            identity: identity("echo", "Echo"),
            script_body: None,
            script_runtime: None,
            tool_kind: ToolKind::PredefinedCommand,
        })
    };
    let mut body = own_script_tool("decomposed-body", "script:decomposed-body");
    body.script_body = Some(format!("printf '{decomposed}\\n'"));

    for (field, block) in [
        ("script_body", RegistryBlock::Tool(body)),
        ("argv", predefined(vec![decomposed.to_owned()], Vec::new())),
        (
            "allowed_values",
            predefined(Vec::new(), vec![decomposed.to_owned()]),
        ),
        ("prompt", instruction("prompt", "Prompt", decomposed)),
    ] {
        let error = ResolvedRegistry::from_blocks([block])
            .expect_err("non-NFC definition text is refused, not converted");
        assert!(
            error
                .to_string()
                .ends_with(&format!("{field} must use NFC text")),
            "{field}: {error}"
        );
    }
}

#[test]
fn non_nfc_predicate_literal_under_ref_named_field_is_rejected() {
    for key in ["target_ref", "result_from"] {
        let mut phase = test_phase();
        phase.output = ValueContract::Map {
            fields: vec![ValueFieldContract {
                name: "choice".to_owned(),
                required: true,
                value_contract: ValueContract::Map {
                    fields: vec![ValueFieldContract {
                        name: key.to_owned(),
                        required: true,
                        value_contract: ValueContract::String { max_length: None },
                    }],
                },
            }],
        };
        phase.loop_config = Some(PhaseLoop {
            max_iterations: 2,
            until: ValuePredicate {
                path: vec![ValuePathSegment::Field {
                    field: "choice".to_owned(),
                }],
                equals: FlowValue::Map(std::collections::BTreeMap::from([(
                    key.to_owned(),
                    FlowValue::String("Cafe\u{301}".to_owned()),
                )])),
            },
        });

        let error = ResolvedRegistry::from_blocks([RegistryBlock::Phase(phase)])
            .expect_err("non-NFC predicate text is refused, not converted");
        assert!(
            matches!(error, RegistryError::NonNfcDefinitionText { .. }),
            "{key}: {error}"
        );
    }
}

#[test]
fn resolved_registry_canonicalizes_names_and_parameter_order() {
    let parameter = |name: &str| AllowedParameter {
        name: name.to_owned(),
        value_type: ParameterValueType::None,
        required: false,
        allowed_values: Vec::new(),
        value_pattern: None,
        max_length: None,
        min: None,
        max: None,
    };
    let mut tool = own_script_tool("canonical-tool", "script:canonical-tool");
    tool.identity.name = "Cafe\u{301}Tool".to_owned();
    tool.allowed_parameters = vec![parameter("--z-last"), parameter("--a-first")];

    let registry = ResolvedRegistry::from_blocks([RegistryBlock::Tool(tool)])
        .expect("canonical tool resolves");
    let tool = registry
        .tool_block("canonical-tool")
        .expect("canonical tool remains available");

    assert_eq!(tool.identity.name, "CaféTool");
    assert_eq!(
        tool.allowed_parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["--a-first", "--z-last"]
    );
}

#[test]
fn canonicalized_block_names_obey_the_character_limit() {
    let at_limit = ResolvedRegistry::from_blocks([instruction(
        "at-limit",
        &"\u{0344}".repeat(MAX_BLOCK_NAME_CHARS / 2),
        "Inspect",
    )])
    .expect("canonical name at the character limit is accepted");
    assert_eq!(
        at_limit
            .instruction_block("at-limit")
            .expect("instruction remains available")
            .identity
            .name
            .chars()
            .count(),
        MAX_BLOCK_NAME_CHARS
    );

    let error = ResolvedRegistry::from_blocks([instruction(
        "over-limit",
        &"\u{0344}".repeat(MAX_BLOCK_NAME_CHARS),
        "Inspect",
    )])
    .expect_err("canonical name above the character limit is rejected");
    assert!(
        error
            .to_string()
            .contains(&format!("at most {MAX_BLOCK_NAME_CHARS} characters")),
        "{error}"
    );
}

#[test]
fn registry_rejects_programmatic_invalid_shapes() {
    let err = ResolvedRegistry::from_blocks([instruction("../bad", "Bad", "Inspect")])
        .expect_err("programmatic blocks must follow registry shape rules");
    assert!(matches!(err, RegistryError::InvalidBlockId(id) if id == "../bad"));

    let err = ResolvedRegistry::from_blocks([instruction("empty-name", "", "Inspect")])
        .expect_err("programmatic blocks must follow registry shape rules");
    assert!(matches!(
        err,
        RegistryError::InvalidBlockName { kind: "instruction", id } if id == "empty-name"
    ));
}

#[test]
fn public_registry_validation_checks_path_and_capability_entry_points() {
    let root = temp_registry_dir("public-validation");
    std::fs::write(
        root.join("instruction.yaml"),
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
    )
    .expect("registry definition written");
    let (workspace, registry_root) = registry_location(&root);

    validate_registry_from_root(workspace, registry_root)
        .expect("path-based registry validation succeeds");
    let workspace_dir = cap_std::fs::Dir::open_ambient_dir(workspace, cap_std::ambient_authority())
        .expect("workspace capability opens");
    validate_registry_from_root_dir(&workspace_dir, workspace, registry_root)
        .expect("capability-based registry validation succeeds");
}

#[test]
fn registry_resolves_normalized_name_references() {
    let registry = ResolvedRegistry::from_blocks([
        instruction("inspect", "Café", "Inspect"),
        RegistryBlock::Phase(PhaseBlock {
            identity: identity("phase", "Phase"),
            instruction_refs: vec!["Cafe\u{301}".to_owned()],
            ..test_phase()
        }),
    ])
    .expect("canonically equivalent name reference resolves");

    assert_eq!(
        registry
            .instruction_block("Cafe\u{301}")
            .expect("decomposed reference resolves")
            .identity
            .id,
        "inspect"
    );
}

#[test]
fn registry_rejects_duplicate_phase_child_and_tool_refs() {
    let err = ResolvedRegistry::from_blocks([
        simple_phase_block("attempt"),
        RegistryBlock::Phase(PhaseBlock {
            identity: identity("phase", "Phase"),
            phase_refs: vec!["attempt".to_owned(), "attempt".to_owned()],
            result_from: Some("attempt".to_owned()),
            ..test_phase()
        }),
    ])
    .expect_err("duplicate direct child Phase references must fail");

    assert!(matches!(
        err,
        RegistryError::DuplicateId {
            kind: "child phase reference",
            id,
        } if id == "phase.attempt"
    ));

    let err = ResolvedRegistry::from_blocks([
        RegistryBlock::Tool(own_script_tool("echo", "script:echo")),
        RegistryBlock::Phase(PhaseBlock {
            identity: identity("phase", "Phase"),
            instruction_refs: Vec::new(),
            tool_refs: vec!["echo".to_owned(), "echo".to_owned()],
            ..test_phase()
        }),
    ])
    .expect_err("duplicate phase tool references must fail");

    assert!(matches!(
        err,
        RegistryError::DuplicateId {
            kind: "phase tool reference",
            id,
        } if id == "phase.echo"
    ));
}

#[test]
fn registry_rejects_invalid_composite_phase_references() {
    let inspect = instruction("inspect", "Inspect", "Inspect");
    let duplicate_instruction = RegistryBlock::Phase(PhaseBlock {
        identity: identity("duplicate-instruction", "Duplicate instruction"),
        instruction_refs: vec!["inspect".to_owned(), "inspect".to_owned()],
        ..test_phase()
    });
    let error = ResolvedRegistry::from_blocks([inspect, duplicate_instruction])
        .expect_err("duplicate Phase instruction references must fail");
    assert!(matches!(
        error,
        RegistryError::DuplicateId {
            kind: "phase instruction reference",
            id,
        } if id == "duplicate-instruction.inspect"
    ));

    let composite = |id: &str, phase_refs: Vec<&str>, result_from: &str| {
        RegistryBlock::Phase(PhaseBlock {
            identity: identity(id, &format!("Phase {id}")),
            phase_refs: phase_refs.into_iter().map(str::to_owned).collect(),
            result_from: Some(result_from.to_owned()),
            ..test_phase()
        })
    };
    let error = ResolvedRegistry::from_blocks([
        simple_phase_block("child"),
        simple_phase_block("outside"),
        composite("parent", vec!["child"], "outside"),
    ])
    .expect_err("a composite result must come from one of its direct children");
    assert!(matches!(
        error,
        RegistryError::MissingReference {
            reference_kind: "direct child Phase result",
            reference,
            ..
        } if reference == "outside"
    ));
}

#[test]
fn registry_rejects_composite_transitions_outside_direct_children() {
    let transition = |from: &str, to: &str| PhaseTransition {
        from_phase_ref: from.to_owned(),
        to_phase_ref: to.to_owned(),
        when: ValuePredicate {
            path: Vec::new(),
            equals: FlowValue::String("continue".to_owned()),
        },
    };
    let composite = |phase_refs: Vec<&str>, result_from: &str, transition| {
        RegistryBlock::Phase(PhaseBlock {
            identity: identity("parent", "Parent"),
            phase_refs: phase_refs.into_iter().map(str::to_owned).collect(),
            result_from: Some(result_from.to_owned()),
            transitions: vec![transition],
            ..test_phase()
        })
    };

    for (phase_refs, result_from, transition, expected_kind) in [
        (
            vec!["target"],
            "target",
            transition("outside", "target"),
            "Transition source child Phase",
        ),
        (
            vec!["source"],
            "source",
            transition("source", "outside"),
            "Transition target child Phase",
        ),
    ] {
        let error = ResolvedRegistry::from_blocks([
            simple_phase_block("source"),
            simple_phase_block("target"),
            simple_phase_block("outside"),
            composite(phase_refs, result_from, transition),
        ])
        .expect_err("Transition endpoints must be direct child Phases");
        assert!(matches!(
            error,
            RegistryError::MissingReference { reference_kind, .. }
                if reference_kind == expected_kind
        ));
    }

    let error = ResolvedRegistry::from_blocks([
        simple_phase_block("source"),
        simple_phase_block("target"),
        RegistryBlock::Phase(PhaseBlock {
            identity: identity("parent", "Parent"),
            phase_refs: vec!["source".to_owned(), "target".to_owned()],
            result_from: Some("target".to_owned()),
            transitions: vec![PhaseTransition {
                from_phase_ref: "source".to_owned(),
                to_phase_ref: "target".to_owned(),
                when: true_predicate(),
            }],
            ..test_phase()
        }),
    ])
    .expect_err("a composite Transition predicate must match its source output");
    assert!(matches!(
        error,
        RegistryError::Semantic(SemanticValidationError::InvalidPhaseDefinition {
            phase_id,
            ..
        }) if phase_id == "parent"
    ));
}

#[test]
fn registry_bounds_recursive_phase_graphs() {
    assert_eq!(MAX_PHASE_NESTING_DEPTH, 16);
    ResolvedRegistry::from_blocks(phase_chain_blocks(MAX_PHASE_NESTING_DEPTH))
        .expect("the maximum Phase nesting depth is accepted");

    let error = ResolvedRegistry::from_blocks(phase_chain_blocks(MAX_PHASE_NESTING_DEPTH + 1))
        .expect_err("Phase nesting beyond the maximum must fail");
    assert!(matches!(
        error,
        RegistryError::PhaseDepthExceeded { depth, max, .. }
            if depth == MAX_PHASE_NESTING_DEPTH + 1 && max == MAX_PHASE_NESTING_DEPTH
    ));

    let mut first = phase_chain_block(0, 2);
    first.phase_refs = vec!["phase-001".to_owned()];
    first.result_from = Some("phase-001".to_owned());
    let mut second = phase_chain_block(1, 2);
    second.phase_refs = vec!["phase-000".to_owned()];
    second.result_from = Some("phase-000".to_owned());
    let error =
        ResolvedRegistry::from_blocks([RegistryBlock::Phase(first), RegistryBlock::Phase(second)])
            .expect_err("recursive Phase cycles must fail");
    assert!(matches!(error, RegistryError::PhaseCycle { .. }));

    let mut blocks = (0..=MAX_PHASE_FANOUT)
        .map(|index| simple_phase_block(&format!("child-{index:03}")))
        .collect::<Vec<_>>();
    blocks.push(RegistryBlock::Phase(PhaseBlock {
        identity: identity("parent", "Parent"),
        phase_refs: (0..=MAX_PHASE_FANOUT)
            .map(|index| format!("child-{index:03}"))
            .collect(),
        result_from: Some("child-000".to_owned()),
        ..test_phase()
    }));
    let error = ResolvedRegistry::from_blocks(blocks)
        .expect_err("Phase child fan-out beyond the maximum must fail");
    assert!(matches!(
        error,
        RegistryError::PhaseFanoutExceeded { count, max, .. }
            if count == MAX_PHASE_FANOUT + 1 && max == MAX_PHASE_FANOUT
    ));

    let mut shared_tail = (0..8)
        .map(|index| {
            let child = (index < 7).then(|| format!("a-tail-{:02}", index + 1));
            RegistryBlock::Phase(PhaseBlock {
                identity: identity(
                    &format!("a-tail-{index:02}"),
                    &format!("Shared tail {index:02}"),
                ),
                phase_refs: child.iter().cloned().collect(),
                result_from: child,
                ..test_phase()
            })
        })
        .collect::<Vec<_>>();
    shared_tail.extend((0..9).map(|index| {
        let child = if index < 8 {
            format!("b-prefix-{:02}", index + 1)
        } else {
            "a-tail-00".to_owned()
        };
        RegistryBlock::Phase(PhaseBlock {
            identity: identity(
                &format!("b-prefix-{index:02}"),
                &format!("Prefix {index:02}"),
            ),
            phase_refs: vec![child.clone()],
            result_from: Some(child),
            ..test_phase()
        })
    }));
    let error = ResolvedRegistry::from_blocks(shared_tail)
        .expect_err("a cached Phase tail must still count toward the root depth");
    assert!(matches!(
        error,
        RegistryError::PhaseDepthExceeded { depth: 17, max, .. }
            if max == MAX_PHASE_NESTING_DEPTH
    ));
}

#[test]
fn m11_recursive_phase_loop_and_forward_transition_parse() {
    let block = parse_registry_block(
        "phase.yaml",
        r#"phase:
  id: delivery
  name: Delivery
  instruction_refs: []
  tool_refs: []
  phase_refs: [draft, review, publish]
  output:
    type: map
    fields:
      - name: approved
        required: true
        value_contract:
          type: boolean
  result_from: review
  loop:
    max_iterations: 3
    until:
      path:
        - field: approved
      equals:
        type: boolean
        value: true
  transitions:
    - from_phase_ref: draft
      to_phase_ref: publish
      when:
        path:
          - field: approved
        equals:
          type: boolean
          value: true
"#,
    )
    .expect("M1.1 recursive Phase parses");

    let RegistryBlock::Phase(phase) = block else {
        panic!("expected Phase block");
    };
    let value = serde_json::to_value(phase).expect("Phase serializes");
    assert_eq!(
        value["phase_refs"],
        serde_json::json!(["draft", "review", "publish"])
    );
    assert_eq!(value["loop"]["max_iterations"], 3);
    assert_eq!(value["transitions"][0]["to_phase_ref"], "publish");
}
