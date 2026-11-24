use std::path::Path;

#[cfg(unix)]
pub(crate) fn owner_only(path: &Path, folder: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if folder { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("Could not protect {}: {e}", path.display()))
}

#[cfg(windows)]
pub(crate) fn owner_only(path: &Path, folder: bool) -> Result<(), String> {
    windows::owner_only(path, folder)
        .map_err(|code| format!("Could not protect {}: Windows error {code}", path.display()))
}

#[cfg(unix)]
fn signal(pid: u32, signal: rustix::process::Signal) {
    if let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process(pid, signal);
    }
}

#[cfg(unix)]
fn signal_tree(pid: u32, how: rustix::process::Signal) {
    match i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        Some(group) if rustix::process::kill_process_group(group, how).is_ok() => {}
        _ => signal(pid, how),
    }
}

#[cfg(unix)]
pub(crate) fn terminate(pid: u32) {
    signal_tree(pid, rustix::process::Signal::TERM);
}

#[cfg(windows)]
pub(crate) fn terminate(pid: u32) {
    windows::terminate(pid);
}

#[cfg(unix)]
pub(crate) fn kill(pid: u32) {
    signal_tree(pid, rustix::process::Signal::KILL);
}

#[cfg(windows)]
pub(crate) fn kill(pid: u32) {
    windows::terminate(pid);
}

#[cfg(unix)]
pub(crate) fn runs(
    pid: u32,
    binary: &Path,
    arguments: &[&str],
    _written: Option<std::time::SystemTime>,
) -> bool {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
    else {
        return false;
    };
    matches_command(&String::from_utf8_lossy(&output.stdout), binary, arguments)
}

#[cfg(any(unix, test))]
pub(crate) fn matches_command(command: &str, binary: &Path, arguments: &[&str]) -> bool {
    command
        .trim_end()
        .strip_prefix(&*binary.to_string_lossy())
        .is_some_and(|rest| rest.starts_with(' ') && rest.trim() == arguments.join(" "))
}

#[cfg(windows)]
pub(crate) fn runs(
    pid: u32,
    binary: &Path,
    _arguments: &[&str],
    written: Option<std::time::SystemTime>,
) -> bool {
    let same = windows::image(pid).is_some_and(|image| {
        std::fs::canonicalize(image).ok() == std::fs::canonicalize(binary).ok()
    });
    let older = match (windows::started(pid), written) {
        (Some(started), Some(written)) => started <= written,
        _ => false,
    };
    same && older
}

#[cfg(unix)]
pub(crate) struct Tree;

#[cfg(unix)]
pub(crate) fn hold_tree(_pid: u32) -> Option<Tree> {
    Some(Tree)
}

#[cfg(windows)]
pub(crate) use windows::Tree;

#[cfg(windows)]
pub(crate) fn hold_tree(pid: u32) -> Option<Tree> {
    windows::hold_tree(pid)
}

#[cfg(windows)]
mod windows {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        EXPLICIT_ACCESS_W, GRANT_ACCESS, SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW,
        TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, GetTokenInformation, NO_INHERITANCE, OBJECT_INHERIT_ACE,
        PROTECTED_DACL_SECURITY_INFORMATION, SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_QUERY,
        TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        QueryFullProcessImageNameW, TerminateProcess,
    };

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    fn open(pid: u32, access: u32) -> Option<Handle> {
        let process = unsafe { OpenProcess(access, 0, pid) };
        (!process.is_null()).then_some(Handle(process))
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn user() -> Result<Vec<u8>, u32> {
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(GetLastError());
            }
            let token = Handle(token);
            let mut size = 0u32;
            GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut size);
            let mut buffer = vec![0u8; size as usize];
            if GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            ) == 0
            {
                return Err(GetLastError());
            }
            Ok(buffer)
        }
    }

    pub(super) fn owner_only(path: &Path, folder: bool) -> Result<(), u32> {
        let token = user()?;
        unsafe {
            let owner = &*(token.as_ptr().cast::<TOKEN_USER>());
            let access = EXPLICIT_ACCESS_W {
                grfAccessPermissions: FILE_ALL_ACCESS,
                grfAccessMode: GRANT_ACCESS,
                grfInheritance: if folder {
                    SUB_CONTAINERS_AND_OBJECTS_INHERIT | OBJECT_INHERIT_ACE
                } else {
                    NO_INHERITANCE
                },
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: std::ptr::null_mut(),
                    MultipleTrusteeOperation: 0,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_USER,
                    ptstrName: owner.User.Sid.cast(),
                },
            };
            let mut acl: *mut ACL = std::ptr::null_mut();
            let made = SetEntriesInAclW(1, &access, std::ptr::null(), &mut acl);
            if made != 0 {
                return Err(made);
            }
            let name = wide(path);
            let set = SetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                acl,
                std::ptr::null(),
            );
            LocalFree(acl.cast());
            if set == 0 { Ok(()) } else { Err(set) }
        }
    }

    pub(crate) struct Tree(Handle);

    unsafe impl Send for Tree {}
    unsafe impl Sync for Tree {}

    pub(super) fn hold_tree(pid: u32) -> Option<Tree> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let job = Handle(job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).ok()?,
            ) == 0
            {
                return None;
            }
            let process = open(pid, PROCESS_TERMINATE | PROCESS_SET_QUOTA)?;
            (AssignProcessToJobObject(job.0, process.0) != 0).then_some(Tree(job))
        }
    }

    pub(super) fn terminate(pid: u32) {
        if let Some(process) = open(pid, PROCESS_TERMINATE) {
            unsafe {
                TerminateProcess(process.0, 1);
            }
        }
    }

    pub(super) fn started(pid: u32) -> Option<std::time::SystemTime> {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::GetProcessTimes;
        const UNIX_EPOCH_IN_TICKS: u64 = 116_444_736_000_000_000;
        let process = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        unsafe {
            let mut times: [FILETIME; 4] = std::mem::zeroed();
            let [created, exited, kernel, user] = &mut times;
            if GetProcessTimes(process.0, created, exited, kernel, user) == 0 {
                return None;
            }
            let ticks =
                (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
            let since = ticks.checked_sub(UNIX_EPOCH_IN_TICKS)?;
            Some(std::time::UNIX_EPOCH + std::time::Duration::from_nanos(since.saturating_mul(100)))
        }
    }

    pub(super) fn image(pid: u32) -> Option<PathBuf> {
        let process = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        unsafe {
            let mut buffer = vec![0u16; 32_768];
            let mut size = u32::try_from(buffer.len()).ok()?;
            if QueryFullProcessImageNameW(
                process.0,
                PROCESS_NAME_WIN32,
                buffer.as_mut_ptr(),
                &mut size,
            ) == 0
            {
                return None;
            }
            buffer.truncate(size as usize);
            Some(PathBuf::from(String::from_utf16_lossy(&buffer)))
        }
    }
}
