use liquid_core::{
    Action, ActionId, AttachmentId, Commit, Document, Error, Found, Hit, IndexWrite, MemoryStore,
    Store,
};
use liquid_storage::Sqlite;

pub(crate) enum Replica {
    Disk(Sqlite),
    Version(MemoryStore),
}

macro_rules! each {
    ($self:ident, $store:ident => $call:expr) => {
        match $self {
            Replica::Disk($store) => $call,
            Replica::Version($store) => $call,
        }
    };
}

impl Store for Replica {
    fn chunks(&mut self, document: &Document) -> Result<Vec<Vec<u8>>, Error> {
        each!(self, store => store.chunks(document))
    }

    fn commit(&mut self, commit: &Commit<'_>) -> Result<(), Error> {
        match self {
            Self::Disk(store) => store.commit(commit),
            Self::Version(_) => Err(Error::NotAllowed),
        }
    }

    fn history(&mut self, before: Option<&ActionId>, limit: usize) -> Result<Vec<Action>, Error> {
        each!(self, store => store.history(before, limit))
    }

    fn history_of(
        &mut self,
        documents: &[Document],
        before: Option<&ActionId>,
        limit: usize,
    ) -> Result<Vec<Action>, Error> {
        each!(self, store => store.history_of(documents, before, limit))
    }

    fn put_part(&mut self, upload: &str, index: u64, bytes: &[u8]) -> Result<(), Error> {
        each!(self, store => store.put_part(upload, index, bytes))
    }

    fn finish_upload(
        &mut self,
        upload: &str,
        attachment: &AttachmentId,
        size: u64,
        parts: u64,
    ) -> Result<(), Error> {
        each!(self, store => store.finish_upload(upload, attachment, size, parts))
    }

    fn discard_upload(&mut self, upload: &str) -> Result<(), Error> {
        each!(self, store => store.discard_upload(upload))
    }

    fn attachment_size(&mut self, attachment: &AttachmentId) -> Result<Option<u64>, Error> {
        each!(self, store => store.attachment_size(attachment))
    }

    fn part(&mut self, attachment: &AttachmentId, index: u64) -> Result<Option<Vec<u8>>, Error> {
        each!(self, store => store.part(attachment, index))
    }

    fn remembered(&mut self, key: &str) -> Result<Option<String>, Error> {
        each!(self, store => store.remembered(key))
    }

    fn remember(&mut self, key: &str, value: Option<&str>, most: usize) -> Result<(), Error> {
        each!(self, store => store.remember(key, value, most))
    }

    fn index(&mut self, writes: &[IndexWrite], version: Option<u32>) -> Result<(), Error> {
        each!(self, store => store.index(writes, version))
    }

    fn index_version(&mut self) -> Result<u32, Error> {
        each!(self, store => store.index_version())
    }

    fn search(&mut self, words: &[String], found: Found, limit: usize) -> Result<Vec<Hit>, Error> {
        each!(self, store => store.search(words, found, limit))
    }
}
