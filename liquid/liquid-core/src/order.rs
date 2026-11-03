//! A Page's View order, kept beside its loaded document so that placing a View or reading a
//! window of the order never reads every View from the document.

use std::collections::HashMap;

use automerge::AutoCommit;

use crate::model::{self, BLOCK, BLOCKS, BOUNDARY, KIND, SORT, VIEWS};
use crate::{BlockId, BlockKind, Error, View, ViewId};

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
    /// The order as the Page document holds it: Views of removed Blocks, and of Blocks of a
    /// kind this version does not know, are left out.
    pub(crate) fn read(doc: &AutoCommit) -> Self {
        let blocks: HashMap<String, BlockKind> = model::sorted(doc, BLOCKS)
            .into_iter()
            .filter_map(|entry| Some((entry.id.clone(), BlockKind::of(entry.get(KIND)?)?)))
            .collect();
        let placed = model::sorted(doc, VIEWS)
            .into_iter()
            .filter_map(|entry| {
                let (block, kind) = entry
                    .get(BLOCK)
                    .and_then(|block| Some((block, *blocks.get(block)?)))?;
                Some(Placed {
                    sort: entry.get(SORT).unwrap_or_default().to_string(),
                    view: View {
                        id: ViewId::stored(&entry.id),
                        block: BlockId::stored(block),
                        boundary: entry.get(BOUNDARY) == Some("true"),
                        kind,
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

    pub(crate) fn view(&self, view: &ViewId) -> Option<&View> {
        self.position(view).map(|index| &self.placed[index].view)
    }

    /// The View just above `view`, if any.
    pub(crate) fn above(&self, view: &ViewId) -> Option<&View> {
        let index = self.position(view)?;
        index.checked_sub(1).map(|above| &self.placed[above].view)
    }

    pub(crate) fn set_boundary(&mut self, view: &ViewId, boundary: bool) {
        if let Some(index) = self.position(view) {
            self.placed[index].view.boundary = boundary;
        }
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

#[cfg(test)]
mod tests {
    use automerge::ActorId;

    use super::Order;
    use crate::BlockKind;
    use crate::model::{self, BLOCK, BLOCKS, KIND, SORT, VIEWS};

    /// A Block of a kind a later version adds, such as a Chart, shows no View here rather
    /// than one that would treat it as text.
    #[test]
    fn a_block_of_an_unknown_kind_shows_no_view() {
        let mut doc = model::document(&ActorId::random());
        model::create_maps(&mut doc, &[BLOCKS, VIEWS]).unwrap();
        for (block, kind, sort) in [("b1", "text", "a0"), ("b2", "chart", "a1")] {
            model::insert(&mut doc, BLOCKS, block, &[(KIND, kind)]).unwrap();
            let view = format!("v-{block}");
            model::insert(&mut doc, VIEWS, &view, &[(BLOCK, block), (SORT, sort)]).unwrap();
        }
        let order = Order::read(&doc);
        let shown: Vec<_> = order
            .placed
            .iter()
            .map(|placed| (placed.view.block.as_str(), placed.view.kind))
            .collect();
        assert_eq!(shown, [("b1", BlockKind::Text)]);
    }
}
