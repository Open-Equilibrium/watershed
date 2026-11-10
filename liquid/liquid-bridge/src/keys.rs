//! Where the replica's database key lives: the operating system's keychain, through the
//! `keyring` crate (ADR-0237). On macOS the item belongs to the app that created it, so the key
//! is released only to Liquid; on Linux and Windows any program of the user may read it
//! (ADR-0254).

use std::path::Path;

/// A place that holds one replica's 256-bit key.
pub(crate) trait Keys {
    /// The key, or none if it was never stored.
    fn read(&self) -> Result<Option<[u8; 32]>, String>;
    /// Stores the key.
    fn write(&self, key: &[u8; 32]) -> Result<(), String>;
}

/// The keychain entry of the replica in one folder.
pub(crate) struct Keychain {
    folder: String,
}

/// The keychain service Liquid files its replica keys under.
const SERVICE: &str = "Liquid replica key";

impl Keychain {
    pub(crate) fn for_replica(folder: &Path) -> Result<Self, String> {
        let folder = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
        // The account must spell the folder exactly, or two folders could share one key.
        let folder = folder
            .to_str()
            .ok_or("the replica's folder path is not UTF-8")?
            .to_owned();
        Ok(Self { folder })
    }

    fn entry(&self) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, &self.folder).map_err(|error| error.to_string())
    }
}

impl Keys for Keychain {
    fn read(&self) -> Result<Option<[u8; 32]>, String> {
        match self.entry()?.get_secret() {
            Ok(secret) => secret
                .try_into()
                .map(Some)
                .map_err(|_| "the keychain holds a key of the wrong length".into()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn write(&self, key: &[u8; 32]) -> Result<(), String> {
        self.entry()?
            .set_secret(key)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
impl Keychain {
    pub(crate) fn delete(&self) -> Result<(), String> {
        self.entry()?
            .delete_credential()
            .map_err(|error| error.to_string())
    }
}
