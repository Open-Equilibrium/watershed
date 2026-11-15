//! Guard: production runtime code writes and syncs files only through the write-and-sync helper in
//! `fs_guards/durability.rs` (ADR-0266), so the power-loss ledger sees every durable byte.
//!
//! A source scan, not a parser: it skips `#[cfg(test)]` items by brace counting, ignores comment
//! lines and flags write calls, sync calls and write-mode opens. Each exemption names its reason.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// Source files outside the helper, with the write or open each one is exempt for and why.
const EXEMPT_FILES: &[(&str, &[&str], &str)] = &[
    (
        "runtime/fs_guards/durability.rs",
        WRITE_CALLS_AND_OPENS,
        "the helper itself",
    ),
    (
        "runtime/fs_guards/durable_steps.rs",
        WRITE_CALLS_AND_OPENS,
        "the test-only crash trace and power-loss ledger, a module declared #[cfg(test)] by its parent",
    ),
    (
        "runtime/fs_guards/anchored_file.rs",
        WRITE_OPENS,
        "the capability-anchored open primitives; their handles are written only through the helper",
    ),
    (
        "runtime/conversations/history_index/scratch.rs",
        &[".write_all("],
        "the generic history validation index writer; a private per-command scratch that is never a durable Conversation format (PROTOCOL.md)",
    ),
    (
        "runtime/credential_store/platform.rs",
        WRITE_OPENS,
        "private open primitives: the credential temporary is written through the helper in storage.rs and the lock file carries no bytes",
    ),
    (
        "runtime/executor/config.rs",
        WRITE_OPENS,
        "private open primitives: the configuration stage is written through the helper and the lock file carries no bytes",
    ),
    (
        "runtime/auth/browser.rs",
        &[".write_all("],
        "answers the local OAuth callback socket; this is no stored Flow state",
    ),
    (
        "runtime/m11_budget_evidence/",
        WRITE_CALLS_AND_OPENS,
        "dev evidence generator behind the m11-budget-evidence feature; it writes fixtures only",
    ),
    (
        "runtime/conversations/conversation_stream/writer.rs",
        &[
            ".write(true)",
            "create_new(true)",
            ".append(true)",
            ".write_all(",
            ".sync_all(",
            ".sync_data(",
        ],
        "append_validated_jsonl_line, the m11-budget-evidence and test append primitive; every other write in this file is test-only",
    ),
];

const WRITE_OPENS: &[&str] = &[
    ".write(true)",
    ".append(true)",
    ".truncate(true)",
    "create_new(true)",
    "File::create(",
    "File::create_new(",
    "fs::write(",
];
const WRITE_CALLS_AND_OPENS: &[&str] = &[
    ".write_all(",
    ".set_len(",
    ".sync_all(",
    ".sync_data(",
    ".write(true)",
    ".append(true)",
    ".truncate(true)",
    "create_new(true)",
    "File::create(",
    "File::create_new(",
    "fs::write(",
];

#[test]
fn durable_writes_go_through_the_write_and_sync_helper() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rust_files(&src.join("runtime"), &mut files);
    assert!(!files.is_empty(), "runtime sources are scanned");

    let mut violations = Vec::new();
    for file in &files {
        let relative = file
            .strip_prefix(&src)
            .expect("runtime file is under src")
            .to_string_lossy()
            .replace('\\', "/");
        let exempt: Vec<&str> = EXEMPT_FILES
            .iter()
            .filter(|(path, _, _)| relative.starts_with(path) || relative == *path)
            .flat_map(|(_, tokens, _)| tokens.iter().copied())
            .collect();
        let source = fs::read_to_string(file).expect("runtime source reads");
        for (line_number, line) in production_lines(&source) {
            let flagged = WRITE_CALLS_AND_OPENS
                .iter()
                .any(|token| line.contains(token) && !exempt.contains(token));
            if flagged {
                violations.push(format!("{relative}:{line_number}: {}", line.trim()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "durable writes and syncs bypass the write-and-sync helper:\n{}",
        violations.join("\n")
    );
}

fn collect_rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("runtime directory reads") {
        let path = entry.expect("runtime entry reads").path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

/// The 1-based numbered lines of `source` outside test-only items and comment lines.
///
/// rustfmt indents an item's closing line to the item's own indentation, so a test-only item
/// ends at the first line at or left of its indentation that closes a block or ends a statement.
fn production_lines(source: &str) -> Vec<(usize, &str)> {
    let indent = |line: &str| line.len() - line.trim_start().len();
    let mut lines = Vec::new();
    let mut awaiting_item = None;
    let mut skipping_indent = None;
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(item_indent) = skipping_indent {
            if indent(line) <= item_indent && (trimmed.starts_with('}') || trimmed.ends_with(';')) {
                skipping_indent = None;
            }
            continue;
        }
        if is_test_attribute(trimmed) {
            awaiting_item = Some(indent(line));
            continue;
        }
        if awaiting_item.is_some() && trimmed.starts_with("#[") {
            continue;
        }
        if let Some(item_indent) = awaiting_item.take() {
            skipping_indent = Some(item_indent);
            if indent(line) <= item_indent && (trimmed.starts_with('}') || trimmed.ends_with(';')) {
                skipping_indent = None;
            }
            continue;
        }
        if !trimmed.starts_with("//") {
            lines.push((index + 1, line));
        }
    }
    lines
}

fn is_test_attribute(trimmed: &str) -> bool {
    trimmed == "#[test]" || trimmed == "#[cfg(test)]" || trimmed.starts_with("#[cfg(all(test,")
}

#[test]
fn production_lines_skip_test_items_and_comments() {
    let source = "fn kept() {\n    // file.sync_all();\n}\n#[cfg(test)]\nmod tests {\n    fn scratch() {\n        f.sync_all();\n    }\n}\nlet kept = 1;\n";

    let lines: Vec<&str> = production_lines(source)
        .into_iter()
        .map(|(_, line)| line)
        .collect();

    assert_eq!(lines, vec!["fn kept() {", "}", "let kept = 1;"]);
}

#[test]
fn production_lines_keep_items_enabled_outside_test() {
    let source = "#[cfg(any(test, unix))]\nfn p(f:&File){\n    f.sync_all().unwrap();\n}\n";

    let lines: Vec<&str> = production_lines(source)
        .into_iter()
        .map(|(_, line)| line)
        .collect();

    assert_eq!(
        lines,
        vec![
            "#[cfg(any(test, unix))]",
            "fn p(f:&File){",
            "    f.sync_all().unwrap();",
            "}"
        ]
    );
}
