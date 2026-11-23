use std::collections::{HashMap, HashSet};

use automerge::ChangeHash;
use automerge::transaction::CommitOptions;

use crate::store::Store;
use crate::workspace::{Recorded, header, page_entry, table_entry};
use crate::{
    Action, ActionId, Actor, Applied, BlockId, BlockKind, Changed, Document, Error, IndexWrite,
    MemoryStore, Mutation, Operation, Origin, PageId, Permission, Purge, RevertKind, Workspace,
    patch,
};

pub const TYPING_PAUSE_MS: i64 = 3_000;
pub const SAVE_POINT_GAP_MS: i64 = 300_000;
const PAGE: usize = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryQuery {
    pub page: Option<PageId>,
    pub actor: Option<Actor>,
    pub origin: Option<Origin>,
    pub before: Option<ActionId>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub actions: Vec<ActionId>,
    pub at: i64,
    pub actor: Actor,
    pub origin: Origin,
    pub permission: Permission,
    pub title: String,
    pub page: Option<PageId>,
    pub undone: bool,
    pub revertable: bool,
    pub purged: bool,
    pub deleted_table: Option<BlockId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entries {
    pub entries: Vec<Entry>,
    pub more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub after: ActionId,
    pub at: i64,
    pub actor: Actor,
    pub save_point: bool,
}

fn typing(action: &Action) -> Option<&BlockId> {
    match &action.operation {
        Operation::Mutation(Mutation::EditText { block, .. }) => Some(block),
        _ => None,
    }
}

fn together(newer: &Action, older: &Action) -> bool {
    newer.actor == older.actor
        && typing(newer).is_some()
        && typing(newer) == typing(older)
        && newer.at - older.at <= TYPING_PAUSE_MS
}

enum Visit {
    Next,
    Take,
    Stop,
}

pub(crate) fn undone(newest_first: &[Action]) -> HashSet<ActionId> {
    let mut undone = HashSet::new();
    for action in newest_first {
        if let Operation::Revert { reverted, .. } = &action.operation
            && !undone.contains(&action.id)
        {
            undone.extend(reverted.iter().cloned());
        }
    }
    undone
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

fn revertable(action: &Action) -> bool {
    matches!(
        action.operation,
        Operation::Mutation(_)
            | Operation::Run { .. }
            | Operation::Revert { .. }
            | Operation::Restore { .. }
    )
}

fn page_of(action: &Action) -> Option<PageId> {
    action
        .changes
        .iter()
        .find_map(|(document, _)| match document {
            Document::Page(page) => Some(page.clone()),
            _ => None,
        })
}

fn quoted(name: &str) -> String {
    format!("“{}”", if name.is_empty() { "Untitled" } else { name })
}

fn purged_title(kind: &str) -> &'static str {
    match kind {
        "EditText" | "AddTextBlock" | "SplitTextBlock" | "JoinTextBlock" | "SetTextStyle" => {
            "Edited text"
        }
        "SetCell" | "AddRow" | "RemoveRow" | "MoveRow" => "Changed rows",
        "AddField" | "RenameField" | "RemoveField" | "SetFieldFormula" | "AddOption" => {
            "Changed fields"
        }
        _ => "Changed a Block",
    }
}

impl<S: Store> Workspace<S> {
    fn page_titles(&mut self) -> Result<HashMap<PageId, String>, Error> {
        let mut titles = HashMap::new();
        for area in self.areas()? {
            for page in self.pages(&area.id)? {
                titles.insert(page.id, page.title);
            }
        }
        Ok(titles)
    }

    pub(crate) fn any_table_name(&mut self, table: &BlockId) -> Option<String> {
        let doc = self.doc(&Document::Workspace).ok()?;
        let map = crate::model::child(doc, &automerge::ROOT, crate::model::TABLES)?;
        let entry = crate::model::child(doc, &map, table.as_str())?;
        crate::model::string(doc, &entry, crate::model::NAME)
    }

    pub(crate) fn title(
        &mut self,
        action: &Action,
        titles: &HashMap<PageId, String>,
        known: &HashMap<ActionId, Action>,
    ) -> String {
        let page = |page: &PageId| quoted(titles.get(page).map_or("", String::as_str));
        let table = |this: &mut Self, table: &BlockId| match this.any_table_name(table) {
            Some(name) => quoted(&name),
            None => "a deleted Table".into(),
        };
        match &action.operation {
            Operation::CreateWorkspace => "Created the Workspace".into(),
            Operation::Purged { kind } => {
                format!("{} · content deleted for good", purged_title(kind))
            }
            Operation::Purge {
                target: Purge::Page(target),
            } => format!("Deleted the earlier versions of {}", page(target)),
            Operation::Purge {
                target: Purge::Table(target),
            } => match self.table_name(target).ok().flatten() {
                Some(name) => format!("Deleted the earlier versions of {}", quoted(&name)),
                None => "Deleted a Table for good".into(),
            },
            Operation::Restore { page: target, .. } => {
                format!("Restored a version of {}", page(target))
            }
            Operation::Run { automation, .. } => {
                let name = self
                    .automations()
                    .ok()
                    .and_then(|all| all.into_iter().find(|kept| kept.id == *automation))
                    .map(|kept| kept.setup.name);
                format!(
                    "Ran {}",
                    name.map_or("an Automation".into(), |name| quoted(&name))
                )
            }
            Operation::Revert { reverted, kind } => {
                let first = reverted
                    .first()
                    .and_then(|id| known.get(id))
                    .cloned()
                    .map(|first| self.title(&first, titles, &HashMap::new()));
                let what = first.unwrap_or_else(|| "a change".into());
                match kind {
                    RevertKind::Revert => format!("Reverted: {what}"),
                    RevertKind::Undo => format!("Undo: {what}"),
                    RevertKind::Redo => format!("Redo: {what}"),
                }
            }
            Operation::Mutation(mutation) => match mutation {
                Mutation::CreateArea { name } => format!("Created the area {}", quoted(name)),
                Mutation::RenameWorkspace { .. } | Mutation::SetWorkspaceIcon { .. } => {
                    "Renamed the Workspace".into()
                }
                Mutation::CreatePage { title, .. } => format!("Created the Page {}", quoted(title)),
                Mutation::RenamePage { title, .. } => {
                    format!("Renamed a Page to {}", quoted(title))
                }
                Mutation::AddTextBlock { page: on, .. }
                | Mutation::EditText { page: on, .. }
                | Mutation::SplitTextBlock { page: on, .. }
                | Mutation::JoinTextBlock { page: on, .. }
                | Mutation::SetTextStyle { page: on, .. }
                | Mutation::SetSectionBoundary { page: on, .. }
                | Mutation::TurnIntoCode { page: on, .. }
                | Mutation::TurnIntoText { page: on, .. }
                | Mutation::TurnIntoMath { page: on, .. }
                | Mutation::SetCodeLanguage { page: on, .. } => {
                    format!("Edited text on {}", page(on))
                }
                Mutation::RemoveView { page: on, .. } => {
                    format!("Removed a Block from {}", page(on))
                }
                Mutation::MoveViews { page: on, .. } => format!("Moved Blocks on {}", page(on)),
                Mutation::SetTile { page: on, .. } | Mutation::SetInnerTile { page: on, .. } => {
                    format!("Arranged {}", page(on))
                }
                Mutation::AddMediaBlock { page: on, .. }
                | Mutation::AddWhiteboardBlock { page: on, .. }
                | Mutation::AddCalculationBlock { page: on, .. }
                | Mutation::AddChartBlock { page: on, .. } => {
                    format!("Added a Block to {}", page(on))
                }
                Mutation::SetChart { page: on, .. }
                | Mutation::AddLine { page: on, .. }
                | Mutation::SetLine { page: on, .. }
                | Mutation::RemoveLine { page: on, .. }
                | Mutation::SetDrawingStyle { page: on, .. }
                | Mutation::EditDrawing { page: on, .. }
                | Mutation::AddTab { page: on, .. }
                | Mutation::RenameTab { page: on, .. }
                | Mutation::SetTabLayout { page: on, .. }
                | Mutation::SetTabGroup { page: on, .. }
                | Mutation::SetTabDate { page: on, .. }
                | Mutation::SetTabSort { page: on, .. }
                | Mutation::SetTabFilter { page: on, .. }
                | Mutation::SetTabTotal { page: on, .. }
                | Mutation::SetTabFolded { page: on, .. }
                | Mutation::MoveTab { page: on, .. }
                | Mutation::RemoveTab { page: on, .. }
                | Mutation::RenameView { page: on, .. } => {
                    format!("Changed a Block on {}", page(on))
                }
                Mutation::AddDatabaseBlock { name, .. } | Mutation::CreateTable { name, .. } => {
                    format!("Created the Table {}", quoted(name))
                }
                Mutation::DuplicateTable { table: from, .. } => {
                    format!("Duplicated the Table {}", table(self, from))
                }
                Mutation::AddTableView {
                    table: shown,
                    page: on,
                    ..
                } => {
                    format!("Added a View of {} to {}", table(self, shown), page(on))
                }
                Mutation::DeleteTable { block } => {
                    format!("Deleted the Table {}", table(self, block))
                }
                Mutation::RenameDatabase { name, .. } => {
                    format!("Renamed a Table to {}", quoted(name))
                }
                Mutation::AddField { block, .. }
                | Mutation::RenameField { block, .. }
                | Mutation::SetFieldFormula { block, .. }
                | Mutation::RemoveField { block, .. }
                | Mutation::AddOption { block, .. } => {
                    format!("Changed the fields of {}", table(self, block))
                }
                Mutation::AddRow { block, .. } => format!("Added a row to {}", table(self, block)),
                Mutation::RemoveRow { block, .. } => {
                    format!("Deleted a row of {}", table(self, block))
                }
                Mutation::MoveRow { block, .. } => format!("Moved a row of {}", table(self, block)),
                Mutation::SetCell { block, .. } => {
                    format!("Changed a cell of {}", table(self, block))
                }
                Mutation::CreateAutomation { setup } => {
                    format!("Created the Automation {}", quoted(&setup.name))
                }
                Mutation::SetAutomation { setup, .. } => {
                    format!("Changed the Automation {}", quoted(&setup.name))
                }
                Mutation::SetAutomationOn { on, .. } => if *on {
                    "Turned an Automation on"
                } else {
                    "Turned an Automation off"
                }
                .into(),
                Mutation::SetAutomationRole { .. } => {
                    "Changed the Role an Automation runs as".into()
                }
                Mutation::RemoveAutomation { .. } => "Deleted an Automation".into(),
                Mutation::CreateRole { setup } => {
                    format!("Created the Role {}", quoted(&setup.name))
                }
                Mutation::SetRole { setup, .. } => {
                    format!("Changed the Role {}", quoted(&setup.name))
                }
                Mutation::RemoveRole { .. } => "Deleted a Role".into(),
                Mutation::ProtectTable {
                    table: protected, ..
                } => {
                    format!("Protected {}", table(self, protected))
                }
                Mutation::UnprotectTable { table: protected } => {
                    format!("Stopped protecting {}", table(self, protected))
                }
            },
        }
    }

    pub fn entries(&mut self, query: &EntryQuery) -> Result<Entries, Error> {
        let documents = match &query.page {
            Some(page) => Some(self.page_documents(page)?),
            None => None,
        };
        let mut listed: Vec<Action> = Vec::new();
        let mut before = query.before.clone();
        let mut starts = 0;
        while starts <= query.limit {
            let page = match &documents {
                Some(documents) => self.store.history_of(documents, before.as_ref(), PAGE)?,
                None => self.store.history(before.as_ref(), PAGE)?,
            };
            let Some(last) = page.last() else {
                break;
            };
            before = Some(last.id.clone());
            for action in page {
                let shown = query
                    .actor
                    .as_ref()
                    .is_none_or(|actor| *actor == action.actor)
                    && query
                        .origin
                        .as_ref()
                        .is_none_or(|origin| *origin == action.origin);
                if !shown {
                    continue;
                }
                if listed.last().is_none_or(|newer| !together(newer, &action)) {
                    starts += 1;
                }
                listed.push(action);
            }
        }
        let oldest: Vec<ActionId> = listed
            .last()
            .map(|action| action.id.clone())
            .into_iter()
            .collect();
        let recent = self.reaching(&oldest)?;
        let undone = undone(&recent);
        let known: HashMap<ActionId, Action> = recent
            .into_iter()
            .chain(listed.iter().cloned())
            .map(|action| (action.id.clone(), action))
            .collect();
        let titles = self.page_titles()?;
        let shown = listed;
        let mut entries: Vec<Entry> = Vec::new();
        let mut more = false;
        let mut last: Option<Action> = None;
        for action in shown {
            let joins = last.as_ref().is_some_and(|newer| {
                together(newer, &action)
                    && undone.contains(&newer.id) == undone.contains(&action.id)
            });
            if joins && let Some(entry) = entries.last_mut() {
                entry.actions.push(action.id.clone());
                last = Some(action);
                continue;
            }
            if entries.len() == query.limit {
                more = true;
                break;
            }
            let purged = matches!(action.operation, Operation::Purged { .. });
            let deleted_table = match &action.operation {
                Operation::Mutation(Mutation::DeleteTable { block })
                    if self.table_name(block)?.is_none()
                        && self
                            .any_table_name(block)
                            .is_some_and(|name| !name.is_empty()) =>
                {
                    Some(block.clone())
                }
                _ => None,
            };
            entries.push(Entry {
                actions: vec![action.id.clone()],
                at: action.at,
                actor: action.actor.clone(),
                origin: action.origin.clone(),
                permission: action.permission.clone(),
                title: self.title(&action, &titles, &known),
                page: page_of(&action),
                undone: undone.contains(&action.id),
                revertable: revertable(&action)
                    && !action.changes.is_empty()
                    && !undone.contains(&action.id)
                    && self.kept(&action),
                purged,
                deleted_table,
            });
            last = Some(action);
        }
        Ok(Entries { entries, more })
    }

    fn page_documents(&mut self, page: &PageId) -> Result<Vec<Document>, Error> {
        let mut documents = vec![Document::Page(page.clone())];
        for view in self.views(page, 0, usize::MAX)? {
            if view.kind == BlockKind::Database {
                let table = Document::Table(view.block);
                if !documents.contains(&table) {
                    documents.push(table);
                }
            }
        }
        Ok(documents)
    }

    fn reaching(&mut self, ids: &[ActionId]) -> Result<Vec<Action>, Error> {
        let mut all: Vec<Action> = Vec::new();
        let mut missing: HashSet<&ActionId> = ids.iter().collect();
        while !missing.is_empty() {
            let page = self
                .store
                .history(all.last().map(|action| &action.id), 1_000)?;
            if page.is_empty() {
                break;
            }
            for action in &page {
                missing.remove(&action.id);
            }
            all.extend(page);
        }
        Ok(all)
    }

    fn actions(&mut self, ids: &[ActionId]) -> Result<Vec<Action>, Error> {
        let recent = self.reaching(ids)?;
        let mut found: Vec<Action> = recent
            .into_iter()
            .filter(|action| ids.contains(&action.id))
            .collect();
        if found.len() != ids.len() {
            return Err(Error::NotFound);
        }
        found.sort_by_key(|action| std::cmp::Reverse(action.at));
        Ok(found)
    }

    pub fn revert(&mut self, actor: &Actor, ids: &[ActionId]) -> Result<Applied, Error> {
        self.permit(actor)?;
        let recent = self.reaching(ids)?;
        let carriers: Vec<ActionId> = ids.iter().map(|id| carrier(&recent, id)).collect();
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
        let mut steps: Vec<(Document, ChangeHash)> = Vec::new();
        for action in actions {
            if action.changes.is_empty() {
                return Err(Error::Purged);
            }
            for (document, hash) in &action.changes {
                let doc = self.doc(document).map_err(|_| Error::Purged)?;
                let change = doc.get_change_by_hash(hash).ok_or(Error::Purged)?;
                let deps = change.deps().to_vec();
                let current = doc.get_heads();
                let undoes: HashSet<_> = patch::touched(&doc.diff(&deps, &[*hash]))
                    .into_iter()
                    .collect();
                let later = patch::touched(&doc.diff(&[*hash], &current));
                let own_group = actions.len() > 1;
                if !own_group && later.iter().any(|key| undoes.contains(key)) {
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

    pub(crate) fn changes_of(
        &mut self,
        document: &Document,
        changed: &mut Vec<Changed>,
        index: &mut Vec<IndexWrite>,
    ) -> Result<(), Error> {
        match document {
            Document::Workspace => {
                changed.extend([
                    Changed::Workspace,
                    Changed::Areas,
                    Changed::Tables,
                    Changed::Automations,
                    Changed::Roles,
                ]);
                for table in self.tables()? {
                    index.push(table_entry(&table.id, Some(&table.name)));
                    changed.push(Changed::Table(table.id));
                }
            }
            Document::Area(area) => {
                changed.push(Changed::Pages(area.clone()));
                for page in self.pages(area)? {
                    index.push(page_entry(&page.id, &page.title));
                }
            }
            Document::Page(page) => {
                changed.push(Changed::Views(page.clone()));
                for view in self.views(page, 0, usize::MAX)? {
                    changed.push(Changed::Block(page.clone(), view.block));
                }
                index.extend(self.page_text(page)?);
            }
            Document::Table(table) => {
                changed.push(Changed::Tables);
                changed.push(Changed::Table(table.clone()));
            }
        }
        Ok(())
    }

    fn own_newest_first(
        &mut self,
        actor: &Actor,
        mut visit: impl FnMut(&Action, &HashSet<ActionId>) -> Visit,
    ) -> Result<Option<Action>, Error> {
        self.permit(actor)?;
        let mut undone = HashSet::new();
        let mut before: Option<ActionId> = None;
        loop {
            let page = self.store.history(before.as_ref(), PAGE)?;
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
                match visit(&action, &undone) {
                    Visit::Next => {}
                    Visit::Take => return Ok(Some(action)),
                    Visit::Stop => return Ok(None),
                }
            }
        }
    }

    pub fn undo(&mut self, actor: &Actor) -> Result<Option<Applied>, Error> {
        let mut group: Vec<Action> = Vec::new();
        self.own_newest_first(actor, |action, undone| {
            let counts = !undone.contains(&action.id)
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
        let target = self.own_newest_first(actor, |action, undone| {
            if undone.contains(&action.id) {
                return Visit::Next;
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

    fn kept(&mut self, action: &Action) -> bool {
        action.changes.iter().all(|(document, hash)| {
            self.doc(document)
                .ok()
                .and_then(|doc| doc.get_change_by_hash(hash))
                .is_some()
        })
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
        let mut versions = Vec::new();
        let mut newer: Option<Action> = None;
        let mut before: Option<ActionId> = None;
        'pages: loop {
            let listed = self.store.history_of(&documents, before.as_ref(), PAGE)?;
            let Some(last) = listed.last() else {
                break;
            };
            before = Some(last.id.clone());
            for action in listed {
                if !self.present(&action, &documents) {
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
                let stops = matches!(action.operation, Operation::Purge { .. });
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
        let documents = self.page_documents(page)?;
        let targets = self.heads_at(&documents, to)?;
        for (document, target) in &targets {
            let doc = self.doc(document)?;
            if target
                .iter()
                .any(|hash| doc.get_change_by_hash(hash).is_none())
            {
                return Err(Error::Purged);
            }
        }
        let mut touched = Vec::new();
        for (document, target) in targets {
            let doc = self.doc(&document)?;
            let current = doc.get_heads();
            if current == target {
                continue;
            }
            let patches = doc.diff(&current, &target);
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
        self.record(action, &touched, Vec::new(), Vec::new())?;
        for document in &touched {
            self.changes_of(document, &mut changed, &mut index)?;
        }
        self.store.index(&index, None)?;
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
        documents.extend(self.page_documents(page)?);
        let targets: HashMap<Document, Vec<ChangeHash>> =
            self.heads_at(&documents, to)?.into_iter().collect();
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
                    doc.fork_at(heads)?.save()
                }
                _ => doc.save(),
            };
            saved.push((document, bytes));
        }
        Ok(saved)
    }
}
