//! The Workspace and its mutation pipeline: permission check, validation, atomic apply,
//! recorded Action, changed resources. No write reaches a document any other way.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use automerge::transaction::{CommitOptions, Transactable};
use automerge::{ActorId, AutoCommit, ObjType, ROOT, ReadDoc};

use crate::database;
use crate::history::Header;
use crate::id::random_bytes;
use crate::model::{
    self, AREAS, BLOCK, BLOCKS, BOUNDARY, KIND, NAME, OWNER, PAGES, PARENT, SORT, STYLE, TEXT,
    TITLE, VIEWS,
};
use crate::order::Order;
use crate::store::{Commit, Store, Write};
use crate::{
    Action, ActionId, Actor, Applied, Area, AreaId, BlockId, Changed, Created, Document, Error,
    Mutation, Operation, Origin, PageEntry, PageId, Permission, PersonId, Request, TextStyle, View,
    ViewId,
};

/// A document is stored again as one snapshot once it has this many chunks, so loading it
/// never replays more changes than that.
const COMPACT_AFTER: usize = 1_000;

/// One person's Workspace on this device. Documents load from the store when first used,
/// and every write is durable in the store before it is acknowledged.
pub struct Workspace<S: Store> {
    pub(crate) store: S,
    device: ActorId,
    /// The Workspace document's owner, which never changes after its first change.
    owner: PersonId,
    pub(crate) docs: HashMap<Document, Loaded>,
}

pub(crate) struct Loaded {
    pub doc: AutoCommit,
    /// How many chunks the store holds for the document.
    chunks: usize,
    /// A Page document's View order, read once it is first needed.
    order: Option<Order>,
    /// Its Database Blocks' rows, each read once it is first needed.
    databases: HashMap<BlockId, database::Index>,
}

