use std::future::Future;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};

pub(crate) const TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) const PING: Duration = Duration::from_secs(30);
pub(crate) const IDLE: Duration = Duration::from_secs(90);

pub(crate) fn pings() -> tokio::time::Interval {
    tokio::time::interval_at(tokio::time::Instant::now() + PING, PING)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn base64url_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().enumerate().fold(0u32, |bits, (index, byte)| {
            bits | u32::from(*byte) << (16 - 8 * index)
        });
        for index in 0..=chunk.len() {
            text.push(char::from(
                ALPHABET[(bits >> (18 - 6 * index) & 63) as usize],
            ));
        }
    }
    text
}

pub(crate) fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let (mut bits, mut count, mut bytes) = (0u32, 0u32, Vec::new());
    for character in text.bytes() {
        let value = match character {
            b'A'..=b'Z' => character - b'A',
            b'a'..=b'z' => character - b'a' + 26,
            b'0'..=b'9' => character - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    Some(bytes)
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) async fn until(limit: Duration, poll: Duration, done: impl Fn() -> bool) -> bool {
    tokio::time::timeout(limit, async {
        while !done() {
            tokio::time::sleep(poll).await;
        }
    })
    .await
    .is_ok()
}

pub fn rfc3339_millis(text: &str) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let [year, month, day] = date.split('-').collect::<Vec<_>>()[..] else {
        return None;
    };
    let (clock, rest) = time.split_at_checked(8)?;
    let [hour, minute, second] = clock.split(':').collect::<Vec<_>>()[..] else {
        return None;
    };
    let (year, month, day) = (field(year, 4)?, field(month, 2)?, field(day, 2)?);
    let (hour, minute, second) = (field(hour, 2)?, field(minute, 2)?, field(second, 2)?);
    let zone_at = rest.find(['Z', '+', '-'])?;
    let (fraction, zone) = rest.split_at(zone_at);
    let fraction = match fraction.strip_prefix('.') {
        None if fraction.is_empty() => 0,
        Some(digits) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
            format!("{digits:0<3}")[..3].parse::<i64>().ok()?
        }
        _ => return None,
    };
    let offset = match zone {
        "Z" => 0,
        _ => {
            let (sign, hours_minutes) = zone.split_at_checked(1)?;
            let (hours, minutes) = hours_minutes.split_once(':')?;
            let (hours, minutes) = (field(hours, 2)?, field(minutes, 2)?);
            if hours > 23 || minutes > 59 {
                return None;
            }
            let seconds = hours * 3600 + minutes * 60;
            match sign {
                "+" => seconds,
                "-" => -seconds,
                _ => return None,
            }
        }
    };
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    Some(seconds * 1000 + fraction)
}

fn field(text: &str, width: usize) -> Option<i64> {
    (text.len() == width && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub(crate) async fn accept_each<F, Fut>(listener: TcpListener, serve: F)
where
    F: Fn(TcpStream) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((socket, _)) => {
                    connections.spawn(serve(socket));
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}
