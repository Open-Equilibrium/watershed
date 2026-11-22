use crate::streaming::stream_live_operation_into;
use flow_agent_core::{
    EmitMode, RuntimeError, SubflowInstanceRequest, SubflowLauncher, SubflowOutcome,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, BufRead, BufReader, Write},
    path::PathBuf,
    process::{ChildStdin, Command, ExitCode, Stdio},
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender},
    },
    thread,
};

const MAX_TREE_INSTANCES: usize = 512;
const MAX_LIVE_INSTANCES: usize = 32;
const CONVERSATION_SEPARATOR: &str = "__";

static ROOT: OnceLock<Arc<ProcessLauncher>> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum InstanceEvent {
    Started { name: String },
    Finished { name: String, failed: bool },
}

#[derive(Default)]
struct Tree {
    counts: BTreeMap<String, usize>,
    started: usize,
    live: BTreeSet<String>,
}

type NamesReply = Sender<Result<Vec<String>, String>>;

struct Upstream {
    out: Mutex<io::Stdout>,
    next: AtomicU64,
    pending: Mutex<BTreeMap<u64, NamesReply>>,
}

impl Upstream {
    fn send(&self, message: &Value) -> io::Result<()> {
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        writeln!(out, "{message}")?;
        out.flush()
    }
}

#[derive(Default)]
pub(crate) struct ProcessLauncher {
    upstream: Option<Upstream>,
    root_conversation: Mutex<Option<String>>,
    tree: Mutex<Tree>,
    children: Mutex<BTreeMap<String, Arc<Mutex<ChildStdin>>>>,
    observer: Mutex<Option<Sender<InstanceEvent>>>,
}

pub(crate) fn install_root_launcher() -> Arc<ProcessLauncher> {
    let launcher = Arc::clone(ROOT.get_or_init(|| Arc::new(ProcessLauncher::default())));
    flow_agent_core::install_subflow_launcher(launcher.clone());
    launcher
}

pub(crate) fn root_launcher() -> Option<Arc<ProcessLauncher>> {
    ROOT.get().cloned()
}

impl ProcessLauncher {
    pub(crate) fn observe(&self, sender: Sender<InstanceEvent>) {
        *self.observer.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
    }

    pub(crate) fn live_names(&self) -> Vec<String> {
        self.tree
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .live
            .iter()
            .cloned()
            .collect()
    }

    pub(crate) fn steer(&self, target: &str, text: &str) {
        self.broadcast(&json!({"steer": {"target": target, "text": text}}));
    }

    pub(crate) fn resume_all(&self) {
        self.broadcast(&json!({"resume": true}));
    }

    pub(crate) fn pause_all(&self) {
        self.broadcast(&json!({"pause": true}));
    }

    pub(crate) fn cancel_all(&self) {
        self.broadcast(&json!({"cancel": true}));
    }

    fn broadcast(&self, message: &Value) {
        let children = self
            .children
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for child in children {
            let _ = write_line(&child, message);
        }
    }

    fn report(&self, event: InstanceEvent) {
        if let InstanceEvent::Finished { name, .. } = &event {
            self.tree
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .live
                .remove(name);
        }
        match &self.upstream {
            Some(upstream) => {
                let message = match &event {
                    InstanceEvent::Started { name } => json!({"instance": {"started": name}}),
                    InstanceEvent::Finished { name, failed } => {
                        json!({"instance": {"finished": name, "failed": failed}})
                    }
                };
                let _ = upstream.send(&message);
            }
            None => {
                if let Some(observer) =
                    &*self.observer.lock().unwrap_or_else(PoisonError::into_inner)
                {
                    let _ = observer.send(event);
                }
            }
        }
    }

    fn allocate(&self, root: &str, flows: &[String]) -> Result<Vec<String>, String> {
        let workspace = std::env::current_dir().map_err(|error| error.to_string())?;
        let mut tree = self.tree.lock().unwrap_or_else(PoisonError::into_inner);
        if tree.started + flows.len() > MAX_TREE_INSTANCES {
            return Err(format!(
                "a Run's tree starts at most {MAX_TREE_INSTANCES} subflow instances"
            ));
        }
        if tree.live.len() + flows.len() > MAX_LIVE_INSTANCES {
            return Err(format!(
                "at most {MAX_LIVE_INSTANCES} subflow instances run at once"
            ));
        }
        let mut names = Vec::with_capacity(flows.len());
        for flow in flows {
            let name = loop {
                let count = tree.counts.entry(flow.clone()).or_default();
                *count += 1;
                let name = format!("{flow}-{count}");
                let conversation = format!("{root}{CONVERSATION_SEPARATOR}{name}");
                if flow_agent_core::conversation_state(&workspace, &conversation)
                    .map_err(|error| error.to_string())?
                    == flow_agent_core::ConversationState::Absent
                {
                    break name;
                }
            };
            names.push(name);
        }
        tree.started += names.len();
        tree.live.extend(names.iter().cloned());
        Ok(names)
    }

