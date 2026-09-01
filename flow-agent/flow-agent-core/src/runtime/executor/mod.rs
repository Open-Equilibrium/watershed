mod client;
#[cfg(any(all(target_os = "linux", target_arch = "x86_64"), test))]
mod config;
#[cfg(any(all(target_os = "linux", target_arch = "x86_64"), test))]
mod probe;
mod process;
mod selection;

#[cfg(test)]
pub(crate) use client::ExecutorToolExecution;
pub(crate) use client::{ExecutorDispatchOutcome, PreparedExecutor, PreparedExecutorTool};
#[cfg(test)]
pub(crate) use config::{EXECUTOR_CONFIG_MAX_BYTES, ExecutorConfigStore};
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
pub(crate) use selection::default_executor_path;
pub(crate) use selection::resolve_executor;
pub use selection::{
    ExecutorSelection, ExecutorSelectionSource, configure_default_executor,
    configure_executor_path, executor_check,
};
