use std::collections::HashSet;

use automerge::transaction::CommitOptions;
use automerge::{ChangeHash, ReadDoc};

use crate::revision::{HISTORY_BATCH, revertable, together};
use crate::store::Store;
use crate::workspace::{Recorded, Workspace, header};
use crate::{
    Action, ActionId, Actor, Applied, Document, Error, Operation, Origin, Permission, RevertKind,
    patch,
};

enum Visit {
    Next,
    Take,
    Stop,
}

fn reverter<'a>(recent: &'a [Action], id: &ActionId) -> Option<&'a Action> {
    recent.iter().find(|action| {
        matches!(&action.operation, Operation::Revert { reverted, .. } if reverted.contains(id))
    })
}

fn carrier(recent: &[Action], id: &ActionId) -> ActionId {
    let mut current = id.clone();
    for _ in 0..recent.len() {
        let Some(undo) = reverter(recent, &current) else {
            break;
        };
        let Some(redo) = reverter(recent, &undo.id) else {
            break;
        };
        current = redo.id.clone();
    }
    current
}

impl<S: Store> Workspace<S> {
    pub fn revert(&mut self, actor: &Actor, ids: &[ActionId]) -> Result<Applied, Error> {
        self.permit(actor)?;
        let recent = self.reaching(ids)?;
        let mut carriers: Vec<ActionId> = Vec::new();
        for id in ids {
            let carried = carrier(&recent, id);
            if !carriers.contains(&carried) {
                carriers.push(carried);
            }
        }
        let actions = self.actions(&carriers)?;
        if actions.iter().any(|action| !revertable(action)) {
            return Err(Error::Purged);
        }
        self.revert_actions(actor, &actions, RevertKind::Revert)
    }

    fn revert_actions(
        &mut self,
        actor: &Actor,
        actions: &[Action],
        kind: RevertKind,
    ) -> Result<Applied, Error> {
        for action in actions {
            if action.changes.is_empty() || !self.kept(action) {
                return Err(Error::Purged);
            }
        }
        let mut steps: Vec<(Document, ChangeHash)> = Vec::new();
        for action in actions {
            for (document, hash) in &action.changes {
                let doc = self.doc(document).map_err(|_| Error::Purged)?;
                let change = doc.get_change_by_hash(hash).ok_or(Error::Purged)?;
                let deps = change.deps().to_vec();
                let current = doc.get_heads();
                let undoes: HashSet<_> = patch::touched(&doc.diff(&deps, &[*hash]))
                    .into_iter()
                    .collect();
                let group: Vec<ChangeHash> = actions
                    .iter()
                    .flat_map(|member| &member.changes)
                    .filter(|(kept, _)| kept == document)
                    .map(|(_, member)| *member)
                    .collect();
                let later = patch::touched(&doc.diff(&group, &current));
                if later.iter().any(|key| undoes.contains(key)) {
                    return Err(Error::Conflict);
                }
                steps.push((document.clone(), *hash));
            }
        }
        let id = ActionId::new()?;
        let action = Action {
            id: id.clone(),
            actor: actor.clone(),
            origin: Origin::Ui,
            permission: Permission::Owner,
            operation: Operation::Revert {
                reverted: actions.iter().map(|action| action.id.clone()).collect(),
                kind,
            },
            changes: Vec::new(),
            heads: Vec::new(),
            at: 0,
        };
        let message = header(&action);
        let mut documents = Vec::new();
        let merged = self
            .merge_reverts(steps, &message, &mut documents)
            .and_then(|recorded| self.settle(id, action, &documents, recorded));
        if merged.is_err() {
            for document in &documents {
                self.docs.remove(document);
            }
        }
        merged
    }

