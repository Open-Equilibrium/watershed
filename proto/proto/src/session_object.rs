use std::fmt;

const SESSION_OBJECT_URI_PREFIX: &str = "session-object:sha256:";

/// Failure to parse or build one canonical session-object URI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionObjectUriError {
    /// The URI did not use the canonical session-object scheme.
    NonCanonicalUri,
    /// The digest was not exactly 64 lowercase hexadecimal characters.
    InvalidDigest,
}

impl fmt::Display for SessionObjectUriError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonCanonicalUri => "session-object URI is not canonical",
            Self::InvalidDigest => "session-object digest is not lowercase SHA-256 hex",
        })
    }
}

impl std::error::Error for SessionObjectUriError {}

/// Parses one canonical session-object URI and returns its lowercase SHA-256 digest.
pub fn parse_session_object_uri(uri: &str) -> Result<&str, SessionObjectUriError> {
    let digest = uri
        .strip_prefix(SESSION_OBJECT_URI_PREFIX)
        .ok_or(SessionObjectUriError::NonCanonicalUri)?;
    validate_digest(digest)?;
    Ok(digest)
}

/// Builds one canonical session-object URI from a lowercase SHA-256 digest.
pub fn build_session_object_uri(digest: &str) -> Result<String, SessionObjectUriError> {
    validate_digest(digest)?;
    Ok(format!("{SESSION_OBJECT_URI_PREFIX}{digest}"))
}

/// Decodes exactly 64 lowercase hexadecimal SHA-256 characters.
pub fn decode_lowercase_sha256_hex(value: &str) -> Option<[u8; 32]> {
    fn nibble(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        }
    }

    let bytes: &[u8; 64] = value.as_bytes().try_into().ok()?;
    let mut digest = [0_u8; 32];
    for (target, pair) in digest.iter_mut().zip(bytes.as_chunks::<2>().0) {
        *target = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(digest)
}

fn validate_digest(digest: &str) -> Result<(), SessionObjectUriError> {
    if decode_lowercase_sha256_hex(digest).is_some() {
        Ok(())
    } else {
        Err(SessionObjectUriError::InvalidDigest)
    }
}
