//! Process-death matrix for productive Runs.
//!
//! Every durable step of a productive root or continuation Run, including its creation, is a
//! point where a real child process dies. The store that survives keeps its real inodes and
//! directory identities. Exact Resume and one-id Resume run on each surviving store and must keep
//! the PROTOCOL.md recovery invariants.

use crate::{
    runtime::{
        config_io::{GlobalConfig, load_global_config},
        context::{CONTEXT_SAFETY_MARGIN, ContextModelProfile, OPERATOR_MODEL_PROFILE_ID},
        conversations::conversation_status_page,
        fs_guards::{
            CRASH_EXIT_CODE, CrashPoint, PowerLoss, finish_durable_trace, start_durable_trace,
            start_power_loss_trace,
        },
        openai_codex::ProviderTurn,
        productive::{ProductiveProvider, ProviderCredential},
        session::{
            continue_conversation_with_provider, resume_conversation_run_with_provider,
            run_productive_session_with_provider,
        },
        types::{EmitMode, RunOutput, RuntimeError},
    },
    tests::{
        helpers::{
            ProductiveExecutionFixture, disabled_configured_smoke_productive_execution_fixture,
            load_productive_execution_fixture, workspace_session_dir, workspace_store_dir,
        },
        test_support::{TempWorkspace, current_test_name, session_home_path},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs, panic,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

const CRASH_CHILD_WORKSPACE: &str = "WATERSHED_CRASH_MATRIX_WORKSPACE";
const CRASH_CHILD_STEP: &str = "WATERSHED_CRASH_MATRIX_STEP";
const CRASH_CHILD_DEATH: &str = "WATERSHED_CRASH_MATRIX_DEATH";
const CRASH_RECORD: &str = "crash-record.json";

const CONVERSATION_ID: &str = "smoke-flow";

/// Counts provider dispatches; the child reports the count it reached in its crash record.
struct EffectProvider {
    effects: Arc<AtomicUsize>,
}

impl ProductiveProvider for EffectProvider {
    fn turn(
        &mut self,
        _credential: &ProviderCredential,
        _body: &serde_json::Value,
    ) -> Result<ProviderTurn, RuntimeError> {
        let calls = self.effects.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(ProviderTurn {
            token_usage: None,
            response_id: format!("matrix-response-{calls}"),
            output_text: "{\"type\":\"string\",\"value\":\"continued\"}".to_owned(),
            retained_items: vec![serde_json::json!({
                "content": [],
                "id": format!("matrix-message-{calls}"),
                "role": "assistant",
                "type": "message"
            })],
            tool_calls: Vec::new(),
        })
    }
}

/// One workspace whose productive Runs share one provider-effect counter.
struct Store {
    config: GlobalConfig,
    effects: Arc<AtomicUsize>,
    fixture: ProductiveExecutionFixture,
    root: PathBuf,
    sessions: PathBuf,
    workspace: PathBuf,
}

impl Store {
    /// A fresh workspace; the guard removes it when dropped.
    fn fresh() -> (TempWorkspace, Self) {
        let (workspace, fixture) = disabled_configured_smoke_productive_execution_fixture();
        let store = Self::with_fixture(&workspace, fixture);
        (workspace, store)
    }

    /// The store of an existing workspace, as the process that crashed or recovers it sees it.
    fn at(workspace: &Path) -> Self {
        Self::with_fixture(workspace, load_productive_execution_fixture(workspace))
    }

    fn with_fixture(workspace: &Path, fixture: ProductiveExecutionFixture) -> Self {
        Self {
            config: load_global_config().expect("productive config"),
            effects: Arc::new(AtomicUsize::new(0)),
            fixture,
            root: workspace_store_dir(workspace),
            sessions: workspace_session_dir(workspace),
            workspace: workspace.to_path_buf(),
        }
    }

    fn provider(&self) -> EffectProvider {
        EffectProvider {
            effects: self.effects.clone(),
        }
    }

    fn start_root(&self) -> Result<RunOutput, RuntimeError> {
        let credential = self.fixture.credential().clone();
        run_productive_session_with_provider(
            &self.workspace,
            &self.fixture.anchored,
            &self.config,
            "gpt-fixture",
            ContextModelProfile {
                context_limit: 128_000,
                id: OPERATOR_MODEL_PROFILE_ID,
                output_reserve: 16_384,
                safety_margin: CONTEXT_SAFETY_MARGIN,
            },
            &self.fixture.registry,
            self.fixture.smoke_flow(),
            &self.fixture.policy,
            None,
            false,
            || Ok(credential),
            // WHY: the fixture workspace has no AGENTS.md, which resume reads as empty guidance;
            // a Run with other guidance replays a different request and cannot resume.
            "",
            None,
            &mut self.provider(),
        )
    }

    fn continue_newest(&self) -> Result<RunOutput, RuntimeError> {
        let credential = self.fixture.credential().clone();
        continue_conversation_with_provider(
            &self.workspace,
            CONVERSATION_ID,
            None,
            None,
            None,
            false,
            || Ok(credential),
            &mut self.provider(),
        )
    }

    fn resume_run(&self, run_session_id: &str) -> Result<RunOutput, RuntimeError> {
        let credential = self.fixture.credential().clone();
        resume_conversation_run_with_provider(
            &self.workspace,
            CONVERSATION_ID,
            run_session_id,
            EmitMode::Human,
            || Ok(credential),
            &mut self.provider(),
        )
    }

    /// Names of the entries under the conversation's `runs` directory, staging names included.
    fn run_entries(&self) -> BTreeSet<String> {
        match fs::read_dir(self.sessions.join(CONVERSATION_ID).join("runs")) {
            Ok(entries) => entries
                .map(|entry| {
                    entry
                        .expect("run entry reads")
                        .file_name()
                        .into_string()
                        .expect("run name is UTF-8")
                })
                .collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(error) => panic!("run directory lists: {error}"),
        }
    }

    fn published_runs(&self) -> BTreeSet<String> {
        self.run_entries()
            .into_iter()
            .filter(|name| !is_staging_name(name))
            .collect()
    }
}

fn is_staging_name(name: &str) -> bool {
    name.starts_with(".run-") && name.ends_with(".staged")
}

fn json_lines(path: &Path) -> Vec<serde_json::Value> {
    match fs::read_to_string(path) {
        Ok(text) => text
            .lines()
            .map(|line| serde_json::from_str(line).expect("store line is JSON"))
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("store file reads: {error}"),
    }
}

/// Checks status.json against the Run directories, Run Logs and history file.
///
/// The oracle parses the stored records directly, so it does not reuse the runtime readers.
fn assert_status_agrees_with_logs(store: &Store, step: &str) {
    let summary = store.sessions.join(CONVERSATION_ID).join("status.json");
    if !summary.exists() {
        // WHY: a Conversation whose creation was interrupted before its summary is either
        // omitted or fails the listing closed (PROTOCOL.md: a missing summary fails closed).
        if let Ok(page) = conversation_status_page(&store.workspace, None) {
            assert!(
                page.conversations
                    .iter()
                    .all(|status| status.conversation_id != CONVERSATION_ID),
                "{step}: a Conversation without a summary is not listed"
            );
        }
        return;
    }
    let page = match conversation_status_page(&store.workspace, None) {
        Ok(page) => page,
        Err(error) => {
            // WHY: a malformed summary fails the listing closed, and so does a pending Run creation
            // until a reservation of its Conversation recovers it (PROTOCOL.md). The next
            // reservation check below verifies that recovery.
            let pending = store
                .sessions
                .join(CONVERSATION_ID)
                .join(".status-transaction.json")
                .exists()
                || store.run_entries().iter().any(|name| is_staging_name(name));
            assert!(
                error.to_string().contains("status summary") || pending,
                "{step}: listing fails closed: {error}"
            );
            return;
        }
    };
    let status = page
        .conversations
        .iter()
        .find(|status| status.conversation_id == CONVERSATION_ID)
        .unwrap_or_else(|| panic!("{step}: status summary exists"));
    let runs = store.published_runs();
    assert_eq!(status.run_count, runs.len(), "{step}: run count");

    let mut uncertain = 0;
    for run in &runs {
        let records = json_lines(
            &store
                .sessions
                .join(CONVERSATION_ID)
                .join("runs")
                .join(run)
                .join("run-log.jsonl"),
        );
        let mut open = BTreeSet::new();
        for record in &records {
            let Some(attempt) = record["attempt_id"].as_str() else {
                continue;
            };
            match record["record_type"].as_str() {
                Some("intent") => {
                    open.insert(attempt.to_owned());
                }
                Some("terminal-result") => {
                    open.remove(attempt);
                }
                _ => {}
            }
        }
        uncertain += open.len();
    }
    assert_eq!(
        status.uncertain_attempts, uncertain,
        "{step}: uncertain attempts"
    );

    let history = json_lines(&store.sessions.join(CONVERSATION_ID).join("history.jsonl"));
    let latest = history
        .last()
        .and_then(|entry| entry["entry_id"].as_str().map(str::to_owned));
    assert_eq!(status.latest_entry_id, latest, "{step}: latest entry");
}

/// What the dying child leaves for its parent: the provider effects before the scenario and at the
/// crash, the Runs published before the scenario, and the durable steps reached (the last one is
/// the fatal step).
#[derive(Deserialize, Serialize)]
struct CrashRecord {
    baseline_effects: usize,
    effects: usize,
    runs: BTreeSet<String>,
    steps: Vec<String>,
}

/// Runs exact Resume and one-id Resume on one surviving store and checks the at-most-once rules.
///
/// Every published Run dispatches at most one provider effect in total, so the effects added
/// since the scenario started never exceed the Runs added since then.
///
/// `must_resume` marks a power cut that lost a final line feed of an event or context log: ADR-0259
/// requires exact Resume to drop it and resume, not merely to fail closed.
fn assert_recovery_invariants(
    store: &Store,
    crashed: &CrashRecord,
    run: Option<&str>,
    step: &str,
    must_resume: bool,
) {
    let added_runs = |store: &Store| store.published_runs().len() - crashed.runs.len();
    let effects = || store.effects.load(Ordering::SeqCst);
    let base = crashed.baseline_effects;

    let before_exact = effects();
    let exact = run.map(|run| store.resume_run(run));
    let exact_dispatches = effects() - before_exact;
    match &exact {
        Some(Ok(output)) => {
            assert!(!output.failed, "{step}: exact Resume completes its Run");
            // A completed Run has exactly one provider effect, whether the crash preceded it.
            assert_eq!(
                before_exact - base + exact_dispatches,
                1,
                "{step}: a completed Run dispatches its provider exactly once"
            );
        }
        Some(Err(error)) => {
            assert!(
                !must_resume,
                "{step}: exact Resume of a cut last line must resume: {error}"
            );
            assert_eq!(
                error.exit_code(),
                65,
                "{step}: exact Resume fails closed: {error}"
            );
            assert_eq!(
                exact_dispatches, 0,
                "{step}: failed exact Resume dispatches nothing"
            );
        }
        None => {}
    }
    assert!(
        effects() - base <= added_runs(store),
        "{step}: exact Resume dispatched a second effect"
    );
    assert_status_agrees_with_logs(store, &format!("{step}, after exact Resume"));

    let before_continue = effects();
    let continued = store.continue_newest();
    let continue_dispatches = effects() - before_continue;
    match &continued {
        Ok(output) => {
            assert!(!output.failed, "{step}: one-id Resume completes its Run");
            assert_eq!(
                continue_dispatches, 1,
                "{step}: a new Run dispatches exactly once"
            );
        }
        Err(error) => {
            assert_eq!(
                error.exit_code(),
                65,
                "{step}: one-id Resume fails closed: {error}"
            );
            assert_eq!(
                continue_dispatches, 0,
                "{step}: failed one-id Resume dispatches nothing"
            );
        }
    }
    assert!(
        effects() - base <= added_runs(store),
        "{step}: one-id Resume dispatched a second effect"
    );
    assert_status_agrees_with_logs(store, &format!("{step}, after one-id Resume"));
}

/// Replaces generated identifiers (digests, counters, timestamps) in step labels.
///
/// Replays compare by step kind and structure; the identifiers differ between processes.
fn normalized(steps: &[String]) -> Vec<String> {
    steps
        .iter()
        .map(|step| {
            let mut normalized = String::new();
            let mut run = String::new();
            for character in step.chars().chain(std::iter::once(' ')) {
                if character.is_ascii_hexdigit() {
                    run.push(character);
                    continue;
                }
                if run.len() >= 8 {
                    normalized.push_str("<id>");
                } else {
                    normalized.push_str(&run);
                }
                run.clear();
                normalized.push(character);
            }
            normalized.pop();
            normalized
        })
        .collect()
}

/// How a child dies at its crash point: a process death keeps every byte it wrote, while a power
/// loss drops the bytes each file had not synced. `Cut` also keeps all but the last byte of the
/// last unsynced write, so a JSONL record survives without its line feed (ADR-0259).
#[derive(Clone, Copy, Debug)]
enum Death {
    Process,
    Power(PowerLoss),
}

impl Death {
    fn label(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Power(PowerLoss::Drop) => "drop",
            Self::Power(PowerLoss::CutLastWrite) => "cut",
        }
    }

    fn parse(label: &str) -> Self {
        match label {
            "process" => Self::Process,
            "drop" => Self::Power(PowerLoss::Drop),
            "cut" => Self::Power(PowerLoss::CutLastWrite),
            other => panic!("unknown crash death {other}"),
        }
    }
}

/// The deaths a parent checks at durable step `index` of a reference with `steps`.
fn deaths_at(power: bool, steps: &[String], index: usize) -> Vec<Death> {
    if !power {
        return vec![Death::Process];
    }
    let mut deaths = vec![Death::Power(PowerLoss::Drop)];
    // WHY: a cut takes bytes only from an unsynced write, so it differs from a drop only when the
    // step before the crash point is a write that no sync has covered.
    if index > 0 && steps[index - 1].starts_with("write ") {
        deaths.push(Death::Power(PowerLoss::CutLastWrite));
    }
    deaths
}

/// The child role of a matrix: the workspace, durable step and death its parent assigned, if any.
fn crash_child_role() -> Option<(PathBuf, usize, Death)> {
    let workspace = std::env::var_os(CRASH_CHILD_WORKSPACE)?;
    let index = std::env::var(CRASH_CHILD_STEP)
        .expect("crash child step is configured")
        .parse()
        .expect("crash child step is an index");
    let death =
        Death::parse(&std::env::var(CRASH_CHILD_DEATH).expect("crash child death is configured"));
    Some((PathBuf::from(workspace), index, death))
}

/// Runs the setup and scenario in a child process that dies at durable step `index`.
fn die_at_step(workspace: &Path, index: usize, death: Death) -> CrashRecord {
    let output = Command::new(std::env::current_exe().expect("test executable resolves"))
        .args(["--exact", &current_test_name(), "--nocapture"])
        .env(CRASH_CHILD_WORKSPACE, workspace)
        .env(CRASH_CHILD_STEP, index.to_string())
        .env(CRASH_CHILD_DEATH, death.label())
        .env("FLOW_AGENT_HOME", session_home_path())
        .output()
        .expect("crash child starts");
    assert_eq!(
        output.status.code(),
        Some(CRASH_EXIT_CODE),
        "crash child dies at durable step {index}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = fs::read(workspace.join(CRASH_RECORD)).expect("crash record is written");
    serde_json::from_slice(&bytes).expect("crash record parses")
}

/// Kills a child process at each selected durable step of `scenario` and checks each survivor.
///
/// `setup` runs untraced in every process before the traced scenario. Returns the reference step
/// list of an unfaulted run.
///
/// A power-loss matrix traces file syncs as steps too, and checks each step with a drop and, after
/// an unsynced write, a cut.
fn run_crash_matrix(
    setup: impl Fn(&Store),
    scenario: impl Fn(&Store) -> Result<RunOutput, RuntimeError>,
    selected: impl Fn(usize) -> bool,
    next_reservation: impl Fn(&Store, &str),
    power: bool,
) -> Vec<String> {
    if let Some((workspace, index, death)) = crash_child_role() {
        let store = Store::at(&workspace);
        setup(&store);
        let record = workspace.join(CRASH_RECORD);
        let effects = store.effects.clone();
        let baseline_effects = effects.load(Ordering::SeqCst);
        let runs = store.published_runs();
        let crash = Some(CrashPoint {
            index,
            power: match death {
                Death::Process => None,
                Death::Power(loss) => Some(loss),
            },
            on_crash: Box::new(move |steps| {
                let crashed = CrashRecord {
                    baseline_effects,
                    effects: effects.load(Ordering::SeqCst),
                    runs,
                    steps: steps.to_vec(),
                };
                fs::write(
                    &record,
                    serde_json::to_vec(&crashed).expect("crash record serializes"),
                )
                .expect("crash record is written");
            }),
        });
        match death {
            Death::Process => start_durable_trace(&store.root, crash),
            Death::Power(_) => start_power_loss_trace(&store.root, crash),
        }
        let outcome = scenario(&store);
        panic!("durable step {index} is not reached: {:?}", outcome.err());
    }

    let reference = {
        let (_workspace, store) = Store::fresh();
        setup(&store);
        if power {
            start_power_loss_trace(&store.root, None);
        } else {
            start_durable_trace(&store.root, None);
        }
        let completed = scenario(&store);
        let steps = finish_durable_trace(&store.root);
        assert!(
            completed.is_ok(),
            "reference scenario completes: {:?}",
            completed.err()
        );
        assert!(!steps.is_empty(), "scenario records durable steps");
        steps
    };

    // WHY: every failing crash point is reported together, so one run shows the whole picture.
    let mut failures = Vec::new();
    for (index, step) in reference
        .iter()
        .enumerate()
        .filter(|(index, _)| selected(*index))
    {
        for death in deaths_at(power, &reference, index) {
            let label = format!("step {index} ({step}, {})", death.label());
            let checked = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                let (_workspace, store) = Store::fresh();
                let crashed = die_at_step(&store.workspace, index, death);
                assert_eq!(
                    normalized(&crashed.steps),
                    normalized(&reference[..=index]),
                    "{label} replays the same steps"
                );
                store.effects.store(crashed.effects, Ordering::SeqCst);
                let run = store
                    .published_runs()
                    .difference(&crashed.runs)
                    .next()
                    .cloned();
                assert_recovery_invariants(
                    &store,
                    &crashed,
                    run.as_deref(),
                    &label,
                    matches!(death, Death::Power(PowerLoss::CutLastWrite))
                        && index > 0
                        && adr_0259_stream_write(&reference[index - 1]),
                );
                next_reservation(&store, &label);
            }));
            if checked.is_err() {
                failures.push(label);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} crash points fail recovery: {}",
        failures.len(),
        failures.join("; ")
    );
    reference
}

