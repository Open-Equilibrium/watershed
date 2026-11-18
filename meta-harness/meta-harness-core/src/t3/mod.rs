mod cli;
mod client;
mod live;
mod projection;
mod server;

pub use live::Watch;

pub(crate) use cli::{link, links, pairing_code, revoke_session, sessions, unlink};
pub(crate) use client::{Client, Failure};
pub(crate) use projection::{Change, Projection, prompt_of};
pub(crate) use server::Server;
pub use server::t3_version;

use serde_json::Value;

pub(crate) fn fresh_id(prefix: &str) -> String {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).expect("the operating system provides random bytes");
    format!("{prefix}-{}", crate::util::hex(&bytes))
}

pub(crate) fn iso(ms: u64) -> String {
    let ms = i64::try_from(ms).unwrap_or(i64::MAX);
    let (days, day_ms) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        day_ms / 3_600_000,
        day_ms / 60_000 % 60,
        day_ms / 1_000 % 60,
        day_ms % 1_000
    )
}

pub(crate) fn millis(value: &Value) -> Option<u64> {
    let (date, time) = value.as_str()?.strip_suffix('Z')?.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
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
