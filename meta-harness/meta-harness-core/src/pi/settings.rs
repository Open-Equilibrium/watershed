//! pi's settings that decide when it compacts its context (ADR-0279), as pi reads them.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::stored;

/// The reserve pi keeps for a response when no setting names one.
const RESERVE_TOKENS: u64 = 16_384;
/// Longest settings file read; pi's settings are a few lines.
const SETTINGS_BYTES: u64 = 1024 * 1024;
/// The byte order mark pi strips from the start of a settings file.
const BYTE_ORDER_MARK: &[u8] = "\u{feff}".as_bytes();

/// The directory of pi's global settings: `PI_CODING_AGENT_DIR`, else `~/.pi/agent`.
pub(crate) fn agent_dir() -> Option<PathBuf> {
    agent_dir_from(
        std::env::var_os("PI_CODING_AGENT_DIR"),
        std::env::var_os("HOME"),
    )
}

/// [`agent_dir`] for the variable `dir` and the home folder `home` as they are set. A leading `~`
/// in `dir` stands for the home folder, as pi expands it.
pub(crate) fn agent_dir_from(dir: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    match dir.filter(|dir| !dir.is_empty()) {
        Some(dir) => match Path::new(&dir).strip_prefix("~") {
            Ok(rest) => Some(PathBuf::from(home?).join(rest)),
            Err(_) => Some(PathBuf::from(dir)),
        },
        None => Some(PathBuf::from(home?).join(".pi").join("agent")),
    }
}

/// The token count at which pi starts compacting a window of `window` tokens: the window less the
/// reserve for the response. Pi reads `compaction` from the global settings in `global`, and a
/// model's override, keyed by its `provider/id`, beats the ordinary reserve. Pi reads the project's
/// `.pi/settings.json` in `project` only once it trusts the project, and whether it does depends on
/// extensions Watershed cannot see, so a project file that sets `compaction` gives `None`. `None`
/// also when a settings file cannot be read or parsed, or holds a reserve pi refuses.
pub(crate) fn compacts_at(
    global: &Path,
    project: &Path,
    model: Option<&str>,
    window: u64,
) -> Option<u64> {
    if settings(&project.join(".pi").join("settings.json"))?
        .get("compaction")
        .is_some()
    {
        return None;
    }
    let global = settings(&global.join("settings.json"))?;
    let compaction = global.get("compaction");
    let ordinary = tokens(compaction.and_then(|compaction| compaction.get("reserveTokens")))?;
    let entry = model.and_then(|id| compaction?.get("modelOverrides")?.get(id));
    let overridden = match entry {
        Some(entry) if !entry.is_object() => return None,
        entry => tokens(entry.and_then(|entry| entry.get("reserveTokens")))?,
    };
    let reserve = overridden.or(ordinary).unwrap_or(RESERVE_TOKENS);
    Some(window.saturating_sub(reserve))
}

/// A settings file as JSON, without the byte order mark pi strips; `Null` when there is no file.
/// It is opened as pi's session files are, so a pipe or device is refused without waiting.
fn settings(path: &Path) -> Option<Value> {
    let file = match stored::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(Value::Null),
        Err(_) => return None,
    };
    let mut text = Vec::new();
    file.take(SETTINGS_BYTES + 1).read_to_end(&mut text).ok()?;
    if text.len() as u64 > SETTINGS_BYTES {
        return None;
    }
    serde_json::from_slice(text.strip_prefix(BYTE_ORDER_MARK).unwrap_or(&text)).ok()
}

/// A token setting: `Some(None)` when it is omitted, `None` when it is not a whole number.
fn tokens(value: Option<&Value>) -> Option<Option<u64>> {
    value.map_or(Some(None), |value| value.as_u64().map(Some))
}
