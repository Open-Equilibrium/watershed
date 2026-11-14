//! The context window pi fills and the token count at which it compacts it (ADR-0279), as pi's
//! compaction reference defines them.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The context window the next request fills, and when pi compacts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    /// Tokens the window holds now, when pi knows them; `None` right after a compaction.
    pub tokens: Option<u64>,
    /// Tokens the model's context window holds.
    pub window: u64,
    /// Tokens at which pi starts compacting; `None` when auto compaction is off or pi's settings
    /// cannot be read.
    pub compacts_at: Option<u64>,
}

/// The reserve pi keeps for a response when no setting names one.
const RESERVE_TOKENS: u64 = 16_384;
/// Longest settings file read; pi's settings are a few lines.
const SETTINGS_BYTES: u64 = 1024 * 1024;

/// The directory of pi's global settings: `PI_CODING_AGENT_DIR`, else `~/.pi/agent`.
pub(crate) fn agent_dir() -> Option<PathBuf> {
    std::env::var_os("PI_CODING_AGENT_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            Some(
                PathBuf::from(std::env::var_os("HOME")?)
                    .join(".pi")
                    .join("agent"),
            )
        })
}

/// The token count at which pi starts compacting a window of `window` tokens: the window less the
/// reserve for the response. Pi reads `compaction` from the global settings in `global`, merged
/// with the project's `.pi/settings.json` in `project`; a model's override, keyed by its
/// `provider/id`, beats the ordinary reserve. `None` when a settings file cannot be read or
/// parsed, or holds a reserve pi refuses.
pub(crate) fn compacts_at(
    global: &Path,
    project: &Path,
    model: Option<&str>,
    window: u64,
) -> Option<u64> {
    let mut compaction = settings(&global.join("settings.json"))?;
    merge(
        &mut compaction,
        settings(&project.join(".pi").join("settings.json"))?,
    );
    let ordinary = tokens(compaction.get("reserveTokens"))?;
    let entry = model.and_then(|id| compaction.get("modelOverrides")?.get(id));
    let overridden = match entry {
        Some(entry) if !entry.is_object() => return None,
        entry => tokens(entry.and_then(|entry| entry.get("reserveTokens")))?,
    };
    let reserve = overridden.or(ordinary).unwrap_or(RESERVE_TOKENS);
    Some(window.saturating_sub(reserve))
}

/// The `compaction` object of a settings file: `Null` when there is no file or it has none.
fn settings(path: &Path) -> Option<Value> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(Value::Null),
        Err(_) => return None,
    };
    let mut text = Vec::new();
    file.take(SETTINGS_BYTES + 1).read_to_end(&mut text).ok()?;
    if text.len() as u64 > SETTINGS_BYTES {
        return None;
    }
    let mut settings: Value = serde_json::from_slice(&text).ok()?;
    Some(
        settings
            .get_mut("compaction")
            .map_or(Value::Null, Value::take),
    )
}

/// Merges `over` into `base` as pi merges its settings files: objects key by key, anything else
/// is replaced.
fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (key, value) in over {
                merge(base.entry(key).or_insert(Value::Null), value);
            }
        }
        (base, over) => *base = over,
    }
}

/// A token setting: `Some(None)` when it is omitted, `None` when it is not a whole number.
fn tokens(value: Option<&Value>) -> Option<Option<u64>> {
    value.map_or(Some(None), |value| value.as_u64().map(Some))
}
