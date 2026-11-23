use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::{Session, Shared, to};
use crate::config::{Store, now_ms};
use crate::handoff::{self, TRANSCRIPT_BYTES};
use crate::host::projects::settled;
use crate::host_move::{
    self, Arrival, ArrivalRow, Departure, MoveFinish, MovePackage, MoveRecord, Saved,
};
use crate::thread::TurnStatus;
use crate::thread::move_out::Saving;
use crate::util::until;
use crate::{Project, Thread};

impl Session {
    pub(super) async fn move_check(&self, thread: &Thread) -> Result<Value, String> {
        let facts = thread.move_facts().await?;
        let state = host_move::state(&self.shared.store.db(), thread.id())?;
        let mut check = host_move::check(&facts, state);
        check.moving = host_move::open_move(&self.shared.store.db(), thread.id())?;
        check.agent = super::agents::move_facts(&self.shared, thread).await?;
        if check.blocked.is_none() {
            check.blocked = check
                .agent
                .as_ref()
                .and_then(|agent| crate::agents::too_large(agent.memory_bytes));
        }
        to(check)
    }

    pub(super) async fn move_out(
        &self,
        thread: &Thread,
        transfer: String,
        target: (String, String, String),
        commit: bool,
        message: Option<String>,
    ) -> Result<Value, String> {
        let shared = self.shared.clone();
        let thread = thread.clone();
        to(detached(
            async move { depart(&shared, &thread, &transfer, &target, commit, message).await },
        )
        .await?)
    }

    pub(super) async fn move_finish(
        &self,
        thread: &Thread,
        transfer: String,
        finish: MoveFinish,
    ) -> Result<Value, String> {
        let shared = self.shared.clone();
        let thread = thread.clone();
        detached(async move { finish_move(&shared, &thread, &transfer, finish).await }).await?;
        to(())
    }

    pub(super) async fn move_in(
        &self,
        root: &str,
        project: &Project,
        package: MovePackage,
    ) -> Result<Value, String> {
        let shared = self.shared.clone();
        let project = project.clone();
        let root = root.to_owned();
        let thread =
            detached(async move { arrive(&shared, &project, &root, package).await }).await?;
        let id = thread.id().to_owned();
        self.keep(thread).await;
        to(id)
    }
}

async fn detached<T: Send + 'static>(
    work: impl Future<Output = Result<T, String>> + Send + 'static,
) -> Result<T, String> {
    match tokio::spawn(work).await {
        Ok(result) => result,
        Err(error) => Err(format!("The move did not finish: {error}")),
    }
}

pub(super) async fn move_status(shared: &Shared, transfer: &str) -> Result<Arrival, String> {
    shared
        .transfers
        .run(transfer, async {
            let row = host_move::arrival(&shared.store.db(), transfer)?;
            Ok(match row {
                Some(row) if row.state == "landed" => {
                    let project = shared.host.project(Path::new(&row.root)).await?;
                    if !project.is_listed(&row.thread).await? {
                        return Ok(Arrival::Unknown);
                    }
                    if delivered(shared, &project, transfer).await? {
                        Arrival::Landed { thread: row.thread }
                    } else {
                        Arrival::Starting
                    }
                }
                Some(row) if row.state == "starting" => Arrival::Starting,
                _ => Arrival::Unknown,
            })
        })
        .await
}

pub(super) async fn move_abort(shared: &Shared, transfer: &str) -> Result<Arrival, String> {
    shared
        .transfers
        .run(transfer, async {
            let row = host_move::arrival(&shared.store.db(), transfer)?;
            if let Some(landed) = row.as_ref().filter(|row| row.state == "landed") {
                let project = shared.host.project(Path::new(&landed.root)).await?;
                if project.is_listed(&landed.thread).await? {
                    return Ok(if delivered(shared, &project, transfer).await? {
                        Arrival::Landed {
                            thread: landed.thread.clone(),
                        }
                    } else {
                        Arrival::Starting
                    });
                }
            }
            let refused = match row {
                Some(row) => ArrivalRow {
                    state: "refused".to_owned(),
                    ..row
                },
                None => ArrivalRow {
                    state: "refused".to_owned(),
                    root: String::new(),
                    thread: String::new(),
                    queued: Vec::new(),
                    delivered: 0,
                },
            };
            host_move::set_arrival(&shared.store.db(), transfer, &refused, now_ms())?;
            withdraw(shared, &refused).await?;
            Ok(Arrival::Refused)
        })
        .await
}

