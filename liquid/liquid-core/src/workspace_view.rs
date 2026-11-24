use automerge::transaction::Transactable;
use automerge::{AutoCommit, ReadDoc};

use crate::model::{
    self, BLOCK, BLOCKS, BOUNDARY, GRID_COLUMNS, INNER, PAGE, SORT, TABLE, TILE, TITLE, VIEWS,
};
use crate::order::Order;
use crate::store::Store;
use crate::workspace::{Workspace, Written};
use crate::{
    BlockId, BlockKind, Changed, Created, Document, Error, Layout, PageId, Place, TabId, TabSetup,
    Tile, View, ViewId, tab,
};

fn table_of(order: &Order, view: &ViewId) -> Result<BlockId, Error> {
    let shown = order.view(view).ok_or(Error::NotFound)?;
    if shown.kind != BlockKind::Database {
        return Err(Error::WrongKind);
    }
    Ok(shown.block.clone())
}

pub(crate) fn move_boundary(
    doc: &mut AutoCommit,
    order: &mut Order,
    from: &ViewId,
    to: &ViewId,
) -> Result<(), Error> {
    for (view, boundary) in [(from, false), (to, true)] {
        let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
        doc.put(&entry, BOUNDARY, boundary)?;
        order.set_boundary(view, boundary);
    }
    Ok(())
}

impl<S: Store> Workspace<S> {
    pub(crate) fn write_add_table_view(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
        table: &BlockId,
        title: &str,
        tabs: &[TabSetup],
    ) -> Result<Written, Error> {
        let first = [TabSetup::of(Layout::Table)];
        let tabs = if tabs.is_empty() { &first[..] } else { tabs };
        let view = ViewId::new()?;
        let tabs: Vec<TabSetup> = {
            let doc = self.table_doc(table)?;
            let tabs: Vec<TabSetup> = tabs
                .iter()
                .map(|setup| tab::live(doc, table, setup))
                .collect();
            for setup in &tabs {
                tab::check(doc, table, None, setup)?;
            }
            model::insert(doc, VIEWS, view.as_str(), &[(PAGE, page.as_str())])?;
            tabs
        };
        let mut written = self.place_table_view(page, after, table, &view, title, &tabs)?;
        written.touched.push(Document::Table(table.clone()));
        written.changed.push(Changed::Table(table.clone()));
        Ok(written)
    }

    pub(crate) fn place_table_view(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
        table: &BlockId,
        view: &ViewId,
        title: &str,
        tabs: &[TabSetup],
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let key = order.key_after(after)?;
        let entry = model::insert(
            doc,
            VIEWS,
            view.as_str(),
            &[(TABLE, table.as_str()), (TITLE, title), (SORT, &key)],
        )?;
        doc.put(&entry, BOUNDARY, false)?;
        let mut last = None;
        for setup in tabs {
            last = Some(tab::add(
                doc,
                &entry,
                last.as_ref().map_or(Place::First, Place::After),
                setup,
            )?);
        }
        order.insert(
            key,
            View {
                id: view.clone(),
                block: table.clone(),
                title: title.to_string(),
                boundary: false,
                kind: BlockKind::Database,
                tabs: tab::read(doc, &entry, view.as_str()),
                tile: None,
                inner: None,
            },
        );
        let mut written = Written::views(page);
        written.created = Some(Created::Block {
            block: table.clone(),
            view: view.clone(),
        });
        Ok(written)
    }

    pub(crate) fn write_rename_view(
        &mut self,
        page: &PageId,
        view: &ViewId,
        title: &str,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
        let table = model::string(doc, &entry, TABLE).map(|table| BlockId::stored(&table));
        doc.put(&entry, TITLE, title)?;
        order.set_title(view, title);
        let mut written = Written::views(page);
        if let Some(table) = table
            && self.table_name(&table)?.as_deref() == Some("")
        {
            let named = self.write_rename_database(&table, title)?;
            written.touched.extend(named.touched);
            written.changed.extend(named.changed);
        }
        Ok(written)
    }

    pub(crate) fn write_remove_view(
        &mut self,
        page: &PageId,
        view: &ViewId,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
        if let Some(table) = model::string(doc, &entry, TABLE) {
            model::mark_deleted(doc, &entry)?;
            order.remove(view);
            let mut written = Written::views(page);
            let table = BlockId::stored(&table);
            let table_doc = match self.table_doc(&table) {
                Ok(doc) => Some(doc),
                Err(Error::NotFound) => None,
                Err(error) => return Err(error),
            };
            if let Some(doc) = table_doc
                && let Some(shown) = model::live(doc, VIEWS, view.as_str())
            {
                model::mark_deleted(doc, &shown)?;
                written.touched.push(Document::Table(table.clone()));
                written.changed.push(Changed::Table(table));
            }
            return Ok(written);
        }
        let block = BlockId::stored(&model::string(doc, &entry, BLOCK).ok_or(Error::NotFound)?);
        model::mark_deleted(doc, &entry)?;
        let mut written = Written::views(page);
        if !order.shows(&block, view) {
            if let Some(block_entry) = model::live(doc, BLOCKS, block.as_str()) {
                model::mark_deleted(doc, &block_entry)?;
            }
            written.changed.push(Changed::Block(page.clone(), block));
        }
        order.remove(view);
        Ok(written)
    }

