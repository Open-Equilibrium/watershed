use std::time::Duration;

use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use super::fake_t3::{Fake, fail, frame, ok};
use crate::t3::{Client, Failure};

const WAIT: Duration = Duration::from_secs(10);

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

fn spawn_call(
    client: &Client,
    tag: &'static str,
) -> tokio::task::JoinHandle<Result<serde_json::Value, String>> {
    tokio::spawn({
        let client = client.clone();
        async move { client.call(tag, json!({})).await }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_returns_what_t3_replies_or_the_cause_it_failed_with() {
    let fake = Fake::start(|tag, _, id| match tag {
        "fail" => vec![fail(id, json!({"_tag": "Fail", "error": "no such thread"}))],
        _ => vec![ok(id, json!({"threads": 2}))],
    })
    .await;
    let client = fake.client().await;
    let reply = client
        .call("orchestration.getThread", json!({"threadId": "t"}))
        .await;
    assert_eq!(reply, Ok(json!({"threads": 2})));
    assert_eq!(
        fake.payloads("orchestration.getThread"),
        vec![json!({"threadId": "t"})]
    );
    let error = client.call("fail", json!({})).await.unwrap_err();
    assert!(error.contains("no such thread"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_is_sent_with_a_fresh_command_id_each_time() {
    let fake = Fake::start(|_, _, id| vec![ok(id, json!({}))]).await;
    let client = fake.client().await;
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
async fn a_failed_call_returns_t3_s_message_not_its_cause_json() {
    let cause = json!([{"_tag": "Fail", "error": {"_tag": "LaunchError",
        "message": "Provider instance \"nosuch\" is disabled."}}]);
    let fake = Fake::start(move |_, _, id| vec![fail(id, cause.clone())]).await;
    let error = fake.client().await.call("x", json!({})).await.unwrap_err();
    assert_eq!(error, "Provider instance \"nosuch\" is disabled.");
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
    let client = fake.client().await;
    assert_eq!(client.call("x", json!({})).await, Ok(json!(7)));
    assert_eq!(client.call("x", json!({})).await, Ok(json!(7)));
    assert_eq!(fake.connections(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_waiting_for_t3_fails_when_the_connection_closes_and_later_calls_fail() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = fake.client().await;
    let call = spawn_call(&client, "x");
    tokio::time::timeout(WAIT, fake.received("x", 1))
        .await
        .unwrap();
    fake.close();
    let answer = tokio::time::timeout(WAIT, call).await.unwrap().unwrap();
    assert!(answer.is_err());

    tokio::time::sleep(Duration::from_millis(200)).await;
    let answer = tokio::time::timeout(WAIT, client.call("x", json!({}))).await;
    assert!(answer.expect("the call ends").is_err());
    let opened = tokio::time::timeout(WAIT, client.stream("subscribe", json!({}))).await;
    assert!(opened.expect("the stream opens or fails").is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_stream_or_call_is_interrupted_or_forgotten_on_t3() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = fake.client().await;
    let items = client.stream("subscribe", json!({})).await.unwrap();
    fake.received("subscribe", 1).await;
    let id = fake.request_id("subscribe");
    assert_eq!(client.waiting(), 1);
    drop(items);
    tokio::time::timeout(WAIT, fake.interrupted(&id))
        .await
        .expect("T3 receives the interrupt");
    assert_eq!(client.waiting(), 0);

    let call = spawn_call(&client, "slow");
    fake.received("slow", 1).await;
    assert_eq!(client.waiting(), 1);
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    assert_eq!(client.waiting(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_in_flight_when_t3_drops_the_connection_fails_and_is_not_sent_again() {
    let fake = Fake::start(|_, payload, id| {
        if payload["retry"] == true {
            vec![ok(id, json!(7))]
        } else {
            Vec::new()
        }
    })
    .await;
    let client = fake.client().await;
    let call = spawn_call(&client, "x");
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
async fn a_chunk_is_acknowledged_only_once_its_last_item_is_taken() {
    for action in [false, true] {
        let fake = Fake::start(|_, _, _| Vec::new()).await;
        let client = fake.client().await;
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
async fn an_open_stream_survives_a_reconnect_and_is_acknowledged_and_interrupted_on_it() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = fake.client().await;
    let mut items = client
        .stream("subscribe", json!({"threadId": "t"}))
        .await
        .unwrap();
    fake.received("subscribe", 1).await;
    let id = fake.request_id("subscribe");
    fake.push("subscribe", vec![json!("one")]);
    assert_eq!(items.recv().await, Some(json!("one")));
    fake.acked(&id, 1).await;
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
    fake.acked(&id, 2).await;
    drop(items);
    tokio::time::timeout(WAIT, fake.interrupted(&id))
        .await
        .expect("T3 receives the interrupt on the new connection");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_client_or_one_whose_t3_stays_gone_ends_its_streams_for_good() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = fake.client().await;
    let mut items = client.stream("subscribe", json!({})).await.unwrap();
    fake.received("subscribe", 1).await;
    client.close();
    fake.drop_connection();
    let ended = tokio::time::timeout(WAIT, items.recv())
        .await
        .expect("the stream ends");
    assert!(ended.is_none());
    assert_eq!(fake.connections(), 1, "a closed client does not reconnect");

    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = fake.client().await;
    let mut items = client.stream("subscribe", json!({})).await.unwrap();
    fake.received("subscribe", 1).await;
    fake.close();
    let ended = tokio::time::timeout(WAIT, items.recv())
        .await
        .expect("the stream ends once the retries failed");
    assert!(ended.is_none());
    assert!(client.call("x", json!({})).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn a_t3_that_accepts_a_connection_and_never_answers_fails_the_connect() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let connect = Client::connect(&origin, || Box::pin(async { Ok("token".to_owned()) }));
    let answer = tokio::time::timeout(Duration::from_secs(600), connect)
        .await
        .expect("the connect gives up");
    assert!(answer.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_action_that_loses_its_connection_records_why_it_ended() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client = fake.client().await;
    let mut action = client.action("subscribe", json!({})).await.unwrap();
    fake.received("subscribe", 1).await;
    fake.push("subscribe", vec![json!("one")]);
    assert_eq!(action.recv().await, Some(json!("one")));
    fake.drop_connection();
    let ended = tokio::time::timeout(WAIT, action.recv())
        .await
        .expect("the action ends with its connection");
    assert_eq!(ended, None);
    assert_eq!(action.failure(), Some(Failure::ConnectionLost));
}
