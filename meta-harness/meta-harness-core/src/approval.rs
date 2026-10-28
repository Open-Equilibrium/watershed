//! Which tool calls need the user's approval (ADR-0188).

use crate::RequestKind;

/// How much an agent may do without asking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ApprovalMode {
    /// Ask for every call that is not read-only.
    #[default]
    Ask,
    /// Also allow file changes without asking.
    AllowEdits,
    /// Allow every call.
    FullAccess,
}

/// What a session-wide allowance covers: the exact command shown for shell tools, whose calls are
/// the most dangerous, and the whole tool otherwise (ADR-0209). The kind keeps pi's own tool
/// apart from an extension's tool of the same name, which asks as a tool call (ADR-0208).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Allowance {
    Command { tool: String, command: String },
    Tool { tool: String, kind: RequestKind },
}

impl Allowance {
    /// The allowance a call of `tool` that asks as `kind` would use; `text` is what its card
    /// shows.
    pub(crate) fn of(kind: RequestKind, tool: &str, text: &str) -> Self {
        match kind {
            RequestKind::CommandExecutionApproval => Self::Command {
                tool: tool.to_owned(),
                command: text.to_owned(),
            },
            RequestKind::FileChangeApproval | RequestKind::ToolApproval => Self::Tool {
                tool: tool.to_owned(),
                kind,
            },
        }
    }
}

/// Whether a call must wait for the user. Approvals are a control, not a protection boundary.
pub(crate) fn needs_user(
    mode: ApprovalMode,
    kind: RequestKind,
    read_only: bool,
    allowed_for_session: bool,
) -> bool {
    let allowed_by_mode = match mode {
        ApprovalMode::Ask => false,
        ApprovalMode::AllowEdits => kind == RequestKind::FileChangeApproval,
        ApprovalMode::FullAccess => true,
    };
    !(read_only || allowed_for_session || allowed_by_mode)
}
