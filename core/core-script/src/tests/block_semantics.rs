use super::super::model::{
    FlowValue, MAX_PHASE_LOOP_ITERATIONS, PhaseBlock, PhaseLoop, PhaseTransition, RegistryBlock,
    ValuePathSegment, ValuePredicate,
};
use super::super::semantics::validate_registry_block_semantics;
use super::{test_flow, test_phase, true_predicate};

fn transition(from: &str, to: &str, when: ValuePredicate) -> PhaseTransition {
    PhaseTransition {
        from_phase_ref: from.to_owned(),
        to_phase_ref: to.to_owned(),
        when,
    }
}

fn invalid_phase(edit: impl FnOnce(&mut PhaseBlock)) -> RegistryBlock {
    let mut phase = test_phase();
    edit(&mut phase);
    RegistryBlock::Phase(phase)
}

fn composite_phase(edit: impl FnOnce(&mut PhaseBlock)) -> RegistryBlock {
    invalid_phase(|phase| {
        phase.phase_refs.push("child".to_owned());
        phase.result_from = Some("child".to_owned());
        edit(phase);
    })
}

#[test]
fn m11_phase_and_flow_semantics_reject_invalid_control_shapes() {
    let empty_field = ValuePredicate {
        path: vec![ValuePathSegment::Field {
            field: String::new(),
        }],
        equals: FlowValue::Boolean(true),
    };
    let loop_with = |max_iterations| {
        invalid_phase(|phase| {
            phase.loop_config = Some(PhaseLoop {
                max_iterations,
                until: ValuePredicate {
                    path: Vec::new(),
                    equals: FlowValue::String("done".to_owned()),
                },
            })
        })
    };
    let phases = [
        loop_with(0),
        loop_with(MAX_PHASE_LOOP_ITERATIONS + 1),
        invalid_phase(|phase| {
            phase.loop_config = Some(PhaseLoop {
                max_iterations: 2,
                until: true_predicate(),
            })
        }),
        invalid_phase(|phase| phase.result_from = Some("child".to_owned())),
        invalid_phase(|phase| {
            phase
                .transitions
                .push(transition("first", "second", true_predicate()))
        }),
        composite_phase(|phase| phase.tool_refs.push("tool".to_owned())),
        invalid_phase(|phase| phase.phase_refs.push("child".to_owned())),
        composite_phase(|phase| {
            phase
                .transitions
                .push(transition("", "child", true_predicate()))
        }),
        composite_phase(|phase| {
            phase
                .transitions
                .push(transition("child", "later", empty_field.clone()))
        }),
    ];
    for phase in phases {
        assert!(validate_registry_block_semantics(&phase).is_err());
    }

    let mut empty_flow = test_flow();
    empty_flow.phase_refs.clear();
    let mut empty_flow_transition = test_flow();
    empty_flow_transition
        .transitions
        .push(transition("phase", "", true_predicate()));
    let mut invalid_flow_predicate = test_flow();
    invalid_flow_predicate
        .transitions
        .push(transition("phase", "later", empty_field));
    for flow in [empty_flow, empty_flow_transition, invalid_flow_predicate] {
        assert!(validate_registry_block_semantics(&RegistryBlock::Flow(flow)).is_err());
    }
}