    fn handle_child_line(
        &self,
        child: &Arc<Mutex<ChildStdin>>,
        message: &Value,
    ) -> Option<SubflowOutcome> {
        if let Some(request) = message.get("name_request") {
            let id = request["id"].clone();
            let flows =
                serde_json::from_value::<Vec<String>>(request["flows"].clone()).unwrap_or_default();
            let reply = match self.names(
                request["conversation_id"].as_str().unwrap_or_default(),
                &flows,
            ) {
                Ok(names) => json!({"names": {"id": id, "names": names}}),
                Err(error) => json!({"names": {"id": id, "error": error.to_string()}}),
            };
            let _ = write_line(child, &reply);
            return None;
        }
        if let Some(names) = message.get("release") {
            self.release(&serde_json::from_value::<Vec<String>>(names.clone()).unwrap_or_default());
            return None;
        }
        if let Some(instance) = message.get("instance") {
            if let Some(name) = instance["started"].as_str() {
                self.report(InstanceEvent::Started {
                    name: name.to_owned(),
                });
            }
            if let Some(name) = instance["finished"].as_str() {
                self.report(InstanceEvent::Finished {
                    name: name.to_owned(),
                    failed: instance["failed"] == true,
                });
            }
            return None;
        }
        if let Some(result) = message.get("result") {
            return Some(
                serde_json::from_value(result.clone())
                    .map(SubflowOutcome::Completed)
                    .unwrap_or_else(|error| {
                        SubflowOutcome::Failed(format!("the subflow result is invalid: {error}"))
                    }),
            );
        }
        message["error"]
            .as_str()
            .map(|error| SubflowOutcome::Failed(error.to_owned()))
    }
}

impl SubflowLauncher for ProcessLauncher {
    fn release(&self, names: &[String]) {
        match &self.upstream {
            Some(upstream) => {
                let _ = upstream.send(&json!({"release": names}));
            }
            None => {
                let mut tree = self.tree.lock().unwrap_or_else(PoisonError::into_inner);
                for name in names {
                    tree.live.remove(name);
                }
            }
        }
    }

    fn names(
        &self,
        parent_conversation_id: &str,
        flows: &[String],
    ) -> Result<Vec<String>, RuntimeError> {
        let root = self
            .root_conversation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| parent_conversation_id.to_owned());
        let Some(upstream) = &self.upstream else {
            return self.allocate(&root, flows).map_err(RuntimeError::Protocol);
        };
        let id = upstream.next.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        upstream
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, sender);
        upstream
            .send(&json!({"name_request": {"id": id, "conversation_id": root, "flows": flows}}))
            .map_err(|source| RuntimeError::Io {
                path: PathBuf::from("<parent>"),
                source,
            })?;
        receiver
            .recv()
            .map_err(|_| RuntimeError::Protocol("the parent Run ended".to_owned()))?
            .map_err(RuntimeError::Protocol)
    }

    fn run(
        &self,
        parent_conversation_id: &str,
        instances: &[SubflowInstanceRequest],
    ) -> Vec<SubflowOutcome> {
        let root = self
            .root_conversation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| parent_conversation_id.to_owned());
        let (sender, receiver) = mpsc::channel();
        let mut outcomes = vec![None; instances.len()];
        let mut running = BTreeSet::new();
        for (index, instance) in instances.iter().enumerate() {
            match self.start(&root, index, instance, sender.clone()) {
                Ok(()) => {
                    running.insert(index);
                }
                Err(error) => {
                    self.report(InstanceEvent::Finished {
                        name: instance.name.clone(),
                        failed: true,
                    });
                    outcomes[index] = Some(SubflowOutcome::Failed(error));
                }
            }
        }
        drop(sender);
        let mut stopping = outcomes.iter().zip(instances).any(|(outcome, instance)| {
            matches!(outcome, Some(SubflowOutcome::Failed(_))) && !instance.continue_on_error
        });
        if stopping {
            self.stop(instances, &running);
        }
        for (index, outcome) in receiver {
            running.remove(&index);
            let failed = matches!(outcome, SubflowOutcome::Failed(_));
            self.report(InstanceEvent::Finished {
                name: instances[index].name.clone(),
                failed,
            });
            if failed && !instances[index].continue_on_error && !stopping {
                stopping = true;
                self.stop(instances, &running);
            }
            outcomes[index] = Some(outcome);
        }
        outcomes
            .into_iter()
            .map(|outcome| {
                outcome.unwrap_or_else(|| {
                    SubflowOutcome::Failed("the subflow process ended without a result".to_owned())
                })
            })
            .collect()
    }
}

