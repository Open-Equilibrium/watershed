use crate::runtime::run_attempts::ProductiveRecovery;
#[cfg(test)]
use crate::runtime::run_attempts::RunAttemptKind;
use crate::runtime::{
    context::ContextModelProfile,
    conversations::MAX_CONVERSATION_RECORD_BYTES,
    executor::{ExecutorDispatchOutcome, PreparedExecutor, PreparedExecutorTool},
    fs_guards::AnchoredWorkspace,
    oauth_credential::CredentialRecord,
    openai_codex::{OPENAI_CODEX_RESPONSES_URL, ProviderTurn, request_responses_at},
    productive_capacity::ProductiveDispatchReservation,
    responses::MAX_RESPONSES_DECODED_STREAM_BYTES,
    tool_runner::{MAX_TOOL_STREAM_BYTES, ToolInvocation},
    types::{
        CANCELLED_REASON, EventClock, MAX_SESSION_OBJECT_BYTES, RUNTIME_ERROR_REASON, RuntimeError,
    },
};

mod execution;
mod platform;
mod provider_result;
mod provider_turn;
mod reconciliation;
mod tool;
mod tool_result;
pub(crate) use execution::execute_productive_flow_with_tool_executor_and_recovery;
#[cfg(test)]
pub(crate) use execution::{
    execute_productive_flow, execute_productive_flow_with_recovery,
    execute_productive_flow_with_tool_executor,
};
pub(crate) use platform::ensure_productive_execution_platform;
pub(crate) use platform::ensure_productive_tool_execution_platform;
#[cfg(test)]
pub(crate) use platform::productive_execution_supported_release;
#[cfg(test)]
pub(crate) use platform::productive_tool_execution_supported_release;
#[cfg(test)]
pub(crate) use provider_result::MAX_ACCUMULATED_PROVIDER_INPUT_BYTES;
pub(crate) use provider_result::MAX_DURABLE_PROVIDER_OUTPUT_BYTES;
#[cfg(test)]
pub(crate) use provider_result::{
    ProviderInput, durable_provider_output, parse_provider_result,
    provider_turn_from_durable_output, verify_provider_result_session_objects,
};
pub use reconciliation::{
    MAX_TOOL_RECONCILIATION_BYTES, read_tool_reconciliation_file, reconcile_tool_attempt,
};
#[cfg(all(test, unix))]
pub(crate) use tool::SystemProductiveToolExecutor;
#[cfg(test)]
pub(crate) use tool::recovered_tool_value;
#[cfg(test)]
pub(crate) use tool::{
    recovered_tool_terminal, test_enforcement_receipt, tool_result_value, tool_terminal,
};

const PROVIDER_CANCELLED_SCHEMA_V0: &str = "flow-provider-cancelled-v0";
const PROVIDER_ERROR_SCHEMA_V0: &str = "flow-provider-error-v0";
const PROVIDER_OUTPUT_SCHEMA_V2: &str = "flow-provider-output-v2";
const EXECUTOR_DISPATCH_ERROR_SCHEMA_V0: &str = "flow-executor-dispatch-error-v0";
const TOOL_ATTEMPT_OUTPUT_SCHEMA_V1: &str = "flow-tool-attempt-output-v1";

#[cfg(test)]
type ProductiveResultPersistObserver = (RunAttemptKind, Box<dyn FnOnce()>);

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProductiveCompletionCommitPoint {
    FlowRecovery,
    FlowEvent,
    TransitionRecovery,
    PhaseRecovery,
    PhaseEvent,
}

#[cfg(test)]
type ProductiveCompletionCommitObserver = (ProductiveCompletionCommitPoint, Box<dyn FnOnce()>);

