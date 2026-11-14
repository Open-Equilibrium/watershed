//! Conversions between the core's model and the types Dart receives.

use std::ffi::OsString;

use meta_harness_core::{
    AgentEvent, ChangeStatus, CommandError, Context, EntryKind, FileChange, ItemDetail, ItemKind,
    ItemStatus, MAX_RECORD_BYTES, Model, ModelUsage, NotificationLevel, Queue, RevertConfirmation,
    RevertFile, SessionEvent, Statistics, StoredSession, ToolCalls, Transcript, TranscriptItem,
    TreeNode, TurnCheckpoints, TurnStatistics, Usage, UserInput, visible_name,
};

use crate::api::events::{
    BridgeChangeStatus, BridgeDisplay, BridgeEvent, BridgeEventKind, BridgeFileChange, BridgeItem,
    BridgeItemKind, BridgeLevel, BridgeQuestion, BridgeQuestionKind, BridgeQueue, BridgeRequest,
    BridgeStep, BridgeUsage,
};
use crate::api::history::{
    BridgeContext, BridgeEntryKind, BridgeModelUsage, BridgeRevertConfirmation, BridgeRevertFile,
    BridgeStatistics, BridgeToolCalls, BridgeTranscript, BridgeTranscriptItem, BridgeTreeNode,
    BridgeTurn, BridgeTurnStatistics,
};
use crate::api::session::{BridgeModel, BridgeStoredSession};

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
        CommandError::ExtensionMissing => {
            "Watershed's pi extension is not loaded, so turns would run without checkpoints"
                .to_owned()
        }
        CommandError::SessionChanged => {
            "the session changed while its history was read; try again".to_owned()
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
                EntryKind::Custom => BridgeEntryKind::Custom,
                EntryKind::Other => BridgeEntryKind::Other,
            },
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

impl From<TurnCheckpoints> for BridgeTurn {
    fn from(turn: TurnCheckpoints) -> Self {
        let counted = |count: fn(&meta_harness_core::ChangeCount) -> u64| {
            turn.changed.as_ref().map(|changed| int(count(changed)))
        };
        Self {
            files: counted(|changed| changed.files),
            added: counted(|changed| changed.added),
            removed: counted(|changed| changed.removed),
            id: turn.turn,
            start_entry: turn.start_entry,
            end_entry: turn.end_entry,
            number: int(turn.number),
            prompt_entry: turn.prompt_entry,
            prompt: turn.prompt,
            started_ms: turn.started_ms.map(int),
            ended_ms: turn.ended_ms.map(int),
            ended: turn.ended,
            failure: turn.failure,
        }
    }
}

impl From<Statistics> for BridgeStatistics {
    fn from(statistics: Statistics) -> Self {
        Self {
            models: statistics.models.into_iter().map(Into::into).collect(),
            tools: statistics.tools.into_iter().map(Into::into).collect(),
            turns: statistics.turns.into_iter().map(Into::into).collect(),
            started_ms: statistics.started_ms.map(int),
            ended_ms: statistics.ended_ms.map(int),
        }
    }
}

impl From<ModelUsage> for BridgeModelUsage {
    fn from(model: ModelUsage) -> Self {
        Self {
            model: model.model,
            usage: model.usage.into(),
            responses: int(model.responses),
            unpriced: int(model.unpriced),
        }
    }
}

impl From<ToolCalls> for BridgeToolCalls {
    fn from(tool: ToolCalls) -> Self {
        Self {
            name: tool.name,
            succeeded: int(tool.succeeded),
            failed: int(tool.failed),
            cancelled: int(tool.cancelled),
            open: int(tool.open),
        }
    }
}

impl From<TurnStatistics> for BridgeTurnStatistics {
    fn from(turn: TurnStatistics) -> Self {
        Self {
            entry: turn.entry,
            prompt: turn.prompt,
            started_ms: turn.started_ms.map(int),
            ended_ms: turn.ended_ms.map(int),
            models: turn.models.into_iter().map(Into::into).collect(),
            model_ms: turn.model_ms.map(int),
            tool_ms: turn.tool_ms.map(int),
            tool_calls: int(turn.tool_calls),
            failed: int(turn.failed),
        }
    }
}

impl From<Context> for BridgeContext {
    fn from(context: Context) -> Self {
        Self {
            tokens: context.tokens.map(int),
            window: int(context.window),
            compacts_at: context.compacts_at.map(int),
        }
    }
}

impl From<ChangeStatus> for BridgeChangeStatus {
    fn from(status: ChangeStatus) -> Self {
        match status {
            ChangeStatus::Added => Self::Added,
            ChangeStatus::Modified => Self::Modified,
            ChangeStatus::Deleted => Self::Deleted,
        }
    }
}

impl From<FileChange> for BridgeFileChange {
    fn from(change: FileChange) -> Self {
        Self {
            shown: visible_name(&change.path),
            path: change.path,
            status: change.status.into(),
            added: int(change.added),
            removed: int(change.removed),
        }
    }
}

impl From<RevertConfirmation> for BridgeRevertConfirmation {
    fn from(value: RevertConfirmation) -> Self {
        Self {
            target: value.target,
            checkout: value.checkout,
            branch: value.branch,
            kept: value.kept,
            ignored: value.ignored,
            ignored_state: value.ignored_state,
        }
    }
}

impl From<BridgeRevertConfirmation> for RevertConfirmation {
    fn from(value: BridgeRevertConfirmation) -> Self {
        Self {
            target: value.target,
            checkout: value.checkout,
            branch: value.branch,
            kept: value.kept,
            ignored: value.ignored,
            ignored_state: value.ignored_state,
        }
    }
}

impl From<RevertFile> for BridgeRevertFile {
    fn from(file: RevertFile) -> Self {
        Self {
            shown: visible_name(&file.path),
            path: file.path,
            status: file.status.into(),
            edited: file.edited,
            modified_ms: file.modified_ms.map(int),
            ignored: file.ignored,
            kept: file.kept,
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

pub(crate) fn event(kind: BridgeEventKind) -> BridgeEvent {
    BridgeEvent {
        kind,
        item: None,
        step: None,
        request: None,
        text: None,
        display: None,
        success: None,
        queue: None,
        checkpoint: None,
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
    BridgeRequest { id, question: None }
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
            AgentEvent::TurnCompleted { checkpoint, .. } => BridgeEvent {
                checkpoint,
                ..event(Kind::TurnCompleted)
            },
            AgentEvent::TurnAborted { checkpoint, .. } => BridgeEvent {
                checkpoint,
                ..event(Kind::TurnAborted)
            },
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

impl TryFrom<StoredSession> for BridgeStoredSession {
    /// The path, when it is not UTF-8 and so cannot name the session to reopen.
    type Error = OsString;

    fn try_from(stored: StoredSession) -> Result<Self, Self::Error> {
        Ok(Self {
            path: stored.path.into_os_string().into_string()?,
            title: stored.title,
            modified_ms: int(stored.modified_ms),
        })
    }
}

impl From<Model> for BridgeModel {
    fn from(model: Model) -> Self {
        Self {
            provider: model.provider,
            id: model.id,
            name: model.name,
            context_window: model.context_window.map(int),
        }
    }
}
