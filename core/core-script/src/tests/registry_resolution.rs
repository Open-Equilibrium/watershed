use super::super::error::{RegistryError, SemanticValidationError};
use super::super::load::{
    parse_registry_block, validate_registry_from_workspace, validate_registry_from_workspace_dir,
};
use super::super::model::{
    AllowedParameter, BlockIdentity, FlowBlock, FlowValue, InstructionBlock, MAX_BLOCK_NAME_CHARS,
    MAX_FLOW_FANOUT, MAX_FLOW_NESTING_DEPTH, MAX_PHASE_FANOUT, MAX_PHASE_NESTING_DEPTH,
    ParameterValueType, PhaseBlock, PhaseTransition, RegistryBlock, ResolvedRegistry,
    ValuePredicate,
};
use super::{
    duplicated_subflow_tail_blocks, flow_chain_blocks, own_script_tool, phase_chain_block,
    phase_chain_blocks, registry_location, simple_phase_block, temp_registry_dir, test_phase,
    true_predicate,
};

#[test]
fn phase_transitions_reject_non_forward_references() {
    let error = ResolvedRegistry::from_blocks([
        simple_phase_block("draft"),
        simple_phase_block("review"),
        RegistryBlock::Phase(PhaseBlock {
            identity: BlockIdentity {
                id: "delivery".to_owned(),
                name: "Delivery".to_owned(),
            },
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
        simple_phase_block("phase"),
        RegistryBlock::Flow(FlowBlock {
            identity: BlockIdentity {
                id: "a".to_owned(),
                name: "A".to_owned(),
            },
            phase_refs: vec!["phase".to_owned()],
            subflow_refs: vec!["b".to_owned()],
            transitions: Vec::new(),
        }),
        RegistryBlock::Flow(FlowBlock {
            identity: BlockIdentity {
                id: "b".to_owned(),
                name: "B".to_owned(),
            },
            phase_refs: vec!["phase".to_owned()],
            subflow_refs: vec!["a".to_owned()],
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
fn registry_reference_validation_rejects_more_than_32_direct_subflows() {
    let mut blocks = flow_chain_blocks(2);
    let RegistryBlock::Flow(root) = &mut blocks[1] else {
        panic!("second chain block must be a flow");
    };
    root.subflow_refs = vec!["flow-001".to_owned(); MAX_FLOW_FANOUT];
    ResolvedRegistry::from_blocks(blocks).expect("32 direct subflow references are accepted");

    let mut blocks = flow_chain_blocks(2);
    let RegistryBlock::Flow(root) = &mut blocks[1] else {
        panic!("second chain block must be a flow");
    };
    root.subflow_refs = vec!["flow-001".to_owned(); MAX_FLOW_FANOUT + 1];

    let err = ResolvedRegistry::from_blocks(blocks)
        .expect_err("more than 32 direct subflow invocations are rejected");

    assert!(err.to_string().contains("subflow fan-out"));
}

#[test]
fn registry_reference_validation_counts_shared_subflow_tails_per_path() {
    let mut blocks = flow_chain_blocks(MAX_FLOW_NESTING_DEPTH);
    blocks.push(RegistryBlock::Flow(FlowBlock {
        identity: BlockIdentity {
            id: "zz-parent".to_owned(),
            name: "Parent".to_owned(),
        },
        phase_refs: vec!["chain-phase".to_owned()],
        subflow_refs: vec!["flow-000".to_owned()],
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
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "alpha".to_owned(),
                name: "Alpha".to_owned(),
            },
            prompt: "first".to_owned(),
            parameters: Vec::new(),
        }),
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "beta".to_owned(),
                name: "alpha".to_owned(),
            },
            prompt: "second".to_owned(),
            parameters: Vec::new(),
        }),
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
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "same".to_owned(),
                name: "First".to_owned(),
            },
            prompt: "first".to_owned(),
            parameters: Vec::new(),
        }),
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "same".to_owned(),
                name: "Second".to_owned(),
            },
            prompt: "second".to_owned(),
            parameters: Vec::new(),
        }),
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
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "first".to_owned(),
                name: "alias".to_owned(),
            },
            prompt: "first".to_owned(),
            parameters: Vec::new(),
        }),
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "alias".to_owned(),
                name: "Second".to_owned(),
            },
            prompt: "second".to_owned(),
            parameters: Vec::new(),
        }),
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
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "composed".to_owned(),
                name: "Café".to_owned(),
            },
            prompt: "Inspect".to_owned(),
            parameters: Vec::new(),
        }),
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "decomposed".to_owned(),
                name: "Cafe\u{301}".to_owned(),
            },
            prompt: "Inspect".to_owned(),
            parameters: Vec::new(),
        }),
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
fn resolved_registry_canonicalizes_runtime_strings_and_parameter_order() {
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
    tool.script_body = Some("printf 'Cafe\u{301}\\n'".to_owned());
    tool.allowed_parameters = vec![parameter("--z-last"), parameter("--a-first")];

    let registry = ResolvedRegistry::from_blocks([RegistryBlock::Tool(tool)])
        .expect("canonical tool resolves");
    let tool = registry
        .tool_block("canonical-tool")
        .expect("canonical tool remains available");

    assert_eq!(tool.identity.name, "CaféTool");
    assert_eq!(tool.script_body.as_deref(), Some("printf 'Café\\n'"));
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
    let instruction = |id: &str, name: String| {
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: id.to_owned(),
                name,
            },
            parameters: Vec::new(),
            prompt: "Inspect".to_owned(),
        })
    };

    let at_limit = ResolvedRegistry::from_blocks([instruction(
        "at-limit",
        "\u{0344}".repeat(MAX_BLOCK_NAME_CHARS / 2),
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
        "\u{0344}".repeat(MAX_BLOCK_NAME_CHARS),
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
    let instruction = |id: &str, name: &str| {
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: id.to_owned(),
                name: name.to_owned(),
            },
            prompt: "Inspect".to_owned(),
            parameters: Vec::new(),
        })
    };
    let cases = [
        (
            instruction("../bad", "Bad"),
            (|err| matches!(err, RegistryError::InvalidBlockId(id) if id == "../bad"))
                as fn(RegistryError) -> bool,
        ),
        (
            instruction("empty-name", ""),
            |err| matches!(err, RegistryError::InvalidBlockName { kind: "instruction", id } if id == "empty-name"),
        ),
    ];

    for (block, matches_expected_error) in cases {
        let err = ResolvedRegistry::from_blocks([block])
            .expect_err("programmatic blocks must follow registry shape rules");
        assert!(matches_expected_error(err));
    }
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

    validate_registry_from_workspace(workspace, registry_root)
        .expect("path-based registry validation succeeds");
    let workspace_dir = cap_std::fs::Dir::open_ambient_dir(workspace, cap_std::ambient_authority())
        .expect("workspace capability opens");
    validate_registry_from_workspace_dir(&workspace_dir, workspace, registry_root)
        .expect("capability-based registry validation succeeds");
}

