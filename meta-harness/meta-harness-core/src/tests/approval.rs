use crate::RequestKind::{CommandExecutionApproval, FileChangeApproval, ToolApproval};
use crate::approval::{ApprovalMode, needs_user};

#[test]
fn modes_decide_which_calls_wait_for_the_user() {
    use ApprovalMode::{AllowEdits, Ask, FullAccess};
    // (mode, kind, read_only, allowed_for_session, needs_user)
    let cases = [
        (Ask, CommandExecutionApproval, false, false, true),
        (Ask, FileChangeApproval, false, false, true),
        (Ask, ToolApproval, true, false, false),
        (Ask, CommandExecutionApproval, false, true, false),
        (AllowEdits, FileChangeApproval, false, false, false),
        (AllowEdits, CommandExecutionApproval, false, false, true),
        (AllowEdits, ToolApproval, false, false, true),
        (FullAccess, CommandExecutionApproval, false, false, false),
    ];
    for (mode, kind, read_only, session, expected) in cases {
        assert_eq!(
            needs_user(mode, kind, read_only, session),
            expected,
            "{mode:?} {kind:?} read_only={read_only} session={session}"
        );
    }
    assert_eq!(ApprovalMode::default(), Ask);
}
