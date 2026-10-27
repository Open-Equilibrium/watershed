//! Conversions between the core's model and the types Dart receives.

use meta_harness_core::{
    AgentEvent, ApprovalMode, ChangeStatus, CommandError, ContextDetail, ContextMessage,
    ContextPart, Decision, EntryKind, FileChange, ItemDetail, ItemKind, ItemStatus,
    MAX_RECORD_BYTES, Model, NotificationLevel, Queue, RecordKind, RequestKind, SessionEvent,
    StepContext, StoredSession, ToolOutcome, Trajectory, TrajectoryRecord, Transcript,
    TranscriptItem, TreeNode, Usage, UserInput, visible_name,
};

use crate::api::events::{
    BridgeApproval, BridgeChangeStatus, BridgeDecision, BridgeDisplay, BridgeEvent,
    BridgeEventKind, BridgeFileChange, BridgeItem, BridgeItemKind, BridgeLevel, BridgeQuestion,
    BridgeQuestionKind, BridgeQueue, BridgeRequest, BridgeRequestKind, BridgeStep, BridgeUsage,
};
use crate::api::history::{
    BridgeContextDetail, BridgeContextMessage, BridgeContextPart, BridgeEntryKind, BridgeRecord,
    BridgeRecordKind, BridgeStepContext, BridgeToolOutcome, BridgeTrajectory, BridgeTranscript,
    BridgeTranscriptItem, BridgeTreeNode,
};
use crate::api::session::{BridgeApprovalMode, BridgeModel, BridgeStoredSession};

/// Merges consecutive content deltas of the same item that piled up while the forwarder was
/// busy, so fewer events cross to Dart. flutter_rust_bridge's sink does not wait for Dart, so a
/// stalled Dart isolate still queues what was sent; flow control that coalesces while the UI
/// falls behind (client architecture rule 4) is not built for the agent stream.
pub(crate) fn coalesce(events: Vec<BridgeEvent>) -> Vec<BridgeEvent> {
    let mut merged: Vec<BridgeEvent> = Vec::with_capacity(events.len());
    for event in events {
        if let Some(last) = merged.last_mut()
            && last.kind == BridgeEventKind::ContentDelta
            && event.kind == BridgeEventKind::ContentDelta
            && last.item.as_ref().map(|item| item.id) == event.item.as_ref().map(|item| item.id)
            && let (Some(text), Some(more)) = (last.text.as_mut(), event.text.as_deref())
        {
            text.push_str(more);
            continue;
        }
        merged.push(event);
    }
    merged
}

/// Describes a failed command for Dart.
pub(crate) fn describe(error: CommandError) -> String {
    match error {
        CommandError::Rejected(reason) => format!("the agent rejected the command: {reason}"),
        CommandError::UnknownRequest => "the request is no longer open".to_owned(),
        CommandError::Closed => "the agent has exited".to_owned(),
        CommandError::TooLarge => format!(
            "the agent's answer exceeded {} MiB and was skipped",
            MAX_RECORD_BYTES / (1024 * 1024)
        ),
        CommandError::Ungated => {
            "Watershed's pi extension is not loaded, so tool calls would run without asking"
                .to_owned()
        }
    }
}

fn int(value: impl TryInto<i64>) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
}

impl From<Usage> for BridgeUsage {
    fn from(usage: Usage) -> Self {
        Self {
            input: int(usage.input),
            output: int(usage.output),
            cache_read: int(usage.cache_read),
            cache_write: int(usage.cache_write),
            reasoning: int(usage.reasoning),
            cost_micro_usd: int(usage.cost_micro_usd),
        }
    }
}

impl From<TreeNode> for BridgeTreeNode {
    fn from(node: TreeNode) -> Self {
        Self {
            id: node.id,
            parent: node.parent,
            kind: match node.kind {
                EntryKind::User => BridgeEntryKind::User,
                EntryKind::Assistant => BridgeEntryKind::Assistant,
                EntryKind::ToolResult => BridgeEntryKind::ToolResult,
                EntryKind::System => BridgeEntryKind::System,
                EntryKind::Approval => BridgeEntryKind::Approval,
                EntryKind::Custom => BridgeEntryKind::Custom,
                EntryKind::Other => BridgeEntryKind::Other,
            },
            preview: node.preview,
            label: node.label,
            at_ms: node.at_ms.map(int),
        }
    }
}

