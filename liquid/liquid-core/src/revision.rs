use std::collections::{HashMap, HashSet};

use crate::store::Store;
use crate::workspace::Workspace;
use crate::workspace_index::{page_entry, table_entry};
use crate::{
    Action, ActionId, Actor, BlockId, BlockKind, Changed, Document, Error, IndexWrite, Mutation,
    Operation, Origin, PageId, Permission, Purge, RevertKind, SourceId,
};

pub const TYPING_PAUSE_MS: i64 = 3_000;

pub const SAVE_POINT_GAP_MS: i64 = 300_000;

pub(crate) const HISTORY_BATCH: usize = 1_000;

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

pub(crate) fn together(newer: &Action, older: &Action) -> bool {
    newer.actor == older.actor
        && typing(newer).is_some()
        && typing(newer) == typing(older)
        && newer.at - older.at <= TYPING_PAUSE_MS
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

pub(crate) fn revertable(action: &Action) -> bool {
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
                Some(documents) => {
                    self.store
                        .history_of(documents, before.as_ref(), HISTORY_BATCH)?
                }
                None => self.store.history(before.as_ref(), HISTORY_BATCH)?,
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
                Mutation::SetPageIcon { page, .. } => match titles.get(page) {
                    Some(title) => format!("Changed the icon of {}", quoted(title)),
                    None => "Changed a Page's icon".into(),
                },
                Mutation::AddSource { name, .. } => format!("Added the Source {}", quoted(name)),
                Mutation::RenameSource { name, .. } => {
                    format!("Renamed a Source to {}", quoted(name))
                }
                Mutation::RemoveSource { source } => match self.any_source_name(source) {
                    Some(name) => format!("Removed the Source {}", quoted(&name)),
                    None => "Removed a Source".into(),
                },
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

    pub(crate) fn page_documents(&mut self, page: &PageId) -> Result<Vec<Document>, Error> {
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

    pub(crate) fn reaching(&mut self, ids: &[ActionId]) -> Result<Vec<Action>, Error> {
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

    pub(crate) fn actions(&mut self, ids: &[ActionId]) -> Result<Vec<Action>, Error> {
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

    pub(crate) fn kept(&mut self, action: &Action) -> bool {
        action.changes.iter().all(|(document, hash)| {
            self.doc(document)
                .ok()
                .and_then(|doc| doc.get_change_by_hash(hash))
                .is_some()
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
                let doc = self.doc(document)?;
                let live = crate::source::entries(doc);
                for id in crate::source::all_ids(doc) {
                    let name = live
                        .iter()
                        .find(|source| source.id.as_str() == id)
                        .map(|source| source.name.as_str());
                    index.push(crate::source::source_entry(&SourceId::stored(&id), name));
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
}