    pub(crate) fn write_tile(
        &mut self,
        page: &PageId,
        view: &ViewId,
        tile: Option<Tile>,
        inner: bool,
    ) -> Result<Written, Error> {
        if tile.is_some_and(|tile| !tile.fits(GRID_COLUMNS)) {
            return Err(Error::OutOfRange);
        }
        let (doc, order) = self.page(page)?;
        let shown = order.view(view).ok_or(Error::NotFound)?;
        let text = shown.kind == BlockKind::Text;
        let starts_section = shown.boundary
            || order
                .above(view)
                .is_none_or(|above| above.kind != BlockKind::Text);
        match (inner, text) {
            (true, false) => return Err(Error::WrongKind),
            (false, true) if !starts_section => return Err(Error::NotAllowed),
            _ => {}
        }
        let (mut placed, mut within) = (shown.tile, shown.inner);
        if inner {
            within = tile;
        } else {
            placed = tile;
        }
        let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
        let key = if inner { INNER } else { TILE };
        match tile {
            Some(tile) => {
                let json = serde_json::to_string(&tile).map_err(|_| Error::WrongKind)?;
                doc.put(&entry, key, json)?;
            }
            None if doc.get(&entry, key)?.is_some() => doc.delete(&entry, key)?,
            None => {}
        }
        order.set_tiles(view, placed, within);
        Ok(Written::views(page))
    }

    pub(crate) fn write_move_views(
        &mut self,
        page: &PageId,
        first: &ViewId,
        last: &ViewId,
        after: Option<&ViewId>,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let run = order.run(first, last).ok_or(Error::OutOfRange)?;
        if order.above(first).map(|above| &above.id) == after {
            return Err(Error::NoChange);
        }
        if let Some(after) = after {
            order.view(after).ok_or(Error::NotFound)?;
            if run.iter().any(|view| &view.id == after) {
                return Err(Error::OutOfRange);
            }
        }
        for view in &run {
            order.remove(&view.id);
        }
        let mut previous = after.cloned();
        for view in run {
            let key = order.key_after(previous.as_ref())?;
            let entry = model::live(doc, VIEWS, view.id.as_str()).ok_or(Error::NotFound)?;
            doc.put(&entry, SORT, key.clone())?;
            previous = Some(view.id.clone());
            order.insert(key, view);
        }
        Ok(Written::views(page))
    }

    pub(crate) fn write_set_section_boundary(
        &mut self,
        page: &PageId,
        view: &ViewId,
        boundary: bool,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
        doc.put(&entry, BOUNDARY, boundary)?;
        order.set_boundary(view, boundary);
        Ok(Written::views(page))
    }

    pub(crate) fn write_add_tab(
        &mut self,
        page: &PageId,
        view: &ViewId,
        after: Option<&TabId>,
        setup: &TabSetup,
    ) -> Result<Written, Error> {
        let table = table_of(self.page(page)?.1, view)?;
        let doc = self.table_doc(&table)?;
        let setup = tab::live(doc, &table, setup);
        tab::check(doc, &table, None, &setup)?;
        self.write_tabs(page, view, |doc, entry| {
            let place = after.map_or(Place::First, Place::After);
            tab::add(doc, entry, place, &setup).map(Some)
        })
    }

    pub(crate) fn write_tab(
        &mut self,
        page: &PageId,
        view: &ViewId,
        tab: &TabId,
        change: impl FnOnce(&mut TabSetup),
    ) -> Result<Written, Error> {
        let order = self.page(page)?.1;
        let table = table_of(order, view)?;
        let before = order
            .view(view)
            .and_then(|shown| shown.tabs.iter().find(|shown| &shown.id == tab))
            .ok_or(Error::NotFound)?
            .setup
            .clone();
        let mut after = before.clone();
        change(&mut after);
        tab::check(self.table_doc(&table)?, &table, Some(&before), &after)?;
        self.write_tabs(page, view, |doc, entry| {
            tab::set(doc, entry, tab, &before, &after).map(|()| None)
        })
    }

    pub(crate) fn write_tabs(
        &mut self,
        page: &PageId,
        view: &ViewId,
        change: impl FnOnce(&mut AutoCommit, &automerge::ObjId) -> Result<Option<TabId>, Error>,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        table_of(order, view)?;
        let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
        tab::keep(doc, &entry, view.as_str())?;
        let created = change(doc, &entry)?;
        order.set_tabs(view, tab::read(doc, &entry, view.as_str()));
        let mut written = Written::views(page);
        written.created = created.map(Created::Tab);
        Ok(written)
    }

    pub(crate) fn carry_tiles(
        &mut self,
        page: &PageId,
        before: &crate::order::Sections,
    ) -> Result<bool, Error> {
        let (doc, order) = self.page(page)?;
        let mut carried = Vec::new();
        let mut lead: Option<(&View, bool)> = None;
        for (view, starts) in order.placed_views().zip(order.leads()) {
            if view.kind != BlockKind::Text {
                lead = None;
                continue;
            }
            if starts {
                lead = Some((view, false));
            } else if view.tile.is_some() {
                carried.push((view.id.clone(), None, view.inner));
            }
            if let Some((shown, false)) = lead
                && let Some(tile) = before.tile_of(&view.id)
            {
                lead = Some((shown, true));
                if shown.tile != tile {
                    carried.push((shown.id.clone(), tile, shown.inner));
                }
            }
        }
        for (view, tile, inner) in &carried {
            let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
            match tile {
                Some(tile) => {
                    let json = serde_json::to_string(tile).map_err(|_| Error::WrongKind)?;
                    doc.put(&entry, TILE, json)?;
                }
                None => doc.delete(&entry, TILE)?,
            }
            order.set_tiles(view, *tile, *inner);
        }
        Ok(!carried.is_empty())
    }
}
