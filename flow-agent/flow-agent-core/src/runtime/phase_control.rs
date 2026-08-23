use super::types::RuntimeError;
use core_script::{FlowValue, PhaseLoop, PhaseTransition};

pub(crate) struct PhaseSequenceState {
    index: usize,
    next_input: Option<FlowValue>,
    selected_result: Option<FlowValue>,
}

impl PhaseSequenceState {
    pub(crate) fn new(input: Option<FlowValue>) -> Self {
        Self {
            index: 0,
            next_input: input,
            selected_result: None,
        }
    }

    pub(crate) fn current_index(&self, phase_count: usize) -> Option<usize> {
        (self.index < phase_count).then_some(self.index)
    }

    pub(crate) fn take_input(&mut self) -> Option<FlowValue> {
        self.next_input.take()
    }

    pub(crate) fn advance(
        &mut self,
        phase_refs: &[String],
        transitions: &[PhaseTransition],
        result_from: Option<&str>,
        result: FlowValue,
    ) -> Result<usize, RuntimeError> {
        let phase_ref = &phase_refs[self.index];
        if result_from == Some(phase_ref.as_str()) {
            self.selected_result = Some(result.clone());
        }
        self.next_input = Some(result.clone());

        let next = select_next_phase_index(phase_refs, transitions, self.index, &result);
        if let Some(result_from) = result_from
            && self.selected_result.is_none()
            && phase_refs
                .get(self.index.saturating_add(1)..next)
                .is_some_and(|skipped| skipped.iter().any(|phase| phase == result_from))
        {
            return Err(RuntimeError::Protocol(format!(
                "composite Phase result_from {result_from} was skipped by a Transition"
            )));
        }
        self.index = next;
        Ok(next)
    }

    pub(crate) fn finish(
        self,
        result_from: Option<&str>,
    ) -> Result<Option<FlowValue>, RuntimeError> {
        if let Some(result_from) = result_from {
            self.selected_result.map(Some).ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "composite Phase result_from {result_from} was skipped by a Transition"
                ))
            })
        } else {
            Ok(self.next_input)
        }
    }
}

pub(crate) fn phase_should_repeat(result: &FlowValue, loop_config: Option<&PhaseLoop>) -> bool {
    loop_config.is_some_and(|config| {
        !core_script::predicate_matches(result, &config.until)
            .expect("validated Phase loop predicate accepts its output contract")
    })
}

pub(crate) fn select_next_phase_index(
    phase_refs: &[String],
    transitions: &[PhaseTransition],
    index: usize,
    result: &FlowValue,
) -> usize {
    let phase_ref = &phase_refs[index];
    for transition in transitions
        .iter()
        .filter(|transition| transition.from_phase_ref == *phase_ref)
    {
        let matched = core_script::predicate_matches(result, &transition.when)
            .expect("validated Transition predicate accepts its source Phase output contract");
        if matched {
            return phase_refs
                .iter()
                .position(|candidate| candidate == &transition.to_phase_ref)
                .expect("validated Transition target remains in the Phase sequence");
        }
    }
    index + 1
}
