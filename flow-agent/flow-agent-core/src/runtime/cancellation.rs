use crate::runtime::types::RuntimeError;
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};

static PRODUCTIVE_OPERATION: ProductiveOperationCoordinator = ProductiveOperationCoordinator::new();

/// Result of linearizing one Unix interrupt against the productive-operation state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductiveInterruptAction {
    /// The interrupt won and controlled cancellation must begin.
    Cancel,
    /// Durable completion already won, so the interrupt is deferred until disarm.
    Defer,
    /// No cancellable operation exists, or cancellation is already in progress.
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProductiveTerminalClaim {
    Cancellation,
    Completion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProductiveOperationState {
    Idle,
    Active,
    PublishingRun,
    CommittingRun,
    Dispatching,
    CommittingDurableState,
    Cancelling,
    Completing,
}

struct ProductiveOperationCoordinator {
    state: Mutex<ProductiveOperationState>,
    cancelled: AtomicBool,
    deferred_interrupt: AtomicBool,
}

pub(crate) struct ProductiveEffectDispatch<'a> {
    coordinator: &'a ProductiveOperationCoordinator,
    claimed: bool,
}

pub(crate) struct ProductiveDurableCommit<'a> {
    coordinator: &'a ProductiveOperationCoordinator,
    claimed: bool,
}

pub(crate) struct ProductiveRunPublication<'a> {
    coordinator: &'a ProductiveOperationCoordinator,
    claimed: bool,
}

pub(crate) struct ProductiveRunPublicationCommit<'a> {
    coordinator: &'a ProductiveOperationCoordinator,
    claimed: bool,
}

impl Drop for ProductiveEffectDispatch<'_> {
    fn drop(&mut self) {
        if self.claimed {
            self.coordinator
                .release_operation(ProductiveOperationState::Dispatching);
        }
    }
}

impl Drop for ProductiveDurableCommit<'_> {
    fn drop(&mut self) {
        if self.claimed {
            self.coordinator
                .release_operation(ProductiveOperationState::CommittingDurableState);
        }
    }
}

impl Drop for ProductiveRunPublication<'_> {
    fn drop(&mut self) {
        if self.claimed {
            self.coordinator
                .release_operation(ProductiveOperationState::PublishingRun);
        }
    }
}

impl<'a> ProductiveRunPublication<'a> {
    pub(crate) fn commit(mut self) -> Result<ProductiveRunPublicationCommit<'a>, RuntimeError> {
        let claimed = self.coordinator.commit_run_creation_publication()?;
        self.claimed = false;
        Ok(ProductiveRunPublicationCommit {
            coordinator: self.coordinator,
            claimed,
        })
    }
}

impl Drop for ProductiveRunPublicationCommit<'_> {
    fn drop(&mut self) {
        if self.claimed {
            self.coordinator
                .release_operation(ProductiveOperationState::CommittingRun);
        }
    }
}

impl ProductiveRunPublicationCommit<'_> {
    pub(crate) fn finish(mut self) -> Result<(), RuntimeError> {
        let result = self.coordinator.finish_run_creation_publication();
        self.claimed = false;
        result
    }
}

impl ProductiveOperationCoordinator {
    const fn new() -> Self {
        Self {
            state: Mutex::new(ProductiveOperationState::Idle),
            cancelled: AtomicBool::new(false),
            deferred_interrupt: AtomicBool::new(false),
        }
    }

    fn begin(&self) -> Result<(), RuntimeError> {
        let mut state = self.lock_state();
        if *state != ProductiveOperationState::Idle {
            return Err(RuntimeError::Protocol(
                "CLI productive operation registration is already active".to_owned(),
            ));
        }
        self.cancelled.store(false, Ordering::Release);
        self.deferred_interrupt.store(false, Ordering::Release);
        *state = ProductiveOperationState::Active;
        Ok(())
    }

    fn interrupt(&self) -> ProductiveInterruptAction {
        let mut state = self.lock_state();
        match *state {
            ProductiveOperationState::Idle | ProductiveOperationState::Cancelling => {
                ProductiveInterruptAction::Exit
            }
            ProductiveOperationState::Completing => {
                if self.deferred_interrupt.swap(true, Ordering::AcqRel) {
                    ProductiveInterruptAction::Exit
                } else {
                    ProductiveInterruptAction::Defer
                }
            }
            ProductiveOperationState::PublishingRun
            | ProductiveOperationState::CommittingRun
            | ProductiveOperationState::CommittingDurableState => {
                if self.cancelled.swap(true, Ordering::AcqRel) {
                    ProductiveInterruptAction::Exit
                } else {
                    ProductiveInterruptAction::Cancel
                }
            }
            ProductiveOperationState::Active | ProductiveOperationState::Dispatching => {
                self.cancelled.store(true, Ordering::Release);
                *state = ProductiveOperationState::Cancelling;
                ProductiveInterruptAction::Cancel
            }
        }
    }

