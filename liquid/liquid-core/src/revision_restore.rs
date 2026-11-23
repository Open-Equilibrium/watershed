use std::collections::{HashMap, HashSet};

use automerge::{AutoCommit, ChangeHash, ObjId, ROOT, ReadDoc, Value};

use crate::model::{self};
use crate::order::Order;
use crate::revision::{HISTORY_BATCH, SAVE_POINT_GAP_MS, Version};
use crate::store::Store;
use crate::workspace::Workspace;
use crate::{
    Action, ActionId, Actor, Applied, BlockKind, Document, Error, MemoryStore, Operation, Origin,
    PageId, Permission, Purge, patch,
};

fn deleted_views(doc: &AutoCommit, at: &[ChangeHash], tables: &HashSet<String>) -> HashSet<ObjId> {
    let Ok(Some((Value::Object(_), views))) = doc.get_at(&ROOT, model::VIEWS, at) else {
        return HashSet::new();
    };
    doc.map_range_at(&views, .., at)
        .filter(|item| {
            matches!(
                doc.get_at(item.id(), model::TABLE, at),
                Ok(Some((Value::Scalar(table), _))) if table.as_str().is_some_and(|table| tables.contains(table))
            )
        })
        .map(|item| item.id())
        .collect()
}

impl<S: Store> Workspace<S> {
    fn deleted_tables(&mut self) -> Result<HashSet<String>, Error> {
        let workspace = self.doc(&Document::Workspace)?;
        let Some(map) = model::child(workspace, &ROOT, model::TABLES) else {
            return Ok(HashSet::new());
        };
        Ok(workspace
            .map_range(&map, ..)
            .map(|item| item.key.to_string())
            .filter(|key| model::live_in(workspace, &map, key).is_none())
            .collect())
    }

    fn page_documents_at(&mut self, page: &PageId, to: &ActionId) -> Result<Vec<Document>, Error> {
        let mut documents = self.page_documents(page)?;
        let mut reaching = documents.clone();
        for action in self.actions(std::slice::from_ref(to))? {
            for (document, _) in action.changes {
                if !reaching.contains(&document) {
                    reaching.push(document);
                }
            }
        }
        let shown = Document::Page(page.clone());
        let heads = self
            .heads_at(&reaching, to)?
            .into_iter()
            .find_map(|(document, heads)| (document == shown).then_some(heads));
        let doc = self.doc(&shown)?;
        let Some(heads) = heads.filter(|heads| {
            heads
                .iter()
                .all(|hash| doc.get_change_by_hash(hash).is_some())
        }) else {
            return Ok(documents);
        };
        let then = doc.fork_at(&heads)?;
        for view in Order::read(&then).window(0, usize::MAX) {
            let table = Document::Table(view.block);
            if view.kind == BlockKind::Database && !documents.contains(&table) {
                documents.push(table);
            }
        }
        Ok(documents)
    }

    fn present(&mut self, action: &Action, documents: &[Document]) -> bool {
        action.changes.iter().all(|(document, hash)| {
            !documents.contains(document)
                || self
                    .doc(document)
                    .ok()
                    .and_then(|doc| doc.get_change_by_hash(hash))
                    .is_some()
        })
    }

