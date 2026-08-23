use super::workspace::{
    disable_smoke_echo_tool, load_test_registry, write_productive_workspace_config,
};
use crate::{
    runtime::{
        context::{ContextHistory, ContextModelProfile},
        execution_plan::runtime_policy_target,
        fs_guards::AnchoredWorkspace,
        oauth_credential::CredentialRecord,
        productive::ProductiveExecution,
        types::EventClock,
    },
    tests::test_support::{TempWorkspace, workspace_copy},
};
use std::path::Path;

fn fixture_credential() -> CredentialRecord {
    CredentialRecord {
        credential_type: "oauth".to_owned(),
        access: "secret-access".to_owned(),
        refresh: "secret-refresh".to_owned(),
        expires: u64::MAX,
        account_id: "secret-account".to_owned(),
        is_fedramp: false,
    }
}

pub(in crate::tests) struct ProductiveExecutionFixture {
    pub(in crate::tests) anchored: AnchoredWorkspace,
    credential: CredentialRecord,
    pub(in crate::tests) policy: core_policy::PolicyArtifact,
    pub(in crate::tests) registry: core_script::ResolvedRegistry,
}

impl ProductiveExecutionFixture {
    pub(in crate::tests) fn credential(&self) -> &CredentialRecord {
        &self.credential
    }

    pub(in crate::tests) fn execution<'a>(
        &'a self,
        root_flow: &'a core_script::FlowBlock,
        session_id: &'a str,
    ) -> ProductiveExecution<'a> {
        ProductiveExecution {
            conversation_id: "conversation",
            clock: EventClock::fixed_fixture(),
            credential: &self.credential,
            model: "gpt-fixture",
            model_profile: ContextModelProfile::stub_v0(),
            policy: &self.policy,
            prior_history: ContextHistory::default(),
            registry: &self.registry,
            repository_instructions: "",
            root_flow,
            root_input: None,
            session_id,
            workspace: &self.anchored,
        }
    }

    pub(in crate::tests) fn smoke_flow(&self) -> &core_script::FlowBlock {
        self.registry.flow_block("smoke-flow").expect("root Flow")
    }
}

pub(in crate::tests) fn smoke_productive_execution_fixture()
-> (TempWorkspace, ProductiveExecutionFixture) {
    let workspace = workspace_copy("smoke-flow");
    let fixture = load_productive_execution_fixture(&workspace);
    (workspace, fixture)
}

pub(in crate::tests) fn disabled_smoke_productive_execution_fixture()
-> (TempWorkspace, ProductiveExecutionFixture) {
    let workspace = workspace_copy("smoke-flow");
    disable_smoke_echo_tool(&workspace);
    let fixture = load_productive_execution_fixture(&workspace);
    (workspace, fixture)
}

#[cfg(unix)]
pub(in crate::tests) fn configured_smoke_productive_execution_fixture()
-> (TempWorkspace, ProductiveExecutionFixture) {
    let (workspace, fixture) = smoke_productive_execution_fixture();
    write_productive_workspace_config(&workspace);
    (workspace, fixture)
}

pub(in crate::tests) fn disabled_configured_smoke_productive_execution_fixture()
-> (TempWorkspace, ProductiveExecutionFixture) {
    let (workspace, fixture) = disabled_smoke_productive_execution_fixture();
    write_productive_workspace_config(&workspace);
    (workspace, fixture)
}

pub(in crate::tests) fn load_productive_execution_fixture(
    workspace: &Path,
) -> ProductiveExecutionFixture {
    load_productive_execution_fixture_for_flow(workspace, "smoke-flow")
}

pub(in crate::tests) fn load_productive_execution_fixture_for_flow(
    workspace: &Path,
    flow_id: &str,
) -> ProductiveExecutionFixture {
    load_productive_execution_fixture_with_credential(workspace, flow_id, fixture_credential())
}

pub(in crate::tests) fn load_productive_execution_fixture_with_credential(
    workspace: &Path,
    flow_id: &str,
    credential: CredentialRecord,
) -> ProductiveExecutionFixture {
    let registry = load_test_registry(workspace, flow_id);
    let policy = core_policy::compile_policy_artifact(&registry, flow_id, runtime_policy_target())
        .expect("policy");
    let anchored = AnchoredWorkspace::open(workspace).expect("workspace anchor");
    ProductiveExecutionFixture {
        anchored,
        credential,
        policy,
        registry,
    }
}
