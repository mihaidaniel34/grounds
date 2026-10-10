//! End-to-end: a git repo, a breaking edit, and the agent-facing checks.

use grounds_engine::session::{CheckOptions, Mode, Session};
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
fn edit_checkpoint_baseline_and_pending_callers() {
    let dir = std::env::temp_dir().join(format!("grounds-session-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "settings.gradle", "rootProject.name = 'demo'\n");
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/demo/Owner.java", "package demo;\npublic class Owner {\n    public void addPet(String p) {}\n    public boolean old(String s) { return s == \"x\"; }\n}\n");
    write(&dir, "src/main/java/demo/Clinic.java", "package demo;\npublic class Clinic {\n    void m(Owner o) { o.addPet(\"rex\"); }\n}\n");
    write(&dir, "src/test/java/demo/OwnerTest.java", "package demo;\nclass OwnerTest { void t() { new Owner().addPet(\"a\"); } }\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);

    let mut s = Session::open(&dir).unwrap();
    // pre-existing problem on an unchanged line is not reported
    let opts = |mode| CheckOptions { mode, files: Vec::new(), all_lines: false, use_baseline: true, min_severity: Severity::Info };
    assert!(s.check(&opts(Mode::Checkpoint)).findings.is_empty());

    // breaking edit: rename + new bug on a changed line
    write(&dir, "src/main/java/demo/Owner.java", "package demo;\npublic class Owner {\n    public void registerPet(String p) { String n = null; n.trim(); }\n    public boolean old(String s) { return s == \"x\"; }\n}\n");
    let r = s.check(&opts(Mode::Edit));
    let rules: Vec<&str> = r.findings.iter().map(|f| f.rule).collect();
    assert!(rules.contains(&"null-dereference"), "{rules:?}");
    assert!(!rules.contains(&"string-reference-equality"), "unchanged line must not be reported: {rules:?}");
    assert_eq!(r.pending.len(), 2, "both callers are broken: {:?}", r.pending);
    assert_eq!(r.changed_apis, vec!["src/main/java/demo/Owner.java".to_string()]);

    // impacted tests
    let tests = s.impacted_tests(&[]);
    assert!(tests.iter().any(|t| t.class == "demo.OwnerTest" && t.task == "test"), "{tests:?}");

    // baseline accepts the current state
    s.write_baseline(&[], false).unwrap();
    let r = s.check(&opts(Mode::Checkpoint));
    assert!(r.findings.is_empty(), "{:?}", r.findings);

    // fixing callers clears the pending list
    write(&dir, "src/main/java/demo/Clinic.java", "package demo;\npublic class Clinic {\n    void m(Owner o) { o.registerPet(\"rex\"); }\n}\n");
    write(&dir, "src/test/java/demo/OwnerTest.java", "package demo;\nclass OwnerTest { void t() { new Owner().registerPet(\"a\"); } }\n");
    let r = s.check(&opts(Mode::Edit));
    assert!(r.pending.is_empty(), "{:?}", r.pending);
    let _ = std::fs::remove_dir_all(&dir);
}
