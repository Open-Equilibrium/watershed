#[path = "support/m11_baselines.rs"]
mod m11_baseline_support;
#[allow(dead_code)]
#[path = "../../tests/support.rs"]
mod test_support;

use m11_baseline_support::BaselineReport;
#[cfg(target_os = "linux")]
use m11_baseline_support::{filesystem_baseline, process_baseline};

#[test]
fn baseline_report_is_machine_readable_and_complete() {
    let report = BaselineReport::fixture();
    let value: serde_json::Value =
        serde_json::from_str(&report.canonical_json()).expect("baseline report must be valid JSON");

    assert_eq!(value["schema"], "m1.1-baseline-v0");
    assert_eq!(value["operation"], "fixture");
    assert!(value["inputs"].is_object());
    assert!(value["exclusions"].is_array());
    assert!(value["metrics"].is_object());
    assert!(value["memory"].is_object());
    assert!(value["environment"].is_object());
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "M1.1 baseline"]
fn m11_filesystem_primitives_baseline() {
    let report = filesystem_baseline();
    assert_eq!(report.operation(), "filesystem_inventory_copy_delete");
    println!("M1_1_BASELINE {}", report.canonical_json());
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "M1.1 baseline"]
fn m11_process_primitives_baseline() {
    let report = process_baseline();
    assert_eq!(report.operation(), "posix_shell_spawn_wait");
    println!("M1_1_BASELINE {}", report.canonical_json());
}
