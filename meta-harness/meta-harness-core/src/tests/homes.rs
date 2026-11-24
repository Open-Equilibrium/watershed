use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::{Dir, put, read};
use crate::config::{Blocks, Homes, McpServer, provider_instances};

fn blocks() -> Blocks {
    Blocks {
        instructions: BTreeMap::from([
            ("b-style".to_owned(), "Write short sentences.".to_owned()),
            ("a-tests".to_owned(), "Run the tests.".to_owned()),
        ]),
        skills: BTreeMap::from([(
            "review".to_owned(),
            BTreeMap::from([
                ("SKILL.md".to_owned(), "---\nname: review\n---\n".to_owned()),
                ("scripts/check.sh".to_owned(), "echo ok\n".to_owned()),
            ]),
        )]),
        mcp_servers: BTreeMap::from([
            (
                "docs".to_owned(),
                McpServer::Command {
                    command: "/opt/docs \"mcp\"".to_owned(),
                    args: vec!["--root".to_owned(), "C:\\docs\nx".to_owned()],
                    env: BTreeMap::from([("TOKEN_FILE".to_owned(), "/run/t".to_owned())]),
                },
            ),
            (
                "web".to_owned(),
                McpServer::Url {
                    url: "https://mcp.example.com/".to_owned(),
                },
            ),
        ]),
    }
}

fn skill(name: &str, files: &[(&str, &str)]) -> Blocks {
    let files = files
        .iter()
        .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
        .collect();
    Blocks {
        skills: BTreeMap::from([(name.to_owned(), files)]),
        ..Blocks::default()
    }
}

fn write(homes: &Homes, blocks: &Blocks, dir: &Dir) {
    homes.write(blocks, &dir.join("person")).unwrap();
}

#[test]
fn every_block_reaches_both_harnesses_in_their_own_formats() {
    let dir = Dir::new();
    let person_codex = dir.join("person-codex");
    let homes = Homes::under(&dir.join("meta"));
    homes.write(&blocks(), &person_codex).unwrap();

    let instructions = "Run the tests.\n\nWrite short sentences.\n";
    assert_eq!(read(&homes.claude.join("CLAUDE.md")), instructions);
    assert_eq!(read(&homes.codex.join("AGENTS.md")), instructions);
    for home in [&homes.claude, &homes.codex] {
        assert_eq!(
            read(&home.join("skills/review/scripts/check.sh")),
            "echo ok\n"
        );
    }

    let claude: serde_json::Value = serde_json::from_str(&read(&homes.claude_mcp())).unwrap();
    assert_eq!(
        claude,
        json!({"mcpServers": {
            "docs": {"type": "stdio", "command": "/opt/docs \"mcp\"",
                     "args": ["--root", "C:\\docs\nx"], "env": {"TOKEN_FILE": "/run/t"}},
            "web": {"type": "http", "url": "https://mcp.example.com/"},
        }})
    );
    assert_eq!(
        read(&homes.codex.join("config.toml")),
        "# Written by Meta-Harness from its central configuration; edits here are replaced.\n\
         \n[mcp_servers.docs]\n\
         command = \"/opt/docs \\\"mcp\\\"\"\n\
         args = [\"--root\", \"C:\\\\docs\\nx\"]\n\
         env = { TOKEN_FILE = \"/run/t\" }\n\
         \n[mcp_servers.web]\n\
         url = \"https://mcp.example.com/\"\n"
    );

    for name in ["auth.json", "auth.lock"] {
        assert_eq!(
            std::fs::read_link(homes.codex.join(name)).unwrap(),
            person_codex.join(name)
        );
    }
    assert!(
        !person_codex.exists(),
        "the person's own home is left alone"
    );
}