async fn withdraw(shared: &Shared, row: &ArrivalRow) -> Result<(), String> {
    if row.thread.is_empty() {
        return Ok(());
    }
    let project = shared.host.project(Path::new(&row.root)).await?;
    if project.is_listed(&row.thread).await? {
        if let Ok(thread) = project.open(&row.thread).await {
            let _ = thread.stop().await;
        }
        project.discard_id(&row.thread).await?;
    }
    Ok(())
}

const NOT_FINISHED: &str = "The moved messages are not all delivered yet, so the session has not finished moving; try again.";

const DELIVERY_WAIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(100);

enum Entry {
    Stored(MovePackage),
    Run(crate::host_move::Movable),
}

async fn depart(
    shared: &Shared,
    thread: &Thread,
    transfer: &str,
    target: &(String, String, String),
    commit: bool,
    message: Option<String>,
) -> Result<MovePackage, String> {
    shared
        .transfers
        .run(transfer, async {
            depart_once(shared, thread, transfer, target, commit, message).await
        })
        .await
}

async fn depart_once(
    shared: &Shared,
    thread: &Thread,
    transfer: &str,
    target: &(String, String, String),
    commit: bool,
    message: Option<String>,
) -> Result<MovePackage, String> {
    if host_move::phase(&shared.store.db(), transfer)? == Some(host_move::Phase::Preparing) {
        reconcile(thread, &shared.store, transfer).await?;
    }
    let facts = thread.move_facts().await?;
    let fresh = host_move::phase(&shared.store.db(), transfer)?.is_none();
    if fresh
        && let Some(agent) = super::agents::move_facts(shared, thread).await?
        && let Some(reason) = crate::agents::too_large(agent.memory_bytes)
    {
        return Err(reason);
    }
    let entry = {
        let _gate = shared.moves.write().await;
        let db = shared.store.db();
        let earlier = host_move::record(&db, thread.id())?.map(|record| record.transcript);
        match host_move::phase(&db, transfer)? {
            Some(host_move::Phase::Handed) => {
                let text = host_move::package(&db, transfer)?
                    .ok_or_else(|| "The stored package of this move is missing.".to_owned())?;
                Entry::Stored(serde_json::from_str(&text).map_err(|e| e.to_string())?)
            }
            Some(host_move::Phase::Preparing) => {
                Entry::Run(host_move::movable(&facts, None, earlier)?)
            }
            Some(_) => return Err(host_move::MOVE_ENDED.to_owned()),
            None => {
                let state = host_move::state(&db, thread.id())?;
                let movable = host_move::movable(&facts, state, earlier)?;
                host_move::begin(
                    &db,
                    thread.id(),
                    thread.root(),
                    transfer,
                    (&target.0, &target.1, &target.2),
                    now_ms(),
                )?;
                Entry::Run(movable)
            }
        }
    };
    let movable = match entry {
        Entry::Stored(package) => return Ok(package),
        Entry::Run(movable) => movable,
    };
    let store = &shared.store;
    let departure = Departure {
        transfer,
        from_host: &shared.name,
    };
    let save = |turn: u64, text: &str| host_move::enqueue(&store.db(), transfer, turn, text);
    let unsave = |turn: u64| {
        let _ = host_move::forget(&store.db(), transfer, turn);
    };
    let open = || match host_move::phase(&store.db(), transfer)? {
        Some(host_move::Phase::Preparing) => Ok(()),
        _ => Err(host_move::MOVE_ENDED.to_owned()),
    };
    let saving = Saving {
        save: &save,
        unsave: &unsave,
        open: &open,
    };
    let mut package = thread
        .move_out(movable, departure, commit, message, saving)
        .await?;
    package.queued = host_move::queued(&store.db(), transfer)?;
    package.agent = super::agents::carry(shared, thread).await?.map(Box::new);
    let text = serde_json::to_string(&package).map_err(|e| e.to_string())?;
    host_move::hand(&store.db(), transfer, &text, now_ms())?;
    Ok(package)
}

