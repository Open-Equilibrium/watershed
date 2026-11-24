use std::ops::Deref;
use std::path::{Path, PathBuf};

pub(crate) struct Dir(PathBuf);

impl Dir {
    pub(crate) fn new() -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let dir = std::env::temp_dir().join(format!("mh-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Deref for Dir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn put(path: impl AsRef<Path>, text: impl AsRef<[u8]>) {
    let path = path.as_ref();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

pub(crate) fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[cfg(unix)]
pub(crate) fn executable(path: impl AsRef<Path>, script: &str) {
    let path = path.as_ref();
    put(path, script);
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}
