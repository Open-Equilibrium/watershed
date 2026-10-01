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
