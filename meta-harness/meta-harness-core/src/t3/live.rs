use serde_json::Value;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::client::Subscription;

pub struct Watch<T> {
    latest: watch::Receiver<T>,
    task: JoinHandle<()>,
}

impl<T: Send + Sync + 'static> Watch<T> {
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

impl<T: serde::de::DeserializeOwned + Send + Sync + 'static> Watch<T> {
    pub(crate) fn from_stream(first: T, mut stream: crate::remote::Stream<T>) -> Self {
        let (sender, latest) = watch::channel(first);
        let task = tokio::spawn(async move {
            while let Some(value) = stream.recv().await {
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        Self { latest, task }
    }
}

impl<T: Clone> Watch<T> {
    pub fn current(&self) -> T {
        self.latest.borrow().clone()
    }

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
