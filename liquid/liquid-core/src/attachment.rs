use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::random_bytes;
use crate::{Error, Store, Workspace};

pub const MAX_ATTACHMENT: u64 = 5 << 30;

pub const PART: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttachmentId(String);

impl AttachmentId {
    pub fn parse(text: &str) -> Option<Self> {
        let stored = text.len() == 64
            && text
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        stored.then(|| Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AttachmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub id: AttachmentId,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Media {
    pub name: String,
    pub media_type: String,
    pub attachment: AttachmentId,
    pub size: u64,
}

pub struct Upload {
    key: String,
    session: automerge::ActorId,
    size: u64,
    written: u64,
    parts: u64,
    hasher: Sha256,
    buffer: Vec<u8>,
    failed: bool,
}

impl Upload {
    pub fn written(&self) -> u64 {
        self.written
    }
}

impl<S: Store> Workspace<S> {
    pub fn begin_upload(&mut self, actor: &crate::Actor, size: u64) -> Result<Upload, Error> {
        self.permit(actor)?;
        if size > MAX_ATTACHMENT {
            return Err(Error::TooLarge);
        }
        let key = random_bytes()?
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Upload {
            key,
            session: self.device.clone(),
            size,
            written: 0,
            parts: 0,
            hasher: Sha256::new(),
            buffer: Vec::with_capacity(PART.min(size as usize)),
            failed: false,
        })
    }

    pub fn upload(&mut self, upload: &mut Upload, mut bytes: &[u8]) -> Result<(), Error> {
        self.owns(upload)?;
        if upload.failed {
            return Err(failed());
        }
        if upload.written + bytes.len() as u64 > upload.size {
            return Err(Error::Changed);
        }
        upload.written += bytes.len() as u64;
        upload.hasher.update(bytes);
        while !bytes.is_empty() {
            let take = (PART - upload.buffer.len()).min(bytes.len());
            upload.buffer.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if upload.buffer.len() == PART {
                self.flush(upload)?;
            }
        }
        Ok(())
    }

    pub fn finish_upload(&mut self, mut upload: Upload) -> Result<Attachment, Error> {
        self.owns(&upload)?;
        if upload.failed {
            self.store.discard_upload(&upload.key)?;
            return Err(failed());
        }
        if upload.written != upload.size {
            self.store.discard_upload(&upload.key)?;
            return Err(Error::Changed);
        }
        if !upload.buffer.is_empty() {
            self.flush(&mut upload)?;
        }
        let id = AttachmentId(
            upload
                .hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        );
        self.store
            .finish_upload(&upload.key, &id, upload.size, upload.parts)?;
        Ok(Attachment {
            id,
            size: upload.size,
        })
    }

    pub fn discard_upload(&mut self, upload: Upload) -> Result<(), Error> {
        self.owns(&upload)?;
        self.store.discard_upload(&upload.key)
    }

    pub fn attachment_size(&mut self, id: &AttachmentId) -> Result<Option<u64>, Error> {
        self.store.attachment_size(id)
    }

    pub fn attachment_part(
        &mut self,
        id: &AttachmentId,
        index: u64,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.store.part(id, index)
    }

    fn owns(&self, upload: &Upload) -> Result<(), Error> {
        if upload.session == self.device {
            Ok(())
        } else {
            Err(Error::Storage(
                "the upload belongs to another Workspace session".into(),
            ))
        }
    }

    fn flush(&mut self, upload: &mut Upload) -> Result<(), Error> {
        let put = self
            .store
            .put_part(&upload.key, upload.parts, &upload.buffer);
        upload.failed = put.is_err();
        put?;
        upload.parts += 1;
        upload.buffer.clear();
        Ok(())
    }
}

fn failed() -> Error {
    Error::Storage("the store refused a part of this upload; add the file again".into())
}
