//! Where the replica's database key lives: the operating system's keychain, through the
//! `keyring` crate (ADR-0237). The keychain files each key under a random name kept in a small
//! file in the replica's folder, so the key follows a moved folder (ADR-0275). On macOS the item
//! belongs to the app that created it, so the key is released only to Liquid; on Linux and
//! Windows any program of the user may read it (ADR-0254).

use std::io::{Read, Write};
use std::path::Path;

/// A place that holds replica keys, each filed under a name.
pub(crate) trait Keys {
    /// The key filed under `name`, or none if none was ever stored there.
    fn read(&self, name: &str) -> Result<Option<[u8; 32]>, String>;
    /// Files `key` under `name`.
    fn write(&self, name: &str, key: &[u8; 32]) -> Result<(), String>;
    /// Removes the key filed under `name`; removing none is not an error.
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// The operating system's keychain.
pub(crate) struct Keychain;

/// The keychain service Liquid files its replica keys under.
const SERVICE: &str = "Liquid replica key";

/// The file in a replica's folder that holds the name its key is filed under (ADR-0275).
pub(crate) const KEY_NAME_FILE: &str = "replica.keyname";

/// The name a replica's key was filed under before key names: its folder's canonical path. The
/// name must spell the folder exactly, or two folders could share one key.
pub(crate) fn path_name(folder: &Path) -> Result<String, String> {
    let folder = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    folder
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "the replica's folder path is not UTF-8".into())
}

/// The replica's key, found through the key name file in `dir`. A folder without that file, or
/// whose key is still filed under its path, takes its key under a new name once: the name file
/// is synced before the key is stored, and the path entry is deleted only after that. Without a
/// key, a replica whose database exists is refused (ADR-0237).
pub(crate) fn replica_key(
    dir: &Path,
    keys: &dyn Keys,
    has_database: bool,
) -> Result<[u8; 32], String> {
    let path = path_name(dir)?;
    let name = read_key_name(dir)?;
    let stored = match &name {
        Some(name) => keys.read(name)?,
        None => None,
    };
    let filed = keys.read(&path)?;
    let key = match (stored, filed) {
        (Some(key), _) | (None, Some(key)) => key,
        (None, None) if has_database => {
            return Err("the replica's key is missing from the keychain".into());
        }
        (None, None) => {
            let mut key = [0u8; 32];
            getrandom::fill(&mut key).map_err(|error| error.to_string())?;
            key
        }
    };
    if stored.is_none() {
        let name = match name {
            Some(name) => name,
            None => {
                let name = random_hex()?;
                write_key_name(dir, &name)?;
                name
            }
        };
        keys.write(&name, &key)?;
    }
    // Only the entry holding the key in use goes: a path entry of another key belongs to a
    // replica that moved away and may still need it.
    if filed == Some(key) {
        keys.delete(&path)?;
    }
    Ok(key)
}

/// The key name in `dir`'s key name file, or none if the folder has no such file. The file must be
/// a plain file, and at most 64 bytes of it are read.
fn read_key_name(dir: &Path) -> Result<Option<String>, String> {
    let path = dir.join(KEY_NAME_FILE);
    let failed =
        |error: std::io::Error| format!("the replica's key name {}: {error}", path.display());
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // Non-blocking, so a pipe in the file's place is refused instead of waited for.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(
        &mut options,
        rustix::fs::OFlags::NONBLOCK.bits() as i32,
    );
    let file = match options.open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failed(error)),
    };
    if !file.metadata().map_err(failed)?.is_file() {
        return Err(format!(
            "the replica's key name {} is no plain file",
            path.display()
        ));
    }
    let mut text = String::new();
    file.take(64).read_to_string(&mut text).map_err(failed)?;
    let name = text.trim();
    if name.len() != 32
        || !name
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(format!(
            "the replica's key name {} is malformed",
            path.display()
        ));
    }
    Ok(Some(name.to_owned()))
}

/// Replaces `dir`'s key name file with `name`. The new file is synced under a temporary name and
/// renamed over the old one, and the folder is synced after, so a crash leaves the old file or
/// the new one whole.
fn write_key_name(dir: &Path, name: &str) -> Result<(), String> {
    let path = dir.join(KEY_NAME_FILE);
    let temporary = dir.join(format!("{KEY_NAME_FILE}.tmp"));
    // Removed first, so a link left in its place is replaced rather than written through.
    let _ = std::fs::remove_file(&temporary);
    let failed =
        |error: std::io::Error| format!("the replica's key name {}: {error}", path.display());
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(failed)?;
    file.write_all(name.as_bytes()).map_err(failed)?;
    file.sync_all().map_err(failed)?;
    drop(file);
    std::fs::rename(&temporary, &path).map_err(failed)?;
    // std cannot sync a folder on Windows, so the rename is forced to disk on unix only.
    #[cfg(unix)]
    std::fs::File::open(dir)
        .and_then(|folder| folder.sync_all())
        .map_err(failed)?;
    Ok(())
}

/// 128 random bits as 32 lowercase hex digits.
pub(crate) fn random_hex() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl Keychain {
    fn entry(name: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, name).map_err(|error| error.to_string())
    }
}

impl Keys for Keychain {
    fn read(&self, name: &str) -> Result<Option<[u8; 32]>, String> {
        match Self::entry(name)?.get_secret() {
            Ok(secret) => secret
                .try_into()
                .map(Some)
                .map_err(|_| "the keychain holds a key of the wrong length".into()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn write(&self, name: &str, key: &[u8; 32]) -> Result<(), String> {
        Self::entry(name)?
            .set_secret(key)
            .map_err(|error| error.to_string())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        match Self::entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}
