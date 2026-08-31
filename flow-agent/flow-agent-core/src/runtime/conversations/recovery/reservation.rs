use super::super::run_objects::read_run_object_uri;
use super::super::{
    contract::{
        CONVERSATION_RUNS_DIR, RUN_CONTEXTS_STEM, RUN_EVENTS_LEAF, RUN_EVENTS_STEM,
        RUN_OBJECTS_DIR, RUN_SESSION_LOCK_LEAF, protocol, validate_id,
    },
    history_index::with_conversation_history_index,
    lifecycle::{
        conversation_candidate_is_occupied, reclaim_unpublished_productive_run, reconcile_releases,
    },
    prefix_reader::{RecoveryPrefixReader, canonical_jsonl_record},
    recovery_record::ProductiveRecoveryRecord,
    run_log::inspect_run_attempts,
    storage::{
        ensure_runtime_roots, existing_anchored_conversation, existing_anchored_run, required_child,
    },
};
use crate::runtime::{
    context::ContextHistory,
    fs_guards::path_io_error,
    run_attempts::RunAttemptLifecycle,
    session_authority::{SessionOwnershipLease, conversation_ownership_key, run_ownership_key},
    session_candidates::{MAX_UNIQUE_SESSION_CANDIDATES, suffixed_session_id},
    session_definition::SessionLogMetadata,
    stage_results::reconcile_operation_and_cleanup,
    types::{
        CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, MAX_CANONICAL_EVENT_BYTES,
        MAX_SESSION_SEGMENT_BYTES, RuntimeError,
    },
    validate::SessionAppendValidationState,
};
use std::path::Path;

use super::selection::{
    conversation_run_definition, read_productive_recovery_header, selected_entry_recovery_history,
};

pub(crate) struct ProductiveConversationReservation {
    conversation_id: String,
    conversation_lease: SessionOwnershipLease,
    parent_entry_id: Option<String>,
    prior_history: ContextHistory,
    prior_event_count: usize,
    recorded_definition: Option<SessionLogMetadata>,
    recovery_event_clock: Option<crate::runtime::types::EventClock>,
    recovery_root_input: Option<core_script::FlowValue>,
    run_lease: SessionOwnershipLease,
    run_session_id: String,
}

struct ProductiveRecoveryReservationState {
    event_clock: crate::runtime::types::EventClock,
    parent_entry_id: Option<String>,
    prior_event_count: usize,
    prior_history: ContextHistory,
    recorded_definition: SessionLogMetadata,
    root_input: Option<core_script::FlowValue>,
}

struct ProductiveContinuationReservationState {
    parent_entry_id: String,
    prior_event_count: usize,
    prior_history: ContextHistory,
    recorded_definition: SessionLogMetadata,
    run_lease: SessionOwnershipLease,
    run_session_id: String,
}

struct ConversationRunOwnership {
    conversation: SessionOwnershipLease,
    run: SessionOwnershipLease,
}

impl ConversationRunOwnership {
    fn acquire(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
        marker: &Path,
    ) -> Result<Self, RuntimeError> {
        let conversation_key = conversation_ownership_key(conversation_id);
        let conversation = SessionOwnershipLease::acquire(workspace, &conversation_key, marker)?;
        let run_key = run_ownership_key(conversation_id, run_session_id);
        let run = match SessionOwnershipLease::acquire(workspace, &run_key, marker) {
            Ok(lease) => lease,
            Err(error) => {
                return reconcile_operation_and_cleanup(Err(error), conversation.release());
            }
        };
        Ok(Self { conversation, run })
    }

    fn release(self) -> Result<(), RuntimeError> {
        reconcile_releases(self.run.release(), self.conversation.release())
    }
}

impl ProductiveConversationReservation {
    pub(crate) fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    pub(crate) fn run_session_id(&self) -> &str {
        &self.run_session_id
    }

    pub(crate) fn parent_entry_id(&self) -> Option<&str> {
        self.parent_entry_id.as_deref()
    }

    pub(crate) fn prior_history(&self) -> &ContextHistory {
        &self.prior_history
    }

    pub(crate) fn prior_event_count(&self) -> usize {
        self.prior_event_count
    }

