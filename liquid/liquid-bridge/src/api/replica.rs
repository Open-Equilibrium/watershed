use std::fs::{File, TryLockError};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use liquid_core::{
    Actor, Applied, AreaId, AutomationId, BlockId, BlockKind, Created, Document, Error, FieldId,
    Found, Origin, PageId, PersonId, Ran, Request, RowId, RowQuery, Workspace,
};
use liquid_storage::Sqlite;

use crate::replica_store::Replica;

use super::automation::{BridgeRan, BridgeWorkspaceAutomation};
use super::calculation::BridgeLine;
use super::chart::BridgeChart;
use super::database::{
    BridgeCell, BridgeDatabase, BridgeGroup, BridgeRow, BridgeTab, computed, filter, sort, totals,
};
use super::drawing::BridgeDrawing;
use super::media::BridgeMedia;
use super::role::{BridgeAllows, BridgeRole, BridgeRoleAllows};
use super::workspace::{
    BridgeAnswer, BridgeApplied, BridgeArea, BridgeBlockKind, BridgeChange, BridgeFound, BridgeHit,
    BridgeMutation, BridgePage, BridgeQuery, BridgeQueryKind, BridgeResource, BridgeShown,
    BridgeTable, BridgeTextStyle, BridgeTile, BridgeView, follows, id, message, window,
};
use crate::frb_generated::StreamSink;
use crate::keys::{Keychain, Keys, random_hex, replica_key};
use crate::subscriptions::Subscriptions;

#[flutter_rust_bridge::frb(opaque)]
pub struct LiquidWorkspace {
    inner: Mutex<Inner>,
    _lock: Option<ReplicaLock>,
    read_only: bool,
}

struct ReplicaLock(File);

pub(super) struct Inner {
    pub(super) workspace: Workspace<Replica>,
    subscriptions: Subscriptions,
}

impl Drop for ReplicaLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn open_lock(path: &Path) -> Result<File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(
        &mut options,
        rustix::fs::OFlags::NOFOLLOW.bits() as i32,
    );
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::custom_flags(&mut options, 0x0020_0000);
    let lock = options
        .open(path)
        .map_err(|error| format!("the replica lock {}: {error}", path.display()))?;
    let plain = lock
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file();
    if !plain {
        return Err(format!(
            "the replica lock {} is no plain file",
            path.display()
        ));
    }
    Ok(lock)
}

const LOCK_PATIENCE: std::time::Duration = std::time::Duration::from_millis(500);

fn take_lock(lock: &File) -> Result<(), String> {
    let since = std::time::Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if since.elapsed() < LOCK_PATIENCE => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(TryLockError::WouldBlock) => {
                return Err("another Liquid session holds this replica".into());
            }
            Err(TryLockError::Error(error)) => return Err(error.to_string()),
        }
    }
}

const STOPPED: &str = "the Workspace stopped after an internal error; quit and reopen Liquid";

