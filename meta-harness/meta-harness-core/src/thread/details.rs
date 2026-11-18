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
    list["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["threadId"] == thread)
        .filter_map(|t| {
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
        })
        .collect()
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

    pub async fn usage_limits(&self) -> Result<Option<UsageLimits>, String> {
        let config = self.client.call("server.getConfig", json!({})).await?;
        let instance = self.state().projection.thread["modelSelection"]["instanceId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        Ok(usage_limits(&config, &instance, now_ms()))
    }
}
