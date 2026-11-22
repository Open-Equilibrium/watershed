use super::super::support::run_isolated_test;
use super::support::{
    MemoryAttempts, MemorySink, ScriptedProvider, smoke_productive_execution_fixture,
    string_provider_turn,
};
use crate::runtime::{
    openai_codex::ProviderTurn,
    productive::{ProductiveProvider, ProviderCredential, execute_productive_flow},
    steering::take_productive_steering,
    types::RuntimeError,
};
use proto::EventType;
use std::{sync::mpsc, thread, time::Duration};

struct SteeringProvider {
    inner: ScriptedProvider,
    message_during_first_turn: Option<String>,
}

impl ProductiveProvider for SteeringProvider {
    fn turn(
        &mut self,
        credential: &ProviderCredential,
        body: &serde_json::Value,
    ) -> Result<ProviderTurn, RuntimeError> {
        if let Some(message) = self.message_during_first_turn.take() {
            crate::steer_productive_run(message).expect("the steering message is accepted");
        }
        self.inner.turn(credential, body)
    }
}

fn user_texts(body: &serde_json::Value) -> Vec<String> {
    body["input"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["role"] == "user")
        .flat_map(|item| item["content"].as_array().cloned().unwrap_or_default())
        .filter_map(|part| part["text"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn a_message_sent_during_the_final_turn_gives_the_phase_another_turn() {
    const CHILD_ENV: &str = "WATERSHED_STEERING_FINAL_TURN_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }
    crate::begin_productive_operation().expect("productive operation begins");
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = SteeringProvider {
        inner: ScriptedProvider::new([
            string_provider_turn("response-draft", "draft"),
            string_provider_turn("response-final", "final"),
        ]),
        message_during_first_turn: Some("Use British spelling".to_owned()),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();

    let execution = execute_productive_flow(
        fixture.execution(flow, "steering-final-turn"),
        &mut provider,
        &mut attempts,
        &mut sink,
    )
    .expect("the steered Run completes");
    crate::settle_productive_operation();

    assert!(!execution.failed);
    let bodies = &provider.inner.bodies;
    assert_eq!(bodies.len(), 2, "the message adds one turn");
    assert!(user_texts(&bodies[0]).is_empty());
    assert_eq!(user_texts(&bodies[1]), ["Use British spelling"]);
    let user_deltas = sink
        .0
        .iter()
        .filter(|event| {
            event.event_type == EventType::MessageDelta && event.payload["role"] == "user"
        })
        .map(|event| event.payload["content_delta"].as_str().unwrap_or_default())
        .collect::<String>();
    assert_eq!(user_deltas, "Use British spelling");
    let completed = sink
        .0
        .iter()
        .find(|event| event.event_type == EventType::PhaseCompleted)
        .expect("the Phase completes");
    assert!(completed.payload.to_string().contains("final"));
}

#[test]
fn a_paused_run_waits_until_a_steering_message_arrives() {
    const CHILD_ENV: &str = "WATERSHED_STEERING_PAUSE_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }
    crate::begin_productive_operation().expect("productive operation begins");
    crate::request_productive_pause();
    let (sender, receiver) = mpsc::channel();
    let waiter = thread::spawn(move || sender.send(take_productive_steering()).unwrap());

    assert!(
        receiver.recv_timeout(Duration::from_millis(200)).is_err(),
        "a paused Run takes no further step"
    );
    crate::steer_productive_run("Continue with the tests".to_owned()).unwrap();

    let taken = receiver
        .recv()
        .unwrap()
        .expect("the wait ends with a message");
    assert_eq!(taken.as_deref(), Some("Continue with the tests"));
    waiter.join().unwrap();
    crate::settle_productive_operation();
}

#[test]
fn an_interrupt_ends_a_paused_wait_as_cancellation() {
    const CHILD_ENV: &str = "WATERSHED_STEERING_PAUSE_INTERRUPT_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }
    crate::begin_productive_operation().expect("productive operation begins");
    crate::request_productive_pause();
    let waiter = thread::spawn(take_productive_steering);
    thread::sleep(Duration::from_millis(50));

    crate::request_productive_interrupt();

    assert!(matches!(
        waiter.join().unwrap(),
        Err(RuntimeError::Cancelled)
    ));
    crate::settle_productive_operation();
}

#[test]
fn a_steering_message_needs_text_within_the_run_input_limit() {
    assert!(crate::steer_productive_run(" \n".to_owned()).is_err());
    assert!(
        crate::steer_productive_run("x".repeat(crate::MAX_STEERING_MESSAGE_BYTES + 1)).is_err()
    );
}

#[test]
fn messages_no_run_took_are_handed_back() {
    const CHILD_ENV: &str = "WATERSHED_STEERING_DRAIN_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }
    crate::steer_productive_run("first".to_owned()).unwrap();
    crate::steer_productive_run("second".to_owned()).unwrap();

    assert_eq!(crate::drain_productive_steering(), ["first", "second"]);
    assert!(crate::drain_productive_steering().is_empty());
}