impl LiquidWorkspace {
    pub(super) fn state(&self) -> Result<MutexGuard<'_, Inner>, String> {
        self.inner.lock().map_err(|_| STOPPED.to_string())
    }

    pub fn open(dir: String) -> Result<LiquidWorkspace, String> {
        let dir = Path::new(&dir);
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        Self::open_with(dir, &Keychain)
    }

    #[flutter_rust_bridge::frb(ignore)]
    pub(crate) fn open_with(dir: &Path, keys: &dyn Keys) -> Result<LiquidWorkspace, String> {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        let lock = open_lock(&dir.join("replica.lock"))?;
        take_lock(&lock)?;
        let path = dir.join("replica.db");
        let key = replica_key(dir, keys, path.exists())?;
        let disk = Replica::Disk(Sqlite::open(&path, &key).map_err(message)?);
        let workspace = match Workspace::open(disk) {
            Ok(workspace) => workspace,
            Err(Error::NotFound) => {
                let store = Replica::Disk(Sqlite::open(&path, &key).map_err(message)?);
                Workspace::create(PersonId::new(random_hex()?), store).map_err(message)?
            }
            Err(error) => return Err(message(error)),
        };
        Ok(Self {
            inner: Mutex::new(Inner {
                workspace,
                subscriptions: Subscriptions::default(),
            }),
            _lock: Some(ReplicaLock(lock)),
            read_only: false,
        })
    }

    #[flutter_rust_bridge::frb(ignore)]
    pub(crate) fn version(documents: Vec<(Document, Vec<u8>)>) -> Result<LiquidWorkspace, String> {
        let store = Replica::Version(liquid_core::MemoryStore::holding(documents));
        Ok(Self {
            inner: Mutex::new(Inner {
                workspace: Workspace::open(store).map_err(message)?,
                subscriptions: Subscriptions::default(),
            }),
            _lock: None,
            read_only: true,
        })
    }

    #[flutter_rust_bridge::frb(ignore)]
    pub(crate) fn change(
        &self,
        change: impl FnOnce(&mut Workspace<Replica>, &Actor) -> Result<Applied, Error>,
    ) -> Result<String, Error> {
        if self.read_only {
            return Err(Error::NotAllowed);
        }
        let mut inner = self.state().map_err(Error::Storage)?;
        let actor = Actor::Person(inner.workspace.owner().clone());
        let applied = change(&mut inner.workspace, &actor);
        if let Ok(applied) = &applied {
            for changed in &applied.changed {
                inner.notify(BridgeResource::of(changed));
            }
        }
        inner.release_unfollowed();
        applied.map(|applied| applied.action.to_string())
    }

    #[flutter_rust_bridge::frb(ignore)]
    pub(crate) fn change_maybe(
        &self,
        change: impl FnOnce(&mut Workspace<Replica>, &Actor) -> Result<Option<Applied>, Error>,
    ) -> Result<bool, String> {
        let mut done = false;
        let outcome = self.change(|workspace, actor| match change(workspace, actor)? {
            Some(applied) => {
                done = true;
                Ok(applied)
            }
            None => Err(Error::NoChange),
        });
        match outcome {
            Ok(_) => Ok(done),
            Err(Error::NoChange) => Ok(false),
            Err(error) => Err(message(error)),
        }
    }

    pub fn apply(&self, mutation: BridgeMutation) -> Result<BridgeApplied, String> {
        if self.read_only {
            return Err("a version is read-only".into());
        }
        let mutation = mutation.to_core()?;
        let mut inner = self.state()?;
        let actor = Actor::Person(inner.workspace.owner().clone());
        let result = inner.workspace.apply(Request {
            actor,
            origin: Origin::Ui,
            mutation,
        });
        let applied = match result {
            Ok(applied) => applied,
            Err(error) => {
                inner.release_unfollowed();
                return Err(message(error));
            }
        };
        for changed in applied
            .changed
            .iter()
            .chain(applied.ran.iter().flat_map(|ran| &ran.changed))
        {
            inner.notify(BridgeResource::of(changed));
        }
        inner.release_unfollowed();
        let mut answer = BridgeApplied {
            action: applied.action.to_string(),
            area: None,
            page: None,
            block: None,
            view: None,
            field: None,
            row: None,
            option: None,
            tab: None,
            elements: Vec::new(),
            line: None,
            automation: None,
            role: None,
            ran: applied.ran.into_iter().map(BridgeRan::of).collect(),
        };
        match applied.created {
            Some(Created::Area(area)) => answer.area = Some(area.to_string()),
            Some(Created::Page(page)) => answer.page = Some(page.to_string()),
            Some(Created::Block { block, view }) => {
                answer.block = Some(block.to_string());
                answer.view = Some(view.to_string());
            }
            Some(Created::Table(table)) => answer.block = Some(table.to_string()),
            Some(Created::Field(field)) => answer.field = Some(field.to_string()),
            Some(Created::Row(row)) => answer.row = Some(row.to_string()),
            Some(Created::Option(option)) => answer.option = Some(option.to_string()),
            Some(Created::Tab(tab)) => answer.tab = Some(tab.to_string()),
            Some(Created::Line(line)) => answer.line = Some(line.to_string()),
            Some(Created::Automation(automation)) => {
                answer.automation = Some(automation.to_string());
            }
            Some(Created::Role(role)) => answer.role = Some(role.to_string()),
            Some(Created::Elements(elements)) => {
                answer.elements = elements.iter().map(ToString::to_string).collect();
            }
            None => {}
        }
        Ok(answer)
    }

    pub fn press(
        &self,
        table: String,
        row: String,
        field: String,
    ) -> Result<Vec<BridgeRan>, String> {
        let table = id(&Some(table), BlockId::parse, "table")?;
        let row = id(&Some(row), RowId::parse, "row")?;
        let field = id(&Some(field), FieldId::parse, "field")?;
        let mut inner = self.state()?;
        let actor = Actor::Person(inner.workspace.owner().clone());
        let ran = inner.workspace.press(&actor, &table, &row, &field);
        inner.reported(ran)
    }

    pub fn test(&self, automation: String) -> Result<BridgeRan, String> {
        let automation = id(&Some(automation), AutomationId::parse, "automation")?;
        let mut inner = self.state()?;
        let actor = Actor::Person(inner.workspace.owner().clone());
        let ran = inner
            .workspace
            .test(&actor, &automation)
            .map(|ran| vec![ran]);
        Ok(inner.reported(ran)?.remove(0))
    }

    pub fn remembered(&self, key: String) -> Result<Option<String>, String> {
        self.state()?.workspace.remembered(&key).map_err(message)
    }

    pub fn remember(&self, key: String, value: Option<String>) -> Result<(), String> {
        let mut inner = self.state()?;
        inner
            .workspace
            .remember(&key, value.as_deref())
            .map_err(message)
    }

    pub fn query(&self, query: BridgeQuery) -> Result<BridgeAnswer, String> {
        let mut inner = self.state()?;
        let (start, limit) = window(query.start, query.limit)?;
        let result = Self::read(&mut inner.workspace, &query, start, limit);
        inner.release_unfollowed();
        result
    }

    fn read(
        workspace: &mut Workspace<Replica>,
        query: &BridgeQuery,
        start: usize,
        limit: usize,
    ) -> Result<BridgeAnswer, String> {
        let mut answer = BridgeAnswer {
            areas: Vec::new(),
            pages: Vec::new(),
            views: Vec::new(),
            count: 0,
            arranged: false,
            text: None,
            style: None,
            language: None,
            media: None,
            drawing: None,
            database: None,
            rows: Vec::new(),
            groups: Vec::new(),
            hits: Vec::new(),
            tables: Vec::new(),
            shown: Vec::new(),
            icon: None,
            lines: Vec::new(),
            totals: Vec::new(),
            chart: None,
            follows: Vec::new(),
            automations: Vec::new(),
            roles: Vec::new(),
            allows: Vec::new(),
        };
        match query.kind {
            BridgeQueryKind::Workspace => {
                let (name, icon) = workspace.name_and_icon().map_err(message)?;
                answer.text = Some(name);
                answer.icon = Some(icon);
            }
            BridgeQueryKind::Areas => {
                let areas = workspace.areas().map_err(message)?;
                answer.count = areas.len() as i64;
                answer.areas = areas
                    .into_iter()
                    .skip(start)
                    .take(limit)
                    .map(|area| BridgeArea {
                        id: area.id.to_string(),
                        name: area.name,
                    })
                    .collect();
            }
            BridgeQueryKind::Pages => {
                let pages = workspace
                    .pages(&id(&query.area, AreaId::parse, "area")?)
                    .map_err(message)?;
                answer.count = pages.len() as i64;
                answer.pages = pages
                    .into_iter()
                    .skip(start)
                    .take(limit)
                    .map(|page| BridgePage {
                        id: page.id.to_string(),
                        parent: page.parent.map(|parent| parent.to_string()),
                        title: page.title,
                    })
                    .collect();
            }
            BridgeQueryKind::Views => {
                let page = id(&query.page, PageId::parse, "page")?;
                answer.count = workspace.view_count(&page).map_err(message)? as i64;
                answer.arranged = workspace.arranged(&page).map_err(message)?;
                answer.views = workspace
                    .views(&page, start, limit)
                    .map_err(message)?
                    .into_iter()
                    .map(|view| BridgeView {
                        id: view.id.to_string(),
                        block: view.block.to_string(),
                        title: view.title,
                        boundary: view.boundary,
                        kind: match view.kind {
                            BlockKind::Text => BridgeBlockKind::Text,
                            BlockKind::Database => BridgeBlockKind::Database,
                            BlockKind::Code => BridgeBlockKind::Code,
                            BlockKind::Media => BridgeBlockKind::Media,
                            BlockKind::Whiteboard => BridgeBlockKind::Whiteboard,
                            BlockKind::Calculation => BridgeBlockKind::Calculation,
                            BlockKind::Chart => BridgeBlockKind::Chart,
                            BlockKind::Math => BridgeBlockKind::Math,
                            BlockKind::Unknown => BridgeBlockKind::Unknown,
                        },
                        tabs: view.tabs.iter().map(BridgeTab::of).collect(),
                        tile: view.tile.map(BridgeTile::of),
                        inner: view.inner.map(BridgeTile::of),
                    })
                    .collect();
            }
            BridgeQueryKind::Text => {
                let page = id(&query.page, PageId::parse, "page")?;
                let block = id(&query.block, BlockId::parse, "block")?;
                answer.text = workspace.text(&page, &block).map_err(message)?;
                answer.style = workspace
                    .style(&page, &block)
                    .map_err(message)?
                    .map(BridgeTextStyle::of);
            }
            BridgeQueryKind::Code => {
                let page = id(&query.page, PageId::parse, "page")?;
                let block = id(&query.block, BlockId::parse, "block")?;
                if let Some(code) = workspace.code(&page, &block).map_err(message)? {
                    answer.text = Some(code.text);
                    answer.language = Some(code.language);
                }
            }
            BridgeQueryKind::Math => {
                let page = id(&query.page, PageId::parse, "page")?;
                let block = id(&query.block, BlockId::parse, "block")?;
                answer.text = workspace.math(&page, &block).map_err(message)?;
            }
            BridgeQueryKind::Media => {
                let page = id(&query.page, PageId::parse, "page")?;
                let block = id(&query.block, BlockId::parse, "block")?;
                answer.media = workspace
                    .media(&page, &block)
                    .map_err(message)?
                    .map(BridgeMedia::of);
            }
            BridgeQueryKind::Drawing => {
                let page = id(&query.page, PageId::parse, "page")?;
                let block = id(&query.block, BlockId::parse, "block")?;
                let pages = u32::try_from(start).unwrap_or(u32::MAX)
                    ..u32::try_from(start.saturating_add(limit)).unwrap_or(u32::MAX);
                answer.drawing = workspace
                    .drawing(&page, &block, pages)
                    .map_err(message)?
                    .map(BridgeDrawing::of);
            }
            BridgeQueryKind::Calculation => {
                let page = id(&query.page, PageId::parse, "page")?;
                let block = id(&query.block, BlockId::parse, "block")?;
                if let Some(calculation) = workspace.calculation(&page, &block).map_err(message)? {
                    answer.follows = follows(&calculation.uses);
                    answer.lines = calculation.lines.into_iter().map(BridgeLine::of).collect();
                }
            }
            BridgeQueryKind::Chart => {
                let page = id(&query.page, PageId::parse, "page")?;
                let block = id(&query.block, BlockId::parse, "block")?;
                if let Some(chart) = workspace.chart(&page, &block).map_err(message)? {
                    answer.follows = follows(&chart.uses);
                    answer.chart = Some(BridgeChart::of(chart));
                }
            }
            BridgeQueryKind::Database => {
                let block = id(&query.block, BlockId::parse, "block")?;
                answer.database = workspace
                    .database(&block)
                    .map_err(message)?
                    .map(BridgeDatabase::of);
            }
            BridgeQueryKind::Rows => {
                let block = id(&query.block, BlockId::parse, "block")?;
                let rows = workspace
                    .rows(&block, &row_query(query, start, limit)?)
                    .map_err(message)?;
                answer.totals = computed(&rows.totals);
                answer.follows = follows(&rows.uses);
                answer.count = rows.count as i64;
                answer.rows = rows.rows.into_iter().map(BridgeRow::of).collect();
            }
            BridgeQueryKind::Search => {
                let text = query.search.as_deref().unwrap_or_default();
                for (found, shown) in [
                    (Found::Page, BridgeFound::Page),
                    (Found::Table, BridgeFound::Table),
                    (Found::Text, BridgeFound::Text),
                ] {
                    for hit in workspace.search(text, found, limit).map_err(message)? {
                        answer.hits.push(BridgeHit {
                            found: shown,
                            id: hit.id,
                            page: hit.page.map(|page| page.to_string()),
                            text: hit.text,
                        });
                    }
                }
            }
            BridgeQueryKind::Groups => {
                let block = id(&query.block, BlockId::parse, "block")?;
                let field = id(&query.field, FieldId::parse, "field")?;
                answer.groups = workspace
                    .groups(&block, &row_query(query, start, limit)?, &field)
                    .map_err(message)?
                    .into_iter()
                    .map(|group| BridgeGroup {
                        value: BridgeCell::of(&group.value),
                        count: group.count as i64,
                        totals: computed(&group.totals),
                        rows: group.rows.into_iter().map(BridgeRow::of).collect(),
                    })
                    .collect();
            }
            BridgeQueryKind::Tables => {
                let tables = workspace.tables().map_err(message)?;
                answer.count = tables.len() as i64;
                answer.tables = tables
                    .into_iter()
                    .skip(start)
                    .take(limit)
                    .map(|table| BridgeTable {
                        id: table.id.to_string(),
                        area: table.area.to_string(),
                        name: table.name,
                        protected: table.protected,
                    })
                    .collect();
            }
            BridgeQueryKind::Automations => {
                let automations = workspace.automations().map_err(message)?;
                answer.count = automations.len() as i64;
                answer.automations = automations
                    .into_iter()
                    .skip(start)
                    .take(limit)
                    .map(BridgeWorkspaceAutomation::of)
                    .collect();
            }
            BridgeQueryKind::Roles => {
                let roles = workspace.roles().map_err(message)?;
                answer.count = roles.len() as i64;
                answer.roles = roles.into_iter().map(BridgeRole::of).collect();
            }
            BridgeQueryKind::Allows => {
                let block = id(&query.block, BlockId::parse, "block")?;
                for role in workspace.roles().map_err(message)? {
                    let allows = workspace.allows(&role.id, &block).map_err(message)?;
                    answer.allows.push(BridgeRoleAllows {
                        role: role.id.to_string(),
                        allows: BridgeAllows::of(allows),
                    });
                }
            }
            BridgeQueryKind::TableViews => {
                let block = id(&query.block, BlockId::parse, "block")?;
                let shown = workspace.table_views(&block).map_err(message)?;
                answer.count = shown.len() as i64;
                let mut listed = Vec::new();
                for (page, view) in shown.into_iter().skip(start).take(limit) {
                    let Some(shown) = workspace.view(&page, &view).map_err(message)? else {
                        continue;
                    };
                    listed.push(BridgeShown {
                        page: page.to_string(),
                        view: view.to_string(),
                        title: shown.title,
                        tabs: shown.tabs.iter().map(BridgeTab::of).collect(),
                    });
                }
                answer.shown = listed;
            }
        }
        Ok(answer)
    }

    pub fn subscribe(
        &self,
        subscription: i64,
        resource: BridgeResource,
        sink: StreamSink<BridgeChange>,
    ) -> Result<(), String> {
        let refused = sink.clone();
        let watched = self.watch(subscription, resource, move |change| {
            sink.add(change).is_ok()
        });
        if let Err(error) = watched {
            let _ = refused.add_error(error);
        }
        Ok(())
    }

    #[flutter_rust_bridge::frb(ignore)]
    pub(crate) fn watch(
        &self,
        subscription: i64,
        resource: BridgeResource,
        deliver: impl FnMut(BridgeChange) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let mut inner = self.state()?;
        inner
            .subscriptions
            .follow(subscription, resource.normalized(), Box::new(deliver))
    }

    pub fn close(&self, subscription: i64) {
        let Ok(mut inner) = self.state() else {
            return;
        };
        if inner.subscriptions.close(subscription).is_some() {
            inner.release_unfollowed();
        }
    }
}

