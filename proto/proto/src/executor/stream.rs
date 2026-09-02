use super::ExecutorProtocolError;

/// Encodes arbitrary Tool stream bytes as canonical padded standard base64.
pub fn encode_executor_stream_v0(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(ALPHABET[usize::from(first >> 2)] as char);
        encoded.push(ALPHABET[usize::from((first & 0x03) << 4 | second >> 4)] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[usize::from((second & 0x0f) << 2 | third >> 6)] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[usize::from(third & 0x3f)] as char
        } else {
            '='
        });
    }
    encoded
}

/// Decodes canonical padded standard base64 Tool stream bytes.
pub fn decode_executor_stream_v0(encoded: &str) -> Result<Vec<u8>, ExecutorProtocolError> {
    if !encoded.len().is_multiple_of(4) || !encoded.is_ascii() {
        return Err(invalid_base64());
    }
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len() / 4 * 3);
    let (chunks, remainder) = bytes.as_chunks::<4>();
    debug_assert!(remainder.is_empty());
    for (index, chunk) in chunks.iter().enumerate() {
        let last = index + 1 == bytes.len() / 4;
        let a = base64_value(chunk[0])?;
        let b = base64_value(chunk[1])?;
        let c = if chunk[2] == b'=' {
            if !last || chunk[3] != b'=' || b & 0x0f != 0 {
                return Err(invalid_base64());
            }
            None
        } else {
            Some(base64_value(chunk[2])?)
        };
        let d = if chunk[3] == b'=' {
            if !last || c.is_some_and(|value| value & 0x03 != 0) {
                return Err(invalid_base64());
            }
            None
        } else {
            Some(base64_value(chunk[3])?)
        };
        decoded.push(a << 2 | b >> 4);
        if let Some(c) = c {
            decoded.push(b << 4 | c >> 2);
            if let Some(d) = d {
                decoded.push(c << 6 | d);
            }
        }
    }
    if encode_executor_stream_v0(&decoded) != encoded {
        return Err(invalid_base64());
    }
    Ok(decoded)
}

fn base64_value(byte: u8) -> Result<u8, ExecutorProtocolError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(invalid_base64()),
    }
}

fn invalid_base64() -> ExecutorProtocolError {
    ExecutorProtocolError::new("Executor Tool stream is not canonical base64")
}
