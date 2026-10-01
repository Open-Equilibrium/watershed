use std::{
    env,
    fs::{File, OpenOptions},
    io,
    os::unix::fs::OpenOptionsExt as _,
    path::{Path, PathBuf},
};

pub(crate) fn stage_controller(directory: &Path) -> io::Result<PathBuf> {
    let path = directory.join("flow");
    let mut source = File::open(env::current_exe()?)?;
    // Cargo may hardlink its example outputs; the installed controller must not
    // retain an alias outside this synthetic installation.
    let mut target = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(&path)?;
    io::copy(&mut source, &mut target)?;
    Ok(path)
}
