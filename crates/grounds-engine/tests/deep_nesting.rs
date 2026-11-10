//! Files nested deeper than the analysis limit are skipped, not crashed on.
//! A 13,000-term string concatenation (checkstyle's test inputs) or a
//! generated lexer overflowed the stack of the thread analyzing it, which
//! aborted the whole process.

use grounds_engine::session::{CheckOptions, Session};
use grounds_rules::Severity;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").status().unwrap().success();
    assert!(ok, "git {args:?}");
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn deeply_nested_files_are_skipped_and_the_rest_is_checked() {
    let dir = std::env::temp_dir().join(format!("grounds-deep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let terms = 20_000;
    write(&dir, "src/main/java/demo/Deep.java", &format!("package demo;\nclass Deep {{\n    static final String S = \"a\"{};\n    String f(String x) {{ return x{}; }}\n    boolean g(String s) {{ return s == \"x\"; }}\n}}\n", " + \"b\"".repeat(terms), " + x".repeat(terms)));
    write(&dir, "src/main/java/demo/Plain.java", "package demo;\nclass Plain {\n    boolean g(String s) { return s == \"x\"; }\n}\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);

    let mut s = Session::open(&dir).unwrap();
    let files = vec![dir.join("src/main/java/demo/Deep.java"), dir.join("src/main/java/demo/Plain.java")];
    let r = s.check(&CheckOptions { mode: grounds_engine::session::Mode::Checkpoint, files, all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None });
    let paths: Vec<&str> = r.findings.iter().filter(|f| f.rule == "string-reference-equality").map(|f| f.path.as_str()).collect();
    assert_eq!(paths.len(), 1, "{:?}", r.findings.iter().map(|f| (f.rule, &f.path)).collect::<Vec<_>>());
    assert!(paths[0].ends_with("Plain.java"), "{paths:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