/// Whether a traced `write` step appended to an event or context log, the streams ADR-0259 covers.
fn adr_0259_stream_write(step: &str) -> bool {
    let Some(path) = step.strip_prefix("write ") else {
        return false;
    };
    let leaf = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    leaf.starts_with("events") || leaf.starts_with("contexts")
}

/// A Conversation's staged Run left by a crash is gone after the next reservation.
fn assert_no_staged_run(store: &Store, step: &str) {
    assert!(
        store
            .run_entries()
            .iter()
            .all(|name| !is_staging_name(name)),
        "{step}: the next reservation reclaims the staged Run"
    );
}

/// The next reservation after a root Run's crash: a fresh root reclaims the Run the crash staged.
fn next_root_reservation(store: &Store, step: &str) {
    let output = store
        .start_root()
        .unwrap_or_else(|error| panic!("{step}: fresh root Run fails: {error}"));
    assert!(!output.failed, "{step}: fresh root Run completes");
    assert_no_staged_run(store, step);
}

#[test]
fn root_run_survives_a_process_death_at_every_durable_step() {
    let reference = run_crash_matrix(
        |_| {},
        Store::start_root,
        |_| true,
        next_root_reservation,
        false,
    );

    for family in [
        "run-log.jsonl",
        "recovery.jsonl",
        "history.jsonl",
        "status.json",
        "objects/",
        "events.jsonl",
    ] {
        assert!(
            reference.iter().any(|step| step.contains(family)),
            "root Run reaches a durable step under {family}"
        );
    }
}

