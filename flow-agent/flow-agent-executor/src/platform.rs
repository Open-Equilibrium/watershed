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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn statically_linked_self() -> bool {
    std::fs::read("/proc/self/exe")
        .ok()
        .is_some_and(|image| elf_has_no_interpreter(&image))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn elf_has_no_interpreter(image: &[u8]) -> bool {
    const ELF_HEADER_BYTES: usize = 64;
    const PROGRAM_HEADER_BYTES: usize = 56;
    const PT_INTERP: u32 = 3;

    if image.len() < ELF_HEADER_BYTES || &image[..4] != b"\x7fELF" || image[4] != 2 || image[5] != 1
    {
        return false;
    }
    let program_offset = u64::from_le_bytes(image[32..40].try_into().unwrap_or_default());
    let entry_size = u16::from_le_bytes(image[54..56].try_into().unwrap_or_default()) as usize;
    let entries = u16::from_le_bytes(image[56..58].try_into().unwrap_or_default()) as usize;
    if entry_size < PROGRAM_HEADER_BYTES {
        return false;
    }
    let Ok(program_offset) = usize::try_from(program_offset) else {
        return false;
    };
    for index in 0..entries {
        let Some(offset) = index
            .checked_mul(entry_size)
            .and_then(|bytes| program_offset.checked_add(bytes))
        else {
            return false;
        };
        let Some(kind) = image.get(offset..offset.saturating_add(4)) else {
            return false;
        };
        if u32::from_le_bytes(kind.try_into().unwrap_or_default()) == PT_INTERP {
            return false;
        }
    }
    true
}
