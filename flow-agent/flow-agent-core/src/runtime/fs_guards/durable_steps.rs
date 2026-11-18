use super::test_path_key;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{FileExt as _, MetadataExt as _},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

pub(crate) const CRASH_EXIT_CODE: i32 = 113;

static TRACES: Mutex<Vec<ActiveTrace>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, Debug)]
pub(crate) enum PowerLoss {
    Drop,
    CutLastWrite,
}

struct ActiveTrace {
    crash: Option<CrashPoint>,
    ledger: Ledger,
    power: bool,
    root: PathBuf,
    steps: Vec<String>,
}

#[derive(Default)]
struct Ledger {
    files: BTreeMap<(u64, u64), LedgerFile>,
    last_write: Option<LastWrite>,
}

struct LedgerFile {
    file: fs::File,
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

type CrashRecorder = Box<dyn FnOnce(&[String]) + Send>;

pub(crate) struct CrashPoint {
    pub(crate) index: usize,
    pub(crate) on_crash: CrashRecorder,
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

pub(crate) fn finish_durable_trace(root: &Path) -> Vec<String> {
    let root = test_path_key(root);
    let mut traces = lock_traces();
    let index = traces
        .iter()
        .position(|trace| trace.root == root)
        .expect("durable trace is active for its root");
    traces.swap_remove(index).steps
}

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

pub(crate) fn record_truncate(path: &Path, file: &fs::File, len: u64) {
    let metadata = file.metadata().expect("truncated file has metadata");
    let key = (metadata.dev(), metadata.ino());
    let current = metadata.len();
    with_ledger(path, |ledger| {
        let entry = ledger.files.entry(key).or_insert_with(|| LedgerFile {
            file: file.try_clone().expect("truncated file duplicates"),
            lost_tail: None,
            synced_len: current,
        });
        let lost_until = entry
            .lost_tail
            .as_ref()
            .map_or(entry.synced_len, |tail| tail.offset);
        if len < lost_until {
            let mut bytes = vec![0; lost_until.min(current).saturating_sub(len) as usize];
            fs::File::open(path)
                .and_then(|readable| readable.read_exact_at(&mut bytes, len))
                .expect("synced bytes read before truncation");
            if let Some(tail) = entry.lost_tail.take() {
                bytes.extend(tail.bytes);
            }
            entry.lost_tail = Some(LostTail { offset: len, bytes });
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

fn find_trace<'a>(traces: &'a mut [ActiveTrace], path: &Path) -> Option<&'a mut ActiveTrace> {
    let key = test_path_key(path);
    let trace = traces
        .iter_mut()
        .find(|trace| key.starts_with(&trace.root))?;
    let scratch = key
        .strip_prefix(&trace.root)
        .is_ok_and(|relative| relative.starts_with("scratch"));
    (!scratch).then_some(trace)
}

fn lock_traces() -> MutexGuard<'static, Vec<ActiveTrace>> {
    TRACES.lock().expect("durable trace seam is not poisoned")
}
