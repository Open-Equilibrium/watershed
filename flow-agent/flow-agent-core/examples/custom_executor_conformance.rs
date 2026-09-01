use std::{
    env,
    error::Error,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const ADVISORY: &str = "Advisory only: passing does not certify compatibility, policy equivalence, security, or operations.";
const CHILD_ARG: &str = "--conformance-child";
const MAX_CHILD_DIAGNOSTIC_BYTES: usize = 1_024;
type DynError = Box<dyn Error + Send + Sync>;

struct TempRoot(PathBuf);

impl TempRoot {
    fn create() -> Result<Self, DynError> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!(
            "flow-custom-executor-conformance-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn parse_args<I, S>(args: I) -> Result<PathBuf, DynError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let [flag, executor] = args.as_slice() else {
        return Err(io::Error::other(
            "usage: custom_executor_conformance --executor <absolute-path>",
        )
        .into());
    };
    if flag != "--executor" {
        return Err(io::Error::other(
            "usage: custom_executor_conformance --executor <absolute-path>",
        )
        .into());
    }
    let executor = PathBuf::from(executor);
    if !executor.is_absolute() {
        return Err(io::Error::other("--executor must be an absolute path").into());
    }
    Ok(executor)
}

fn run_with(
    executor: &Path,
    mut output: impl Write,
    check: impl FnOnce(&Path) -> Result<(), DynError>,
) -> Result<(), DynError> {
    writeln!(output, "{ADVISORY}")?;
    check(executor)?;
    writeln!(
        output,
        "Custom Executor observable protocol conformance: PASS"
    )?;
    Ok(())
}

fn bounded_diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_CHILD_DIAGNOSTIC_BYTES)])
        .trim()
        .to_owned()
}

fn run_isolated_check(executor: &Path) -> Result<(), DynError> {
    let session = TempRoot::create()?;
    let workspace = session.path().join("workspace");
    fs::create_dir(&workspace)?;
    let child = Command::new(env::current_exe()?)
        .arg(CHILD_ARG)
        .arg(executor)
        .arg(&workspace)
        .env("FLOW_AGENT_HOME", session.path().join(".flow"))
        .env("XDG_CONFIG_HOME", session.path().join(".config"))
        .output()?;
    if !child.status.success() {
        return Err(io::Error::other(format!(
            "isolated conformance child failed: {}",
            bounded_diagnostic(&child.stderr)
        ))
        .into());
    }
    Ok(())
}

fn run_child_with(
    executor: &Path,
    workspace: &Path,
    ensure_host: impl FnOnce() -> Result<(), DynError>,
    configure: impl FnOnce(&Path) -> Result<(), DynError>,
    check: impl FnOnce(&Path) -> Result<(), DynError>,
) -> Result<(), DynError> {
    ensure_host()?;
    configure(executor)?;
    check(workspace)
}

fn run_child(executor: &Path, workspace: &Path) -> Result<(), DynError> {
    run_child_with(
        executor,
        workspace,
        || {
            flow_agent_core::ensure_m12_executor_host()
                .map_err(|error| io::Error::other(error).into())
        },
        |path| {
            flow_agent_core::configure_executor_path(path)
                .map(|_| ())
                .map_err(|error| io::Error::other(error).into())
        },
        |root| {
            flow_agent_core::run_m12_executor_startup(root)
                .map(|_| ())
                .map_err(|error| io::Error::other(error).into())
        },
    )
}

fn run_main() -> Result<(), DynError> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if let [child_arg, executor, workspace] = args.as_slice()
        && child_arg == CHILD_ARG
    {
        return run_child(Path::new(executor), Path::new(workspace));
    }
    let executor = parse_args(args)?;
    run_with(&executor, io::stdout().lock(), run_isolated_check)
}

fn main() {
    if let Err(error) = run_main() {
        eprintln!("custom_executor_conformance: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{ADVISORY, DynError, parse_args, run_child_with, run_with};
    use std::{cell::Cell, io, path::PathBuf};

    fn executor_path() -> PathBuf {
        PathBuf::from(if cfg!(windows) {
            r"C:\trusted\flow-executor.exe"
        } else {
            "/trusted/flow-executor"
        })
    }

    #[test]
    fn grammar_requires_one_absolute_executor() {
        assert_eq!(
            parse_args(["--executor", executor_path().to_str().unwrap()]).unwrap(),
            executor_path()
        );
        for args in [
            Vec::<&str>::new(),
            vec!["--executor"],
            vec!["--executor", "relative"],
            vec!["--path", executor_path().to_str().unwrap()],
            vec!["--executor", executor_path().to_str().unwrap(), "extra"],
        ] {
            assert!(parse_args(args).is_err());
        }
    }

    #[test]
    fn passing_suite_is_explicitly_advisory() {
        let mut output = Vec::new();
        let mut observed = None;

        run_with(&executor_path(), &mut output, |path| {
            observed = Some(path.to_owned());
            Ok(())
        })
        .unwrap();

        assert_eq!(observed, Some(executor_path()));
        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("{ADVISORY}\nCustom Executor observable protocol conformance: PASS\n")
        );
    }

    #[test]
    fn failed_suite_still_states_the_advisory_boundary() {
        let mut output = Vec::new();

        let error = run_with(&executor_path(), &mut output, |_| -> Result<(), DynError> {
            Err(io::Error::other("injected failure").into())
        })
        .expect_err("failed check remains failed");

        assert_eq!(error.to_string(), "injected failure");
        assert_eq!(String::from_utf8(output).unwrap(), format!("{ADVISORY}\n"));
    }

    #[test]
    fn unsupported_release_precedes_configuration_and_executor_spawn() {
        let configured = Cell::new(0_u8);
        let spawned = Cell::new(0_u8);

        let error = run_child_with(
            &executor_path(),
            &PathBuf::from("workspace"),
            || Err(io::Error::other("unsupported release").into()),
            |_| {
                configured.set(configured.get() + 1);
                Ok(())
            },
            |_| {
                spawned.set(spawned.get() + 1);
                Ok(())
            },
        )
        .expect_err("unsupported hosts fail closed before side effects");

        assert_eq!(error.to_string(), "unsupported release");
        assert_eq!(configured.get(), 0);
        assert_eq!(spawned.get(), 0);
    }
}
