use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::Thread;
use crate::t3::{Watch, millis};

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LocalServer {
    pub host: String,
    pub port: u16,
    pub url: String,
    pub process: Option<String>,
    pub pid: Option<u32>,
    pub terminal: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Never,
    Running,
    Succeeded,
    Failed,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleKind {
    Interval,
    FixedTime,
    Once,
    Webhook,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub kind: ScheduleKind,
    pub every_ms: Option<u64>,
    pub time_of_day: Option<String>,
    pub weekdays: Vec<u8>,
    pub run_at: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Automation {
    pub id: String,
    pub title: String,
    pub schedule: Schedule,
    pub enabled: bool,
    pub next_run_ms: Option<u64>,
    pub last_run: RunStatus,
    pub blocked_by_policy: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    Ahead,
    On,
    Under,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UsageWindow {
    pub label: String,
    pub remaining_percent: i64,
    pub resets_at_ms: Option<u64>,
    pub pace: Option<Pace>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UsageLimits {
    pub notice: Option<String>,
    pub windows: Vec<UsageWindow>,
}

pub(crate) fn local_servers(list: &Value, thread: &str) -> Vec<LocalServer> {
    list["servers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["terminal"]["threadId"] == thread || s["agent"]["threadId"] == thread)
        .filter_map(|s| {
            Some(LocalServer {
                host: s["host"].as_str()?.to_owned(),
                port: u16::try_from(s["port"].as_u64()?).ok()?,
                url: s["url"].as_str()?.to_owned(),
                process: s["processName"].as_str().map(str::to_owned),
                pid: s["pid"].as_u64().and_then(|pid| u32::try_from(pid).ok()),
                terminal: !s["terminal"].is_null(),
            })
        })
        .collect()
}

pub(crate) fn automations(list: &Value, thread: &str) -> Vec<Automation> {
    bound(list, thread).filter_map(routine).collect()
}

fn bound<'a>(list: &'a Value, thread: &'a str) -> impl Iterator<Item = &'a Value> {
    list["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(move |t| t["threadId"] == thread)
}

pub(crate) fn routine(t: &Value) -> Option<Automation> {
    let schedule = &t["schedule"];
    Some(Automation {
        id: t["id"].as_str()?.to_owned(),
        title: t["title"].as_str()?.to_owned(),
        schedule: Schedule {
            kind: match schedule["type"].as_str()? {
                "interval" => ScheduleKind::Interval,
                "fixed_time" => ScheduleKind::FixedTime,
                "once" => ScheduleKind::Once,
                "webhook" => ScheduleKind::Webhook,
                _ => return None,
            },
            every_ms: schedule["everyMs"].as_u64(),
            time_of_day: schedule["timeOfDay"].as_str().map(str::to_owned),
            weekdays: schedule["weekdays"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|day| u8::try_from(day.as_u64()?).ok())
                .collect(),
            run_at: schedule["runAt"].as_str().map(str::to_owned),
        },
        enabled: t["enabled"].as_bool().unwrap_or_default(),
        next_run_ms: millis(&t["nextRunAt"]),
        last_run: match t["lastRunStatus"].as_str() {
            Some("running") => RunStatus::Running,
            Some("succeeded") => RunStatus::Succeeded,
            Some("failed") => RunStatus::Failed,
            _ => RunStatus::Never,
        },
        blocked_by_policy: t["blockedByPolicy"].as_str().map(str::to_owned),
    })
}

pub(crate) fn usage_limits(config: &Value, instance: &str, now_ms: u64) -> Option<UsageLimits> {
    let provider = config["providers"]
        .as_array()?
        .iter()
        .find(|p| p["instanceId"] == instance)?;
    let limits = &provider["usageLimits"];
    if limits.is_null() {
        return None;
    }
    let windows = limits["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|window| usage_window(window, now_ms))
        .collect();
    Some(UsageLimits {
        notice: notice(limits),
        windows,
    })
}

fn notice(limits: &Value) -> Option<String> {
    let unavailable = &limits["unavailable"];
    let message = unavailable["message"].as_str();
    match unavailable["reason"].as_str() {
        Some("unsupported") => Some(
            message
                .unwrap_or("This account has no subscription limits.")
                .to_owned(),
        ),
        Some("probeFailed") => Some(message.unwrap_or("Could not read limits.").to_owned()),
        _ => limits["windows"]
            .as_array()
            .is_none_or(Vec::is_empty)
            .then(|| "No limits reported.".to_owned()),
    }
}

fn usage_window(window: &Value, now_ms: u64) -> Option<UsageWindow> {
    let used = window["usedPercent"].as_f64()?.clamp(0.0, 100.0);
    let resets_at = millis(&window["resetsAt"]);
    Some(UsageWindow {
        label: window["label"].as_str()?.to_owned(),
        remaining_percent: (100.0 - used).round() as i64,
        resets_at_ms: resets_at.filter(|at| *at > now_ms),
        pace: pace(window, used, resets_at, now_ms),
    })
}

fn pace(window: &Value, used: f64, resets_at: Option<u64>, now_ms: u64) -> Option<Pace> {
    let minutes = window["windowDurationMins"].as_u64().filter(|m| *m > 0)?;
    let length = minutes as f64 * 60_000.0;
    let resets_at = resets_at? as f64;
    let elapsed = ((length - (resets_at - now_ms as f64)) / length).clamp(0.0, 1.0);
    let gap = used - elapsed * 100.0;
    Some(if gap > 5.0 {
        Pace::Ahead
    } else if gap < -5.0 {
        Pace::Under
    } else {
        Pace::On
    })
}

fn schedule_json(schedule: &Schedule) -> Value {
    match schedule.kind {
        ScheduleKind::Interval => json!({"type": "interval", "everyMs": schedule.every_ms}),
        ScheduleKind::FixedTime if schedule.weekdays.is_empty() => {
            json!({"type": "fixed_time", "timeOfDay": schedule.time_of_day})
        }
        ScheduleKind::FixedTime => json!({
            "type": "fixed_time",
            "timeOfDay": schedule.time_of_day,
            "weekdays": schedule.weekdays,
        }),
        ScheduleKind::Once => json!({"type": "once", "runAt": schedule.run_at}),
        ScheduleKind::Webhook => json!({"type": "webhook"}),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or_default()
}

impl Thread {
    pub async fn servers(&self) -> Result<Watch<Vec<LocalServer>>, String> {
        let thread = self.id.clone();
        let stream = self
            .client
            .stream("subscribeDiscoveredLocalServers", json!({}))
            .await?;
        Watch::follow(stream, move |list| Some(local_servers(list, &thread))).await
    }

    pub async fn stop_server(&self, pid: u32, port: u16) -> Result<(), String> {
        self.client
            .call("preview.stopLocalServer", json!({"pid": pid, "port": port}))
            .await
            .map(|_| ())
    }

    pub async fn automations(&self) -> Result<Watch<Vec<Automation>>, String> {
        let thread = self.id.clone();
        let stream = self
            .client
            .stream("scheduledTasks.subscribe", json!({}))
            .await?;
        Watch::follow(stream, move |list| Some(automations(list, &thread))).await
    }

    pub async fn set_automation_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        self.client
            .call(
                "scheduledTasks.setEnabled",
                json!({"id": id, "enabled": enabled}),
            )
            .await
            .map(|_| ())
    }

    pub async fn run_automation_now(&self, id: &str) -> Result<(), String> {
        self.client
            .call("scheduledTasks.runNow", json!({"id": id}))
            .await
            .map(|_| ())
    }

    pub(crate) async fn add_routine(
        &self,
        agent: &crate::agents::Agent,
        routine: &crate::agents::NewRoutine,
    ) -> Result<crate::agents::AddedRoutine, String> {
        let project = self.state().projection.thread["projectId"].clone();
        let answer = self
            .client
            .call(
                "scheduledTasks.upsert",
                json!({
                    "title": routine.title,
                    "prompt": routine.prompt,
                    "enabled": true,
                    "schedule": schedule_json(&routine.schedule),
                    "projectId": project,
                    "threadId": self.id,
                    "threadMode": "continue",
                    "workspaceStrategy": {"type": "root"},
                    "modelSelection": agent.model.to_t3(),
                    "runtimeMode": agent.permissions.mode(),
                    "interactionMode": "default",
                }),
            )
            .await?;
        let task = &answer["task"];
        Ok(crate::agents::AddedRoutine {
            id: task["id"]
                .as_str()
                .ok_or("T3 saved the routine without an id")?
                .to_owned(),
            webhook: task["webhook"]["url"]
                .as_str()
                .or_else(|| task["webhook"]["path"].as_str())
                .map(str::to_owned),
        })
    }

    pub(crate) async fn remove_routine(&self, id: &str) -> Result<(), String> {
        self.client
            .call("scheduledTasks.delete", json!({"id": id}))
            .await
            .map(|_| ())
    }

    pub(crate) async fn routine_tasks(&self) -> Result<Vec<Value>, String> {
        let mut stream = self
            .client
            .stream("scheduledTasks.subscribe", json!({}))
            .await?;
        let list = stream.recv().await.ok_or("T3 sent no routines")?;
        Ok(bound(&list, &self.id)
            .map(crate::agents::routine_copy)
            .collect())
    }

    pub(crate) async fn save_routine(&self, copy: &Value, existing: bool) -> Result<Value, String> {
        let mut task = copy.clone();
        task["threadId"] = json!(self.id);
        task["threadMode"] = json!("continue");
        if existing {
            task["requireExisting"] = json!(true);
        }
        let answer = self.client.call("scheduledTasks.upsert", task).await?;
        Ok(crate::agents::routine_copy(&answer["task"]))
    }

    pub(crate) async fn rebind_routines(&self, to: &Thread) -> Result<(), String> {
        for task in self.routine_tasks().await? {
            to.save_routine(&task, true).await?;
        }
        Ok(())
    }

    pub(crate) async fn retune_routines(
        &self,
        model: &crate::ModelChoice,
        permissions: crate::Permissions,
    ) -> Result<(), String> {
        for mut task in self.routine_tasks().await? {
            task["modelSelection"] = model.to_t3();
            task["runtimeMode"] = json!(permissions.mode());
            self.save_routine(&task, true).await?;
        }
        Ok(())
    }

    pub async fn usage_limits(&self) -> Result<Option<UsageLimits>, String> {
        let config = self.client.call("server.getConfig", json!({})).await?;
        let instance = self.state().projection.thread["modelSelection"]["instanceId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        Ok(usage_limits(&config, &instance, now_ms()))
    }
}
