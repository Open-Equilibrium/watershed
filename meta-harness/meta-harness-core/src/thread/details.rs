//! What a thread's details card shows beside its chat, as T3 0.1.9 reports it: local servers its
//! terminals and agent started, scheduled tasks bound to it, and its provider's usage limits.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::Thread;
use crate::t3::{Watch, millis};

/// A local server a terminal or the agent of this thread started.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LocalServer {
    /// The host, such as `localhost`.
    pub host: String,
    /// The port it listens on.
    pub port: u16,
    /// Its address.
    pub url: String,
    /// The process's name, when T3 knows it.
    pub process: Option<String>,
    /// The process id, which stopping the server needs.
    pub pid: Option<u32>,
    /// Whether a terminal started it rather than the agent.
    pub terminal: bool,
}

/// How a scheduled task's last run ended, or that it never ran.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// It never ran.
    Never,
    /// It runs now.
    Running,
    /// Its last run succeeded.
    Succeeded,
    /// Its last run failed.
    Failed,
}

/// When a scheduled task runs.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleKind {
    /// Every `every_ms`.
    Interval,
    /// At `time_of_day` on `weekdays`.
    FixedTime,
    /// Once, at `run_at`.
    Once,
    /// On each webhook request.
    Webhook,
}

/// A scheduled task's schedule, with the fields of its kind.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    /// The kind.
    pub kind: ScheduleKind,
    /// The interval, for an interval schedule.
    pub every_ms: Option<u64>,
    /// The time of day, `HH:MM`, for a fixed-time schedule.
    pub time_of_day: Option<String>,
    /// The weekdays, 0 for Sunday; empty for every day.
    pub weekdays: Vec<u8>,
    /// The ISO 8601 time with its offset, for a one-time schedule.
    pub run_at: Option<String>,
}

/// A scheduled task bound to a thread, as Automations shows it.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Automation {
    /// T3's task id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// When it runs.
    pub schedule: Schedule,
    /// Whether it runs at all.
    pub enabled: bool,
    /// When it runs next, in milliseconds since the Unix epoch.
    pub next_run_ms: Option<u64>,
    /// How its last run ended.
    pub last_run: RunStatus,
    /// Why policy keeps it from running, whatever `enabled` says.
    pub blocked_by_policy: Option<String>,
}

/// Where a usage window stands against the time it covers, as T3 names it.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    /// Used faster than time passed.
    Ahead,
    /// Used about as fast as time passed.
    On,
    /// Used slower than time passed.
    Under,
}

/// One window of a provider's usage limits.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UsageWindow {
    /// Its label, such as `5h`.
    pub label: String,
    /// The percentage left, from 0 to 100.
    pub remaining_percent: i64,
    /// When it resets, when that is still ahead.
    pub resets_at_ms: Option<u64>,
    /// Its pace, when T3 can tell.
    pub pace: Option<Pace>,
}

/// A provider's usage limits: its windows, or why there are none.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UsageLimits {
    /// Why the windows are missing or empty, as T3 words it.
    pub notice: Option<String>,
    /// The windows, in T3's order.
    pub windows: Vec<UsageWindow>,
}

/// The servers of `list` (T3's discovered servers) that a terminal or the agent of `thread`
/// started.
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

/// The scheduled tasks of `list` (T3's task list) bound to `thread`.
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

/// The usage limits of the provider instance `instance` in T3's `server.getConfig` answer `config`,
/// at `now_ms`; `None` when that provider reports none.
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

/// Why `limits` has no windows to show, as T3 words it; the provider's own message wins.
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

/// The pace of a window: how far its used share runs ahead of the share of its time that passed.
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
    /// The local servers this thread's terminals and agent started, as T3 discovers them. The
    /// watch holds T3's discovery subscription open; dropping it closes that.
    pub async fn servers(&self) -> Result<Watch<Vec<LocalServer>>, String> {
        let thread = self.id.clone();
        let stream = self
            .client
            .stream("subscribeDiscoveredLocalServers", json!({}))
            .await?;
        Watch::follow(stream, move |list| Some(local_servers(list, &thread))).await
    }

    /// Stops the local server `pid` on `port`.
    pub async fn stop_server(&self, pid: u32, port: u16) -> Result<(), String> {
        self.client
            .call("preview.stopLocalServer", json!({"pid": pid, "port": port}))
            .await
            .map(|_| ())
    }

    /// The scheduled tasks bound to this thread, kept current as T3 changes them.
    pub async fn automations(&self) -> Result<Watch<Vec<Automation>>, String> {
        let thread = self.id.clone();
        let stream = self
            .client
            .stream("scheduledTasks.subscribe", json!({}))
            .await?;
        Watch::follow(stream, move |list| Some(automations(list, &thread))).await
    }

    /// Pauses or resumes the scheduled task `id`.
    pub async fn set_automation_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        self.client
            .call(
                "scheduledTasks.setEnabled",
                json!({"id": id, "enabled": enabled}),
            )
            .await
            .map(|_| ())
    }

    /// Runs the scheduled task `id` now.
    pub async fn run_automation_now(&self, id: &str) -> Result<(), String> {
        self.client
            .call("scheduledTasks.runNow", json!({"id": id}))
            .await
            .map(|_| ())
    }

    /// The usage limits of the provider this thread runs on, or `None` when it reports none.
    pub async fn usage_limits(&self) -> Result<Option<UsageLimits>, String> {
        let config = self.client.call("server.getConfig", json!({})).await?;
        let instance = self.state().projection.thread["modelSelection"]["instanceId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        Ok(usage_limits(&config, &instance, now_ms()))
    }
}
