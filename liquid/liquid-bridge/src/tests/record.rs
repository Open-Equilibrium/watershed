use super::workspace::Folder;

#[test]
fn a_failed_export_leaves_no_folder_behind() {
    let downloads = Folder::new("failed");
    std::fs::create_dir_all(&downloads.0).expect("downloads");
    let failed = crate::api::record::write_folder(&downloads.0, "Broken", |folder| {
        std::fs::write(folder.join("thread.json"), "partial").expect("partial copy");
        Err("disk full".to_owned())
    });
    assert_eq!(failed, Err("disk full".to_owned()));
    assert_eq!(
        std::fs::read_dir(&downloads.0).expect("listed").count(),
        0,
        "neither the folder nor its temporary copy stays"
    );
}

#[test]
fn an_export_never_deletes_a_folder_it_did_not_create() {
    let downloads = Folder::new("foreign");
    std::fs::create_dir_all(&downloads.0).expect("downloads");
    let foreign = downloads.0.join(".Demo – record 2026-11-12.partial");
    std::fs::create_dir(&foreign).expect("foreign folder");
    std::fs::write(foreign.join("keep.txt"), "not ours").expect("foreign file");
    let _ = crate::api::record::write_folder(&downloads.0, "Demo – record 2026-11-12", |folder| {
        std::fs::write(folder.join("thread.json"), "copy").map_err(|error| error.to_string())
    });
    assert!(
        foreign.join("keep.txt").is_file(),
        "a folder the export did not create survives"
    );
}

#[test]
fn record_names_fit_a_file_name_and_name_an_untitled_session_by_its_day() {
    let title = "語".repeat(80);
    let name = crate::api::record::record_name(&title, "2026-11-11");
    assert!(format!(".{name}.partial").len() <= 255, "{name}");
    assert_eq!(
        crate::api::record::record_name(" . ", "2026-11-15"),
        "Untitled session – record 2026-11-15"
    );
}

#[test]
fn a_second_export_of_the_same_name_keeps_the_first_folder() {
    let downloads = Folder::new("twice");
    std::fs::create_dir_all(&downloads.0).expect("downloads");
    let write = |folder: &std::path::Path| {
        std::fs::write(folder.join("thread.json"), "copy").map_err(|error| error.to_string())
    };
    let first = crate::api::record::write_folder(&downloads.0, "Demo", write).expect("first");
    let second = crate::api::record::write_folder(&downloads.0, "Demo", write).expect("second");
    assert_eq!(first.file_name().unwrap(), "Demo");
    assert_eq!(second.file_name().unwrap(), "Demo 2");
    assert!(first.join("thread.json").is_file() && second.join("thread.json").is_file());
}

#[test]
fn an_export_without_a_downloads_folder_says_so_and_writes_nothing() {
    let missing = Folder::new("missing");
    let failed = crate::api::record::write_folder(&missing.0, "Demo", |_| Ok(()));
    assert!(
        failed.is_err_and(|error| error.starts_with("could not create ")),
        "the export names the folder it could not create"
    );
    assert!(!missing.0.exists());
}
