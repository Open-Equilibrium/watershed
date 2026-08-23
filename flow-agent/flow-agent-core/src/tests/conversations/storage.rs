use super::super::helpers::empty_workspace;
use crate::runtime::conversations::real_child_file_names;
use std::fs;

#[test]
fn real_child_file_enumeration_stops_at_the_first_over_limit_witness() {
    let workspace = empty_workspace("bounded-real-child-file-enumeration");
    let objects = workspace.join("objects");
    fs::create_dir(&objects).expect("object directory creates");
    for name in ["a", "b", "c"] {
        fs::write(objects.join(name), name).expect("real child file writes");
    }

    let names = real_child_file_names(&objects, 1).expect("child files enumerate");

    assert_eq!(names.len(), 2);
}
