use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_harness_core::{
    ModelChoice, NewAgent, NewRoutine, Permissions, Schedule, ScheduleKind, remote,
};

use crate::{USAGE, print};

const MINUTE_MS: u64 = 60_000;

pub async fn agent(host: &remote::Host, args: &[&str]) -> Result<(), String> {
    match args {
        ["list"] => print(&host.agents().await?),
        ["create", name, rest @ ..] => {
            let agent = host.agent_create(new_agent(name, rest)?).await?;
            println!(
                "Created agent {}; its memory folder is {}.",
                agent.name,
                agent.memory.display()
            );
            Ok(())
        }
        ["remove", name] => {
            let memory = host.agent_remove(name).await?;
            println!(
                "Removed agent {name} and its routines; its memory stays in {}.",
                memory.display()
            );
            Ok(())
        }
        ["routines", name] => print(&host.agent_routines(name).await?),
        ["routine", "add", name, rest @ ..] => {
            let added = host.agent_routine_add(name, routine(rest)?).await?;
            match added.webhook {
                Some(webhook) => println!("Added routine {}; its webhook is {webhook}.", added.id),
                None => println!("Added routine {}.", added.id),
            }
            Ok(())
        }
        ["routine", "remove", name, id] => {
            host.agent_routine_remove(name, id).await?;
            println!("Removed routine {id}.");
            Ok(())
        }
        _ => Err(format!("unknown agent command\n{USAGE}")),
    }
}

fn new_agent(name: &str, mut args: &[&str]) -> Result<NewAgent, String> {
    let (mut root, mut model, mut permissions, mut instructions) =
        (None, None, Permissions::Supervised, String::new());
    while let [flag, value, rest @ ..] = args {
        match *flag {
            "--project" => {
                let path = std::fs::canonicalize(Path::new(value))
                    .map_err(|e| format!("The project {value} is not readable: {e}"))?;
                if !path.is_dir() {
                    return Err(format!("The project {value} is not a folder."));
                }
                root = Some(path.to_string_lossy().into_owned());
            }
            "--model" => {
                let (instance, model_id) = value.split_once('/').ok_or_else(|| {
                    format!("{value:?} is no INSTANCE/MODEL, such as codex/gpt-6.1.")
                })?;
                model = Some(ModelChoice {
                    instance: instance.to_owned(),
                    model: model_id.to_owned(),
                    options: Vec::new(),
                });
            }
            "--permissions" => {
                permissions = Permissions::from_mode(value).ok_or_else(|| {
                    format!(
                        "{value:?} is no permission mode: approval-required, auto-accept-edits, \
                         auto or full-access."
                    )
                })?;
            }
            "--instructions" => (*value).clone_into(&mut instructions),
            _ => return Err(format!("meta agent create does not take {flag}\n{USAGE}")),
        }
        args = rest;
    }
    if !args.is_empty() {
        return Err(format!(
            "meta agent create misses a value after {}",
            args[0]
        ));
    }
    Ok(NewAgent {
        name: Some(name.to_owned()),
        root: root.ok_or("meta agent create needs --project PATH")?,
        model: model.ok_or("meta agent create needs --model INSTANCE/MODEL")?,
        permissions,
        instructions,
        request: None,
    })
}

fn routine(mut args: &[&str]) -> Result<NewRoutine, String> {
    let (mut schedule, mut prompt, mut title, mut days) = (None, None, None, Vec::new());
    while !args.is_empty() {
        args = match args {
            ["--webhook", rest @ ..] => {
                choose(&mut schedule, blank(ScheduleKind::Webhook))?;
                rest
            }
            ["--every", every, rest @ ..] => {
                let every_ms = Some(duration(every)?);
                choose(
                    &mut schedule,
                    Schedule {
                        every_ms,
                        ..blank(ScheduleKind::Interval)
                    },
                )?;
                rest
            }
            ["--at", time, rest @ ..] => {
                time_of_day(time)?;
                choose(
                    &mut schedule,
                    Schedule {
                        time_of_day: Some((*time).to_owned()),
                        ..blank(ScheduleKind::FixedTime)
                    },
                )?;
                rest
            }
            ["--once", at, rest @ ..] => {
                future_instant(at)?;
                choose(
                    &mut schedule,
                    Schedule {
                        run_at: Some((*at).to_owned()),
                        ..blank(ScheduleKind::Once)
                    },
                )?;
                rest
            }
            ["--days", list, rest @ ..] => {
                days = weekdays(list)?;
                rest
            }
            ["--prompt", text, rest @ ..] => {
                prompt = Some((*text).to_owned());
                rest
            }
            ["--title", text, rest @ ..] => {
                title = Some((*text).to_owned());
                rest
            }
            [flag, ..] => {
                return Err(format!(
                    "meta agent routine add does not take {flag}\n{USAGE}"
                ));
            }
            [] => unreachable!("the loop stops on no arguments"),
        };
    }
    let mut schedule =
        schedule.ok_or("meta agent routine add needs --every, --at, --once or --webhook")?;
    if !days.is_empty() {
        if schedule.kind != ScheduleKind::FixedTime {
            return Err("--days goes only with --at".to_owned());
        }
        schedule.weekdays = days;
    }
    let prompt = prompt
        .filter(|text| !text.trim().is_empty())
        .ok_or("meta agent routine add needs a non-empty --prompt TEXT")?;
    Ok(NewRoutine {
        title: title.unwrap_or_else(|| {
            prompt
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(80)
                .collect()
        }),
        prompt,
        schedule,
    })
}

