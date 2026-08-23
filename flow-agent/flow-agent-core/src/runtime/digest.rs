use sha2::{Digest, Sha256};

pub(crate) const SHA256_PREFIX: &str = "sha256:";

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    encode_sha256(&digest)
}

pub(crate) fn finish_sha256(hasher: Sha256) -> String {
    let digest = hasher.finalize();
    encode_sha256(&digest)
}

fn encode_sha256(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(digest.len() * 2);
    for &byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

pub(crate) fn prefixed_sha256_hex(bytes: &[u8]) -> String {
    format!("{SHA256_PREFIX}{}", sha256_hex(bytes))
}

pub(crate) fn strip_sha256_prefix(value: &str) -> Option<&str> {
    value.strip_prefix(SHA256_PREFIX)
}

pub(crate) fn is_lowercase_sha256_hex(value: &str) -> bool {
    proto::decode_lowercase_sha256_hex(value).is_some()
}