impl From<Trajectory> for BridgeTrajectory {
    fn from(trajectory: Trajectory) -> Self {
        Self {
            records: trajectory.records.into_iter().map(Into::into).collect(),
            duration_ms: int(trajectory.duration_ms),
        }
    }
}

impl From<TrajectoryRecord> for BridgeRecord {
    fn from(record: TrajectoryRecord) -> Self {
        Self {
            entry: record.entry,
            turn: int(record.turn),
            kind: match record.kind {
                RecordKind::User => BridgeRecordKind::User,
                RecordKind::Message => BridgeRecordKind::Message,
                RecordKind::Tool => BridgeRecordKind::Tool,
                RecordKind::Approval => BridgeRecordKind::Approval,
            },
            label: record.label,
            detail: record.detail,
            target: record.target,
            changed_file: record.changed_file,
            decision: record.decision.map(Into::into),
            start_ms: int(record.start_ms),
            duration_ms: record.duration_ms.map(int),
            first_token_ms: record.first_token_ms.map(int),
            model: record.model,
            usage: record.usage.map(Into::into),
            stop_reason: record.stop_reason,
            failed: record.failed,
            context: record.context.map(Into::into),
            outcome: record.outcome.into(),
        }
    }
}

impl From<Queue> for BridgeQueue {
    fn from(queue: Queue) -> Self {
        Self {
            steering: queue.steering,
            follow_up: queue.follow_up,
        }
    }
}

impl From<Transcript> for BridgeTranscript {
    fn from(transcript: Transcript) -> Self {
        Self {
            items: transcript.items.into_iter().map(Into::into).collect(),
            more: transcript.more,
            start: transcript.start,
        }
    }
}

impl From<TranscriptItem> for BridgeTranscriptItem {
    fn from(item: TranscriptItem) -> Self {
        Self {
            entry: item.entry,
            kind: item.kind.into(),
            text: item.text,
            tool: item.tool,
            target: item.target,
            failed: item.failed,
            duration_ms: item.duration_ms.map(int),
            added: item.lines.map(|lines| int(lines.added)),
            removed: item.lines.map(|lines| int(lines.removed)),
        }
    }
}

impl From<ToolOutcome> for BridgeToolOutcome {
    fn from(outcome: ToolOutcome) -> Self {
        Self {
            added: outcome.added.map(int),
            removed: outcome.removed.map(int),
            exit_code: outcome.exit_code,
            tests_passed: outcome.tests.map(|tests| int(tests.passed)),
            tests_failed: outcome.tests.map(|tests| int(tests.failed)),
        }
    }
}

