//! The WebSocket client to T3: calls, streams, and what ends them (ADR-0292).

use std::time::Duration;

use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use super::fake_t3::{Fake, fail, frame, ok};
use crate::t3::Client;

const WAIT: Duration = Duration::from_secs(10);

async fn connect(fake: &Fake) -> Client {
    Client::connect(&fake.origin, "token").await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_returns_what_t3_replies_and_sends_its_payload() {
    let fake = Fake::start(|_, _, id| vec![ok(id, json!({"threads": 2}))]).await;
    let client = connect(&fake).await;
    let reply = client
        .call("orchestration.getThread", json!({"threadId": "t"}))
        .await;
    assert_eq!(reply, Ok(json!({"threads": 2})));
    assert_eq!(
        fake.payloads("orchestration.getThread"),
        vec![json!({"threadId": "t"})]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_is_sent_with_a_fresh_command_id_each_time() {
    let fake = Fake::start(|_, _, id| vec![ok(id, json!({}))]).await;
    let client = connect(&fake).await;
    for _ in 0..2 {
        client
            .command("orchestration.dispatchCommand", json!({"type": "x"}))
            .await
            .unwrap();
    }
    let sent = fake.payloads("orchestration.dispatchCommand");
    assert!(sent.iter().all(|command| command["type"] == "x"));
    assert!(
        sent[0]["commandId"]
            .as_str()
            .is_some_and(|id| id.starts_with("cmd-"))
    );
    assert_ne!(sent[0]["commandId"], sent[1]["commandId"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_call_returns_the_cause_t3_sent() {
    let fake =
        Fake::start(|_, _, id| vec![fail(id, json!({"_tag": "Fail", "error": "no such thread"}))])
            .await;
    let error = connect(&fake).await.call("x", json!({})).await.unwrap_err();
    assert!(error.contains("no such thread"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn frames_without_a_request_and_batches_are_read_and_malformed_ones_skipped() {
    let fake = Fake::start(|_, _, id| {
        vec![
            Message::Ping(Vec::new().into()),
            Message::Text("not JSON".into()),
            frame(json!({"_tag": "Pong"})),
            frame(json!([
                {"_tag": "Pong"},
                {"_tag": "Exit", "requestId": id, "exit": {"_tag": "Success", "value": 7}},
            ])),
        ]
    })
    .await;
    let client = connect(&fake).await;
    assert_eq!(client.call("x", json!({})).await, Ok(json!(7)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_waiting_for_t3_fails_when_the_connection_closes() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = connect(&fake).await;
    let call = tokio::spawn({
        let client = client.clone();
        async move { client.call("x", json!({})).await }
    });
    tokio::time::timeout(WAIT, fake.received("x", 1))
        .await
        .unwrap();
    fake.close();
    let answer = tokio::time::timeout(WAIT, call).await.unwrap().unwrap();
    assert!(answer.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_after_the_connection_closed_fails_instead_of_waiting() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = connect(&fake).await;
    fake.close();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let answer = tokio::time::timeout(WAIT, client.call("x", json!({}))).await;
    assert!(answer.expect("the call ends").is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_opened_after_the_connection_closed_fails_instead_of_waiting() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = connect(&fake).await;
    fake.close();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let opened = tokio::time::timeout(WAIT, client.stream("subscribe", json!({}))).await;
    assert!(opened.expect("the stream opens or fails").is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_stream_interrupts_its_request_on_t3_and_is_forgotten() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = connect(&fake).await;
    let items = client.stream("subscribe", json!({})).await.unwrap();
    fake.received("subscribe", 1).await;
    let id = fake.request_id("subscribe");
    assert_eq!(client.waiting(), 1);
    drop(items);
    tokio::time::timeout(WAIT, fake.interrupted(&id))
        .await
        .expect("T3 receives the interrupt");
    assert_eq!(client.waiting(), 0);
}
