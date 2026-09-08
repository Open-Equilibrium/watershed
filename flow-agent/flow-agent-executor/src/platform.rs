#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn official_host() -> bool {
    let Ok(release) = std::fs::read_to_string("/etc/os-release") else {
        return false;
    };
    let mut id = None;
    let mut version = None;
    for line in release.lines() {
        if let Some(value) = line.strip_prefix("ID=") {
            id = Some(value.trim_matches('"'));
        } else if let Some(value) = line.strip_prefix("VERSION_ID=") {
            version = Some(value.trim_matches('"'));
        }
    }
    id == Some("ubuntu") && version == Some("24.04")
}
