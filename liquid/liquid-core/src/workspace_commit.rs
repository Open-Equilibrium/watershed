use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use automerge::transaction::{CommitOptions, Transactable};
use automerge::{AutoCommit, ReadDoc};

use crate::history::Header;
use crate::store::{Commit, Store, Write};
use crate::workspace::{Loaded, Recorded, Workspace};
use crate::{
    Action, ActionId, Applied, Created, Document, Error, IndexWrite, Mutation, Operation, Request,
    RowId,
};

const COMPACT_AFTER: usize = 1_000;

fn trigger(mutation: &Mutation) -> Option<(crate::Trigger, Option<RowId>)> {
    match mutation {
        Mutation::AddRow { block, .. } => Some((crate::Trigger::RowAdded(block.clone()), None)),
        Mutation::SetCell {
            block, row, field, ..
        } => Some((
            crate::Trigger::FieldChanged(block.clone(), field.clone()),
            Some(row.clone()),
        )),
        _ => None,
    }
}

impl<S: Store> Workspace<S> {
    pub fn apply(&mut self, request: Request) -> Result<Applied, Error> {
        let permission = self.permit(&request.actor)?;
        let id = ActionId::new()?;
        let leaving = self.leaving(&request.mutation);
        let trigger = trigger(&request.mutation);
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
            heads: Vec::new(),
            at: 0,
        };
        self.record(action, &written.touched, written.new, index)?;
        let ran = match (trigger, &written.created) {
            (Some((trigger, Some(row))), _) => self.run_triggered(&trigger, &row, Some(&id)),
            (Some((trigger, None)), Some(Created::Row(row))) => {
                self.run_triggered(&trigger, row, Some(&id))
            }
            _ => Vec::new(),
        };
        Ok(Applied {
            action: id,
            created: written.created,
            changed: written.changed,
            ran,
        })
    }

    pub(crate) fn rollback(&mut self) {
        for loaded in self.docs.values_mut() {
            if loaded.doc.rollback() > 0 {
                loaded.order = None;
                loaded.databases.clear();
            }
        }
    }

    pub(crate) fn record(
        &mut self,
        action: Action,
        touched: &[Document],
        new: Vec<(Document, AutoCommit)>,
        index: Vec<IndexWrite>,
    ) -> Result<(), Error> {
        self.record_with(action, touched, new, index, Recorded::default())
    }

    pub(crate) fn record_with(
        &mut self,
        mut action: Action,
        touched: &[Document],
        new: Vec<(Document, AutoCommit)>,
        index: Vec<IndexWrite>,
        recorded: Recorded,
    ) -> Result<(), Error> {
        action.at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64);
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
        let mut writes = recorded.writes;
        for (document, hash, chunk) in recorded.merged {
            action.changes.push((document.clone(), hash));
            if let Some(loaded) = self.docs.get_mut(&document) {
                loaded.chunks += 1;
            }
            writes.push(Write::Append { document, chunk });
        }
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
        let mut documents: Vec<Document> = action
            .changes
            .iter()
            .map(|(document, _)| document.clone())
            .collect();
        documents.sort_by_key(Document::key);
        documents.dedup();
        for document in documents {
            if let Some(loaded) = self.docs.get_mut(&document) {
                action.heads.push((document, loaded.doc.get_heads()));
            }
        }
        let commit = Commit {
            action: &action,
            writes,
            index,
            redact: recorded.redact,
        };
        if let Err(error) = self.store.commit(&commit) {
            for document in touched.iter().chain(&recorded.dropped) {
                self.docs.remove(document);
            }
            return Err(error);
        }
        Ok(())
    }
}
