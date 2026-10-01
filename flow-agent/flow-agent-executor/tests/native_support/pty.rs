use std::{
    ffi::{CStr, c_char, c_void},
    fs::File,
    os::fd::FromRawFd,
    path::PathBuf,
};

pub struct Terminal {
    pub master: File,
    pub slave: File,
    pub path: PathBuf,
}

impl Terminal {
    pub fn new() -> Self {
        let (mut master, mut slave) = (-1, -1);
        let mut name = [0 as c_char; 128];
        // openpty initializes both owned descriptors and the native device name.
        let result = unsafe {
            openpty(
                &mut master,
                &mut slave,
                name.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
        let path = unsafe { CStr::from_ptr(name.as_ptr()) }
            .to_str()
            .unwrap()
            .into();
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        for file in [&master, &slave] {
            rustix::io::fcntl_setfd(file, rustix::io::FdFlags::CLOEXEC).unwrap();
        }
        Self {
            master,
            slave,
            path,
        }
    }
}

#[cfg_attr(target_os = "linux", link(name = "util"))]
unsafe extern "C" {
    fn openpty(
        master: *mut i32,
        slave: *mut i32,
        name: *mut c_char,
        termios: *const c_void,
        winsize: *const c_void,
    ) -> i32;
}
