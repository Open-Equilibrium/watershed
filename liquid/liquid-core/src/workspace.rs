use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use automerge::transaction::{CommitOptions, Transactable};
use automerge::{ActorId, AutoCommit, ObjType, ROOT, ReadDoc};

use crate::history::Header;
use crate::id::random_bytes;
use crate::model::{
    self, AREA, AREAS, BLOCK, BLOCKS, BOUNDARY, GRID_COLUMNS, ICON, INNER, KIND, LANGUAGE, NAME,
    OWNER, PAGE, PAGES, PARENT, SORT, STYLE, TABLE, TABLES, TEXT, TILE, TITLE, VIEWS,
};

const MAX_ICON: usize = 64;
use crate::order::Order;
use crate::store::{Commit, Store, Write};
use crate::{
    Action, ActionId, Actor, Applied, Area, AreaId, AttachmentId, BlockId, BlockKind, Cell,
    Changed, Code, Created, Document, Error, FieldId, FieldKind, Found, Hit, INDEX_VERSION,
    IndexEntry, IndexWrite, Layout, Media, Mutation, Operation, Origin, PageEntry, PageId,
    Permission, PersonId, Place, Request, RowId, TabId, TabSetup, TableEntry, TextStyle, Tile,
    View, ViewId, search_words,
};
use crate::{calculation, database, drawing, tab};

const COMPACT_AFTER: usize = 1_000;
pub const MAX_DEVICE_KEY: usize = 256;
pub const MAX_DEVICE_VALUE: usize = 4_096;
pub const MAX_DEVICE_VALUES: usize = 10_000;

pub struct Workspace<S: Store> {
    pub(crate) store: S,
    pub(crate) device: ActorId,
    owner: PersonId,
    pub(crate) docs: HashMap<Document, Loaded>,
}

pub(crate) struct Loaded {
    pub doc: AutoCommit,
    chunks: usize,
    order: Option<Order>,
    databases: HashMap<BlockId, database::Index>,
}

struct Written {
    created: Option<Created>,
    changed: Vec<Changed>,
    touched: Vec<Document>,
    new: Vec<(Document, AutoCommit)>,
}

impl Written {
    fn new(touched: Document, changed: Changed) -> Self {
        Self {
            created: None,
            changed: vec![changed],
            touched: vec![touched],
            new: Vec::new(),
        }
    }
}

impl<S: Store> Workspace<S> {
    pub fn create(owner: PersonId, mut store: S) -> Result<Self, Error> {
        if !store.chunks(&Document::Workspace)?.is_empty() {
            return Err(Error::Storage("the store already holds a Workspace".into()));
        }
        let device = ActorId::from(&random_bytes()?[..]);
        let mut doc = model::document(&device);
        doc.put(ROOT, OWNER, owner.as_str())?;
        model::create_maps(&mut doc, &[AREAS])?;
        let mut this = Self {
            store,
            device,
            owner: owner.clone(),
            docs: HashMap::new(),
        };
        let action = Action {
            id: ActionId::new()?,
            actor: Actor::Person(owner),
            origin: Origin::Ui,
            permission: Permission::Owner,
            operation: Operation::CreateWorkspace,
            changes: Vec::new(),
        };
        this.record(
            action,
            &[Document::Workspace],
            vec![(Document::Workspace, doc)],
            Vec::new(),
        )?;
        this.store.index(&[], Some(INDEX_VERSION)).ok();
        Ok(this)
    }

    pub fn open(store: S) -> Result<Self, Error> {
        let mut this = Self {
            store,
            device: ActorId::from(&random_bytes()?[..]),
            owner: PersonId::new(""),
            docs: HashMap::new(),
        };
        let owner = model::string(this.doc(&Document::Workspace)?, &ROOT, OWNER)
            .ok_or_else(|| Error::Document("the Workspace document names no owner".into()))?;
        this.owner = PersonId::new(owner);
        this.ensure_index().ok();
        Ok(this)
    }

    pub fn into_store(self) -> S {
        self.store
    }

    pub fn search(&mut self, query: &str, found: Found, limit: usize) -> Result<Vec<Hit>, Error> {
        if query.chars().count() > database::MAX_FILTER_TEXT {
            return Err(Error::TooLong);
        }
        let words = search_words(query);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        self.store.search(&words, found, limit)
    }

    fn ensure_index(&mut self) -> Result<(), Error> {
        if self.store.index_version()? >= INDEX_VERSION {
            return Ok(());
        }
        let mut complete = true;
        for area in self.areas()? {
            for page in self.pages(&area.id)? {
                let mut writes = vec![page_entry(&page.id, &page.title)];
                match self.page_text(&page.id) {
                    Ok(text) => writes.extend(text),
                    Err(_) => complete = false,
                }
                self.close_page(&page.id);
                self.store.index(&writes, None)?;
            }
        }
        let tables: Vec<IndexWrite> = self
            .tables()?
            .iter()
            .map(|table| table_entry(&table.id, Some(&table.name)))
            .collect();
        self.store.index(&tables, complete.then_some(INDEX_VERSION))
    }

    fn page_text(&mut self, page: &PageId) -> Result<Vec<IndexWrite>, Error> {
        let blocks: Vec<BlockId> = self
            .views(page, 0, usize::MAX)?
            .into_iter()
            .filter(|view| matches!(view.kind, BlockKind::Text | BlockKind::Code))
            .map(|view| view.block)
            .collect();
        blocks
            .iter()
            .map(|block| self.text_entry(page, block))
            .collect()
    }

