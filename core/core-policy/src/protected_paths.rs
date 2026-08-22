/// Default protected path patterns that policy artifacts must carry.
pub const DEFAULT_PROTECTED_PATHS: &[&str] = &[
    "**/*.env",
    "**/*.key",
    "**/*.local",
    "**/*.p12",
    "**/*.pem",
    "**/*.pfx",
    "**/.aws",
    "**/.aws/**",
    "**/.azure",
    "**/.azure/**",
    "**/.config/gcloud",
    "**/.config/gcloud/**",
    "**/.config/gh",
    "**/.config/gh/**",
    "**/.docker",
    "**/.docker/**",
    "**/.env",
    "**/.env.*",
    "**/.git",
    "**/.git-credentials",
    "**/.git/**",
    "**/.gnupg",
    "**/.gnupg/**",
    "**/.kube",
    "**/.kube/**",
    "**/.flow",
    "**/.flow/**",
    "**/.netrc",
    "**/.npmrc",
    "**/.pypirc",
    "**/.ssh",
    "**/.ssh/**",
    "**/credentials",
    "**/credentials.toml",
    "**/credentials/**",
    "**/id_dsa",
    "**/id_ecdsa",
    "**/id_ecdsa_sk",
    "**/id_ed25519",
    "**/id_ed25519_sk",
    "**/id_rsa",
    "**/secrets",
    "**/secrets/**",
];

/// Case handling used when matching protected path patterns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectedPathMatchMode {
    /// Match protected path patterns exactly.
    CaseSensitive,
    /// Match protected path patterns using ASCII case folding.
    CaseInsensitive,
}

impl ProtectedPathMatchMode {
    /// Returns the canonical token recorded in runtime intent signatures.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CaseSensitive => "case-sensitive",
            Self::CaseInsensitive => "case-insensitive",
        }
    }
}

pub(crate) fn protected_path_grant_is_inside_scope(grant: &str, scope: &str) -> bool {
    let literal_prefix = grant.find(['*', '?']).map_or(grant, |wildcard| {
        grant[..wildcard]
            .rsplit_once('/')
            .map_or("", |(prefix, _)| prefix)
    });
    !literal_prefix.is_empty() && core_script::relative_path_is_inside_scope(literal_prefix, scope)
}

/// Returns whether a protected path glob pattern matches a normalized path.
///
/// The grammar is slash-normalized, path-segment based, accepts `*` and `?`
/// within a segment, and treats `**` as a whole segment matching zero or more
/// path segments. The direct path and its `workspace/`-relative form are both
/// considered because policy checks compare both workspace-scoped and
/// workspace-root-relative paths.
pub fn protected_path_pattern_matches(
    match_mode: ProtectedPathMatchMode,
    pattern: &str,
    path: &str,
) -> bool {
    let Some(pattern) = normalize_protected_path_match_input(match_mode, pattern) else {
        return false;
    };
    let Some(path) = normalize_protected_path_match_input(match_mode, path) else {
        return false;
    };

    protected_path_pattern_matches_normalized(&pattern, &path)
        || core_script::strip_workspace_scope(&path).is_some_and(|root_relative| {
            protected_path_pattern_matches_normalized(&pattern, root_relative)
        })
}

pub(crate) fn normalize_protected_path_match_input(
    match_mode: ProtectedPathMatchMode,
    value: &str,
) -> Option<String> {
    let normalized = core_script::normalize_protected_path_pattern(value)?;
    match match_mode {
        ProtectedPathMatchMode::CaseSensitive => Some(normalized),
        ProtectedPathMatchMode::CaseInsensitive => Some(normalized.to_ascii_lowercase()),
    }
}

fn protected_path_pattern_matches_normalized(pattern: &str, path: &str) -> bool {
    let pattern_segments = pattern.split('/').collect::<Vec<_>>();
    let path_segments = path.split('/').collect::<Vec<_>>();
    protected_segments_match(&pattern_segments, &path_segments)
}

fn protected_segments_match(pattern: &[&str], path: &[&str]) -> bool {
    let mut pattern_index = 0;
    let mut path_index = 0;
    let mut globstar = None;

    while path_index < path.len() {
        if pattern.get(pattern_index).is_some_and(|segment| {
            *segment != "**" && protected_segment_match(segment, path[path_index])
        }) {
            pattern_index += 1;
            path_index += 1;
        } else if pattern
            .get(pattern_index)
            .is_some_and(|segment| *segment == "**")
        {
            globstar = Some((pattern_index, path_index));
            pattern_index += 1;
        } else if let Some((globstar_index, matched_path_index)) = globstar {
            path_index = matched_path_index + 1;
            globstar = Some((globstar_index, path_index));
            pattern_index = globstar_index + 1;
        } else {
            return false;
        }
    }

    while pattern
        .get(pattern_index)
        .is_some_and(|segment| *segment == "**")
    {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
}

fn protected_segment_match(pattern: &str, path: &str) -> bool {
    let mut pattern_index = 0;
    let mut path_index = 0;
    let mut star_pattern_index = None;
    let mut star_path_index = 0;

    while path_index < path.len() {
        let pattern_char = pattern[pattern_index..].chars().next();
        let path_char = path[path_index..]
            .chars()
            .next()
            .expect("path index remains on a character boundary");
        if pattern_char.is_some_and(|ch| ch == '?' || ch == path_char) {
            pattern_index += pattern_char.expect("matched pattern character").len_utf8();
            path_index += path_char.len_utf8();
        } else if pattern_char == Some('*') {
            star_pattern_index = Some(pattern_index);
            pattern_index += 1;
            star_path_index = path_index;
        } else if let Some(star_index) = star_pattern_index {
            pattern_index = star_index + 1;
            star_path_index += path[star_path_index..]
                .chars()
                .next()
                .expect("star backtracking remains within the path")
                .len_utf8();
            path_index = star_path_index;
        } else {
            return false;
        }
    }

    while pattern[pattern_index..].starts_with('*') {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
}
