//! Flow Agent M1 deterministic runtime.

#![deny(missing_docs)]
#![cfg_attr(not(test), deny(clippy::wildcard_imports))]

mod runtime;

pub use runtime::{
    EmitMode, LiveEventNotification, LiveEventNotifier, LiveEventNotifyStatus,
    LiveEventReceiveError, LiveEventReceiver, RunOutput, RuntimeError, SessionEventReader,
    list_sessions, live_event_channel, render_human_failure_status, replay_session, resume_session,
    resume_session_with_live_events, run_flow, run_flow_with_live_events,
    validate_protocol_jsonl_text,
};

#[cfg(test)]
mod tests;
