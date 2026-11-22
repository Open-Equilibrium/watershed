use crate::runtime::{run_input::MAX_FLOW_RUN_INPUT_BYTES, types::RuntimeError};
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex, MutexGuard},
};

pub const MAX_STEERING_MESSAGE_BYTES: usize = MAX_FLOW_RUN_INPUT_BYTES;

static STEERING: Steering = Steering::new();

struct Steering {
    state: Mutex<SteeringState>,
    changed: Condvar,
}

struct SteeringState {
    pause: bool,
    messages: VecDeque<String>,
}

impl Steering {
    const fn new() -> Self {
        Self {
            state: Mutex::new(SteeringState {
                pause: false,
                messages: VecDeque::new(),
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, SteeringState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub fn request_productive_pause() {
    STEERING.lock().pause = true;
    STEERING.changed.notify_all();
}

pub fn steer_productive_run(message: String) -> Result<(), RuntimeError> {
    if message.trim().is_empty() {
        return Err(RuntimeError::Protocol(
            "steering message must contain text".to_owned(),
        ));
    }
    if message.len() > MAX_STEERING_MESSAGE_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "steering message size {} bytes exceeds max {MAX_STEERING_MESSAGE_BYTES}",
            message.len()
        )));
    }
    let mut state = STEERING.lock();
    state.pause = false;
    state.messages.push_back(message);
    drop(state);
    STEERING.changed.notify_all();
    Ok(())
}

pub fn drain_productive_steering() -> Vec<String> {
    let mut state = STEERING.lock();
    state.pause = false;
    state.messages.drain(..).collect()
}

pub(crate) fn wake_productive_steering() {
    drop(STEERING.lock());
    STEERING.changed.notify_all();
}

pub(crate) fn take_productive_steering() -> Result<Option<String>, RuntimeError> {
    let mut state = STEERING.lock();
    loop {
        if !state.pause {
            return Ok(state.messages.pop_front());
        }
        crate::runtime::cancellation::ensure_productive_dispatch_allowed()?;
        state = STEERING
            .changed
            .wait(state)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}
