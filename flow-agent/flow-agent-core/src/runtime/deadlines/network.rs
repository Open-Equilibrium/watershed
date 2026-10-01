use crate::runtime::types::RuntimeError;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::{
    io,
    net::ToSocketAddrs,
    sync::{Arc, OnceLock},
};
use tokio::{runtime::Runtime, sync::Semaphore};

const SYSTEM_LOOKUP_CAPACITY: usize = 32;

// Static ownership avoids Runtime::drop waiting for stalled system DNS at command
// completion or process exit. Only admitted jobs reach the blocking pool.
static NETWORK_RUNTIME: OnceLock<Result<Runtime, io::Error>> = OnceLock::new();
static LOOKUP_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

pub(super) fn runtime() -> Result<&'static Runtime, RuntimeError> {
    NETWORK_RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .max_blocking_threads(SYSTEM_LOOKUP_CAPACITY)
                .enable_io()
                .enable_time()
                .build()
        })
        .as_ref()
        .map_err(|_| RuntimeError::Protocol("network runtime construction failed".to_owned()))
}

#[cfg(test)]
type Lookup = Arc<dyn Fn(&str) -> io::Result<Addrs> + Send + Sync>;

#[cfg(test)]
thread_local! {
    static TEST_LOOKUP: std::cell::RefCell<Option<Lookup>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_system_lookup<T>(lookup: Lookup, operation: impl FnOnce() -> T) -> T {
    struct Reset(Option<Lookup>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_LOOKUP.set(self.0.take());
        }
    }
    let _reset = Reset(TEST_LOOKUP.replace(Some(lookup)));
    operation()
}

pub(super) struct SystemResolver {
    slots: Arc<Semaphore>,
    #[cfg(test)]
    lookup: Option<Lookup>,
}

impl SystemResolver {
    pub(super) fn new() -> Self {
        Self {
            slots: Arc::clone(
                LOOKUP_SLOTS.get_or_init(|| Arc::new(Semaphore::new(SYSTEM_LOOKUP_CAPACITY))),
            ),
            #[cfg(test)]
            lookup: TEST_LOOKUP.with_borrow(Clone::clone),
        }
    }
}

impl Resolve for SystemResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let slots = Arc::clone(&self.slots);
        #[cfg(test)]
        let lookup = self.lookup.clone();
        Box::pin(async move {
            let slot = slots.acquire_owned().await?;
            let addresses = runtime()?
                .spawn_blocking(move || {
                    // Cancellation drops the waiting future, never this job's slot.
                    let _slot = slot;
                    #[cfg(test)]
                    if let Some(lookup) = lookup {
                        return lookup(name.as_str());
                    }
                    (name.as_str(), 0)
                        .to_socket_addrs()
                        .map(|addresses| Box::new(addresses) as Addrs)
                })
                .await??;
            Ok(addresses)
        })
    }
}
