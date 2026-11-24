use std::path::{Path, PathBuf};

use meta_harness_core::{default_home, t3_version};

use crate::login;
use crate::{USAGE, ignore_missing};

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

pub fn install_files(
    bundle: &Path,
    prefix: &Path,
    platform: &str,
    homes: &[PathBuf],
) -> Result<PathBuf, String> {
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
    if let Some(data) = data_in_installation(prefix, homes) {
        return Err(format!(
            "The data folder {} lies in the installation that an upgrade replaces, so nothing was \
             changed. Move it outside {}, set META_HARNESS_HOME to its new place and install again.",
            data.display(),
            prefix.display()
        ));
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

fn prefix() -> Result<PathBuf, String> {
    let program = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| format!("Could not find this program: {e}"))?;
    program
        .parent()
        .filter(|bin| bin.ends_with("bin"))
        .and_then(Path::parent)
        .filter(|prefix| prefix.join("lib/meta-harness").is_dir())
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{} is not an installed meta.", program.display()))
}

pub(crate) fn installed_t3() -> Result<PathBuf, String> {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("windows", "x86_64") => "win32-x64",
        (os, arch) => return Err(format!("Meta-Harness has no T3 Code for {os} {arch}.")),
    };
    let prefix = prefix().map_err(|e| {
        format!(
            "{e} For a build that is not installed, set META_HARNESS_T3 to its T3 Code program."
        )
    })?;
    Ok(prefix
        .join("lib/meta-harness")
        .join(format!("t3-{}-{platform}", t3_version()))
        .join(format!("t3{}", std::env::consts::EXE_SUFFIX)))
}

pub(crate) fn install_download(args: &[&str]) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("On macOS and Linux, run install.sh from the download.".to_owned());
    }
    let (mut prefix, mut service) = (None, true);
    let mut rest = args;
    while let Some((flag, more)) = rest.split_first() {
        rest = match (*flag, more) {
            ("--prefix", [path, after @ ..]) => {
                prefix = Some(PathBuf::from(path));
                after
            }
            ("--no-service", after) => {
                service = false;
                after
            }
            _ => return Err(format!("meta install does not take {flag}\n{USAGE}")),
        };
    }
    let prefix = match prefix {
        Some(prefix) => prefix,
        None => std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or("LOCALAPPDATA is not set; pass --prefix.")?
            .join("Programs")
            .join("Meta-Harness"),
    };
    let bundle = std::env::current_exe()
        .map_err(|e| format!("Could not find this program: {e}"))?
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or("meta install runs from the bin folder of an unpacked download.")?;
    let task = |step: &str| {
        std::process::Command::new("schtasks")
            .args([step, "/TN", "Meta-Harness"])
            .output()
            .is_ok_and(|output| output.status.success())
    };
    let homes: Vec<PathBuf> = std::iter::once(default_home()?)
        .chain(login::Manager::current()?.recorded_home()?)
        .collect();
    let ended = task("/End");
    let installed = install_files(&bundle, &prefix, "win32-x64", &homes);
    if ended && (installed.is_err() || !service) {
        task("/Run");
    }
    let program = installed?;
    if service {
        let status = std::process::Command::new(&program)
            .args(["service", "install"])
            .status()
            .map_err(|e| format!("Could not run {}: {e}", program.display()))?;
        if !status.success() {
            return Err(format!(
                "the files are installed in {}, but the login service did not start; \
                 --no-service installs without it",
                prefix.display()
            ));
        }
    }
    println!(
        "Installed Meta-Harness {} in {}",
        bundle_info(&bundle)?.0,
        prefix.display()
    );
    println!(
        "Add {} to your PATH to run meta from any folder.",
        prefix.join("bin").display()
    );
    Ok(())
}

