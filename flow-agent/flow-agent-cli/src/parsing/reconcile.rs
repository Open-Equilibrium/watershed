use super::validate_paired_session_ids;
use flow_agent_core::RuntimeError;

pub(super) const RECONCILE_TOOL_USAGE: &str =
    "flow reconcile-tool <conversation-id> <run-session-id> --result <file|->";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReconcileToolCommand {
    pub(crate) conversation_id: String,
    pub(crate) run_session_id: String,
    pub(crate) result: String,
}

pub(crate) fn reconcile_tool_args(args: &[String]) -> Result<ReconcileToolCommand, RuntimeError> {
    let [
        command,
        conversation_id,
        run_session_id,
        result_flag,
        result,
    ] = args
    else {
        return Err(usage_error());
    };
    if command != "reconcile-tool" || result_flag != "--result" {
        return Err(usage_error());
    }
    validate_paired_session_ids(conversation_id, run_session_id)?;
    Ok(ReconcileToolCommand {
        conversation_id: conversation_id.clone(),
        run_session_id: run_session_id.clone(),
        result: result.clone(),
    })
}

fn usage_error() -> RuntimeError {
    RuntimeError::Usage(format!("usage: {RECONCILE_TOOL_USAGE}"))
}

#[cfg(test)]
mod tests {
    use super::{ReconcileToolCommand, reconcile_tool_args};
    use crate::parsing::strings;

    #[test]
    fn reconcile_tool_has_one_attempt_free_grammar() {
        assert_eq!(
            reconcile_tool_args(&strings(&[
                "reconcile-tool",
                "review",
                "run-1",
                "--result",
                "result.json",
            ]))
            .expect("Tool reconciliation grammar is valid"),
            ReconcileToolCommand {
                conversation_id: "review".to_owned(),
                run_session_id: "run-1".to_owned(),
                result: "result.json".to_owned(),
            }
        );
        for args in [
            &[
                "reconcile-tool",
                "review",
                "run-1",
                "attempt-1",
                "--result",
                "result.json",
            ][..],
            &["reconcile-tool", "review", "run-1"][..],
            &[
                "reconcile-tool",
                "review",
                "run-1",
                "--result",
                "result.json",
                "extra",
            ][..],
        ] {
            assert!(
                reconcile_tool_args(&strings(args)).is_err(),
                "accepted {args:?}"
            );
        }
    }
}
