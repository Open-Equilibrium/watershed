//! The Liquid Workspace for the app, opened from the device's replica: its folder's lock, its
//! key and its four calls, each of which runs on flutter_rust_bridge's worker threads, never on
//! the Dart main isolate. The calls take and answer the messages of [`super::workspace`].

use std::fs::{File, TryLockError};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use liquid_core::{
    Actor, AreaId, BlockId, BlockKind, Created, Error, Mutation, Origin, PageId, PersonId, Request,
    Workspace,
};
use liquid_storage::Sqlite;

use super::database::{BridgeDatabase, BridgeRow, row_query};
use super::workspace::{
    BridgeAnswer, BridgeApplied, BridgeArea, BridgeBlockKind, BridgeChange, BridgeMutation,
    BridgePage, BridgeQuery, BridgeQueryKind, BridgeResource, BridgeTextStyle, BridgeView, id,
    message, window,
};
use crate::frb_generated::StreamSink;
use crate::keys::{Keychain, Keys, random_hex, replica_key};
use crate::subscriptions::Subscriptions;

/// The device's Workspace, opened from its replica.
#[flutter_rust_bridge::frb(opaque)]
pub struct LiquidWorkspace {
    inner: Mutex<Inner>,
    /// The replica folder's lock, held while the Workspace is open.
    _lock: File,
}

struct Inner {
    workspace: Workspace<Sqlite>,
    subscriptions: Subscriptions,
}

impl Drop for LiquidWorkspace {
    fn drop(&mut self) {
        // A concurrently spawned child may still hold an inherited descriptor until exec.
        let _ = self._lock.unlock();
    }
}

/// Opens the replica folder's lock file without truncating it, and refuses one that is a
/// link or no plain file, so opening never writes to a file elsewhere.
fn open_lock(path: &Path) -> Result<File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(
        &mut options,
        rustix::fs::OFlags::NOFOLLOW.bits() as i32,
    );
    // FILE_FLAG_OPEN_REPARSE_POINT: opens a link itself, which the plain-file check refuses.
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

/// How long a held replica lock is tried again: a process this one starts holds the lock
/// it inherited only until it runs its program, while another session holds it for good.
const LOCK_PATIENCE: std::time::Duration = std::time::Duration::from_millis(500);

/// Takes the replica's lock, trying again for a moment while it is held.
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

impl LiquidWorkspace {
    /// Opens the Workspace whose replica lives in the folder `dir`, creating both on first
    /// use. The replica's key lives in the operating system's keychain (ADR-0237).
    pub fn open(dir: String) -> Result<LiquidWorkspace, String> {
        let dir = Path::new(&dir);
        // Created first, so the folder's canonical path is known when an older key is looked up.
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        Self::open_with(dir, &Keychain)
    }

    /// Opens with `keys` in place of the keychain, for tests.
    #[flutter_rust_bridge::frb(ignore)]
    pub(crate) fn open_with(dir: &Path, keys: &dyn Keys) -> Result<LiquidWorkspace, String> {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        // Taken before the key is read or made, so two sessions opening a new folder at once
        // cannot each store a key of their own.
        let lock = open_lock(&dir.join("replica.lock"))?;
        take_lock(&lock)?;
        let path = dir.join("replica.db");
        let key = replica_key(dir, keys, path.exists())?;
        let workspace = match Workspace::open(Sqlite::open(&path, &key).map_err(message)?) {
            Ok(workspace) => workspace,
            // A new replica: until accounts exist, the device's person creates and owns it.
            Err(Error::NotFound) => {
                let store = Sqlite::open(&path, &key).map_err(message)?;
                Workspace::create(PersonId::new(random_hex()?), store).map_err(message)?
            }
            Err(error) => return Err(message(error)),
        };
        Ok(Self {
            inner: Mutex::new(Inner {
                workspace,
                subscriptions: Subscriptions::default(),
            }),
            _lock: lock,
        })
    }

