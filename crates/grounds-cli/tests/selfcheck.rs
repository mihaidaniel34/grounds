//! The `grounds selfcheck` command: its exit codes (0 no disagreement, 2
//! disagreements, 1 failure, as `grounds changed`) and its JSON. What it
//! compares is tested in crates/grounds-engine/tests/selfcheck.rs.

use std::path::{Path, PathBuf};
use std::process::Command;

fn javac() -> Option<PathBuf> {
    let home = grounds_classfile::find_java_home().map(|h| h.join("bin/javac"));
    home.into_iter().chain([PathBuf::from("javac")]).find(|c| Command::new(c).arg("-version").env_remove("JAVA_TOOL_OPTIONS").output().is_ok_and(|o| o.status.success()))
}

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grounds-cli-selfcheck-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, text) in [("pom.xml", "<project><modelVersion>4.0.0</modelVersion><groupId>t</groupId><artifactId>t</artifactId><version>1</version></project>\n"), ("grounds.toml", "java_release = 17\n[classpath]\nscan_caches = false\n")].iter().chain(files) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn selfcheck(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_grounds")).arg("selfcheck").arg("--root").arg(dir).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

const SIGNAL: &str = "package demo;\nenum Kind { NEXT, DONE }\nfinal class Signal {\n    Kind kind = Kind.NEXT;\n    public String toString() {\n        switch (this.kind) {\n            case DONE:\n                return \"done\";\n            default:\n                return \"next\";\n        }\n    }\n    private static final Signal DONE = new Signal();\n}\n";

#[test]
fn no_build_output_exits_0_with_the_way_to_get_one() {
    let dir = project("none", &[("src/main/java/demo/Signal.java", SIGNAL)]);
    let (code, out) = selfcheck(&dir, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("no build output found (looked in target/classes, build/classes/java/main)"), "{out}");
    assert!(out.contains("`mvn test-compile`"), "{out}");
    let (code, _) = selfcheck(&dir, &["--format", "sarif"]);
    assert_eq!(code, 1, "a format it does not write is a failure");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_disagreement_exits_2() {
    let Some(javac) = javac() else {
        eprintln!("skipped: no javac (set JAVA_HOME or put javac on PATH)");
        return;
    };
    let dir = project("disagree", &[("src/main/java/demo/Signal.java", SIGNAL)]);
    let out = Command::new(javac).arg("--release").arg("17").arg("-d").arg(dir.join("target/classes")).arg(dir.join("src/main/java/demo/Signal.java")).env_remove("JAVA_TOOL_OPTIONS").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // (the enum case label grounds resolves to the field in scope: see
    // an_enum_case_label_resolved_to_a_field_in_scope_is_reported)
    let (code, out) = selfcheck(&dir, &["--format", "json"]);
    assert_eq!(code, 2, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["disagreements"][0]["path"], "src/main/java/demo/Signal.java");
    assert_eq!(v["disagreements"][0]["line"], 7);
    assert_eq!(v["categories"]["fields"]["disagree"], 1);
    assert_eq!(v["modules"][0]["source_sets"][0]["classes"], "target/classes");
    let _ = std::fs::remove_dir_all(&dir);
}