pub(crate) fn install_service(home: PathBuf) -> Result<(), String> {
    let program = prefix()?
        .join("bin")
        .join(format!("meta{}", std::env::consts::EXE_SUFFIX));
    let plan = login::Plan::new(program, home, |name| std::env::var(name).ok())?;
    login::Manager::current()?.install(&plan, run_step)?;
    println!("Installed the Meta-Harness login service.");
    if cfg!(target_os = "linux") && !lingering() {
        eprintln!(
            "meta: Meta-Harness stops when you log out. To keep it running, an administrator \
             runs once: sudo loginctl enable-linger \"$(id -un)\""
        );
    }
    Ok(())
}

fn lingering() -> bool {
    let user = std::env::var("USER").unwrap_or_default();
    std::process::Command::new("loginctl")
        .args(["show-user", &user, "--property=Linger"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "Linger=yes")
}

pub(crate) fn remove_login_service() -> Result<(), String> {
    for failure in login::Manager::current()?.uninstall(run_step)? {
        eprintln!("meta: warning: {failure}");
    }
    println!("Removed the Meta-Harness login service.");
    Ok(())
}

fn removes_data(prefix: &Path, home: &Path) -> bool {
    let resolved = |path: PathBuf| path.canonicalize().unwrap_or(path);
    home == prefix
        || ["lib/meta-harness", "share/meta-harness"]
            .iter()
            .any(|folder| home.starts_with(resolved(prefix.join(folder))))
}

fn data_in_installation(prefix: &Path, homes: &[PathBuf]) -> Option<PathBuf> {
    homes
        .iter()
        .find(|home| {
            let target = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
            removes_data(prefix, &target)
        })
        .cloned()
}

pub(crate) fn uninstall() -> Result<(), String> {
    let prefix = prefix()?;
    let home = default_home()?;
    let manager = login::Manager::current()?;
    let homes: Vec<PathBuf> = std::iter::once(home.clone())
        .chain(manager.recorded_home()?)
        .collect();
    if let Some(data) = data_in_installation(&prefix, &homes) {
        return Err(format!(
            "The data folder {} lies in the installation that uninstall removes, so nothing was \
             removed. Move it outside {}, set META_HARNESS_HOME to its new place, run `meta \
             service install` and then uninstall again.",
            data.display(),
            prefix.display()
        ));
    }
    remove_login_service()?;
    remove_installation(&prefix)?;
    println!(
        "Meta-Harness is uninstalled; its data stays in {}.",
        home.display()
    );
    Ok(())
}

fn remove_installation(prefix: &Path) -> Result<(), String> {
    for folder in [
        prefix.join("lib/meta-harness"),
        prefix.join("share/meta-harness"),
    ] {
        ignore_missing(std::fs::remove_dir_all(&folder))
            .map_err(|e| format!("Could not remove {}: {e}", folder.display()))?;
    }
    let program = format!("meta{}", std::env::consts::EXE_SUFFIX);
    for name in [program.clone(), format!("{program}.old")] {
        let path = prefix.join("bin").join(&name);
        let removed = std::fs::remove_file(&path).or_else(|error| {
            let aside = std::env::temp_dir()
                .join(format!("meta-uninstalled-{}-{name}", std::process::id()));
            if cfg!(windows) && path.is_file() && std::fs::rename(&path, &aside).is_ok() {
                Ok(())
            } else {
                Err(error)
            }
        });
        ignore_missing(removed).map_err(|e| format!("Could not remove {}: {e}", path.display()))?;
    }
    for folder in [
        prefix.join("bin"),
        prefix.join("lib"),
        prefix.join("share"),
        prefix.to_owned(),
    ] {
        let _ = std::fs::remove_dir(folder);
    }
    Ok(())
}

fn run_step(step: &login::Step) -> Result<(), String> {
    let output = std::process::Command::new(step.program)
        .args(&step.args)
        .output()
        .map_err(|e| format!("Could not run {}: {e}", step.program))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} {} failed: {}",
            step.program,
            step.args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Dir, put, read};

    #[test]
    fn a_download_installs_meta_t3_and_notices_and_replaces_an_older_t3() {
        let root = Dir::new();
        let bundle = root.join("meta-harness-1.0.0-test-x64");
        let meta = format!("meta{}", std::env::consts::EXE_SUFFIX);
        put(
            bundle.join("lib/meta-harness/t3-0.1.11-test-x64/client/index.html"),
            "t3",
        );
        put(bundle.join("bin").join(&meta), "new meta");
        for notice in NOTICES {
            put(bundle.join(notice), notice);
        }
        put(bundle.join("bundle-info"), "1.0.0\ntest-x64\n");
        let prefix = root.join("prefix");
        std::fs::create_dir_all(prefix.join("lib/meta-harness/t3-0.1.9-test-x64")).unwrap();
        put(prefix.join("bin").join(&meta), "old meta");

        assert!(
            install_files(&bundle, &prefix, "other-x64", &[])
                .unwrap_err()
                .contains("not other-x64")
        );
        let data = prefix.join("lib/meta-harness/t3-user-data");
        put(data.join("meta-harness.sqlite"), "data");
        let refused =
            install_files(&bundle, &prefix, "test-x64", std::slice::from_ref(&data)).unwrap_err();
        assert!(refused.contains("t3-user-data"), "{refused}");
        assert_eq!(read(prefix.join("bin").join(&meta)), "old meta");
        assert!(data.join("meta-harness.sqlite").exists());
        let lock = std::fs::File::create(prefix.join("lib/meta-harness/.install.lock")).unwrap();
        lock.lock().unwrap();
        let busy = install_files(&bundle, &prefix, "test-x64", &[]).unwrap_err();
        assert!(busy.contains("Another"), "{busy}");
        assert_eq!(read(prefix.join("bin").join(&meta)), "old meta");
        drop(lock);
        let program = install_files(&bundle, &prefix, "test-x64", &[]).unwrap();

        assert_eq!(read(&program), "new meta");
        assert_eq!(
            read(prefix.join("lib/meta-harness/t3-0.1.11-test-x64/client/index.html")),
            "t3"
        );
        assert!(!prefix.join("lib/meta-harness/t3-0.1.9-test-x64").exists());
        assert!(
            prefix
                .join("share/meta-harness/THIRD_PARTY_NOTICES.md")
                .is_file()
        );
    }

    #[test]
    fn uninstall_removes_the_program_and_its_backup_but_never_a_folder_in_their_place() {
        let prefix = Dir::new();
        let program = format!("meta{}", std::env::consts::EXE_SUFFIX);
        std::fs::create_dir_all(prefix.join("lib/meta-harness/t3")).unwrap();
        std::fs::create_dir_all(prefix.join("share/meta-harness")).unwrap();
        put(prefix.join("bin").join(&program), "meta");
        put(prefix.join("bin").join(format!("{program}.old")), "old");
        remove_installation(&prefix).unwrap();
        assert!(!prefix.exists());

        let kept = prefix
            .join("bin")
            .join(format!("{program}.old"))
            .join("notes.txt");
        put(&kept, "the person's notes");
        assert!(remove_installation(&prefix).is_err());
        assert_eq!(read(&kept), "the person's notes");
    }

    #[test]
    fn uninstall_refuses_a_data_folder_inside_what_it_removes_or_the_login_service_keeps() {
        let prefix = Path::new("/opt/mh");
        for (home, refused) in [
            ("/opt/mh/lib/meta-harness/data", true),
            ("/opt/mh/share/meta-harness", true),
            ("/opt/mh", true),
            ("/opt/mh/lib/meta-harness-data", false),
            ("/opt/mh/bin/meta-data", false),
            ("/home/p/.meta-harness", false),
        ] {
            assert_eq!(removes_data(prefix, Path::new(home)), refused, "{home}");
        }
        let recorded = PathBuf::from("/opt/mh/lib/meta-harness/data");
        let shell = Path::new("/home/p/.meta-harness");
        assert_eq!(
            data_in_installation(prefix, &[shell.to_path_buf(), recorded.clone()]),
            Some(recorded)
        );
        assert_eq!(data_in_installation(prefix, &[shell.to_path_buf()]), None);
    }
}
