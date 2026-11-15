//! Writing a record folder into Downloads (ADR-0257).

#[test]
fn a_failed_export_leaves_no_folder_behind() {
    let downloads =
        std::env::temp_dir().join(format!("liquid-bridge-failed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&downloads);
    std::fs::create_dir_all(&downloads).expect("downloads");
    let failed = crate::api::record::write_folder(&downloads, "Broken", |folder| {
        std::fs::write(folder.join("thread.json"), "partial").expect("partial copy");
        Err("disk full".to_owned())
    });
    assert_eq!(failed, Err("disk full".to_owned()));
    assert_eq!(
        std::fs::read_dir(&downloads).expect("listed").count(),
        0,
        "neither the folder nor its temporary copy stays"
    );
    let _ = std::fs::remove_dir_all(&downloads);
}

#[test]
fn an_export_never_deletes_a_folder_it_did_not_create() {
    let downloads =
        std::env::temp_dir().join(format!("liquid-bridge-foreign-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&downloads);
    std::fs::create_dir_all(&downloads).expect("downloads");
    let foreign = downloads.join(".Demo – record 2026-11-12.partial");
    std::fs::create_dir(&foreign).expect("foreign folder");
    std::fs::write(foreign.join("keep.txt"), "not ours").expect("foreign file");
    let _ = crate::api::record::write_folder(&downloads, "Demo – record 2026-11-12", |folder| {
        std::fs::write(folder.join("thread.json"), "copy").map_err(|error| error.to_string())
    });
    assert!(
        foreign.join("keep.txt").is_file(),
        "a folder the export did not create survives"
    );
    let _ = std::fs::remove_dir_all(&downloads);
}

#[test]
fn a_long_multibyte_title_keeps_its_record_name_within_a_file_name() {
    // File systems such as ext4 count bytes, and the temporary folder adds `.` and `.partial`.
    let title = "語".repeat(80);
    let name = crate::api::record::record_name(&title, "2026-11-11");
    assert!(format!(".{name}.partial").len() <= 255, "{name}");
}

#[test]
fn downloads_go_to_the_folder_the_system_names_or_to_downloads() {
    let home = std::path::Path::new("/home/ana");
    assert_eq!(
        crate::api::record::download_dir_in(home, None),
        home.join("Downloads")
    );
    assert_eq!(
        crate::api::record::download_dir_in(
            home,
            Some("XDG_DESKTOP_DIR=\"$HOME/Desk\"\nXDG_DOWNLOAD_DIR=\"$HOME/Transfers\"\n")
        ),
        home.join("Transfers")
    );
    assert_eq!(
        crate::api::record::download_dir_in(home, Some("XDG_DOWNLOAD_DIR=\"/srv/incoming\"")),
        std::path::PathBuf::from("/srv/incoming")
    );
    assert_eq!(
        crate::api::record::download_dir_in(home, Some(r#"XDG_DOWNLOAD_DIR="$HOME/My \$tuff""#)),
        home.join("My $tuff")
    );
    assert_eq!(
        crate::api::record::download_dir_in(
            home,
            Some(r#"XDG_DOWNLOAD_DIR="/srv/\"q\" \\ \`x\`""#)
        ),
        std::path::PathBuf::from(r#"/srv/"q" \ `x`"#)
    );
    assert_eq!(
        crate::api::record::download_dir_in(
            home,
            Some("XDG_DOWNLOAD_DIR=\"$HOME/First\"\nXDG_DOWNLOAD_DIR=\"$HOME/Second\"\n")
        ),
        home.join("Second"),
        "the shell reads the file in order, so the last assignment is the one in effect"
    );
}

#[test]
fn a_record_of_an_untitled_session_is_named_for_its_day() {
    assert_eq!(
        crate::api::record::record_name(" . ", "2026-11-15"),
        "Untitled session – record 2026-11-15"
    );
}

#[test]
fn a_second_export_of_the_same_name_keeps_the_first_folder() {
    let downloads =
        std::env::temp_dir().join(format!("liquid-bridge-twice-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&downloads);
    std::fs::create_dir_all(&downloads).expect("downloads");
    let write = |folder: &std::path::Path| {
        std::fs::write(folder.join("thread.json"), "copy").map_err(|error| error.to_string())
    };
    let first = crate::api::record::write_folder(&downloads, "Demo", write).expect("first");
    let second = crate::api::record::write_folder(&downloads, "Demo", write).expect("second");
    assert_eq!(first.file_name().unwrap(), "Demo");
    assert_eq!(second.file_name().unwrap(), "Demo 2");
    assert!(first.join("thread.json").is_file() && second.join("thread.json").is_file());
    let _ = std::fs::remove_dir_all(&downloads);
}

#[test]
fn an_export_without_a_downloads_folder_says_so_and_writes_nothing() {
    let missing =
        std::env::temp_dir().join(format!("liquid-bridge-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let failed = crate::api::record::write_folder(&missing, "Demo", |_| Ok(()));
    assert!(
        failed.is_err_and(|error| error.starts_with("could not create ")),
        "the export names the folder it could not create"
    );
    assert!(!missing.exists());
}