fn choose(slot: &mut Option<Schedule>, schedule: Schedule) -> Result<(), String> {
    if slot.is_some() {
        return Err(
            "meta agent routine add takes one of --every, --at, --once or --webhook".to_owned(),
        );
    }
    *slot = Some(schedule);
    Ok(())
}

fn blank(kind: ScheduleKind) -> Schedule {
    Schedule {
        kind,
        every_ms: None,
        time_of_day: None,
        weekdays: Vec::new(),
        run_at: None,
    }
}

fn duration(text: &str) -> Result<u64, String> {
    let wrong = || format!("{text:?} is no duration such as 30m, 2h or 1d.");
    let (count, minutes) = [("m", 1), ("h", 60), ("d", 24 * 60)]
        .into_iter()
        .find_map(|(unit, minutes)| Some((text.strip_suffix(unit)?, minutes)))
        .ok_or_else(wrong)?;
    let count: u64 = count.parse().map_err(|_| wrong())?;
    count
        .checked_mul(minutes * MINUTE_MS)
        .filter(|ms| *ms >= MINUTE_MS)
        .ok_or_else(|| format!("{text:?} is shorter than a minute or too long."))
}

fn time_of_day(text: &str) -> Result<(), String> {
    let valid = |part: &str, below: u32| {
        part.len() <= 2
            && part.bytes().all(|b| b.is_ascii_digit())
            && part.parse::<u32>().is_ok_and(|n| n < below)
    };
    match text.split_once(':') {
        Some((hour, minute)) if valid(hour, 24) && minute.len() == 2 && valid(minute, 60) => Ok(()),
        _ => Err(format!(
            "{text:?} is no time of day such as 07:30, with hours 00 to 23 and minutes 00 to 59."
        )),
    }
}

fn future_instant(text: &str) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    match unix_seconds(text) {
        Some(at) if at > i64::try_from(now).unwrap_or(i64::MAX) => Ok(()),
        Some(_) => Err(format!("{text:?} is not later than now.")),
        None => Err(format!(
            "{text:?} is no date and time with an offset, such as 2026-12-01T07:30:00+01:00."
        )),
    }
}

fn unix_seconds(text: &str) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let (clock, zone) = time.split_at_checked(8)?;
    let [year, month, day] = date.split('-').collect::<Vec<_>>()[..] else {
        return None;
    };
    let [hour, minute, second] = clock.split(':').collect::<Vec<_>>()[..] else {
        return None;
    };
    let (year, month, day) = (field(year, 4)?, field(month, 2)?, field(day, 2)?);
    let (hour, minute, second) = (field(hour, 2)?, field(minute, 2)?, field(second, 2)?);
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let offset = match zone {
        "Z" => 0,
        _ => {
            let (sign, hours_minutes) = zone.split_at_checked(1)?;
            let (hours, minutes) = hours_minutes.split_once(':')?;
            let seconds = field(hours, 2)? * 3600 + field(minutes, 2)? * 60;
            match sign {
                "+" => seconds,
                "-" => -seconds,
                _ => return None,
            }
        }
    };
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
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

