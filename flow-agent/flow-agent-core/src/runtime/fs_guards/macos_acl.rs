use std::{
    ffi::{c_int, c_void},
    io,
    os::fd::AsRawFd,
    ptr,
};

const ACL_TYPE_EXTENDED: c_int = 0x0000_0100;
const ACL_FIRST_ENTRY: c_int = 0;
const EINVAL: i32 = 22;
const ENOENT: i32 = 2;

type Acl = *mut c_void;

unsafe extern "C" {
    fn acl_free(object: *mut c_void) -> c_int;
    fn acl_get_entry(acl: Acl, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
    fn acl_get_fd_np(fd: c_int, acl_type: c_int) -> Acl;
    fn acl_init(count: c_int) -> Acl;
    fn acl_set_fd_np(fd: c_int, acl: Acl, acl_type: c_int) -> c_int;
}

struct OwnedAcl(Acl);

impl Drop for OwnedAcl {
    fn drop(&mut self) {
        unsafe {
            acl_free(self.0);
        }
    }
}

pub(crate) fn has_entries(opened: &impl AsRawFd) -> io::Result<bool> {
    let acl = unsafe { acl_get_fd_np(opened.as_raw_fd(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(ENOENT) {
            Ok(false)
        } else {
            Err(error)
        };
    }
    let acl = OwnedAcl(acl);
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