impl ProcessLauncher {
    fn start(
        &self,
        root: &str,
        index: usize,
        instance: &SubflowInstanceRequest,
        sender: Sender<(usize, SubflowOutcome)>,
    ) -> Result<(), String> {
        let conversation_id = format!("{root}{CONVERSATION_SEPARATOR}{}", instance.name);
        if !proto::is_valid_session_id(&conversation_id) {
            return Err(format!(
                "the subflow conversation id {conversation_id} is not valid"
            ));
        }
        let program = std::env::current_exe().map_err(|error| error.to_string())?;
        let mut process = Command::new(program)
            .arg("subflow-run")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("the subflow process does not start: {error}"))?;
        let stdin = Arc::new(Mutex::new(
            process
                .stdin
                .take()
                .ok_or("the subflow process has no input")?,
        ));
        let stdout = process
            .stdout
            .take()
            .ok_or("the subflow process has no output")?;
        write_line(
            &stdin,
            &json!({"start": {
                "conversation_id": conversation_id,
                "root_conversation_id": root,
                "name": instance.name,
                "flow": instance.flow,
                "input": instance.input,
            }}),
        )
        .map_err(|error| format!("the subflow process takes no input: {error}"))?;
        self.children
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(instance.name.clone(), Arc::clone(&stdin));
        self.report(InstanceEvent::Started {
            name: instance.name.clone(),
        });
        let launcher = self.handle();
        let name = instance.name.clone();
        thread::spawn(move || {
            let mut outcome = None;
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if let Some(result) = launcher.handle_child_line(&stdin, &message) {
                    outcome = Some(result);
                }
            }
            let _ = process.wait();
            launcher
                .children
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&name);
            let _ = sender.send((
                index,
                outcome.unwrap_or_else(|| {
                    SubflowOutcome::Failed("the subflow process ended without a result".to_owned())
                }),
            ));
        });
        Ok(())
    }

    fn handle(&self) -> Arc<ProcessLauncher> {
        match ROOT.get() {
            Some(root) if std::ptr::eq(Arc::as_ptr(root), self) => Arc::clone(root),
            _ => Arc::clone(CHILD.get().expect("a child launcher is installed")),
        }
    }

    fn stop(&self, instances: &[SubflowInstanceRequest], running: &BTreeSet<usize>) {
        let children = self.children.lock().unwrap_or_else(PoisonError::into_inner);
        for index in running {
            if let Some(child) = children.get(&instances[*index].name) {
                let _ = write_line(child, &json!({"cancel": true}));
            }
        }
    }
}

static CHILD: OnceLock<Arc<ProcessLauncher>> = OnceLock::new();

fn write_line(child: &Mutex<ChildStdin>, message: &Value) -> io::Result<()> {
    let mut child = child.lock().unwrap_or_else(PoisonError::into_inner);
    writeln!(child, "{message}")?;
    child.flush()
}

