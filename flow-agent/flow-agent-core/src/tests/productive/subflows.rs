use super::super::support::run_isolated_test;
use super::super::test_support::workspace_copy;
use super::support::{
    MemoryAttempts, MemorySink, ScriptedProvider, load_productive_execution_fixture,
    string_provider_turn,
};
use crate::runtime::{
    openai_codex::{ProviderToolCall, ProviderTurn},
    productive::execute_productive_flow,
    subflows::{SubflowInstanceRequest, SubflowLauncher, SubflowOutcome},
    types::RuntimeError,
};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct FakeLauncher {
    counts: Mutex<BTreeMap<String, usize>>,
    batches: Mutex<Vec<Vec<SubflowInstanceRequest>>>,
    failing: Vec<String>,
}

impl SubflowLauncher for FakeLauncher {
    fn names(
        &self,
        _parent_conversation_id: &str,
        flows: &[String],
    ) -> Result<Vec<String>, RuntimeError> {
        let mut counts = self.counts.lock().unwrap();
        Ok(flows
            .iter()
            .map(|flow| {
                let count = counts.entry(flow.clone()).or_default();
                *count += 1;
                format!("{flow}-{count}")
            })
            .collect())
    }

    fn release(&self, _names: &[String]) {}

    fn run(
        &self,
        _parent_conversation_id: &str,
        instances: &[SubflowInstanceRequest],
    ) -> Vec<SubflowOutcome> {
        self.batches.lock().unwrap().push(instances.to_vec());
        instances
            .iter()
            .map(|instance| {
                if self.failing.contains(&instance.name) {
                    SubflowOutcome::Failed("the page did not load".to_owned())
                } else {
                    let core_script::FlowValue::String(text) = &instance.input else {
                        return SubflowOutcome::Failed("input is not text".to_owned());
                    };
                    SubflowOutcome::Completed(core_script::FlowValue::String(format!(
                        "checked {text}"
                    )))
                }
            })
            .collect()
    }
}

fn write_registry(workspace: &Path, subflows: &str) {
    for (path, source) in [
        (
            "phases/smoke.yaml",
            format!(
                "phase:\n  id: smoke\n  name: Smoke\n  instruction_refs: []\n  tool_refs: []\n  subflows:\n{subflows}  output:\n    type: string\n"
            ),
        ),
        (
            "phases/check.yaml",
            "phase:\n  id: check\n  name: Check\n  instruction_refs: []\n  tool_refs: []\n  output:\n    type: string\n".to_owned(),
        ),
        (
            "flows/scraper.yaml",
            "flow:\n  id: scraper\n  name: Scraper\n  phase_refs: [check]\n".to_owned(),
        ),
    ] {
        fs::write(workspace.join("registry").join(path), source).expect("definition writes");
    }
}

fn subflow_call(call_id: &str, input: &str) -> ProviderToolCall {
    ProviderToolCall {
        call_id: call_id.to_owned(),
        name: "subflow-scraper".to_owned(),
        arguments: serde_json::json!({"input": input}).to_string(),
    }
}

fn run(
    subflows: &str,
    turns: Vec<ProviderTurn>,
    root_input: Option<core_script::FlowValue>,
    launcher: Arc<FakeLauncher>,
) -> (Result<bool, RuntimeError>, Vec<serde_json::Value>) {
    crate::runtime::subflows::install_subflow_launcher(launcher);
    let workspace = workspace_copy("smoke-flow");
    write_registry(&workspace, subflows);
    let fixture = load_productive_execution_fixture(&workspace);
    let mut execution = fixture.execution(fixture.smoke_flow(), "subflow-fixture");
    execution.root_input = root_input;
    let mut provider = ScriptedProvider::new(turns);
    let result = execute_productive_flow(
        execution,
        &mut provider,
        &mut MemoryAttempts::default(),
        &mut MemorySink::default(),
    )
    .map(|execution| execution.failed);
    (result, provider.bodies)
}

