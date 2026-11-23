use automerge::ROOT;
use automerge::transaction::Transactable;

use crate::model::{self, AREA, BLOCKS, NAME, PAGE, SORT, TABLES, VIEWS};
use crate::store::Store;
use crate::workspace::{Workspace, Written};
use crate::{
    AreaId, BlockId, Cell, Changed, Created, Document, Error, FieldId, FieldKind, Layout, Media,
    PageId, Place, RowId, TabSetup, ViewId, database, tab,
};

pub(crate) fn table_changed(table: &BlockId, created: Option<Created>) -> Written {
    let mut written = Written::new(
        Document::Table(table.clone()),
        Changed::Table(table.clone()),
    );
    written.created = created;
    written
}

impl<S: Store> Workspace<S> {
    pub(crate) fn write_add_database_block(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
        name: &str,
        layout: Layout,
    ) -> Result<Written, Error> {
        let area = self.area_of(page)?;
        let (table, mut written) = self.write_create_table(&area, name, None)?;
        let (_, doc) = written.new.last_mut().expect("the new Table");
        let mut setup = TabSetup::of(layout);
        match layout {
            Layout::Table => {}
            Layout::Board => {
                let status =
                    database::add_field(doc, &table, Place::Last, "Status", FieldKind::Select)?;
                for (color, option) in ["Not started", "In progress", "Done"].iter().enumerate() {
                    database::add_option(doc, &table, &status, option, color as u8)?;
                }
                setup.group = Some(status);
            }
            Layout::Calendar => {
                let date = database::add_field(doc, &table, Place::Last, "Date", FieldKind::Date)?;
                setup.date = Some(date);
            }
        }
        self.show_new_table(written, page, after, &table, name, &[setup])
    }

    pub(crate) fn write_duplicate_table(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
        source: &BlockId,
        name: &str,
        tabs: &[TabSetup],
    ) -> Result<Written, Error> {
        let doc = self.table_doc(source)?;
        let tabs: Vec<TabSetup> = tabs
            .iter()
            .map(|setup| tab::live(doc, source, setup))
            .collect();
        for setup in &tabs {
            tab::check(doc, source, None, setup)?;
        }
        let area = self.area_of(page)?;
        let (table, written) = self.write_create_table(&area, name, Some(source))?;
        self.show_new_table(written, page, after, &table, name, &tabs)
    }

    fn show_new_table(
        &mut self,
        mut written: Written,
        page: &PageId,
        after: Option<&ViewId>,
        table: &BlockId,
        title: &str,
        tabs: &[TabSetup],
    ) -> Result<Written, Error> {
        let view = ViewId::new()?;
        let (_, doc) = written.new.last_mut().expect("the new Table");
        model::insert(doc, VIEWS, view.as_str(), &[(PAGE, page.as_str())])?;
        let shown = self.place_table_view(page, after, table, &view, title, tabs)?;
        written.touched.extend(shown.touched);
        written.changed.extend(shown.changed);
        written.created = shown.created;
        Ok(written)
    }

    pub(crate) fn write_create_table(
        &mut self,
        area: &AreaId,
        name: &str,
        source: Option<&BlockId>,
    ) -> Result<(BlockId, Written), Error> {
        self.doc(&Document::Area(area.clone()))?;
        let table = BlockId::new()?;
        let workspace = self.doc(&Document::Workspace)?;
        if model::child(workspace, &ROOT, TABLES).is_none() {
            model::create_maps(workspace, &[TABLES])?;
        }
        let siblings = model::sorted(workspace, TABLES);
        let sort = model::sort_at(&siblings, Place::Last)?;
        model::insert(
            workspace,
            TABLES,
            table.as_str(),
            &[(NAME, name), (AREA, area.as_str()), (SORT, &sort)],
        )?;
        let mut doc = model::document(&self.device);
        model::create_maps(&mut doc, &[BLOCKS, VIEWS])?;
        match source {
            Some(source) => database::copy(self.table_doc(source)?, source, &mut doc, &table)?,
            None => database::create(&mut doc, &table)?,
        }
        let mut written = Written::new(Document::Workspace, Changed::Tables);
        written.touched.push(Document::Table(table.clone()));
        written.new.push((Document::Table(table.clone()), doc));
        Ok((table, written))
    }

    pub(crate) fn write_rename_database(
        &mut self,
        table: &BlockId,
        name: &str,
    ) -> Result<Written, Error> {
        let workspace = self.doc(&Document::Workspace)?;
        let entry = model::live(workspace, TABLES, table.as_str()).ok_or(Error::NotFound)?;
        workspace.put(&entry, NAME, name)?;
        let mut written = Written::new(Document::Workspace, Changed::Tables);
        written.changed.push(Changed::Table(table.clone()));
        Ok(written)
    }

    pub(crate) fn write_delete_table(&mut self, table: &BlockId) -> Result<Written, Error> {
        let shown = self.table_views(table)?;
        let database = self.database(table)?.ok_or(Error::NotFound)?;
        let workspace = self.doc(&Document::Workspace)?;
        let entry = model::live(workspace, TABLES, table.as_str()).ok_or(Error::NotFound)?;
        model::mark_deleted(workspace, &entry)?;
        let mut written = Written::new(Document::Workspace, Changed::Tables);
        written.changed.push(Changed::Table(table.clone()));
        self.table_deleted(table, &database, &mut written)?;
        self.table_left_roles(table, &mut written)?;
        for (page, view) in shown {
            let Ok((doc, order)) = self.page(&page) else {
                continue;
            };
            if let Some(entry) = model::live(doc, VIEWS, view.as_str()) {
                model::mark_deleted(doc, &entry)?;
                order.remove(&view);
                written.touched.push(Document::Page(page.clone()));
                written.changed.push(Changed::Views(page));
            }
        }
        Ok(written)
    }

    pub(crate) fn write_set_cell(
        &mut self,
        block: &BlockId,
        row: &RowId,
        field: &FieldId,
        value: &Cell,
    ) -> Result<Written, Error> {
        let sized;
        let value = match value {
            Cell::Files(files) => {
                let mut checked = Vec::with_capacity(files.len().min(crate::MAX_FILES + 1));
                for file in files.iter().take(crate::MAX_FILES + 1) {
                    let size = self
                        .store
                        .attachment_size(&file.attachment)?
                        .ok_or(Error::NotFound)?;
                    checked.push(Media {
                        size,
                        ..file.clone()
                    });
                }
                sized = Cell::Files(checked);
                &sized
            }
            value => value,
        };
        let (doc, index) = self.table_index(block)?;
        database::set_cell(doc, index, block, row, field, value)?;
        Ok(table_changed(block, None))
    }
}
