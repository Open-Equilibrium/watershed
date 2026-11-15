mod temp_root;

pub(crate) use temp_root::TempRoot;

use serde::Serialize;
#[cfg(target_os = "linux")]
use std::fs;
use std::{
    env,
    error::Error,
    ffi::OsStr,
    io::{Read, Write},
    os::unix::process::CommandExt as _,
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::Duration,
};

pub(crate) const FLOW_AGENT_HOME: (&str, &str) = ("FLOW_AGENT_HOME", ".flow");
const MAX_SAMPLE_COUNT: usize = 1_000;
/// Most bytes kept from each measurement child stream; a child that writes more is killed.
const MAX_MEASUREMENT_CHILD_STREAM_BYTES: usize = 64 * 1024;

pub(crate) type DynError = Box<dyn Error + Send + Sync>;

pub(crate) fn launch_measurement_child<I, S>(
    session_root: &TempRoot,
    executable: &Path,
    args: I,
    isolated_homes: &[(&str, &str)],
) -> Result<Output, DynError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    for (variable, leaf) in isolated_homes {
        command.env(variable, session_root.path().join(leaf));
    }
    let mut child = command.spawn()?;
    // The child leads its own process group, so a kill also reaches any grandchild holding a pipe.
    let group = rustix::process::Pid::from_child(&child);
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let stdout_reader = thread::spawn(move || read_bounded(stdout, group));
    let stderr_reader = thread::spawn(move || read_bounded(stderr, group));
    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;
    // Reaping happens only after both readers ended, so the leader's id cannot be reused while signalled.
    let status = child.wait()?;
    if stdout.overflowed || stderr.overflowed {
        return Err(std::io::Error::other(format!(
            "measurement child wrote more than {MAX_MEASUREMENT_CHILD_STREAM_BYTES} bytes to one stream"
        ))
        .into());
    }
    Ok(Output {
        status,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
    })
}

struct BoundedStream {
    bytes: Vec<u8>,
    overflowed: bool,
}

/// Reads at most one byte past the bound, and kills the still-unreaped group on overflow.
fn read_bounded(stream: impl Read, group: rustix::process::Pid) -> std::io::Result<BoundedStream> {
    let mut bytes = Vec::new();
    stream
        .take(MAX_MEASUREMENT_CHILD_STREAM_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let overflowed = bytes.len() > MAX_MEASUREMENT_CHILD_STREAM_BYTES;
    if overflowed {
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
    Ok(BoundedStream { bytes, overflowed })
}

fn join_reader(
    reader: thread::JoinHandle<std::io::Result<BoundedStream>>,
) -> Result<BoundedStream, DynError> {
    reader
        .join()
        .map_err(|_| std::io::Error::other("measurement output reader panicked"))?
        .map_err(Into::into)
}

pub(crate) fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

pub(crate) fn parse_positive(value: &str, flag: &str) -> Result<usize, DynError> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| std::io::Error::other(format!("{flag} must be an integer")))?;
    if parsed == 0 || parsed > MAX_SAMPLE_COUNT {
        return Err(std::io::Error::other(format!(
            "{flag} must be between 1 and {MAX_SAMPLE_COUNT}"
        ))
        .into());
    }
    Ok(parsed)
}

pub(crate) fn percentile(sorted: &[u64], numerator: usize, denominator: usize) -> u64 {
    let rank = sorted
        .len()
        .saturating_mul(numerator)
        .div_ceil(denominator)
        .saturating_sub(1);
    sorted[rank.min(sorted.len().saturating_sub(1))]
}

pub(crate) fn write_jsonl(writer: &mut impl Write, value: &impl Serialize) -> Result<(), DynError> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    Ok(())
}

pub(crate) fn bounded_environment_value(name: &str, maximum_bytes: usize) -> Option<String> {
    let value = env::var(name).ok()?;
    (value.len() <= maximum_bytes).then_some(value)
}

#[derive(Serialize)]
pub(crate) struct Environment {
    pub(crate) os: &'static str,
    pub(crate) arch: &'static str,
    pub(crate) rustc: String,
    pub(crate) reference_platform: bool,
    pub(crate) commit_sha: Option<String>,
    pub(crate) runner_image: Option<String>,
    pub(crate) runner_image_version: Option<String>,
    pub(crate) logical_cpus: usize,
    pub(crate) cpu_model: Option<String>,
    pub(crate) total_memory_bytes: Option<u64>,
}

pub(crate) fn current_environment() -> Environment {
    let rustc = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unavailable".to_owned());
    let (cpu_model, total_memory_bytes) = hardware_metadata();
    Environment {
        os: env::consts::OS,
        arch: env::consts::ARCH,
        rustc,
        reference_platform: cfg!(all(target_os = "linux", target_arch = "x86_64")),
        commit_sha: bounded_environment_value("GITHUB_SHA", 128),
        runner_image: bounded_environment_value("ImageOS", 128),
        runner_image_version: bounded_environment_value("ImageVersion", 128),
        logical_cpus: std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
        cpu_model,
        total_memory_bytes,
    }
}

#[cfg(target_os = "linux")]
fn hardware_metadata() -> (Option<String>, Option<u64>) {
    let cpu_model = fs::read_to_string("/proc/cpuinfo").ok().and_then(|source| {
        source.lines().find_map(|line| {
            line.strip_prefix("model name")
                .and_then(|line| line.split_once(':'))
                .map(|(_, value)| value.trim().chars().take(256).collect())
        })
    });
    let memory = fs::read_to_string("/proc/meminfo").ok().and_then(|source| {
        source.lines().find_map(|line| {
            let value = line.strip_prefix("MemTotal:")?.split_whitespace().next()?;
            value.parse::<u64>().ok()?.checked_mul(1024)
        })
    });
    (cpu_model, memory)
}

#[cfg(not(target_os = "linux"))]
fn hardware_metadata() -> (Option<String>, Option<u64>) {
    (None, None)
}

#[cfg(test)]
pub(crate) mod test {
    use std::io::{self, Write};

    #[derive(Default)]
    pub(crate) struct FlushTrackingWriter {
        pub(crate) bytes: Vec<u8>,
        pub(crate) flushed: bool,
    }

    impl Write for FlushTrackingWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushed = true;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TempRoot, percentile};
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn measurement_roots_resolve_existing_temporary_directory_aliases() {
        let owner = TempRoot::create("flow-evidence-root-test").unwrap();
        let base = owner.path().join("base");
        fs::create_dir(&base).unwrap();
        let alias = owner.path().join("alias");
        symlink(&base, &alias).unwrap();
        let session = TempRoot::create_in(&alias, "measurement").unwrap();
        assert_eq!(session.path(), fs::canonicalize(session.path()).unwrap());
        assert!(session.path().starts_with(fs::canonicalize(base).unwrap()));
    }

    #[test]
    fn nearest_rank_percentile_is_deterministic() {
        let samples = (1..=30).collect::<Vec<_>>();
        assert_eq!(percentile(&samples, 50, 100), 15);
        assert_eq!(percentile(&samples, 95, 100), 29);
    }
}
