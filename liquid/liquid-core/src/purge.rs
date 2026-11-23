use automerge::ROOT;
use automerge::transaction::{CommitOptions, Transactable};

use crate::model::{self, NAME, TABLES};
use crate::store::{Store, Write};
use crate::workspace::{Recorded, header};
use crate::{
    Action, ActionId, Actor, Applied, Document, Error, Mutation, Operation, Origin, PageId,
    Permission, Purge, Workspace, patch,
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

impl<S: Store> Workspace<S> {
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
