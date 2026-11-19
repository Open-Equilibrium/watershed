use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use automerge::transaction::{CommitOptions, Transactable};
use automerge::{ActorId, AutoCommit, ObjType, ROOT, ReadDoc};

use crate::history::Header;
use crate::id::random_bytes;
use crate::model::{
    self, AREA, AREAS, BLOCK, BLOCKS, BOUNDARY, KIND, LANGUAGE, NAME, OWNER, PAGE, PAGES, PARENT,
    ROW_FILTER, ROW_SORT, SORT, STYLE, TABLE, TABLES, TEXT, TITLE, VIEWS,
};
use crate::order::Order;
use crate::store::{Commit, Store, Write};
use crate::{
    Action, ActionId, Actor, Applied, Area, AreaId, AttachmentId, BlockId, BlockKind, Cell,
    Changed, Code, Created, Document, Error, FieldId, Media, Mutation, Operation, Origin,
    PageEntry, PageId, Permission, PersonId, Place, Request, RowId, TableEntry, TextStyle, View,
    ViewId,
};
use crate::{database, drawing};

const COMPACT_AFTER: usize = 1_000;

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
        )?;
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
        Ok(this)
    }

    pub fn apply(&mut self, request: Request) -> Result<Applied, Error> {
        let permission = self.permit(&request.actor)?;
        let id = ActionId::new()?;
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
        let action = Action {
            id: id.clone(),
            actor: request.actor,
            origin: request.origin,
            permission,
            operation: Operation::Mutation(request.mutation),
            changes: Vec::new(),
        };
        self.record(action, &written.touched, written.new)?;
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
        database::rows(doc, index, table, query)
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

    fn database_index(
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

    fn doc(&mut self, document: &Document) -> Result<&mut AutoCommit, Error> {
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
        match mutation {
            Mutation::CreateArea { name } => self.write_create_area(name),
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
            Mutation::AddDatabaseBlock { page, after, name } => {
                self.write_add_database_block(page, after.as_ref(), name)
            }
            Mutation::CreateTable { area, name } => {
                let (table, mut written) = self.write_create_table(area, name)?;
                written.created = Some(Created::Table(table));
                Ok(written)
            }
            Mutation::AddTableView {
                page,
                after,
                table,
                title,
                sort,
                filter,
            } => self.write_add_table_view(
                page,
                after.as_ref(),
                table,
                title,
                (sort.as_ref(), filter.as_ref()),
            ),
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
            Mutation::SetViewSort { page, view, sort } => {
                self.write_view_sort(page, view, sort.as_ref())
            }
            Mutation::SetViewFilter { page, view, filter } => {
                self.write_view_filter(page, view, filter.as_ref())
            }
        }
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

    fn write_add_database_block(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
        name: &str,
    ) -> Result<Written, Error> {
        let area = self.area_of(page)?;
        let (table, mut written) = self.write_create_table(&area, name)?;
        let view = ViewId::new()?;
        let (_, doc) = written.new.last_mut().expect("the new Table");
        model::insert(doc, VIEWS, view.as_str(), &[(PAGE, page.as_str())])?;
        let shown = self.place_table_view(page, after, &table, &view, name, (None, None))?;
        written.touched.extend(shown.touched);
        written.changed.extend(shown.changed);
        written.created = shown.created;
        Ok(written)
    }

    fn write_create_table(
        &mut self,
        area: &AreaId,
        name: &str,
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
        database::create(&mut doc, &table)?;
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
        (sort, filter): (Option<&database::Sort>, Option<&database::Filter>),
    ) -> Result<Written, Error> {
        let view = ViewId::new()?;
        {
            let doc = self.table_doc(table)?;
            if let Some(sort) = sort {
                database::check_sort(doc, table, sort)?;
            }
            if let Some(filter) = filter {
                database::check_filter(doc, table, filter)?;
            }
            model::insert(doc, VIEWS, view.as_str(), &[(PAGE, page.as_str())])?;
        }
        let mut written =
            self.place_table_view(page, after, table, &view, title, (sort, filter))?;
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
        (sort, filter): (Option<&database::Sort>, Option<&database::Filter>),
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
        put_setting(doc, view, ROW_SORT, sort)?;
        put_setting(doc, view, ROW_FILTER, filter)?;
        order.insert(
            key,
            View {
                id: view.clone(),
                block: table.clone(),
                title: title.to_string(),
                boundary: false,
                kind: BlockKind::Database,
                sort: sort.cloned(),
                filter: filter.cloned(),
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

    fn write_view_sort(
        &mut self,
        page: &PageId,
        view: &ViewId,
        sort: Option<&database::Sort>,
    ) -> Result<Written, Error> {
        let block = table_of(self.page(page)?.1, view)?;
        if let Some(sort) = sort {
            database::check_sort(self.table_doc(&block)?, &block, sort)?;
        }
        let (doc, order) = self.page(page)?;
        put_setting(doc, view, ROW_SORT, sort)?;
        order.set_sort(view, sort.cloned());
        Ok(Written::new(
            Document::Page(page.clone()),
            Changed::Views(page.clone()),
        ))
    }

    fn write_view_filter(
        &mut self,
        page: &PageId,
        view: &ViewId,
        filter: Option<&database::Filter>,
    ) -> Result<Written, Error> {
        let block = table_of(self.page(page)?.1, view)?;
        if let Some(filter) = filter {
            database::check_filter(self.table_doc(&block)?, &block, filter)?;
        }
        let (doc, order) = self.page(page)?;
        put_setting(doc, view, ROW_FILTER, filter)?;
        order.set_filter(view, filter.cloned());
        Ok(Written::new(
            Document::Page(page.clone()),
            Changed::Views(page.clone()),
        ))
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

fn put_setting<T: serde::Serialize>(
    doc: &mut AutoCommit,
    view: &ViewId,
    key: &str,
    value: Option<&T>,
) -> Result<(), Error> {
    let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
    match value {
        Some(value) => {
            let json = serde_json::to_string(value).map_err(|_| Error::WrongKind)?;
            doc.put(&entry, key, json)?;
        }
        None if doc.get(&entry, key)?.is_some() => doc.delete(&entry, key)?,
        None => {}
    }
    Ok(())
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
        sort: None,
        filter: None,
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