    pub(crate) fn recorded_definition(&self) -> Option<&SessionLogMetadata> {
        self.recorded_definition.as_ref()
    }

    pub(crate) fn recovery_event_clock(&self) -> Option<crate::runtime::types::EventClock> {
        self.recovery_event_clock
    }

    pub(crate) fn recovery_root_input(&self) -> Option<&core_script::FlowValue> {
        self.recovery_root_input.as_ref()
    }

    pub(crate) fn release(self) -> Result<(), RuntimeError> {
        let run = self.run_lease.release();
        let conversation = self.conversation_lease.release();
        reconcile_releases(run, conversation)
    }
}

pub(crate) fn reserve_new_conversation_run(
    workspace: &Path,
    base_id: &str,
) -> Result<ProductiveConversationReservation, RuntimeError> {
    validate_id(base_id, "conversation")?;
    let roots = ensure_runtime_roots(workspace)?;
    SessionOwnershipLease::ensure_store_available(workspace)?;
    for ordinal in 1..=MAX_UNIQUE_SESSION_CANDIDATES {
        let candidate = if ordinal == 1 {
            base_id.to_owned()
        } else {
            suffixed_session_id(base_id, ordinal)
        };
        let marker = roots
            .sessions
            .path
            .join(&candidate)
            .join(CONVERSATION_RUNS_DIR)
            .join(&candidate)
            .join(RUN_SESSION_LOCK_LEAF);
        let ownership =
            match ConversationRunOwnership::acquire(workspace, &candidate, &candidate, &marker) {
                Ok(ownership) => ownership,
                Err(RuntimeError::ActiveSession { .. }) => continue,
                Err(error) => return Err(error),
            };
        let occupied = reclaim_unpublished_productive_run(workspace, &candidate, &candidate)
            .and_then(|()| {
                conversation_candidate_is_occupied(&roots.sessions, &roots.logs, &candidate)
            });
        match occupied {
            Ok(true) => {
                ownership.release()?;
                continue;
            }
            Ok(false) => {}
            Err(error) => {
                return reconcile_operation_and_cleanup(Err(error), ownership.release());
            }
        }
        let ConversationRunOwnership {
            conversation: conversation_lease,
            run: run_lease,
        } = ownership;
        return Ok(ProductiveConversationReservation {
            conversation_id: candidate.clone(),
            conversation_lease,
            parent_entry_id: None,
            prior_history: ContextHistory::default(),
            prior_event_count: 0,
            recorded_definition: None,
            recovery_event_clock: None,
            recovery_root_input: None,
            run_lease,
            run_session_id: candidate,
        });
    }
    Err(protocol(format!(
        "could not allocate a unique conversation id for {base_id}"
    )))
}

