use crate::runtime::windows_anchored_dir::{
    NativeDirectoryOpenError, open_native_anchored_directory,
};
use cap_std::fs::Dir;
use std::{
    ffi::{OsStr, c_void},
    fs, io, mem,
    os::windows::{ffi::OsStrExt as _, io::AsRawHandle as _},
    path::Path,
    ptr, slice,
};
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_CREATE, FILE_DIRECTORY_FILE, FILE_SYNCHRONOUS_IO_NONALERT,
};
use windows_sys::Win32::Security::{
    Authorization::{SetNamedSecurityInfoW, SetSecurityInfo},
    GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, ERROR_INSUFFICIENT_BUFFER, HANDLE, LocalFree,
        STATUS_OBJECT_NAME_COLLISION,
    },
    Security::{
        ACCESS_ALLOWED_ACE, ACL_SIZE_INFORMATION, AclSizeInformation,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
        },
        CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation,
        GetSecurityDescriptorControl, GetTokenInformation, INHERITED_ACE, OBJECT_INHERIT_ACE,
        OWNER_SECURITY_INFORMATION, PSID, SE_DACL_PRESENT, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
        TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::{CreateDirectoryW, FILE_ALL_ACCESS},
    System::{
        SystemServices::ACCESS_ALLOWED_ACE_TYPE,
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct CurrentUserSid {
    token_information: Vec<usize>,
}

impl CurrentUserSid {
    fn get() -> io::Result<Self> {
        let mut token = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = OwnedHandle(token);

        let mut required = 0;
        if unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut required) }
            != 0
            || io::Error::last_os_error().raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32)
        {
            return Err(io::Error::last_os_error());
        }
        if required < mem::size_of::<TOKEN_USER>() as u32 {
            return Err(invalid_security_data(
                "current-user token information is too short",
            ));
        }

        let word_count = (required as usize).div_ceil(mem::size_of::<usize>());
        let mut token_information = vec![0usize; word_count];
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                token_information.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let current = Self { token_information };
        if current.as_ptr().is_null() {
            return Err(invalid_security_data("current-user token has no SID"));
        }
        Ok(current)
    }

    fn as_ptr(&self) -> PSID {
        unsafe {
            (*(self.token_information.as_ptr().cast::<TOKEN_USER>()))
                .User
                .Sid
        }
    }

    fn as_sddl(&self) -> io::Result<String> {
        let mut encoded = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(self.as_ptr(), &mut encoded) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let encoded = LocalAllocation(encoded.cast());
        let mut len = 0;
        unsafe {
            while *encoded.0.cast::<u16>().add(len) != 0 {
                len += 1;
            }
            String::from_utf16(slice::from_raw_parts(encoded.0.cast(), len))
                .map_err(|_| invalid_security_data("current-user SID string is invalid"))
        }
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn invalid_security_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn security_descriptor(sddl: &str) -> io::Result<LocalAllocation> {
    let encoded = wide(OsStr::new(sddl));
    let mut descriptor = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            encoded.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(LocalAllocation(descriptor))
}

fn private_directory_security_descriptor() -> io::Result<LocalAllocation> {
    let current_user = CurrentUserSid::get()?;
    let sid = current_user.as_sddl()?;
    security_descriptor(&format!("O:{sid}D:P(A;OICI;FA;;;{sid})"))
}

pub(super) fn create(path: &Path) -> io::Result<()> {
    let descriptor = private_directory_security_descriptor()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let path = wide(path.as_os_str());
    if unsafe { CreateDirectoryW(path.as_ptr(), &attributes) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn create_anchored(parent: &Dir, leaf: &str) -> io::Result<()> {
    let descriptor = private_directory_security_descriptor()?;
    open_native_anchored_directory(
        parent,
        leaf,
        descriptor.0.cast(),
        FILE_CREATE,
        FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
    )
    .map(drop)
    .map_err(|error| match error {
        NativeDirectoryOpenError::NtStatus(STATUS_OBJECT_NAME_COLLISION) => {
            io::Error::from_raw_os_error(ERROR_ALREADY_EXISTS as i32)
        }
        error => error.into_io_error(),
    })
}

pub(super) fn opened_is_current_user_only(dir: &Dir) -> io::Result<bool> {
    opened_handle_is_current_user_only(
        dir.as_raw_handle() as HANDLE,
        (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as u8,
    )
}

fn opened_handle_is_current_user_only(handle: HANDLE, expected_ace_flags: u8) -> io::Result<bool> {
    opened_handle_has_current_user_only_access(handle, expected_ace_flags, true)
}

fn opened_handle_has_current_user_only_access(
    handle: HANDLE,
    expected_ace_flags: u8,
    require_protected_dacl: bool,
) -> io::Result<bool> {
    let mut owner = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalAllocation(descriptor);

    let mut control = 0;
    let mut revision = 0;
    if unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if control & SE_DACL_PRESENT == 0
        || (require_protected_dacl && control & SE_DACL_PROTECTED == 0)
        || owner.is_null()
        || dacl.is_null()
    {
        return Ok(false);
    }

    let current_user = CurrentUserSid::get()?;
    if unsafe { EqualSid(owner, current_user.as_ptr()) } == 0 {
        return Ok(false);
    }

    let mut size = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    if unsafe {
        GetAclInformation(
            dacl,
            (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
            mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if size.AceCount != 1 {
        return Ok(false);
    }

    let mut raw_ace = ptr::null_mut();
    if unsafe { GetAce(dacl, 0, &mut raw_ace) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let ace = unsafe { &*raw_ace.cast::<ACCESS_ALLOWED_ACE>() };
    let flags_match = if require_protected_dacl {
        ace.Header.AceFlags == expected_ace_flags
    } else {
        ace.Header.AceFlags == 0 || u32::from(ace.Header.AceFlags) == INHERITED_ACE
    };
    if ace.Header.AceType != ACCESS_ALLOWED_ACE_TYPE as u8
        || !flags_match
        || ace.Mask != FILE_ALL_ACCESS
    {
        return Ok(false);
    }
    let ace_sid = (&ace.SidStart as *const u32).cast_mut().cast();
    Ok(unsafe { EqualSid(ace_sid, current_user.as_ptr()) } != 0)
}

pub(super) fn file_is_current_user_only(path: &Path) -> io::Result<bool> {
    let file = fs::File::open(path)?;
    opened_handle_is_current_user_only(file.as_raw_handle() as HANDLE, 0)
}

#[cfg(test)]
pub(super) fn file_has_current_user_only_access(path: &Path) -> io::Result<bool> {
    let file = fs::File::open(path)?;
    opened_handle_has_current_user_only_access(file.as_raw_handle() as HANDLE, 0, false)
}

pub(super) fn opened_file_is_current_user_only(file: &fs::File) -> io::Result<bool> {
    opened_handle_is_current_user_only(file.as_raw_handle() as HANDLE, 0)
}

pub(super) fn set_opened_file_current_user_only(file: &fs::File) -> io::Result<()> {
    let current_user = CurrentUserSid::get()?;
    let sid = current_user.as_sddl()?;
    let descriptor = security_descriptor(&format!("D:P(A;;FA;;;{sid})"))?;
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl = ptr::null_mut();
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    if present == 0 || dacl.is_null() {
        return Err(invalid_security_data(
            "private file security descriptor has no DACL",
        ));
    }
    let status = unsafe {
        SetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            current_user.as_ptr(),
            ptr::null_mut(),
            dacl,
            ptr::null_mut(),
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

pub(super) fn set_file_current_user_only(path: &Path) -> io::Result<()> {
    let current_user = CurrentUserSid::get()?;
    let sid = current_user.as_sddl()?;
    set_named_dacl(path, &format!("D:P(A;;FA;;;{sid})"), current_user.as_ptr())
}

#[cfg(test)]
pub(super) fn set_world_access(path: &Path) -> io::Result<()> {
    set_named_dacl(path, "D:P(A;OICI;FA;;;WD)", ptr::null_mut())
}

fn set_named_dacl(path: &Path, sddl: &str, owner: PSID) -> io::Result<()> {
    let descriptor = security_descriptor(sddl)?;
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl = ptr::null_mut();
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    if present == 0 || dacl.is_null() {
        return Err(invalid_security_data(
            "test security descriptor has no DACL",
        ));
    }
    let path = wide(path.as_os_str());
    let mut security_information = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
    if !owner.is_null() {
        security_information |= OWNER_SECURITY_INFORMATION;
    }
    let status = unsafe {
        SetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            security_information,
            owner,
            ptr::null_mut(),
            dacl,
            ptr::null_mut(),
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}
