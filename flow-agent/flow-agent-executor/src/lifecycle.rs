use std::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InnerStatusPolicy {
    Ignore,
    IfReportable,
    RequiredAndClassify,
}

pub(crate) fn inner_status_policy(
    classification: Option<proto::ExecutorToolClassificationV0>,
) -> InnerStatusPolicy {
    use proto::ExecutorToolClassificationV0 as Classification;

    match classification {
        None | Some(Classification::NonzeroExit | Classification::SignalTermination) => {
            InnerStatusPolicy::RequiredAndClassify
        }
        Some(
            Classification::StderrCapExceeded
            | Classification::StdoutCapExceeded
            | Classification::StdoutStderrCapExceeded
            | Classification::OutputCollectorFailed
            | Classification::OutputDrainTimeout,
        ) => InnerStatusPolicy::IfReportable,
        Some(
            Classification::Cancelled
            | Classification::ProcessCapacityExceeded
            | Classification::ToolTimedOut,
        ) => InnerStatusPolicy::Ignore,
    }
}

pub(crate) fn capacity_can_classify(
    classification: Option<proto::ExecutorToolClassificationV0>,
) -> bool {
    use proto::ExecutorToolClassificationV0 as Classification;

    matches!(
        classification,
        None | Some(Classification::NonzeroExit | Classification::SignalTermination)
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CleanupAction {
    Wait,
    ForceKill,
    FailClosed,
    Complete,
    OutputDrainTimeout,
}

enum CleanupPhase {
    TermGrace,
    ForcedReap,
    OutputDrain,
}

pub(crate) struct CleanupController {
    phase: CleanupPhase,
    deadline: Instant,
}

impl CleanupController {
    pub(crate) fn new(started: Instant) -> Self {
        Self {
            phase: CleanupPhase::TermGrace,
            deadline: started + proto::TOOL_TERMINATION_GRACE_V0,
        }
    }

    pub(crate) fn advance(
        &mut self,
        now: Instant,
        cleanup_complete: bool,
        output_drained: bool,
    ) -> CleanupAction {
        match self.phase {
            CleanupPhase::TermGrace if cleanup_complete => {
                self.begin_output_drain(now, output_drained)
            }
            CleanupPhase::TermGrace if now >= self.deadline => {
                self.phase = CleanupPhase::ForcedReap;
                self.deadline = now + proto::TOOL_FORCED_REAP_DEADLINE_V0;
                CleanupAction::ForceKill
            }
            CleanupPhase::TermGrace => CleanupAction::Wait,
            CleanupPhase::ForcedReap if cleanup_complete => {
                self.begin_output_drain(now, output_drained)
            }
            CleanupPhase::ForcedReap if now >= self.deadline => CleanupAction::FailClosed,
            CleanupPhase::ForcedReap => CleanupAction::Wait,
            CleanupPhase::OutputDrain if output_drained => CleanupAction::Complete,
            CleanupPhase::OutputDrain if now >= self.deadline => CleanupAction::OutputDrainTimeout,
            CleanupPhase::OutputDrain => CleanupAction::Wait,
        }
    }

    fn begin_output_drain(&mut self, now: Instant, output_drained: bool) -> CleanupAction {
        self.phase = CleanupPhase::OutputDrain;
        self.deadline = now + proto::TOOL_OUTPUT_DRAIN_DEADLINE_V0;
        if output_drained {
            CleanupAction::Complete
        } else {
            CleanupAction::Wait
        }
    }
}