    fn text_entry(&mut self, page: &PageId, block: &BlockId) -> Result<IndexWrite, Error> {
        let text = match self.text(page, block)? {
            Some(text) => Some(text),
            None => self.code(page, block)?.map(|code| code.text),
        };
        Ok(IndexWrite {
            found: Found::Text,
            id: block.to_string(),
            entry: text.map(|text| IndexEntry {
                page: Some(page.clone()),
                text,
            }),
        })
    }

    fn leaving(&mut self, mutation: &Mutation) -> Option<(PageId, BlockId)> {
        let (Mutation::RemoveView { page, view } | Mutation::JoinTextBlock { page, view }) =
            mutation
        else {
            return None;
        };
        let block = self.page(page).ok()?.1.view(view)?.block.clone();
        Some((page.clone(), block))
    }

    fn index_writes(
        &mut self,
        mutation: &Mutation,
        written: &Written,
        leaving: Option<(PageId, BlockId)>,
    ) -> Result<Vec<IndexWrite>, Error> {
        let mut writes = Vec::new();
        let mut blocks: Vec<(PageId, BlockId)> = leaving.into_iter().collect();
        let mut tables: Vec<BlockId> = Vec::new();
        match (mutation, &written.created) {
            (Mutation::CreatePage { title, .. }, Some(Created::Page(page))) => {
                writes.push(page_entry(page, title));
            }
            (Mutation::RenamePage { page, title }, _) => writes.push(page_entry(page, title)),
            (
                Mutation::AddTextBlock { page, .. } | Mutation::SplitTextBlock { page, .. },
                Some(Created::Block { block, .. }),
            ) => blocks.push((page.clone(), block.clone())),
            (Mutation::CreateTable { .. }, Some(Created::Table(table)))
            | (
                Mutation::AddDatabaseBlock { .. } | Mutation::DuplicateTable { .. },
                Some(Created::Block { block: table, .. }),
            ) => tables.push(table.clone()),
            _ => {}
        }
        if let Mutation::SplitTextBlock { page, block, .. } = mutation {
            blocks.push((page.clone(), block.clone()));
        }
        for changed in &written.changed {
            match changed {
                Changed::Block(page, block) => blocks.push((page.clone(), block.clone())),
                Changed::Table(table) => tables.push(table.clone()),
                _ => {}
            }
        }
        blocks.sort();
        blocks.dedup();
        tables.sort();
        tables.dedup();
        for (page, block) in blocks {
            writes.push(self.text_entry(&page, &block)?);
        }
        for table in tables {
            let name = self.table_name(&table)?;
            writes.push(table_entry(&table, name.as_deref()));
        }
        Ok(writes)
    }

    pub fn apply(&mut self, request: Request) -> Result<Applied, Error> {
        let permission = self.permit(&request.actor)?;
        let id = ActionId::new()?;
        let leaving = self.leaving(&request.mutation);
        let written = match self.write(&request.mutation) {
            Ok(written) => written,
            Err(error) => {
                self.rollback();
                return Err(error);
            }
        };
        let unchanged = written.new.is_empty()
            && written.touched.iter().all(|document| {
                self.docs
                    .get(document)
                    .is_none_or(|loaded| loaded.doc.pending_ops() == 0)
            });
        if unchanged {
            return Err(Error::NoChange);
        }
        let index = match self.index_writes(&request.mutation, &written, leaving) {
            Ok(index) => index,
            Err(error) => {
                self.rollback();
                return Err(error);
            }
        };
        let action = Action {
            id: id.clone(),
            actor: request.actor,
            origin: request.origin,
            permission,
            operation: Operation::Mutation(request.mutation),
            changes: Vec::new(),
        };
        self.record(action, &written.touched, written.new, index)?;
        Ok(Applied {
            action: id,
            created: written.created,
            changed: written.changed,
        })
    }

