use std::{
    ffi::{c_int, c_void},
    io,
    os::fd::AsRawFd,
    ptr,
};

const ACL_TYPE_EXTENDED: c_int = 0x0000_0100;
const ACL_FIRST_ENTRY: c_int = 0;
const ACL_NEXT_ENTRY: c_int = -1;
const ACL_MAX_ENTRIES: usize = 128;
const ACL_ALLOW: c_int = 1;
const ACL_DENY: c_int = 2;
const ACL_ONLY_INHERIT: c_int = 1 << 8;
// Darwin sys/acl.h and sys/kauth.h: data, namespace and security mutation.
const MUTATION: u64 =
    (1 << 2) | (1 << 4) | (1 << 5) | (1 << 6) | (1 << 8) | (1 << 10) | (1 << 12) | (1 << 13);
const GENERIC_WRITE: u64 = 1 << 23;
const GENERIC_ALL: u64 = 1 << 21;
const KNOWN_PERMISSIONS: u64 = 0x3ffe | (1 << 20) | (0xf << 21);
const WELL_KNOWN_PREFIX: [u8; 12] = [
    0xab, 0xcd, 0xef, 0xab, 0xcd, 0xef, 0xab, 0xcd, 0xef, 0xab, 0xcd, 0xef,
];
const EINVAL: i32 = 22;
const ENOENT: i32 = 2;

type Acl = *mut c_void;

