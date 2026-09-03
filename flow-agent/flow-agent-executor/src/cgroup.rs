use rustix::{
    fd::{AsRawFd, OwnedFd},
    process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal},
};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::unix::process::CommandExt,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

pub(crate) const SCOPED_ARGUMENT: &str = "--capacity-scoped";
pub(crate) const SELF_TEST_ARGUMENT: &str = "--capacity-self-test";

const CGROUP_ROOT: &str = "/sys/fs/cgroup";
const SYSTEMD_RUN: &str = "/usr/bin/systemd-run";

pub(crate) fn enter_transient_scope() -> Result<(), String> {
    let image = inherited_self_image()?;
    let error = scope_command(&image, SCOPED_ARGUMENT).exec();
    Err(format!("failed to enter transient Executor scope: {error}"))
}

pub(crate) fn probe() -> Result<(), String> {
    let image = inherited_self_image()?;
    let output = scope_command(&image, SELF_TEST_ARGUMENT)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("failed to start transient Executor scope: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    let diagnostic = diagnostic.trim();
    let suffix = if diagnostic.is_empty() {
        String::new()
    } else {
        format!(": {diagnostic}")
    };
    Err(format!(
        "transient Executor scope self-test failed with {}{suffix}",
        output.status
    ))
}

pub(crate) fn scope_self_test() -> Result<(), String> {
    enter_supervisor_subgroup()?;
    ToolCgroup::create(1)?.finish()?;
    Ok(())
}

fn inherited_self_image() -> Result<File, String> {
    let image = File::open("/proc/self/exe")
        .map_err(|error| format!("failed to retain Executor image: {error}"))?;
    rustix::io::fcntl_setfd(&image, rustix::io::FdFlags::empty())
        .map_err(|error| format!("failed to inherit retained Executor image: {error}"))?;
    Ok(image)
}

fn scope_command(image: &File, argument: &str) -> Command {
    let mut command = Command::new(SYSTEMD_RUN);
    command.args([
        "--user",
        "--scope",
        "--quiet",
        "--collect",
        "--property=Delegate=pids",
        "--",
    ]);
    command
        .arg(format!("/proc/self/fd/{}", image.as_raw_fd()))
        .arg(argument);
    command
}

pub(crate) struct ToolCgroup {
    path: PathBuf,
    initial_max_events: u64,
    removed: bool,
}

impl ToolCgroup {
    pub(crate) fn create(max_tasks: u32) -> Result<Self, String> {
        if max_tasks == 0 {
            return Err("Tool process capacity must be positive".to_owned());
        }
        let scope = current_delegated_scope()?;
        write_control(&scope.join("cgroup.subtree_control"), "+pids")?;
        let path = scope.join("tool");
        std::fs::create_dir(&path)
            .map_err(|error| format!("failed to create Tool cgroup: {error}"))?;
        let mut cgroup = Self {
            path,
            initial_max_events: 0,
            removed: false,
        };
        let configured = (|| {
            write_control(&cgroup.path.join("pids.max"), &max_tasks.to_string())?;
            let observed = read_trimmed(&cgroup.path.join("pids.max"))?;
            if observed != max_tasks.to_string() {
                return Err("Tool cgroup process capacity did not persist exactly".to_owned());
            }
            let current = read_u64(&cgroup.path.join("pids.current"))?;
            if current != 0 {
                return Err("fresh Tool cgroup is unexpectedly populated".to_owned());
            }
            cgroup.initial_max_events = read_event(&cgroup.path.join("pids.events"), "max")?;
            if cgroup.initial_max_events != 0 {
                return Err("fresh Tool cgroup contains prior capacity events".to_owned());
            }
            if !cgroup.path.join("cgroup.kill").is_file() {
                return Err("Tool cgroup kill control is unavailable".to_owned());
            }
            Ok(())
        })();
        if let Err(error) = configured {
            let _ = cgroup.finish();
            return Err(error);
        }
        Ok(cgroup)
    }

    pub(crate) fn process_descriptor(&self) -> Result<OwnedFd, String> {
        OpenOptions::new()
            .write(true)
            .open(self.path.join("cgroup.procs"))
            .map(OwnedFd::from)
            .map_err(|error| format!("failed to open Tool cgroup membership: {error}"))
    }

    pub(crate) fn signal_termination(&self) -> Result<(), String> {
        signal_processes(&self.path)
    }

    pub(crate) fn force_kill(&self) -> Result<(), String> {
        write_control(&self.path.join("cgroup.kill"), "1")
    }

    pub(crate) fn is_empty(&self) -> Result<bool, String> {
        Ok(read_event(&self.path.join("cgroup.events"), "populated")? == 0)
    }

    pub(crate) fn finish(mut self) -> Result<bool, String> {
        let exceeded = observe_finished_capacity(&self.path, self.initial_max_events)?;
        std::fs::remove_dir(&self.path)
            .map_err(|error| format!("failed to remove Tool cgroup: {error}"))?;
        self.removed = true;
        Ok(exceeded)
    }
}

fn signal_processes(path: &Path) -> Result<(), String> {
    // This snapshot cannot include later forks. PID descriptors bind TERM to
    // the captured process identities; pids.max bounds later forks and the
    // forced cgroup.kill fallback covers the complete live leaf atomically.
    let targets = read_process_ids(path)?
        .into_iter()
        .filter_map(|raw| {
            let pid = Pid::from_raw(raw).expect("cgroup process IDs are positive");
            loop {
                match pidfd_open(pid, PidfdFlags::empty()) {
                    Ok(descriptor) => return Some(Ok((raw, descriptor))),
                    Err(rustix::io::Errno::INTR) => {}
                    Err(rustix::io::Errno::SRCH) => return None,
                    Err(error) => {
                        return Some(Err(format!(
                            "failed to retain Tool process for termination: {error}"
                        )));
                    }
                }
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (_, descriptor) in targets {
        loop {
            match pidfd_send_signal(&descriptor, Signal::TERM) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => break,
                Err(rustix::io::Errno::INTR) => {}
                Err(error) => {
                    return Err(format!("failed to terminate Tool process: {error}"));
                }
            }
        }
    }
    Ok(())
}

fn read_process_ids(path: &Path) -> Result<Vec<i32>, String> {
    read_trimmed(&path.join("cgroup.procs"))?
        .lines()
        .map(|line| {
            let raw = line
                .parse::<i32>()
                .map_err(|_| "cgroup.procs contained an invalid process ID".to_owned())?;
            Pid::from_raw(raw)
                .map(|_| raw)
                .ok_or_else(|| "cgroup.procs contained an invalid process ID".to_owned())
        })
        .collect()
}

fn observe_finished_capacity(path: &Path, initial_max_events: u64) -> Result<bool, String> {
    if read_event(&path.join("cgroup.events"), "populated")? != 0 {
        return Err("Tool cgroup remained populated after cleanup".to_owned());
    }
    Ok(read_event(&path.join("pids.events"), "max")? > initial_max_events)
}

impl Drop for ToolCgroup {
    fn drop(&mut self) {
        if self.removed {
            return;
        }
        let _ = write_control(&self.path.join("cgroup.kill"), "1");
        let _ = std::fs::remove_dir(&self.path);
    }
}

fn current_delegated_scope() -> Result<PathBuf, String> {
    let relative = current_cgroup_relative()?;
    let Some(scope_relative) = relative
        .parent()
        .filter(|_| relative.file_name().and_then(|name| name.to_str()) == Some("supervisor"))
    else {
        return Err("Executor is not in a delegated transient scope supervisor".to_owned());
    };
    if !scope_relative
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".scope"))
    {
        return Err("Executor is not in a delegated transient scope supervisor".to_owned());
    }
    let scope = Path::new(CGROUP_ROOT).join(scope_relative);
    let controllers = read_trimmed(&scope.join("cgroup.controllers"))?;
    if !controllers
        .split_ascii_whitespace()
        .any(|value| value == "pids")
    {
        return Err("delegated transient scope lacks the pids controller".to_owned());
    }
    if !read_trimmed(&scope.join("cgroup.procs"))?.is_empty() {
        return Err("delegated transient scope root is not empty".to_owned());
    }
    Ok(scope)
}

pub(crate) fn enter_supervisor_subgroup() -> Result<(), String> {
    let relative = current_cgroup_relative()?;
    if !relative
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".scope"))
    {
        return Err("Executor is not in a delegated transient scope".to_owned());
    }
    let scope = Path::new(CGROUP_ROOT).join(relative);
    let supervisor = scope.join("supervisor");
    std::fs::create_dir(&supervisor)
        .map_err(|error| format!("failed to create Executor supervisor cgroup: {error}"))?;
    write_control(&supervisor.join("cgroup.procs"), "0")?;
    let observed = current_delegated_scope()?;
    if observed != scope {
        return Err("Executor supervisor cgroup changed transient scope".to_owned());
    }
    Ok(())
}