    /// Applies one write as the device's person through Liquid's interface, then tells the
    /// subscribers of every resource it changed.
    pub fn apply(&self, mutation: BridgeMutation) -> Result<BridgeApplied, String> {
        let mutation = mutation.to_core()?;
        let mutation_page = match &mutation {
            Mutation::CreateArea { .. }
            | Mutation::CreatePage { .. }
            | Mutation::RenamePage { .. } => None,
            Mutation::AddTextBlock { page, .. }
            | Mutation::EditText { page, .. }
            | Mutation::RemoveView { page, .. }
            | Mutation::SplitTextBlock { page, .. }
            | Mutation::JoinTextBlock { page, .. }
            | Mutation::SetTextStyle { page, .. }
            | Mutation::SetSectionBoundary { page, .. }
            | Mutation::AddDatabaseBlock { page, .. }
            | Mutation::RenameDatabase { page, .. }
            | Mutation::AddField { page, .. }
            | Mutation::RenameField { page, .. }
            | Mutation::RemoveField { page, .. }
            | Mutation::AddOption { page, .. }
            | Mutation::AddRow { page, .. }
            | Mutation::RemoveRow { page, .. }
            | Mutation::SetCell { page, .. } => Some(page.clone()),
        };
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let actor = Actor::Person(inner.workspace.owner().clone());
        let result = inner.workspace.apply(Request {
            actor,
            origin: Origin::Ui,
            mutation,
        });
        // A refused write may have loaded its Page too.
        let applied = match result {
            Ok(applied) => applied,
            Err(error) => {
                if let Some(page) = &mutation_page {
                    inner.release_page(page);
                }
                return Err(message(error));
            }
        };
        for changed in &applied.changed {
            inner.notify(BridgeResource::of(changed));
        }
        if let Some(page) = mutation_page.as_ref().or(match &applied.created {
            Some(Created::Page(page)) => Some(page),
            _ => None,
        }) {
            inner.release_page(page);
        }
        let mut answer = BridgeApplied {
            action: applied.action.to_string(),
            area: None,
            page: None,
            block: None,
            view: None,
            field: None,
            row: None,
            option: None,
        };
        match applied.created {
            Some(Created::Area(area)) => answer.area = Some(area.to_string()),
            Some(Created::Page(page)) => answer.page = Some(page.to_string()),
            Some(Created::Block { block, view }) => {
                answer.block = Some(block.to_string());
                answer.view = Some(view.to_string());
            }
            Some(Created::Field(field)) => answer.field = Some(field.to_string()),
            Some(Created::Row(row)) => answer.row = Some(row.to_string()),
            Some(Created::Option(option)) => answer.option = Some(option.to_string()),
            None => {}
        }
        Ok(answer)
    }

