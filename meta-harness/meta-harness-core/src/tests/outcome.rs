use crate::TestCount;
use crate::history::test_counts;

#[test]
fn reads_test_counts_of_common_runners() {
    let count = |passed, failed| Some(TestCount { passed, failed });
    let cases = [
        (
            "running 3 tests\ntest result: ok. 3 passed; 0 failed; 0 ignored\n\ntest result: FAILED. 9 passed; 2 failed; 0 ignored",
            count(12, 2),
        ),
        ("Tests:       1 failed, 12 passed, 13 total", count(12, 1)),
        ("      Tests  1 failed | 11 passed (12)", count(11, 1)),
        ("      Tests  12 passed (12)", count(12, 0)),
        ("===== 12 passed, 1 failed in 0.52s =====", count(12, 1)),
        ("============ 3 passed in 0.01s ============", count(3, 0)),
        ("ℹ tests 6\nℹ pass 5\nℹ fail 1", count(5, 1)),
        ("# pass 6\n# fail 0", count(6, 0)),
        (
            "--- PASS: TestA (0.00s)\n--- FAIL: TestB (0.01s)\n--- PASS: TestC (0.00s)",
            count(2, 1),
        ),
        ("00:01 +37: All tests passed!", count(37, 0)),
        (
            "00:01 +12: loading\n00:02 +30 -2: Some tests failed.",
            count(30, 2),
        ),
        ("Successfully wrote to a.ts", None),
        ("ok  \texample.com/pkg\t0.512s", None),
    ];
    for (output, expected) in cases {
        assert_eq!(test_counts(output), expected, "{output}");
    }
}
