//! Attachments through the bridge (ADR-0305): a file's bytes go in from its path, read in
//! Rust so they never cross to Dart, or from the clipboard, and come out to a path the person
//! chose or, for an image Liquid shows, as bytes. Each part takes the Workspace's lock on
//! its own, so edits go on while a large file goes in.

use std::fs::File;
use std::io::{Read, Write};

use liquid_core::{Actor, AttachmentId, Media, PART, Upload};

use super::replica::LiquidWorkspace;
use super::workspace::message;
use crate::frb_generated::StreamSink;
use crate::keys::random_hex;

/// The largest attachment Liquid reads whole, to show it as an image; a larger image shows
/// as a file.
pub const MAX_SHOWN: i64 = 100 << 20;

/// A named file: a Media Block's, or one of a file cell's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMedia {
    /// The file's name.
    pub name: String,
    /// The file's media type, such as `image/png`; empty if unknown.
    pub media_type: String,
    /// Its attachment's name.
    pub attachment: String,
    /// Its size in bytes.
    pub size: i64,
}

impl BridgeMedia {
    pub(crate) fn to_core(&self) -> Result<Media, String> {
        Ok(Media {
            name: self.name.clone(),
            media_type: self.media_type.clone(),
            attachment: attachment(&self.attachment)?,
            size: u64::try_from(self.size).unwrap_or(0),
        })
    }

    pub(crate) fn of(media: Media) -> Self {
        Self {
            name: media.name,
            media_type: media.media_type,
            attachment: media.attachment.to_string(),
            size: i64::try_from(media.size).unwrap_or(i64::MAX),
        }
    }
}

/// How far a file has gone in; the last event names its finished attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeUpload {
    /// The bytes in so far.
    pub written: i64,
    /// The file's size.
    pub size: i64,
    /// The finished attachment's name, in the last event only.
    pub attachment: Option<String>,
}

fn attachment(name: &str) -> Result<AttachmentId, String> {
    AttachmentId::parse(name).ok_or_else(|| "attachment is no attachment name".to_string())
}

fn bytes(size: u64) -> i64 {
    i64::try_from(size).unwrap_or(i64::MAX)
}

impl LiquidWorkspace {
    /// Adds the file at `path` as an attachment, sending its progress and, last, its name;
    /// a refusal arrives as the stream's error. Add a Media Block for it with
    /// `AddMediaBlock`.
    pub fn add_file(&self, path: String, sink: StreamSink<BridgeUpload>) -> Result<(), String> {
        let refused = sink.clone();
        if let Err(error) = self.add_file_with(path, |upload| sink.add(upload).is_ok()) {
            let _ = refused.add_error(error);
        }
        Ok(())
    }

    /// Adds the file at `path`, handing each step to `deliver`; one that returns `false`
    /// stops the upload. Tests use it instead of a Dart stream.
    #[flutter_rust_bridge::frb(ignore)]
    pub(crate) fn add_file_with(
        &self,
        path: String,
        mut deliver: impl FnMut(BridgeUpload) -> bool,
    ) -> Result<(), String> {
        let failed = |error: std::io::Error| format!("{path}: {error}");
        let mut file = File::open(&path).map_err(failed)?;
        let size = file.metadata().map_err(failed)?.len();
        let mut upload = self.begin(size)?;
        let mut buffer = vec![0; PART];
        loop {
            let read = match fill(&mut file, &mut buffer) {
                Ok(read) => read,
                Err(error) => return Err(self.discard(upload, failed(error))),
            };
            if read == 0 {
                break;
            }
            let written = self
                .state()
                .and_then(|mut inner| {
                    inner
                        .workspace
                        .upload(&mut upload, &buffer[..read])
                        .map_err(message)
                })
                .map(|()| upload.written());
            let written = match written {
                Ok(written) => written,
                Err(error) => return Err(self.discard(upload, error)),
            };
            let progress = BridgeUpload {
                written: bytes(written),
                size: bytes(size),
                attachment: None,
            };
            if !deliver(progress) {
                let _ = self.discard(upload, String::new());
                return Ok(());
            }
        }
        let finished = self
            .state()?
            .workspace
            .finish_upload(upload)
            .map_err(message)?;
        deliver(BridgeUpload {
            written: bytes(finished.size),
            size: bytes(finished.size),
            attachment: Some(finished.id.to_string()),
        });
        Ok(())
    }

