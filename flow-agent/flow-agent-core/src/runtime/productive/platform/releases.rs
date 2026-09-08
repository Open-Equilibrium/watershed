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

pub(crate) fn productive_tool_execution_supported_release(
    target_os: &str,
    target_arch: &str,
    release: &str,
) -> bool {
    productive_execution_supported_release(target_os, target_arch, release)
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

#[cfg(test)]
mod tests {
    use super::{
        productive_execution_supported_release, productive_tool_execution_supported_release,
    };

    #[test]
    fn productive_execution_accepts_supported_releases() {
        assert!(productive_execution_supported_release(
            "linux",
            "x86_64",
            "ID=ubuntu\nVERSION_ID=24.04\n"
        ));
        assert!(productive_execution_supported_release(
            "macos", "aarch64", "26.0"
        ));
    }

    #[test]
    fn productive_execution_support_requires_the_exact_pinned_release() {
        assert!(productive_execution_supported_release(
            "linux",
            "x86_64",
            "ID=ubuntu\nVERSION_ID=\"24.04\"\n"
        ));
        for (target_os, target_arch, release) in [
            ("linux", "x86_64", "ID=ubuntu\nVERSION_ID=\"24.10\"\n"),
            ("linux", "x86_64", "ID=debian\nVERSION_ID=\"24.04\"\n"),
            ("linux", "x86_64", "ID=ubuntu\n"),
            ("macos", "aarch64", "25.9"),
            ("macos", "aarch64", "260"),
        ] {
            assert!(
                !productive_execution_supported_release(target_os, target_arch, release),
                "{target_os}/{target_arch}/{release:?} must be unavailable"
            );
        }
    }

    #[test]
    fn productive_execution_rejects_ambiguous_linux_release_metadata() {
        for release in [
            "ID=ubuntu\nID=ubuntu\nVERSION_ID=24.04\n",
            "ID=ubuntu\nVERSION_ID=24.04\nVERSION_ID=24.04\n",
            "ID=ubuntu\nVERSION_ID=24.04\nID=debian\n",
            "ID=ubuntu\nVERSION_ID='24.10'\n",
        ] {
            for supports in [
                productive_execution_supported_release,
                productive_tool_execution_supported_release,
            ] {
                assert!(
                    !supports("linux", "x86_64", release),
                    "ambiguous Linux release {release:?} must be unavailable"
                );
            }
        }
    }

    #[test]
    fn productive_execution_rejects_malformed_macos_versions() {
        for release in ["", "26", "26..0", "26.0.beta"] {
            for supports in [
                productive_execution_supported_release,
                productive_tool_execution_supported_release,
            ] {
                assert!(
                    !supports("macos", "aarch64", release),
                    "malformed macOS release {release:?} must be unavailable"
                );
            }
        }
    }

    #[test]
    fn productive_tool_execution_accepts_only_the_official_native_releases() {
        for (target_os, target_arch, release) in [
            ("linux", "x86_64", "ID=ubuntu\nVERSION_ID='24.04'\n"),
            ("macos", "aarch64", "26.0"),
            ("macos", "aarch64", "26.6.2\n"),
        ] {
            assert!(
                productive_tool_execution_supported_release(target_os, target_arch, release),
                "{target_os}/{target_arch}/{release:?} must admit native Tool preparation"
            );
        }
        for (target_os, target_arch, release) in [
            ("linux", "x86_64", "ID=ubuntu\nVERSION_ID=24.10\n"),
            ("linux", "aarch64", "ID=ubuntu\nVERSION_ID=24.04\n"),
            ("macos", "x86_64", "26.0"),
            ("macos", "aarch64", "25.9"),
            ("macos", "aarch64", "27.0"),
            ("windows", "x86_64", "26.0"),
            ("windows", "aarch64", "26.0"),
        ] {
            assert!(
                !productive_tool_execution_supported_release(target_os, target_arch, release),
                "{target_os}/{target_arch}/{release:?} must reject productive Tools"
            );
        }
    }
}