/// Builds the twelve-Run chain that the depth-twelve matrices start from.
fn build_twelve_run_chain(store: &Store) {
    store.start_root().expect("root Run completes");
    for depth in 1..=11 {
        let output = store
            .continue_newest()
            .unwrap_or_else(|error| panic!("continuation {depth} completes: {error}"));
        assert!(!output.failed, "continuation {depth} completes");
    }
    assert_eq!(
        store.published_runs().len(),
        12,
        "the chain holds twelve Runs"
    );
}

#[test]
fn continuation_run_at_depth_twelve_survives_a_process_death_at_every_durable_step() {
    // WHY: each child replays the twelve-Run prefix, so the chain's matrix samples its steps to
    // bound the runtime: every fourth step of the Run creation prefix, then every twentieth step
    // of the attempt and recovery records that follow.
    let reference = run_crash_matrix(
        build_twelve_run_chain,
        Store::continue_newest,
        |index| (index < 40 && index % 4 == 0) || index % 20 == 0,
        assert_no_staged_run,
        false,
    );
    assert!(
        reference.iter().any(|step| step.contains("history.jsonl")),
        "a continuation appends its history entry"
    );
}

#[test]
fn root_run_survives_a_power_loss_at_every_durable_step() {
    let reference = run_crash_matrix(
        |_| {},
        Store::start_root,
        |_| true,
        next_root_reservation,
        true,
    );
    assert!(
        reference.iter().any(|step| step.starts_with("sync ")),
        "a power-loss trace records file syncs as durable steps"
    );
}

#[test]
fn continuation_run_at_depth_twelve_survives_a_power_loss_at_sampled_durable_steps() {
    // WHY: a power trace also records file syncs, so it has about twice the steps of the process
    // trace. This sample covers the first eighty steps every eighth step, then every fortieth.
    let reference = run_crash_matrix(
        build_twelve_run_chain,
        Store::continue_newest,
        |index| (index < 80 && index % 8 == 0) || index % 40 == 0,
        assert_no_staged_run,
        true,
    );
    assert!(
        reference.iter().any(|step| step.starts_with("sync ")),
        "a power-loss trace records file syncs as durable steps"
    );
}
