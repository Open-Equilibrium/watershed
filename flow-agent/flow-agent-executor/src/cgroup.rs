use rustix::fd::{AsRawFd, OwnedFd};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::unix::process::CommandExt,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
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
    ToolCgroup::create(1)?.finish(Duration::from_millis(250))?;
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
        "--property=DelegateSubgroup=supervisor",
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
            let _ = cgroup.finish(Duration::from_millis(250));
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

    pub(crate) fn finish(mut self, grace: Duration) -> Result<bool, String> {
        let exceeded = drain_and_observe_capacity(&self.path, self.initial_max_events, grace)?;
        std::fs::remove_dir(&self.path)
            .map_err(|error| format!("failed to remove Tool cgroup: {error}"))?;
        self.removed = true;
        Ok(exceeded)
    }
}

fn drain_and_observe_capacity(
    path: &Path,
    initial_max_events: u64,
    grace: Duration,
) -> Result<bool, String> {
    write_control(&path.join("cgroup.kill"), "1")?;
    let deadline = Instant::now() + grace;
    while read_event(&path.join("cgroup.events"), "populated")? != 0 {
        if Instant::now() >= deadline {
            return Err("Tool cgroup remained populated after cleanup".to_owned());
        }
        thread::sleep(Duration::from_millis(5));
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
    let membership = read_trimmed(Path::new("/proc/self/cgroup"))?;
    let mut lines = membership.lines();
    let line = lines
        .next()
        .filter(|_| lines.next().is_none())
        .ok_or_else(|| "Executor requires one unified cgroup-v2 membership".to_owned())?;
    let relative = line
        .strip_prefix("0::/")
        .ok_or_else(|| "Executor requires unified cgroup-v2 membership".to_owned())?;
    let components = Path::new(relative).components().collect::<Vec<_>>();
    if components.len() < 2
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
        || components
            .last()
            .and_then(|component| component.as_os_str().to_str())
            != Some("supervisor")
        || !components[components.len() - 2]
            .as_os_str()
            .to_string_lossy()
            .ends_with(".scope")
    {
        return Err("Executor is not in a delegated transient scope supervisor".to_owned());
    }
    let scope_relative =
        components[..components.len() - 1]
            .iter()
            .fold(PathBuf::new(), |mut path, component| {
                path.push(component.as_os_str());
                path
            });
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
    use super::drain_and_observe_capacity;
    use std::{
        fs,
        path::Path,
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::{Duration, Instant},
    };

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn cleanup_observes_capacity_events_triggered_before_descendants_exit() {
        let path = std::env::temp_dir().join(format!(
            "watershed-cgroup-cleanup-{}-{}",
            std::process::id(),
            NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("fake cgroup is created");
        fs::write(path.join("pids.events"), "max 0\n").expect("capacity events are initialized");
        fs::write(path.join("cgroup.events"), "populated 1\n")
            .expect("fake cgroup starts populated");
        fs::write(path.join("cgroup.kill"), "").expect("kill control is initialized");

        let descendant_path = path.clone();
        let descendant = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while fs::read_to_string(descendant_path.join("cgroup.kill"))
                .expect("kill control remains readable")
                .trim()
                != "1"
            {
                assert!(Instant::now() < deadline, "cleanup must issue cgroup.kill");
                thread::yield_now();
            }
            fs::write(descendant_path.join("pids.events"), "max 1\n")
                .expect("capacity event is recorded");
            fs::write(descendant_path.join("cgroup.events"), "populated 0\n")
                .expect("descendant exit is recorded");
        });

        let observed = drain_and_observe_capacity(&path, 0, Duration::from_secs(2));
        descendant.join().expect("fake descendant exits");
        remove_fake_cgroup(&path);

        assert_eq!(observed, Ok(true));
    }

    fn remove_fake_cgroup(path: &Path) {
        fs::remove_dir_all(path).expect("fake cgroup is removed");
    }
}
