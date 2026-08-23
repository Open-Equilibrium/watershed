use crate::{DEFAULT_PROTECTED_PATHS, ProtectedPathMatchMode, protected_path_pattern_matches};
use proptest::prelude::*;

#[test]
fn protected_path_matcher_covers_normalization_and_pattern_edges() {
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "src/main.rs",
        "workspace/src/main.rs"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        r"workspace\.ssh\**",
        "workspace/.ssh/id_rsa"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "**/.ssh/**",
        "workspace/home/user/.ssh/config"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "**/*.env",
        "workspace/app/.env"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "**/secret*",
        "workspace/app/secret-token"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "**/secrets",
        "workspace/app/secrets"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "workspace/*/id_???",
        "workspace/keys/id_rsa"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "workspace/**",
        "workspace"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "workspace/secret*",
        "workspace/secret"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseInsensitive,
        "**/.SSH/**",
        "workspace/home/user/.ssh/config"
    ));
    for (pattern, path, expected) in [
        ("workspace/?.pem", "workspace/é.pem", true),
        ("workspace/??.pem", "workspace/é.pem", false),
        ("workspace/??.pem", "workspace/éa.pem", true),
    ] {
        assert_eq!(
            protected_path_pattern_matches(ProtectedPathMatchMode::CaseSensitive, pattern, path),
            expected
        );
    }

    for (pattern, path) in [
        ("", "workspace/app/.env"),
        ("/absolute", "workspace/app/.env"),
        ("bad$pattern", "workspace/app/.env"),
        ("bad/**suffix", "workspace/app/.env"),
        (".", "workspace/app/.env"),
        ("..", "workspace/app/.env"),
        ("C:/secret", "workspace/app/.env"),
        ("**/.env", ""),
        ("**/.env", "/absolute"),
        ("**/.env", "workspace/$secret"),
        ("**/.env", "workspace/../secret"),
    ] {
        assert!(
            !protected_path_pattern_matches(ProtectedPathMatchMode::CaseSensitive, pattern, path),
            "{pattern:?} must not match {path:?}"
        );
    }
}

#[test]
fn default_protected_paths_have_behavioral_denial_examples() {
    let cases = [
        ("**/*.env", "workspace/app/config.env"),
        ("**/*.key", "workspace/keys/service.key"),
        ("**/*.local", "workspace/app/settings.local"),
        ("**/*.p12", "workspace/certs/client.p12"),
        ("**/*.pem", "workspace/certs/client.pem"),
        ("**/*.pfx", "workspace/certs/client.pfx"),
        ("**/.aws", "workspace/home/user/.aws"),
        ("**/.aws/**", "workspace/home/user/.aws/config"),
        ("**/.azure", "workspace/home/user/.azure"),
        ("**/.azure/**", "workspace/home/user/.azure/token"),
        ("**/.config/gcloud", "workspace/home/user/.config/gcloud"),
        (
            "**/.config/gcloud/**",
            "workspace/home/user/.config/gcloud/application_default_credentials.json",
        ),
        ("**/.config/gh", "workspace/home/user/.config/gh"),
        (
            "**/.config/gh/**",
            "workspace/home/user/.config/gh/hosts.yml",
        ),
        ("**/.docker", "workspace/home/user/.docker"),
        ("**/.docker/**", "workspace/home/user/.docker/config.json"),
        ("**/.env", "workspace/app/.env"),
        ("**/.env.*", "workspace/app/.env.production"),
        ("**/.git", "workspace/project/.git"),
        (
            "**/.git-credentials",
            "workspace/home/user/.git-credentials",
        ),
        ("**/.git/**", "workspace/project/.git/config"),
        ("**/.gnupg", "workspace/home/user/.gnupg"),
        (
            "**/.gnupg/**",
            "workspace/home/user/.gnupg/private-keys-v1.d/key",
        ),
        ("**/.kube", "workspace/home/user/.kube"),
        ("**/.kube/**", "workspace/home/user/.kube/config"),
        ("**/.flow", "workspace/project/.flow"),
        ("**/.flow/**", "workspace/project/.flow/sessions/run.jsonl"),
        ("**/.netrc", "workspace/home/user/.netrc"),
        ("**/.npmrc", "workspace/project/.npmrc"),
        ("**/.pypirc", "workspace/home/user/.pypirc"),
        ("**/.ssh", "workspace/home/user/.ssh"),
        ("**/.ssh/**", "workspace/home/user/.ssh/config"),
        ("**/credentials", "workspace/app/credentials"),
        ("**/credentials.toml", "workspace/app/credentials.toml"),
        ("**/credentials/**", "workspace/app/credentials/token"),
        ("**/id_dsa", "workspace/keys/id_dsa"),
        ("**/id_ecdsa", "workspace/keys/id_ecdsa"),
        ("**/id_ecdsa_sk", "workspace/keys/id_ecdsa_sk"),
        ("**/id_ed25519", "workspace/keys/id_ed25519"),
        ("**/id_ed25519_sk", "workspace/keys/id_ed25519_sk"),
        ("**/id_rsa", "workspace/keys/id_rsa"),
        ("**/secrets", "workspace/app/secrets"),
        ("**/secrets/**", "workspace/app/secrets/prod.txt"),
    ];

    assert_eq!(cases.len(), DEFAULT_PROTECTED_PATHS.len());
    for pattern in DEFAULT_PROTECTED_PATHS {
        assert!(
            cases
                .iter()
                .any(|(case_pattern, _path)| case_pattern == pattern),
            "missing behavioral example for {pattern}"
        );
    }
    for (pattern, path) in cases {
        assert!(
            protected_path_pattern_matches(ProtectedPathMatchMode::CaseSensitive, pattern, path),
            "{pattern} must deny {path}"
        );
    }
}

#[test]
fn protected_path_double_star_handles_deep_valid_paths_without_recursion() {
    let mut segments = vec!["segment"; 100_000];
    segments.push("target.key");
    let path = segments.join("/");

    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "**/target.key",
        &path
    ));
    assert!(!protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "**/.env",
        &path
    ));
}

proptest! {
    #[test]
    fn protected_path_double_star_matches_any_depth(
        segments in prop::collection::vec("[a-z0-9][a-z0-9_-]{0,7}", 0..6)
            .prop_filter("portable path components", |segments| {
                segments
                    .iter()
                    .all(|segment| !core_script::relative_path_has_windows_alias(segment))
            })
    ) {
        let middle = if segments.is_empty() {
            String::new()
        } else {
            format!("{}/", segments.join("/"))
        };
        let path = format!("workspace/{middle}target.key");

        prop_assert!(protected_path_pattern_matches(
            ProtectedPathMatchMode::CaseSensitive,
            "workspace/**/target.key",
            &path
        ));
        prop_assert!(protected_path_pattern_matches(
            ProtectedPathMatchMode::CaseSensitive,
            "**/*.key",
            &path
        ));
    }

    #[test]
    fn protected_path_segment_wildcards_do_not_cross_slashes(
        middle in "[a-z0-9][a-z0-9_-]{0,7}"
    ) {
        let same_segment = format!("workspace/secret-{middle}.pem");
        let nested_segment = format!("workspace/secret-{middle}/pem");

        prop_assert!(protected_path_pattern_matches(
            ProtectedPathMatchMode::CaseSensitive,
            "workspace/secret-*.pem",
            &same_segment
        ));
        prop_assert!(!protected_path_pattern_matches(
            ProtectedPathMatchMode::CaseSensitive,
            "workspace/secret-*.pem",
            &nested_segment
        ));
    }

}
