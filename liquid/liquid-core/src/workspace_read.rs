use automerge::ROOT;

use crate::model::{self, ICON, NAME, PAGE, STYLE, VIEWS};
use crate::store::Store;
use crate::workspace::Workspace;
use crate::workspace_page::text_block;
use crate::{
    Action, ActionId, Area, AreaId, BlockId, Code, Document, Error, FieldId, Media, PageEntry,
    PageId, TableEntry, TextStyle, View, ViewId, calculation, chart, database, drawing,
};

impl<S: Store> Workspace<S> {
    pub fn areas(&mut self) -> Result<Vec<Area>, Error> {
        Ok(model::areas(self.doc(&Document::Workspace)?))
    }

    pub fn pages(&mut self, area: &AreaId) -> Result<Vec<PageEntry>, Error> {
        Ok(model::pages(self.doc(&Document::Area(area.clone()))?))
    }

    pub fn views(&mut self, page: &PageId, start: usize, limit: usize) -> Result<Vec<View>, Error> {
        Ok(self.page(page)?.1.window(start, limit))
    }

    pub fn arranged(&mut self, page: &PageId) -> Result<bool, Error> {
        Ok(self.page(page)?.1.arranged())
    }

    pub fn view_count(&mut self, page: &PageId) -> Result<usize, Error> {
        Ok(self.page(page)?.1.len())
    }

    pub fn style(&mut self, page: &PageId, block: &BlockId) -> Result<Option<TextStyle>, Error> {
        let doc = self.page_doc(page)?;
        Ok(text_block(doc, block)
            .map(|entry| model::style(model::string(doc, &entry, STYLE).as_deref())))
    }

    pub fn text(&mut self, page: &PageId, block: &BlockId) -> Result<Option<String>, Error> {
        Ok(model::text(self.page_doc(page)?, block.as_str()))
    }

    pub fn code(&mut self, page: &PageId, block: &BlockId) -> Result<Option<Code>, Error> {
        Ok(model::code(self.page_doc(page)?, block.as_str()))
    }

    pub fn math(&mut self, page: &PageId, block: &BlockId) -> Result<Option<String>, Error> {
        Ok(model::math(self.page_doc(page)?, block.as_str()))
    }

    pub fn media(&mut self, page: &PageId, block: &BlockId) -> Result<Option<Media>, Error> {
        Ok(model::media(self.page_doc(page)?, block.as_str()))
    }

    pub fn drawing(
        &mut self,
        page: &PageId,
        block: &BlockId,
        pages: std::ops::Range<u32>,
    ) -> Result<Option<drawing::Drawing>, Error> {
        drawing::read(self.page_doc(page)?, block, pages)
    }

    pub fn calculation(
        &mut self,
        page: &PageId,
        block: &BlockId,
    ) -> Result<Option<calculation::Calculation>, Error> {
        calculation::read(self, page, block)
    }

    pub fn chart(&mut self, page: &PageId, block: &BlockId) -> Result<Option<chart::Chart>, Error> {
        chart::read(self, page, block)
    }

    pub fn name_and_icon(&mut self) -> Result<(String, String), Error> {
        let doc = self.doc(&Document::Workspace)?;
        Ok((
            model::string(doc, &ROOT, NAME).unwrap_or_default(),
            model::string(doc, &ROOT, ICON).unwrap_or_default(),
        ))
    }

    pub fn tables(&mut self) -> Result<Vec<TableEntry>, Error> {
        Ok(model::tables(self.doc(&Document::Workspace)?))
    }

    pub fn database(&mut self, table: &BlockId) -> Result<Option<database::Database>, Error> {
        let Some(name) = self.table_name(table)? else {
            return Ok(None);
        };
        let read = database::read(self.doc(&Document::Table(table.clone()))?, table)?;
        Ok(read.map(|database| database::Database { name, ..database }))
    }

    pub fn rows(
        &mut self,
        table: &BlockId,
        query: &database::RowQuery,
    ) -> Result<database::Rows, Error> {
        let (doc, index) = self.table_index(table)?;
        let mut rows = database::rows(doc, index, table, query)?;
        rows.uses = calculation::fill(self, table, &mut rows.rows)?;
        if !query.totals.is_empty() {
            let all = self.all_rows(table, query)?;
            rows.totals = crate::total::compute(&all, &query.totals);
        }
        Ok(rows)
    }

    pub fn groups(
        &mut self,
        table: &BlockId,
        query: &database::RowQuery,
        field: &FieldId,
    ) -> Result<Vec<database::Group>, Error> {
        let (doc, index) = self.table_index(table)?;
        let mut groups = database::groups(doc, index, table, query, field)?;
        for group in &mut groups {
            calculation::fill(self, table, &mut group.rows)?;
        }
        if !query.totals.is_empty() {
            let all = self.all_rows(table, query)?;
            for group in &mut groups {
                let within: Vec<_> = all
                    .iter()
                    .filter(|row| database::in_group(row.cell(field), &group.value))
                    .cloned()
                    .collect();
                group.totals = crate::total::compute(&within, &query.totals);
            }
        }
        Ok(groups)
    }

    fn all_rows(
        &mut self,
        table: &BlockId,
        query: &database::RowQuery,
    ) -> Result<Vec<database::Row>, Error> {
        let all = database::RowQuery {
            start: 0,
            limit: usize::MAX,
            ..query.clone()
        };
        let (doc, index) = self.database_index(table)?;
        let mut rows = database::rows(doc, index, table, &all)?.rows;
        calculation::fill(self, table, &mut rows)?;
        Ok(rows)
    }

    pub fn table_views(&mut self, table: &BlockId) -> Result<Vec<(PageId, ViewId)>, Error> {
        if self.table_name(table)?.is_none() {
            return Err(Error::NotFound);
        }
        let doc = self.doc(&Document::Table(table.clone()))?;
        Ok(model::sorted(doc, VIEWS)
            .into_iter()
            .filter_map(|entry| Some((PageId::stored(entry.get(PAGE)?), ViewId::stored(&entry.id))))
            .collect())
    }

    pub fn view(&mut self, page: &PageId, view: &ViewId) -> Result<Option<View>, Error> {
        Ok(self.page(page)?.1.view(view).cloned())
    }

    pub fn history(
        &mut self,
        before: Option<&ActionId>,
        limit: usize,
    ) -> Result<Vec<Action>, Error> {
        self.store.history(before, limit)
    }
}