    /// Reads one bounded window, one Block's text or one Database's fields.
    pub fn query(&self, query: BridgeQuery) -> Result<BridgeAnswer, String> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let (start, limit) = window(query.start, query.limit)?;
        let mut read = None;
        let result = Self::read(&mut inner.workspace, &query, start, limit, &mut read);
        if let Some(page) = read {
            inner.release_page(&page);
        }
        result
    }

    fn read(
        workspace: &mut Workspace<Sqlite>,
        query: &BridgeQuery,
        start: usize,
        limit: usize,
        read: &mut Option<PageId>,
    ) -> Result<BridgeAnswer, String> {
        let mut answer = BridgeAnswer {
            areas: Vec::new(),
            pages: Vec::new(),
            views: Vec::new(),
            count: 0,
            text: None,
            style: None,
            database: None,
            rows: Vec::new(),
        };
        match query.kind {
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
                *read = Some(page.clone());
                answer.count = workspace.view_count(&page).map_err(message)? as i64;
                answer.views = workspace
                    .views(&page, start, limit)
                    .map_err(message)?
                    .into_iter()
                    .map(|view| BridgeView {
                        id: view.id.to_string(),
                        block: view.block.to_string(),
                        boundary: view.boundary,
                        kind: match view.kind {
                            BlockKind::Text => BridgeBlockKind::Text,
                            BlockKind::Database => BridgeBlockKind::Database,
                            BlockKind::Unknown => BridgeBlockKind::Unknown,
                        },
                    })
                    .collect();
            }
            BridgeQueryKind::Text => {
                let page = id(&query.page, PageId::parse, "page")?;
                *read = Some(page.clone());
                let block = id(&query.block, BlockId::parse, "block")?;
                answer.text = workspace.text(&page, &block).map_err(message)?;
                answer.style = workspace
                    .style(&page, &block)
                    .map_err(message)?
                    .map(BridgeTextStyle::of);
            }
            BridgeQueryKind::Database => {
                let page = id(&query.page, PageId::parse, "page")?;
                *read = Some(page.clone());
                let block = id(&query.block, BlockId::parse, "block")?;
                answer.database = workspace
                    .database(&page, &block)
                    .map_err(message)?
                    .map(BridgeDatabase::of);
            }
            BridgeQueryKind::Rows => {
                let page = id(&query.page, PageId::parse, "page")?;
                *read = Some(page.clone());
                let block = id(&query.block, BlockId::parse, "block")?;
                let rows = row_query(&query.sort, &query.filter, start, limit)?;
                let rows = workspace.rows(&page, &block, &rows).map_err(message)?;
                answer.count = rows.count as i64;
                answer.rows = rows.rows.into_iter().map(BridgeRow::of).collect();
            }
        }
        Ok(answer)
    }

    /// Streams `resource`'s changes to Dart under the identity `subscription`, which Dart
    /// chooses and passes to [`LiquidWorkspace::close`] when it cancels the stream. The first
    /// change carries the resource's current count and marks the registration: a reader queries
    /// after it, so it misses no later change.
    pub fn subscribe(
        &self,
        subscription: i64,
        resource: BridgeResource,
        sink: StreamSink<BridgeChange>,
    ) -> Result<(), String> {
        // Dart's stream sees only what the sink carries, so a refusal travels through it.
        let refused = sink.clone();
        let watched = self.watch(subscription, resource, move |change| {
            sink.add(change).is_ok()
        });
        if let Err(error) = watched {
            let _ = refused.add_error(error);
        }
        Ok(())
    }

    /// Hands `resource`'s changes to `deliver` until it returns `false`; tests use it instead
    /// of a Dart stream. `deliver` runs while the Workspace is locked, so it must not call
    /// the Workspace, as a stream sink never does.
    #[flutter_rust_bridge::frb(ignore)]
    pub(crate) fn watch(
        &self,
        subscription: i64,
        resource: BridgeResource,
        deliver: impl FnMut(BridgeChange) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner
            .subscriptions
            .follow(subscription, resource.normalized(), Box::new(deliver))
    }

    /// Ends a subscription, also one whose registration has not arrived yet. A Page that no
    /// subscription follows any more leaves memory.
    pub fn close(&self, subscription: i64) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(resource) = inner.subscriptions.close(subscription) {
            inner.release(&resource);
        }
    }
}

#[cfg(test)]
impl LiquidWorkspace {
    pub(crate) fn is_loaded(&self, page: &PageId) -> bool {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.workspace.is_loaded(page)
    }

    /// How many subscription identities wait for their other end: a registration or a close.
    pub(crate) fn lifecycle_markers(&self) -> usize {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.subscriptions.lifecycle_markers()
    }
}

impl Inner {
    /// Counts a change of `resource` and tells its subscribers; a Page whose last subscriber's
    /// Dart stream is gone leaves memory.
    fn notify(&mut self, resource: BridgeResource) {
        for gone in self.subscriptions.notify(resource) {
            self.release(&gone);
        }
    }

    /// Closes the Page `resource` reads once no other subscription follows it.
    fn release(&mut self, resource: &BridgeResource) {
        if let Some(page) = resource.page().and_then(PageId::parse) {
            self.release_page(&page);
        }
    }

    /// Closes `page` unless a subscription follows it, so a Page that a call loaded for
    /// nobody does not stay in memory.
    fn release_page(&mut self, page: &PageId) {
        if !self.subscriptions.follows(page) {
            self.workspace.close_page(page);
        }
    }
}
