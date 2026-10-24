//! Conversions between the core's model and the types Dart receives.

use meta_harness_core::{
    ApprovalMode, CommandError, Decision, ItemDetail, ItemKind, ItemStatus, NotificationLevel,
    RequestKind, RuntimeEvent, SessionEvent, UserInput,
};

use crate::api::events::{
    BridgeApproval, BridgeContext, BridgeDecision, BridgeDisplay, BridgeEvent, BridgeEventKind,
    BridgeItem, BridgeItemKind, BridgeLevel, BridgeQuestion, BridgeQuestionKind, BridgeRequest,
    BridgeRequestKind, BridgeSection, BridgeStep, BridgeUsage,
};
use crate::api::session::BridgeApprovalMode;

/// Describes a failed command for Dart.
pub(crate) fn describe(error: CommandError) -> String {
    match error {
        CommandError::Rejected(reason) => format!("the agent rejected the command: {reason}"),
        CommandError::UnknownRequest => "the request is no longer open".to_owned(),
        CommandError::Closed => "the agent has exited".to_owned(),
    }
}

fn int(value: impl TryInto<i64>) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
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
                    usage: BridgeUsage {
                        input: int(usage.input),
                        output: int(usage.output),
                        cache_read: int(usage.cache_read),
                        cache_write: int(usage.cache_write),
                        reasoning: int(usage.reasoning),
                        cost_micro_usd: int(usage.cost_micro_usd),
                    },
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
        }
    }
}
