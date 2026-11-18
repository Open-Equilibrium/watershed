use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct TransferLocks(Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl TransferLocks {
    pub(crate) async fn run<T>(
        &self,
        transfer: &str,
        body: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        let entry = Entry::join(self, transfer);
        let _held = entry.lock.lock().await;
        body.await
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        crate::util::lock(&self.0).is_empty()
    }
}

struct Entry<'a> {
    locks: &'a TransferLocks,
    transfer: String,
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl<'a> Entry<'a> {
    fn join(locks: &'a TransferLocks, transfer: &str) -> Self {
        let lock = crate::util::lock(&locks.0)
            .entry(transfer.to_owned())
            .or_default()
            .clone();
        Self {
            locks,
            transfer: transfer.to_owned(),
            lock,
        }
    }
}

impl Drop for Entry<'_> {
    fn drop(&mut self) {
        let mut transfers = crate::util::lock(&self.locks.0);
        self.lock = Arc::default();
        if transfers
            .get(&self.transfer)
            .is_some_and(|known| Arc::strong_count(known) == 1)
        {
            transfers.remove(&self.transfer);
        }
    }
}
