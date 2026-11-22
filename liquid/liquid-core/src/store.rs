use std::collections::HashMap;

use crate::{Action, ActionId, AttachmentId, Document, Error};

pub struct Commit<'a> {
    pub action: &'a Action,
    pub writes: Vec<Write>,
}

pub enum Write {
    Append {
        document: Document,
        chunk: Vec<u8>,
    },
    Replace {
        document: Document,
        snapshot: Vec<u8>,
    },
}

pub trait Store {
    fn chunks(&mut self, document: &Document) -> Result<Vec<Vec<u8>>, Error>;
    fn commit(&mut self, commit: &Commit<'_>) -> Result<(), Error>;
    fn history(&mut self, before: Option<&ActionId>, limit: usize) -> Result<Vec<Action>, Error>;
    fn put_part(&mut self, upload: &str, index: u64, bytes: &[u8]) -> Result<(), Error>;
    fn finish_upload(
        &mut self,
        upload: &str,
        attachment: &AttachmentId,
        size: u64,
        parts: u64,
    ) -> Result<(), Error>;
    fn discard_upload(&mut self, upload: &str) -> Result<(), Error>;
    fn attachment_size(&mut self, attachment: &AttachmentId) -> Result<Option<u64>, Error>;
    fn part(&mut self, attachment: &AttachmentId, index: u64) -> Result<Option<Vec<u8>>, Error>;
    fn remembered(&mut self, key: &str) -> Result<Option<String>, Error>;
    fn remember(&mut self, key: &str, value: Option<&str>) -> Result<(), Error>;
    fn device_values(&mut self) -> Result<usize, Error>;
    fn forget_oldest(&mut self) -> Result<(), Error>;
}

#[derive(Default)]
pub struct MemoryStore {
    chunks: HashMap<Document, Vec<Vec<u8>>>,
    actions: Vec<Action>,
    uploads: HashMap<String, Vec<Vec<u8>>>,
    attachments: HashMap<AttachmentId, (u64, Vec<Vec<u8>>)>,
    device: Vec<(String, String)>,
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
        parts: u64,
    ) -> Result<(), Error> {
        let kept = self.uploads.get(upload).map_or(0, Vec::len);
        if kept as u64 != parts {
            return Err(Error::Storage(format!(
                "the upload has {kept} of its {parts} parts"
            )));
        }
        let kept = self.uploads.remove(upload).unwrap_or_default();
        self.attachments
            .entry(attachment.clone())
            .or_insert((size, kept));
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

    fn remembered(&mut self, key: &str) -> Result<Option<String>, Error> {
        Ok(self
            .device
            .iter()
            .find(|(kept, _)| kept == key)
            .map(|(_, value)| value.clone()))
    }

    fn remember(&mut self, key: &str, value: Option<&str>) -> Result<(), Error> {
        self.device.retain(|(kept, _)| kept != key);
        if let Some(value) = value {
            self.device.push((key.to_string(), value.to_string()));
        }
        Ok(())
    }

    fn device_values(&mut self) -> Result<usize, Error> {
        Ok(self.device.len())
    }

    fn forget_oldest(&mut self) -> Result<(), Error> {
        if !self.device.is_empty() {
            self.device.remove(0);
        }
        Ok(())
    }
}
