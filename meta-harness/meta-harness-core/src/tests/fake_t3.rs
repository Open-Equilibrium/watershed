use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{Notify, mpsc};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

use super::until;
use crate::t3::Client;

pub fn frame(value: Value) -> Message {
    Message::Text(value.to_string().into())
}

pub fn ok(id: &str, value: Value) -> Message {
    frame(json!({"_tag": "Exit", "requestId": id, "exit": {"_tag": "Success", "value": value}}))
}

pub fn fail(id: &str, cause: Value) -> Message {
    frame(json!({"_tag": "Exit", "requestId": id, "exit": {"_tag": "Failure", "cause": cause}}))
}

pub fn chunk(id: &str, values: Vec<Value>) -> Message {
    frame(json!({"_tag": "Chunk", "requestId": id, "values": values}))
}

enum Out {
    Chunk(String, Vec<Value>),
    Disconnect,
    Stop,
}

#[derive(Default)]
struct Log {
    requests: Vec<(String, Value)>,
    latest: HashMap<String, String>,
    interrupts: Vec<String>,
    acks: Vec<String>,
}

pub struct Fake {
    pub origin: String,
    log: Arc<Mutex<Log>>,
    accepted: Arc<AtomicUsize>,
    hung_up: Arc<AtomicUsize>,
    out: mpsc::UnboundedSender<Out>,
    stop: Arc<Notify>,
}

impl Fake {
    pub async fn start(
        answer: impl Fn(&str, &Value, &str) -> Vec<Message> + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let log: Arc<Mutex<Log>> = Arc::default();
        let accepted: Arc<AtomicUsize> = Arc::default();
        let hung_up: Arc<AtomicUsize> = Arc::default();
        let (out, mut outgoing) = mpsc::unbounded_channel();
        let stop = Arc::new(Notify::new());
        let (shared, counter, stop_task) = (log.clone(), accepted.clone(), stop.clone());
        let hung_up_task = hung_up.clone();
        tokio::spawn(async move {
            let mut handle_open = true;
            'connections: loop {
                let connection = tokio::select! {
                    biased;
                    _ = stop_task.notified() => break 'connections,
                    connection = listener.accept() => connection.unwrap().0,
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let mut socket = accept_async(connection).await.unwrap();
                loop {
                    tokio::select! {
                        incoming = socket.next() => {
                            let Some(Ok(message)) = incoming else { break };
                            let text = match message {
                                Message::Text(text) => text,
                                Message::Ping(_) | Message::Pong(_) => continue,
                                _ => break,
                            };
                            let frames = match serde_json::from_str::<Value>(&text).unwrap() {
                                Value::Array(frames) => frames,
                                frame => vec![frame],
                            };
                            for request in frames.iter().filter(|f| f["_tag"] == "Request") {
                                let id = request["id"].as_str().unwrap().to_owned();
                                let tag = request["tag"].as_str().unwrap().to_owned();
                                let payload = request["payload"].clone();
                                {
                                    let mut log = shared.lock().unwrap();
                                    log.latest.insert(tag.clone(), id.clone());
                                    log.requests.push((tag.clone(), payload.clone()));
                                }
                                for reply in answer(&tag, &payload, &id) {
                                    socket.send(reply).await.unwrap();
                                }
                            }
                            for interrupt in frames.iter().filter(|f| f["_tag"] == "Interrupt") {
                                let id = interrupt["requestId"].as_str().unwrap().to_owned();
                                shared.lock().unwrap().interrupts.push(id);
                            }
                            for ack in frames.iter().filter(|f| f["_tag"] == "Ack") {
                                let id = ack["requestId"].as_str().unwrap().to_owned();
                                shared.lock().unwrap().acks.push(id);
                            }
                        }
                        next = outgoing.recv(), if handle_open => match next {
                            Some(Out::Chunk(tag, values)) => {
                                let id = shared.lock().unwrap().latest[&tag].clone();
                                if !shared.lock().unwrap().interrupts.contains(&id) {
                                    socket.send(chunk(&id, values)).await.unwrap();
                                }
                            }
                            Some(Out::Disconnect) => {
                                let _ = socket.close(None).await;
                                continue 'connections;
                            }
                            Some(Out::Stop) => {
                                let _ = socket.close(None).await;
                                break 'connections;
                            }
                            None => handle_open = false,
                        },
                    }
                }
                hung_up_task.fetch_add(1, Ordering::SeqCst);
            }
        });
        Self {
            origin,
            log,
            accepted,
            hung_up,
            out,
            stop,
        }
    }

    pub async fn client(&self) -> Client {
        Client::connect(&self.origin, || Box::pin(async { Ok("token".to_owned()) }))
            .await
            .unwrap()
    }

    pub fn push(&self, tag: &str, values: Vec<Value>) {
        let _ = self.out.send(Out::Chunk(tag.to_owned(), values));
    }

    pub fn request_id(&self, tag: &str) -> String {
        self.log.lock().unwrap().latest[tag].clone()
    }

    pub fn acks(&self, id: &str) -> usize {
        self.log
            .lock()
            .unwrap()
            .acks
            .iter()
            .filter(|ack| ack.as_str() == id)
            .count()
    }

    pub async fn acked(&self, id: &str, count: usize) {
        until(format!("{count} acknowledgements of request {id}"), || {
            self.acks(id) >= count
        })
        .await;
    }

    pub async fn interrupted(&self, id: &str) {
        until(format!("an interrupt of request {id}"), || {
            self.log.lock().unwrap().interrupts.iter().any(|i| i == id)
        })
        .await;
    }

    pub fn drop_connection(&self) {
        let _ = self.out.send(Out::Disconnect);
    }

    pub fn close(&self) {
        self.stop.notify_one();
        let _ = self.out.send(Out::Stop);
    }

    pub fn connections(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    pub async fn hung_up(&self, count: usize) {
        until(format!("{count} hang-ups"), || {
            self.hung_up.load(Ordering::SeqCst) >= count
        })
        .await;
    }

    pub async fn accepted(&self, count: usize) {
        until(format!("{count} accepted connections"), || {
            self.connections() >= count
        })
        .await;
    }

    pub async fn received(&self, tag: &str, count: usize) {
        until(format!("{count} requests for {tag}"), || {
            self.payloads(tag).len() >= count
        })
        .await;
    }

    pub fn payloads(&self, tag: &str) -> Vec<Value> {
        self.log
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(t, _)| t == tag)
            .map(|(_, payload)| payload.clone())
            .collect()
    }
}
