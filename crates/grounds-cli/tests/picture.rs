//! `grounds check` on a project never built: its compile-break findings are
//! made under a guessed build picture, and one line on stderr says so for the
//! run, not one per finding; `[classpath] warn = false` turns it off with the
//! other classpath warnings. The text findings are as they were.

use std::path::{Path, PathBuf};
use std::process::Command;

fn project(name: &str, toml: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grounds-cli-picture-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let files = [
        ("pom.xml", "<project><modelVersion>4.0.0</modelVersion><groupId>t</groupId><artifactId>t</artifactId><version>1</version></project>\n"),
        ("grounds.toml", toml),
        ("src/main/java/app/Lib.java", "package app;\npublic class Lib { public void run() {} }\n"),
        ("src/main/java/app/Use.java", "package app;\nclass Use {\n    void f(Lib l) { l.missing(); }\n    void g(Lib l) { l.absent(); }\n}\n"),
    ];
    for (rel, text) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn check(dir: &Path) -> (String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_grounds")).arg("check").arg("--root").arg(dir).arg("--rules").arg("unresolved-method").output().unwrap();
    (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn a_guessed_build_picture_is_said_once_per_run() {
    let dir = project("once", "java_release = 17\n[classpath]\nscan_caches = false\n");
    let (out, err) = check(&dir);
    assert_eq!(out, "src/main/java/app/Use.java:3:23: error [unresolved-method] `Lib` has no method `missing`\nsrc/main/java/app/Use.java:4:23: error [unresolved-method] `Lib` has no method `absent`\n");
    assert_eq!(err.matches("grounds: warning: the build picture is guessed for 1 module(s): 2 compile-break finding(s)").count(), 1, "{err}");
    let dir2 = project("off", "java_release = 17\n[classpath]\nscan_caches = false\nwarn = false\n");
    let (out2, err2) = check(&dir2);
    assert_eq!(out2, out);
    assert!(!err2.contains("build picture"), "{err2}");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}
