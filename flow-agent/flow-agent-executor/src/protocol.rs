use proto::{
    EXECUTOR_BACKEND_V0, EXECUTOR_NAME_V0, EXECUTOR_PLATFORM_V0, EXECUTOR_PROBE_SCHEMA_V0,
    EXECUTOR_PROTOCOL_VERSION_V0, ExecutorProbeV0, MAX_EXECUTOR_REQUEST_BYTES_V0,
    canonical_executor_probe_v0, canonical_executor_response_v0, parse_executor_request_v0,
};
use std::io::{self, Read, Write};

pub(crate) fn run() -> Result<(), String> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [mode] if mode == "--probe" => write_probe(),
        [] => execute_request(),
        [mode, request_fd] if mode == "--inner" => run_inner(request_fd),
        [mode] if mode == "--inner-self-test" => Ok(()),
        _ => Err("usage: flow-executor [--probe]".to_owned()),
    }
}

fn write_probe() -> Result<(), String> {
    let state = crate::backend::probe();
    let probe = ExecutorProbeV0 {
        schema: EXECUTOR_PROBE_SCHEMA_V0.to_owned(),
        executor: EXECUTOR_NAME_V0.to_owned(),
        executor_version: env!("CARGO_PKG_VERSION").to_owned(),
        backend: EXECUTOR_BACKEND_V0.to_owned(),
        backend_version: state.backend_version,
        platform: EXECUTOR_PLATFORM_V0.to_owned(),
        protocol_versions: vec![EXECUTOR_PROTOCOL_VERSION_V0.to_owned()],
        ready: state.ready,
        runtime_mounts: crate::backend::runtime_mount_manifest(),
        supported_policy_features: state.features,
    };
    let bytes = canonical_executor_probe_v0(&probe).map_err(|error| error.to_string())?;
    io::stdout()
        .write_all(&bytes)
        .map_err(|error| format!("failed to write Executor probe: {error}"))
}

fn execute_request() -> Result<(), String> {
    let bytes = read_bounded(io::stdin(), MAX_EXECUTOR_REQUEST_BYTES_V0, "request")?;
    let request = parse_executor_request_v0(&bytes).map_err(|error| error.to_string())?;
    let response = crate::backend::execute(request);
    let bytes = canonical_executor_response_v0(&response).map_err(|error| error.to_string())?;
    io::stdout()
        .write_all(&bytes)
        .map_err(|error| format!("failed to write Executor response: {error}"))
}

fn read_bounded(input: impl Read, limit: usize, kind: &str) -> Result<Vec<u8>, String> {
    let take_limit = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    input
        .take(take_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read Executor {kind}: {error}"))?;
    if bytes.len() > limit {
        return Err(format!("Executor {kind} exceeds its byte limit"));
    }
    Ok(bytes)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn run_inner(request_fd: &str) -> Result<(), String> {
    crate::backend::linux::run_inner(request_fd)
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn run_inner(_request_fd: &str) -> Result<(), String> {
    Err("inner Executor mode is unavailable on this platform".to_owned())
}
