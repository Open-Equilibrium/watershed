//! Process-death matrix for productive Runs.
//!
//! Every state-changing step of a productive root or continuation Run, including its creation,
//! is captured as the store image a dying process leaves at that boundary. Exact Resume and
//! one-id Resume run on each image and must keep the PROTOCOL.md recovery invariants.

use crate::{
    runtime::{
        config_io::{GlobalConfig, load_global_config},
        context::{CONTEXT_SAFETY_MARGIN, ContextModelProfile, OPERATOR_MODEL_PROFILE_ID},
        conversations::conversation_status_page,
        fs_guards::{
            StoreImage, capture_store_image, finish_durable_trace, restore_store_image,
            start_durable_trace,
        },
        oauth_credential::CredentialRecord,
        openai_codex::ProviderTurn,
        productive::ProductiveProvider,
        session::{
            continue_conversation_with_provider, resume_conversation_run_with_provider,
            run_productive_session_with_provider,
        },
        types::{EmitMode, RunOutput, RuntimeError},
    },
    tests::{
        helpers::{
            ProductiveExecutionFixture, disabled_configured_smoke_productive_execution_fixture,
            workspace_session_dir, workspace_store_dir,
        },
        test_support::TempWorkspace,
    },
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

const CONVERSATION_ID: &str = "smoke-flow";

/// Counts provider dispatches; the count is part of every captured store image.
struct EffectProvider {
    effects: Arc<AtomicUsize>,
}

impl ProductiveProvider for EffectProvider {
    fn turn(
        &mut self,
        _credential: &CredentialRecord,
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
    workspace: TempWorkspace,
}

impl Store {
    fn new() -> Self {
        let (workspace, fixture) = disabled_configured_smoke_productive_execution_fixture();
        Self {
            config: load_global_config().expect("productive config"),
            effects: Arc::new(AtomicUsize::new(0)),
            fixture,
            root: workspace_store_dir(&workspace),
            sessions: workspace_session_dir(&workspace),
            workspace,
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
            "Agent guidance.",
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
            // WHY: a malformed summary fails the listing closed (PROTOCOL.md). A pending Run
            // creation fails it on its identity marker, which binds the inode of a directory
            // that the store image copy recreates; only that refusal during a pending
            // transaction is accepted.
            let pending = store
                .sessions
                .join(CONVERSATION_ID)
                .join(".status-transaction.json")
                .exists()
                || store.run_entries().iter().any(|name| is_staging_name(name));
            let message = error.to_string();
            assert!(
                message.contains("status summary")
                    || (pending && message.contains("identity changed")),
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

/// The store before a scenario: its image, its published Runs and its provider-effect count.
struct Baseline {
    image: StoreImage,
    runs: BTreeSet<String>,
}

impl Baseline {
    fn capture(store: &Store) -> Self {
        Self {
            image: capture_store_image(&store.root, &store.effects),
            runs: store.published_runs(),
        }
    }

    fn restore(&self, store: &Store) {
        restore_store_image(&store.root, &self.image);
        store.effects.store(self.image.effects(), Ordering::SeqCst);
    }
}

/// Runs exact Resume and one-id Resume on one captured image and checks the at-most-once rules.
///
/// Every published Run dispatches at most one provider effect in total, so the effects added
/// since the baseline never exceed the Runs added since the baseline.
fn assert_recovery_invariants(store: &Store, baseline: &Baseline, run: Option<&str>, step: &str) {
    let added_runs = |store: &Store| store.published_runs().len() - baseline.runs.len();
    let effects = || store.effects.load(Ordering::SeqCst);
    let base = baseline.image.effects();

    let before_exact = effects();
    let exact = run.map(|run| store.resume_run(run));
    let exact_dispatches = effects() - before_exact;
    match &exact {
        Some(Ok(output)) => {
            assert!(!output.failed, "{step}: exact Resume completes its Run");
            // A completed Run has exactly one provider effect, whether the crash preceded it.
            assert_eq!(
                before_exact - baseline.image.effects() + exact_dispatches,
                1,
                "{step}: a completed Run dispatches its provider exactly once"
            );
        }
        Some(Err(error)) => {
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

/// Replays `scenario` once per selected durable step and checks each captured crash image.
///
/// Returns the reference step list of an unfaulted run.
fn run_crash_matrix(
    store: &Store,
    scenario: impl Fn(&Store) -> Result<RunOutput, RuntimeError>,
    selected: impl Fn(usize) -> bool,
) -> Vec<String> {
    let baseline = Baseline::capture(store);
    start_durable_trace(&store.root, None, Some(store.effects.clone()));
    let completed = scenario(store);
    let reference = finish_durable_trace(&store.root).steps;
    assert!(
        completed.is_ok(),
        "reference scenario completes: {:?}",
        completed.err()
    );
    assert!(!reference.is_empty(), "scenario records durable steps");

    for (index, step) in reference
        .iter()
        .enumerate()
        .filter(|(index, _)| selected(*index))
    {
        baseline.restore(store);
        start_durable_trace(&store.root, Some(index), Some(store.effects.clone()));
        let _ = scenario(store);
        let trace = finish_durable_trace(&store.root);
        assert_eq!(
            normalized(&trace.steps),
            normalized(&reference),
            "step {index} ({step}) replays the same steps"
        );
        let image = trace
            .image
            .unwrap_or_else(|| panic!("step {index} ({step}) captures its image"));
        restore_store_image(&store.root, &image);
        store.effects.store(image.effects(), Ordering::SeqCst);
        let run = store
            .published_runs()
            .difference(&baseline.runs)
            .next()
            .cloned();
        assert_recovery_invariants(
            store,
            &baseline,
            run.as_deref(),
            &format!("step {index} ({step})"),
        );
    }
    reference
}

#[test]
fn root_run_survives_a_process_death_at_every_durable_step() {
    let store = Store::new();
    let reference = run_crash_matrix(&store, Store::start_root, |_| true);

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

#[test]
fn continuation_run_at_depth_twelve_survives_a_process_death_at_every_durable_step() {
    let store = Store::new();
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

    // WHY: the chain's matrix samples its steps to bound the runtime: the Run creation prefix
    // in full, then every tenth step of the attempt and recovery records that follow.
    let reference = run_crash_matrix(&store, Store::continue_newest, |index| {
        index < 40 || index % 10 == 0
    });
    assert!(
        reference.iter().any(|step| step.contains("history.jsonl")),
        "a continuation appends its history entry"
    );
}
