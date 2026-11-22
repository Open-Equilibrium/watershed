use std::path::Path;

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
                root = Some(
                    std::fs::canonicalize(Path::new(value))
                        .map_err(|e| format!("The project {value} is not readable: {e}"))?
                        .to_string_lossy()
                        .into_owned(),
                );
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
        name: name.to_owned(),
        root: root.ok_or("meta agent create needs --project PATH")?,
        model: model.ok_or("meta agent create needs --model INSTANCE/MODEL")?,
        permissions,
        instructions,
    })
}

fn routine(mut args: &[&str]) -> Result<NewRoutine, String> {
    let (mut schedule, mut prompt, mut title, mut days) = (None, None, None, Vec::new());
    while !args.is_empty() {
        args = match args {
            ["--webhook", rest @ ..] => {
                schedule = Some(blank(ScheduleKind::Webhook));
                rest
            }
            ["--every", every, rest @ ..] => {
                schedule = Some(Schedule {
                    every_ms: Some(duration(every)?),
                    ..blank(ScheduleKind::Interval)
                });
                rest
            }
            ["--at", time, rest @ ..] => {
                schedule = Some(Schedule {
                    time_of_day: Some((*time).to_owned()),
                    ..blank(ScheduleKind::FixedTime)
                });
                rest
            }
            ["--once", at, rest @ ..] => {
                schedule = Some(Schedule {
                    run_at: Some((*at).to_owned()),
                    ..blank(ScheduleKind::Once)
                });
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
    let prompt: String = prompt.ok_or("meta agent routine add needs --prompt TEXT")?;
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
    let (count, unit) = text.split_at(text.len().saturating_sub(1));
    let count: u64 = count.parse().map_err(|_| wrong())?;
    let minutes = match unit {
        "m" => 1,
        "h" => 60,
        "d" => 24 * 60,
        _ => return Err(wrong()),
    };
    count
        .checked_mul(minutes * MINUTE_MS)
        .filter(|ms| *ms >= MINUTE_MS)
        .ok_or_else(|| format!("{text:?} is shorter than a minute or too long."))
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

    use super::{new_agent, routine};

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
            &["--webhook"],
            &["--webhook", "--days", "mon", "--prompt", "Days need --at."],
            &["--at", "07:30", "--days", "someday", "--prompt", "Plan."],
        ] {
            assert!(routine(wrong).is_err(), "{wrong:?} is refused");
        }
    }
}
