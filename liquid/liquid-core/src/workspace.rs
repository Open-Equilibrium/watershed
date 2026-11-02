//! The Workspace and its mutation pipeline: permission check, validation, atomic apply,
//! recorded Action, changed resources. No write reaches a document any other way.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use automerge::transaction::{CommitOptions, Transactable};
use automerge::{ActorId, AutoCommit, ObjType, ROOT};

use crate::history::Header;
use crate::id::random_bytes;
use crate::model::{
    self, AREAS, BLOCK, BLOCKS, KIND, NAME, OWNER, PAGES, PARENT, SORT, TEXT, TITLE, VIEWS,
};
use crate::{
    Action, ActionId, Actor, Applied, Area, AreaId, BlockId, Changed, Created, Document, Error,
    Mutation, Operation, Origin, PageEntry, PageId, Permission, PersonId, Request, View, ViewId,
};

/// One person's Workspace on this device, held in memory.
pub struct Workspace {
    device: ActorId,
    /// The Workspace document's owner, which never changes after its first change.
    owner: PersonId,
    pub(crate) workspace: AutoCommit,
    pub(crate) areas: HashMap<AreaId, AutoCommit>,
    pub(crate) pages: HashMap<PageId, AutoCommit>,
    history: Vec<Action>,
}

/// What a mutation wrote before its changes are committed.
struct Written {
    created: Option<Created>,
    changed: Vec<Changed>,
    touched: Vec<Document>,
    new_area: Option<(AreaId, AutoCommit)>,
    new_page: Option<(PageId, AutoCommit)>,
}

impl Written {
    fn new(touched: Document, changed: Changed) -> Self {
        Self {
            created: None,
            changed: vec![changed],
            touched: vec![touched],
            new_area: None,
            new_page: None,
        }
    }
}

impl Workspace {
    /// Creates an empty Workspace owned by `owner`, recorded as its first Action.
    pub fn create(owner: PersonId) -> Result<Self, Error> {
        let device = ActorId::from(&random_bytes()?[..]);
        let mut workspace = model::document(&device);
        workspace.put(ROOT, OWNER, owner.as_str())?;
        model::create_maps(&mut workspace, &[AREAS])?;
        let mut this = Self {
            device,
            owner: owner.clone(),
            workspace,
            areas: HashMap::new(),
            pages: HashMap::new(),
            history: Vec::new(),
        };
        let action = Action {
            id: ActionId::new()?,
            actor: Actor::Person(owner),
            origin: Origin::Ui,
            permission: Permission::Owner,
            operation: Operation::CreateWorkspace,
            changes: Vec::new(),
        };
        this.record(action, &[Document::Workspace]);
        Ok(this)
    }