    /// Adds pasted bytes as an attachment and answers its name.
    pub fn add_bytes(&self, bytes: Vec<u8>) -> Result<String, String> {
        let mut upload = self.begin(bytes.len() as u64)?;
        for part in bytes.chunks(PART) {
            let mut inner = self.state()?;
            if let Err(error) = inner.workspace.upload(&mut upload, part) {
                drop(inner);
                return Err(self.discard(upload, message(error)));
            }
        }
        let finished = self
            .state()?
            .workspace
            .finish_upload(upload)
            .map_err(message)?;
        Ok(finished.id.to_string())
    }

    /// The bytes of an attachment of at most [`MAX_SHOWN`] bytes, to show it as an image or a PDF.
    pub fn read_attachment(&self, attachment: String) -> Result<Vec<u8>, String> {
        let id = self::attachment(&attachment)?;
        let size = self.size(&id)?;
        if size > MAX_SHOWN as u64 {
            return Err("too large to show".into());
        }
        let mut bytes = Vec::with_capacity(size as usize);
        self.each_part(&id, |part| {
            bytes.extend_from_slice(&part);
            Ok(())
        })?;
        Ok(bytes)
    }

    /// Writes an attachment's bytes to `path`, which the person chose in the save dialog,
    /// replacing what is there only once the whole copy is written: it goes to a new hidden
    /// file of its own beside `path` first, so a save that fails leaves `path` as it was.
    pub fn save_attachment(&self, attachment: String, path: String) -> Result<(), String> {
        let id = self::attachment(&attachment)?;
        self.size(&id)?;
        let failed = |error: std::io::Error| format!("{path}: {error}");
        let target = std::path::Path::new(&path);
        // A short name, so a target name as long as the file system allows still fits.
        let partial = target.with_file_name(format!(".liquid-saving-{}", random_hex()?));
        // Only a file this call created is written, renamed or removed.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(failed)?;
        let written = self
            .each_part(&id, |part| file.write_all(&part).map_err(failed))
            .and_then(|()| file.sync_all().map_err(failed));
        drop(file);
        let renamed = written.and_then(|()| std::fs::rename(&partial, target).map_err(failed));
        if renamed.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
        renamed
    }

    fn begin(&self, size: u64) -> Result<Upload, String> {
        let mut inner = self.state()?;
        let owner = Actor::Person(inner.workspace.owner().clone());
        inner.workspace.begin_upload(&owner, size).map_err(message)
    }

    /// Drops an unfinished upload and answers `error`.
    fn discard(&self, upload: Upload, error: String) -> String {
        if let Ok(mut inner) = self.state() {
            let _ = inner.workspace.discard_upload(upload);
        }
        error
    }

    fn size(&self, id: &AttachmentId) -> Result<u64, String> {
        self.state()?
            .workspace
            .attachment_size(id)
            .map_err(message)?
            .ok_or_else(|| "not found".to_string())
    }

    /// Hands each part to `take`, taking the lock for one part at a time.
    fn each_part(
        &self,
        id: &AttachmentId,
        mut take: impl FnMut(Vec<u8>) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut index = 0;
        loop {
            let part = self
                .state()?
                .workspace
                .attachment_part(id, index)
                .map_err(message)?;
            let Some(part) = part else {
                return Ok(());
            };
            take(part)?;
            index += 1;
        }
    }
}

/// Reads until `buffer` is full or the file ends, and answers how much it read.
fn fill(file: &mut File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut read = 0;
    while read < buffer.len() {
        match file.read(&mut buffer[read..]) {
            Ok(0) => break,
            Ok(more) => read += more,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(read)
}