#[cfg(test)]
thread_local! {
    static PRODUCTIVE_RESULT_PERSIST_OBSERVER: std::cell::RefCell<Option<ProductiveResultPersistObserver>> =
        std::cell::RefCell::new(None);
    static PRODUCTIVE_COMPLETION_COMMIT_OBSERVER: std::cell::RefCell<Option<ProductiveCompletionCommitObserver>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_productive_completion_commit_observer(
    point: ProductiveCompletionCommitPoint,
    observer: impl FnOnce() + 'static,
) {
    PRODUCTIVE_COMPLETION_COMMIT_OBSERVER.with_borrow_mut(|slot| {
        *slot = Some((point, Box::new(observer)));
    });
}

#[cfg(test)]
fn observe_productive_completion_commit(point: ProductiveCompletionCommitPoint) {
    let observer = PRODUCTIVE_COMPLETION_COMMIT_OBSERVER.with_borrow_mut(|slot| {
        if slot.as_ref().is_some_and(|(target, _)| *target == point) {
            slot.take().map(|(_, observer)| observer)
        } else {
            None
        }
    });
    if let Some(observer) = observer {
        observer();
    }
}

#[cfg(test)]
pub(crate) fn set_productive_result_persist_observer(
    kind: RunAttemptKind,
    observer: impl FnOnce() + 'static,
) {
    PRODUCTIVE_RESULT_PERSIST_OBSERVER.with_borrow_mut(|slot| {
        *slot = Some((kind, Box::new(observer)));
    });
}

#[cfg(test)]
fn observe_productive_result_persist(kind: RunAttemptKind) {
    let observer = PRODUCTIVE_RESULT_PERSIST_OBSERVER.with_borrow_mut(|slot| {
        if slot.as_ref().is_some_and(|(target, _)| *target == kind) {
            slot.take().map(|(_, observer)| observer)
        } else {
            None
        }
    });
    if let Some(observer) = observer {
        observer();
    }
}

pub(crate) trait ProductiveProvider {
    fn turn(
        &mut self,
        credential: &CredentialRecord,
        body: &serde_json::Value,
    ) -> Result<ProviderTurn, RuntimeError>;
}

#[cfg(test)]
struct NoopProductiveRecovery;

#[cfg(test)]
impl ProductiveRecovery for NoopProductiveRecovery {}

pub(crate) struct OpenAiCodexProvider;

impl ProductiveProvider for OpenAiCodexProvider {
    fn turn(
        &mut self,
        credential: &CredentialRecord,
        body: &serde_json::Value,
    ) -> Result<ProviderTurn, RuntimeError> {
        request_responses_at(OPENAI_CODEX_RESPONSES_URL, credential, body)
    }
}

pub(crate) trait ProductiveToolExecutor {
    type Prepared;

    fn supports_productive_tools(&self) -> bool;

    fn prepare(
        &mut self,
        invocation: &ToolInvocation,
        workspace: &AnchoredWorkspace,
        policy: &core_policy::PolicyArtifact,
        command_policy: &core_policy::CommandPolicy,
        request_id: &str,
    ) -> Result<Self::Prepared, RuntimeError>;

    fn request_hash<'a>(&self, prepared: &'a Self::Prepared) -> &'a str;

    fn policy_digest<'a>(&self, prepared: &'a Self::Prepared) -> &'a str;

    fn runtime_profile(&self, prepared: &Self::Prepared) -> proto::RuntimeReadProfileV0;

    fn validate_enforcement_receipt(
        &self,
        prepared: &Self::Prepared,
        receipt: &proto::EnforcementReceiptV0,
    ) -> Result<(), RuntimeError> {
        proto::validate_enforcement_receipt_v0(
            receipt,
            self.policy_digest(prepared),
            self.runtime_profile(prepared),
        )
        .map_err(|_| {
            RuntimeError::Protocol(
                "Executor enforcement receipt does not match its prepared request".to_owned(),
            )
        })
    }

    fn execute_prepared(
        &mut self,
        prepared: Self::Prepared,
    ) -> Result<ExecutorDispatchOutcome, RuntimeError>;
}

impl ProductiveToolExecutor for Option<PreparedExecutor> {
    type Prepared = PreparedExecutorTool;

    fn supports_productive_tools(&self) -> bool {
        self.is_some()
    }

    fn prepare(
        &mut self,
        invocation: &ToolInvocation,
        workspace: &AnchoredWorkspace,
        policy: &core_policy::PolicyArtifact,
        command_policy: &core_policy::CommandPolicy,
        request_id: &str,
    ) -> Result<Self::Prepared, RuntimeError> {
        self.as_ref()
            .ok_or(RuntimeError::ProductiveExecutionUnavailable)?
            .prepare_tool(workspace, policy, command_policy, invocation, request_id)
    }

