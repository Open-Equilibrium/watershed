use std::collections::HashMap;

use automerge::AutoCommit;

use crate::model::{self, BLOCK, BLOCKS, BOUNDARY, INNER, KIND, TABLE, TILE, TITLE, VIEWS};
use crate::{BlockId, BlockKind, Error, Place, Tab, Tile, View, ViewId};

struct Placed {
    sort: String,
    view: View,
}

pub(crate) struct Order {
    placed: Vec<Placed>,
}

impl Order {
    pub(crate) fn read(doc: &AutoCommit) -> Self {
        let blocks: HashMap<String, BlockKind> = model::sorted(doc, BLOCKS)
            .into_iter()
            .map(|entry| match BlockKind::of(entry.get(KIND)) {
                BlockKind::Database => (entry.id.clone(), BlockKind::Unknown),
                kind => (entry.id.clone(), kind),
            })
            .collect();
        let placed = model::sorted(doc, VIEWS)
            .into_iter()
            .filter_map(|entry| {
                let mut tabs = Vec::new();
                let (block, kind) = match entry.get(TABLE) {
                    Some(table) => {
                        if let Some(view) = model::live(doc, VIEWS, &entry.id) {
                            tabs = crate::tab::read(doc, &view, &entry.id);
                        }
                        (table, BlockKind::Database)
                    }
                    None => entry
                        .get(BLOCK)
                        .and_then(|block| Some((block, *blocks.get(block)?)))?,
                };
                Some(Placed {
                    sort: entry.sort().unwrap_or_default().to_string(),
                    view: View {
                        id: ViewId::stored(&entry.id),
                        block: BlockId::stored(block),
                        title: entry.get(TITLE).unwrap_or_default().to_string(),
                        boundary: entry.get(BOUNDARY) == Some("true"),
                        kind,
                        tabs,
                        tile: stored(entry.get(TILE)),
                        inner: stored(entry.get(INNER)),
                    },
                })
            })
            .collect();
        Self { placed }
    }

    pub(crate) fn key_after(&self, after: Option<&ViewId>) -> Result<String, Error> {
        crate::sort_key::at(
            &self.placed,
            after.map_or(Place::First, Place::After),
            |placed| &placed.view.id,
            |placed| Some(&placed.sort),
        )
    }

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

    pub(crate) fn shows(&self, block: &BlockId, besides: &ViewId) -> bool {
        self.placed
            .iter()
            .any(|placed| &placed.view.block == block && &placed.view.id != besides)
    }

    pub(crate) fn view(&self, view: &ViewId) -> Option<&View> {
        self.position(view).map(|index| &self.placed[index].view)
    }

    pub(crate) fn above(&self, view: &ViewId) -> Option<&View> {
        let index = self.position(view)?;
        index.checked_sub(1).map(|above| &self.placed[above].view)
    }

    pub(crate) fn set_boundary(&mut self, view: &ViewId, boundary: bool) {
        if let Some(index) = self.position(view) {
            self.placed[index].view.boundary = boundary;
        }
    }

    pub(crate) fn set_title(&mut self, view: &ViewId, title: &str) {
        if let Some(index) = self.position(view) {
            self.placed[index].view.title = title.to_string();
        }
    }

    pub(crate) fn set_tiles(&mut self, view: &ViewId, tile: Option<Tile>, inner: Option<Tile>) {
        if let Some(index) = self.position(view) {
            self.placed[index].view.tile = tile;
            self.placed[index].view.inner = inner;
        }
    }

    pub(crate) fn run(&self, first: &ViewId, last: &ViewId) -> Option<Vec<View>> {
        let (from, to) = (self.position(first)?, self.position(last)?);
        (from <= to).then(|| {
            self.placed[from..=to]
                .iter()
                .map(|placed| placed.view.clone())
                .collect()
        })
    }

    pub(crate) fn set_tabs(&mut self, view: &ViewId, tabs: Vec<Tab>) {
        if let Some(index) = self.position(view) {
            self.placed[index].view.tabs = tabs;
        }
    }

