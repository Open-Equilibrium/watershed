//! A T3 stream held as its latest value. Dropping the watch ends the stream, which tells T3 to
//! stop sending it.

use serde_json::Value;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::client::Subscription;

/// The latest value of a T3 stream, kept current until the watch is dropped.
pub struct Watch<T> {
    latest: watch::Receiver<T>,
    task: JoinHandle<()>,
}

impl<T: Send + Sync + 'static> Watch<T> {
    /// Follows `stream`, keeping the latest value `map` makes of its items. Returns once the
    /// first item maps to a value.
    pub(crate) async fn follow(
        mut stream: Subscription,
        map: impl Fn(&Value) -> Option<T> + Send + 'static,
    ) -> Result<Self, String> {
        let first = loop {
            let item = stream.recv().await.ok_or("T3 ended the stream")?;
            if let Some(value) = map(&item) {
                break value;
            }
        };
        let (sender, latest) = watch::channel(first);
        let task = tokio::spawn(async move {
            while let Some(item) = stream.recv().await {
                if let Some(value) = map(&item)
                    && sender.send(value).is_err()
                {
                    break;
                }
            }
        });
        Ok(Self { latest, task })
    }
}

impl<T: Clone> Watch<T> {
    /// The latest value.
    pub fn current(&self) -> T {
        self.latest.borrow().clone()
    }

    /// Waits for a value newer than the last one `changed` returned; `None` once T3 ended the
    /// stream.
    pub async fn changed(&mut self) -> Option<T> {
        self.latest.changed().await.ok()?;
        Some(self.latest.borrow_and_update().clone())
    }
}

impl<T> Drop for Watch<T> {
    fn drop(&mut self) {
        self.task.abort();
    }
}
