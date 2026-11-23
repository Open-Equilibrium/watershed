use std::collections::HashSet;

use automerge::legacy::OpType;
use automerge::transaction::{CommitOptions, Transactable};
use automerge::{AutoCommit, ROOT, ReadDoc, ScalarValue};

use crate::model::{self, AREAS, NAME, PAGES, TABLES};
use crate::store::{Store, Write};
use crate::workspace::{Recorded, header};
use crate::{
    Action, ActionId, Actor, Applied, AreaId, AttachmentId, BlockId, Document, Error, Mutation,
    Operation, Origin, PageId, Permission, Purge, Workspace, patch,
};

const PAGE: usize = 1_000;

fn kind(operation: &Operation) -> String {
    match operation {
        Operation::Mutation(mutation) => serde_json::to_value(mutation)
            .ok()
            .and_then(|value| value.as_object()?.keys().next().cloned())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn redacted(
    mut action: Action,
    gone: &Document,
    mention: Option<&str>,
    created: bool,
) -> Option<Action> {
    let touches = action.changes.iter().any(|(document, _)| document == gone);
    let names = mention.is_some_and(|id| {
        matches!(action.operation, Operation::Mutation(_))
            && serde_json::to_string(&action.operation).is_ok_and(|text| text.contains(id))
    });
    if !touches && !names && !created {
        return None;
    }
    action.heads.retain(|(document, _)| document != gone);
    if matches!(action.operation, Operation::Mutation(_)) {
        action.changes.retain(|(document, _)| document != gone);
        action.operation = Operation::Purged {
            kind: kind(&action.operation),
        };
    }
    Some(action)
}

fn attachments(
    doc: &mut AutoCommit,
    wanted: Option<&HashSet<AttachmentId>>,
) -> HashSet<AttachmentId> {
    let mut found = HashSet::new();
    for change in doc.get_changes(&[]) {
        for op in change.decode().operations {
            if let OpType::Put(ScalarValue::Str(text)) = op.action
                && let Some(id) = AttachmentId::parse(&text)
                && wanted.is_none_or(|wanted| wanted.contains(&id))
            {
                found.insert(id);
            }
        }
    }
    found
}

fn keys(doc: &AutoCommit, map: &str) -> Vec<String> {
    model::child(doc, &ROOT, map).map_or_else(Vec::new, |top| {
        doc.map_range(&top, ..)
            .map(|item| item.key.to_string())
            .collect()
    })
}

impl<S: Store> Workspace<S> {
    fn documents(&mut self) -> Result<Vec<Document>, Error> {
        let workspace = self.doc(&Document::Workspace)?;
        let tables = keys(workspace, TABLES);
        let areas = keys(workspace, AREAS);
        let mut documents = vec![Document::Workspace];
        for area in areas {
            let area = Document::Area(AreaId::stored(&area));
            let pages = keys(self.doc(&area)?, PAGES);
            documents.push(area);
            documents.extend(
                pages
                    .iter()
                    .map(|page| Document::Page(PageId::stored(page))),
            );
        }
        documents.extend(
            tables
                .iter()
                .map(|table| Document::Table(BlockId::stored(table))),
        );
        Ok(documents)
    }

    fn forgotten(
        &mut self,
        candidates: HashSet<AttachmentId>,
        gone: Option<&Document>,
    ) -> Result<Vec<Write>, Error> {
        let mut unused = candidates;
        for document in self.documents()? {
            if unused.is_empty() {
                break;
            }
            if Some(&document) == gone {
                continue;
            }
            let doc = match self.doc(&document) {
                Ok(doc) => doc,
                Err(Error::NotFound) => continue,
                Err(error) => return Err(error),
            };
            for used in attachments(doc, Some(&unused)) {
                unused.remove(&used);
            }
        }
        Ok(unused
            .into_iter()
            .map(|attachment| Write::Forget { attachment })
            .collect())
    }

    fn creates(&mut self, action: &Action, page: &PageId) -> Result<bool, Error> {
        if !matches!(
            action.operation,
            Operation::Mutation(Mutation::CreatePage { .. })
        ) {
            return Ok(false);
        }
        for (document, hash) in &action.changes {
            let doc = self.doc(document)?;
            let Some(change) = doc.get_change_by_hash(hash) else {
                continue;
            };
            let deps = change.deps().to_vec();
            if patch::touched(&doc.diff(&deps, &[*hash]))
                .iter()
                .any(|(_, key)| key == page.as_str())
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn purge(&mut self, actor: &Actor, target: &Purge) -> Result<Applied, Error> {
        self.permit(actor)?;
        let purged = self.purging(actor, target);
        if purged.is_err() {
            self.rollback();
            let document = match target {
                Purge::Page(page) => Document::Page(page.clone()),
                Purge::Table(table) => Document::Table(table.clone()),
            };
            self.docs.remove(&document);
        }
        purged
    }

    fn purging(&mut self, actor: &Actor, target: &Purge) -> Result<Applied, Error> {
        let (document, removed) = match target {
            Purge::Page(page) => {
                self.doc(&Document::Page(page.clone()))?;
                (Document::Page(page.clone()), false)
            }
            Purge::Table(table) => {
                let deleted = self.table_name(table)?.is_none();
                if deleted && self.any_table_name(table).is_none() {
                    return Err(Error::NotFound);
                }
                (Document::Table(table.clone()), deleted)
            }
        };
        let id = ActionId::new()?;
        let mut action = Action {
            id: id.clone(),
            actor: actor.clone(),
            origin: Origin::Ui,
            permission: Permission::Owner,
            operation: Operation::Purge {
                target: target.clone(),
            },
            changes: Vec::new(),
            heads: Vec::new(),
            at: 0,
        };
        let message = header(&action);
        let candidates = attachments(self.doc(&document)?, None);
        let mut recorded = Recorded::default();
        let (mention, scanned) = match target {
            Purge::Table(table) if removed => (Some(table.to_string()), None),
            Purge::Table(_) => (None, Some(vec![document.clone()])),
            Purge::Page(page) => {
                let mut scanned = vec![document.clone()];
                scanned.extend(self.area_of(page).ok().map(Document::Area));
                (Some(page.to_string()), Some(scanned))
            }
        };
        let mut rebuilt = Vec::new();
        let mut touched = Vec::new();
        if removed {
            recorded.writes.push(Write::Remove {
                document: document.clone(),
            });
            recorded.dropped.push(document.clone());
            self.docs.remove(&document);
            let Purge::Table(table) = target else {
                unreachable!("only a Table is removed")
            };
            let doc = self.doc(&Document::Workspace)?;
            let map = model::child(doc, &ROOT, TABLES).ok_or(Error::NotFound)?;
            let entry = model::child(doc, &map, table.as_str()).ok_or(Error::NotFound)?;
            doc.put(&entry, NAME, "")?;
            touched.push(Document::Workspace);
        } else {
            rebuilt.push(document.clone());
        }
        for kept in rebuilt {
            let mut fresh = model::document(&self.device);
            let doc = self.doc(&kept)?;
            patch::rebuild(doc, &mut fresh)?;
            let options = CommitOptions::default().with_message(message.clone());
            let made = fresh.commit_with(options).ok_or(Error::NoChange)?;
            let snapshot = fresh.save();
            action.changes.push((kept.clone(), made));
            recorded.writes.push(Write::Replace {
                document: kept.clone(),
                snapshot,
            });
            recorded.dropped.push(kept.clone());
            if let Some(loaded) = self.docs.get_mut(&kept) {
                loaded.doc = fresh;
                loaded.chunks = 1;
                loaded.order = None;
                loaded.databases.clear();
            }
        }
        let mut redact: Vec<Action> = Vec::new();
        let mut before: Option<ActionId> = None;
        loop {
            let listed = match &scanned {
                None => self.store.history(before.as_ref(), PAGE)?,
                Some(scanned) => self.store.history_of(scanned, before.as_ref(), PAGE)?,
            };
            let Some(last) = listed.last() else {
                break;
            };
            before = Some(last.id.clone());
            for kept in listed {
                let created = match target {
                    Purge::Page(page) => self.creates(&kept, page)?,
                    Purge::Table(_) => false,
                };
                redact.extend(redacted(kept, &document, mention.as_deref(), created));
            }
        }
        recorded.redact = redact;
        let gone = removed.then_some(&document);
        let forgotten = self.forgotten(candidates, gone)?;
        recorded.writes.extend(forgotten);
        let mut changed = Vec::new();
        let mut index = Vec::new();
        let shown: Vec<Document> = action
            .changes
            .iter()
            .map(|(kept, _)| kept.clone())
            .chain(touched.iter().cloned())
            .collect();
        for kept in &shown {
            self.changes_of(kept, &mut changed, &mut index)?;
        }
        if let (true, Purge::Table(table)) = (removed, target) {
            changed.push(crate::Changed::Table(table.clone()));
        }
        self.record_with(action, &touched, Vec::new(), index, recorded)?;
        changed.dedup();
        Ok(Applied {
            action: id,
            created: None,
            changed,
            ran: Vec::new(),
        })
    }
}
