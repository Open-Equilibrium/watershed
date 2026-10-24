//! Conversions between the core's model and the types Dart receives.

use meta_harness_core::{
    ApprovalMode, ChangeStatus, CommandError, ContextDetail, ContextMessage, ContextPart, Decision,
    EntryKind, FileChange, ItemDetail, ItemKind, ItemStatus, NotificationLevel, Queue, RecordKind,
    RequestKind, RuntimeEvent, SessionEvent, StepContext, ToolOutcome, Trajectory,
    TrajectoryRecord, Transcript, TranscriptItem, TreeNode, Usage, UserInput,
};

use crate::api::events::{
    BridgeApproval, BridgeChangeStatus, BridgeContext, BridgeDecision, BridgeDisplay, BridgeEvent,
    BridgeEventKind, BridgeFileChange, BridgeItem, BridgeItemKind, BridgeLevel, BridgeQuestion,
    BridgeQuestionKind, BridgeQueue, BridgeRequest, BridgeRequestKind, BridgeSection, BridgeStep,
    BridgeUsage,
};
use crate::api::history::{
    BridgeContextDetail, BridgeContextMessage, BridgeContextPart, BridgeEntryKind, BridgeRecord,
    BridgeRecordKind, BridgeStepContext, BridgeToolOutcome, BridgeTrajectory, BridgeTranscript,
    BridgeTranscriptItem, BridgeTreeNode,
};
use crate::api::session::BridgeApprovalMode;

/// Merges consecutive content deltas of the same item, so a lagging UI receives the latest text
/// in fewer events instead of every intermediate one (client architecture rule 4).
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
        CommandError::TooLarge => "the agent's answer exceeded 16 MiB and was skipped".to_owned(),
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
                EntryKind::Custom(_) => BridgeEntryKind::Custom,
                EntryKind::Other(_) => BridgeEntryKind::Other,
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
            arguments: item.arguments,
            failed: item.failed,
            duration_ms: item.duration_ms.map(int),
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
        turn: None,
        item: None,
        step: None,
        request: None,
        text: None,
        display: None,
        context: None,
        success: None,
        queue: None,
    }
}

fn item(id: u64) -> BridgeItem {
    BridgeItem {
        id: int(id),
        kind: None,
        tool: None,
        arguments: None,
        failed: false,
        duration_ms: None,
    }
}

fn request(id: String) -> BridgeRequest {
    BridgeRequest {
        id,
        approval: None,
        question: None,
        decision: None,
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
            SessionEvent::Runtime(runtime) => runtime.into(),
            SessionEvent::Failed(reason) => BridgeEvent {
                text: Some(reason),
                ..event(BridgeEventKind::Failed)
            },
            SessionEvent::Exited { success } => BridgeEvent {
                success: Some(success),
                ..event(BridgeEventKind::Exited)
            },
        }
    }
}