#[test]
fn registry_resolves_normalized_name_references() {
    let registry = ResolvedRegistry::from_blocks([
        RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "inspect".to_owned(),
                name: "Café".to_owned(),
            },
            prompt: "Inspect".to_owned(),
            parameters: Vec::new(),
        }),
        RegistryBlock::Phase(PhaseBlock {
            identity: BlockIdentity {
                id: "phase".to_owned(),
                name: "Phase".to_owned(),
            },
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
fn registry_rejects_duplicate_child_phase_refs() {
    let err = ResolvedRegistry::from_blocks([
        simple_phase_block("attempt"),
        RegistryBlock::Phase(PhaseBlock {
            identity: BlockIdentity {
                id: "phase".to_owned(),
                name: "Phase".to_owned(),
            },
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
}

#[test]
fn registry_rejects_duplicate_phase_tool_refs() {
    let err = ResolvedRegistry::from_blocks([
        RegistryBlock::Tool(own_script_tool("echo", "script:echo")),
        RegistryBlock::Phase(PhaseBlock {
            identity: BlockIdentity {
                id: "phase".to_owned(),
                name: "Phase".to_owned(),
            },
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
    let instruction = RegistryBlock::Instruction(InstructionBlock {
        identity: BlockIdentity {
            id: "inspect".to_owned(),
            name: "Inspect".to_owned(),
        },
        prompt: "Inspect".to_owned(),
        parameters: Vec::new(),
    });
    let duplicate_instruction = RegistryBlock::Phase(PhaseBlock {
        identity: BlockIdentity {
            id: "duplicate-instruction".to_owned(),
            name: "Duplicate instruction".to_owned(),
        },
        instruction_refs: vec!["inspect".to_owned(), "inspect".to_owned()],
        ..test_phase()
    });
    let error = ResolvedRegistry::from_blocks([instruction, duplicate_instruction])
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
            identity: BlockIdentity {
                id: id.to_owned(),
                name: format!("Phase {id}"),
            },
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
            identity: BlockIdentity {
                id: "parent".to_owned(),
                name: "Parent".to_owned(),
            },
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
            identity: BlockIdentity {
                id: "parent".to_owned(),
                name: "Parent".to_owned(),
            },
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
        identity: BlockIdentity {
            id: "parent".to_owned(),
            name: "Parent".to_owned(),
        },
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
                identity: BlockIdentity {
                    id: format!("a-tail-{index:02}"),
                    name: format!("Shared tail {index:02}"),
                },
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
            identity: BlockIdentity {
                id: format!("b-prefix-{index:02}"),
                name: format!("Prefix {index:02}"),
            },
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
