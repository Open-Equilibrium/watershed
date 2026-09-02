use serde::Serialize;
use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(crate) const FLOW_AGENT_HOME: (&str, &str) = ("FLOW_AGENT_HOME", ".flow");
const MAX_SAMPLE_COUNT: usize = 1_000;

pub(crate) type DynError = Box<dyn Error + Send + Sync>;

pub(crate) struct TempRoot(PathBuf);

impl TempRoot {
    pub(crate) fn create(prefix: &str) -> Result<Self, DynError> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()));
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

pub(crate) fn launch_measurement_child<I, S>(
    session_root: &TempRoot,
    args: I,
    isolated_homes: &[(&str, &str)],
) -> Result<Output, DynError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(env::current_exe()?);
    command.args(args);
    for (variable, leaf) in isolated_homes {
        command.env(variable, session_root.path().join(leaf));
    }
    Ok(command.output()?)
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
    use super::percentile;

    #[test]
    fn nearest_rank_percentile_is_deterministic() {
        let samples = (1..=30).collect::<Vec<_>>();
        assert_eq!(percentile(&samples, 50, 100), 15);
        assert_eq!(percentile(&samples, 95, 100), 29);
    }
}
