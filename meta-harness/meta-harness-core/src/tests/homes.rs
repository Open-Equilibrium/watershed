//! The harness homes Meta-Harness writes the central configuration into, and the T3 provider
//! settings that point Claude Code and Codex at them (ADR-0298).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::config::{Blocks, Homes, McpServer, provider_instances};

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let dir = std::env::temp_dir().join(format!("mh-homes-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

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

#[test]
fn every_block_reaches_both_harnesses_in_their_own_formats() {
    let dir = Dir::new();
    let person_codex = dir.0.join("person-codex");
    let homes = Homes::under(&dir.0.join("meta"));
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

    // Codex keeps using the person's login: its credential files link to the person's home.
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
    let homes = Homes::under(&dir.0.join("meta"));
    homes.write(&blocks(), &dir.0.join("person")).unwrap();
    // What the harnesses keep in their homes, and a login made in the home itself.
    std::fs::create_dir_all(homes.codex.join("sessions")).unwrap();
    std::fs::write(homes.codex.join("sessions/s.jsonl"), "{}").unwrap();
    std::fs::create_dir_all(homes.claude.join("skills/own")).unwrap();
    std::fs::write(homes.claude.join("skills/own/SKILL.md"), "mine").unwrap();
    std::fs::remove_file(homes.codex.join("auth.json")).unwrap();
    std::fs::write(homes.codex.join("auth.json"), "{}").unwrap();

    homes
        .write(&Blocks::default(), &dir.0.join("person"))
        .unwrap();

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
                            "config": {"binaryPath": "/bin/claude", "homePath": "/m/harness/claude",
                                       "launchArgs": "--mcp-config \"/m/harness/claude/mcp.json\""}},
            "codex": {"driver": "codex", "config": {"homePath": "/m/harness/codex"}},
            "scripted": {"driver": "acpRegistry", "config": {"commandPath": "/x"}},
        })
    );
    // A person who keeps Claude Code's login under their own CLAUDE_CONFIG_DIR keeps it.
    let own = provider_instances(&json!(null), &homes, Some("/Users/p/.claude-work")).unwrap();
    assert_eq!(
        own["claudeAgent"]["environment"],
        json!([{"name": "CLAUDE_SECURESTORAGE_CONFIG_DIR", "value": "/Users/p/.claude-work",
                "sensitive": false}])
    );
    // A home whose path T3 could not pass on as one argument is refused.
    assert!(provider_instances(&json!({}), &Homes::under(Path::new("/a\"b")), None).is_err());
}

#[test]
fn a_tampered_skill_list_never_removes_folders_outside_the_skills() {
    let dir = Dir::new();
    let homes = Homes::under(&dir.0.join("meta"));
    homes
        .write(&Blocks::default(), &dir.0.join("person"))
        .unwrap();
    let (relative, absolute) = (dir.0.join("meta/harness/keep"), dir.0.join("absolute"));
    for folder in [&relative, &absolute] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let names = serde_json::to_string(&["../../keep", &absolute.to_string_lossy(), ".."]).unwrap();
    for home in [&homes.claude, &homes.codex] {
        std::fs::write(home.join(".meta-harness-skills.json"), &names).unwrap();
    }
    homes
        .write(&Blocks::default(), &dir.0.join("person"))
        .unwrap();
    assert!(relative.is_dir() && absolute.is_dir());
}