impl From<RuntimeEvent> for BridgeEvent {
    fn from(runtime: RuntimeEvent) -> Self {
        use BridgeEventKind as Kind;
        match runtime {
            RuntimeEvent::TurnStarted { turn } => BridgeEvent {
                turn: Some(int(turn.0)),
                ..event(Kind::TurnStarted)
            },
            RuntimeEvent::TurnCompleted { turn } => BridgeEvent {
                turn: Some(int(turn.0)),
                ..event(Kind::TurnCompleted)
            },
            RuntimeEvent::TurnAborted { turn } => BridgeEvent {
                turn: Some(int(turn.0)),
                ..event(Kind::TurnAborted)
            },
            RuntimeEvent::ItemStarted {
                turn,
                item: id,
                kind,
                detail,
            } => {
                let (tool, arguments) = match detail {
                    ItemDetail::Tool { name, arguments } => (Some(name), Some(arguments)),
                    ItemDetail::None => (None, None),
                };
                BridgeEvent {
                    turn: Some(int(turn.0)),
                    item: Some(BridgeItem {
                        kind: Some(kind.into()),
                        tool,
                        arguments,
                        ..item(id.0)
                    }),
                    ..event(Kind::ItemStarted)
                }
            }
            RuntimeEvent::ItemUpdated { item: id, output } => BridgeEvent {
                item: Some(item(id.0)),
                text: Some(output),
                ..event(Kind::ItemUpdated)
            },
            RuntimeEvent::ContentDelta { item: id, delta } => BridgeEvent {
                item: Some(item(id.0)),
                text: Some(delta),
                ..event(Kind::ContentDelta)
            },
            RuntimeEvent::ItemCompleted {
                turn,
                item: id,
                kind,
                text,
                status,
                duration_ms,
            } => BridgeEvent {
                turn: Some(int(turn.0)),
                item: Some(BridgeItem {
                    kind: Some(kind.into()),
                    failed: status == ItemStatus::Failed,
                    duration_ms: duration_ms.map(int),
                    ..item(id.0)
                }),
                text: Some(text),
                ..event(Kind::ItemCompleted)
            },
            RuntimeEvent::ContextChanged {
                turn,
                sections,
                appended_chars,
                tools_added,
                tools_removed,
            } => BridgeEvent {
                turn: Some(int(turn.0)),
                context: Some(BridgeContext {
                    sections: sections
                        .into_iter()
                        .map(|section| BridgeSection {
                            name: section.name,
                            chars: section.chars.map(int),
                        })
                        .collect(),
                    appended_chars: int(appended_chars),
                    tools_added,
                    tools_removed,
                }),
                ..event(Kind::ContextChanged)
            },
            RuntimeEvent::ModelRequestStarted { turn, step } => BridgeEvent {
                turn: Some(int(turn.0)),
                step: Some(BridgeStep {
                    id: int(step.0),
                    model: String::new(),
                    usage: BridgeUsage::default(),
                    stop_reason: String::new(),
                    error: None,
                    first_token_ms: None,
                    duration_ms: 0,
                }),
                ..event(Kind::ModelRequestStarted)
            },
            RuntimeEvent::ModelRequestCompleted {
                turn,
                step,
                model,
                usage,
                stop_reason,
                error,
                first_token_ms,
                duration_ms,
            } => BridgeEvent {
                turn: Some(int(turn.0)),
                step: Some(BridgeStep {
                    id: int(step.0),
                    model,
                    usage: usage.into(),
                    stop_reason,
                    error,
                    first_token_ms: first_token_ms.map(int),
                    duration_ms: int(duration_ms),
                }),
                ..event(Kind::ModelRequestCompleted)
            },
            RuntimeEvent::RequestOpened {
                request: id,
                kind,
                tool,
                arguments,
                read_only,
            } => BridgeEvent {
                request: Some(BridgeRequest {
                    approval: Some(BridgeApproval {
                        kind: kind.into(),
                        tool,
                        arguments,
                        read_only,
                    }),
                    ..request(id.0)
                }),
                ..event(Kind::RequestOpened)
            },
            RuntimeEvent::RequestResolved {
                request: id,
                decision,
            } => BridgeEvent {
                request: Some(BridgeRequest {
                    decision: Some(decision.into()),
                    ..request(id.0)
                }),
                ..event(Kind::RequestResolved)
            },
            RuntimeEvent::UserInputRequested { request: id, input } => {
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
            RuntimeEvent::UserInputResolved { request: id } => BridgeEvent {
                request: Some(request(id.0)),
                ..event(Kind::UserInputResolved)
            },
            RuntimeEvent::Notification { level, message } => BridgeEvent {
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
            RuntimeEvent::StatusChanged { key, text } => BridgeEvent {
                display: Some(display(key)),
                text,
                ..event(Kind::StatusChanged)
            },
            RuntimeEvent::WidgetChanged { key, lines } => BridgeEvent {
                display: Some(BridgeDisplay {
                    lines,
                    ..display(key)
                }),
                ..event(Kind::WidgetChanged)
            },
            RuntimeEvent::TitleChanged { title } => BridgeEvent {
                text: Some(title),
                ..event(Kind::TitleChanged)
            },
            RuntimeEvent::InputTextSet { text } => BridgeEvent {
                text: Some(text),
                ..event(Kind::InputTextSet)
            },
            RuntimeEvent::QueueChanged(queue) => BridgeEvent {
                queue: Some(queue.into()),
                ..event(Kind::QueueChanged)
            },
        }
    }
}