impl From<ContextDetail> for BridgeContextDetail {
    fn from(detail: ContextDetail) -> Self {
        let parts = |parts: Vec<ContextPart>| {
            parts
                .into_iter()
                .map(|part| BridgeContextPart {
                    name: part.name,
                    chars: int(part.chars),
                })
                .collect()
        };
        Self {
            sections: parts(detail.sections),
            tools: parts(detail.tools),
            messages: detail.messages.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ContextMessage> for BridgeContextMessage {
    fn from(message: ContextMessage) -> Self {
        Self {
            entry: message.entry,
            label: message.label,
            chars: int(message.chars),
            in_context: message.in_context,
            fresh: message.fresh,
        }
    }
}

impl From<StepContext> for BridgeStepContext {
    fn from(context: StepContext) -> Self {
        Self {
            prompt_chars: int(context.prompt_chars),
            tools: context.tools,
            tool_chars: int(context.tool_chars),
            messages: int(context.messages),
            message_chars: int(context.message_chars),
            new_since_previous: context.new_since_previous,
        }
    }
}

impl From<FileChange> for BridgeFileChange {
    fn from(change: FileChange) -> Self {
        Self {
            shown: visible_name(&change.path),
            path: change.path,
            status: match change.status {
                ChangeStatus::Added => BridgeChangeStatus::Added,
                ChangeStatus::Modified => BridgeChangeStatus::Modified,
                ChangeStatus::Deleted => BridgeChangeStatus::Deleted,
            },
            added: int(change.added),
            removed: int(change.removed),
        }
    }
}

impl From<BridgeApprovalMode> for ApprovalMode {
    fn from(mode: BridgeApprovalMode) -> Self {
        match mode {
            BridgeApprovalMode::Ask => Self::Ask,
            BridgeApprovalMode::AllowEdits => Self::AllowEdits,
            BridgeApprovalMode::FullAccess => Self::FullAccess,
        }
    }
}

impl From<BridgeDecision> for Decision {
    fn from(decision: BridgeDecision) -> Self {
        match decision {
            BridgeDecision::AllowOnce => Self::AllowOnce,
            BridgeDecision::AllowSession => Self::AllowSession,
            BridgeDecision::Reject => Self::Reject,
        }
    }
}

impl From<Decision> for BridgeDecision {
    fn from(decision: Decision) -> Self {
        match decision {
            Decision::AllowOnce => Self::AllowOnce,
            Decision::AllowSession => Self::AllowSession,
            Decision::Reject => Self::Reject,
        }
    }
}

impl From<ItemKind> for BridgeItemKind {
    fn from(kind: ItemKind) -> Self {
        match kind {
            ItemKind::UserMessage => Self::UserMessage,
            ItemKind::AssistantMessage => Self::AssistantMessage,
            ItemKind::Reasoning => Self::Reasoning,
            ItemKind::CommandExecution => Self::CommandExecution,
            ItemKind::FileChange => Self::FileChange,
            ItemKind::DynamicToolCall => Self::DynamicToolCall,
        }
    }
}

impl From<RequestKind> for BridgeRequestKind {
    fn from(kind: RequestKind) -> Self {
        match kind {
            RequestKind::CommandExecutionApproval => Self::CommandExecution,
            RequestKind::FileChangeApproval => Self::FileChange,
            RequestKind::ToolApproval => Self::Tool,
        }
    }
}

fn event(kind: BridgeEventKind) -> BridgeEvent {
    BridgeEvent {
        kind,
        item: None,
        step: None,
        request: None,
        text: None,
        display: None,
        success: None,
        queue: None,
    }
}

fn item(id: u64) -> BridgeItem {
    BridgeItem {
        id: int(id),
        kind: None,
        tool: None,
        target: None,
        failed: false,
        duration_ms: None,
        added: None,
        removed: None,
    }
}

fn request(id: String) -> BridgeRequest {
    BridgeRequest {
        id,
        approval: None,
        question: None,
    }
}

fn display(key: String) -> BridgeDisplay {
    BridgeDisplay {
        key,
        level: None,
        lines: None,
    }
}

impl From<SessionEvent> for BridgeEvent {
    fn from(session_event: SessionEvent) -> Self {
        match session_event {
            SessionEvent::Agent(event) => event.into(),
            SessionEvent::Failed(reason) => BridgeEvent {
                text: Some(reason),
                ..event(BridgeEventKind::Failed)
            },
            SessionEvent::Exited {
                success,
                error_output,
            } => BridgeEvent {
                success: Some(success),
                text: (!error_output.is_empty()).then_some(error_output),
                ..event(BridgeEventKind::Exited)
            },
        }
    }
}

impl From<AgentEvent> for BridgeEvent {
    fn from(agent: AgentEvent) -> Self {
        use BridgeEventKind as Kind;
        match agent {
            AgentEvent::TurnStarted { .. } => event(Kind::TurnStarted),
            AgentEvent::TurnCompleted { .. } => event(Kind::TurnCompleted),
            AgentEvent::TurnAborted { .. } => event(Kind::TurnAborted),
            AgentEvent::ItemStarted {
                item: id,
                kind,
                detail,
                ..
            } => {
                let (tool, target) = match detail {
                    ItemDetail::Tool { name, target, .. } => (Some(name), Some(target)),
                    ItemDetail::None => (None, None),
                };
                BridgeEvent {
                    item: Some(BridgeItem {
                        kind: Some(kind.into()),
                        tool,
                        target,
                        ..item(id.0)
                    }),
                    ..event(Kind::ItemStarted)
                }
            }
            AgentEvent::ItemUpdated { item: id, output } => BridgeEvent {
                item: Some(item(id.0)),
                text: Some(output),
                ..event(Kind::ItemUpdated)
            },
            AgentEvent::ContentDelta { item: id, delta } => BridgeEvent {
                item: Some(item(id.0)),
                text: Some(delta),
                ..event(Kind::ContentDelta)
            },
            AgentEvent::ItemCompleted {
                item: id,
                kind,
                text,
                status,
                duration_ms,
                lines,
                ..
            } => BridgeEvent {
                item: Some(BridgeItem {
                    kind: Some(kind.into()),
                    failed: status == ItemStatus::Failed,
                    duration_ms: duration_ms.map(int),
                    added: lines.map(|lines| int(lines.added)),
                    removed: lines.map(|lines| int(lines.removed)),
                    ..item(id.0)
                }),
                text: Some(text),
                ..event(Kind::ItemCompleted)
            },
            AgentEvent::ModelRequestCompleted {
                usage,
                stop_reason,
                error,
                ..
            } => BridgeEvent {
                step: Some(BridgeStep {
                    usage: usage.into(),
                    stop_reason,
                    error,
                }),
                ..event(Kind::ModelRequestCompleted)
            },
            AgentEvent::RequestOpened {
                request: id,
                kind,
                tool,
                text,
                item,
                ..
            } => BridgeEvent {
                request: Some(BridgeRequest {
                    approval: Some(BridgeApproval {
                        kind: kind.into(),
                        tool,
                        text,
                        item: item.map(|item| int(item.0)),
                    }),
                    ..request(id.0)
                }),
                ..event(Kind::RequestOpened)
            },
            AgentEvent::RequestResolved { request: id, .. } => BridgeEvent {
                request: Some(request(id.0)),
                ..event(Kind::RequestResolved)
            },
            AgentEvent::UserInputRequested { request: id, input } => {
                let (kind, title, detail, options) = match input {
                    UserInput::Select { title, options } => {
                        (BridgeQuestionKind::Select, title, String::new(), options)
                    }
                    UserInput::Confirm { title, message } => {
                        (BridgeQuestionKind::Confirm, title, message, Vec::new())
                    }
                    UserInput::Input { title, placeholder } => {
                        (BridgeQuestionKind::Input, title, placeholder, Vec::new())
                    }
                    UserInput::Editor { title, prefill } => {
                        (BridgeQuestionKind::Editor, title, prefill, Vec::new())
                    }
                };
                BridgeEvent {
                    request: Some(BridgeRequest {
                        question: Some(BridgeQuestion {
                            kind,
                            title,
                            detail,
                            options,
                        }),
                        ..request(id.0)
                    }),
                    ..event(Kind::UserInputRequested)
                }
            }
            AgentEvent::UserInputResolved { request: id } => BridgeEvent {
                request: Some(request(id.0)),
                ..event(Kind::UserInputResolved)
            },
            AgentEvent::Notification { level, message } => BridgeEvent {
                display: Some(BridgeDisplay {
                    level: Some(match level {
                        NotificationLevel::Info => BridgeLevel::Info,
                        NotificationLevel::Warning => BridgeLevel::Warning,
                        NotificationLevel::Error => BridgeLevel::Error,
                    }),
                    ..display(String::new())
                }),
                text: Some(message),
                ..event(Kind::Notification)
            },
            AgentEvent::StatusChanged { key, text } => BridgeEvent {
                display: Some(display(key)),
                text,
                ..event(Kind::StatusChanged)
            },
            AgentEvent::WidgetChanged { key, lines } => BridgeEvent {
                display: Some(BridgeDisplay {
                    lines,
                    ..display(key)
                }),
                ..event(Kind::WidgetChanged)
            },
            AgentEvent::TitleChanged { title } => BridgeEvent {
                text: Some(title),
                ..event(Kind::TitleChanged)
            },
            AgentEvent::InputTextSet { text } => BridgeEvent {
                text: Some(text),
                ..event(Kind::InputTextSet)
            },
            AgentEvent::QueueChanged(queue) => BridgeEvent {
                queue: Some(queue.into()),
                ..event(Kind::QueueChanged)
            },
        }
    }
}

impl From<StoredSession> for BridgeStoredSession {
    fn from(stored: StoredSession) -> Self {
        Self {
            path: stored.path.display().to_string(),
            title: stored.title,
            modified_ms: int(stored.modified_ms),
        }
    }
}

impl From<Model> for BridgeModel {
    fn from(model: Model) -> Self {
        Self {
            provider: model.provider,
            id: model.id,
            name: model.name,
        }
    }
}
