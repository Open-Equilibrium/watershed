use serde_json::Value;

use super::{Turn, TurnFile, TurnStatus};
use crate::event::{
    ApprovalOption, ItemDetail, ItemKind, ItemStatus, PendingRequest, Question, Request, RequestId,
    StepStatus, TaskStep, TurnId, Usage,
};
use crate::git::count;
use crate::t3::{Projection, millis, prompt_of};

pub(super) fn active(run: &Value) -> bool {
    matches!(
        run["status"].as_str(),
        Some("starting" | "running" | "waiting")
    )
}

pub(super) fn run_id(runs: &[Value], matches: impl Fn(&Value) -> bool) -> Option<Value> {
    runs.iter().find(|r| matches(r)).map(|r| r["id"].clone())
}

pub(super) fn ready_checkpoint(projection: &Projection, number: u64) -> Option<&Value> {
    projection.checkpoints.iter().find(|c| {
        c["status"] == "ready"
            && if number == 0 {
                c["ordinalWithinScope"] == 0 && c["appRunOrdinal"].is_null()
            } else {
                c["appRunOrdinal"].as_u64() == Some(number)
            }
    })
}

pub(super) fn predecessor(projection: &Projection, turn: u64) -> u64 {
    projection
        .runs
        .iter()
        .filter(|run| run["status"] != "rolled_back")
        .filter_map(|run| run["ordinal"].as_u64())
        .filter(|&ordinal| ordinal < turn && ready_checkpoint(projection, ordinal).is_some())
        .max()
        .unwrap_or(0)
}

pub(super) fn turn_status(run: &Value) -> TurnStatus {
    match run["status"].as_str() {
        Some("queued" | "preparing") => TurnStatus::Queued,
        Some("completed") => TurnStatus::Completed,
        Some("interrupted" | "failed" | "cancelled") => TurnStatus::Aborted,
        _ => TurnStatus::Running,
    }
}

pub(super) fn turns(projection: &Projection) -> Vec<Turn> {
    let prompts = projection.prompts();
    projection
        .runs
        .iter()
        .filter(|run| run["status"] != "rolled_back")
        .map(|run| {
            let number = run["ordinal"].as_u64().unwrap_or_default();
            let checkpoint = ready_checkpoint(projection, number);
            let files: Vec<TurnFile> = checkpoint
                .and_then(|c| c["files"].as_array())
                .into_iter()
                .flatten()
                .map(|file| TurnFile {
                    path: file["path"].as_str().unwrap_or_default().to_owned(),
                    kind: file["kind"].as_str().unwrap_or("modified").to_owned(),
                    added: count(&file["additions"]),
                    removed: count(&file["deletions"]),
                })
                .collect();
            Turn {
                number,
                status: turn_status(run),
                prompt: prompt_of(&prompts, run).to_owned(),
                started_ms: millis(&run["startedAt"]),
                ended_ms: millis(&run["completedAt"]),
                added: files.iter().map(|f| f.added).sum(),
                removed: files.iter().map(|f| f.removed).sum(),
                files,
                checkpoint: checkpoint.is_some(),
            }
        })
        .collect()
}

pub(super) fn pending(projection: &Projection, item: &Value) -> Option<PendingRequest> {
    let id = &item["requestId"];
    let waiting = projection
        .runtime_requests
        .iter()
        .any(|r| &r["id"] == id && r["status"] == "pending");
    if !waiting {
        return None;
    }
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    let request = match item["type"].as_str()? {
        "approval_request" => {
            let mut options: Vec<ApprovalOption> = item["options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|o| ApprovalOption {
                    decision: text(&o["decision"]),
                    label: text(&o["label"]),
                    warning: o["warning"].as_str().map(str::to_owned),
                })
                .collect();
            if options.is_empty() {
                options = [
                    ("cancel", "Cancel"),
                    ("decline", "Decline"),
                    ("acceptForSession", "Always allow this session"),
                    ("accept", "Approve"),
                ]
                .map(|(decision, label)| ApprovalOption {
                    decision: decision.to_owned(),
                    label: label.to_owned(),
                    warning: None,
                })
                .to_vec();
            }
            Request::Approval {
                kind: text(&item["requestKind"]),
                prompt: text(&item["prompt"]),
                reason: item["reason"].as_str().map(str::to_owned),
                options,
            }
        }
        "user_input_request" => Request::Questions(
            item["questions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|q| Question {
                    id: text(&q["id"]),
                    header: text(&q["header"]),
                    question: text(&q["question"]),
                    options: q["options"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|o| (text(&o["label"]), text(&o["description"])))
                        .collect(),
                    multi_select: q["multiSelect"].as_bool().unwrap_or_default(),
                    custom: q["allowCustomAnswer"].as_bool().unwrap_or_default(),
                })
                .collect(),
        ),
        _ => return None,
    };
    Some(PendingRequest {
        id: RequestId(text(id)),
        turn: turn_of(projection, item),
        request,
    })
}

