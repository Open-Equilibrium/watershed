//! A Page's View order, kept beside its loaded document so that placing a View or reading a
//! window of the order never reads every View from the document.

use std::collections::HashSet;

use automerge::AutoCommit;

use crate::model::{self, BLOCK, BLOCKS, SORT, VIEWS};
use crate::{BlockId, Error, View, ViewId};

/// One live View with its sort key.
struct Placed {
    sort: String,
    view: View,
}

/// A Page's live Views, ordered by sort key, then identity, as the document orders them.
pub(crate) struct Order {
    placed: Vec<Placed>,
}

impl Order {
    /// The order as the Page document holds it: Views of removed Blocks are left out.
    pub(crate) fn read(doc: &AutoCommit) -> Self {
        let blocks: HashSet<String> = model::sorted(doc, BLOCKS)
            .into_iter()
            .map(|entry| entry.id)
            .collect();
        let placed = model::sorted(doc, VIEWS)
            .into_iter()
            .filter_map(|entry| {
                let block = entry.get(BLOCK).filter(|block| blocks.contains(*block))?;
                Some(Placed {
                    sort: entry.get(SORT).unwrap_or_default().to_string(),
                    view: View {
                        id: ViewId::stored(&entry.id),
                        block: BlockId::stored(block),
                    },
                })
            })
            .collect();
        Self { placed }
    }

    /// The sort key for a View placed after `after`, or first.
    pub(crate) fn key_after(&self, after: Option<&ViewId>) -> Result<String, Error> {
        let index = match after {
            None => 0,
            Some(after) => self.position(after).ok_or(Error::NotFound)? + 1,
        };
        let key = |i: usize| self.placed.get(i).map(|placed| placed.sort.as_str());
        Ok(crate::sort_key::between(
            index.checked_sub(1).and_then(key),
            key(index),
        ))
    }

    pub(crate) fn insert(&mut self, sort: String, view: View) {
        let index = self
            .placed
            .partition_point(|placed| (&placed.sort, &placed.view.id) < (&sort, &view.id));
        self.placed.insert(index, Placed { sort, view });
    }

    pub(crate) fn remove(&mut self, view: &ViewId) {
        if let Some(index) = self.position(view) {
            self.placed.remove(index);
        }
    }

    /// Whether a View other than `besides` shows `block`.
    pub(crate) fn shows(&self, block: &BlockId, besides: &ViewId) -> bool {
        self.placed
            .iter()
            .any(|placed| &placed.view.block == block && &placed.view.id != besides)
    }

    pub(crate) fn len(&self) -> usize {
        self.placed.len()
    }

    /// Up to `limit` Views from position `start`.
    pub(crate) fn window(&self, start: usize, limit: usize) -> Vec<View> {
        self.placed
            .iter()
            .skip(start)
            .take(limit)
            .map(|placed| placed.view.clone())
            .collect()
    }

    fn position(&self, view: &ViewId) -> Option<usize> {
        self.placed
            .iter()
            .position(|placed| &placed.view.id == view)
    }
}