fn weekdays(list: &str) -> Result<Vec<u8>, String> {
    list.split(',')
        .map(|day| {
            ["sun", "mon", "tue", "wed", "thu", "fri", "sat"]
                .iter()
                .position(|name| *name == day)
                .and_then(|index| u8::try_from(index).ok())
                .ok_or_else(|| {
                    format!("{day:?} is no weekday: sun, mon, tue, wed, thu, fri or sat.")
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use meta_harness_core::{Permissions, ScheduleKind};

    use super::{new_agent, routine, unix_seconds};

    #[test]
    fn an_agent_needs_a_project_and_a_model_and_asks_for_approval_by_default() {
        let project = std::env::temp_dir();
        let project = project.to_str().unwrap();

        let agent = new_agent(
            "news-bot",
            &[
                "--project",
                project,
                "--model",
                "codex/gpt-6.1",
                "--instructions",
                "Keep the news.",
            ],
        )
        .unwrap();

        assert_eq!(
            (agent.model.instance.as_str(), agent.model.model.as_str()),
            ("codex", "gpt-6.1")
        );
        assert_eq!(agent.permissions, Permissions::Supervised);
        assert_eq!(agent.instructions, "Keep the news.");
        assert!(new_agent("news-bot", &["--model", "codex/gpt-6.1"]).is_err());
        assert!(new_agent("news-bot", &["--project", project]).is_err());
        assert!(new_agent("news-bot", &["--project", project, "--model", "gpt"]).is_err());
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
        assert!(new_agent("news-bot", &["--project", file, "--model", "codex/gpt-6.1"]).is_err());
        let full = new_agent(
            "news-bot",
            &[
                "--project",
                project,
                "--model",
                "codex/gpt-6.1",
                "--permissions",
                "full-access",
            ],
        )
        .unwrap();
        assert_eq!(full.permissions, Permissions::FullAccess);
    }

    #[test]
    fn a_routine_takes_one_schedule_and_a_prompt() {
        let every =
            routine(&["--every", "2h", "--prompt", "Check the feeds.\nThen write."]).unwrap();
        assert_eq!(every.schedule.every_ms, Some(7_200_000));
        assert_eq!(every.title, "Check the feeds.");

        let at = routine(&[
            "--at", "07:30", "--days", "mon,fri", "--prompt", "Plan.", "--title", "Morning",
        ])
        .unwrap();
        assert_eq!(
            (
                at.schedule.kind,
                at.schedule.time_of_day.as_deref(),
                at.schedule.weekdays,
                at.title.as_str()
            ),
            (
                ScheduleKind::FixedTime,
                Some("07:30"),
                vec![1, 5],
                "Morning"
            )
        );

        let hook = routine(&["--webhook", "--prompt", "Read {{body}}."]).unwrap();
        assert_eq!(hook.schedule.kind, ScheduleKind::Webhook);

        for wrong in [
            &["--prompt", "No schedule."][..],
            &["--every", "30s", "--prompt", "Too often."],
            &["--every", "0m", "--prompt", "Never."],
            &["--every", "5ü", "--prompt", "Not a unit."],
            &["--webhook"],
            &["--webhook", "--every", "30m", "--prompt", "Two schedules."],
            &[
                "--every",
                "30m",
                "--at",
                "07:00",
                "--prompt",
                "Two schedules.",
            ],
            &["--every", "30m", "--prompt", ""],
            &["--at", "25:00", "--prompt", "Late."],
            &["--at", "7:60", "--prompt", "Late."],
            &["--at", "7:3", "--prompt", "Late."],
            &["--webhook", "--days", "mon", "--prompt", "Days need --at."],
            &["--at", "07:30", "--days", "someday", "--prompt", "Plan."],
        ] {
            assert!(routine(wrong).is_err(), "{wrong:?} is refused");
        }
    }

    #[test]
    fn a_once_routine_takes_a_future_time_with_an_offset() {
        for wrong in [
            "2026-12-01T07:30:00",
            "tomorrow",
            "2000-01-01T00:00:00Z",
            "2999-02-29T07:30:00Z",
            "2999-12-01T24:00:00Z",
            "2999-12-01 07:30:00Z",
        ] {
            assert!(
                routine(&["--once", wrong, "--prompt", "Once."]).is_err(),
                "{wrong} is refused"
            );
        }
        for right in [
            "2999-12-01T07:30:00Z",
            "2999-12-01T07:30:00+01:00",
            "2999-06-01T07:30:00-05:00",
        ] {
            let once = routine(&["--once", right, "--prompt", "Once."]).unwrap();
            assert_eq!(once.schedule.run_at.as_deref(), Some(right));
        }
        assert_eq!(unix_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_seconds("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(unix_seconds("2000-03-01T01:00:00+01:00"), Some(951_868_800));
    }
}
