//! The lines a file change added and removed, read from the patch pi stored with its result.

/// Lines a unified patch adds and removes. Each hunk header says how many old and new lines
/// follow, so changed content that starts with `---` or `+++` is not taken for a file name.
pub(crate) fn patch_lines(patch: &str) -> (u32, u32) {
    let (mut added, mut removed) = (0, 0);
    let (mut old, mut new) = (0u32, 0u32);
    for line in patch.lines() {
        if old == 0 && new == 0 {
            (old, new) = hunk(line).unwrap_or_default();
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => {
                added += 1;
                new = new.saturating_sub(1);
            }
            Some(b'-') => {
                removed += 1;
                old = old.saturating_sub(1);
            }
            // "\ No newline at end of file" belongs to the line before it.
            Some(b'\\') => {}
            _ => {
                old = old.saturating_sub(1);
                new = new.saturating_sub(1);
            }
        }
    }
    (added, removed)
}

/// The old and new line counts of a hunk header `@@ -a,b +c,d @@`; a missing count is one.
fn hunk(line: &str) -> Option<(u32, u32)> {
    let (ranges, _) = line.strip_prefix("@@ -")?.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let count = |range: &str| {
        range
            .split_once(',')
            .map_or(Some(1), |(_, count)| count.parse().ok())
    };
    Some((count(old)?, count(new)?))
}