fn current_cgroup_relative() -> Result<PathBuf, String> {
    let membership = read_trimmed(Path::new("/proc/self/cgroup"))?;
    let mut lines = membership.lines();
    let line = lines
        .next()
        .filter(|_| lines.next().is_none())
        .ok_or_else(|| "Executor requires one unified cgroup-v2 membership".to_owned())?;
    let relative = line
        .strip_prefix("0::/")
        .ok_or_else(|| "Executor requires unified cgroup-v2 membership".to_owned())?;
    let relative = PathBuf::from(relative);
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Executor requires a normal unified cgroup-v2 path".to_owned());
    }
    Ok(relative)
}

fn write_control(path: &Path, value: &str) -> Result<(), String> {
    std::fs::write(path, value)
        .map_err(|error| format!("failed to write {}: {error}", control_name(path)))
}

fn read_trimmed(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("failed to open {}: {error}", control_name(path)))?;
    let mut value = String::new();
    file.read_to_string(&mut value)
        .map_err(|error| format!("failed to read {}: {error}", control_name(path)))?;
    Ok(value.trim().to_owned())
}

fn read_u64(path: &Path) -> Result<u64, String> {
    read_trimmed(path)?
        .parse::<u64>()
        .map_err(|_| format!("{} did not contain an unsigned integer", control_name(path)))
}