    pub fn versions(&mut self, page: &PageId) -> Result<Vec<Version>, Error> {
        let documents = self.page_documents(page)?;
        let shown = Document::Page(page.clone());
        let mut versions = Vec::new();
        let mut newer: Option<Action> = None;
        let mut before: Option<ActionId> = None;
        'pages: loop {
            let listed = self
                .store
                .history_of(&documents, before.as_ref(), HISTORY_BATCH)?;
            let Some(last) = listed.last() else {
                break;
            };
            before = Some(last.id.clone());
            for action in listed {
                if !self.present(&action, std::slice::from_ref(&shown)) {
                    break 'pages;
                }
                let gap = newer
                    .as_ref()
                    .is_some_and(|newer| newer.at - action.at >= SAVE_POINT_GAP_MS);
                let boundary = newer
                    .as_ref()
                    .is_none_or(|newer| gap || newer.actor != action.actor);
                if boundary {
                    versions.push(Version {
                        after: action.id.clone(),
                        at: action.at,
                        actor: action.actor.clone(),
                        save_point: gap,
                    });
                }
                let stops = matches!(
                    &action.operation,
                    Operation::Purge { target: Purge::Page(purged) } if purged == page
                );
                newer = Some(action);
                if stops {
                    break 'pages;
                }
            }
        }
        Ok(versions)
    }

    fn heads_at(
        &mut self,
        documents: &[Document],
        to: &ActionId,
    ) -> Result<Vec<(Document, Vec<ChangeHash>)>, Error> {
        let mut found: HashMap<Document, Vec<ChangeHash>> = HashMap::new();
        let mut reached = false;
        let mut before: Option<ActionId> = None;
        while found.len() < documents.len() {
            let page = self.store.history_of(documents, before.as_ref(), 1_000)?;
            let Some(last) = page.last() else {
                break;
            };
            before = Some(last.id.clone());
            for action in page {
                reached = reached || action.id == *to;
                if !reached {
                    continue;
                }
                for document in documents {
                    if found.contains_key(document) {
                        continue;
                    }
                    let heads = action
                        .heads
                        .iter()
                        .find(|(kept, _)| kept == document)
                        .map(|(_, heads)| heads.clone())
                        .or_else(|| {
                            action
                                .changes
                                .iter()
                                .find(|(kept, _)| kept == document)
                                .map(|(_, hash)| vec![*hash])
                        });
                    if let Some(heads) = heads {
                        found.insert(document.clone(), heads);
                    }
                }
            }
        }
        if !reached {
            return Err(Error::NotFound);
        }
        Ok(documents
            .iter()
            .filter_map(|document| Some((document.clone(), found.remove(document)?)))
            .collect())
    }

    pub fn restore(
        &mut self,
        actor: &Actor,
        page: &PageId,
        to: &ActionId,
    ) -> Result<Applied, Error> {
        self.permit(actor)?;
        let documents = self.page_documents_at(page, to)?;
        let mut targets = Vec::new();
        for (document, target) in self.heads_at(&documents, to)? {
            let doc = self.doc(&document)?;
            if target
                .iter()
                .all(|hash| doc.get_change_by_hash(hash).is_some())
            {
                targets.push((document, target));
            } else if document == Document::Page(page.clone()) {
                return Err(Error::Purged);
            }
        }
        let deleted = self.deleted_tables()?;
        let mut touched = Vec::new();
        for (document, target) in targets {
            let doc = self.doc(&document)?;
            let current = doc.get_heads();
            if current == target {
                continue;
            }
            let hidden = match document {
                Document::Page(_) => deleted_views(doc, &target, &deleted),
                _ => HashSet::new(),
            };
            let patches = doc
                .diff(&current, &target)
                .into_iter()
                .filter(|patch| !hidden.contains(&patch.obj))
                .collect();
            if let Err(error) = patch::apply(doc, patches) {
                self.rollback();
                return Err(error);
            }
            touched.push(document);
        }
        if touched.is_empty() {
            return Err(Error::NoChange);
        }
        let id = ActionId::new()?;
        let action = Action {
            id: id.clone(),
            actor: actor.clone(),
            origin: Origin::Ui,
            permission: Permission::Owner,
            operation: Operation::Restore {
                page: page.clone(),
                to: to.clone(),
            },
            changes: Vec::new(),
            heads: Vec::new(),
            at: 0,
        };
        let mut changed = Vec::new();
        let mut index = Vec::new();
        for document in &touched {
            if let Some(loaded) = self.docs.get_mut(document) {
                loaded.order = None;
                loaded.databases.clear();
            }
        }
        for document in &touched {
            self.changes_of(document, &mut changed, &mut index)?;
        }
        self.record(action, &touched, Vec::new(), index)?;
        changed.dedup();
        Ok(Applied {
            action: id,
            created: None,
            changed,
            ran: Vec::new(),
        })
    }

    pub fn preview(
        &mut self,
        page: &PageId,
        to: &ActionId,
    ) -> Result<Workspace<MemoryStore>, Error> {
        Workspace::open(MemoryStore::holding(self.version_documents(page, to)?))
    }

    pub fn version_documents(
        &mut self,
        page: &PageId,
        to: &ActionId,
    ) -> Result<Vec<(Document, Vec<u8>)>, Error> {
        let mut documents = vec![Document::Workspace];
        for area in self.areas()? {
            documents.push(Document::Area(area.id));
        }
        documents.extend(self.page_documents_at(page, to)?);
        let targets: HashMap<Document, Vec<ChangeHash>> =
            self.heads_at(&documents, to)?.into_iter().collect();
        let deleted = self.deleted_tables()?;
        let mut saved = Vec::new();
        for document in documents {
            let Ok(doc) = self.doc(&document) else {
                continue;
            };
            let bytes = match targets.get(&document) {
                Some(heads)
                    if heads
                        .iter()
                        .all(|hash| doc.get_change_by_hash(hash).is_some()) =>
                {
                    let mut then = doc.fork_at(heads)?;
                    if let Document::Page(_) = document {
                        let at = then.get_heads();
                        for view in deleted_views(&then, &at, &deleted) {
                            model::mark_deleted(&mut then, &view)?;
                        }
                    }
                    then.save()
                }
                _ => doc.save(),
            };
            saved.push((document, bytes));
        }
        Ok(saved)
    }
}
