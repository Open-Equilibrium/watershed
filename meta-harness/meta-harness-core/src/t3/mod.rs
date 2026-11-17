//! T3 Code's side: its server, its client protocol and a thread's projection (ADR-0281).

mod cli;
mod client;
mod live;
mod projection;
mod server;

pub use live::Watch;

pub(crate) use cli::{link, links, pairing_code, unlink};
pub(crate) use client::Client;
pub(crate) use projection::{Change, Projection, prompt_of};
pub(crate) use server::Server;
pub use server::t3_version;

use serde_json::Value;

/// A new id for a T3 record or command: `prefix-` and 24 random hex digits.
pub(crate) fn fresh_id(prefix: &str) -> String {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).expect("the operating system provides random bytes");
    format!("{prefix}-{}", crate::util::hex(&bytes))
}

/// Milliseconds since the Unix epoch of an ISO 8601 UTC time such as
/// `2026-11-15T14:26:04.246Z`.
pub(crate) fn millis(value: &Value) -> Option<u64> {
    let (date, time) = value.as_str()?.strip_suffix('Z')?.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    // Days from the civil date (Howard Hinnant's algorithm).
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + (153 * m + 2) / 5 + day - 1;
    let days = era * 146_097 + doe - 719_468;
    let mut t = time.split(':');
    let (h, min) = (
        t.next()?.parse::<i64>().ok()?,
        t.next()?.parse::<i64>().ok()?,
    );
    let secs: f64 = t.next()?.parse().ok()?;
    let ms = ((days * 24 + h) * 60 + min) * 60_000 + (secs * 1000.0).round() as i64;
    u64::try_from(ms).ok()
}
