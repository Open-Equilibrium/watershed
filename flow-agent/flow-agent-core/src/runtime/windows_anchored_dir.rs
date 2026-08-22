use cap_std::fs::Dir;
use std::{
    ffi::OsStr,
    fs, io, mem,
    os::windows::{
        ffi::OsStrExt as _,
        io::{AsRawHandle as _, FromRawHandle as _},
    },
    ptr,
};
use windows_sys::Wdk::{
    Foundation::OBJECT_ATTRIBUTES,
    Storage::FileSystem::{
        FILE_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT, FILE_RENAME_INFORMATION,
        FILE_SYNCHRONOUS_IO_NONALERT, FileRenameInformation, NtCreateFile, NtSetInformationFile,
    },
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, RtlNtStatusToDosError, STATUS_SUCCESS, UNICODE_STRING},
    Security::SECURITY_DESCRIPTOR,
    Storage::FileSystem::{
        DELETE, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
    System::IO::IO_STATUS_BLOCK,
};

const OBJECT_ATTRIBUTES_CASE_INSENSITIVE: u32 = 0x40;

pub(super) struct NativeDirectoryHandle(Option<HANDLE>);

impl NativeDirectoryHandle {
    fn into_file(mut self) -> fs::File {
        let handle = self.0.take().expect("native directory handle is present");
        unsafe { fs::File::from_raw_handle(handle as _) }
    }
}

impl Drop for NativeDirectoryHandle {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            unsafe {
                CloseHandle(handle);
            }
        }
    }
}

pub(super) enum NativeDirectoryOpenError {
    InvalidLeaf(io::Error),
    NtStatus(i32),
}

impl NativeDirectoryOpenError {
    pub(super) fn into_io_error(self) -> io::Error {
        match self {
            Self::InvalidLeaf(error) => error,
            Self::NtStatus(status) => {
                io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)
            }
        }
    }
}

pub(super) fn open_anchored_read_only(parent: &Dir, leaf: &str) -> io::Result<Dir> {
    let handle = open_native_anchored_directory(
        parent,
        leaf,
        ptr::null_mut(),
        FILE_OPEN,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
    )
    .map_err(NativeDirectoryOpenError::into_io_error)?;
    Ok(Dir::from_std_file(handle.into_file()))
}

pub(super) fn open_anchored_for_publication(parent: &Dir, leaf: &str) -> io::Result<Dir> {
    // WHY: retaining this no-delete-share handle prevents path replacement while DELETE lets the
    // same verified directory handle publish itself after population.
    let handle = open_native_anchored_directory_with_access(
        parent,
        leaf,
        ptr::null_mut(),
        FILE_OPEN,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
        FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
    )
    .map_err(NativeDirectoryOpenError::into_io_error)?;
    Ok(Dir::from_std_file(handle.into_file()))
}

pub(super) fn publish_anchored_directory(
    directory: &Dir,
    parent: &Dir,
    leaf: &str,
) -> io::Result<()> {
    let (leaf, byte_length) = anchored_leaf(leaf)?;
    let buffer_length = mem::size_of::<FILE_RENAME_INFORMATION>()
        .checked_add(usize::from(byte_length))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rename buffer is too long"))?;
    let word_count = buffer_length.div_ceil(mem::size_of::<usize>());
    let mut buffer = vec![0usize; word_count];
    let rename = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    unsafe {
        (*rename).Anonymous.ReplaceIfExists = false;
        (*rename).RootDirectory = parent.as_raw_handle() as HANDLE;
        (*rename).FileNameLength = u32::from(byte_length);
        ptr::copy_nonoverlapping(leaf.as_ptr(), (*rename).FileName.as_mut_ptr(), leaf.len());
    }
    let mut status_block = unsafe { mem::zeroed::<IO_STATUS_BLOCK>() };
    let status = unsafe {
        NtSetInformationFile(
            directory.as_raw_handle() as HANDLE,
            &mut status_block,
            rename.cast(),
            u32::try_from(buffer_length).expect("anchored rename buffer length fits u32"),
            FileRenameInformation,
        )
    };
    if status != STATUS_SUCCESS {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ));
    }
    Ok(())
}

pub(super) fn open_native_anchored_directory(
    parent: &Dir,
    leaf: &str,
    security_descriptor: *const SECURITY_DESCRIPTOR,
    disposition: u32,
    options: u32,
) -> Result<NativeDirectoryHandle, NativeDirectoryOpenError> {
    open_native_anchored_directory_with_access(
        parent,
        leaf,
        security_descriptor,
        disposition,
        options,
        FILE_GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )
}

fn open_native_anchored_directory_with_access(
    parent: &Dir,
    leaf: &str,
    security_descriptor: *const SECURITY_DESCRIPTOR,
    disposition: u32,
    options: u32,
    desired_access: u32,
    share_access: u32,
) -> Result<NativeDirectoryHandle, NativeDirectoryOpenError> {
    let (leaf, byte_length) = anchored_leaf(leaf).map_err(NativeDirectoryOpenError::InvalidLeaf)?;
    let name = UNICODE_STRING {
        Length: byte_length,
        MaximumLength: byte_length,
        Buffer: leaf.as_ptr().cast_mut(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle() as HANDLE,
        ObjectName: &name,
        Attributes: OBJECT_ATTRIBUTES_CASE_INSENSITIVE,
        SecurityDescriptor: security_descriptor,
        SecurityQualityOfService: ptr::null(),
    };
    let mut handle = ptr::null_mut();
    let mut status_block = unsafe { mem::zeroed::<IO_STATUS_BLOCK>() };
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            desired_access,
            &attributes,
            &mut status_block,
            ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            share_access,
            disposition,
            options,
            ptr::null(),
            0,
        )
    };
    if status != STATUS_SUCCESS {
        return Err(NativeDirectoryOpenError::NtStatus(status));
    }
    Ok(NativeDirectoryHandle(Some(handle)))
}

fn anchored_leaf(leaf: &str) -> io::Result<(Vec<u16>, u16)> {
    validate_anchored_leaf(leaf)?;
    let leaf = OsStr::new(leaf).encode_wide().collect::<Vec<_>>();
    let byte_length = leaf
        .len()
        .checked_mul(mem::size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "directory leaf is too long"))?;
    Ok((leaf, byte_length))
}

pub(super) fn validate_anchored_leaf(leaf: &str) -> io::Result<()> {
    if leaf.is_empty() || matches!(leaf, "." | "..") || leaf.contains(['/', '\\', ':', '\0']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory leaf must be one Windows filename component",
        ));
    }
    Ok(())
}