async fn finish_move(
    shared: &Shared,
    thread: &Thread,
    transfer: &str,
    finish: MoveFinish,
) -> Result<(), String> {
    let seal_to = match &finish {
        MoveFinish::Seal { to_host } => Some(to_host.clone()),
        MoveFinish::Release => None,
    };
    shared
        .transfers
        .run(transfer, async {
            {
                let _gate = shared.moves.write().await;
                match &seal_to {
                    Some(to_host) => {
                        host_move::seal(&shared.store.db(), transfer, to_host, now_ms())?
                    }
                    None => return_queued(&shared.store, thread, transfer).await?,
                }
            }
            if seal_to.is_some() {
                let away = super::agents::moved_away(shared, thread).await;
                pause(shared, thread).await;
                away?;
            }
            Ok(())
        })
        .await
}

async fn pause(shared: &Shared, thread: &Thread) {
    match thread.pause_automations().await {
        Ok(()) => {
            let _ = host_move::pause_done(&shared.store.db(), thread.id());
        }
        Err(error) => {
            eprintln!("meta: the moved session's scheduled tasks are not all paused yet: {error}");
        }
    }
}

pub(super) async fn retry_pauses(shared: Arc<Shared>) {
    let pending = match host_move::pauses_pending(&shared.store.db()) {
        Ok(pending) => pending,
        Err(error) => {
            eprintln!("meta: the scheduled-task pauses of moved sessions are not read: {error}");
            return;
        }
    };
    for (thread_id, root) in pending {
        let Ok(project) = shared.host.project(Path::new(&root)).await else {
            continue;
        };
        if !project.is_listed(&thread_id).await.unwrap_or(true) {
            let _ = host_move::pause_done(&shared.store.db(), &thread_id);
            continue;
        }
        if let Ok(thread) = project.open(&thread_id).await {
            pause(&shared, &thread).await;
        }
    }
}

async fn delivered(shared: &Shared, project: &Project, transfer: &str) -> Result<bool, String> {
    if let Err(error) = resume(shared, project, transfer).await {
        eprintln!(
            "meta: the moved messages of a landed session are not all delivered yet: {error}"
        );
    }
    Ok(!host_move::arrival(&shared.store.db(), transfer)?.is_some_and(|row| row.undelivered()))
}

async fn resume(shared: &Shared, project: &Project, transfer: &str) -> Result<(), String> {
    let Some(row) = host_move::arrival(&shared.store.db(), transfer)?
        .filter(|row| row.state == "landed" && row.undelivered())
    else {
        return Ok(());
    };
    let thread = project.open(&row.thread).await?;
    deliver(&shared.store, &thread, transfer).await
}

async fn deliver(store: &Store, thread: &Thread, transfer: &str) -> Result<(), String> {
    let Some(row) = host_move::arrival(&store.db(), transfer)? else {
        return Ok(());
    };
    let done = usize::try_from(row.delivered).unwrap_or(usize::MAX);
    for (index, text) in row.queued.iter().enumerate().skip(done) {
        if let Some(previous) = index.checked_sub(1).map(|earlier| &row.queued[earlier]) {
            start_held(thread, previous).await?;
        }
        thread
            .send_once(&host_move::queued_id(transfer, index), text, false)
            .await?;
        host_move::set_delivered(&store.db(), transfer, index + 1)?;
    }
    Ok(())
}

async fn start_held(thread: &Thread, text: &str) -> Result<(), String> {
    let queued = |thread: &Thread| {
        thread
            .turns()
            .into_iter()
            .find(|turn| turn.prompt == text && turn.status == TurnStatus::Queued)
    };
    let Some(turn) = queued(thread) else {
        return Ok(());
    };
    if thread
        .turns()
        .iter()
        .any(|turn| turn.status == TurnStatus::Running)
    {
        return Ok(());
    }
    thread.send_queued_now(turn.number).await?;
    if until(DELIVERY_WAIT, POLL, || queued(thread).is_none()).await {
        Ok(())
    } else {
        Err(
            "A moved message did not start in time, so the rest of the moved messages wait."
                .to_owned(),
        )
    }
}

