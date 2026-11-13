//! Test-only trace of state-changing filesystem steps for crash injection.
//!
//! A traced operation reports every create, rename, removal, directory creation, link, write and
//! truncation under its store root in order. A process-death trace records no file syncs: a dying
//! process keeps everything already written, so a sync never changes what survives it. A
//! power-loss trace also records each file sync, and keeps a ledger of the bytes every file has
//! synced, so a crash point can drop the unsynced bytes of every file (ADR-0266). A crash point
//! ends the process before its step runs, so the surviving store keeps the real inodes and
//! directory identities.

use super::test_path_key;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{FileExt as _, MetadataExt as _},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

/// Exit status of a process that dies at its crash point.
pub(crate) const CRASH_EXIT_CODE: i32 = 113;

static TRACES: Mutex<Vec<ActiveTrace>> = Mutex::new(Vec::new());

/// Which unsynced bytes a power loss takes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PowerLoss {
    /// Every file keeps exactly the bytes its last sync covered.
    Drop,
    /// As `Drop`, except that an unsynced last write keeps all its bytes but the final one, so a
    /// JSONL record survives without its line feed.
    CutLastWrite,
}

struct ActiveTrace {
    crash: Option<CrashPoint>,
    ledger: Ledger,
    power: bool,
    root: PathBuf,
    steps: Vec<String>,
}

/// Bytes written and synced per file, keyed by (device, inode) so renames and links keep identity.
#[derive(Default)]
struct Ledger {
    files: BTreeMap<(u64, u64), LedgerFile>,
    last_write: Option<LastWrite>,
}

struct LedgerFile {
    /// A duplicate handle of the traced file, kept to take unsynced bytes away at a power loss.
    file: fs::File,
    /// Synced bytes a truncation removed, from its offset up to `synced_len`.
    lost_tail: Option<LostTail>,
    synced_len: u64,
}

struct LostTail {
    offset: u64,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
struct LastWrite {
    key: (u64, u64),
    end: u64,
}

impl Ledger {
    fn apply(&self, loss: PowerLoss) {
        for (key, entry) in &self.files {
            if let Some(tail) = &entry.lost_tail {
                entry
                    .file
                    .write_all_at(&tail.bytes, tail.offset)
                    .expect("power loss restores an unsynced truncation");
            }
            let mut keep = entry.synced_len;
            if let Some(last) = self.last_write.filter(|last| {
                matches!(loss, PowerLoss::CutLastWrite) && last.key == *key && last.end > keep
            }) {
                keep = keep.max(last.end - 1);
            }
            let len = entry
                .file
                .metadata()
                .expect("ledgered file has metadata")
                .len();
            if len > keep {
                entry
                    .file
                    .set_len(keep)
                    .expect("power loss truncates an unsynced file");
            }
        }
    }
}

/// Records the durable steps reached so far when the process is about to die.
type CrashRecorder = Box<dyn FnOnce(&[String]) + Send>;

/// Ends the process at durable step `index`, after `on_crash` records the steps reached so far.
pub(crate) struct CrashPoint {
    pub(crate) index: usize,
    pub(crate) on_crash: CrashRecorder,
    /// The power loss applied before `on_crash`; `None` for a process death.
    pub(crate) power: Option<PowerLoss>,
}

pub(crate) fn start_durable_trace(root: &Path, crash: Option<CrashPoint>) {
    start_trace(root, crash, false);
}

pub(crate) fn start_power_loss_trace(root: &Path, crash: Option<CrashPoint>) {
    start_trace(root, crash, true);
}

fn start_trace(root: &Path, crash: Option<CrashPoint>, power: bool) {
    lock_traces().push(ActiveTrace {
        crash,
        ledger: Ledger::default(),
        power,
        root: test_path_key(root),
        steps: Vec::new(),
    });
}

/// Stops the trace under `root` and returns its ordered steps.
pub(crate) fn finish_durable_trace(root: &Path) -> Vec<String> {
    let root = test_path_key(root);
    let mut traces = lock_traces();
    let index = traces
        .iter()
        .position(|trace| trace.root == root)
        .expect("durable trace is active for its root");
    traces.swap_remove(index).steps
}

/// Applies `loss` to the files of the active trace under `root`, as a power cut would.
#[cfg(test)]
pub(crate) fn apply_power_loss(root: &Path, loss: PowerLoss) {
    let root = test_path_key(root);
    let traces = lock_traces();
    let trace = traces
        .iter()
        .find(|trace| trace.root == root)
        .expect("power loss targets an active trace");
    trace.ledger.apply(loss);
}

/// Records one state-changing step under an active trace root.
pub(crate) fn durable_step(path: &Path, operation: &str) {
    let crash = {
        let mut traces = lock_traces();
        let Some(trace) = find_trace(&mut traces, path) else {
            return;
        };
        if operation == "sync" && !trace.power {
            return;
        }
        let relative = test_path_key(path);
        let relative = relative
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
            let crash = trace.crash.take().expect("crash point is armed");
            if let Some(loss) = crash.power {
                trace.ledger.apply(loss);
            }
            Some((crash.on_crash, trace.steps.clone()))
        } else {
            None
        }
    };
    if let Some((on_crash, steps)) = crash {
        on_crash(&steps);
        std::process::exit(CRASH_EXIT_CODE);
    }
}

