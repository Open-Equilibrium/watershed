use crate::runtime::{
    context::ContextManifestCheckpoint,
    event_writer::RuntimeEventSink,
    productive_capacity::ProductiveDispatchReservation,
    types::{CANCELLED_REASON, RuntimeError},
};
use proto::{EventEnvelope, EventType};
use std::time::Instant;
pub(in super::super) struct InterruptingSink {
    pub(in super::super) action: Option<crate::ProductiveInterruptAction>,
    pub(in super::super) events: Vec<EventEnvelope>,
    pub(in super::super) trigger: EventType,
}

impl RuntimeEventSink for InterruptingSink {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        _context_manifest: Option<ContextManifestCheckpoint>,
        _measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        self.events.push(event.clone());
        if event.event_type == self.trigger && self.action.is_none() {
            self.action = Some(crate::request_productive_interrupt());
        }
        Ok(())
    }
}

pub(in super::super) struct RejectingReservationSink {
    pub(in super::super) events: Vec<EventEnvelope>,
    pub(in super::super) reject_at: usize,
    pub(in super::super) reservations: Vec<ProductiveDispatchReservation>,
}

impl RejectingReservationSink {
    pub(in super::super) fn new(reject_at: usize) -> Self {
        Self {
            events: Vec::new(),
            reject_at,
            reservations: Vec::new(),
        }
    }
}

impl RuntimeEventSink for RejectingReservationSink {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        _context_manifest: Option<ContextManifestCheckpoint>,
        _measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        self.events.push(event.clone());
        Ok(())
    }

    fn reserve_productive_dispatch(
        &mut self,
        reservation: ProductiveDispatchReservation,
    ) -> Result<(), RuntimeError> {
        self.reservations.push(reservation);
        if self.reservations.len() == self.reject_at {
            return Err(RuntimeError::Protocol(
                "fixture productive storage reservation rejected".to_owned(),
            ));
        }
        Ok(())
    }
}

pub(in super::super) fn assert_controlled_cancellation_lifecycle(events: &[EventEnvelope]) {
    for (event_type, field) in [
        (EventType::PhaseFailed, "error"),
        (EventType::FlowFailed, "error"),
        (EventType::SessionFailed, "reason"),
    ] {
        let event = events
            .iter()
            .find(|event| event.event_type == event_type)
            .unwrap_or_else(|| panic!("missing {event_type:?}"));
        assert_eq!(event.payload[field], CANCELLED_REASON);
    }
    let error = events
        .iter()
        .find(|event| event.event_type == EventType::Error)
        .expect("controlled cancellation emits one runtime error");
    assert_eq!(error.payload["code"], CANCELLED_REASON);
    assert!(!events.iter().any(|event| {
        matches!(
            event.event_type,
            EventType::PhaseCompleted | EventType::FlowCompleted | EventType::SessionCompleted
        )
    }));
}
