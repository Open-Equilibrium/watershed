use std::path::PathBuf;

pub fn executor_artifact() -> PathBuf {
    std::env::var_os("FLOW_EXECUTOR_UNDER_TEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_flow-executor").into())
}