#[cfg(test)]
impl LiquidWorkspace {
    pub(crate) fn is_loaded(&self, page: &PageId) -> bool {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.workspace.is_loaded(page)
    }

    pub(crate) fn lifecycle_markers(&self) -> usize {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.subscriptions.lifecycle_markers()
    }
}

impl Inner {
    fn reported(&mut self, ran: Result<Vec<Ran>, Error>) -> Result<Vec<BridgeRan>, String> {
        let ran = match ran {
            Ok(ran) => ran,
            Err(error) => {
                self.release_unfollowed();
                return Err(message(error));
            }
        };
        for changed in ran.iter().flat_map(|ran| &ran.changed) {
            self.notify(BridgeResource::of(changed));
        }
        self.release_unfollowed();
        Ok(ran.into_iter().map(BridgeRan::of).collect())
    }

    fn notify(&mut self, resource: BridgeResource) {
        if !self.subscriptions.notify(resource).is_empty() {
            self.release_unfollowed();
        }
    }

    fn release_unfollowed(&mut self) {
        let subscriptions = &self.subscriptions;
        self.workspace.retain_loaded(|document| match document {
            Document::Page(page) => subscriptions.follows(page),
            Document::Table(table) => subscriptions.follows_table(table),
            _ => true,
        });
    }
}

fn row_query(query: &BridgeQuery, start: usize, limit: usize) -> Result<RowQuery, String> {
    if let Some(search) = &query.search
        && search.chars().count() > liquid_core::MAX_FILTER_TEXT
    {
        return Err("the search is too long".into());
    }
    Ok(RowQuery {
        sort: sort(&query.sort)?,
        filter: filter(&query.filter)?,
        within: filter(&query.within)?,
        start,
        limit,
        search: query.search.clone(),
        totals: totals(&query.totals)?,
    })
}
