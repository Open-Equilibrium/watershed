//! The durable store a Workspace commits to before it acknowledges a write (client
//! architecture rule 9). `liquid-storage` implements it on SQLite; [`MemoryStore`] keeps
//! it in memory for tests and observations.

use std::collections::HashMap;

use crate::{Action, ActionId, Document, Error};

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
pub trait Store {
    /// The stored chunks of `document`, oldest first; none if it was never stored.
    fn chunks(&mut self, document: &Document) -> Result<Vec<Vec<u8>>, Error>;
    /// Applies one Action's writes and records it, durably and atomically.
    fn commit(&mut self, commit: &Commit<'_>) -> Result<(), Error>;
    /// Up to `limit` recorded Actions, newest first, starting before `before`.
    fn history(&mut self, before: Option<&ActionId>, limit: usize) -> Result<Vec<Action>, Error>;
}

/// A store in memory, which loses everything when dropped.
#[derive(Default)]
pub struct MemoryStore {
    chunks: HashMap<Document, Vec<Vec<u8>>>,
    actions: Vec<Action>,
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
}
