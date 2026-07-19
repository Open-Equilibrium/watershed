/// Returns whether `value` is a valid v0 block id.
pub fn is_valid_block_id(value: &str) -> bool {
    proto::is_valid_session_id(value)
}

/// Returns whether `value` is a valid predefined command id.
pub fn is_valid_command_id(value: &str) -> bool {
    matches_lower_token(value, 1, 64)
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
}

/// Returns whether `value` is a valid allowed-parameter name.
pub fn is_valid_allowed_parameter_name(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("--") else {
        return false;
    };
    let mut bytes = rest.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Returns whether `value` is a canonical IPv4 or IPv6 CIDR.
pub fn is_valid_canonical_cidr(value: &str) -> bool {
    let Some((addr, prefix)) = value.split_once('/') else {
        return false;
    };
    if prefix.len() > 1 && prefix.starts_with('0') {
        return false;
    }
    if value.matches('/').count() != 1 {
        return false;
    }

    let Ok(prefix) = prefix.parse::<u8>() else {
        return false;
    };
    match addr.parse::<IpAddr>() {
        Ok(IpAddr::V4(addr)) => {
            prefix <= 32
                && host_bits_are_zero_v4(addr, prefix)
                && value == format!("{addr}/{prefix}")
        }
        Ok(IpAddr::V6(addr)) => {
            prefix <= 128
                && host_bits_are_zero_v6(addr, prefix)
                && value == format!("{addr}/{prefix}")
        }
        Err(_) => false,
    }
}

/// Normalizes a safe slash-separated relative path or rejects unsafe aliases.
pub fn normalize_safe_relative_path(value: &str) -> Option<String> {
    if value.is_empty()
        || value.starts_with('/')
        || has_windows_drive_prefix(value)
        || value.contains('\\')
    {
        return None;
    }

    for component in value.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return None;
        }
        if path_component_has_windows_alias(component)
            || path_component_has_windows_invalid_character(component)
        {
            return None;
        }
    }
    Some(value.to_owned())
}

/// Returns whether `path` is equal to or contained under `scope`.
pub fn relative_path_is_inside_scope(path: &str, scope: &str) -> bool {
    path == scope
        || path
            .strip_prefix(scope)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

/// Returns whether any path component would alias a Windows device or trimmed name.
pub fn relative_path_has_windows_alias(value: &str) -> bool {
    value.split('/').any(|component| {
        !matches!(component, "" | "." | "..") && path_component_has_windows_alias(component)
    })
}

fn has_windows_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn path_component_has_windows_alias(component: &str) -> bool {
    if component.ends_with('.') || component.ends_with(' ') {
        return true;
    }
    let basename = component
        .split_once('.')
        .map_or(component, |(basename, _)| basename);
    let uppercase = basename.to_ascii_uppercase();
    matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || uppercase
            .strip_prefix("COM")
            .or_else(|| uppercase.strip_prefix("LPT"))
            .is_some_and(|digit| {
                matches!(
                    digit,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
}

fn path_component_has_windows_invalid_character(component: &str) -> bool {
    component
        .bytes()
        .any(|byte| byte < b' ' || matches!(byte, b'<' | b'>' | b':' | b'"' | b'|' | b'?' | b'*'))
}

fn matches_lower_token(value: &str, min_len: usize, max_len: usize) -> bool {
    value.len() >= min_len
        && value.len() <= max_len
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

fn host_bits_are_zero_v4(addr: Ipv4Addr, prefix: u8) -> bool {
    let value = u32::from(addr);
    match 32 - prefix {
        0 => true,
        32 => value == 0,
        host_bits => {
            let host_mask = (1u32 << host_bits) - 1;
            value & host_mask == 0
        }
    }
}

fn host_bits_are_zero_v6(addr: Ipv6Addr, prefix: u8) -> bool {
    let value = u128::from(addr);
    match 128 - prefix {
        0 => true,
        128 => value == 0,
        host_bits => {
            let host_mask = (1u128 << host_bits) - 1;
            value & host_mask == 0
        }
    }
}
