//! The WebSocket client to T3: calls, streams, what ends them, and the reconnection after T3 drops
//! a connection (ADR-0292).

use std::time::Duration;

use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use super::fake_t3::{Fake, fail, frame, ok};
use crate::t3::Client;

const WAIT: Duration = Duration::from_secs(10);

async fn connect(fake: &Fake) -> Client {
    Client::connect(&fake.origin, || Ok("token".to_owned()))
        .await
        .unwrap()
}

/// Waits until the client reconnected after T3 dropped its connection.
async fn reconnected(fake: &Fake, client: &Client) {
    let reconnect = async {
        fake.accepted(2).await;
        while !client.connected() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(WAIT, reconnect)
        .await
        .expect("the client reconnects");
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
    // The client answers the Ping on the same connection, which stays open for the next call.
    assert_eq!(client.call("x", json!({})).await, Ok(json!(7)));
    assert_eq!(fake.connections(), 1);
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

#[tokio::test(flavor = "multi_thread")]
async fn a_call_after_t3_drops_the_connection_runs_on_a_new_one() {
    let fake = Fake::start(|_, _, id| vec![ok(id, json!({"threads": 2}))]).await;
    let client = connect(&fake).await;
    fake.drop_connection();
    reconnected(&fake, &client).await;
    assert_eq!(client.call("x", json!({})).await, Ok(json!({"threads": 2})));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_in_flight_when_t3_drops_the_connection_fails_and_is_not_sent_again() {
    // Only the retry gets an answer, so the call that waited for the lost connection never does.
    let fake = Fake::start(|_, payload, id| {
        if payload["retry"] == true {
            vec![ok(id, json!(7))]
        } else {
            Vec::new()
        }
    })
    .await;
    let client = connect(&fake).await;
    let call = tokio::spawn({
        let client = client.clone();
        async move { client.call("x", json!({})).await }
    });
    tokio::time::timeout(WAIT, fake.received("x", 1))
        .await
        .unwrap();
    fake.drop_connection();
    let answer = tokio::time::timeout(WAIT, call).await.unwrap().unwrap();
    assert!(answer.is_err());
    reconnected(&fake, &client).await;
    assert_eq!(client.call("x", json!({"retry": true})).await, Ok(json!(7)));
    assert_eq!(fake.payloads("x"), vec![json!({}), json!({"retry": true})]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_open_stream_is_sent_again_with_its_payload_and_keeps_its_items() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = connect(&fake).await;
    let mut items = client
        .stream("subscribe", json!({"threadId": "t"}))
        .await
        .unwrap();
    fake.received("subscribe", 1).await;
    let id = fake.request_id("subscribe");
    fake.push("subscribe", vec![json!("one")]);
    assert_eq!(items.recv().await, Some(json!("one")));
    fake.drop_connection();
    tokio::time::timeout(WAIT, fake.received("subscribe", 2))
        .await
        .expect("the stream is sent again");
    assert_eq!(fake.request_id("subscribe"), id);
    assert_eq!(
        fake.payloads("subscribe"),
        vec![json!({"threadId": "t"}), json!({"threadId": "t"})]
    );
    fake.push("subscribe", vec![json!("two")]);
    assert_eq!(items.recv().await, Some(json!("two")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_stream_interrupts_its_request_after_a_reconnect() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = connect(&fake).await;
    let items = client.stream("subscribe", json!({})).await.unwrap();
    fake.received("subscribe", 1).await;
    let id = fake.request_id("subscribe");
    fake.drop_connection();
    tokio::time::timeout(WAIT, fake.received("subscribe", 2))
        .await
        .expect("the stream is sent again");
    drop(items);
    tokio::time::timeout(WAIT, fake.interrupted(&id))
        .await
        .expect("T3 receives the interrupt on the new connection");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chunk_is_acknowledged_only_once_its_last_item_is_taken() {
    // T3 sends a chunk's successor only after its acknowledgement (Effect RPC flow control), so an
    // item the consumer has not taken holds T3 back. Streams and actions both work this way.
    for action in [false, true] {
        let fake = Fake::start(|_, _, _| Vec::new()).await;
        let client = connect(&fake).await;
        let opened = if action {
            client.action("subscribe", json!({})).await
        } else {
            client.stream("subscribe", json!({})).await
        };
        let mut items = opened.unwrap();
        fake.received("subscribe", 1).await;
        let id = fake.request_id("subscribe");
        fake.push("subscribe", vec![json!("one"), json!("two")]);
        assert_eq!(items.recv().await, Some(json!("one")));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(fake.acks(&id), 0, "action: {action}");
        assert_eq!(items.recv().await, Some(json!("two")));
        tokio::time::timeout(WAIT, fake.acked(&id, 1))
            .await
            .expect("T3 receives the acknowledgement");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resent_stream_is_acknowledged_on_its_new_connection() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = connect(&fake).await;
    let mut items = client.stream("subscribe", json!({})).await.unwrap();
    fake.received("subscribe", 1).await;
    fake.drop_connection();
    tokio::time::timeout(WAIT, fake.received("subscribe", 2))
        .await
        .expect("the stream is sent again");
    let id = fake.request_id("subscribe");
    fake.push("subscribe", vec![json!("one")]);
    assert_eq!(items.recv().await, Some(json!("one")));
    tokio::time::timeout(WAIT, fake.acked(&id, 1))
        .await
        .expect("T3 receives the acknowledgement on the new connection");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_client_does_not_reconnect_when_its_connection_drops() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = connect(&fake).await;
    let mut items = client.stream("subscribe", json!({})).await.unwrap();
    fake.received("subscribe", 1).await;
    client.close();
    fake.drop_connection();
    let ended = tokio::time::timeout(WAIT, items.recv())
        .await
        .expect("the stream ends");
    assert!(ended.is_none());
    assert_eq!(fake.connections(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_whose_t3_stays_gone_closes_for_good() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = connect(&fake).await;
    let mut items = client.stream("subscribe", json!({})).await.unwrap();
    fake.received("subscribe", 1).await;
    fake.close();
    let ended = tokio::time::timeout(WAIT, items.recv())
        .await
        .expect("the stream ends once the retries failed");
    assert!(ended.is_none());
    assert!(client.call("x", json!({})).await.is_err());
}
