#[path = "../../release.rs"]
mod releases;
pub(crate) use releases::supported_release;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn official_host() -> bool {
    let Ok(release) = std::fs::read_to_string("/etc/os-release") else {
        return false;
    };
    supported_release("linux", "x86_64", &release)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ubuntu_release_requires_unambiguous_exact_fields() {
        for (release, expected) in [
            ("ID='ubuntu'\nVERSION_ID='24.04'\n", true),
            ("ID=debian\nID=ubuntu\nVERSION_ID=24.04\n", false),
            ("ID=ubuntu\nVERSION_ID=24.04\nVERSION_ID=24.04\n", false),
        ] {
            assert_eq!(
                super::supported_release("linux", "x86_64", release),
                expected,
                "{release:?}"
            );
        }
    }
}
