//! Attachments (ADR-0305): a file's bytes, kept in the store in parts of at most [`PART`]
//! bytes outside every document and named by their SHA-256, so one file added twice is kept
//! once. A Media Block names its attachment; the bytes stay until purge (ADR-0250).

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::random_bytes;
use crate::{Error, Store, Workspace};

/// The largest attachment the core accepts: 5 GiB, Notion's limit per file.
pub const MAX_ATTACHMENT: u64 = 5 << 30;

/// The size of an attachment's parts; the last one may be shorter.
pub const PART: usize = 1 << 20;

/// An attachment's name: the SHA-256 of its bytes, as 64 lowercase hex digits.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttachmentId(String);

impl AttachmentId {
    /// The name `text` gives, if it has the stored form.
    pub fn parse(text: &str) -> Option<Self> {
        let stored = text.len() == 64
            && text
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        stored.then(|| Self(text.to_string()))
    }

    /// The name as the store and documents keep it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AttachmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A finished attachment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    /// Its name.
    pub id: AttachmentId,
    /// Its size in bytes.
    pub size: u64,
}

/// A Media Block's content: the attachment it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Media {
    /// The file's name.
    pub name: String,
    /// The file's media type, such as `image/png`; empty if unknown.
    pub media_type: String,
    /// Its attachment.
    pub attachment: AttachmentId,
    /// The attachment's size in bytes.
    pub size: u64,
}

/// An attachment being added: its parts reach the store as they fill, and it gets its name
/// when it finishes. One that never finishes is dropped from the store when it next opens.
pub struct Upload {
    key: String,
    /// The session that began it, the only one it goes to.
    session: automerge::ActorId,
    size: u64,
    written: u64,
    parts: u64,
    hasher: Sha256,
    buffer: Vec<u8>,
    /// Whether the store refused a part, which leaves the upload unfinishable.
    failed: bool,
}

impl Upload {
    /// How many bytes it has taken so far.
    pub fn written(&self) -> u64 {
        self.written
    }
}

impl<S: Store> Workspace<S> {
    /// Starts adding a file of `size` bytes for `actor`, refused past [`MAX_ATTACHMENT`].
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

    /// Adds the file's next bytes, refused if they pass its declared size. Once the store
    /// refused a part, the upload takes nothing more and never finishes.
    pub fn upload(&mut self, upload: &mut Upload, mut bytes: &[u8]) -> Result<(), Error> {
        self.owns(upload)?;
        if upload.failed {
            return Err(failed());
        }
        if upload.written + bytes.len() as u64 > upload.size {
            return Err(Error::TooLarge);
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

    /// Stores the rest and names the attachment; refused if fewer bytes came than declared.
    pub fn finish_upload(&mut self, mut upload: Upload) -> Result<Attachment, Error> {
        self.owns(&upload)?;
        if upload.failed {
            self.store.discard_upload(&upload.key)?;
            return Err(failed());
        }
        if upload.written != upload.size {
            self.store.discard_upload(&upload.key)?;
            return Err(Error::OutOfRange);
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

    /// Drops an unfinished upload's parts.
    pub fn discard_upload(&mut self, upload: Upload) -> Result<(), Error> {
        self.owns(&upload)?;
        self.store.discard_upload(&upload.key)
    }

    /// A finished attachment's size; none if there is no such attachment.
    pub fn attachment_size(&mut self, id: &AttachmentId) -> Result<Option<u64>, Error> {
        self.store.attachment_size(id)
    }

    /// Part `index` of a finished attachment; none past its last part.
    pub fn attachment_part(
        &mut self,
        id: &AttachmentId,
        index: u64,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.store.part(id, index)
    }

    /// Refuses an upload another Workspace session began, whose parts are in its store.
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
