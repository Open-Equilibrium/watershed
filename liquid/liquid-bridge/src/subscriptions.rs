use std::collections::{HashMap, HashSet};

use liquid_core::PageId;

use crate::api::workspace::{BridgeChange, BridgeResource};

type Deliver = Box<dyn FnMut(BridgeChange) -> bool + Send>;

#[derive(Default)]
pub(crate) struct Subscriptions {
    open: HashMap<i64, (BridgeResource, Deliver)>,
    closed: HashSet<i64>,
    ended: HashSet<i64>,
    seqs: HashMap<BridgeResource, i64>,
}

impl Subscriptions {
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

    pub(crate) fn follows(&self, page: &PageId) -> bool {
        self.open
            .values()
            .any(|(other, _)| other.page() == Some(page.as_str()))
    }

    #[cfg(test)]
    pub(crate) fn lifecycle_markers(&self) -> usize {
        self.closed.len() + self.ended.len()
    }
}
