mod creation;
mod reclamation;
mod recovery;

use crate::runtime::{stage_results::reconcile_cleanup_failures, types::RuntimeError};

pub(super) use creation::conversation_candidate_is_occupied;
pub(crate) use creation::create_unpublished_productive_conversation_run_with_model_profile;
#[cfg(all(test, unix))]
pub(crate) use creation::set_run_creation_stage_observer;
#[cfg(test)]
pub(crate) use creation::{
    create_conversation_run, create_conversation_run_with_model_profile,
    create_unpublished_productive_conversation_run, set_partial_run_cleanup_observer,
    set_productive_run_creation_observer,
};
pub(super) use reclamation::remove_unpublished_productive_run_marker;
#[cfg(test)]
pub(crate) use reclamation::set_run_sibling_scan_observer;
pub(crate) use reclamation::{reclaim_productive_run_creation, reclaim_unpublished_productive_run};
#[cfg(test)]
pub(crate) use recovery::set_conversation_lifecycle_cleanup_observer;

#[cfg(test)]
type ConversationRootCleanupObserver = Box<dyn FnOnce(&std::path::Path)>;

#[cfg(test)]
std::thread_local! {
    static CONVERSATION_ROOT_CLEANUP_OBSERVER:
        std::cell::RefCell<Option<ConversationRootCleanupObserver>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_conversation_root_cleanup_observer(
    observer: impl FnOnce(&std::path::Path) + 'static,
) {
    CONVERSATION_ROOT_CLEANUP_OBSERVER.with(|slot| slot.replace(Some(Box::new(observer))));
}

#[cfg(test)]
fn observe_conversation_root_cleanup(path: &std::path::Path) {
    CONVERSATION_ROOT_CLEANUP_OBSERVER.with(|slot| {
        if let Some(observer) = slot.replace(None) {
            observer(path);
        }
    });
}

pub(super) fn reconcile_releases(
    run: Result<(), RuntimeError>,
    conversation: Result<(), RuntimeError>,
    legacy: Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    reconcile_release_results([run, conversation, legacy])
}

fn reconcile_release_results(
    results: impl IntoIterator<Item = Result<(), RuntimeError>>,
) -> Result<(), RuntimeError> {
    let failures = results
        .into_iter()
        .filter_map(Result::err)
        .collect::<Vec<_>>();
    reconcile_cleanup_failures(failures)
}