/// What a mutation wrote before its changes are committed.
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
    /// Creates an empty Workspace owned by `owner` in an empty `store`, recorded as its
    /// first Action.
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

    /// Opens the Workspace that `store` holds. This device writes under a new Automerge
    /// actor each time.
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

    /// Applies one write through the pipeline and acknowledges it once it is durably
    /// recorded as one Action, or refuses it and changes nothing.
    pub fn apply(&mut self, request: Request) -> Result<Applied, Error> {
        // Until Roles exist the permission depends only on the actor, so it is checked
        // before anything is read and a refused actor learns nothing about the Workspace.
        let permission = self.permit(&request.actor)?;
        let id = ActionId::new()?;
        let written = match self.write(&request.mutation) {
            Ok(written) => written,
            Err(error) => {
                self.rollback();
                return Err(error);
            }
        };
        // Automerge drops a write of the value already stored, so a mutation can leave
        // nothing to commit; it records no Action.
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

    /// The person who created the Workspace and holds Can manage.
    pub fn owner(&self) -> &PersonId {
        &self.owner
    }

    /// The areas, in Explorer order.
    pub fn areas(&mut self) -> Result<Vec<Area>, Error> {
        Ok(model::areas(self.doc(&Document::Workspace)?))
    }

    /// An area's Pages, each followed by its descendants, siblings in Explorer order.
    pub fn pages(&mut self, area: &AreaId) -> Result<Vec<PageEntry>, Error> {
        Ok(model::pages(self.doc(&Document::Area(area.clone()))?))
    }

    /// Up to `limit` of a Page's Views in order, from position `start`, so a reader takes
    /// only the window it shows (client architecture rule 3).
    pub fn views(&mut self, page: &PageId, start: usize, limit: usize) -> Result<Vec<View>, Error> {
        Ok(self.page(page)?.1.window(start, limit))
    }

    /// How many Views a Page shows.
    pub fn view_count(&mut self, page: &PageId) -> Result<usize, Error> {
        Ok(self.page(page)?.1.len())
    }

    /// How a live Text Block reads, from the document of its home `page`.
    pub fn style(&mut self, page: &PageId, block: &BlockId) -> Result<Option<TextStyle>, Error> {
        let doc = self.doc(&Document::Page(page.clone()))?;
        Ok(text_block(doc, block)
            .map(|entry| model::style(model::string(doc, &entry, STYLE).as_deref())))
    }

    /// A live Text Block's text, from the document of its home `page`.
    pub fn text(&mut self, page: &PageId, block: &BlockId) -> Result<Option<String>, Error> {
        Ok(model::text(
            self.doc(&Document::Page(page.clone()))?,
            block.as_str(),
        ))
    }

    /// A live Database Block's name and fields, from the document of its home `page`; none
    /// if `block` is no live Database Block.
    pub fn database(
        &mut self,
        page: &PageId,
        block: &BlockId,
    ) -> Result<Option<database::Database>, Error> {
        database::read(self.doc(&Document::Page(page.clone()))?, block)
    }

    /// A window of a Database Block's rows, filtered and sorted as `query` asks.
    pub fn rows(
        &mut self,
        page: &PageId,
        block: &BlockId,
        query: &database::RowQuery,
    ) -> Result<database::Rows, Error> {
        let (doc, index) = self.database_index(page, block)?;
        database::rows(doc, index, block, query)
    }

    /// Whether a Page's document is in memory.
    pub fn is_loaded(&self, page: &PageId) -> bool {
        self.docs.contains_key(&Document::Page(page.clone()))
    }

    /// Releases a Page's document, View order and Database row indexes from memory; the next read loads them
    /// again from the store. A Page holds no uncommitted writes between requests.
    pub fn close_page(&mut self, page: &PageId) {
        self.docs.remove(&Document::Page(page.clone()));
    }

    /// Up to `limit` recorded Actions, newest first, starting before `before`.
    pub fn history(
        &mut self,
        before: Option<&ActionId>,
        limit: usize,
    ) -> Result<Vec<Action>, Error> {
        self.store.history(before, limit)
    }

    /// A Page document with its View order.
    fn page(&mut self, page: &PageId) -> Result<(&mut AutoCommit, &mut Order), Error> {
        let document = Document::Page(page.clone());
        self.doc(&document)?;
        let loaded = self.docs.get_mut(&document).expect("loaded above");
        let order = loaded.order.get_or_insert_with(|| Order::read(&loaded.doc));
        Ok((&mut loaded.doc, order))
    }

    /// A Page document with the row index of one of its Database Blocks.
    fn database_index(
        &mut self,
        page: &PageId,
        block: &BlockId,
    ) -> Result<(&mut AutoCommit, &mut database::Index), Error> {
        let document = Document::Page(page.clone());
        self.doc(&document)?;
        let loaded = self.docs.get_mut(&document).expect("loaded above");
        if !loaded.databases.contains_key(block) {
            let index = database::Index::read(&loaded.doc, block)?;
            loaded.databases.insert(block.clone(), index);
        }
        let index = loaded.databases.get_mut(block).expect("read above");
        Ok((&mut loaded.doc, index))
    }

    /// A document, loaded from the store on first use.
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

    fn permit(&self, actor: &Actor) -> Result<Permission, Error> {
        let Actor::Person(person) = actor;
        if *person == self.owner {
            Ok(Permission::Owner)
        } else {
            Err(Error::NotAllowed)
        }
    }

    fn write(&mut self, mutation: &Mutation) -> Result<Written, Error> {
        match mutation {
            Mutation::CreateArea { name } => {
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
            Mutation::CreatePage {
                area,
                parent,
                title,
            } => {
                let area_doc = self.doc(&Document::Area(area.clone()))?;
                if let Some(parent) = parent {
                    model::live(area_doc, PAGES, parent.as_str()).ok_or(Error::NotFound)?;
                }
                let siblings: Vec<_> = model::sorted(area_doc, PAGES)
                    .into_iter()
                    .filter(|entry| entry.get(PARENT) == parent.as_ref().map(PageId::as_str))
                    .collect();
                let last = siblings.last().map(|entry| entry.id.as_str());
                let sort = model::sort_after(&siblings, last)?;
                let id = PageId::new()?;
                let mut fields = vec![(TITLE, title.as_str()), (SORT, &sort)];
                if let Some(parent) = parent {
                    fields.push((PARENT, parent.as_str()));
                }
                model::insert(area_doc, PAGES, id.as_str(), &fields)?;
                let mut doc = model::document(&self.device);
                model::create_maps(&mut doc, &[BLOCKS, VIEWS])?;
                let mut written =
                    Written::new(Document::Area(area.clone()), Changed::Pages(area.clone()));
                written.touched.push(Document::Page(id.clone()));
                written.created = Some(Created::Page(id.clone()));
                written.new.push((Document::Page(id), doc));
                Ok(written)
            }
            Mutation::RenamePage { page, title } => {
                for area in model::areas(self.doc(&Document::Workspace)?) {
                    let area_doc = self.doc(&Document::Area(area.id.clone()))?;
                    if let Some(entry) = model::live(area_doc, PAGES, page.as_str()) {
                        area_doc.put(&entry, TITLE, title.as_str())?;
                        return Ok(Written::new(
                            Document::Area(area.id.clone()),
                            Changed::Pages(area.id),
                        ));
                    }
                }
                Err(Error::NotFound)
            }
            Mutation::AddTextBlock { page, after, text } => {
                let (doc, order) = self.page(page)?;
                let (block, view) =
                    add_text_block(doc, order, after.as_ref(), text, TextStyle::Paragraph)?;
                let mut written =
                    Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
                written.created = Some(Created::Block { block, view });
                Ok(written)
            }
            Mutation::SplitTextBlock {
                page,
                block,
                view,
                at,
                style,
            } => {
                let (doc, order) = self.page(page)?;
                if order.view(view).is_none_or(|shown| &shown.block != block) {
                    return Err(Error::NotFound);
                }
                let content = text_of(doc, block)?;
                if !model::on_boundary(doc, &content, *at) {
                    return Err(Error::OutOfRange);
                }
                let text = doc.text(&content)?;
                let tail = text[utf8_offset(&text, *at)..].to_string();
                let (new_block, new_view) = if *at == 0 {
                    let above = order.above(view).map(|above| above.id.clone());
                    let starts_section = order.view(view).is_some_and(|shown| shown.boundary);
                    let added = add_text_block(doc, order, above.as_ref(), "", *style)?;
                    // A Section that started here now starts at the new Text Block above.
                    if starts_section {
                        move_boundary(doc, order, view, &added.1)?;
                    }
                    added
                } else {
                    let length = isize::try_from(doc.length(&content) - at)
                        .map_err(|_| Error::OutOfRange)?;
                    doc.splice_text(&content, *at, length, "")?;
                    add_text_block(doc, order, Some(view), &tail, *style)?
                };
                let mut written =
                    Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
                written
                    .changed
                    .push(Changed::Block(page.clone(), block.clone()));
                written.created = Some(Created::Block {
                    block: new_block,
                    view: new_view,
                });
                Ok(written)
            }
            Mutation::JoinTextBlock { page, view } => {
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
                let mut written =
                    Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
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
            Mutation::SetTextStyle { page, block, style } => {
                let name = model::style_name(*style)?;
                let (doc, _) = self.page(page)?;
                let entry = text_block(doc, block).ok_or(Error::NotFound)?;
                doc.put(&entry, STYLE, name)?;
                Ok(Written::new(
                    Document::Page(page.clone()),
                    Changed::Block(page.clone(), block.clone()),
                ))
            }
            Mutation::SetSectionBoundary {
                page,
                view,
                boundary,
            } => {
                let (doc, order) = self.page(page)?;
                let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
                doc.put(&entry, BOUNDARY, *boundary)?;
                order.set_boundary(view, *boundary);
                Ok(Written::new(
                    Document::Page(page.clone()),
                    Changed::Views(page.clone()),
                ))
            }
            Mutation::EditText {
                page,
                block,
                at,
                delete,
                insert,
            } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                let entry = model::live(doc, BLOCKS, block.as_str()).ok_or(Error::NotFound)?;
                let content = model::child(doc, &entry, TEXT).ok_or(Error::NotFound)?;
                let end = at.checked_add(*delete).ok_or(Error::OutOfRange)?;
                if !model::on_boundary(doc, &content, *at)
                    || !model::on_boundary(doc, &content, end)
                {
                    return Err(Error::OutOfRange);
                }
                let delete = isize::try_from(*delete).map_err(|_| Error::OutOfRange)?;
                doc.splice_text(&content, *at, delete, insert)?;
                Ok(Written::new(
                    Document::Page(page.clone()),
                    Changed::Block(page.clone(), block.clone()),
                ))
            }
            Mutation::AddDatabaseBlock { page, after, name } => {
                let (doc, order) = self.page(page)?;
                let block = BlockId::new()?;
                database::create(doc, &block, name)?;
                let view = add_view(doc, order, after.as_ref(), &block)?;
                let mut written =
                    Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
                written.created = Some(Created::Block { block, view });
                Ok(written)
            }
            Mutation::RenameDatabase { page, block, name } => {
                database::rename(self.doc(&Document::Page(page.clone()))?, block, name)?;
                Ok(database_changed(page, block, None))
            }
            Mutation::AddField {
                page,
                block,
                after,
                name,
                kind,
            } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                let field = database::add_field(doc, block, after.as_ref(), name, *kind)?;
                Ok(database_changed(page, block, Some(Created::Field(field))))
            }
            Mutation::RenameField {
                page,
                block,
                field,
                name,
            } => {
                database::rename_field(
                    self.doc(&Document::Page(page.clone()))?,
                    block,
                    field,
                    name,
                )?;
                Ok(database_changed(page, block, None))
            }
            Mutation::RemoveField { page, block, field } => {
                database::remove_field(self.doc(&Document::Page(page.clone()))?, block, field)?;
                Ok(database_changed(page, block, None))
            }
            Mutation::AddOption {
                page,
                block,
                field,
                name,
                color,
            } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                let option = database::add_option(doc, block, field, name, *color)?;
                Ok(database_changed(page, block, Some(Created::Option(option))))
            }
            Mutation::AddRow { page, block, after } => {
                let (doc, index) = self.database_index(page, block)?;
                let row = database::add_row(doc, index, block, after.as_ref())?;
                Ok(database_changed(page, block, Some(Created::Row(row))))
            }
            Mutation::RemoveRow { page, block, row } => {
                let (doc, index) = self.database_index(page, block)?;
                database::remove_row(doc, index, block, row)?;
                Ok(database_changed(page, block, None))
            }
            Mutation::SetCell {
                page,
                block,
                row,
                field,
                value,
            } => {
                let (doc, index) = self.database_index(page, block)?;
                database::set_cell(doc, index, block, row, field, value)?;
                Ok(database_changed(page, block, None))
            }
            Mutation::RemoveView { page, view } => {
                let (doc, order) = self.page(page)?;
                let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
                let block =
                    BlockId::stored(&model::string(doc, &entry, BLOCK).ok_or(Error::NotFound)?);
                model::mark_deleted(doc, &entry)?;
                let mut written =
                    Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
                if !order.shows(&block, view) {
                    if let Some(block_entry) = model::live(doc, BLOCKS, block.as_str()) {
                        model::mark_deleted(doc, &block_entry)?;
                    }
                    written.changed.push(Changed::Block(page.clone(), block));
                }
                order.remove(view);
                Ok(written)
            }
        }
    }

    /// Discards every uncommitted write, so a refused mutation leaves no trace.
    fn rollback(&mut self) {
        for loaded in self.docs.values_mut() {
            loaded.doc.rollback();
        }
    }

    /// Commits each touched document as one change that carries the Action's header and
    /// writes them with the Action to the store. If the store fails, the touched documents
    /// are dropped from memory, so they load again as the store holds them.
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