async fn arrive(
    shared: &Arc<Shared>,
    project: &Project,
    root: &str,
    package: MovePackage,
) -> Result<Thread, String> {
    let transfer = package.transfer.clone();
    shared.transfers.run(&transfer, async {
        let existing = host_move::arrival(&shared.store.db(), &transfer)?;
        let thread_id = match existing {
            Some(row) if row.state == "refused" => {
                return Err("This transfer was refused, so the session did not move.".to_owned());
            }
            Some(row) if row.state == "landed" => {
                if !project.is_listed(&row.thread).await? {
                    return Err(
                        "The session this transfer landed as is no longer listed here, so the session did not move again."
                            .to_owned(),
                    );
                }
                let thread = project.open(&row.thread).await?;
                if !delivered(shared, project, &transfer).await? {
                    return Err(NOT_FINISHED.to_owned());
                }
                return Ok(thread);
            }
            Some(row) if project.is_listed(&row.thread).await? => {
                let thread = project.open(&row.thread).await?;
                settled(&thread, &package.transfer, &package.handoff).await?;
                return land(shared, root, package, thread).await;
            }
            Some(row) if project.open(&row.thread).await.is_ok() => {
                crate::t3::fresh_id("thread")
            }
            Some(row) => row.thread,
            None => crate::t3::fresh_id("thread"),
        };
        host_move::within_budget(&package)?;
        let sealed = host_move::sealed_threads(&shared.store.db())?;
        let thread = project
            .move_in(&package, &thread_id, &sealed, || {
                let row = ArrivalRow {
                    state: "starting".to_owned(),
                    root: root.to_owned(),
                    thread: thread_id.clone(),
                    queued: Vec::new(),
                    delivered: 0,
                };
                host_move::set_arrival(&shared.store.db(), &transfer, &row, now_ms())
            })
            .await?;
        land(shared, root, package, thread).await
    })
    .await
}

async fn land(
    shared: &Arc<Shared>,
    root: &str,
    package: MovePackage,
    thread: Thread,
) -> Result<Thread, String> {
    let transfer = package.transfer.clone();
    let (transcript, _capped) = handoff::cap_transcript(package.transcript, TRANSCRIPT_BYTES);
    let record = MoveRecord {
        from_host: package.from_host,
        to_host: shared.name.clone(),
        at_ms: now_ms(),
        transcript,
    };
    host_move::put_record(&shared.store.db(), thread.id(), &transfer, &record)?;
    if let Some(carry) = package.agent {
        super::agents::adopt(shared, root, &thread, *carry).await?;
    }
    let landed = ArrivalRow {
        state: "landed".to_owned(),
        root: root.to_owned(),
        thread: thread.id().to_owned(),
        queued: package.queued,
        delivered: 0,
    };
    host_move::set_arrival(&shared.store.db(), &transfer, &landed, now_ms())?;
    if let Err(error) = deliver(&shared.store, &thread, &transfer).await {
        eprintln!(
            "meta: the moved messages of a landed session are not all delivered yet: {error}"
        );
        return Err(NOT_FINISHED.to_owned());
    }
    Ok(thread)
}

async fn reconcile(thread: &Thread, store: &Store, transfer: &str) -> Result<Vec<Saved>, String> {
    let turns = thread.turns();
    let mut kept = Vec::new();
    let saved = host_move::saved(&store.db(), transfer)?;
    for entry in saved {
        let keep = match turns.iter().find(|turn| turn.number == entry.turn) {
            Some(turn) if turn.status == TurnStatus::Queued => {
                thread.cancel_queued(entry.turn).await.is_ok()
            }
            Some(turn) => turn.status == TurnStatus::Aborted && turn.started_ms.is_none(),
            None => false,
        };
        if keep {
            kept.push(entry);
        } else {
            host_move::forget(&store.db(), transfer, entry.turn)?;
        }
    }
    Ok(kept)
}

async fn return_queued(store: &Store, thread: &Thread, transfer: &str) -> Result<(), String> {
    match host_move::phase(&store.db(), transfer)? {
        Some(host_move::Phase::Released) => return Ok(()),
        Some(host_move::Phase::Sealed) => {
            return Err("This move was sealed, so it cannot be released.".to_owned());
        }
        _ => {}
    }
    let kept = reconcile(thread, store, transfer).await?;
    for entry in kept {
        thread
            .requeue_once(&host_move::returned_id(transfer, entry.turn), &entry.text)
            .await?;
        host_move::forget(&store.db(), transfer, entry.turn)?;
    }
    host_move::release(&store.db(), transfer)
}
