use super::super::paths::{
    normalize_safe_relative_path, relative_path_has_windows_alias, relative_path_is_inside_scope,
};
use proptest::prelude::*;

#[test]
fn scope_containment_rejects_noncanonical_inputs() {
    assert!(relative_path_is_inside_scope("safe", "safe"));
    assert!(relative_path_is_inside_scope("safe/child", "safe"));

    for path in ["safe/../outside", "safe/./child", "/safe/child"] {
        assert!(!relative_path_is_inside_scope(path, "safe"), "{path:?}");
    }
    for scope in ["safe/..", "safe/.", "/safe"] {
        assert!(
            !relative_path_is_inside_scope("safe/child", scope),
            "{scope:?}"
        );
    }
}

#[test]
fn windows_console_device_paths_are_rejected() {
    for component in ["CONIN$", "conin$.txt", "CONOUT$", "conout$.log"] {
        let path = format!("safe/{component}");
        assert_eq!(normalize_safe_relative_path(&path), None, "{path:?}");
        assert!(!relative_path_is_inside_scope(&path, "safe"), "{path:?}");
    }
}

proptest! {
    #[test]
    fn safe_relative_paths_accept_generated_literal_segments(
        segments in prop::collection::vec("[a-z0-9][a-z0-9_-]{0,7}", 1..8)
            .prop_filter("portable path components", |segments| {
                segments
                    .iter()
                    .all(|segment| !relative_path_has_windows_alias(segment))
            })
    ) {
        let path = segments.join("/");
        let normalized = normalize_safe_relative_path(&path);
        let child = format!("{}/leaf", path);
        let sibling = format!("{}x/leaf", path);

        prop_assert_eq!(normalized, Some(path.clone()));
        prop_assert!(relative_path_is_inside_scope(&path, &path));
        prop_assert!(relative_path_is_inside_scope(&child, &path));
        prop_assert!(!relative_path_is_inside_scope(&sibling, &path));
    }

    #[test]
    fn safe_relative_paths_reject_nonportable_components(
        prefix in prop::collection::vec("[a-z0-9][a-z0-9_-]{0,7}", 0..4),
        suffix in prop::collection::vec("[a-z0-9][a-z0-9_-]{0,7}", 0..4),
        bad in prop_oneof![
            Just(".".to_owned()),
            Just("..".to_owned()),
            Just("CON".to_owned()),
            Just("NUL.txt".to_owned()),
            Just("COM1".to_owned()),
            Just("COM¹".to_owned()),
            Just("com².txt".to_owned()),
            Just("LPT³.tar.gz".to_owned()),
            Just("trail.".to_owned()),
            Just("trail ".to_owned()),
            prop::sample::select(vec!['<', '>', ':', '"', '|', '?', '*', '\u{1}'])
                .prop_map(|character| format!("bad{character}name")),
        ],
    ) {
        let mut segments = prefix;
        segments.push(bad);
        segments.extend(suffix);
        let path = segments.join("/");

        prop_assert_eq!(normalize_safe_relative_path(&path), None);
    }
}
