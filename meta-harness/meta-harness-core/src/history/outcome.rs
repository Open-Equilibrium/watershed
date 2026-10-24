//! What a tool call did, read from its stored result: changed lines, exit code and test counts.

use crate::TestCount;

/// Lines a unified patch adds and removes.
pub(super) fn patch_lines(patch: &str) -> (u32, u32) {
    let mut added = 0;
    let mut removed = 0;
    for line in patch.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => added += 1,
            Some(b'-') => removed += 1,
            _ => {}
        }
    }
    (added, removed)
}

/// The exit code pi appends to a failed shell command's output.
pub(super) fn exit_code(output: &str) -> Option<i32> {
    let (_, rest) = output.rsplit_once("Command exited with code ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Passed and failed tests in the output of cargo, jest, vitest, pytest, node --test, go test
/// -v or flutter test; other output yields nothing.
pub(crate) fn test_counts(output: &str) -> Option<TestCount> {
    let lines: Vec<&str> = output.lines().map(str::trim).collect();
    cargo(&lines)
        .or_else(|| summary(&lines))
        .or_else(|| node(&lines))
        .or_else(|| flutter(&lines))
        .or_else(|| go(&lines))
}

/// cargo prints one `test result:` line per test binary.
fn cargo(lines: &[&str]) -> Option<TestCount> {
    let results: Vec<_> = lines
        .iter()
        .filter(|line| line.starts_with("test result:"))
        .collect();
    (!results.is_empty()).then(|| TestCount {
        passed: results.iter().map(|line| before(line, " passed")).sum(),
        failed: results.iter().map(|line| before(line, " failed")).sum(),
    })
}

/// jest's `Tests:` line, vitest's `Tests` line and pytest's closing `=== … in …s ===` line.
fn summary(lines: &[&str]) -> Option<TestCount> {
    let line = lines.iter().rev().find(|line| {
        let counts = line.contains(" passed") || line.contains(" failed");
        counts
            && (line.starts_with("Tests:")
                || line.starts_with("Tests ")
                || (line.starts_with('=') && line.contains(" in ")))
    })?;
    Some(TestCount {
        passed: before(line, " passed"),
        failed: before(line, " failed"),
    })
}

/// node --test ends with `ℹ pass N` and `ℹ fail N`, or `# pass N` in TAP output.
fn node(lines: &[&str]) -> Option<TestCount> {
    let value = |name: &str| {
        lines.iter().rev().find_map(|line| {
            let rest = line
                .strip_prefix("ℹ ")
                .or_else(|| line.strip_prefix("# "))?
                .strip_prefix(name)?;
            rest.trim().parse::<u32>().ok()
        })
    };
    let passed = value("pass ")?;
    Some(TestCount {
        passed,
        failed: value("fail ").unwrap_or(0),
    })
}

/// flutter test ends with `+N: All tests passed!` or `+N -M: Some tests failed.`.
fn flutter(lines: &[&str]) -> Option<TestCount> {
    let line = lines
        .iter()
        .rev()
        .find(|line| line.ends_with("All tests passed!") || line.ends_with("Some tests failed."))?;
    let number = |sign: char| {
        line.split_whitespace()
            .find_map(|word| word.strip_prefix(sign)?.trim_end_matches(':').parse().ok())
            .unwrap_or(0)
    };
    Some(TestCount {
        passed: number('+'),
        failed: number('-'),
    })
}

/// go test -v marks each test with `--- PASS:` or `--- FAIL:`.
fn go(lines: &[&str]) -> Option<TestCount> {
    let count = |prefix: &str| lines.iter().filter(|line| line.starts_with(prefix)).count();
    let (passed, failed) = (count("--- PASS:"), count("--- FAIL:"));
    (passed + failed > 0).then(|| TestCount {
        passed: u32::try_from(passed).unwrap_or(u32::MAX),
        failed: u32::try_from(failed).unwrap_or(u32::MAX),
    })
}

/// The number directly before `word` in `line`, or zero.
fn before(line: &str, word: &str) -> u32 {
    line.find(word)
        .map(|end| {
            let digits: String = line[..end]
                .chars()
                .rev()
                .take_while(char::is_ascii_digit)
                .collect();
            digits
                .chars()
                .rev()
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}
