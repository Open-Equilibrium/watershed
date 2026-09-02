use proto::{
    EXECUTOR_BACKEND_V0, EXECUTOR_NAME_V0, EXECUTOR_PLATFORM_V0, EXECUTOR_PROBE_SCHEMA_V0,
    EXECUTOR_PROTOCOL_VERSION_V0, ExecutorProbeV0, MAX_EXECUTOR_REQUEST_BYTES_V0,
    canonical_executor_probe_v0, canonical_executor_response_v0, parse_executor_request_v0,
};
use std::io::{self, Read, Write};

pub(crate) const MAX_READINESS_DIAGNOSTIC_BYTES: usize = 1024;
const READINESS_DIAGNOSTIC_PREFIX: &str = "flow-executor readiness: ";

pub(crate) fn run() -> Result<(), String> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    if arguments.is_empty()
        && crate::platform::official_host()
        && crate::platform::statically_linked_self()
    {
        return crate::cgroup::enter_transient_scope();
    }
    let stdin = io::stdin();
    let stdout = io::stdout();
    let stderr = io::stderr();
    run_with_diagnostics(&arguments, stdin.lock(), stdout.lock(), stderr.lock())
}

#[cfg(test)]
pub(crate) fn run_with(
    arguments: &[String],
    input: impl Read,
    output: impl Write,
) -> Result<(), String> {
    run_with_diagnostics(arguments, input, output, io::sink())
}

pub(crate) fn run_with_diagnostics(
    arguments: &[String],
    input: impl Read,
    output: impl Write,
    diagnostics: impl Write,
) -> Result<(), String> {
    match arguments {
        [mode] if mode == "--probe" => write_probe(crate::backend::probe(), output, diagnostics),
        [] => execute_request(input, output),
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        [mode] if mode == crate::cgroup::SCOPED_ARGUMENT => execute_request(input, output),
        [mode, request_fd, status_fd, tool_cgroup_fd] if mode == "--inner" => {
            run_inner(request_fd, status_fd, tool_cgroup_fd)
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        [mode] if mode == crate::cgroup::SELF_TEST_ARGUMENT => crate::cgroup::scope_self_test(),
        [mode] if mode == "--inner-self-test" => Ok(()),
        _ => Err("usage: flow-executor [--probe]".to_owned()),
    }
}

pub(crate) fn write_probe(
    state: crate::backend::ProbeState,
    mut output: impl Write,
    mut diagnostics: impl Write,
) -> Result<(), String> {
    let crate::backend::ProbeState {
        backend_version,
        ready,
        features,
        readiness_error,
    } = state;
    let probe = ExecutorProbeV0 {
        schema: EXECUTOR_PROBE_SCHEMA_V0.to_owned(),
        executor: EXECUTOR_NAME_V0.to_owned(),
        executor_version: env!("CARGO_PKG_VERSION").to_owned(),
        backend: EXECUTOR_BACKEND_V0.to_owned(),
        backend_version,
        platform: EXECUTOR_PLATFORM_V0.to_owned(),
        protocol_versions: vec![EXECUTOR_PROTOCOL_VERSION_V0.to_owned()],
        ready,
        runtime_mounts: crate::backend::runtime_mount_manifest(),
        supported_policy_features: features,
    };
    let bytes = canonical_executor_probe_v0(&probe).map_err(|error| error.to_string())?;
    output
        .write_all(&bytes)
        .map_err(|error| format!("failed to write Executor probe: {error}"))?;
    if let Some(error) = readiness_error {
        write_readiness_diagnostic(&mut diagnostics, &error)?;
    }
    Ok(())
}

fn write_readiness_diagnostic(mut output: impl Write, reason: &str) -> Result<(), String> {
    let content_limit =
        MAX_READINESS_DIAGNOSTIC_BYTES.saturating_sub(READINESS_DIAGNOSTIC_PREFIX.len() + 1);
    let mut content = String::with_capacity(content_limit);
    let mut pending_space = false;
    for character in reason.chars() {
        if character.is_control() || character.is_whitespace() {
            pending_space = !content.is_empty();
            continue;
        }
        let separator_bytes = usize::from(pending_space);
        if content.len() + separator_bytes + character.len_utf8() > content_limit {
            break;
        }
        if pending_space {
            content.push(' ');
            pending_space = false;
        }
        content.push(character);
    }
    if content.is_empty() {
        content.push_str("unavailable");
    }
    let diagnostic = format!("{READINESS_DIAGNOSTIC_PREFIX}{content}\n");
    output
        .write_all(diagnostic.as_bytes())
        .map_err(|error| format!("failed to write Executor readiness diagnostic: {error}"))
}

fn execute_request(input: impl Read, mut output: impl Write) -> Result<(), String> {
    let bytes = read_bounded(input, MAX_EXECUTOR_REQUEST_BYTES_V0, "request")?;
    let request = parse_executor_request_v0(&bytes).map_err(|error| error.to_string())?;
    let response = crate::backend::execute(request)?;
    let bytes = canonical_executor_response_v0(&response).map_err(|error| error.to_string())?;
    output
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
fn run_inner(request_fd: &str, status_fd: &str, tool_cgroup_fd: &str) -> Result<(), String> {
    crate::backend::linux::run_inner(request_fd, status_fd, tool_cgroup_fd)
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn run_inner(_request_fd: &str, _status_fd: &str, _tool_cgroup_fd: &str) -> Result<(), String> {
    Err("inner Executor mode is unavailable on this platform".to_owned())
}