    fn request_hash<'a>(&self, prepared: &'a Self::Prepared) -> &'a str {
        prepared.request_hash()
    }

    fn policy_digest<'a>(&self, prepared: &'a Self::Prepared) -> &'a str {
        prepared.policy_digest()
    }

    fn runtime_profile(&self, prepared: &Self::Prepared) -> proto::RuntimeReadProfileV0 {
        prepared.runtime_profile()
    }

    fn execute_prepared(
        &mut self,
        prepared: Self::Prepared,
    ) -> Result<ExecutorDispatchOutcome, RuntimeError> {
        self.as_ref()
            .ok_or(RuntimeError::ProductiveExecutionUnavailable)?
            .execute_prepared(prepared)
    }

    fn validate_enforcement_receipt(
        &self,
        prepared: &Self::Prepared,
        receipt: &proto::EnforcementReceiptV0,
    ) -> Result<(), RuntimeError> {
        self.as_ref()
            .ok_or(RuntimeError::ProductiveExecutionUnavailable)?
            .validate_prepared_receipt(prepared, receipt)
    }
}

struct ProductiveContext<'a, P, A, S, T> {
    execution: ProductiveExecution<'a>,
    event_commit_failed: bool,
    provider: &'a mut P,
    attempts: &'a mut A,
    sink: &'a mut S,
    provider_attempts: u64,
    recovery: &'a mut dyn ProductiveRecovery,
    recovery_failed: bool,
    runtime_error_emitted: bool,
    tool_attempts: u64,
    tool_executor: &'a mut T,
}

fn mark_recovery_failure<T>(
    recovery_failed: &mut bool,
    result: Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    if result.is_err() {
        *recovery_failed = true;
    }
    result
}

fn emit_and_commit<S: crate::runtime::event_writer::RuntimeEventSink>(
    builder: &mut crate::runtime::event_construction::RuntimeEventBuilder,
    invocation: Option<&crate::runtime::stream_signature::FlowInvocation>,
    event_type: proto::EventType,
    payload: serde_json::Value,
    sink: &mut S,
    event_commit_failed: &mut bool,
) -> Result<(), RuntimeError> {
    builder.emit(invocation, event_type, payload)?;
    let crate::runtime::execution_plan::FlowExecutionAction::Event(action) = builder
        .actions
        .last()
        .expect("emitting an event records an action")
    else {
        unreachable!("emitting an event cannot record a fixture action")
    };
    let result = sink.commit(
        &action.event,
        &action.canonical_jsonl,
        action.context_checkpoint.clone(),
    );
    if result.is_err() {
        *event_commit_failed = true;
    }
    result
}

pub(crate) const MAX_MESSAGE_DELTA_UTF8_BYTES: usize = 32 * 1024;
pub(crate) const MAX_PROVIDER_MESSAGE_DELTA_CHUNKS: usize = MAX_RESPONSES_DECODED_STREAM_BYTES
    .div_ceil(MAX_MESSAGE_DELTA_UTF8_BYTES - (char::MAX_LEN_UTF8 - 1));

pub(crate) const PRODUCTIVE_CLOSURE_RECORDS: usize =
    3 * core_script::MAX_PHASE_NESTING_DEPTH + core_script::MAX_FLOW_NESTING_DEPTH + 5;
pub(crate) const PRODUCTIVE_METADATA_RESERVATION_BYTES: u64 = 512 * 1024;
pub(crate) const PROVIDER_EVENT_RESERVATION_BYTES: u64 =
    ((MAX_PROVIDER_MESSAGE_DELTA_CHUNKS + PRODUCTIVE_CLOSURE_RECORDS)
        * (MAX_CONVERSATION_RECORD_BYTES + 1)) as u64;
pub(crate) const TOOL_EVENT_RESERVATION_BYTES: u64 =
    ((PRODUCTIVE_CLOSURE_RECORDS + 2) * (MAX_CONVERSATION_RECORD_BYTES + 1)) as u64;