    pub(crate) fn set_kind(&mut self, block: &BlockId, kind: BlockKind) {
        for placed in &mut self.placed {
            if &placed.view.block == block {
                placed.view.kind = kind;
            }
        }
    }

    pub(crate) fn leads(&self) -> Vec<bool> {
        let mut above: Option<&View> = None;
        self.placed
            .iter()
            .map(|placed| {
                let view = &placed.view;
                let lead = view.kind == BlockKind::Text
                    && (view.boundary || above.is_none_or(|above| above.kind != BlockKind::Text));
                above = Some(view);
                lead
            })
            .collect()
    }

    pub(crate) fn sections(&self) -> Sections {
        let mut members = HashMap::new();
        let mut tiles = HashMap::new();
        let mut lead = 0;
        for (placed, starts) in self.placed.iter().zip(self.leads()) {
            let id = hashed(&placed.view.id);
            if starts {
                lead = id;
                tiles.insert(id, placed.view.tile);
            }
            if placed.view.kind == BlockKind::Text {
                members.insert(id, lead);
            }
        }
        Sections { members, tiles }
    }

    pub(crate) fn placed_views(&self) -> impl Iterator<Item = &View> {
        self.placed.iter().map(|placed| &placed.view)
    }

    pub(crate) fn arranged(&self) -> bool {
        self.placed
            .iter()
            .any(|placed| placed.view.tile.is_some() || placed.view.inner.is_some())
    }

    pub(crate) fn len(&self) -> usize {
        self.placed.len()
    }

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

pub(crate) struct Sections {
    members: HashMap<u64, u64>,
    tiles: HashMap<u64, Option<Tile>>,
}

impl Sections {
    pub(crate) fn tile_of(&self, member: &ViewId) -> Option<Option<Tile>> {
        let lead = self.members.get(&hashed(member))?;
        self.tiles.get(lead).copied()
    }
}

pub(crate) fn hashed(view: &ViewId) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    view.hash(&mut hasher);
    hasher.finish()
}

fn stored<T: serde::de::DeserializeOwned>(json: Option<&str>) -> Option<T> {
    serde_json::from_str(json?).ok()
}

#[cfg(test)]
mod tests {
    use automerge::ActorId;

    use super::Order;
    use crate::model::{self, BLOCK, BLOCKS, KIND, SORT, VIEWS};

    fn ordered(blocks: &[(&str, &str, &str)]) -> Order {
        let mut doc = model::document(&ActorId::random());
        model::create_maps(&mut doc, &[BLOCKS, VIEWS]).unwrap();
        for &(block, kind, sort) in blocks {
            model::insert(&mut doc, BLOCKS, block, &[(KIND, kind)]).unwrap();
            let view = format!("v-{block}");
            model::insert(&mut doc, VIEWS, &view, &[(BLOCK, block), (SORT, sort)]).unwrap();
        }
        Order::read(&doc)
    }

    fn blocks(order: &Order) -> Vec<&str> {
        order
            .placed
            .iter()
            .map(|placed| placed.view.block.as_str())
            .collect()
    }

    #[test]
    fn an_unknown_kind_keeps_its_view_and_an_unusable_key_orders_as_no_key() {
        let over = format!("a0{}", "z".repeat(crate::sort_key::MAX_KEY));
        let mut order = ordered(&[
            ("b1", "text", "a0"),
            ("b2", "app", "a1"),
            ("b3", "text", &over),
        ]);
        assert_eq!(blocks(&order), ["b3", "b1", "b2"]);
        assert_eq!(order.placed[1].view.kind, crate::BlockKind::Text);
        assert_eq!(order.placed[2].view.kind, crate::BlockKind::Unknown);
        let first = order.placed[0].view.id.clone();
        assert_eq!(order.key_before(&first), Err(crate::Error::TooLong));
        let mut added = order.placed[1].view.clone();
        added.block = crate::BlockId::stored("b4");
        order.insert("a0V".to_owned(), added);
        assert_eq!(blocks(&order), ["b3", "b1", "b4", "b2"]);
    }
}