#[test]
fn a_removed_block_leaves_and_the_harness_s_own_files_stay() {
    let dir = Dir::new();
    let homes = Homes::under(&dir.join("meta"));
    write(&homes, &blocks(), &dir);
    put(homes.codex.join("sessions/s.jsonl"), "{}");
    put(homes.claude.join("skills/own/SKILL.md"), "mine");
    std::fs::remove_file(homes.codex.join("auth.json")).unwrap();
    put(homes.codex.join("auth.json"), "{}");

    write(&homes, &Blocks::default(), &dir);

    for home in [&homes.claude, &homes.codex] {
        assert!(!home.join("skills/review").exists());
    }
    assert!(!homes.claude.join("CLAUDE.md").exists());
    assert!(!homes.codex.join("AGENTS.md").exists());
    assert_eq!(read(&homes.claude_mcp()), "{\"mcpServers\":{}}");
    assert_eq!(read(&homes.codex.join("sessions/s.jsonl")), "{}");
    assert_eq!(read(&homes.claude.join("skills/own/SKILL.md")), "mine");
    assert_eq!(read(&homes.codex.join("auth.json")), "{}");
}

#[test]
fn t3_runs_claude_code_and_codex_in_the_homes_and_keeps_its_other_settings() {
    let homes = Homes::under(Path::new("/m"));
    let stored = json!({
        "claudeAgent": {"driver": "claudeAgent", "enabled": true,
                        "environment": [{"name": "HTTPS_PROXY", "value": "p", "sensitive": false},
                                        {"name": "CLAUDE_SECURESTORAGE_CONFIG_DIR", "value": "old",
                                         "sensitive": false}],
                        "config": {"binaryPath": "/bin/claude"}},
        "scripted": {"driver": "acpRegistry", "config": {"commandPath": "/x"}},
    });
    let patched = provider_instances(&stored, &homes, None).unwrap();
    assert_eq!(
        patched,
        json!({
            "claudeAgent": {"driver": "claudeAgent", "enabled": true,
                            "environment": [
                                {"name": "HTTPS_PROXY", "value": "p", "sensitive": false},
                                {"name": "CLAUDE_SECURESTORAGE_CONFIG_DIR", "value": "",
                                 "sensitive": false}],
                            "config": {"binaryPath": "/bin/claude", "homePath": homes.claude,
                                       "launchArgs": "--mcp-config \"/m/harness/claude/mcp.json\""}},
            "codex": {"driver": "codex", "config": {"homePath": homes.codex}},
            "scripted": {"driver": "acpRegistry", "config": {"commandPath": "/x"}},
        })
    );
    let own = provider_instances(&json!(null), &homes, Some("/Users/p/.claude-work")).unwrap();
    assert_eq!(
        own["claudeAgent"]["environment"],
        json!([{"name": "CLAUDE_SECURESTORAGE_CONFIG_DIR", "value": "/Users/p/.claude-work",
                "sensitive": false}])
    );
    assert!(provider_instances(&json!({}), &Homes::under(Path::new("/a\"b")), None).is_err());
    for stored in [json!({"codex": false}), json!({"claudeAgent": "on"})] {
        assert!(provider_instances(&stored, &Homes::under(Path::new("/m")), None).is_err());
    }
}

#[test]
fn a_tampered_skill_list_never_removes_folders_outside_the_skills() {
    let dir = Dir::new();
    let homes = Homes::under(&dir.join("meta"));
    write(&homes, &Blocks::default(), &dir);
    let (relative, absolute) = (dir.join("meta/harness/keep"), dir.join("absolute"));
    for folder in [&relative, &absolute] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let names = serde_json::to_string(&["../../keep", &absolute.to_string_lossy(), ".."]).unwrap();
    for home in [&homes.claude, &homes.codex] {
        put(home.join(".meta-harness-skills.json"), &names);
    }
    write(&homes, &Blocks::default(), &dir);
    assert!(relative.is_dir() && absolute.is_dir());
}

