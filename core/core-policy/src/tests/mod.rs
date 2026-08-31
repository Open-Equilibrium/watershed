mod artifact;
mod compile;

use crate::{PolicyTarget, TrustedPredefinedCommand};
use std::{collections::BTreeSet, path::Path};

const POLICY_TARGETS: [PolicyTarget; 2] = [
    PolicyTarget::LinuxLandlockSeccomp,
    PolicyTarget::MacosSeatbelt,
];

#[test]
fn trusted_predefined_command_identities_are_stable_and_complete() {
    let mut command_ids = BTreeSet::new();
    for command in TrustedPredefinedCommand::ALL {
        assert!(command_ids.insert(command.as_str()));
        assert_eq!(
            TrustedPredefinedCommand::parse(command.as_str()),
            Some(command)
        );
        assert_eq!(
            command.executable(),
            format!("registry:{}", command.as_str())
        );
    }
    assert_eq!(
        command_ids,
        BTreeSet::from(["agent-echo", "agent-negative", "agent-read"])
    );
    assert_eq!(TrustedPredefinedCommand::parse("agent-unknown"), None);
}

fn fixture_registry(fixture: &str, flow_ref: &str) -> core_script::ResolvedRegistry {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../flow-agent/fixtures")
        .join(fixture);
    core_script::load_flow_registry_from_root(&workspace, Path::new("registry"), flow_ref)
        .expect("fixture registry loads")
}