pub(crate) fn subflow_run_command() -> Result<ExitCode, RuntimeError> {
    let mut input = BufReader::new(io::stdin());
    let mut first = String::new();
    input
        .read_line(&mut first)
        .map_err(|source| RuntimeError::Io {
            path: PathBuf::from("<stdin>"),
            source,
        })?;
    let start = serde_json::from_str::<Value>(&first)
        .ok()
        .and_then(|message| message.get("start").cloned())
        .ok_or_else(|| RuntimeError::Usage("flow subflow-run needs a start message".to_owned()))?;
    let field = |name: &str| start[name].as_str().unwrap_or_default().to_owned();
    let (name, flow, conversation_id) = (field("name"), field("flow"), field("conversation_id"));
    let root_input = serde_json::from_value::<core_script::FlowValue>(start["input"].clone())
        .map_err(|error| RuntimeError::Usage(format!("subflow input is invalid: {error}")))?;
    let launcher = Arc::clone(CHILD.get_or_init(|| {
        Arc::new(ProcessLauncher {
            upstream: Some(Upstream {
                out: Mutex::new(io::stdout()),
                next: AtomicU64::new(1),
                pending: Mutex::new(BTreeMap::new()),
            }),
            root_conversation: Mutex::new(Some(field("root_conversation_id"))),
            ..ProcessLauncher::default()
        })
    }));
    flow_agent_core::install_subflow_launcher(launcher.clone());
    let reader = Arc::clone(&launcher);
    let own_name = name.clone();
    thread::spawn(move || follow_parent(input, &reader, &own_name));

    let workspace = std::env::current_dir().map_err(|source| RuntimeError::Io {
        path: PathBuf::from("."),
        source,
    })?;
    let mut capture = ResultCapture::default();
    let result = stream_live_operation_into(
        workspace.clone(),
        move |notifier| {
            flow_agent_core::run_flow_with_execution_activation(
                &workspace,
                &flow,
                Some(&conversation_id),
                Some(root_input),
                Some(notifier),
                EmitMode::Jsonl,
                |productive| {
                    if productive {
                        flow_agent_core::begin_productive_operation()?;
                    }
                    Ok(SettleOnDrop(productive))
                },
            )
        },
        &mut capture,
    );
    let upstream = launcher
        .upstream
        .as_ref()
        .expect("a child launcher has a parent");
    let message = match (result, capture.result) {
        (Ok(output), Some(value)) if !output.failed => json!({"result": value}),
        (Ok(_), _) => {
            json!({"error": capture.error.unwrap_or_else(|| "the subflow Run failed".to_owned())})
        }
        (Err(error), _) => json!({"error": error.to_string()}),
    };
    upstream.send(&message).map_err(|source| RuntimeError::Io {
        path: PathBuf::from("<stdout>"),
        source,
    })?;
    Ok(ExitCode::SUCCESS)
}

fn follow_parent(input: impl BufRead, launcher: &ProcessLauncher, own_name: &str) {
    for line in input.lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(steer) = message.get("steer") {
            let text = steer["text"].as_str().unwrap_or_default().to_owned();
            if steer["target"] == own_name {
                let _ = flow_agent_core::steer_productive_run(text);
            } else {
                launcher.steer(steer["target"].as_str().unwrap_or_default(), &text);
            }
        } else if message.get("resume").is_some() {
            flow_agent_core::resume_productive_run();
            launcher.resume_all();
        } else if message.get("pause").is_some() {
            flow_agent_core::request_productive_pause();
            launcher.pause_all();
        } else if message.get("cancel").is_some() {
            flow_agent_core::request_productive_interrupt();
            launcher.cancel_all();
        } else if let Some(reply) = message.get("names") {
            let id = reply["id"].as_u64().unwrap_or_default();
            let answer = serde_json::from_value::<Vec<String>>(reply["names"].clone())
                .map_err(|_| reply["error"].as_str().unwrap_or("no names").to_owned());
            if let Some(upstream) = &launcher.upstream
                && let Some(sender) = upstream
                    .pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id)
            {
                let _ = sender.send(answer);
            }
        }
    }
    flow_agent_core::request_productive_interrupt();
    launcher.cancel_all();
}

struct SettleOnDrop(bool);

impl Drop for SettleOnDrop {
    fn drop(&mut self) {
        if self.0 {
            flow_agent_core::settle_productive_operation();
        }
    }
}

#[derive(Default)]
struct ResultCapture {
    line: Vec<u8>,
    result: Option<core_script::FlowValue>,
    error: Option<String>,
}

impl Write for ResultCapture {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        for byte in buffer {
            if *byte != b'\n' {
                self.line.push(*byte);
                continue;
            }
            let line = std::mem::take(&mut self.line);
            let Ok(event) = serde_json::from_slice::<Value>(&line) else {
                continue;
            };
            match event["event_type"].as_str() {
                Some("flow.completed") if event.get("parent_flow_id").is_none() => {
                    self.result = serde_json::from_value(event["payload"]["result"].clone()).ok();
                }
                Some("error") => {
                    self.error = event["payload"]["message"].as_str().map(str::to_owned);
                }
                _ => {}
            }
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
