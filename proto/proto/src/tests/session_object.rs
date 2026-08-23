use crate::decode_lowercase_sha256_hex;

#[test]
fn lowercase_sha256_decoder_owns_the_complete_digest_grammar() {
    let encoded = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let decoded = decode_lowercase_sha256_hex(encoded).expect("canonical digest");

    assert_eq!(
        decoded[0..8],
        [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]
    );
    assert!(decode_lowercase_sha256_hex(&encoded.to_uppercase()).is_none());
    assert!(decode_lowercase_sha256_hex(&encoded[..63]).is_none());
}