pub(crate) const PRODUCTIVE_CLOSURE_OBJECTS: usize =
    core_script::MAX_FLOW_NESTING_DEPTH + core_script::MAX_PHASE_NESTING_DEPTH + 1;
pub(crate) const PRODUCTIVE_CLOSURE_OBJECT_BYTES: u64 = MAX_SESSION_OBJECT_BYTES
    + ((core_script::MAX_FLOW_NESTING_DEPTH + core_script::MAX_PHASE_NESTING_DEPTH)
        * core_script::MAX_FLOW_VALUE_BYTES) as u64;

pub(crate) fn provider_dispatch_reservation(
    compiled: &crate::runtime::context::CompiledContext,
) -> ProductiveDispatchReservation {
    let context_bytes = u64::try_from(compiled.manifest.line.len()).unwrap_or(u64::MAX);
    let context_object_bytes = compiled.objects.iter().fold(0_u64, |total, object| {
        total.saturating_add(u64::try_from(object.bytes.len()).unwrap_or(u64::MAX))
    });
    ProductiveDispatchReservation {
        context_bytes,
        event_bytes: PROVIDER_EVENT_RESERVATION_BYTES,
        event_count: u64::try_from(MAX_PROVIDER_MESSAGE_DELTA_CHUNKS + PRODUCTIVE_CLOSURE_RECORDS)
            .unwrap_or(u64::MAX),
        event_record_bytes: u64::try_from(MAX_CONVERSATION_RECORD_BYTES + 1).unwrap_or(u64::MAX),
        metadata_bytes: PRODUCTIVE_METADATA_RESERVATION_BYTES,
        object_bytes: context_object_bytes
            .saturating_add(MAX_DURABLE_PROVIDER_OUTPUT_BYTES as u64)
            .saturating_add(PRODUCTIVE_CLOSURE_OBJECT_BYTES),
        object_count: compiled
            .objects
            .len()
            .saturating_add(
                MAX_DURABLE_PROVIDER_OUTPUT_BYTES.div_ceil(MAX_SESSION_OBJECT_BYTES as usize),
            )
            .saturating_add(PRODUCTIVE_CLOSURE_OBJECTS),
    }
}

pub(crate) fn tool_dispatch_reservation() -> ProductiveDispatchReservation {
    ProductiveDispatchReservation {
        context_bytes: 0,
        event_bytes: TOOL_EVENT_RESERVATION_BYTES,
        event_count: u64::try_from(PRODUCTIVE_CLOSURE_RECORDS + 2).unwrap_or(u64::MAX),
        event_record_bytes: u64::try_from(MAX_CONVERSATION_RECORD_BYTES + 1).unwrap_or(u64::MAX),
        metadata_bytes: PRODUCTIVE_METADATA_RESERVATION_BYTES,
        object_bytes: (2 * MAX_TOOL_STREAM_BYTES) as u64 + PRODUCTIVE_CLOSURE_OBJECT_BYTES,
        object_count: 2 + PRODUCTIVE_CLOSURE_OBJECTS,
    }
}

pub(crate) fn message_delta_chunks(mut content: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        if content.is_empty() {
            return None;
        }
        let mut end = content.len().min(MAX_MESSAGE_DELTA_UTF8_BYTES);
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        let chunk = &content[..end];
        content = &content[end..];
        Some(chunk)
    })
}

pub(crate) struct ProductiveExecution<'a> {
    pub(crate) clock: EventClock,
    pub(crate) conversation_id: &'a str,
    pub(crate) credential: &'a CredentialRecord,
    pub(crate) model: &'a str,
    pub(crate) model_profile: ContextModelProfile,
    pub(crate) policy: &'a core_policy::PolicyArtifact,
    pub(crate) prior_history: crate::runtime::context::ContextHistory,
    pub(crate) registry: &'a core_script::ResolvedRegistry,
    pub(crate) agent_instructions: &'a str,
    pub(crate) root_flow: &'a core_script::FlowBlock,
    pub(crate) root_input: Option<core_script::FlowValue>,
    pub(crate) session_id: &'a str,
    pub(crate) workspace: &'a AnchoredWorkspace,
}
