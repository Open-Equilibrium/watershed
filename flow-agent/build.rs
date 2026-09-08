fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").expect("Cargo supplies the target OS");
    let arch =
        std::env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo supplies the target architecture");
    assert!(
        target_is_supported(&os, &arch),
        "Flow Agent supports only Linux x86_64 and macOS ARM64; unsupported target: {os}/{arch}"
    );
}

fn target_is_supported(os: &str, arch: &str) -> bool {
    matches!((os, arch), ("linux", "x86_64") | ("macos", "aarch64"))
}

#[cfg(test)]
mod tests {
    use super::target_is_supported;

    #[test]
    fn build_target_matrix() {
        for (os, arch, supported) in [
            ("linux", "x86_64", true),
            ("macos", "aarch64", true),
            ("windows", "x86_64", false),
            ("windows", "aarch64", false),
            ("linux", "aarch64", false),
            ("macos", "x86_64", false),
        ] {
            assert_eq!(target_is_supported(os, arch), supported, "{os}/{arch}");
        }
    }
}
