use std::collections::HashMap;

use automerge::transaction::Transactable;
use automerge::{ActorId, AutoCommit, ROOT};

use crate::history::Header;
use crate::id::random_bytes;
use crate::model::{self, AREAS, NAME, OWNER, PAGES, TABLES};
use crate::order::Order;
use crate::store::{Store, Write};
use crate::{
    Action, ActionId, Actor, AreaId, BlockId, Changed, Created, Document, Error, INDEX_VERSION,
    Operation, Origin, PageId, Permission, PersonId, database,
};

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
    pub(crate) chunks: usize,
    pub(crate) order: Option<Order>,
    pub(crate) databases: HashMap<BlockId, database::Index>,
}

impl Loaded {
    pub(crate) fn new(doc: AutoCommit, chunks: usize) -> Self {
        Self {
            doc,
            chunks,
            order: None,
            databases: HashMap::new(),
        }
    }
}

pub(crate) struct Written {
    pub(crate) created: Option<Created>,
    pub(crate) changed: Vec<Changed>,
    pub(crate) touched: Vec<Document>,
    pub(crate) new: Vec<(Document, AutoCommit)>,
}

impl Written {
    pub(crate) fn new(touched: Document, changed: Changed) -> Self {
        Self {
            created: None,
            changed: vec![changed],
            touched: vec![touched],
            new: Vec::new(),
        }
    }

    pub(crate) fn views(page: &PageId) -> Self {
        Self::new(Document::Page(page.clone()), Changed::Views(page.clone()))
    }

    pub(crate) fn block(page: &PageId, block: &BlockId) -> Self {
        Self::new(
            Document::Page(page.clone()),
            Changed::Block(page.clone(), block.clone()),
        )
    }
}

#[derive(Default)]
pub(crate) struct Recorded {
    pub(crate) merged: Vec<(Document, automerge::ChangeHash, Vec<u8>)>,
    pub(crate) writes: Vec<Write>,
    pub(crate) redact: Vec<Action>,
    pub(crate) dropped: Vec<Document>,
}

pub(crate) fn header(action: &Action) -> String {
    serde_json::to_string(&Header::of(action)).expect("a serializable header")
}

impl<S: Store> Workspace<S> {
    pub fn create(owner: PersonId, mut store: S) -> Result<Self, Error> {
        if !store.chunks(&Document::Workspace)?.is_empty() {
            return Err(Error::Storage("the store already holds a Workspace".into()));
        }
        let device = ActorId::from(&random_bytes()?[..]);
        let mut doc = model::document(&device);
        doc.put(ROOT, OWNER, owner.as_str())?;
        model::create_maps(&mut doc, &[AREAS, TABLES])?;
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
            heads: Vec::new(),
            at: 0,
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

    pub fn owner(&self) -> &PersonId {
        &self.owner
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

    pub(crate) fn table_name(&mut self, table: &BlockId) -> Result<Option<String>, Error> {
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

    pub(crate) fn page(&mut self, page: &PageId) -> Result<(&mut AutoCommit, &mut Order), Error> {
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
            self.docs
                .insert(document.clone(), Loaded::new(doc, chunks.len()));
        }
        Ok(&mut self.docs.get_mut(document).expect("loaded above").doc)
    }

    pub(crate) fn permit(&self, actor: &Actor) -> Result<Permission, Error> {
        match actor {
            Actor::Person(person) if *person == self.owner => Ok(Permission::Owner),
            _ => Err(Error::NotAllowed),
        }
    }

    pub(crate) fn area_of(&mut self, page: &PageId) -> Result<AreaId, Error> {
        for area in model::areas(self.doc(&Document::Workspace)?) {
            let area_doc = self.doc(&Document::Area(area.id.clone()))?;
            if model::live(area_doc, PAGES, page.as_str()).is_some() {
                return Ok(area.id);
            }
        }
        Err(Error::NotFound)
    }

    pub(crate) fn table_doc(&mut self, table: &BlockId) -> Result<&mut AutoCommit, Error> {
        if self.table_name(table)?.is_none() {
            return Err(Error::NotFound);
        }
        self.doc(&Document::Table(table.clone()))
    }

    pub(crate) fn table_index(
        &mut self,
        block: &BlockId,
    ) -> Result<(&mut AutoCommit, &mut database::Index), Error> {
        self.table_doc(block)?;
        self.database_index(block)
    }

    pub(crate) fn page_doc(&mut self, page: &PageId) -> Result<&mut AutoCommit, Error> {
        self.doc(&Document::Page(page.clone()))
    }
}