    fn claim_terminal(&self) -> ProductiveTerminalClaim {
        let mut state = self.lock_state();
        match *state {
            ProductiveOperationState::Idle | ProductiveOperationState::Completing => {
                ProductiveTerminalClaim::Completion
            }
            ProductiveOperationState::Cancelling => ProductiveTerminalClaim::Cancellation,
            ProductiveOperationState::Active => {
                *state = ProductiveOperationState::Completing;
                ProductiveTerminalClaim::Completion
            }
            ProductiveOperationState::PublishingRun
            | ProductiveOperationState::CommittingRun
            | ProductiveOperationState::Dispatching
            | ProductiveOperationState::CommittingDurableState => {
                self.cancelled.store(true, Ordering::Release);
                *state = ProductiveOperationState::Cancelling;
                ProductiveTerminalClaim::Cancellation
            }
        }
    }

    fn claim_effect_dispatch(&self) -> Result<ProductiveEffectDispatch<'_>, RuntimeError> {
        let claimed = self.claim_operation(ProductiveOperationState::Dispatching)?;
        Ok(ProductiveEffectDispatch {
            coordinator: self,
            claimed,
        })
    }

    fn claim_durable_commit(&self) -> Result<ProductiveDurableCommit<'_>, RuntimeError> {
        let claimed = self.claim_operation(ProductiveOperationState::CommittingDurableState)?;
        Ok(ProductiveDurableCommit {
            coordinator: self,
            claimed,
        })
    }

    fn claim_run_creation_publication(&self) -> Result<ProductiveRunPublication<'_>, RuntimeError> {
        let claimed = self.claim_operation(ProductiveOperationState::PublishingRun)?;
        Ok(ProductiveRunPublication {
            coordinator: self,
            claimed,
        })
    }

    fn commit_run_creation_publication(&self) -> Result<bool, RuntimeError> {
        let mut state = self.lock_state();
        match *state {
            ProductiveOperationState::Idle => Ok(false),
            ProductiveOperationState::PublishingRun => {
                if self.cancelled.load(Ordering::Acquire) {
                    *state = ProductiveOperationState::Cancelling;
                    Err(RuntimeError::Cancelled)
                } else {
                    *state = ProductiveOperationState::CommittingRun;
                    Ok(true)
                }
            }
            ProductiveOperationState::Active
            | ProductiveOperationState::CommittingRun
            | ProductiveOperationState::Dispatching
            | ProductiveOperationState::CommittingDurableState
            | ProductiveOperationState::Cancelling
            | ProductiveOperationState::Completing => Err(RuntimeError::Cancelled),
        }
    }

    fn finish_run_creation_publication(&self) -> Result<(), RuntimeError> {
        let mut state = self.lock_state();
        match *state {
            ProductiveOperationState::Idle => Ok(()),
            ProductiveOperationState::CommittingRun => {
                if self.cancelled.load(Ordering::Acquire) {
                    *state = ProductiveOperationState::Cancelling;
                    Err(RuntimeError::Cancelled)
                } else {
                    *state = ProductiveOperationState::Active;
                    Ok(())
                }
            }
            ProductiveOperationState::Active
            | ProductiveOperationState::PublishingRun
            | ProductiveOperationState::Dispatching
            | ProductiveOperationState::CommittingDurableState
            | ProductiveOperationState::Cancelling
            | ProductiveOperationState::Completing => Err(RuntimeError::Cancelled),
        }
    }

    fn claim_operation(
        &self,
        claimed_state: ProductiveOperationState,
    ) -> Result<bool, RuntimeError> {
        let mut state = self.lock_state();
        match *state {
            ProductiveOperationState::Idle => Ok(false),
            ProductiveOperationState::Active => {
                *state = claimed_state;
                Ok(true)
            }
            ProductiveOperationState::Dispatching
            | ProductiveOperationState::PublishingRun
            | ProductiveOperationState::CommittingRun
            | ProductiveOperationState::CommittingDurableState
            | ProductiveOperationState::Cancelling
            | ProductiveOperationState::Completing => Err(RuntimeError::Cancelled),
        }
    }

    fn release_operation(&self, expected: ProductiveOperationState) {
        let mut state = self.lock_state();
        if *state == expected {
            *state = if self.cancelled.load(Ordering::Acquire) {
                ProductiveOperationState::Cancelling
            } else {
                ProductiveOperationState::Active
            };
        }
    }

    fn effect_dispatch_allowed(&self) -> bool {
        matches!(
            *self.lock_state(),
            ProductiveOperationState::Idle | ProductiveOperationState::Active
        )
    }

    fn settle(&self) -> bool {
        let mut state = self.lock_state();
        self.cancelled.store(false, Ordering::Release);
        let deferred_interrupt = self.deferred_interrupt.swap(false, Ordering::AcqRel);
        *state = ProductiveOperationState::Idle;
        deferred_interrupt
    }

    #[cfg(test)]
    fn state(&self) -> ProductiveOperationState {
        *self.lock_state()
    }

    fn lock_state(&self) -> MutexGuard<'_, ProductiveOperationState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub(crate) fn productive_cancellation() -> &'static AtomicBool {
    &PRODUCTIVE_OPERATION.cancelled
}

