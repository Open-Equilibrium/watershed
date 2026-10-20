use super::{Fixture, assert_success, limits};
use serde_json::{Value, json};
use std::fs;

#[cfg(target_os = "macos")]
fn xcrun(argument: &str) -> String {
    let output = std::process::Command::new("/usr/bin/xcrun")
        .args(argument.split_whitespace())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "required host compiler/SDK unavailable: {output:?}"
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn compiler(kind: &str) -> (String, Vec<String>) {
    if kind == "javascript" {
        return (String::new(), vec![]);
    }
    #[cfg(target_os = "macos")]
    {
        if kind == "native-xcrun" {
            return ("/usr/bin/xcrun".to_owned(), vec!["clang".to_owned()]);
        }
        let clang = xcrun("--find clang");
        let linker = xcrun("--find ld");
        let sdk = xcrun("--show-sdk-path");
        (
            clang,
            vec![
                "-isysroot".to_owned(),
                sdk,
                "-B".to_owned(),
                std::path::Path::new(&linker)
                    .parent()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
            ],
        )
    }
    #[cfg(target_os = "linux")]
    {
        ("/usr/bin/cc".to_owned(), vec![])
    }
}

pub fn npm_lifecycle(kind: &str) {
    let (compiler, compiler_args) = compiler(kind);
    for guarded in [false, true] {
        let fixture = Fixture::new();
        fs::write(fixture.home.join("npm-parent"), b"original").unwrap();
        fs::write(fixture.project.join("package.json"), serde_json::to_vec(&json!({
            "name":"native-guard-build", "version":"1.0.0", "private":true,
            "scripts":{
                "prebuild":"node -e \"require('fs').writeFileSync('prebuild','ok')\"",
                "build":"node build.cjs",
                "postbuild":"node -e \"const f=require('fs'); if(f.readFileSync('artifact','utf8')!=='built') process.exit(1); f.writeFileSync('postbuild','ok')\""
            }
        })).unwrap()).unwrap();
        fs::write(fixture.project.join("build.cjs"), include_str!("build.cjs")).unwrap();
        fs::write(
            fixture.project.join("native_writer.c"),
            include_str!("native_writer.c"),
        )
        .unwrap();
        fs::write(fixture.project.join("build-settings.json"), serde_json::to_vec(&json!({
            "kind":kind, "compiler":compiler, "compiler_args":compiler_args,
            "target":fixture.home.join("AGENTS.md"), "parent_target":fixture.home.join("npm-parent")
        })).unwrap()).unwrap();
        for name in ["user-npmrc", "global-npmrc"] {
            fs::write(fixture.project.join(name), "").unwrap();
        }
        let request = fixture.request(
            "export TMPDIR=$PWD; export npm_config_cache=$PWD/cache; export npm_config_userconfig=$PWD/user-npmrc; export npm_config_globalconfig=$PWD/global-npmrc; exec npm --offline --no-audit --no-fund run build",
            limits(16 * 1024, 16 * 1024, 15_000));
        if guarded {
            let mut running = fixture.spawn(&request);
            running.ready(&request);
            running.start(&request);
            assert_success(&running.completed(&request));
        } else {
            fixture.baseline(&request, &[]);
        }
        for (path, expected) in [
            ("prebuild", "ok"),
            ("artifact", "built"),
            ("postbuild", "ok"),
        ] {
            assert_eq!(
                fs::read_to_string(fixture.project.join(path)).unwrap(),
                expected
            );
        }
        let result: Value =
            serde_json::from_slice(&fs::read(fixture.project.join("build-result.json")).unwrap())
                .unwrap();
        let expected = if guarded { 10 } else { 0 };
        assert_eq!(result, json!({"child":expected, "parent":expected}));
        assert_eq!(
            fs::read(fixture.home.join("AGENTS.md")).unwrap(),
            if guarded {
                b"global instructions".as_slice()
            } else {
                b"changed"
            }
        );
        assert_eq!(
            fs::read(fixture.home.join("npm-parent")).unwrap(),
            if guarded {
                b"original".as_slice()
            } else {
                b"changed"
            }
        );
    }
}

#[test]
fn npm_compiles_and_runs_native_children_with_the_direct_host_compiler() {
    npm_lifecycle("native-direct");
}

#[cfg(target_os = "macos")]
#[test]
fn npm_compiles_and_runs_native_children_through_xcrun() {
    npm_lifecycle("native-xcrun");
}

#[test]
fn shell_compiles_and_runs_a_native_writer_under_the_guard() {
    #[cfg(target_os = "macos")]
    let compile = "/usr/bin/xcrun clang native_writer.c -o generated";
    #[cfg(target_os = "linux")]
    let compile = "/usr/bin/cc native_writer.c -o generated";
    for guarded in [false, true] {
        let fixture = Fixture::new();
        fs::write(
            fixture.project.join("native_writer.c"),
            include_str!("native_writer.c"),
        )
        .unwrap();
        let script = format!(
            "set -eu\nexport TMPDIR=$PWD\n{compile}\n./generated artifact built\nstatus=0\n./generated \"$1/AGENTS.md\" changed || status=$?\nprintf '%s' \"$status\" > writer-result"
        );
        let request = fixture.request(&script, limits(4096, 4096, 15_000));
        if guarded {
            let mut running = fixture.spawn(&request);
            running.ready(&request);
            running.start(&request);
            assert_success(&running.completed(&request));
        } else {
            fixture.baseline(&request, &[]);
        }
        assert_eq!(
            fs::read(fixture.project.join("artifact")).unwrap(),
            b"built"
        );
        assert_eq!(
            fs::read_to_string(fixture.project.join("writer-result")).unwrap(),
            if guarded { "10" } else { "0" }
        );
        assert_eq!(
            fs::read(fixture.home.join("AGENTS.md")).unwrap(),
            if guarded {
                b"global instructions".as_slice()
            } else {
                b"changed"
            }
        );
    }
}