/// Records `written` bytes just written to `file` at `path`.
///
/// A file the ledger has not seen before counts as synced up to the length before this write.
pub(crate) fn record_write(path: &Path, file: &fs::File, written: u64) {
    let metadata = file.metadata().expect("written file has metadata");
    let end = metadata.len();
    let key = (metadata.dev(), metadata.ino());
    with_ledger(path, |ledger| {
        ledger.files.entry(key).or_insert_with(|| LedgerFile {
            file: file.try_clone().expect("written file duplicates"),
            lost_tail: None,
            synced_len: end.saturating_sub(written),
        });
        ledger.last_write = Some(LastWrite { key, end });
    });
}

/// Records that every byte of `file` at `path` is synced.
pub(crate) fn record_sync(path: &Path, file: &fs::File) {
    let metadata = file.metadata().expect("synced file has metadata");
    let key = (metadata.dev(), metadata.ino());
    let len = metadata.len();
    with_ledger(path, |ledger| {
        if let Some(entry) = ledger.files.get_mut(&key) {
            entry.synced_len = len;
            entry.lost_tail = None;
        }
    });
}

/// Records that `file` at `path` is about to shrink to `len` bytes, before the shrink. The synced
/// bytes it removes are kept, so a power loss restores them: the truncation is durable only at a sync.
pub(crate) fn record_truncate(path: &Path, file: &fs::File, len: u64) {
    let metadata = file.metadata().expect("truncated file has metadata");
    let key = (metadata.dev(), metadata.ino());
    let current = metadata.len();
    with_ledger(path, |ledger| {
        if let Some(entry) = ledger.files.get_mut(&key) {
            let lost_until = entry
                .lost_tail
                .as_ref()
                .map_or(entry.synced_len, |tail| tail.offset);
            if len < lost_until {
                let mut bytes = vec![0; lost_until.min(current).saturating_sub(len) as usize];
                // WHY: the traced handle is write-only, and reading it by path leaves its offset alone.
                fs::File::open(path)
                    .and_then(|readable| readable.read_exact_at(&mut bytes, len))
                    .expect("synced bytes read before truncation");
                if let Some(tail) = entry.lost_tail.take() {
                    bytes.extend(tail.bytes);
                }
                entry.lost_tail = Some(LostTail { offset: len, bytes });
            }
        }
        if let Some(last) = ledger.last_write.as_mut().filter(|last| last.key == key) {
            last.end = last.end.min(len);
        }
    });
}

fn with_ledger(path: &Path, record: impl FnOnce(&mut Ledger)) {
    let mut traces = lock_traces();
    if let Some(trace) = find_trace(&mut traces, path) {
        record(&mut trace.ledger);
    }
}

/// The active trace that records steps on `path`. Scratch paths are not durable steps.
fn find_trace<'a>(traces: &'a mut [ActiveTrace], path: &Path) -> Option<&'a mut ActiveTrace> {
    let key = test_path_key(path);
    let trace = traces
        .iter_mut()
        .find(|trace| key.starts_with(&trace.root))?;
    // WHY: the whole-history validation scratch is a private per-command index, never a durable
    // Conversation format (PROTOCOL.md), so its steps do not change recoverable state.
    let scratch = key
        .strip_prefix(&trace.root)
        .is_ok_and(|relative| relative.starts_with("scratch"));
    (!scratch).then_some(trace)
}

fn lock_traces() -> MutexGuard<'static, Vec<ActiveTrace>> {
    TRACES.lock().expect("durable trace seam is not poisoned")
}
