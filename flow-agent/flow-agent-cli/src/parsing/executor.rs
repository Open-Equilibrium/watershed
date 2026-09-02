use flow_agent_core::RuntimeError;
use std::path::PathBuf;

pub(super) const EXECUTOR_USAGE: &str = concat!(
    "usage: flow executor check\n",
    "       flow executor configure --path <absolute-path>\n",
    "       flow executor configure --default",
);
const EXECUTOR_HELP: &str = concat!(
    "Usage:\n",
    "  flow executor check\n",
    "  flow executor configure --path <absolute-path>\n",
    "  flow executor configure --default\n",
);
const EXECUTOR_CHECK_HELP: &str = concat!(
    "Usage:\n",
    "  flow executor check\n",
    "\n",
    "Checks the configured Executor and reports its readiness.\n",
);
const EXECUTOR_CONFIGURE_HELP: &str = concat!(
    "Usage:\n",
    "  flow executor configure --path <absolute-path>\n",
    "  flow executor configure --default\n",
    "\n",
    "Options:\n",
    "  --path <absolute-path>  Select an administrator-supplied Executor.\n",
    "  --default               Remove the Custom override and restore default sibling resolution.\n",
);

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ExecutorCommand {
    Check,
    ConfigurePath(PathBuf),
    ConfigureDefault,
}

pub(super) fn executor_help(args: &[String]) -> Option<&'static str> {
    match args {
        [executor, help] if executor == "executor" && matches!(help.as_str(), "--help" | "-h") => {
            Some(EXECUTOR_HELP)
        }
        [executor, check, help]
            if executor == "executor"
                && check == "check"
                && matches!(help.as_str(), "--help" | "-h") =>
        {
            Some(EXECUTOR_CHECK_HELP)
        }
        [executor, configure, help]
            if executor == "executor"
                && configure == "configure"
                && matches!(help.as_str(), "--help" | "-h") =>
        {
            Some(EXECUTOR_CONFIGURE_HELP)
        }
        _ => None,
    }
}

pub(crate) fn executor_args(args: &[String]) -> Result<ExecutorCommand, RuntimeError> {
    match args {
        [executor, check] if executor == "executor" && check == "check" => {
            Ok(ExecutorCommand::Check)
        }
        [executor, configure, default]
            if executor == "executor" && configure == "configure" && default == "--default" =>
        {
            Ok(ExecutorCommand::ConfigureDefault)
        }
        [executor, configure, path_flag, path]
            if executor == "executor" && configure == "configure" && path_flag == "--path" =>
        {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(RuntimeError::Usage(
                    "Executor path must be absolute".to_owned(),
                ));
            }
            Ok(ExecutorCommand::ConfigurePath(path))
        }
        [executor, configure, ..] if executor == "executor" && configure == "configure" => {
            Err(RuntimeError::Usage(
                "usage: flow executor configure --path <absolute-path>|--default".to_owned(),
            ))
        }
        _ => Err(RuntimeError::Usage(EXECUTOR_USAGE.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::{ExecutorCommand, executor_args};

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn executor_grammar_is_closed() {
        assert!(executor_args(&strings(&["executor", "status"])).is_err());
        assert_eq!(
            executor_args(&strings(&["executor", "check"])).unwrap(),
            ExecutorCommand::Check
        );
        assert_eq!(
            executor_args(&strings(&["executor", "configure", "--default"])).unwrap(),
            ExecutorCommand::ConfigureDefault
        );
        let absolute = if cfg!(windows) {
            r"C:\trusted\flow-executor.exe"
        } else {
            "/trusted/flow-executor"
        };
        assert_eq!(
            executor_args(&strings(&["executor", "configure", "--path", absolute])).unwrap(),
            ExecutorCommand::ConfigurePath(absolute.into())
        );
        for args in [
            &["executor"][..],
            &["executor", "configure"][..],
            &["executor", "configure", "--default", "extra"][..],
            &["executor", "status", "extra"][..],
        ] {
            assert!(executor_args(&strings(args)).is_err(), "{args:?}");
        }
    }
}