#[test]
fn a_damaged_skill_list_stops_the_write_before_any_skill_or_the_list_changes() {
    let dir = Dir::new();
    let homes = Homes::under(&dir.join("meta"));
    write(
        &homes,
        &skill("review", &[("SKILL.md", "---\nname: review\n---\n")]),
        &dir,
    );
    put(homes.codex.join(".meta-harness-skills.json"), "not a list");

    assert!(
        homes
            .write(&Blocks::default(), &dir.join("person"))
            .is_err()
    );
    assert_eq!(
        read(&homes.codex.join(".meta-harness-skills.json")),
        "not a list"
    );
    assert!(homes.codex.join("skills/review/SKILL.md").exists());
}

#[test]
fn a_skill_left_half_written_by_a_failed_write_is_removed_by_the_next_write() {
    let dir = Dir::new();
    let homes = Homes::under(&dir.join("meta"));
    let broken = skill("review", &[("a", "a file"), ("a/b", "under a file")]);
    assert!(homes.write(&broken, &dir.join("person")).is_err());
    write(&homes, &Blocks::default(), &dir);
    assert!(!homes.claude.join("skills/review").exists());
}

fn listing(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path.clone());
            }
            found.push(path.strip_prefix(root).unwrap().to_path_buf());
        }
    }
    found.sort();
    found
}

#[cfg(unix)]
#[test]
fn a_linked_harness_home_or_skills_folder_is_replaced_and_nothing_outside_changes() {
    for link in ["harness", "home", "skills"] {
        let dir = Dir::new();
        let external = dir.join("external");
        put(external.join("example/sentinel"), "keep");
        let before = listing(&external);
        let homes = Homes::under(&dir.join("meta"));
        let harness = homes.claude.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(dir.join("meta")).unwrap();
        match link {
            "harness" => std::os::unix::fs::symlink(&external, &harness).unwrap(),
            "home" => {
                std::fs::create_dir(&harness).unwrap();
                std::os::unix::fs::symlink(&external, &homes.claude).unwrap();
            }
            _ => {
                std::fs::create_dir_all(&homes.claude).unwrap();
                std::os::unix::fs::symlink(&external, homes.claude.join("skills")).unwrap();
            }
        }
        let blocks = skill("example", &[("SKILL.md", "---\nname: example\n---\n")]);
        write(&homes, &blocks, &dir);

        assert_eq!(listing(&external), before, "link={link}");
        let real_folder = |path: PathBuf| std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir());
        assert!(
            real_folder(harness.clone())
                && real_folder(homes.claude.clone())
                && real_folder(homes.claude.join("skills"))
        );
        assert_eq!(
            read(&homes.claude.join("skills/example/SKILL.md")),
            "---\nname: example\n---\n"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_link_planted_at_the_temporary_name_is_never_written_through() {
    let dir = Dir::new();
    let homes = Homes::under(&dir.join("meta"));
    std::fs::create_dir_all(&homes.codex).unwrap();
    let sentinel = dir.join("outside");
    put(&sentinel, "keep");
    let planted = homes.codex.join(".AGENTS.md.meta-harness-tmp");
    std::os::unix::fs::symlink(&sentinel, planted).unwrap();

    write(&homes, &blocks(), &dir);

    assert_eq!(read(&sentinel), "keep");
    assert_eq!(
        read(&homes.codex.join("AGENTS.md")),
        "Run the tests.\n\nWrite short sentences.\n"
    );
}

#[test]
fn a_write_that_cannot_replace_its_file_leaves_no_temporary_file() {
    let dir = Dir::new();
    let homes = Homes::under(&dir.join("meta"));
    std::fs::create_dir_all(homes.claude.join("CLAUDE.md")).unwrap();
    assert!(homes.write(&blocks(), &dir.join("person")).is_err());
    let leftover: Vec<_> = std::fs::read_dir(&homes.claude)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().ends_with(".meta-harness-tmp"))
        .collect();
    assert!(leftover.is_empty(), "{leftover:?}");
}