fn read_event(path: &Path, key: &str) -> Result<u64, String> {
    let value = read_trimmed(path)?;
    let mut found = None;
    for line in value.lines() {
        let mut fields = line.split_ascii_whitespace();
        let name = fields
            .next()
            .ok_or_else(|| format!("{} contains an empty event", control_name(path)))?;
        let count = fields
            .next()
            .filter(|_| fields.next().is_none())
            .ok_or_else(|| format!("{} contains an invalid event", control_name(path)))?
            .parse::<u64>()
            .map_err(|_| format!("{} contains an invalid count", control_name(path)))?;
        if name == key && found.replace(count).is_some() {
            return Err(format!("{} repeats {key}", control_name(path)));
        }
    }
    found.ok_or_else(|| format!("{} omits {key}", control_name(path)))
}

fn control_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("cgroup control")
}

pub(crate) fn move_current_process(descriptor: i32) -> std::io::Result<()> {
    let bytes = b"0\n";
    // SAFETY: the inherited descriptor is a pre-opened cgroup.procs file and
    // write(2) is async-signal-safe in the single-threaded pre-exec child.
    let written = unsafe { c_write(descriptor, bytes.as_ptr(), bytes.len()) };
    if written == bytes.len() as isize {
        Ok(())
    } else if written < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            "short Tool cgroup membership write",
        ))
    }
}

unsafe extern "C" {
    #[link_name = "write"]
    unsafe fn c_write(descriptor: i32, buffer: *const u8, count: usize) -> isize;
}

#[cfg(test)]
mod tests {
    use super::observe_finished_capacity;
    use std::{
        fs,
        path::Path,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn cleanup_observes_capacity_events_after_the_tool_tree_is_empty() {
        let path = std::env::temp_dir().join(format!(
            "watershed-cgroup-cleanup-{}-{}",
            std::process::id(),
            NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("fake cgroup is created");
        fs::write(path.join("pids.events"), "max 1\n").expect("capacity event is recorded");
        fs::write(path.join("cgroup.events"), "populated 1\n").expect("fake cgroup is populated");

        let premature = observe_finished_capacity(&path, 0);
        fs::write(path.join("cgroup.events"), "populated 0\n").expect("fake cgroup is empty");

        let observed = observe_finished_capacity(&path, 0);
        remove_fake_cgroup(&path);

        assert_eq!(
            premature,
            Err("Tool cgroup remained populated after cleanup".to_owned())
        );
        assert_eq!(observed, Ok(true));
    }

    fn remove_fake_cgroup(path: &Path) {
        fs::remove_dir_all(path).expect("fake cgroup is removed");
    }
}
