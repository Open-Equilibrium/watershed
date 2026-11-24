use std::fs::File;
use std::io::{Read, Write};

use liquid_core::{Actor, AttachmentId, Media, PART, Upload};

use super::replica::LiquidWorkspace;
use super::workspace::message;
use crate::frb_generated::StreamSink;
use crate::keys::random_hex;

pub const MAX_SHOWN: i64 = 100 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMedia {
    pub name: String,
    pub media_type: String,
    pub attachment: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeUpload {
    pub written: i64,
    pub size: i64,
    pub attachment: Option<String>,
}

fn attachment(name: &str) -> Result<AttachmentId, String> {
    AttachmentId::parse(name).ok_or_else(|| "attachment is no attachment name".to_string())
}

fn bytes(size: u64) -> i64 {
    i64::try_from(size).unwrap_or(i64::MAX)
}

impl LiquidWorkspace {
    pub fn add_file(&self, path: String, sink: StreamSink<BridgeUpload>) -> Result<(), String> {
        let refused = sink.clone();
        if let Err(error) = self.add_file_with(path, |upload| sink.add(upload).is_ok()) {
            let _ = refused.add_error(error);
        }
        Ok(())
    }

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

    pub fn save_attachment(&self, attachment: String, path: String) -> Result<(), String> {
        let id = self::attachment(&attachment)?;
        self.size(&id)?;
        let failed = |error: std::io::Error| format!("{path}: {error}");
        let target = std::path::Path::new(&path);
        let partial = target.with_file_name(format!(".liquid-saving-{}", random_hex()?));
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

    #[flutter_rust_bridge::frb(ignore)]
    pub(crate) fn each_part(
        &self,
        id: &AttachmentId,
        mut take: impl FnMut(Vec<u8>) -> Result<(), String>,
    ) -> Result<(), String> {
        let size = self.size(id)?;
        let mut index = 0;
        let mut total = 0;
        loop {
            let part = self
                .state()?
                .workspace
                .attachment_part(id, index)
                .map_err(message)?;
            let Some(part) = part else {
                break;
            };
            total += part.len() as u64;
            take(part)?;
            index += 1;
        }
        if total != size {
            return Err("the file changed while it was read".into());
        }
        Ok(())
    }
}

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
