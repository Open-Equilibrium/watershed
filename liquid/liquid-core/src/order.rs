//! A Page's View order, kept beside its loaded document so that placing a View or reading a
//! window of the order never reads every View from the document.

use std::collections::HashMap;

use automerge::AutoCommit;

use crate::model::{self, BLOCK, BLOCKS, BOUNDARY, KIND, VIEWS};
use crate::{BlockId, BlockKind, Error, Place, View, ViewId};

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
        let blocks: HashMap<String, BlockKind> = model::sorted(doc, BLOCKS)
            .into_iter()
            .map(|entry| (entry.id.clone(), BlockKind::of(entry.get(KIND))))
            .collect();
        let placed = model::sorted(doc, VIEWS)
            .into_iter()
            .filter_map(|entry| {
                let (block, kind) = entry
                    .get(BLOCK)
                    .and_then(|block| Some((block, *blocks.get(block)?)))?;
                Some(Placed {
                    sort: entry.sort().unwrap_or_default().to_string(),
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
        crate::sort_key::at(
            &self.placed,
            after.map_or(Place::First, Place::After),
            |placed| &placed.view.id,
            |placed| Some(&placed.sort),
        )
    }

    /// The sort key for a View placed right before `before`.
    pub(crate) fn key_before(&self, before: &ViewId) -> Result<String, Error> {
        crate::sort_key::before(
            &self.placed,
            before,
            |placed| &placed.view.id,
            |placed| Some(&placed.sort),
        )
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

    /// Every View of `block` shows it as `kind` now.
    pub(crate) fn set_kind(&mut self, block: &BlockId, kind: BlockKind) {
        for placed in &mut self.placed {
            if &placed.view.block == block {
                placed.view.kind = kind;
            }
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

    /// A Block of a kind a later version adds, such as a Chart, keeps its View, which shows a
    /// placeholder (ADR-0245).
    #[test]
    fn a_block_of_an_unknown_kind_keeps_its_view() {
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
        assert_eq!(shown, [("b1", BlockKind::Text), ("b2", BlockKind::Unknown)]);
    }

    /// A key past the cap, as another device may send, reads as no key, as placing does
    /// (ADR-0304).
    #[test]
    fn a_stored_key_past_the_cap_orders_as_no_key() {
        let mut doc = model::document(&ActorId::random());
        model::create_maps(&mut doc, &[BLOCKS, VIEWS]).unwrap();
        let over = format!("a0{}", "z".repeat(crate::sort_key::MAX_KEY));
        for (block, sort) in [("b1", "a0"), ("b2", over.as_str()), ("b3", "a1")] {
            model::insert(&mut doc, BLOCKS, block, &[(KIND, "text")]).unwrap();
            let view = format!("v-{block}");
            model::insert(&mut doc, VIEWS, &view, &[(BLOCK, block), (SORT, sort)]).unwrap();
        }
        let mut order = Order::read(&doc);
        let shown = |order: &Order| -> Vec<String> {
            order
                .placed
                .iter()
                .map(|placed| placed.view.block.as_str().to_owned())
                .collect()
        };
        assert_eq!(shown(&order), ["b2", "b1", "b3"]);
        let first = order.placed[0].view.id.clone();
        assert!(
            matches!(order.key_before(&first), Err(crate::Error::TooLong)),
            "nothing sorts before a View without a usable key"
        );
        let mut added = order.placed[1].view.clone();
        added.block = crate::BlockId::stored("b4");
        order.insert("a0V".to_owned(), added);
        assert_eq!(
            shown(&order),
            ["b2", "b1", "b4", "b3"],
            "an insert agrees with the read"
        );
    }
}
