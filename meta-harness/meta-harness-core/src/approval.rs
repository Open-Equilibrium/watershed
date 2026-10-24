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
