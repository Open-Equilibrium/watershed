use std::path::{Path, PathBuf};

const NOTICES: [&str; 3] = ["LICENSE", "LICENSE-t3code", "THIRD_PARTY_NOTICES.md"];
const MOVE_TRIES: u32 = 60;
const MOVE_WAIT: std::time::Duration = std::time::Duration::from_millis(500);

pub fn bundle_info(bundle: &Path) -> Result<(String, String), String> {
    let text = std::fs::read_to_string(bundle.join("bundle-info")).map_err(|_| {
        "missing bundle-info; run meta install from an unpacked download".to_owned()
    })?;
    match text.lines().collect::<Vec<_>>()[..] {
        [version, target] if !version.is_empty() && !target.is_empty() => {
            Ok((version.to_owned(), target.to_owned()))
        }
        _ => Err("invalid bundle-info".to_owned()),
    }
}

pub fn install_files(bundle: &Path, prefix: &Path, platform: &str) -> Result<PathBuf, String> {
    let (_, target) = bundle_info(bundle)?;
    if target != platform {
        return Err(format!("download is for {target}, not {platform}"));
    }
    let meta = format!("meta{}", std::env::consts::EXE_SUFFIX);
    let source_lib = bundle.join("lib").join("meta-harness");
    let t3: Vec<PathBuf> = std::fs::read_dir(&source_lib)
        .map_err(|_| format!("download lacks lib/meta-harness for {platform}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(&format!("-{platform}")))
        })
        .collect();
    let [t3_source] = &t3[..] else {
        return Err(format!("download lacks one T3 Code folder for {platform}"));
    };
    for needed in [bundle.join("bin").join(&meta)]
        .into_iter()
        .chain(NOTICES.iter().map(|name| bundle.join(name)))
    {
        if !needed.is_file() {
            return Err(format!("download lacks {}", needed.display()));
        }
    }
    let lib = prefix.join("lib").join("meta-harness");
    let bin = prefix.join("bin");
    let share = prefix.join("share").join("meta-harness");
    for folder in [&lib, &bin, &share] {
        std::fs::create_dir_all(folder)
            .map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
    }
    let lock_file = lib.join(".install.lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_file)
        .map_err(|e| format!("Could not open {}: {e}", lock_file.display()))?;
    lock.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => format!(
            "Another Meta-Harness installation runs in {}; wait for it to finish, then install again.",
            prefix.display()
        ),
        std::fs::TryLockError::Error(error) => {
            format!("Could not lock {}: {error}", lock_file.display())
        }
    })?;
    let name = t3_source.file_name().unwrap_or_default();
    let stage = lib.join(format!(".{}.install", name.to_string_lossy()));
    let _ = std::fs::remove_dir_all(&stage);
    copy_tree(t3_source, &stage)?;
    let program = bin.join(&meta);
    let previous = bin.join(format!("{meta}.old"));
    let kept = program.exists();
    if kept {
        let _ = std::fs::remove_file(&previous);
        std::fs::rename(&program, &previous).map_err(|e| {
            let _ = std::fs::remove_dir_all(&stage);
            format!(
                "Could not move the running {} aside: {e}",
                program.display()
            )
        })?;
    }
    let restore = |error: String| {
        let _ = std::fs::remove_dir_all(&stage);
        if kept {
            let _ = std::fs::remove_file(&program);
            let _ = std::fs::rename(&previous, &program);
        }
        error
    };
    std::fs::copy(bundle.join("bin").join(&meta), &program)
        .map_err(|e| restore(format!("Could not install {}: {e}", program.display())))?;
    let installed = lib.join(name);
    let aside = lib.join(format!(".{}.old", name.to_string_lossy()));
    let _ = std::fs::remove_dir_all(&aside);
    if installed.exists() {
        let mut tries = 0;
        while let Err(error) = std::fs::rename(&installed, &aside) {
            tries += 1;
            if tries >= MOVE_TRIES {
                return Err(restore(format!(
                    "The running Meta-Harness still holds {} ({error}), so nothing was \
                     replaced. Stop it with `meta service uninstall` and install again.",
                    installed.display()
                )));
            }
            std::thread::sleep(MOVE_WAIT);
        }
    }
    if let Err(error) = std::fs::rename(&stage, &installed) {
        let _ = std::fs::rename(&aside, &installed);
        return Err(restore(format!(
            "Could not place {}: {error}",
            installed.display()
        )));
    }
    for notice in NOTICES {
        std::fs::copy(bundle.join(notice), share.join(notice))
            .map_err(|e| format!("Could not install {notice}: {e}"))?;
    }
    for old in std::fs::read_dir(&lib)
        .map_err(|e| format!("Could not read {}: {e}", lib.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
    {
        let leftover = old.file_name().is_some_and(|n| {
            let n = n.to_string_lossy();
            n.starts_with("t3-") || n.starts_with(".t3-")
        });
        if leftover && old != installed {
            let _ = std::fs::remove_dir_all(&old);
        }
    }
    Ok(program)
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("Could not create {}: {e}", to.display()))?;
    for entry in
        std::fs::read_dir(from).map_err(|e| format!("Could not read {}: {e}", from.display()))?
    {
        let entry = entry.map_err(|e| format!("Could not read {}: {e}", from.display()))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("Could not read {}: {e}", entry.path().display()))?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)
                .map_err(|e| format!("Could not copy {}: {e}", entry.path().display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_installs_meta_t3_and_notices_and_replaces_an_older_t3() {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let root = std::env::temp_dir().join(format!(
            "mh-install-{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        let bundle = root.join("meta-harness-1.0.0-test-x64");
        let meta = format!("meta{}", std::env::consts::EXE_SUFFIX);
        let t3 = bundle.join("lib/meta-harness/t3-0.1.11-test-x64");
        std::fs::create_dir_all(t3.join("client")).unwrap();
        std::fs::write(t3.join("client/index.html"), "t3").unwrap();
        std::fs::create_dir_all(bundle.join("bin")).unwrap();
        std::fs::write(bundle.join("bin").join(&meta), "new meta").unwrap();
        for notice in NOTICES {
            std::fs::write(bundle.join(notice), notice).unwrap();
        }
        std::fs::write(bundle.join("bundle-info"), "1.0.0\ntest-x64\n").unwrap();
        let prefix = root.join("prefix");
        std::fs::create_dir_all(prefix.join("lib/meta-harness/t3-0.1.9-test-x64")).unwrap();
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::write(prefix.join("bin").join(&meta), "old meta").unwrap();

        assert!(
            install_files(&bundle, &prefix, "other-x64")
                .unwrap_err()
                .contains("not other-x64")
        );
        let lock = std::fs::File::create(prefix.join("lib/meta-harness/.install.lock")).unwrap();
        lock.lock().unwrap();
        let busy = install_files(&bundle, &prefix, "test-x64").unwrap_err();
        assert!(busy.contains("Another"), "{busy}");
        assert_eq!(
            std::fs::read_to_string(prefix.join("bin/meta")).unwrap(),
            "old meta"
        );
        drop(lock);
        let program = install_files(&bundle, &prefix, "test-x64").unwrap();

        assert_eq!(std::fs::read_to_string(&program).unwrap(), "new meta");
        assert_eq!(
            std::fs::read_to_string(
                prefix.join("lib/meta-harness/t3-0.1.11-test-x64/client/index.html")
            )
            .unwrap(),
            "t3"
        );
        assert!(!prefix.join("lib/meta-harness/t3-0.1.9-test-x64").exists());
        assert!(
            prefix
                .join("share/meta-harness/THIRD_PARTY_NOTICES.md")
                .is_file()
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
