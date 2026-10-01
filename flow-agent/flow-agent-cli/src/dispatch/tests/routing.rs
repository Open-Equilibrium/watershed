use super::super::dispatch_in_workspace;
use crate::interrupt::InterruptCoordinator;
use flow_agent_core::RuntimeError;
use std::path::Path;

#[test]
fn usage_errors_precede_workspace_access() {
    let cases: &[&[&str]] = &[
        &[],
        &["unknown"],
        &["init", "--registry-root"],
        &["validate", "--unknown"],
        &["create"],
        &["create", "connection"],
        &["create", "tool", "--id"],
        &[
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
        ],
        &[
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
        ],
        &[
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
        ],
        &[
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
        ],
        &["sessions", "--bogus"],
        &["replay", "INVALID", "run"],
        &["replay", "conversation", "INVALID"],
        &["tail", "INVALID", "run"],
        &["tail", "conversation", "INVALID"],
    ];
    for case in cases {
        let args = case.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
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
