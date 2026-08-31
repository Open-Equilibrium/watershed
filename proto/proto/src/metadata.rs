use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
/// Invalid field in one event envelope's stream-independent metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventMetadataError {
    pub(crate) field: &'static str,
    pub(crate) requirement: &'static str,
}

impl EventMetadataError {
    /// Returns the invalid envelope field.
    pub const fn field(self) -> &'static str {
        self.field
    }
}

impl fmt::Display for EventMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.field, self.requirement)
    }
}

impl std::error::Error for EventMetadataError {}

/// v0 normalized runtime event types.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum EventType {
    /// A session started.
    SessionStarted,
    /// A session paused before reaching a terminal state.
    SessionPaused,
    /// A previously persisted session resumed.
    SessionResumed,
    /// A session completed successfully.
    SessionCompleted,
    /// A session reached a failed terminal state.
    SessionFailed,
    /// A flow invocation started.
    FlowStarted,
    /// A flow invocation completed successfully.
    FlowCompleted,
    /// A flow invocation failed.
    FlowFailed,
    /// Runtime entered a phase.
    PhaseEntered,
    /// A phase iteration completed with a validated result.
    PhaseCompleted,
    /// A phase iteration failed without looping or transitioning.
    PhaseFailed,
    /// Assistant message content chunk.
    MessageDelta,
    /// Assistant message completed.
    MessageCompleted,
    /// Tool invocation started.
    ToolStarted,
    /// Tool invocation emitted structured progress.
    ToolProgress,
    /// Tool invocation completed successfully.
    ToolCompleted,
    /// Tool invocation failed.
    ToolFailed,
    /// Tool invocation exceeded its runtime limit.
    ToolTimedOut,
    /// Artifact metadata was recorded.
    ArtifactLogged,
    /// Human or external attention was requested.
    AttentionRequested,
    /// Runtime metric sample was emitted.
    MetricSample,
    /// Runtime error event.
    Error,
}

impl EventType {
    /// Every v0 event type in canonical protocol order.
    pub const ALL: [Self; 22] = [
        Self::SessionStarted,
        Self::SessionPaused,
        Self::SessionResumed,
        Self::SessionCompleted,
        Self::SessionFailed,
        Self::FlowStarted,
        Self::FlowCompleted,
        Self::FlowFailed,
        Self::PhaseEntered,
        Self::PhaseCompleted,
        Self::PhaseFailed,
        Self::MessageDelta,
        Self::MessageCompleted,
        Self::ToolStarted,
        Self::ToolProgress,
        Self::ToolCompleted,
        Self::ToolFailed,
        Self::ToolTimedOut,
        Self::ArtifactLogged,
        Self::AttentionRequested,
        Self::MetricSample,
        Self::Error,
    ];

    /// Returns whether this event belongs to one Flow and therefore requires `flow_id`.
    pub const fn requires_flow_id(self) -> bool {
        matches!(
            self,
            Self::FlowStarted
                | Self::FlowCompleted
                | Self::FlowFailed
                | Self::PhaseEntered
                | Self::PhaseCompleted
                | Self::PhaseFailed
                | Self::MessageDelta
                | Self::MessageCompleted
                | Self::ToolStarted
                | Self::ToolProgress
                | Self::ToolCompleted
                | Self::ToolFailed
                | Self::ToolTimedOut
        )
    }
}

impl EventType {
    /// Returns the stable protocol string for this event type.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStarted => "session.started",
            Self::SessionPaused => "session.paused",
            Self::SessionResumed => "session.resumed",
            Self::SessionCompleted => "session.completed",
            Self::SessionFailed => "session.failed",
            Self::FlowStarted => "flow.started",
            Self::FlowCompleted => "flow.completed",
            Self::FlowFailed => "flow.failed",
            Self::PhaseEntered => "phase.entered",
            Self::PhaseCompleted => "phase.completed",
            Self::PhaseFailed => "phase.failed",
            Self::MessageDelta => "message.delta",
            Self::MessageCompleted => "message.completed",
            Self::ToolStarted => "tool.started",
            Self::ToolProgress => "tool.progress",
            Self::ToolCompleted => "tool.completed",
            Self::ToolFailed => "tool.failed",
            Self::ToolTimedOut => "tool.timed_out",
            Self::ArtifactLogged => "artifact.logged",
            Self::AttentionRequested => "attention.requested",
            Self::MetricSample => "metric.sample",
            Self::Error => "error",
        }
    }
}

impl TryFrom<&str> for EventType {
    type Error = UnknownEventType;

    fn try_from(value: &str) -> Result<Self, UnknownEventType> {
        Self::ALL
            .into_iter()
            .find(|event_type| event_type.as_str() == value)
            .ok_or_else(|| UnknownEventType(value.to_owned()))
    }
}

impl Serialize for EventType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EventType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value.as_str()).map_err(serde::de::Error::custom)
    }
}

/// Error returned when a string is not a known v0 event type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownEventType(String);

impl fmt::Display for UnknownEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown event type: {}", self.0)
    }
}

impl std::error::Error for UnknownEventType {}

/// Maximum UTF-8 bytes in a protocol session identifier.
pub const MAX_SESSION_ID_BYTES: usize = 128;

/// Returns whether a value is a lowercase path-safe session id.
pub fn is_valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SESSION_ID_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
        && !is_windows_dos_device_basename(value)
}

fn is_windows_dos_device_basename(value: &str) -> bool {
    matches!(value, "con" | "prn" | "aux" | "nul")
        || value
            .strip_prefix("com")
            .or_else(|| value.strip_prefix("lpt"))
            .is_some_and(|suffix| matches!(suffix.as_bytes(), [b'1'..=b'9']))
}

/// Parses the protocol's canonical RFC 3339 UTC `Z` form to Unix seconds.
pub fn parse_rfc3339_utc_timestamp(value: &str) -> Option<i64> {
    let value = value.strip_suffix('Z')?;
    let (date, time) = value.split_once('T')?;

    let mut date_parts = date.split('-');
    let year = date_parts.next().and_then(|part| parse_digits(part, 4))?;
    let month = date_parts.next().and_then(|part| parse_digits(part, 2))?;
    let day = date_parts.next().and_then(|part| parse_digits(part, 2))?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }

    let mut time_parts = time.split(':');
    let hour = time_parts.next().and_then(|part| parse_digits(part, 2))?;
    let minute = time_parts.next().and_then(|part| parse_digits(part, 2))?;
    let second_part = time_parts.next()?;
    if time_parts.next().is_some() {
        return None;
    }
    let (second, fraction) = second_part
        .split_once('.')
        .map_or((second_part, None), |(second, fraction)| {
            (second, Some(fraction))
        });
    let second = parse_digits(second, 2)?;
    if fraction
        .is_some_and(|value| value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let days = days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    Some(
        days.saturating_mul(86_400)
            .saturating_add(i64::from(hour) * 3_600)
            .saturating_add(i64::from(minute) * 60)
            .saturating_add(i64::from(second)),
    )
}

/// Formats Unix seconds as the protocol's canonical four-digit UTC `Z` form.
pub fn format_rfc3339_utc_timestamp(seconds: i64) -> Option<String> {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    if !(0..=9_999).contains(&year) {
        return None;
    }
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn parse_digits(value: &str, len: usize) -> Option<u16> {
    (value.len() == len && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn days_in_month(year: u16, month: u16) -> u16 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month, day)
}