    pub fn owner(&self) -> &PersonId {
        &self.owner
    }

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
        let doc = self.doc(&Document::Page(page.clone()))?;
        Ok(text_block(doc, block)
            .map(|entry| model::style(model::string(doc, &entry, STYLE).as_deref())))
    }

    pub fn text(&mut self, page: &PageId, block: &BlockId) -> Result<Option<String>, Error> {
        Ok(model::text(
            self.doc(&Document::Page(page.clone()))?,
            block.as_str(),
        ))
    }

    pub fn code(&mut self, page: &PageId, block: &BlockId) -> Result<Option<Code>, Error> {
        Ok(model::code(
            self.doc(&Document::Page(page.clone()))?,
            block.as_str(),
        ))
    }

    pub fn media(&mut self, page: &PageId, block: &BlockId) -> Result<Option<Media>, Error> {
        Ok(model::media(
            self.doc(&Document::Page(page.clone()))?,
            block.as_str(),
        ))
    }

    pub fn drawing(
        &mut self,
        page: &PageId,
        block: &BlockId,
        pages: std::ops::Range<u32>,
    ) -> Result<Option<drawing::Drawing>, Error> {
        drawing::read(self.doc(&Document::Page(page.clone()))?, block, pages)
    }

    pub fn calculation(
        &mut self,
        page: &PageId,
        block: &BlockId,
    ) -> Result<Option<calculation::Calculation>, Error> {
        calculation::read(self, page, block)
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
        if self.table_name(table)?.is_none() {
            return Err(Error::NotFound);
        }
        let (doc, index) = self.database_index(table)?;
        let mut rows = database::rows(doc, index, table, query)?;
        calculation::fill(self, table, &mut rows.rows)?;
        Ok(rows)
    }

    pub fn groups(
        &mut self,
        table: &BlockId,
        query: &database::RowQuery,
        field: &FieldId,
    ) -> Result<Vec<database::Group>, Error> {
        if self.table_name(table)?.is_none() {
            return Err(Error::NotFound);
        }
        let (doc, index) = self.database_index(table)?;
        let mut groups = database::groups(doc, index, table, query, field)?;
        for group in &mut groups {
            calculation::fill(self, table, &mut group.rows)?;
        }
        Ok(groups)
    }

    pub fn remembered(&mut self, key: &str) -> Result<Option<String>, Error> {
        self.store.remembered(key)
    }

    pub fn remember(&mut self, key: &str, value: Option<&str>) -> Result<(), Error> {
        if key.len() > MAX_DEVICE_KEY || value.is_some_and(|value| value.len() > MAX_DEVICE_VALUE) {
            return Err(Error::TooLong);
        }
        self.store.remember(key, value, MAX_DEVICE_VALUES)
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

    fn table_name(&mut self, table: &BlockId) -> Result<Option<String>, Error> {
        let doc = self.doc(&Document::Workspace)?;
        Ok(model::live(doc, TABLES, table.as_str())
            .map(|entry| model::string(doc, &entry, NAME).unwrap_or_default()))
    }

    pub fn is_loaded(&self, page: &PageId) -> bool {
        self.docs.contains_key(&Document::Page(page.clone()))
    }

    pub fn close_page(&mut self, page: &PageId) {
        self.docs.remove(&Document::Page(page.clone()));
    }

    pub fn retain_loaded(&mut self, mut keep: impl FnMut(&Document) -> bool) {
        self.docs.retain(|document, _| {
            matches!(document, Document::Workspace | Document::Area(_)) || keep(document)
        });
    }

    pub fn history(
        &mut self,
        before: Option<&ActionId>,
        limit: usize,
    ) -> Result<Vec<Action>, Error> {
        self.store.history(before, limit)
    }

    fn page(&mut self, page: &PageId) -> Result<(&mut AutoCommit, &mut Order), Error> {
        let document = Document::Page(page.clone());
        self.doc(&document)?;
        let loaded = self.docs.get_mut(&document).expect("loaded above");
        let order = loaded.order.get_or_insert_with(|| Order::read(&loaded.doc));
        Ok((&mut loaded.doc, order))
    }

    pub(crate) fn database_index(
        &mut self,
        block: &BlockId,
    ) -> Result<(&mut AutoCommit, &mut database::Index), Error> {
        let document = Document::Table(block.clone());
        self.doc(&document)?;
        let loaded = self.docs.get_mut(&document).expect("loaded above");
        if !loaded.databases.contains_key(block) {
            let index = database::Index::read(&loaded.doc, block)?;
            loaded.databases.insert(block.clone(), index);
        }
        let index = loaded.databases.get_mut(block).expect("read above");
        Ok((&mut loaded.doc, index))
    }

    pub(crate) fn doc(&mut self, document: &Document) -> Result<&mut AutoCommit, Error> {
        if !self.docs.contains_key(document) {
            let chunks = self.store.chunks(document)?;
            if chunks.is_empty() {
                return Err(Error::NotFound);
            }
            let doc = model::load(&chunks.concat(), &self.device)?;
            let loaded = Loaded {
                doc,
                chunks: chunks.len(),
                order: None,
                databases: HashMap::new(),
            };
            self.docs.insert(document.clone(), loaded);
        }
        Ok(&mut self.docs.get_mut(document).expect("loaded above").doc)
    }

    pub(crate) fn permit(&self, actor: &Actor) -> Result<Permission, Error> {
        let Actor::Person(person) = actor;
        if *person == self.owner {
            Ok(Permission::Owner)
        } else {
            Err(Error::NotAllowed)
        }
    }

    fn write(&mut self, mutation: &Mutation) -> Result<Written, Error> {
        let page = match mutation {
            Mutation::AddTextBlock { page, .. }
            | Mutation::SplitTextBlock { page, .. }
            | Mutation::JoinTextBlock { page, .. }
            | Mutation::SetSectionBoundary { page, .. }
            | Mutation::TurnIntoCode { page, .. }
            | Mutation::TurnIntoText { page, .. }
            | Mutation::AddMediaBlock { page, .. }
            | Mutation::AddWhiteboardBlock { page, .. }
            | Mutation::AddCalculationBlock { page, .. }
            | Mutation::AddDatabaseBlock { page, .. }
            | Mutation::AddTableView { page, .. }
            | Mutation::DuplicateTable { page, .. }
            | Mutation::RemoveView { page, .. }
            | Mutation::MoveViews { page, .. } => page,
            Mutation::DeleteTable { block } => return self.write_across_pages(mutation, block),
            _ => return self.write_mutation(mutation),
        };
        let order = self.page(page)?.1;
        if !order.arranged() {
            return self.write_mutation(mutation);
        }
        let before = order.sections();
        let mut written = self.write_mutation(mutation)?;
        if self.carry_tiles(page, &before)? {
            let document = Document::Page(page.clone());
            if !written.touched.contains(&document) {
                written.touched.push(document);
            }
            written.changed.push(Changed::Views(page.clone()));
        }
        Ok(written)
    }

    fn write_across_pages(
        &mut self,
        mutation: &Mutation,
        table: &BlockId,
    ) -> Result<Written, Error> {
        let mut pages: Vec<PageId> = self
            .table_views(table)?
            .into_iter()
            .map(|(page, _)| page)
            .collect();
        pages.sort();
        pages.dedup();
        let mut before = Vec::new();
        for page in pages {
            let order = match self.page(&page) {
                Ok((_, order)) => order,
                Err(Error::NotFound) => continue,
                Err(error) => return Err(error),
            };
            if order.arranged() {
                before.push((page, order.sections()));
            }
        }
        let mut written = self.write_mutation(mutation)?;
        for (page, sections) in &before {
            if self.carry_tiles(page, sections)? {
                let document = Document::Page(page.clone());
                if !written.touched.contains(&document) {
                    written.touched.push(document);
                }
                written.changed.push(Changed::Views(page.clone()));
            }
        }
        Ok(written)
    }

    fn carry_tiles(
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

    fn write_mutation(&mut self, mutation: &Mutation) -> Result<Written, Error> {
        match mutation {
            Mutation::CreateArea { name } => self.write_create_area(name),
            Mutation::RenameWorkspace { name } => self.write_workspace(NAME, name),
            Mutation::SetWorkspaceIcon { icon } => {
                if icon.len() > MAX_ICON {
                    return Err(Error::TooLong);
                }
                self.write_workspace(ICON, icon)
            }
            Mutation::CreatePage {
                area,
                parent,
                title,
            } => self.write_create_page(area, parent.as_ref(), title),
            Mutation::RenamePage { page, title } => self.write_rename_page(page, title),
            Mutation::AddTextBlock { page, after, text } => {
                self.write_add_text_block(page, after.as_ref(), text)
            }
            Mutation::SplitTextBlock {
                page,
                block,
                view,
                at,
                style,
            } => self.write_split_text_block(page, block, view, *at, *style),
            Mutation::JoinTextBlock { page, view } => self.write_join_text_block(page, view),
            Mutation::SetTextStyle { page, block, style } => {
                self.write_set_text_style(page, block, *style)
            }
            Mutation::SetSectionBoundary {
                page,
                view,
                boundary,
            } => self.write_set_section_boundary(page, view, *boundary),
            Mutation::EditText {
                page,
                block,
                at,
                delete,
                insert,
            } => self.write_edit_text(page, block, *at, *delete, insert),
            Mutation::TurnIntoCode { page, block } => {
                self.write_turn_into(page, block, BlockKind::Text, (BlockKind::Code, "code"))
            }
            Mutation::TurnIntoText { page, block } => {
                self.write_turn_into(page, block, BlockKind::Code, (BlockKind::Text, "text"))
            }
            Mutation::SetCodeLanguage {
                page,
                block,
                language,
            } => self.write_set_code_language(page, block, language),
            Mutation::AddMediaBlock {
                page,
                after,
                name,
                media_type,
                attachment,
            } => self.write_add_media_block(page, after.as_ref(), name, media_type, attachment),
            Mutation::AddWhiteboardBlock { page, after } => {
                self.write_add_whiteboard_block(page, after.as_ref())
            }
            Mutation::AddCalculationBlock { page, after } => {
                self.write_add_calculation_block(page, after.as_ref())
            }
            Mutation::AddLine {
                page,
                block,
                after,
                line,
            } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                let id = calculation::add_line(doc, block, after.as_ref(), line)?;
                Ok(block_changed(page, block, Some(Created::Line(id))))
            }
            Mutation::SetLine {
                page,
                block,
                line,
                setup,
            } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                calculation::set_line(doc, block, line, setup)?;
                Ok(block_changed(page, block, None))
            }
            Mutation::RemoveLine { page, block, line } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                calculation::remove_line(doc, block, line)?;
                Ok(block_changed(page, block, None))
            }
            Mutation::SetDrawingStyle { page, block, style } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                drawing::set_style(doc, block, *style)?;
                Ok(block_changed(page, block, None))
            }
            Mutation::EditDrawing {
                page,
                block,
                add,
                change,
                remove,
            } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                let ids = drawing::edit(doc, block, add, change, remove)?;
                let created = (!ids.is_empty()).then_some(Created::Elements(ids));
                Ok(block_changed(page, block, created))
            }
            Mutation::AddDatabaseBlock {
                page,
                after,
                name,
                layout,
            } => self.write_add_database_block(page, after.as_ref(), name, *layout),
            Mutation::DuplicateTable {
                page,
                after,
                table,
                name,
                tabs,
            } => self.write_duplicate_table(page, after.as_ref(), table, name, tabs),
            Mutation::CreateTable { area, name } => {
                let (table, mut written) = self.write_create_table(area, name, None)?;
                written.created = Some(Created::Table(table));
                Ok(written)
            }
            Mutation::AddTableView {
                page,
                after,
                table,
                title,
                tabs,
            } => self.write_add_table_view(page, after.as_ref(), table, title, tabs),
            Mutation::RenameView { page, view, title } => self.write_rename_view(page, view, title),
            Mutation::DeleteTable { block } => self.write_delete_table(block),
            Mutation::RenameDatabase { block, name } => self.write_rename_database(block, name),
            Mutation::AddField {
                block,
                place,
                name,
                kind,
            } => {
                let doc = self.table_doc(block)?;
                let field = database::add_field(doc, block, place.as_ref(), name, *kind)?;
                Ok(table_changed(block, Some(Created::Field(field))))
            }
            Mutation::SetFieldFormula {
                block,
                field,
                formula,
            } => {
                database::set_formula(self.table_doc(block)?, block, field, formula)?;
                Ok(table_changed(block, None))
            }
            Mutation::RenameField { block, field, name } => {
                database::rename_field(self.table_doc(block)?, block, field, name)?;
                Ok(table_changed(block, None))
            }
            Mutation::RemoveField { block, field } => {
                database::remove_field(self.table_doc(block)?, block, field)?;
                Ok(table_changed(block, None))
            }
            Mutation::AddOption {
                block,
                field,
                name,
                color,
            } => {
                let doc = self.table_doc(block)?;
                let option = database::add_option(doc, block, field, name, *color)?;
                Ok(table_changed(block, Some(Created::Option(option))))
            }
            Mutation::AddRow { block, place } => {
                self.table_doc(block)?;
                let (doc, index) = self.database_index(block)?;
                let row = database::add_row(doc, index, block, place.as_ref())?;
                Ok(table_changed(block, Some(Created::Row(row))))
            }
            Mutation::MoveRow { block, row, place } => {
                self.table_doc(block)?;
                let (doc, index) = self.database_index(block)?;
                database::move_row(doc, index, block, row, place.as_ref())?;
                Ok(table_changed(block, None))
            }
            Mutation::RemoveRow { block, row } => {
                self.table_doc(block)?;
                let (doc, index) = self.database_index(block)?;
                database::remove_row(doc, index, block, row)?;
                Ok(table_changed(block, None))
            }
            Mutation::SetCell {
                block,
                row,
                field,
                value,
            } => self.write_set_cell(block, row, field, value),
            Mutation::RemoveView { page, view } => self.write_remove_view(page, view),
            Mutation::SetTile { page, view, tile } => self.write_tile(page, view, *tile, false),
            Mutation::SetInnerTile { page, view, tile } => self.write_tile(page, view, *tile, true),
            Mutation::MoveViews {
                page,
                first,
                last,
                after,
            } => self.write_move_views(page, first, last, after.as_ref()),
            Mutation::AddTab {
                page,
                view,
                after,
                setup,
            } => self.write_add_tab(page, view, after.as_ref(), setup),
            Mutation::RenameTab {
                page,
                view,
                tab,
                name,
            } => self.write_tab(page, view, tab, |setup| setup.name.clone_from(name)),
            Mutation::SetTabLayout {
                page,
                view,
                tab,
                layout,
            } => self.write_tab(page, view, tab, |setup| setup.layout = *layout),
            Mutation::SetTabGroup {
                page,
                view,
                tab,
                group,
            } => self.write_tab(page, view, tab, |setup| setup.group.clone_from(group)),
            Mutation::SetTabDate {
                page,
                view,
                tab,
                date,
            } => self.write_tab(page, view, tab, |setup| setup.date.clone_from(date)),
            Mutation::SetTabSort {
                page,
                view,
                tab,
                sort,
            } => self.write_tab(page, view, tab, |setup| setup.sort.clone_from(sort)),
            Mutation::SetTabFilter {
                page,
                view,
                tab,
                filter,
            } => self.write_tab(page, view, tab, |setup| setup.filter.clone_from(filter)),
            Mutation::SetTabFolded {
                page,
                view,
                tab,
                folded,
            } => self.write_tab(page, view, tab, |setup| setup.folded.clone_from(folded)),
            Mutation::MoveTab {
                page,
                view,
                tab,
                after,
            } => self.write_tabs(page, view, |doc, entry| {
                tab::move_after(doc, entry, tab, after.as_ref()).map(|()| None)
            }),
            Mutation::RemoveTab { page, view, tab } => self.write_tabs(page, view, |doc, entry| {
                tab::remove(doc, entry, tab).map(|()| None)
            }),
        }
    }

    fn write_workspace(&mut self, key: &str, value: &str) -> Result<Written, Error> {
        self.doc(&Document::Workspace)?.put(ROOT, key, value)?;
        Ok(Written::new(Document::Workspace, Changed::Workspace))
    }

    fn write_create_area(&mut self, name: &str) -> Result<Written, Error> {
        let id = AreaId::new()?;
        let mut doc = model::document(&self.device);
        model::create_maps(&mut doc, &[PAGES])?;
        let workspace = self.doc(&Document::Workspace)?;
        let siblings = model::sorted(workspace, AREAS);
        let last = siblings.last().map(|entry| entry.id.as_str());
        let sort = model::sort_after(&siblings, last)?;
        model::insert(
            workspace,
            AREAS,
            id.as_str(),
            &[(NAME, name), (SORT, &sort)],
        )?;
        let mut written = Written::new(Document::Workspace, Changed::Areas);
        written.touched.push(Document::Area(id.clone()));
        written.created = Some(Created::Area(id.clone()));
        written.new.push((Document::Area(id), doc));
        Ok(written)
    }

    fn write_create_page(
        &mut self,
        area: &AreaId,
        parent: Option<&PageId>,
        title: &str,
    ) -> Result<Written, Error> {
        let area_doc = self.doc(&Document::Area(area.clone()))?;
        if let Some(parent) = parent {
            model::live(area_doc, PAGES, parent.as_str()).ok_or(Error::NotFound)?;
        }
        let siblings: Vec<_> = model::sorted(area_doc, PAGES)
            .into_iter()
            .filter(|entry| entry.get(PARENT) == parent.map(PageId::as_str))
            .collect();
        let last = siblings.last().map(|entry| entry.id.as_str());
        let sort = model::sort_after(&siblings, last)?;
        let id = PageId::new()?;
        let mut fields = vec![(TITLE, title), (SORT, &sort)];
        if let Some(parent) = parent {
            fields.push((PARENT, parent.as_str()));
        }
        model::insert(area_doc, PAGES, id.as_str(), &fields)?;
        let mut doc = model::document(&self.device);
        model::create_maps(&mut doc, &[BLOCKS, VIEWS])?;
        let mut written = Written::new(Document::Area(area.clone()), Changed::Pages(area.clone()));
        written.touched.push(Document::Page(id.clone()));
        written.created = Some(Created::Page(id.clone()));
        written.new.push((Document::Page(id), doc));
        Ok(written)
    }

    fn write_rename_page(&mut self, page: &PageId, title: &str) -> Result<Written, Error> {
        let area = self.area_of(page)?;
        let area_doc = self.doc(&Document::Area(area.clone()))?;
        let entry = model::live(area_doc, PAGES, page.as_str()).ok_or(Error::NotFound)?;
        area_doc.put(&entry, TITLE, title)?;
        Ok(Written::new(
            Document::Area(area.clone()),
            Changed::Pages(area),
        ))
    }

    fn area_of(&mut self, page: &PageId) -> Result<AreaId, Error> {
        for area in model::areas(self.doc(&Document::Workspace)?) {
            let area_doc = self.doc(&Document::Area(area.id.clone()))?;
            if model::live(area_doc, PAGES, page.as_str()).is_some() {
                return Ok(area.id);
            }
        }
        Err(Error::NotFound)
    }

    fn table_doc(&mut self, table: &BlockId) -> Result<&mut AutoCommit, Error> {
        if self.table_name(table)?.is_none() {
            return Err(Error::NotFound);
        }
        self.doc(&Document::Table(table.clone()))
    }

    fn write_add_text_block(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
        text: &str,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let sort = order.key_after(after)?;
        let (block, view) = add_text_block(doc, order, sort, text, TextStyle::Paragraph)?;
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written.created = Some(Created::Block { block, view });
        Ok(written)
    }

    fn write_split_text_block(
        &mut self,
        page: &PageId,
        block: &BlockId,
        view: &ViewId,
        at: usize,
        style: TextStyle,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        if order.view(view).is_none_or(|shown| &shown.block != block) {
            return Err(Error::NotFound);
        }
        let content = text_of(doc, block)?;
        if !model::on_boundary(doc, &content, at) {
            return Err(Error::OutOfRange);
        }
        let (new_block, new_view) = if at == 0 {
            let starts_section = order.view(view).is_some_and(|shown| shown.boundary);
            let sort = order.key_before(view)?;
            let added = add_text_block(doc, order, sort, "", style)?;
            if starts_section {
                move_boundary(doc, order, view, &added.1)?;
            }
            added
        } else {
            let text = doc.text(&content)?;
            let tail = text[utf8_offset(&text, at)..].to_string();
            let length =
                isize::try_from(doc.length(&content) - at).map_err(|_| Error::OutOfRange)?;
            doc.splice_text(&content, at, length, "")?;
            let sort = order.key_after(Some(view))?;
            add_text_block(doc, order, sort, &tail, style)?
        };
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written
            .changed
            .push(Changed::Block(page.clone(), block.clone()));
        written.created = Some(Created::Block {
            block: new_block,
            view: new_view,
        });
        Ok(written)
    }

    fn write_join_text_block(&mut self, page: &PageId, view: &ViewId) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let joining = order.view(view).ok_or(Error::NotFound)?.clone();
        if joining.boundary {
            return Err(Error::SectionBoundary);
        }
        let above = order.above(view).ok_or(Error::NotFound)?.clone();
        let text = model::text(doc, joining.block.as_str()).ok_or(Error::NotFound)?;
        let target = text_of(doc, &above.block)?;
        let end = doc.length(&target);
        doc.splice_text(&target, end, 0, &text)?;
        let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
        model::mark_deleted(doc, &entry)?;
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written
            .changed
            .push(Changed::Block(page.clone(), above.block.clone()));
        if !order.shows(&joining.block, view) {
            if let Some(block_entry) = model::live(doc, BLOCKS, joining.block.as_str()) {
                model::mark_deleted(doc, &block_entry)?;
            }
            written
                .changed
                .push(Changed::Block(page.clone(), joining.block));
        }
        order.remove(view);
        Ok(written)
    }

    fn write_set_text_style(
        &mut self,
        page: &PageId,
        block: &BlockId,
        style: TextStyle,
    ) -> Result<Written, Error> {
        let name = model::style_name(style)?;
        let (doc, _) = self.page(page)?;
        let entry = text_block(doc, block).ok_or(Error::NotFound)?;
        doc.put(&entry, STYLE, name)?;
        Ok(Written::new(
            Document::Page(page.clone()),
            Changed::Block(page.clone(), block.clone()),
        ))
    }

    fn write_set_section_boundary(
        &mut self,
        page: &PageId,
        view: &ViewId,
        boundary: bool,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
        doc.put(&entry, BOUNDARY, boundary)?;
        order.set_boundary(view, boundary);
        Ok(Written::new(
            Document::Page(page.clone()),
            Changed::Views(page.clone()),
        ))
    }

    fn write_edit_text(
        &mut self,
        page: &PageId,
        block: &BlockId,
        at: usize,
        delete: usize,
        insert: &str,
    ) -> Result<Written, Error> {
        let doc = self.doc(&Document::Page(page.clone()))?;
        let entry = model::of_kind(doc, block.as_str(), BlockKind::Text)
            .or_else(|| model::of_kind(doc, block.as_str(), BlockKind::Code))
            .ok_or(Error::NotFound)?;
        let content = model::child(doc, &entry, TEXT).ok_or(Error::NotFound)?;
        let end = at.checked_add(delete).ok_or(Error::OutOfRange)?;
        if !model::on_boundary(doc, &content, at) || !model::on_boundary(doc, &content, end) {
            return Err(Error::OutOfRange);
        }
        let delete = isize::try_from(delete).map_err(|_| Error::OutOfRange)?;
        doc.splice_text(&content, at, delete, insert)?;
        Ok(Written::new(
            Document::Page(page.clone()),
            Changed::Block(page.clone(), block.clone()),
        ))
    }

    fn write_turn_into(
        &mut self,
        page: &PageId,
        block: &BlockId,
        from: BlockKind,
        (to, name): (BlockKind, &str),
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let entry = model::of_kind(doc, block.as_str(), from).ok_or(Error::NotFound)?;
        doc.put(&entry, KIND, name)?;
        match to {
            BlockKind::Text => doc.put(&entry, STYLE, model::style_name(TextStyle::Paragraph)?)?,
            _ => doc.delete(&entry, LANGUAGE)?,
        }
        order.set_kind(block, to);
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written
            .changed
            .push(Changed::Block(page.clone(), block.clone()));
        Ok(written)
    }

    fn write_set_code_language(
        &mut self,
        page: &PageId,
        block: &BlockId,
        language: &str,
    ) -> Result<Written, Error> {
        let doc = self.doc(&Document::Page(page.clone()))?;
        let entry = model::of_kind(doc, block.as_str(), BlockKind::Code).ok_or(Error::NotFound)?;
        if language.is_empty() {
            doc.delete(&entry, LANGUAGE)?;
        } else {
            doc.put(&entry, LANGUAGE, language)?;
        }
        Ok(Written::new(
            Document::Page(page.clone()),
            Changed::Block(page.clone(), block.clone()),
        ))
    }

    fn write_add_media_block(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
        name: &str,
        media_type: &str,
        attachment: &AttachmentId,
    ) -> Result<Written, Error> {
        let size = self
            .store
            .attachment_size(attachment)?
            .ok_or(Error::NotFound)?;
        let (doc, order) = self.page(page)?;
        let block = BlockId::new()?;
        let entry = model::insert(doc, BLOCKS, block.as_str(), &[(KIND, "media")])?;
        let file = Media {
            name: name.into(),
            media_type: media_type.into(),
            attachment: attachment.clone(),
            size,
        };
        model::put_file(doc, &entry, &file)?;
        let sort = order.key_after(after)?;
        let view = add_view(doc, order, sort, &block, BlockKind::Media)?;
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written.created = Some(Created::Block { block, view });
        Ok(written)
    }

    fn write_add_whiteboard_block(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let block = BlockId::new()?;
        drawing::create(doc, &block)?;
        let sort = order.key_after(after)?;
        let view = add_view(doc, order, sort, &block, BlockKind::Whiteboard)?;
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written.created = Some(Created::Block { block, view });
        Ok(written)
    }

    fn write_add_calculation_block(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let block = BlockId::new()?;
        calculation::create(doc, &block)?;
        let sort = order.key_after(after)?;
        let view = add_view(doc, order, sort, &block, BlockKind::Calculation)?;
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written.created = Some(Created::Block { block, view });
        Ok(written)
    }

    fn write_add_database_block(
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

    fn write_duplicate_table(
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

    fn write_create_table(
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

    fn write_add_table_view(
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

    fn place_table_view(
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
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written.created = Some(Created::Block {
            block: table.clone(),
            view: view.clone(),
        });
        Ok(written)
    }

    fn write_rename_view(
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
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        if let Some(table) = table
            && self.table_name(&table)?.as_deref() == Some("")
        {
            let named = self.write_rename_database(&table, title)?;
            written.touched.extend(named.touched);
            written.changed.extend(named.changed);
        }
        Ok(written)
    }

    fn write_rename_database(&mut self, table: &BlockId, name: &str) -> Result<Written, Error> {
        let workspace = self.doc(&Document::Workspace)?;
        let entry = model::live(workspace, TABLES, table.as_str()).ok_or(Error::NotFound)?;
        workspace.put(&entry, NAME, name)?;
        let mut written = Written::new(Document::Workspace, Changed::Tables);
        written.changed.push(Changed::Table(table.clone()));
        Ok(written)
    }

    fn write_delete_table(&mut self, table: &BlockId) -> Result<Written, Error> {
        let shown = self.table_views(table)?;
        let workspace = self.doc(&Document::Workspace)?;
        let entry = model::live(workspace, TABLES, table.as_str()).ok_or(Error::NotFound)?;
        model::mark_deleted(workspace, &entry)?;
        let mut written = Written::new(Document::Workspace, Changed::Tables);
        written.changed.push(Changed::Table(table.clone()));
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

    fn write_set_cell(
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
        self.table_doc(block)?;
        let (doc, index) = self.database_index(block)?;
        database::set_cell(doc, index, block, row, field, value)?;
        Ok(table_changed(block, None))
    }

    fn write_remove_view(&mut self, page: &PageId, view: &ViewId) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
        if let Some(table) = model::string(doc, &entry, TABLE) {
            model::mark_deleted(doc, &entry)?;
            order.remove(view);
            let mut written =
                Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
            let table = BlockId::stored(&table);
            if let Ok(doc) = self.table_doc(&table)
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
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        if !order.shows(&block, view) {
            if let Some(block_entry) = model::live(doc, BLOCKS, block.as_str()) {
                model::mark_deleted(doc, &block_entry)?;
            }
            written.changed.push(Changed::Block(page.clone(), block));
        }
        order.remove(view);
        Ok(written)
    }

    fn write_tile(
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
        Ok(Written::new(
            Document::Page(page.clone()),
            Changed::Views(page.clone()),
        ))
    }

    fn write_move_views(
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
        Ok(Written::new(
            Document::Page(page.clone()),
            Changed::Views(page.clone()),
        ))
    }

    fn write_add_tab(
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

    fn write_tab(
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

    fn write_tabs(
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
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written.created = created.map(Created::Tab);
        Ok(written)
    }

    fn rollback(&mut self) {
        for loaded in self.docs.values_mut() {
            loaded.doc.rollback();
        }
    }

    fn record(
        &mut self,
        mut action: Action,
        touched: &[Document],
        new: Vec<(Document, AutoCommit)>,
        index: Vec<IndexWrite>,
    ) -> Result<(), Error> {
        for (document, doc) in new {
            let loaded = Loaded {
                doc,
                chunks: 0,
                order: None,
                databases: HashMap::new(),
            };
            self.docs.insert(document, loaded);
        }
        let message = serde_json::to_string(&Header::of(&action)).expect("a serializable header");
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64);
        let mut writes = Vec::new();
        for document in touched {
            let Some(loaded) = self.docs.get_mut(document) else {
                continue;
            };
            let options = CommitOptions::default()
                .with_message(message.clone())
                .with_time(time);
            let Some(hash) = loaded.doc.commit_with(options) else {
                continue;
            };
            action.changes.push((document.clone(), hash));
            let document = document.clone();
            if loaded.chunks >= COMPACT_AFTER {
                let snapshot = loaded.doc.save();
                writes.push(Write::Replace { document, snapshot });
                loaded.chunks = 1;
            } else if let Some(mut change) = loaded.doc.get_change_by_hash(&hash) {
                let chunk = change.bytes().into_owned();
                writes.push(Write::Append { document, chunk });
                loaded.chunks += 1;
            }
        }
        let commit = Commit {
            action: &action,
            writes,
            index,
        };
        if let Err(error) = self.store.commit(&commit) {
            for document in touched {
                self.docs.remove(document);
            }
            return Err(error);
        }
        Ok(())
    }
}

fn add_text_block(
    doc: &mut AutoCommit,
    order: &mut Order,
    sort: String,
    text: &str,
    style: TextStyle,
) -> Result<(BlockId, ViewId), Error> {
    let style = model::style_name(style)?;
    let block = BlockId::new()?;
    let entry = model::insert(
        doc,
        BLOCKS,
        block.as_str(),
        &[(KIND, "text"), (STYLE, &style)],
    )?;
    let content = doc.put_object(&entry, TEXT, ObjType::Text)?;
    doc.splice_text(&content, 0, 0, text)?;
    let view = add_view(doc, order, sort, &block, BlockKind::Text)?;
    Ok((block, view))
}

fn page_entry(page: &PageId, title: &str) -> IndexWrite {
    IndexWrite {
        found: Found::Page,
        id: page.to_string(),
        entry: Some(IndexEntry {
            page: Some(page.clone()),
            text: title.to_string(),
        }),
    }
}

fn table_entry(table: &BlockId, name: Option<&str>) -> IndexWrite {
    IndexWrite {
        found: Found::Table,
        id: table.to_string(),
        entry: name.map(|name| IndexEntry {
            page: None,
            text: name.to_string(),
        }),
    }
}

fn block_changed(page: &PageId, block: &BlockId, created: Option<Created>) -> Written {
    let mut written = Written::new(
        Document::Page(page.clone()),
        Changed::Block(page.clone(), block.clone()),
    );
    written.created = created;
    written
}

fn table_changed(table: &BlockId, created: Option<Created>) -> Written {
    let mut written = Written::new(
        Document::Table(table.clone()),
        Changed::Table(table.clone()),
    );
    written.created = created;
    written
}

fn table_of(order: &Order, view: &ViewId) -> Result<BlockId, Error> {
    let shown = order.view(view).ok_or(Error::NotFound)?;
    if shown.kind != BlockKind::Database {
        return Err(Error::WrongKind);
    }
    Ok(shown.block.clone())
}

fn add_view(
    doc: &mut AutoCommit,
    order: &mut Order,
    sort: String,
    block: &BlockId,
    kind: BlockKind,
) -> Result<ViewId, Error> {
    let view = ViewId::new()?;
    model::insert(
        doc,
        VIEWS,
        view.as_str(),
        &[(BLOCK, block.as_str()), (SORT, &sort)],
    )?;
    doc.put(
        model::live(doc, VIEWS, view.as_str())
            .as_ref()
            .ok_or(Error::NotFound)?,
        BOUNDARY,
        false,
    )?;
    let placed = View {
        id: view.clone(),
        block: block.clone(),
        title: String::new(),
        boundary: false,
        kind,
        tabs: Vec::new(),
        tile: None,
        inner: None,
    };
    order.insert(sort, placed);
    Ok(view)
}

fn text_block(doc: &AutoCommit, block: &BlockId) -> Option<automerge::ObjId> {
    model::of_kind(doc, block.as_str(), BlockKind::Text)
}

fn text_of(doc: &AutoCommit, block: &BlockId) -> Result<automerge::ObjId, Error> {
    let entry = text_block(doc, block).ok_or(Error::NotFound)?;
    model::child(doc, &entry, TEXT).ok_or(Error::NotFound)
}

fn utf8_offset(text: &str, at: usize) -> usize {
    let mut units = 0;
    for (offset, c) in text.char_indices() {
        if units >= at {
            return offset;
        }
        units += c.len_utf16();
    }
    text.len()
}

fn move_boundary(
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
