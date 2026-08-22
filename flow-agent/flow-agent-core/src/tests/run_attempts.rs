use crate::runtime::run_attempts::{ProviderTerminalClassification, ToolTerminalClassification};
#[test]
fn terminal_classifications_have_one_bounded_wire_grammar() {
    for classification in [
        ProviderTerminalClassification::ProviderError,
        ProviderTerminalClassification::Cancelled,
    ] {
        assert_eq!(
            ProviderTerminalClassification::parse(classification.as_str()),
            Some(classification)
        );
    }
    assert_eq!(
        ProviderTerminalClassification::parse("unknown_failure"),
        None
    );

    for classification in [
        ToolTerminalClassification::Cancelled,
        ToolTerminalClassification::NonzeroExit,
        ToolTerminalClassification::OutputCollectorFailed,
        ToolTerminalClassification::OutputDrainTimeout,
        ToolTerminalClassification::ProcessReapFailed,
        ToolTerminalClassification::ProcessSetupFailed,
        ToolTerminalClassification::ProcessSignalFailed,
        ToolTerminalClassification::ReconciledFailure,
        ToolTerminalClassification::SignalTermination,
        ToolTerminalClassification::StderrCapExceeded,
        ToolTerminalClassification::StdoutCapExceeded,
        ToolTerminalClassification::StdoutStderrCapExceeded,
        ToolTerminalClassification::ToolTimedOut,
    ] {
        assert_eq!(
            ToolTerminalClassification::parse(classification.as_str()),
            Some(classification)
        );
    }
    for unknown in ["tool_failed", "unknown_failure"] {
        assert_eq!(ToolTerminalClassification::parse(unknown), None);
    }
}
