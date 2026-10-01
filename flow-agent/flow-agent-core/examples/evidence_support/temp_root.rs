use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

type DynError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) struct TempRoot(PathBuf);

impl TempRoot {
    pub(crate) fn create(prefix: &str) -> Result<Self, DynError> {
        Self::create_in(&env::temp_dir(), prefix)
    }

    pub(crate) fn create_in(base: &Path, prefix: &str) -> Result<Self, DynError> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = fs::canonicalize(base)?.join(format!("{prefix}-{}-{nonce}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