/// Adds a Text Block of `style` holding `text`, with its View after `after` or first.
fn add_text_block(
    doc: &mut AutoCommit,
    order: &mut Order,
    after: Option<&ViewId>,
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
    let view = add_view(doc, order, after, &block)?;
    Ok((block, view))
}

/// What a write to a Database Block changed: only that Block.
fn database_changed(page: &PageId, block: &BlockId, created: Option<Created>) -> Written {
    let mut written = Written::new(
        Document::Page(page.clone()),
        Changed::Block(page.clone(), block.clone()),
    );
    written.created = created;
    written
}

/// Adds a View of `block` after `after` or first.
fn add_view(
    doc: &mut AutoCommit,
    order: &mut Order,
    after: Option<&ViewId>,
    block: &BlockId,
) -> Result<ViewId, Error> {
    let sort = order.key_after(after)?;
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
        boundary: false,
    };
    order.insert(sort, placed);
    Ok(view)
}

/// A live Text Block's entry; none for a Block of another type.
fn text_block(doc: &AutoCommit, block: &BlockId) -> Option<automerge::ObjId> {
    let entry = model::live(doc, BLOCKS, block.as_str())?;
    (model::string(doc, &entry, KIND).as_deref() == Some("text")).then_some(entry)
}

/// A live Text Block's text object.
fn text_of(doc: &AutoCommit, block: &BlockId) -> Result<automerge::ObjId, Error> {
    let entry = model::live(doc, BLOCKS, block.as_str()).ok_or(Error::NotFound)?;
    model::child(doc, &entry, TEXT).ok_or(Error::NotFound)
}

/// The UTF-8 byte offset of the UTF-16 offset `at`, which lies between characters.
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

/// Moves a Section boundary from the View `from` to the View `to`.
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
