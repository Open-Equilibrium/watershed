use super::super::dispatch_in_workspace;
use crate::interrupt::InterruptCoordinator;
use flow_agent_core::RuntimeError;
use std::path::Path;

#[test]
fn usage_errors_precede_workspace_access() {
    for args in [
        Vec::<String>::new(),
        vec!["unknown".to_owned()],
        vec!["init".to_owned(), "--registry-root".to_owned()],
        vec!["validate".to_owned(), "--unknown".to_owned()],
        vec!["create".to_owned()],
        vec!["create".to_owned(), "connection".to_owned()],
        vec!["create".to_owned(), "tool".to_owned(), "--id".to_owned()],
        [
            "create",
            "instruction",
            "--id",
            "review",
            "--name",
            "Review",
            "--prompt-file",
            "missing-prompt.txt",
            "--parameter",
            "--parameter-name",
            "project",
            "--parameter-contract-file",
            "missing-contract.yaml",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        [
            "create",
            "instruction",
            "--id",
            "review",
            "--name",
            "Review",
            "--parameter",
            "--parameter-name",
            "project",
            "--parameter-contract-file",
            "missing-contract.yaml",
            "--end-parameter",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        [
            "create",
            "phase",
            "--id",
            "review",
            "--name",
            "Review",
            "--output-contract-file",
            "missing-contract.yaml",
            "--loop",
            "--loop-max-iterations",
            "1",
            "--loop-until-file",
            "missing-until.yaml",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        [
            "create",
            "flow",
            "--id",
            "review",
            "--name",
            "Review",
            "--phase-ref",
            "review-phase",
            "--transition",
            "--transition-from-phase-ref",
            "review-phase",
            "--transition-to-phase-ref",
            "publish-phase",
            "--transition-when-file",
            "missing-when.yaml",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        vec!["sessions".to_owned(), "--bogus".to_owned()],
        vec!["replay".to_owned(), "INVALID".to_owned(), "run".to_owned()],
        vec![
            "replay".to_owned(),
            "conversation".to_owned(),
            "INVALID".to_owned(),
        ],
        vec!["tail".to_owned(), "INVALID".to_owned(), "run".to_owned()],
        vec![
            "tail".to_owned(),
            "conversation".to_owned(),
            "INVALID".to_owned(),
        ],
    ] {
        let interrupts = InterruptCoordinator::new();
        let error = dispatch_in_workspace(
            &args,
            &interrupts,
            Path::new("missing-usage-validation-workspace"),
        )
        .expect_err("invalid grammar is a usage error before workspace access");

        assert!(matches!(error, RuntimeError::Usage(_)));
    }
}