pub(super) fn turn_of(projection: &Projection, record: &Value) -> TurnId {
    let run = if record["runId"].is_string() {
        record["runId"].clone()
    } else {
        projection
            .attempts
            .iter()
            .find(|a| a["id"] == record["runAttemptId"])
            .map_or(Value::Null, |a| a["runId"].clone())
    };
    TurnId(
        projection
            .run(&run)
            .and_then(|r| r["ordinal"].as_u64())
            .unwrap_or_default(),
    )
}

pub(super) fn text_of(item: &Value) -> &str {
    let text = ["text", "output", "aggregatedOutput", "message"]
        .iter()
        .find_map(|key| item[*key].as_str())
        .or_else(|| item["failure"]["message"].as_str())
        .unwrap_or_default();
    if opening(item).is_some() {
        crate::agents::spoken(text).unwrap_or_default()
    } else {
        text
    }
}

fn opening(item: &Value) -> Option<u64> {
    if item["type"] != "user_message" {
        return None;
    }
    crate::agents::opening_generation(item["messageId"].as_str()?)
}

fn message_detail(item: &Value) -> ItemDetail {
    if let Some(task) = item["scheduledTaskId"].as_str() {
        ItemDetail::Routine {
            task: task.to_owned(),
        }
    } else if let Some(generation) = opening(item) {
        ItemDetail::Opening {
            handoff: generation > 0,
        }
    } else {
        ItemDetail::None
    }
}

pub(super) fn status_of(item: &Value) -> ItemStatus {
    if failed(item) {
        ItemStatus::Failed
    } else {
        ItemStatus::Completed
    }
}

pub(crate) fn failed(item: &Value) -> bool {
    item["status"] == "failed" || item["exitCode"].as_i64().is_some_and(|code| code != 0)
}

pub(crate) fn duration(item: &Value) -> Option<u64> {
    millis(&item["completedAt"])?.checked_sub(millis(&item["startedAt"])?)
}

pub(crate) fn describe(item: &Value) -> Option<(ItemKind, ItemDetail)> {
    let line = |key: &str| {
        item[key]
            .as_str()
            .map(|s| s.lines().next().unwrap_or_default().to_owned())
    };
    let tool = |name: String, target: Option<String>| ItemDetail::Tool {
        name,
        target: target.unwrap_or_default(),
    };
    Some(match item["type"].as_str()? {
        "user_message" => (ItemKind::UserMessage, message_detail(item)),
        "assistant_message" => (ItemKind::AssistantMessage, ItemDetail::None),
        "reasoning" => (ItemKind::Reasoning, ItemDetail::None),
        "command_execution" => (
            ItemKind::CommandExecution,
            tool("bash".to_owned(), line("input").or_else(|| line("title"))),
        ),
        "file_change" => (
            ItemKind::FileChange,
            tool("edit".to_owned(), line("fileName")),
        ),
        "error" => (ItemKind::Error, ItemDetail::None),
        "todo_list" => (
            ItemKind::TaskList,
            ItemDetail::Tasks {
                steps: item["steps"]
                    .as_array()?
                    .iter()
                    .map(|step| TaskStep {
                        text: step["text"].as_str().unwrap_or_default().to_owned(),
                        status: match step["status"].as_str() {
                            Some("completed") => StepStatus::Completed,
                            Some("running") => StepStatus::Running,
                            _ => StepStatus::Pending,
                        },
                    })
                    .collect(),
            },
        ),
        "dynamic_tool" | "file_search" | "web_search" => (
            ItemKind::DynamicToolCall,
            tool(
                line("title")
                    .unwrap_or_else(|| item["type"].as_str().unwrap_or_default().to_owned()),
                line("input"),
            ),
        ),
        _ => return None,
    })
}

pub(crate) fn usage_of(turn: &Value) -> Option<Usage> {
    let u = &turn["turnTokenUsage"];
    if u["usageStatus"] != "complete" {
        return None;
    }
    let cache_read = u["cachedInputTokens"].as_u64().unwrap_or_default();
    let cache_write = u["cacheCreationTokens"].as_u64().unwrap_or_default();
    Some(Usage {
        input: u["inputTokens"]
            .as_u64()?
            .saturating_sub(cache_read + cache_write),
        output: u["outputTokens"].as_u64()?,
        cache_read,
        cache_write,
        reasoning: u["reasoningTokens"].as_u64().unwrap_or_default(),
    })
}