/// Registers one process-wide productive CLI operation.
pub fn begin_productive_operation() -> Result<(), RuntimeError> {
    PRODUCTIVE_OPERATION.begin()
}

/// Linearizes one Unix interrupt against the active productive operation.
pub fn request_productive_interrupt() -> ProductiveInterruptAction {
    PRODUCTIVE_OPERATION.interrupt()
}

pub(crate) fn claim_productive_terminal() -> ProductiveTerminalClaim {
    PRODUCTIVE_OPERATION.claim_terminal()
}

pub(crate) fn ensure_productive_dispatch_allowed() -> Result<(), RuntimeError> {
    PRODUCTIVE_OPERATION
        .effect_dispatch_allowed()
        .then_some(())
        .ok_or(RuntimeError::Cancelled)
}

/// Claims the finite transition that orders an external effect before or after Ctrl-C.
pub(crate) fn claim_productive_effect_dispatch()
-> Result<ProductiveEffectDispatch<'static>, RuntimeError> {
    PRODUCTIVE_OPERATION.claim_effect_dispatch()
}

/// Claims one bounded durable commit and orders it before or after Ctrl-C.
pub(crate) fn claim_productive_durable_commit()
-> Result<ProductiveDurableCommit<'static>, RuntimeError> {
    PRODUCTIVE_OPERATION.claim_durable_commit()
}

/// Claims the cancellable preparation and bounded commit of one Run publication.
pub(crate) fn claim_productive_run_creation_publication()
-> Result<ProductiveRunPublication<'static>, RuntimeError> {
    PRODUCTIVE_OPERATION.claim_run_creation_publication()
}

/// Disarms the process-wide operation after its complete CLI result has been handled.
pub fn settle_productive_operation() -> bool {
    PRODUCTIVE_OPERATION.settle()
}

#[cfg(test)]
mod tests {
    use super::{
        ProductiveInterruptAction, ProductiveOperationCoordinator, ProductiveOperationState,
        ProductiveTerminalClaim,
    };
    use crate::runtime::types::RuntimeError;
    use std::sync::atomic::Ordering;