    /// Applies one write through the pipeline and records it as one Action, or refuses it
    /// and changes nothing.
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
        let unchanged = written.new_area.is_none()
            && written.new_page.is_none()
            && written.touched.iter().all(|document| {
                self.document(document)
                    .is_none_or(|doc| doc.pending_ops() == 0)
            });
        if unchanged {
            return Err(Error::NoChange);
        }
        if let Some((area, doc)) = written.new_area {
            self.areas.insert(area, doc);
        }
        if let Some((page, doc)) = written.new_page {
            self.pages.insert(page, doc);
        }
        let action = Action {
            id: id.clone(),
            actor: request.actor,
            origin: request.origin,
            permission,
            operation: Operation::Mutation(request.mutation),
            changes: Vec::new(),
        };
        self.record(action, &written.touched);
        Ok(Applied {
            action: id,
            created: written.created,
            changed: written.changed,
        })
    }

    /// The areas, in Explorer order.
    pub fn areas(&self) -> Vec<Area> {
        model::areas(&self.workspace)
    }

    /// An area's Pages, each followed by its descendants, siblings in Explorer order.
    pub fn pages(&self, area: &AreaId) -> Vec<PageEntry> {
        self.areas.get(area).map(model::pages).unwrap_or_default()
    }

    /// A Page's Views in order.
    pub fn views(&self, page: &PageId) -> Vec<View> {
        self.pages.get(page).map(model::views).unwrap_or_default()
    }

    /// A live Text Block's text, from the document of its home `page`.
    pub fn text(&self, page: &PageId, block: &BlockId) -> Option<String> {
        model::text(self.pages.get(page)?, block.as_str())
    }

    /// Every recorded Action, oldest first.
    pub fn history(&self) -> &[Action] {
        &self.history
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
                let siblings = model::sorted(&self.workspace, AREAS);
                let last = siblings.last().map(|entry| entry.id.as_str());
                let sort = model::sort_after(&siblings, last)?;
                model::insert(
                    &mut self.workspace,
                    AREAS,
                    id.as_str(),
                    &[(NAME, name), (SORT, &sort)],
                )?;
                let mut written = Written::new(Document::Workspace, Changed::Areas);
                written.touched.push(Document::Area(id.clone()));
                written.created = Some(Created::Area(id.clone()));
                written.new_area = Some((id, doc));
                Ok(written)
            }
            Mutation::CreatePage {
                area,
                parent,
                title,
            } => {
                let area_doc = self.areas.get_mut(area).ok_or(Error::NotFound)?;
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
                written.new_page = Some((id, doc));
                Ok(written)
            }
            Mutation::RenamePage { page, title } => {
                let (area, area_doc) = self
                    .areas
                    .iter_mut()
                    .find(|(_, doc)| model::live(doc, PAGES, page.as_str()).is_some())
                    .ok_or(Error::NotFound)?;
                let entry = model::live(area_doc, PAGES, page.as_str()).ok_or(Error::NotFound)?;
                area_doc.put(&entry, TITLE, title.as_str())?;
                Ok(Written::new(
                    Document::Area(area.clone()),
                    Changed::Pages(area.clone()),
                ))
            }
            Mutation::AddTextBlock { page, after, text } => {
                let doc = self.pages.get_mut(page).ok_or(Error::NotFound)?;
                let siblings = model::sorted(doc, VIEWS);
                let sort = model::sort_after(&siblings, after.as_ref().map(ViewId::as_str))?;
                let (block, view) = (BlockId::new()?, ViewId::new()?);
                let entry = model::insert(doc, BLOCKS, block.as_str(), &[(KIND, "text")])?;
                let content = doc.put_object(&entry, TEXT, ObjType::Text)?;
                doc.splice_text(&content, 0, 0, text)?;
                model::insert(
                    doc,
                    VIEWS,
                    view.as_str(),
                    &[(BLOCK, block.as_str()), (SORT, &sort)],
                )?;
                let mut written =
                    Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
                written.created = Some(Created::Block { block, view });
                Ok(written)
            }
            Mutation::EditText {
                page,
                block,
                at,
                delete,
                insert,
            } => {
                let doc = self.pages.get_mut(page).ok_or(Error::NotFound)?;
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
            Mutation::RemoveView { page, view } => {
                let doc = self.pages.get_mut(page).ok_or(Error::NotFound)?;
                let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
                let block = model::string(doc, &entry, BLOCK).ok_or(Error::NotFound)?;
                model::mark_deleted(doc, &entry)?;
                let mut written =
                    Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
                let shown = model::sorted(doc, VIEWS)
                    .iter()
                    .any(|other| other.get(BLOCK) == Some(block.as_str()));
                if !shown {
                    if let Some(block_entry) = model::live(doc, BLOCKS, &block) {
                        model::mark_deleted(doc, &block_entry)?;
                    }
                    written
                        .changed
                        .push(Changed::Block(page.clone(), BlockId::stored(&block)));
                }
                Ok(written)
            }
        }
    }

    fn document(&mut self, document: &Document) -> Option<&mut AutoCommit> {
        match document {
            Document::Workspace => Some(&mut self.workspace),
            Document::Area(area) => self.areas.get_mut(area),
            Document::Page(page) => self.pages.get_mut(page),
        }
    }

    /// Discards every uncommitted write, so a refused mutation leaves no trace.
    fn rollback(&mut self) {
        self.workspace.rollback();
        self.areas.values_mut().for_each(|doc| {
            doc.rollback();
        });
        self.pages.values_mut().for_each(|doc| {
            doc.rollback();
        });
    }

    /// Commits each touched document as one change that carries the Action's header, then
    /// appends the Action to History.
    fn record(&mut self, mut action: Action, touched: &[Document]) {
        let message = serde_json::to_string(&Header::of(&action)).expect("a serializable header");
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64);
        for document in touched {
            let doc = self.document(document);
            let options = CommitOptions::default()
                .with_message(message.clone())
                .with_time(time);
            if let Some(hash) = doc.and_then(|doc| doc.commit_with(options)) {
                action.changes.push((document.clone(), hash));
            }
        }
        self.history.push(action);
    }
}