    fn merge_reverts(
        &mut self,
        steps: Vec<(Document, ChangeHash)>,
        message: &str,
        documents: &mut Vec<Document>,
    ) -> Result<Recorded, Error> {
        let mut recorded = Recorded::default();
        for (document, hash) in steps {
            let doc = self.doc(&document)?;
            let deps = doc
                .get_change_by_hash(&hash)
                .ok_or(Error::Purged)?
                .deps()
                .to_vec();
            let mut fork = doc.fork_at(&[hash])?;
            let patches = fork.diff(&[hash], &deps);
            patch::apply(&mut fork, patches)?;
            let options = CommitOptions::default().with_message(message.to_owned());
            let Some(made) = fork.commit_with(options) else {
                continue;
            };
            let chunk = fork
                .get_change_by_hash(&made)
                .ok_or(Error::NotFound)?
                .bytes()
                .into_owned();
            documents.push(document.clone());
            doc.merge(&mut fork)?;
            recorded.merged.push((document, made, chunk));
        }
        if recorded.merged.is_empty() {
            return Err(Error::NoChange);
        }
        Ok(recorded)
    }

    fn settle(
        &mut self,
        id: ActionId,
        action: Action,
        documents: &[Document],
        recorded: Recorded,
    ) -> Result<Applied, Error> {
        let mut changed = Vec::new();
        let mut index = Vec::new();
        for document in documents {
            if let Some(loaded) = self.docs.get_mut(document) {
                loaded.order = None;
                loaded.databases.clear();
            }
            self.changes_of(document, &mut changed, &mut index)?;
        }
        self.record_with(action, &[], Vec::new(), index, recorded)?;
        changed.dedup();
        Ok(Applied {
            action: id,
            created: None,
            changed,
            ran: Vec::new(),
        })
    }

    fn own_newest_first(
        &mut self,
        actor: &Actor,
        mut visit: impl FnMut(&Action, &HashSet<ActionId>, bool) -> Visit,
    ) -> Result<Option<Action>, Error> {
        self.permit(actor)?;
        let mut undone = HashSet::new();
        let mut before: Option<ActionId> = None;
        loop {
            let page = self.store.history(before.as_ref(), HISTORY_BATCH)?;
            let Some(last) = page.last() else {
                return Ok(None);
            };
            before = Some(last.id.clone());
            for action in page {
                if let Operation::Revert { reverted, .. } = &action.operation
                    && !undone.contains(&action.id)
                {
                    undone.extend(reverted.iter().cloned());
                }
                if action.actor != *actor || action.origin != Origin::Ui {
                    continue;
                }
                let kept = self.kept(&action);
                match visit(&action, &undone, kept) {
                    Visit::Next => {}
                    Visit::Take => return Ok(Some(action)),
                    Visit::Stop => return Ok(None),
                }
            }
        }
    }

    pub fn undo(&mut self, actor: &Actor) -> Result<Option<Applied>, Error> {
        let mut group: Vec<Action> = Vec::new();
        self.own_newest_first(actor, |action, undone, kept| {
            let counts = kept
                && !undone.contains(&action.id)
                && match &action.operation {
                    Operation::Mutation(_) | Operation::Restore { .. } => true,
                    Operation::Revert { kind, .. } => *kind != RevertKind::Undo,
                    _ => false,
                };
            match group.last() {
                None if counts => group.push(action.clone()),
                None => {}
                Some(newer) if counts && together(newer, action) => group.push(action.clone()),
                Some(_) => return Visit::Stop,
            }
            Visit::Next
        })?;
        if group.is_empty() {
            return Ok(None);
        }
        self.revert_actions(actor, &group, RevertKind::Undo)
            .map(Some)
    }

    pub fn redo(&mut self, actor: &Actor) -> Result<Option<Applied>, Error> {
        let target = self.own_newest_first(actor, |action, undone, kept| {
            if undone.contains(&action.id) {
                return Visit::Next;
            }
            if !kept {
                return Visit::Stop;
            }
            match &action.operation {
                Operation::Revert {
                    kind: RevertKind::Redo,
                    ..
                } => Visit::Next,
                Operation::Revert {
                    kind: RevertKind::Undo,
                    ..
                } => Visit::Take,
                _ => Visit::Stop,
            }
        })?;
        match target {
            Some(undo) => self
                .revert_actions(actor, &[undo], RevertKind::Redo)
                .map(Some),
            None => Ok(None),
        }
    }
}