pub(crate) fn reserve_conversation_continuation(
    workspace: &Path,
    conversation_id: &str,
    from_entry_id: Option<&str>,
) -> Result<ProductiveConversationReservation, RuntimeError> {
    validate_id(conversation_id, "conversation")?;
    if let Some(entry_id) = from_entry_id {
        validate_id(entry_id, "conversation entry")?;
    }
    let conversation = existing_anchored_conversation(workspace, conversation_id)?;
    SessionOwnershipLease::ensure_store_available(workspace)?;
    let marker = conversation
        .path
        .join(CONVERSATION_RUNS_DIR)
        .join(conversation_id)
        .join(RUN_SESSION_LOCK_LEAF);
    let conversation_key = conversation_ownership_key(conversation_id);
    let conversation_lease = SessionOwnershipLease::acquire(workspace, &conversation_key, &marker)?;

    let operation = with_conversation_history_index(
        workspace,
        conversation_id,
        from_entry_id,
        None,
        #[cfg(test)]
        None,
        |_index, summary| {
            let selected = match from_entry_id {
                Some(entry_id) => summary.selected.ok_or_else(|| {
                    RuntimeError::PersistedState(format!(
                        "conversation {conversation_id} has no entry {entry_id}"
                    ))
                })?,
                None => summary.latest.ok_or_else(|| {
                    RuntimeError::PersistedState(format!(
                        "conversation {conversation_id} has no committed entry to continue"
                    ))
                })?,
            };
            let (prior_history, prior_event_count) =
                selected_entry_recovery_history(workspace, conversation_id, &selected)?;
            let recorded_definition =
                conversation_run_definition(workspace, conversation_id, &selected.run_session_id)?;
            let runs = required_child(
                &conversation,
                CONVERSATION_RUNS_DIR,
                "conversation runs directory",
            )?;

            for ordinal in 1..=MAX_UNIQUE_SESSION_CANDIDATES {
                let candidate = if ordinal == 1 {
                    conversation_id.to_owned()
                } else {
                    suffixed_session_id(conversation_id, ordinal)
                };
                let candidate_path = runs.path.join(&candidate);
                match runs.dir.symlink_metadata(&candidate) {
                    Ok(_) => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(source) => return Err(path_io_error(&candidate_path, source)),
                }
                let run_marker = candidate_path.join(RUN_SESSION_LOCK_LEAF);
                let run_key = run_ownership_key(conversation_id, &candidate);
                let run_lease =
                    match SessionOwnershipLease::acquire(workspace, &run_key, &run_marker) {
                        Ok(lease) => lease,
                        Err(RuntimeError::ActiveSession { .. }) => continue,
                        Err(error) => return Err(error),
                    };
                match runs.dir.symlink_metadata(&candidate) {
                    Ok(_) => {
                        run_lease.release()?;
                        continue;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(source) => {
                        return reconcile_operation_and_cleanup(
                            Err(path_io_error(&candidate_path, source)),
                            run_lease.release(),
                        );
                    }
                }
                return Ok(ProductiveContinuationReservationState {
                    parent_entry_id: selected.entry_id,
                    prior_history,
                    prior_event_count,
                    recorded_definition,
                    run_lease,
                    run_session_id: candidate,
                });
            }
            Err(protocol(format!(
                "could not allocate a unique run id for conversation {conversation_id}"
            )))
        },
    );
    match operation {
        Ok(state) => Ok(ProductiveConversationReservation {
            conversation_id: conversation_id.to_owned(),
            conversation_lease,
            parent_entry_id: Some(state.parent_entry_id),
            prior_history: state.prior_history,
            prior_event_count: state.prior_event_count,
            recorded_definition: Some(state.recorded_definition),
            recovery_event_clock: None,
            recovery_root_input: None,
            run_lease: state.run_lease,
            run_session_id: state.run_session_id,
        }),
        Err(error) => reconcile_operation_and_cleanup(Err(error), conversation_lease.release()),
    }
}

pub(crate) fn reserve_conversation_run_recovery(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
) -> Result<ProductiveConversationReservation, RuntimeError> {
    validate_id(conversation_id, "conversation")?;
    validate_id(run_session_id, "run session")?;
    let run = existing_anchored_run(workspace, conversation_id, run_session_id)?;
    SessionOwnershipLease::ensure_store_available(workspace)?;
    let marker = run.file(RUN_SESSION_LOCK_LEAF);
    let ConversationRunOwnership {
        conversation: conversation_lease,
        run: run_lease,
    } = ConversationRunOwnership::acquire(
        workspace,
        conversation_id,
        run_session_id,
        marker.diagnostic_path(),
    )?;

    let operation = (|| {
        let terminal = with_conversation_history_index(
            workspace,
            conversation_id,
            None,
            Some(run_session_id),
            #[cfg(test)]
            None,
            |_index, summary| Ok(summary.contains_run),
        )?;
        if terminal {
            return Err(RuntimeError::TerminalSession(run_session_id.to_owned()));
        }
        let uncertain = inspect_run_attempts(workspace, conversation_id, run_session_id)?
            .iter()
            .filter(|attempt| attempt.lifecycle == RunAttemptLifecycle::Uncertain)
            .count();
        if uncertain > 0 {
            return Err(RuntimeError::PersistedState(format!(
                "run {run_session_id} has {uncertain} uncertain productive attempt(s); reconcile them from external evidence before Resume"
            )));
        }
        let event_path = run.file(RUN_EVENTS_LEAF);
        let mut event_prefix = RecoveryPrefixReader::open(
            &run,
            RUN_EVENTS_STEM,
            EVENT_STREAM_LIMITS,
            MAX_CANONICAL_EVENT_BYTES,
        )?;
        let mut event_validation = SessionAppendValidationState::empty(run_session_id);
        while let Some(line) = event_prefix.next_line()? {
            event_validation
                .validate_appended_with(event_path.diagnostic_path(), &line, |_| Ok(()))?;
        }
        let mut context_prefix = RecoveryPrefixReader::open(
            &run,
            RUN_CONTEXTS_STEM,
            CONTEXT_MANIFEST_STREAM_LIMITS,
            usize::try_from(MAX_SESSION_SEGMENT_BYTES).unwrap_or(usize::MAX),
        )?;
        while let Some(line) = context_prefix.next_line()? {
            canonical_jsonl_record(&line, "context")?;
        }

        let header = read_productive_recovery_header(&run, conversation_id, run_session_id)?;
        let ProductiveRecoveryRecord::Header {
            flow_definition_id,
            registry_hash,
            flow_definition_hash,
            root_input,
            parent_entry_id,
            event_clock_base_unix_seconds,
            prior_history_object,
            prior_event_count,
            ..
        } = header
        else {
            unreachable!("recovery header reader only returns Header records")
        };
        let recorded_definition =
            conversation_run_definition(workspace, conversation_id, run_session_id)?;
        if recorded_definition.flow_definition_id.as_deref() != Some(&flow_definition_id)
            || recorded_definition.registry_hash.as_deref() != Some(&registry_hash)
            || recorded_definition.flow_definition_hash.as_deref() != Some(&flow_definition_hash)
        {
            return Err(protocol(
                "productive recovery header conflicts with the run definition",
            ));
        }
        let objects = required_child(&run, RUN_OBJECTS_DIR, "run object directory")?;
        let prior_history = ContextHistory::from_recovery_bytes(&read_run_object_uri(
            &objects,
            &prior_history_object,
        )?)?;
        let prior_event_count = usize::try_from(prior_event_count)
            .map_err(|_| protocol("prior conversation event count exceeds this platform"))?;
        let recovery_root_input = if root_input.is_null() {
            None
        } else {
            Some(
                core_script::parse_flow_value_v0(root_input).map_err(|error| {
                    protocol(format!(
                        "productive recovery root input is invalid: {error}"
                    ))
                })?,
            )
        };
        Ok(ProductiveRecoveryReservationState {
            event_clock: crate::runtime::types::EventClock {
                base_unix_seconds: event_clock_base_unix_seconds,
            },
            parent_entry_id,
            prior_history,
            prior_event_count,
            recorded_definition,
            root_input: recovery_root_input,
        })
    })();
    match operation {
        Ok(state) => Ok(ProductiveConversationReservation {
            conversation_id: conversation_id.to_owned(),
            conversation_lease,
            parent_entry_id: state.parent_entry_id,
            prior_history: state.prior_history,
            prior_event_count: state.prior_event_count,
            recorded_definition: Some(state.recorded_definition),
            recovery_event_clock: Some(state.event_clock),
            recovery_root_input: state.root_input,
            run_lease,
            run_session_id: run_session_id.to_owned(),
        }),
        Err(error) => {
            let release = reconcile_releases(run_lease.release(), conversation_lease.release());
            reconcile_operation_and_cleanup(Err(error), release)
        }
    }
}

pub(crate) fn with_conversation_run_ownership<T>(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    operation: impl FnOnce() -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    validate_id(conversation_id, "conversation")?;
    validate_id(run_session_id, "run session")?;
    let run = existing_anchored_run(workspace, conversation_id, run_session_id)?;
    SessionOwnershipLease::ensure_store_available(workspace)?;
    let marker = run.file(RUN_SESSION_LOCK_LEAF);
    let ownership = ConversationRunOwnership::acquire(
        workspace,
        conversation_id,
        run_session_id,
        marker.diagnostic_path(),
    )?;
    let result = operation();
    reconcile_operation_and_cleanup(result, ownership.release())
}
