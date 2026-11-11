//! Test-only trace of state-changing filesystem steps for crash injection.
//!
//! A traced operation reports every create, rename, removal, directory creation, link and write
//! under its store root in order. Syncs are not steps: a dying process keeps everything already
//! written, so a sync never changes what survives a crash. A crash point ends the process before
//! its step runs, so the surviving store keeps the real inodes and directory identities.

use super::test_path_key;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Exit status of a process that dies at its crash point.
pub(crate) const CRASH_EXIT_CODE: i32 = 113;

static TRACES: Mutex<Vec<ActiveTrace>> = Mutex::new(Vec::new());

struct ActiveTrace {
    crash: Option<CrashPoint>,
    root: PathBuf,
    steps: Vec<String>,
}

/// Records the durable steps reached so far when the process is about to die.
type CrashRecorder = Box<dyn FnOnce(&[String]) + Send>;

/// Ends the process at durable step `index`, after `on_crash` records the steps reached so far.
pub(crate) struct CrashPoint {
    pub(crate) index: usize,
    pub(crate) on_crash: CrashRecorder,
}

pub(crate) fn start_durable_trace(root: &Path, crash: Option<CrashPoint>) {
    TRACES
        .lock()
        .expect("durable trace seam is not poisoned")
        .push(ActiveTrace {
            crash,
            root: test_path_key(root),
            steps: Vec::new(),
        });
}

/// Stops the trace under `root` and returns its ordered steps.
pub(crate) fn finish_durable_trace(root: &Path) -> Vec<String> {
    let root = test_path_key(root);
    let mut traces = TRACES.lock().expect("durable trace seam is not poisoned");
    let index = traces
        .iter()
        .position(|trace| trace.root == root)
        .expect("durable trace is active for its root");
    traces.swap_remove(index).steps
}

/// Records one state-changing step under an active trace root.
pub(crate) fn durable_step(path: &Path, operation: &str) {
    let crash = {
        let mut traces = TRACES.lock().expect("durable trace seam is not poisoned");
        if traces.is_empty() {
            return;
        }
        let key = test_path_key(path);
        let Some(trace) = traces.iter_mut().find(|trace| key.starts_with(&trace.root)) else {
            return;
        };
        // WHY: the whole-history validation scratch is a private per-command index, never a durable
        // Conversation format (PROTOCOL.md), so its steps do not change recoverable state.
        if key
            .strip_prefix(&trace.root)
            .is_ok_and(|relative| relative.starts_with("scratch"))
        {
            return;
        }
        let relative = key
            .strip_prefix(&trace.root)
            .expect("traced step is under its root");
        trace
            .steps
            .push(format!("{operation} {}", relative.display()));
        if trace
            .crash
            .as_ref()
            .is_some_and(|crash| crash.index + 1 == trace.steps.len())
        {
            trace
                .crash
                .take()
                .map(|crash| (crash.on_crash, trace.steps.clone()))
        } else {
            None
        }
    };
    if let Some((on_crash, steps)) = crash {
        on_crash(&steps);
        std::process::exit(CRASH_EXIT_CODE);
    }
}
