use super::super::canonical::canonicalize_registry_block;
use super::super::load::{
    RegistryFile, RegistryRoot, parse_registry_block, read_registry_file_to_string,
};
use super::super::model::MAX_REGISTRY_FILE_BYTES;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// Maps each stable refusal fragment to its recorded class. A diagnostic that matches no
/// fragment is recorded as `unclassified`, so a changed message fails the corpus loudly.
const REFUSAL_CLASSES: [(&str, &str); 19] = [
    ("DenyAnchors", "anchor"),
    ("MergeKeyPolicy", "merge-key"),
    ("duplicate key", "duplicate-key"),
    ("collide after string conversion", "key-collision"),
    ("explicit YAML tag", "explicit-tag"),
    ("YAML null values", "null"),
    ("max_documents budget", "document-count"),
    ("recursion depth limit", "parser-depth"),
    ("tab characters", "tab-indentation"),
    ("unsupported registry block kind", "block-kind"),
    ("unknown field", "unknown-field"),
    ("invalid type", "type"),
    ("exceeds the maximum of 65536 bytes", "definition-size"),
    ("exceeds max 16", "flow-depth"),
    ("member count exceeds max 1024", "member-count"),
    ("must contain 1 to 256 characters", "key-length"),
    ("must use NFC text", "nfc-text"),
    ("canonical decimal form", "integer-form"),
    ("invalid utf-8", "invalid-utf8"),
];

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/hostile-yaml")
}

fn sidecar_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.canonical.json", name.trim_end_matches(".yaml")))
}

fn refusal_class(message: &str) -> String {
    REFUSAL_CLASSES
        .iter()
        .find(|(fragment, _)| message.contains(fragment))
        .map_or_else(
            || format!("unclassified: {message}"),
            |(_, class)| (*class).to_owned(),
        )
}

/// Runs one case through the registry read path. Returns `accepted` with the canonical JSON
/// of the parsed block, or `refused <class>`.
fn observe(root: &RegistryRoot, file: &RegistryFile) -> (String, Option<String>) {
    let name = file.path.display().to_string();
    let parsed = read_registry_file_to_string(root, file, MAX_REGISTRY_FILE_BYTES)
        .and_then(|source| parse_registry_block(&name, &source));
    match parsed {
        Ok(block) => {
            let canonical =
                canonicalize_registry_block(block).expect("accepted blocks canonicalize");
            let json = serde_json::to_value(&canonical).expect("blocks serialize");
            let text = proto::canonical_json(&json).expect("canonical JSON");
            ("accepted".to_owned(), Some(text))
        }
        Err(error) => (
            format!("refused {}", refusal_class(&error.to_string())),
            None,
        ),
    }
}

#[test]
fn hostile_definitions_keep_their_recorded_outcomes() {
    let dir = corpus_dir();
    let manifest: BTreeMap<String, String> = serde_json::from_str(
        &std::fs::read_to_string(dir.join("outcomes.json")).expect("outcome manifest is readable"),
    )
    .expect("outcome manifest maps case names to outcomes");
    let (root, files) = super::collect_registry_files(&dir).expect("corpus is collected");

    let names: BTreeSet<String> = files
        .iter()
        .map(|file| file.path.display().to_string())
        .collect();
    assert_eq!(
        names,
        manifest.keys().cloned().collect(),
        "every case file needs exactly one manifest entry"
    );

    let mut failures = Vec::new();
    for file in &files {
        let name = file.path.display().to_string();
        let expected = &manifest[&name];
        let (label, canonical) = observe(&root, file);
        let failure = match canonical {
            Some(canonical) if expected == "accepted" => {
                let sidecar = sidecar_path(&dir, &name);
                let recorded = std::fs::read_to_string(&sidecar).unwrap_or_default();
                (recorded != format!("{canonical}\n"))
                    .then(|| format!("{name}: canonical JSON differs from {}", sidecar.display()))
            }
            _ => (label != *expected)
                .then(|| format!("{name}: expected {expected}, observed {label}")),
        };
        failures.extend(failure);
    }
    assert!(
        failures.is_empty(),
        "hostile definitions changed outcome:\n{}",
        failures.join("\n")
    );
}
