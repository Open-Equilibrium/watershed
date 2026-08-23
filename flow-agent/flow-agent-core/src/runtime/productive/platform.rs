use super::RuntimeError;

pub(crate) fn ensure_productive_execution_platform() -> Result<(), RuntimeError> {
    let release = current_productive_execution_release();
    if release.as_deref().is_some_and(|release| {
        productive_execution_supported_release(
            std::env::consts::OS,
            std::env::consts::ARCH,
            release,
        )
    }) {
        Ok(())
    } else {
        Err(RuntimeError::ProductiveExecutionUnavailable)
    }
}

pub(crate) fn productive_execution_supported_release(
    target_os: &str,
    target_arch: &str,
    release: &str,
) -> bool {
    match (target_os, target_arch) {
        ("linux", "x86_64") => ubuntu_24_04_release(release),
        ("macos", "aarch64") => macos_26_release(release),
        _ => false,
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn current_productive_execution_release() -> Option<String> {
    std::fs::read_to_string("/etc/os-release").ok()
}

#[cfg(target_os = "macos")]
pub(crate) fn current_productive_execution_release() -> Option<String> {
    use std::process::{Command, Stdio};

    let output = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .env_clear()
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn current_productive_execution_release() -> Option<String> {
    None
}

fn ubuntu_24_04_release(release: &str) -> bool {
    let mut id = None;
    let mut version_id = None;
    for line in release.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                value
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(value);
        let slot = match key {
            "ID" => &mut id,
            "VERSION_ID" => &mut version_id,
            _ => continue,
        };
        if slot.replace(value).is_some() {
            return false;
        }
    }
    id == Some("ubuntu") && version_id == Some("24.04")
}

fn macos_26_release(release: &str) -> bool {
    let components = release.trim().split('.').collect::<Vec<_>>();
    components.len() >= 2
        && components[0] == "26"
        && components.iter().all(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        })
}