unsafe extern "C" {
    fn acl_free(object: *mut c_void) -> c_int;
    fn acl_get_entry(acl: Acl, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
    fn acl_get_fd_np(fd: c_int, acl_type: c_int) -> Acl;
    fn acl_valid(acl: Acl) -> c_int;
    fn acl_get_tag_type(entry: *mut c_void, tag: *mut c_int) -> c_int;
    fn acl_get_permset_mask_np(entry: *mut c_void, permissions: *mut u64) -> c_int;
    fn acl_get_flagset_np(entry: *mut c_void, flags: *mut *mut c_void) -> c_int;
    fn acl_get_flag_np(flags: *mut c_void, flag: c_int) -> c_int;
    fn acl_get_qualifier(entry: *mut c_void) -> *mut c_void;
    fn mbr_uuid_to_id(uuid: *const u8, id: *mut u32, id_type: *mut c_int) -> c_int;
    fn acl_init(count: c_int) -> Acl;
    fn acl_set_fd_np(fd: c_int, acl: Acl, acl_type: c_int) -> c_int;
}

/// Prove that each mutating grant applies only to root/current installation owner,
/// or is blocked by an earlier deny for that principal or everyone. Membership
/// in any other group cannot establish that all of its users are trusted.
pub(crate) fn ensure_no_other_user_mutation(
    opened: &impl AsRawFd,
    effective_uid: u32,
) -> io::Result<()> {
    let Some(acl) = OwnedAcl::read(opened)? else {
        return Ok(());
    };
    if unsafe { acl_valid(acl.0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut denied = Vec::<([u8; 16], u64)>::new();
    let mut everybody_denied = 0;
    for index in 0..=ACL_MAX_ENTRIES {
        let mut entry = ptr::null_mut();
        let result = unsafe {
            acl_get_entry(
                acl.0,
                if index == 0 {
                    ACL_FIRST_ENTRY
                } else {
                    ACL_NEXT_ENTRY
                },
                &mut entry,
            )
        };
        if result == -1 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(EINVAL) {
                Ok(())
            } else {
                Err(error)
            };
        }
        if result != 0 || entry.is_null() || index == ACL_MAX_ENTRIES {
            return Err(io::Error::other("invalid macOS ACL entry inventory"));
        }
        let mut tag = 0;
        let mut permissions = 0;
        let mut flags = ptr::null_mut();
        if unsafe { acl_get_tag_type(entry, &mut tag) } != 0
            || unsafe { acl_get_permset_mask_np(entry, &mut permissions) } != 0
            || unsafe { acl_get_flagset_np(entry, &mut flags) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if !matches!(tag, ACL_ALLOW | ACL_DENY) || permissions & !KNOWN_PERMISSIONS != 0 {
            return Err(io::Error::other("unsupported macOS ACL metadata"));
        }
        let inherit_only = unsafe { acl_get_flag_np(flags, ACL_ONLY_INHERIT) };
        match inherit_only {
            1 => continue,
            0 => {}
            _ => return Err(io::Error::last_os_error()),
        }
        // kauth_acl_evaluate expands generic write/all before ordered evaluation.
        if permissions & (GENERIC_WRITE | GENERIC_ALL) != 0 {
            permissions |= MUTATION & !(1 << 13);
        }
        let mutation = permissions & MUTATION;
        if mutation == 0 {
            continue;
        }
        let qualifier = unsafe { acl_get_qualifier(entry) };
        if qualifier.is_null() {
            return Err(io::Error::last_os_error());
        }
        let qualifier = OwnedAcl(qualifier);
        // acl_get_qualifier returns an owned Darwin uuid_t (16 bytes).
        let principal = unsafe { *qualifier.0.cast::<[u8; 16]>() };
        let well_known = if principal[..12] == WELL_KNOWN_PREFIX {
            u32::from_be_bytes(principal[12..].try_into().expect("UUID tail"))
        } else {
            0
        };
        if well_known == 0xffff_fffe {
            continue;
        } // Nobody never applies.
        if tag == ACL_DENY {
            if well_known == 12 {
                everybody_denied |= mutation;
            } else {
                denied.push((principal, mutation));
            }
            continue;
        }
        let blocked = denied
            .iter()
            .filter(|(uuid, _)| *uuid == principal)
            .fold(everybody_denied, |bits, (_, permissions)| {
                bits | permissions
            });
        if mutation & !blocked == 0 || well_known == 10 {
            continue;
        } // Owner is already trusted.
        let mut id = 0;
        let mut id_type = -1;
        if well_known == 0
            && unsafe { mbr_uuid_to_id(principal.as_ptr(), &mut id, &mut id_type) } == 0
            && id_type == 0
            && (id == 0 || id == effective_uid)
        {
            continue;
        }
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "macOS ACL permits mutation by other users",
        ));
    }
    unreachable!("bounded ACL iteration returns at its limit")
}

struct OwnedAcl(Acl);

impl OwnedAcl {
    fn read(opened: &impl AsRawFd) -> io::Result<Option<Self>> {
        let acl = unsafe { acl_get_fd_np(opened.as_raw_fd(), ACL_TYPE_EXTENDED) };
        if !acl.is_null() {
            return Ok(Some(Self(acl)));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ENOENT) {
            Ok(None)
        } else {
            Err(error)
        }
    }
}

impl Drop for OwnedAcl {
    fn drop(&mut self) {
        unsafe {
            acl_free(self.0);
        }
    }
}

pub(crate) fn has_entries(opened: &impl AsRawFd) -> io::Result<bool> {
    let Some(acl) = OwnedAcl::read(opened)? else {
        return Ok(false);
    };
    let mut entry = ptr::null_mut();
    let result = unsafe { acl_get_entry(acl.0, ACL_FIRST_ENTRY, &mut entry) };
    match result {
        0 => Ok(true),
        -1 => {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(EINVAL) {
                Ok(false)
            } else {
                Err(error)
            }
        }
        _ => Err(io::Error::other(
            "macOS ACL query returned an invalid result",
        )),
    }
}

pub(crate) fn clear_entries(opened: &impl AsRawFd) -> io::Result<()> {
    if !has_entries(opened)? {
        return Ok(());
    }
    let empty_acl = unsafe { acl_init(0) };
    if empty_acl.is_null() {
        return Err(io::Error::last_os_error());
    }
    let empty_acl = OwnedAcl(empty_acl);
    if unsafe { acl_set_fd_np(opened.as_raw_fd(), empty_acl.0, ACL_TYPE_EXTENDED) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if has_entries(opened)? {
        return Err(io::Error::other("macOS ACL removal did not clear entries"));
    }
    Ok(())
}
