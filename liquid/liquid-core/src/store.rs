//! The durable store a Workspace commits to before it acknowledges a write (client
//! architecture rule 9). `liquid-storage` implements it on SQLite; [`MemoryStore`] keeps
//! it in memory for tests and observations.

use std::collections::HashMap;

use crate::{Action, ActionId, AttachmentId, Document, Error};

/// One Action's durable writes, which a store applies all together or not at all.
pub struct Commit<'a> {
    /// The Action, recorded in History.
    pub action: &'a Action,
    /// What each document it touched stores.
    pub writes: Vec<Write>,
}

/// What a document stores after a commit.
pub enum Write {
    /// The document's new change, after its stored chunks.
    Append {
        /// The document.
        document: Document,
        /// The change, as Automerge encodes it.
        chunk: Vec<u8>,
    },
    /// The whole document, replacing its stored chunks, so loading it stays short.
    Replace {
        /// The document.
        document: Document,
        /// The document, as Automerge saves it.
        snapshot: Vec<u8>,
    },
}

/// Where a Workspace keeps its documents and History.
///
/// A store belongs to one Workspace for as long as that Workspace lives, because the
/// Workspace keeps documents in memory and compacts from them: an implementation over
/// shared data refuses a second session while the first holds it.
pub trait Store {
    /// The stored chunks of `document`, oldest first; none if it was never stored.
    fn chunks(&mut self, document: &Document) -> Result<Vec<Vec<u8>>, Error>;
    /// Applies one Action's writes and records it, durably and atomically.
    fn commit(&mut self, commit: &Commit<'_>) -> Result<(), Error>;
    /// Up to `limit` recorded Actions, newest first, starting before `before`.
    fn history(&mut self, before: Option<&ActionId>, limit: usize) -> Result<Vec<Action>, Error>;
    /// Keeps part `index` of the unfinished upload `upload`.
    fn put_part(&mut self, upload: &str, index: u64, bytes: &[u8]) -> Result<(), Error>;
    /// Names `upload`'s parts `attachment`, durably; if that attachment is kept already,
    /// drops them instead.
    fn finish_upload(
        &mut self,
        upload: &str,
        attachment: &AttachmentId,
        size: u64,
    ) -> Result<(), Error>;
    /// Drops an unfinished upload's parts.
    fn discard_upload(&mut self, upload: &str) -> Result<(), Error>;
    /// A finished attachment's size; none if it is not kept.
    fn attachment_size(&mut self, attachment: &AttachmentId) -> Result<Option<u64>, Error>;
    /// Part `index` of a finished attachment; none past its last part.
    fn part(&mut self, attachment: &AttachmentId, index: u64) -> Result<Option<Vec<u8>>, Error>;
}

/// A store in memory, which loses everything when dropped.
#[derive(Default)]
pub struct MemoryStore {
    chunks: HashMap<Document, Vec<Vec<u8>>>,
    actions: Vec<Action>,
    uploads: HashMap<String, Vec<Vec<u8>>>,
    attachments: HashMap<AttachmentId, (u64, Vec<Vec<u8>>)>,
}

impl Store for MemoryStore {
    fn chunks(&mut self, document: &Document) -> Result<Vec<Vec<u8>>, Error> {
        Ok(self.chunks.get(document).cloned().unwrap_or_default())
    }

    fn commit(&mut self, commit: &Commit<'_>) -> Result<(), Error> {
        for write in &commit.writes {
            match write {
                Write::Append { document, chunk } => {
                    self.chunks
                        .entry(document.clone())
                        .or_default()
                        .push(chunk.clone());
                }
                Write::Replace { document, snapshot } => {
                    self.chunks.insert(document.clone(), vec![snapshot.clone()]);
                }
            }
        }
        self.actions.push(commit.action.clone());
        Ok(())
    }

    fn history(&mut self, before: Option<&ActionId>, limit: usize) -> Result<Vec<Action>, Error> {
        let end = match before {
            None => self.actions.len(),
            Some(id) => self
                .actions
                .iter()
                .position(|action| &action.id == id)
                .ok_or(Error::NotFound)?,
        };
        Ok(self.actions[..end]
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect())
    }

    fn put_part(&mut self, upload: &str, index: u64, bytes: &[u8]) -> Result<(), Error> {
        let parts = self.uploads.entry(upload.to_string()).or_default();
        if index != parts.len() as u64 {
            return Err(Error::Storage(format!("part {index} out of order")));
        }
        parts.push(bytes.to_vec());
        Ok(())
    }

    fn finish_upload(
        &mut self,
        upload: &str,
        attachment: &AttachmentId,
        size: u64,
    ) -> Result<(), Error> {
        let parts = self.uploads.remove(upload).unwrap_or_default();
        self.attachments
            .entry(attachment.clone())
            .or_insert((size, parts));
        Ok(())
    }

    fn discard_upload(&mut self, upload: &str) -> Result<(), Error> {
        self.uploads.remove(upload);
        Ok(())
    }

    fn attachment_size(&mut self, attachment: &AttachmentId) -> Result<Option<u64>, Error> {
        Ok(self.attachments.get(attachment).map(|(size, _)| *size))
    }

    fn part(&mut self, attachment: &AttachmentId, index: u64) -> Result<Option<Vec<u8>>, Error> {
        Ok(self
            .attachments
            .get(attachment)
            .and_then(|(_, parts)| parts.get(usize::try_from(index).ok()?).cloned()))
    }
}
