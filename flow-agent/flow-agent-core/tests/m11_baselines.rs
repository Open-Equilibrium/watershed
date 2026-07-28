#[path = "support/m11_baselines.rs"]
mod m11_baseline_support;
#[allow(dead_code)]
#[path = "../../tests/support.rs"]
mod test_support;

use m11_baseline_support::{BaselineReport, memory_report};
#[cfg(target_os = "linux")]
use m11_baseline_support::{filesystem_baseline, process_baseline};

#[test]
fn baseline_report_is_machine_readable_and_complete() {
    let report = BaselineReport::fixture();
    assert_machine_readable_and_complete(&report, "fixture");
}

fn assert_machine_readable_and_complete(report: &BaselineReport, expected_operation: &str) {
    let value: serde_json::Value =
        serde_json::from_str(&report.canonical_json()).expect("baseline report must be valid JSON");

    assert_eq!(value["schema"], "m1.1-baseline-v0");
    assert_eq!(value["operation"], expected_operation);
    assert!(value["inputs"].is_object());
    assert!(value["exclusions"].is_array());
    assert!(value["metrics"].is_object());
    assert!(value["memory"].is_object());
    assert!(value["environment"].is_object());
}

#[test]
fn memory_report_peak_includes_the_final_rss_sample() {
    let memory = memory_report(100, 110, 120, "test_rss");

    assert_eq!(memory["before_bytes"], 100);
    assert_eq!(memory["after_bytes"], 120);
    assert_eq!(memory["peak_bytes"], 120);
    assert_eq!(memory["peak_growth_bytes"], 20);
    assert_eq!(memory["retained_growth_bytes"], 20);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "M1.1 baseline"]
fn m11_filesystem_primitives_baseline() {
    let report = filesystem_baseline();
    assert_eq!(report.operation(), "filesystem_inventory_copy_delete");
    assert_machine_readable_and_complete(&report, "filesystem_inventory_copy_delete");
    println!("M1_1_BASELINE {}", report.canonical_json());
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "M1.1 baseline"]
fn m11_process_primitives_baseline() {
    let report = process_baseline();
    assert_eq!(report.operation(), "posix_shell_spawn_wait");
    assert_machine_readable_and_complete(&report, "posix_shell_spawn_wait");
    println!("M1_1_BASELINE {}", report.canonical_json());
}