#[test]
fn a_phase_starts_one_instance_per_list_item_before_its_first_turn() {
    const CHILD_ENV: &str = "WATERSHED_SUBFLOW_EACH_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }
    let launcher = Arc::new(FakeLauncher::default());
    let input = core_script::FlowValue::Map(BTreeMap::from([(
        "claims".to_owned(),
        core_script::FlowValue::List(vec![
            core_script::FlowValue::String("sky".to_owned()),
            core_script::FlowValue::String("sea".to_owned()),
        ]),
    )]));

    let (failed, bodies) = run(
        "    - flow: scraper\n      start:\n        each: claims\n",
        vec![string_provider_turn("response", "done")],
        Some(input),
        Arc::clone(&launcher),
    );

    assert!(!failed.expect("the Run ends"));
    let batches = launcher.batches.lock().unwrap();
    assert_eq!(batches.len(), 1, "both instances start together");
    assert_eq!(
        batches[0]
            .iter()
            .map(|request| request.name.as_str())
            .collect::<Vec<_>>(),
        ["scraper-1", "scraper-2"]
    );
    let first_input = bodies[0]["input"].to_string();
    assert!(first_input.contains("checked sky"), "{first_input}");
    assert!(first_input.contains("scraper-2"), "{first_input}");
}

#[test]
fn the_phase_llm_starts_instances_like_tools_and_gets_their_results() {
    const CHILD_ENV: &str = "WATERSHED_SUBFLOW_LLM_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }
    let launcher = Arc::new(FakeLauncher::default());
    let mut calling = string_provider_turn("response-calls", "unused");
    calling.output_text.clear();
    calling.tool_calls = vec![
        subflow_call("call-1", "rust"),
        subflow_call("call-2", "dart"),
    ];

    let (failed, bodies) = run(
        "    - flow: scraper\n      start: llm\n",
        vec![calling, string_provider_turn("response-final", "done")],
        None,
        Arc::clone(&launcher),
    );

    assert!(!failed.expect("the Run ends"));
    assert!(
        bodies[0]["tools"]
            .as_array()
            .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == "subflow-scraper"))
    );
    assert_eq!(
        launcher.batches.lock().unwrap().len(),
        1,
        "one turn's calls run together"
    );
    let outputs = bodies[1]["input"]
        .as_array()
        .expect("the second turn carries input")
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .map(|item| (item["call_id"].clone(), item["output"].to_string()))
        .collect::<Vec<_>>();
    assert_eq!(outputs.len(), 2);
    assert!(outputs[1].1.contains("checked dart"), "{outputs:?}");
}

#[test]
fn a_failed_instance_fails_the_phase_unless_it_may_continue() {
    const CHILD_ENV: &str = "WATERSHED_SUBFLOW_FAILURE_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }
    let calling = || {
        let mut calling = string_provider_turn("response-calls", "unused");
        calling.output_text.clear();
        calling.tool_calls = vec![
            subflow_call("call-1", "rust"),
            subflow_call("call-2", "dart"),
        ];
        calling
    };
    let failing = || {
        Arc::new(FakeLauncher {
            failing: vec!["scraper-1".to_owned()],
            ..FakeLauncher::default()
        })
    };

    let (stopped, _) = run(
        "    - flow: scraper\n      start: llm\n",
        vec![calling(), string_provider_turn("response-final", "done")],
        None,
        failing(),
    );
    let (continued, bodies) = run(
        "    - flow: scraper\n      start: llm\n      continue_on_error: true\n",
        vec![calling(), string_provider_turn("response-final", "done")],
        None,
        failing(),
    );

    assert!(stopped.expect("the Run ends"));
    assert!(!continued.expect("the Run ends"));
    assert!(
        bodies[1]["input"]
            .to_string()
            .contains("the page did not load")
    );
}

#[test]
fn a_turn_asking_for_too_many_instances_names_none_and_fails_the_run() {
    const CHILD_ENV: &str = "WATERSHED_SUBFLOW_TOO_MANY_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }
    let launcher = Arc::new(FakeLauncher::default());
    let mut calling = string_provider_turn("response-calls", "unused");
    calling.output_text.clear();
    calling.tool_calls = (0..=crate::MAX_SUBFLOW_BATCH)
        .map(|index| subflow_call(&format!("call-{index}"), "topic"))
        .collect();

    let (failed, _) = run(
        "    - flow: scraper\n      start: llm\n",
        vec![calling],
        None,
        Arc::clone(&launcher),
    );

    assert!(failed.expect("the refusal ends the Run as failed"));
    assert!(
        launcher.counts.lock().unwrap().is_empty(),
        "no name is taken"
    );
    assert!(launcher.batches.lock().unwrap().is_empty());
}
