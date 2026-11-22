use crate::subflows::InstanceEvent;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct RunView {
    phases: Vec<Phase>,
    messages: BTreeMap<String, String>,
}

struct Phase {
    execution_id: String,
    name: String,
    status: &'static str,
    instance: bool,
}

impl RunView {
    pub(super) fn observe(&mut self, event: &Value) -> Vec<Value> {
        let payload = &event["payload"];
        let field = |name: &str| payload[name].as_str().unwrap_or_default();
        match event["event_type"].as_str().unwrap_or_default() {
            "phase.entered" => {
                self.set_phase(
                    field("phase_execution_id"),
                    field("phase_name"),
                    "in_progress",
                );
                vec![self.plan()]
            }
            "phase.completed" | "phase.failed" => {
                let status = if payload["will_repeat"] == true {
                    "in_progress"
                } else {
                    "completed"
                };
                self.set_phase(field("phase_execution_id"), "", status);
                vec![self.plan()]
            }
            "message.delta" if payload["role"] == "assistant" => {
                self.messages
                    .entry(field("message_id").to_owned())
                    .or_default()
                    .push_str(field("content_delta"));
                Vec::new()
            }
            "message.completed" => self
                .messages
                .remove(field("message_id"))
                .and_then(|content| reply_text(&content))
                .map(|text| vec![chunk(&text)])
                .unwrap_or_default(),
            "tool.started" => vec![json!({
                "sessionUpdate": "tool_call",
                "toolCallId": tool_call_id(payload),
                "title": field("tool_name"),
                "kind": "execute",
                "status": "in_progress",
            })],
            "tool.completed" => vec![tool_update(payload, "completed")],
            "tool.failed" | "tool.timed_out" => vec![tool_update(payload, "failed")],
            "error" => vec![chunk(&format!("Flow Agent: {}", field("message")))],
            _ => Vec::new(),
        }
    }

    pub(super) fn instance(&mut self, event: &InstanceEvent) -> Value {
        match event {
            InstanceEvent::Started { name } => {
                let phase = self
                    .phases
                    .iter()
                    .rev()
                    .find(|phase| phase.status == "in_progress" && !phase.instance)
                    .map(|phase| phase.name.clone())
                    .unwrap_or_default();
                self.phases.push(Phase {
                    execution_id: name.clone(),
                    name: format!("{phase} › {name}"),
                    status: "in_progress",
                    instance: true,
                });
            }
            InstanceEvent::Finished { name, .. } => {
                if let Some(phase) = self
                    .phases
                    .iter_mut()
                    .find(|phase| phase.instance && phase.execution_id == *name)
                {
                    phase.status = "completed";
                }
            }
        }
        self.plan()
    }

    pub(super) fn plan(&self) -> Value {
        let entries = self
            .phases
            .iter()
            .map(|phase| json!({"content": phase.name, "priority": "medium", "status": phase.status}))
            .collect::<Vec<_>>();
        json!({"sessionUpdate": "plan", "entries": entries})
    }

    fn set_phase(&mut self, execution_id: &str, name: &str, status: &'static str) {
        match self
            .phases
            .iter_mut()
            .find(|phase| phase.execution_id == execution_id)
        {
            Some(phase) => phase.status = status,
            None => self.phases.push(Phase {
                execution_id: execution_id.to_owned(),
                name: name.to_owned(),
                status,
                instance: false,
            }),
        }
    }
}

fn reply_text(content: &str) -> Option<String> {
    match serde_json::from_str::<Value>(content) {
        Ok(value) if value.get("requested_tools").is_some() => None,
        Ok(value) if value["type"] == "string" => value["value"].as_str().map(str::to_owned),
        _ => Some(content.to_owned()),
    }
}

fn chunk(text: &str) -> Value {
    json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}})
}

fn tool_call_id(payload: &Value) -> Value {
    payload
        .get("attempt_id")
        .unwrap_or(&payload["tool_id"])
        .clone()
}

fn tool_update(payload: &Value, status: &str) -> Value {
    json!({"sessionUpdate": "tool_call_update", "toolCallId": tool_call_id(payload), "status": status})
}
