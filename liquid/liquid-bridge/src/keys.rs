use std::io::{Read, Write};
use std::path::Path;

pub(crate) trait Keys {
    fn read(&self, name: &str) -> Result<Option<[u8; 32]>, String>;
    fn write(&self, name: &str, key: &[u8; 32]) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}

pub(crate) struct Keychain;

const SERVICE: &str = "Liquid replica key";

const TOKEN_SERVICE: &str = "Liquid agent host token";

pub(crate) const KEY_NAME_FILE: &str = "replica.keyname";

pub(crate) fn path_name(folder: &Path) -> Result<String, String> {
    let folder = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    folder
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "the replica's folder path is not UTF-8".into())
}

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
    let filed = match stored {
        Some(_) => keys.read(&path).unwrap_or(None),
        None => keys.read(&path)?,
    };
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
    if filed == Some(key) {
        keys.delete(&path)?;
    }
    Ok(key)
}

fn read_key_name(dir: &Path) -> Result<Option<String>, String> {
    let path = dir.join(KEY_NAME_FILE);
    let failed =
        |error: std::io::Error| format!("the replica's key name {}: {error}", path.display());
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
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

fn write_key_name(dir: &Path, name: &str) -> Result<(), String> {
    let path = dir.join(KEY_NAME_FILE);
    let temporary = dir.join(format!("{KEY_NAME_FILE}.tmp"));
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
    #[cfg(unix)]
    std::fs::File::open(dir)
        .and_then(|folder| folder.sync_all())
        .map_err(failed)?;
    Ok(())
}

pub(crate) fn random_hex() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) trait Tokens {
    fn read(&self, address: &str) -> Result<Option<String>, String>;
    fn write(&self, address: &str, token: &str) -> Result<(), String>;
    fn delete(&self, address: &str) -> Result<(), String>;
}

impl Keychain {
    fn entry(service: &str, name: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(service, name).map_err(|error| error.to_string())
    }
}

impl Keys for Keychain {
    fn read(&self, name: &str) -> Result<Option<[u8; 32]>, String> {
        match Self::entry(SERVICE, name)?.get_secret() {
            Ok(secret) => secret
                .try_into()
                .map(Some)
                .map_err(|_| "the keychain holds a key of the wrong length".into()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn write(&self, name: &str, key: &[u8; 32]) -> Result<(), String> {
        Self::entry(SERVICE, name)?
            .set_secret(key)
            .map_err(|error| error.to_string())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        match Self::entry(SERVICE, name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

pub(crate) struct DeviceTokens {}

impl Tokens for DeviceTokens {
    fn read(&self, address: &str) -> Result<Option<String>, String> {
        match Keychain::entry(TOKEN_SERVICE, address)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn write(&self, address: &str, token: &str) -> Result<(), String> {
        Keychain::entry(TOKEN_SERVICE, address)?
            .set_password(token)
            .map_err(|error| error.to_string())
    }

    fn delete(&self, address: &str) -> Result<(), String> {
        match Keychain::entry(TOKEN_SERVICE, address)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}