    #[test]
    fn cancellation_wins_one_finite_operation_transition_table() {
        let coordinator = ProductiveOperationCoordinator::new();

        assert_eq!(coordinator.state(), ProductiveOperationState::Idle);
        coordinator.begin().expect("idle operation begins");
        assert_eq!(coordinator.state(), ProductiveOperationState::Active);
        assert!(coordinator.effect_dispatch_allowed());
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Cancel);
        assert_eq!(coordinator.state(), ProductiveOperationState::Cancelling);
        assert!(coordinator.cancelled.load(Ordering::Acquire));
        assert!(!coordinator.effect_dispatch_allowed());
        assert_eq!(
            coordinator.claim_terminal(),
            ProductiveTerminalClaim::Cancellation
        );
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Exit);
        assert!(!coordinator.settle());
        assert_eq!(coordinator.state(), ProductiveOperationState::Idle);
        assert!(!coordinator.cancelled.load(Ordering::Acquire));
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Exit);
    }

    #[test]
    fn completion_wins_one_finite_operation_transition_table() {
        let coordinator = ProductiveOperationCoordinator::new();

        coordinator.begin().expect("idle operation begins");
        assert_eq!(
            coordinator.claim_terminal(),
            ProductiveTerminalClaim::Completion
        );
        assert_eq!(coordinator.state(), ProductiveOperationState::Completing);
        assert!(!coordinator.effect_dispatch_allowed());
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Defer);
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Exit);
        assert!(
            coordinator.settle(),
            "the deferred interrupt is delivered when completion disarms"
        );
        assert_eq!(coordinator.state(), ProductiveOperationState::Idle);
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Exit);
    }

    #[test]
    fn effect_dispatch_and_interrupt_have_one_atomic_order() {
        let coordinator = ProductiveOperationCoordinator::new();
        coordinator.begin().expect("idle operation begins");
        let dispatch = coordinator
            .claim_effect_dispatch()
            .expect("active operation claims its effect dispatch");
        assert_eq!(coordinator.state(), ProductiveOperationState::Dispatching);
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Cancel);
        drop(dispatch);
        assert_eq!(coordinator.state(), ProductiveOperationState::Cancelling);
        assert!(!coordinator.effect_dispatch_allowed());
        assert!(!coordinator.settle());

        coordinator.begin().expect("next operation begins");
        let dispatch = coordinator
            .claim_effect_dispatch()
            .expect("active operation claims its effect dispatch");
        drop(dispatch);
        assert_eq!(coordinator.state(), ProductiveOperationState::Active);
        assert!(coordinator.effect_dispatch_allowed());
        assert!(!coordinator.settle());
    }

    #[test]
    fn durable_commit_and_interrupt_have_one_atomic_order() {
        let coordinator = ProductiveOperationCoordinator::new();
        coordinator.begin().expect("productive operation starts");

        let commit = coordinator
            .claim_durable_commit()
            .expect("active operation claims the bounded durable commit");
        assert_eq!(
            coordinator.state(),
            ProductiveOperationState::CommittingDurableState
        );
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Cancel);
        assert_eq!(
            coordinator.state(),
            ProductiveOperationState::CommittingDurableState
        );
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Exit);
        drop(commit);
        assert_eq!(coordinator.state(), ProductiveOperationState::Cancelling);
        assert!(!coordinator.settle());

        coordinator.begin().expect("next operation begins");
        let commit = coordinator
            .claim_durable_commit()
            .expect("next durable commit is claimed");
        drop(commit);
        assert_eq!(coordinator.state(), ProductiveOperationState::Active);
        assert!(!coordinator.settle());
    }

    #[test]
    fn run_creation_publication_and_interrupt_have_one_atomic_order() {
        let coordinator = ProductiveOperationCoordinator::new();
        coordinator.begin().expect("productive operation starts");
        let publication = coordinator
            .claim_run_creation_publication()
            .expect("active operation claims Run publication");
        assert_eq!(coordinator.state(), ProductiveOperationState::PublishingRun);
        assert!(
            coordinator.state.try_lock().is_ok(),
            "publication must not hold the signal mutex"
        );
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Cancel);
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Exit);
        assert!(matches!(publication.commit(), Err(RuntimeError::Cancelled)));
        assert_eq!(coordinator.state(), ProductiveOperationState::Cancelling);
        assert!(!coordinator.settle());

        coordinator
            .begin()
            .expect("next productive operation starts");
        let publication = coordinator
            .claim_run_creation_publication()
            .expect("next Run publication is claimed");
        let publication = publication.commit().expect("publication commit wins");
        assert_eq!(coordinator.state(), ProductiveOperationState::CommittingRun);
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Cancel);
        assert_eq!(coordinator.interrupt(), ProductiveInterruptAction::Exit);
        assert!(matches!(publication.finish(), Err(RuntimeError::Cancelled)));
        assert_eq!(coordinator.state(), ProductiveOperationState::Cancelling);
        assert!(!coordinator.settle());
    }

    #[test]
    fn terminal_claim_during_effect_dispatch_fails_closed() {
        let coordinator = ProductiveOperationCoordinator::new();
        coordinator.begin().expect("productive operation starts");

        let dispatch = coordinator
            .claim_effect_dispatch()
            .expect("effect dispatch is claimed");
        assert_eq!(coordinator.state(), ProductiveOperationState::Dispatching);

        assert_eq!(
            coordinator.claim_terminal(),
            ProductiveTerminalClaim::Cancellation
        );
        assert!(coordinator.cancelled.load(Ordering::Acquire));
        assert_eq!(coordinator.state(), ProductiveOperationState::Cancelling);

        drop(dispatch);
        assert_eq!(coordinator.state(), ProductiveOperationState::Cancelling);
        assert!(!coordinator.settle());
    }

    #[test]
    fn activation_and_abandonment_are_bounded_and_non_overlapping() {
        let coordinator = ProductiveOperationCoordinator::new();
        coordinator.begin().expect("idle operation begins");
        assert!(coordinator.begin().is_err());
        assert!(!coordinator.settle());
        coordinator
            .begin()
            .expect("settled operation may be replaced");
        assert!(!coordinator.settle());
        assert_eq!(coordinator.state(), ProductiveOperationState::Idle);
    }
}
