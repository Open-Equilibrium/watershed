use super::super::model::{
    FlowValue, MAX_PHASE_LOOP_ITERATIONS, NetworkAllowEntry, NetworkAllowKind, NetworkDefault,
    NetworkPolicy, NetworkTransport, PhaseBlock, PhaseLoop, PhaseTransition, RegistryBlock,
    ValuePathSegment, ValuePredicate,
};
use super::super::semantics::{validate_registry_block_semantics, validate_tool_semantics};
use super::{own_script_tool, test_flow, test_phase, true_predicate};

#[test]
fn m11_phase_and_flow_semantics_reject_invalid_control_shapes() {
    let mut phase_cases = Vec::new();

    for max_iterations in [0, MAX_PHASE_LOOP_ITERATIONS + 1] {
        let mut phase = test_phase();
        let PhaseBlock { loop_config, .. } = &mut phase;
        *loop_config = Some(PhaseLoop {
            max_iterations,
            until: ValuePredicate {
                path: Vec::new(),
                equals: FlowValue::String("done".to_owned()),
            },
        });
        phase_cases.push(phase);
    }

    let mut mismatched_loop = test_phase();
    let PhaseBlock { loop_config, .. } = &mut mismatched_loop;
    *loop_config = Some(PhaseLoop {
        max_iterations: 2,
        until: true_predicate(),
    });
    phase_cases.push(mismatched_loop);

    let mut leaf_result = test_phase();
    leaf_result.result_from = Some("child".to_owned());
    phase_cases.push(leaf_result);

    let mut leaf_transition = test_phase();
    leaf_transition.transitions.push(PhaseTransition {
        from_phase_ref: "first".to_owned(),
        to_phase_ref: "second".to_owned(),
        when: true_predicate(),
    });
    phase_cases.push(leaf_transition);

    let mut composite_with_tool = test_phase();
    composite_with_tool.phase_refs.push("child".to_owned());
    composite_with_tool.result_from = Some("child".to_owned());
    composite_with_tool.tool_refs.push("tool".to_owned());
    phase_cases.push(composite_with_tool);

    let mut composite_without_result = test_phase();
    composite_without_result.phase_refs.push("child".to_owned());
    phase_cases.push(composite_without_result);

    let mut empty_transition_ref = test_phase();
    empty_transition_ref.phase_refs.push("child".to_owned());
    empty_transition_ref.result_from = Some("child".to_owned());
    empty_transition_ref.transitions.push(PhaseTransition {
        from_phase_ref: String::new(),
        to_phase_ref: "child".to_owned(),
        when: true_predicate(),
    });
    phase_cases.push(empty_transition_ref);

    let mut invalid_transition_predicate = test_phase();
    invalid_transition_predicate
        .phase_refs
        .push("child".to_owned());
    invalid_transition_predicate.result_from = Some("child".to_owned());
    invalid_transition_predicate
        .transitions
        .push(PhaseTransition {
            from_phase_ref: "child".to_owned(),
            to_phase_ref: "later".to_owned(),
            when: ValuePredicate {
                path: vec![ValuePathSegment::Field {
                    field: String::new(),
                }],
                equals: FlowValue::Boolean(true),
            },
        });
    phase_cases.push(invalid_transition_predicate);

    for phase in phase_cases {
        assert!(validate_registry_block_semantics(&RegistryBlock::Phase(phase)).is_err());
    }

    let mut empty_flow = test_flow();
    empty_flow.phase_refs.clear();
    let mut empty_flow_transition = test_flow();
    empty_flow_transition.transitions.push(PhaseTransition {
        from_phase_ref: "phase".to_owned(),
        to_phase_ref: String::new(),
        when: true_predicate(),
    });
    let mut invalid_flow_predicate = test_flow();
    invalid_flow_predicate.transitions.push(PhaseTransition {
        from_phase_ref: "phase".to_owned(),
        to_phase_ref: "later".to_owned(),
        when: ValuePredicate {
            path: vec![ValuePathSegment::Field {
                field: String::new(),
            }],
            equals: FlowValue::Boolean(true),
        },
    });
    for flow in [empty_flow, empty_flow_transition, invalid_flow_predicate] {
        assert!(validate_registry_block_semantics(&RegistryBlock::Flow(flow)).is_err());
    }

    let mut zero_port = own_script_tool("network-tool", "script:network-tool");
    zero_port.network = NetworkPolicy::Declared {
        default: NetworkDefault::Deny,
        allow: vec![NetworkAllowEntry {
            kind: NetworkAllowKind::Cidr,
            transport: NetworkTransport::Tcp,
            cidr: "127.0.0.1/32".to_owned(),
            port: 0,
        }],
    };
    assert!(validate_tool_semantics(&zero_port).is_err());
}
