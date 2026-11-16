//! Who follows which Workspace resource. A subscription's registration and its close arrive
//! on flutter_rust_bridge's worker threads in either order, and each resource counts its
//! changes, so a reader that sees a gap knows to query again.

use std::collections::{HashMap, HashSet};

use liquid_core::PageId;

use crate::api::workspace::{BridgeChange, BridgeResource};

type Deliver = Box<dyn FnMut(BridgeChange) -> bool + Send>;

#[derive(Default)]
pub(crate) struct Subscriptions {
    /// Open subscriptions by the identity Dart chose.
    open: HashMap<i64, (BridgeResource, Deliver)>,
    /// Subscriptions whose close arrived before their registration, which calls on
    /// flutter_rust_bridge's worker threads may do.
    closed: HashSet<i64>,
    /// Subscriptions that ended because their stream was gone, until Dart's close arrives.
    ended: HashSet<i64>,
    /// Each changed resource's change count.
    seqs: HashMap<BridgeResource, i64>,
}

impl Subscriptions {
    /// Registers `subscription` for `resource`, as normalized, and hands `deliver` the
    /// resource's current count. A refused registration ends the subscription like any other end.
    pub(crate) fn follow(
        &mut self,
        subscription: i64,
        resource: Result<BridgeResource, String>,
        mut deliver: Deliver,
    ) -> Result<(), String> {
        let resource = match resource {
            Ok(resource) => resource,
            Err(error) => {
                if !self.closed.remove(&subscription) {
                    self.ended.insert(subscription);
                }
                return Err(error);
            }
        };
        // Dart cancelled it before it arrived, which is a normal end, not an error.
        if self.closed.remove(&subscription) {
            return Ok(());
        }
        if self.open.contains_key(&subscription) {
            return Err(format!("subscription {subscription} is already open"));
        }
        let seq = self.seqs.get(&resource).copied().unwrap_or_default();
        let current = BridgeChange {
            resource: resource.clone(),
            seq,
        };
        if deliver(current) {
            self.open.insert(subscription, (resource, deliver));
        } else {
            self.ended.insert(subscription);
        }
        Ok(())
    }

    /// Ends `subscription`, also one whose registration has not arrived yet, and answers the
    /// resource it followed.
    pub(crate) fn close(&mut self, subscription: i64) -> Option<BridgeResource> {
        match self.open.remove(&subscription) {
            Some((resource, _)) => Some(resource),
            None if self.ended.remove(&subscription) => None,
            None => {
                self.closed.insert(subscription);
                None
            }
        }
    }

    /// Counts a change of `resource` and tells its subscribers, ending those whose Dart
    /// stream is gone; answers the resources those followed.
    pub(crate) fn notify(&mut self, resource: BridgeResource) -> Vec<BridgeResource> {
        let seq = self.seqs.entry(resource.clone()).or_default();
        *seq += 1;
        let change = BridgeChange {
            resource: resource.clone(),
            seq: *seq,
        };
        let mut gone = Vec::new();
        for (id, (followed, deliver)) in &mut self.open {
            if *followed == resource && !deliver(change.clone()) {
                gone.push(*id);
            }
        }
        gone.into_iter()
            .filter_map(|id| {
                let (resource, _) = self.open.remove(&id)?;
                self.ended.insert(id);
                Some(resource)
            })
            .collect()
    }

    /// Whether a subscription reads `page`.
    pub(crate) fn follows(&self, page: &PageId) -> bool {
        self.open
            .values()
            .any(|(other, _)| other.page() == Some(page.as_str()))
    }

    /// How many subscription identities wait for their other end: a registration or a close.
    #[cfg(test)]
    pub(crate) fn lifecycle_markers(&self) -> usize {
        self.closed.len() + self.ended.len()
    }
}
