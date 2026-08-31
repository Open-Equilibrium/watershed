use crate::runtime::{
    execution_plan::{
        FlowExecutionAction, FlowExecutionPlan, PlannedFlowFailureBoundary, ToolSideEffectMode,
    },
    types::{MAX_LIVE_FLOW_INVOCATIONS, RuntimeError},
};
use proto::{EventEnvelope, EventType};
use std::sync::atomic::{AtomicUsize, Ordering};

static LIVE_FLOW_INVOCATIONS: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct LiveFlowInvocationGuard;

pub(crate) fn acquire_live_flow_invocation() -> Result<LiveFlowInvocationGuard, RuntimeError> {
    let mut observed = LIVE_FLOW_INVOCATIONS.load(Ordering::Acquire);
    loop {
        if observed >= MAX_LIVE_FLOW_INVOCATIONS {
            return Err(RuntimeError::Protocol(format!(
                "global live flow invocation limit reached: max {MAX_LIVE_FLOW_INVOCATIONS}"
            )));
        }
        match LIVE_FLOW_INVOCATIONS.compare_exchange_weak(
            observed,
            observed + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Ok(LiveFlowInvocationGuard),
            Err(actual) => observed = actual,
        }
    }
}

impl Drop for LiveFlowInvocationGuard {
    fn drop(&mut self) {
        LIVE_FLOW_INVOCATIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

struct ActiveLiveInvocation {
    boundary: PlannedFlowFailureBoundary,
    _guard: Option<LiveFlowInvocationGuard>,
}

pub(super) struct LiveFlowInvocations {
    acquire_slots: bool,
    active: Vec<ActiveLiveInvocation>,
    prefix_event_count: u64,
    tracks_events: bool,
}

impl LiveFlowInvocations {
    pub(super) fn for_application(
        plan: &FlowExecutionPlan,
        side_effect_mode: ToolSideEffectMode,
    ) -> Result<Self, RuntimeError> {
        let Some((prefix_event_count, acquire_slots)) = (match side_effect_mode {
            ToolSideEffectMode::Apply => Some((0, true)),
            ToolSideEffectMode::Plan => None,
        }) else {
            return Ok(Self {
                acquire_slots: false,
                active: Vec::new(),
                prefix_event_count: 0,
                tracks_events: false,
            });
        };
        let mut tracker = Self {
            acquire_slots,
            active: Vec::new(),
            prefix_event_count,
            tracks_events: true,
        };
        for action in plan.actions.iter() {
            let FlowExecutionAction::Event(action) = action else {
                continue;
            };
            if action.event.sequence > prefix_event_count {
                break;
            }
            tracker.reconstruct_prefix_event(&action.event)?;
        }
        if tracker.acquire_slots {
            for active in &mut tracker.active {
                active._guard = Some(acquire_live_flow_invocation()?);
            }
        }
        Ok(tracker)
    }

    fn reconstruct_prefix_event(&mut self, event: &EventEnvelope) -> Result<(), RuntimeError> {
        match event.event_type {
            EventType::FlowStarted => self.active.push(ActiveLiveInvocation {
                boundary: flow_boundary(event)?,
                _guard: None,
            }),
            EventType::FlowCompleted | EventType::FlowFailed => self.finish(event),
            _ => {}
        }
        Ok(())
    }

    pub(super) fn before_event(&mut self, event: &EventEnvelope) -> Result<(), RuntimeError> {
        if !self.should_process(event) {
            return Ok(());
        }
        if event.event_type == EventType::FlowStarted {
            self.active.push(ActiveLiveInvocation {
                boundary: flow_boundary(event)?,
                _guard: if self.acquire_slots {
                    Some(acquire_live_flow_invocation()?)
                } else {
                    None
                },
            });
        }
        Ok(())
    }

    pub(super) fn after_event(&mut self, event: &EventEnvelope) {
        if self.should_process(event)
            && matches!(
                event.event_type,
                EventType::FlowCompleted | EventType::FlowFailed
            )
        {
            self.finish(event);
        }
    }

    fn finish(&mut self, event: &EventEnvelope) {
        let Some(flow_id) = event.flow_id.as_deref() else {
            return;
        };
        if let Some(index) = self
            .active
            .iter()
            .rposition(|active| active.boundary.flow_id == flow_id)
        {
            self.active.remove(index);
        }
    }

    pub(super) fn active_boundaries(&self) -> Vec<PlannedFlowFailureBoundary> {
        self.active
            .iter()
            .map(|active| active.boundary.clone())
            .collect()
    }

    pub(super) fn should_process(&self, event: &EventEnvelope) -> bool {
        self.tracks_events && event.sequence > self.prefix_event_count
    }

    pub(super) fn is_empty(&self) -> bool {
        self.active.is_empty()
    }
}

fn flow_boundary(event: &EventEnvelope) -> Result<PlannedFlowFailureBoundary, RuntimeError> {
    let flow_id = event.flow_id.clone().ok_or_else(|| {
        RuntimeError::Protocol("flow.started action is missing flow_id".to_owned())
    })?;
    let flow_definition_id = event
        .payload
        .get("flow_definition_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            RuntimeError::Protocol(
                "flow.started action is missing payload.flow_definition_id".to_owned(),
            )
        })?;
    Ok(PlannedFlowFailureBoundary {
        flow_definition_id: flow_definition_id.to_owned(),
        flow_id,
        parent_flow_id: event.parent_flow_id.clone(),
    })
}
