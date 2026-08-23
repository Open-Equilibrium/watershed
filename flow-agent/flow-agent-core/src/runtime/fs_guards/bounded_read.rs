use super::{Path, Read, RuntimeError, io};

const MAX_FILE_READ_REQUEST_BYTES: usize = 1024 * 1024;

pub fn path_io_error(path: &Path, source: io::Error) -> RuntimeError {
    RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
pub fn for_each_reader_line_with_limit(
    reader: impl Read,
    path: &Path,
    max_bytes: u64,
    visit: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<u64, RuntimeError> {
    for_each_reader_line_with_limit_inner(reader, path, max_bytes, false, visit)
}

pub(super) fn for_each_reader_line_with_limit_inner(
    reader: impl Read,
    path: &Path,
    max_bytes: u64,
    require_trailing_lf: bool,
    mut visit: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<u64, RuntimeError> {
    let mut reader = io::BufReader::new(reader.take(max_bytes.saturating_add(1)));
    let mut line = Vec::new();
    let mut total = 0u64;
    loop {
        line.clear();
        let read = io::BufRead::read_until(&mut reader, b'\n', &mut line)
            .map_err(|source| path_io_error(path, source))?;
        if read == 0 {
            if require_trailing_lf && total == 0 {
                return Err(RuntimeError::Protocol(format!(
                    "{} non-final segment must end with LF",
                    path.display()
                )));
            }
            break;
        }
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if total > max_bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} read size {total} bytes exceeds max {max_bytes}",
                path.display()
            )));
        }
        if require_trailing_lf && !line.ends_with(b"\n") {
            return Err(RuntimeError::Protocol(format!(
                "{} non-final segment must end with LF",
                path.display()
            )));
        }
        let line = std::str::from_utf8(&line).map_err(|source| {
            RuntimeError::Protocol(format!("{} is not valid UTF-8: {source}", path.display()))
        })?;
        visit(line)?;
    }
    Ok(total)
}

pub fn decode_utf8(path: &Path, bytes: Vec<u8>) -> Result<String, RuntimeError> {
    String::from_utf8(bytes).map_err(|source| {
        RuntimeError::Protocol(format!("{} is not valid UTF-8: {source}", path.display()))
    })
}

pub fn read_opened_file_with_limit(
    file: impl Read,
    total_len: u64,
    path: &Path,
    max_bytes: u64,
) -> Result<Vec<u8>, RuntimeError> {
    if total_len > max_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {total_len} bytes exceeds max {max_bytes}",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    let mut file = file.take(max_bytes.saturating_add(1));
    let buffer_bytes = max_bytes
        .saturating_add(1)
        .min(MAX_FILE_READ_REQUEST_BYTES as u64) as usize;
    let mut buffer = vec![0; buffer_bytes];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| path_io_error(path, source))?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&buffer[..read]);
        let bytes_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if bytes_len > max_bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} read size {bytes_len} bytes exceeds max {max_bytes}",
                path.display()
            )));
        }
    }
}
