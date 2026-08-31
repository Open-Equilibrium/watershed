use flow_agent_core::RuntimeError;
use std::path::PathBuf;

pub(super) const EXECUTOR_USAGE: &str = concat!(
    "usage: flow executor status|check\n",
    "       flow executor configure --path <absolute-path>\n",
    "       flow executor configure --default",
);

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ExecutorCommand {
    Status,
    Check,
    ConfigurePath(PathBuf),
    ConfigureDefault,
}

pub(crate) fn executor_args(args: &[String]) -> Result<ExecutorCommand, RuntimeError> {
    match args {
        [executor, status] if executor == "executor" && status == "status" => {
            Ok(ExecutorCommand::Status)
        }
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
        assert_eq!(
            executor_args(&strings(&["executor", "status"])).unwrap(),
            ExecutorCommand::Status
        );
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
