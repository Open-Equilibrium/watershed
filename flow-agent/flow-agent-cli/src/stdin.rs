use flow_agent_core::RuntimeError;
use std::{io::Read, path::PathBuf};

pub(crate) fn read_bounded_utf8_stdin(
    limit: usize,
    diagnostic_label: &str,
) -> Result<String, RuntimeError> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take(u64::try_from(limit.saturating_add(1)).expect("stdin limit fits u64"))
        .read_to_end(&mut bytes)
        .map_err(|source| RuntimeError::Io {
            path: PathBuf::from("<stdin>"),
            source,
        })?;
    if bytes.len() > limit {
        return Err(RuntimeError::Usage(format!(
            "{diagnostic_label} exceeds {limit} bytes"
        )));
    }
    String::from_utf8(bytes)
        .map_err(|_| RuntimeError::Usage(format!("{diagnostic_label} must be valid UTF-8")))
}
