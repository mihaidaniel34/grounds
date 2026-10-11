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
    let opts = |mode| CheckOptions { mode, files: Vec::new(), all_lines: false, use_baseline: true, min_severity: Severity::Info, base: None };
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

#[test]
fn body_only_edits_update_references() {
    let dir = std::env::temp_dir().join(format!("grounds-session-refs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/demo/Api.java", "package demo;\npublic class Api {\n    public void old() {}\n}\n");
    write(&dir, "src/main/java/demo/Caller.java", "package demo;\npublic class Caller {\n    void test(Api a) {}\n}\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);
    let mut s = Session::open(&dir).unwrap();
    let caller = dir.join("src/main/java/demo/Caller.java").canonicalize().unwrap();
    let api = dir.join("src/main/java/demo/Api.java").canonicalize().unwrap();
    let opts = |files: Vec<std::path::PathBuf>| CheckOptions { mode: Mode::Edit, files, all_lines: false, use_baseline: false, min_severity: Severity::Info, base: None };

    // body-only edit: Caller starts using Api.old() (its signature is unchanged)
    write(&dir, "src/main/java/demo/Caller.java", "package demo;\npublic class Caller {\n    void test(Api a) { a.old(); }\n}\n");
    s.check(&opts(vec![caller.clone()]));
    // renaming Api.old() must list Caller as a broken caller
    write(&dir, "src/main/java/demo/Api.java", "package demo;\npublic class Api {\n    public void renamed() {}\n}\n");
    let r = s.check(&opts(vec![api]));
    assert_eq!(r.pending.len(), 1, "{:?}", r.pending);
    assert!(r.pending[0].path.ends_with("Caller.java"), "{:?}", r.pending);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn baseline_does_not_hide_the_same_bug_in_another_class() {
    let dir = std::env::temp_dir().join(format!("grounds-session-fp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/demo/Pair.java", "package demo;\nclass Pair {\n}\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);
    let mut s = Session::open(&dir).unwrap();
    let opts = CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: true, use_baseline: true, min_severity: Severity::Info, base: None };
    // A.f() has a bug; accept it into the baseline
    write(&dir, "src/main/java/demo/Pair.java", "package demo;\nclass Pair {\n    static class A {\n        void f() {\n            Object x = null;\n            x.toString();\n        }\n    }\n}\n");
    assert!(s.check(&opts).findings.iter().any(|f| f.rule == "null-dereference"));
    s.write_baseline(&[], false).unwrap();
    assert!(!s.check(&opts).findings.iter().any(|f| f.rule == "null-dereference"));
    // fix A.f(), introduce the identical statement in B.f(): a new bug
    write(&dir, "src/main/java/demo/Pair.java", "package demo;\nclass Pair {\n    static class A {\n        void f() {\n        }\n    }\n    static class B {\n        void f() {\n            Object x = null;\n            x.toString();\n        }\n    }\n}\n");
    let r = s.check(&opts);
    assert!(r.findings.iter().any(|f| f.rule == "null-dereference"), "{:?}", r.findings);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fingerprints_tell_array_overloads_apart() {
    let dir = std::env::temp_dir().join(format!("grounds-session-arr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/demo/Case.java", "package demo;\nclass Case {\n}\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);
    let mut s = Session::open(&dir).unwrap();
    let opts = CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: true, use_baseline: true, min_severity: Severity::Info, base: None };
    write(&dir, "src/main/java/demo/Case.java", "package demo;\nclass Case {\n    void f(Object p) {\n        Object x = null;\n        x.toString();\n    }\n    void f(Object p[]) {\n    }\n}\n");
    s.check(&opts);
    s.write_baseline(&[], false).unwrap();
    write(&dir, "src/main/java/demo/Case.java", "package demo;\nclass Case {\n    void f(Object p) {\n    }\n    void f(Object p[]) {\n        Object x = null;\n        x.toString();\n    }\n}\n");
    let r = s.check(&opts);
    assert!(r.findings.iter().any(|f| f.rule == "null-dereference"), "{:?}", r.findings);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn base_mode_finds_breaks_in_unchanged_files_and_writes_sarif() {
    let repo = std::env::temp_dir().join(format!("grounds-base-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    let dir = repo.join("app");
    write(&dir, "settings.gradle", "rootProject.name = 'demo'\n");
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    for a in ["Service", "Component"] {
        write(&dir, &format!("src/main/java/org/springframework/stereotype/{a}.java"), &format!("package org.springframework.stereotype;\npublic @interface {a} {{}}\n"));
    }
    write(&dir, "src/main/java/demo/Owner.java", "package demo;\npublic class Owner {\n    public void addPet(String p) {}\n    public boolean old(String s) { return s == \"x\"; }\n}\n");
    write(&dir, "src/main/java/demo/Clinic.java", "package demo;\npublic class Clinic {\n    void m(Owner o) { o.addPet(\"rex\"); }\n}\n");
    write(&dir, "src/main/java/demo/Pets.java", "package demo;\npublic interface Pets {}\n");
    write(&dir, "src/main/java/demo/PetService.java", "package demo;\n@org.springframework.stereotype.Service\npublic class PetService implements Pets {}\n");
    write(&dir, "src/main/java/demo/Vet.java", "package demo;\n@org.springframework.stereotype.Component\npublic class Vet {\n    Vet(Pets pets) {}\n}\n");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    git(&repo, &["checkout", "-qb", "feature"]);
    // the branch renames a method its caller still uses, deletes the only
    // `Pets` bean, and adds a bug on a changed line
    write(&dir, "src/main/java/demo/Owner.java", "package demo;\npublic class Owner {\n    public void registerPet(String p) { String n = null; n.trim(); }\n    public boolean old(String s) { return s == \"x\"; }\n}\n");
    std::fs::remove_file(dir.join("src/main/java/demo/PetService.java")).unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "feature"]);

    let mut s = Session::open(&dir).unwrap();
    let opts = |base: Option<String>| CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base };
    // against HEAD the working tree is clean: nothing to report
    let r = s.check(&opts(None));
    assert!(r.findings.is_empty() && r.pending.is_empty(), "{:?} {:?}", r.findings, r.pending);

    let mb = grounds_engine::git::merge_base(&dir, "main").expect("merge base");
    let r = s.check(&opts(Some(mb)));
    let rules: Vec<&str> = r.findings.iter().map(|f| f.rule).collect();
    assert!(rules.contains(&"null-dereference"), "{rules:?}");
    assert!(!rules.contains(&"string-reference-equality"), "unchanged line must not be reported: {rules:?}");
    let pending: Vec<(&str, &str)> = r.pending.iter().map(|f| (f.path.as_str(), f.rule)).collect();
    assert!(pending.contains(&("src/main/java/demo/Clinic.java", "unresolved-method")), "{pending:?}");
    assert!(pending.contains(&("src/main/java/demo/Vet.java", "bean-not-found")), "the deleted bean breaks an unchanged file: {pending:?}");
    assert_eq!(pending.len(), 2, "{pending:?}");
    // the rename is anchored on the changed line; the deleted file has no line to anchor on
    assert_eq!(r.breaks.len(), 1, "{:?}", r.breaks);
    let b = &r.breaks[0].finding;
    assert_eq!((b.path.as_str(), b.line, b.rule), ("src/main/java/demo/Owner.java", 3, "breaks-other-files"), "{b:?}");
    assert!(b.message.contains("Clinic.java:3"), "{}", b.message);

    // the index is back to the current version afterwards
    let again = s.check(&opts(grounds_engine::git::merge_base(&dir, "main")));
    assert_eq!(again.pending.len(), 2);
    assert!(s.check(&opts(None)).pending.is_empty());

    // SARIF: paths from the repository root, the break with related locations
    let sarif: serde_json::Value = serde_json::from_str(&grounds_engine::report::sarif_report(&r, &dir)).unwrap();
    let results = sarif["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results.len(), r.findings.len() + 3);
    let uris: Vec<&str> = results.iter().map(|x| x["locations"][0]["physicalLocation"]["artifactLocation"]["uri"].as_str().unwrap()).collect();
    assert!(uris.contains(&"app/src/main/java/demo/Vet.java"), "{uris:?}");
    let brk = results.iter().find(|x| x["ruleId"] == "breaks-other-files").unwrap();
    assert_eq!(brk["relatedLocations"][0]["physicalLocation"]["artifactLocation"]["uri"], "app/src/main/java/demo/Clinic.java");
    let rule_ids: Vec<&str> = sarif["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    for x in results {
        assert_eq!(rule_ids[x["ruleIndex"].as_u64().unwrap() as usize], x["ruleId"].as_str().unwrap());
    }
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn base_mode_edge_cases() {
    let repo = std::env::temp_dir().join(format!("grounds-base-edge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    let dir = repo.join("my app");
    write(&dir, "settings.gradle", "rootProject.name = 'demo'\n");
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    let j = |f: &str| format!("src/main/java/{f}");
    // the same class in two files: resolution must not depend on load or swap order
    write(&dir, &j("demo/Api.java"), "package demo;\npublic interface Api {}\n");
    write(&dir, &j("zz/Api.java"), "package demo;\npublic interface Api { void required(); }\n");
    write(&dir, &j("demo/C.java"), "package demo;\npublic class C implements Api {}\n");
    // two calls on one line: one already broken at the base
    write(&dir, &j("demo/Two.java"), "package demo;\npublic interface Two {\n    void foo();\n}\n");
    write(&dir, &j("demo/UseTwo.java"), "package demo;\nclass UseTwo {\n    void t(Two a) { a.foo(); a.bar(); }\n}\n");
    // same method name in two classes; only B's is called
    write(&dir, &j("demo/A.java"), "package demo;\npublic class A {\n    public void foo() {}\n}\n");
    write(&dir, &j("demo/B.java"), "package demo;\npublic class B {\n    public void foo() {}\n}\n");
    write(&dir, &j("demo/UseB.java"), "package demo;\nclass UseB {\n    void t(B b) { b.foo(); }\n}\n");
    write(&dir, &j("demo/I.java"), "package demo;\npublic interface I {\n}\n");
    write(&dir, &j("demo/Impl.java"), "package demo;\npublic class Impl implements I {}\n");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "diff.relative", "true"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    git(&repo, &["checkout", "-qb", "feature"]);
    write(&dir, &j("demo/Api.java"), "package demo;\n// a comment\npublic interface Api {}\n");
    write(&dir, &j("demo/Two.java"), "package demo;\npublic interface Two {\n    void bar();\n}\n");
    write(&dir, &j("demo/A.java"), "package demo;\npublic class A {\n    public void foo2() {}\n}\n");
    write(&dir, &j("demo/B.java"), "package demo;\npublic class B {\n    public void foo2() {}\n}\n");
    write(&dir, &j("demo/I.java"), "package demo;\npublic interface I {\n    void required();\n}\n");
    git(&repo, &["commit", "-qam", "feature"]);

    let mut s = Session::open(&dir).unwrap();
    let mb = grounds_engine::git::merge_base(&dir, "main").unwrap();
    let opts = CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base: Some(mb) };
    let r = s.check(&opts);
    let mut pending: Vec<(&str, u32, &str)> = r.pending.iter().map(|f| (f.path.rsplit('/').next().unwrap(), f.line, f.rule)).collect();
    pending.sort();
    assert_eq!(pending, vec![("Impl.java", 2, "unimplemented-abstract-method"), ("UseB.java", 3, "unresolved-method"), ("UseTwo.java", 3, "unresolved-method")], "{:?}", r.pending);
    let mut breaks: Vec<(&str, u32)> = r.breaks.iter().map(|b| (b.finding.path.rsplit('/').next().unwrap(), b.finding.line)).collect();
    breaks.sort();
    assert_eq!(breaks, vec![("B.java", 3), ("I.java", 3), ("Two.java", 3)], "{:?}", r.breaks);

    // the swap left duplicate-class resolution as it was
    let c = CheckOptions { mode: Mode::Checkpoint, files: vec![dir.join(j("demo/C.java"))], all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None };
    assert!(s.check(&c).findings.iter().all(|f| f.rule != "unimplemented-abstract-method"));

    let sarif: serde_json::Value = serde_json::from_str(&grounds_engine::report::sarif_report(&r, &dir)).unwrap();
    let uris: Vec<&str> = sarif["runs"][0]["results"].as_array().unwrap().iter().map(|x| x["locations"][0]["physicalLocation"]["artifactLocation"]["uri"].as_str().unwrap()).collect();
    assert!(uris.contains(&"my%20app/src/main/java/demo/UseB.java"), "{uris:?}");
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn base_mode_removed_classes_and_new_beans() {
    let repo = std::env::temp_dir().join(format!("grounds-base-removed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    let dir = repo.clone();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/org/springframework/stereotype/Component.java", "package org.springframework.stereotype;\npublic @interface Component {}\n");
    write(&dir, "src/main/java/demo/Gone.java", "package demo;\npublic class Gone {\n    public static int value() { return 1; }\n}\n");
    write(&dir, "src/main/java/demo/SamePackage.java", "package demo;\nclass SamePackage {\n    int v() { return Gone.value(); }\n}\n");
    write(&dir, "src/main/java/other/UseGone.java", "package other;\nimport demo.Gone;\nclass UseGone {\n    int v() { return Gone.value(); }\n}\n");
    // another class named Gone is imported explicitly: not the removed one
    write(&dir, "src/main/java/third/Gone.java", "package third;\npublic class Gone {}\n");
    write(&dir, "src/main/java/third/Shadowed.java", "package third;\nimport third.Gone;\nclass Shadowed { Gone g; }\n");
    write(&dir, "src/main/java/demo/Api.java", "package demo;\npublic interface Api {}\n");
    write(&dir, "src/main/java/demo/ApiImpl.java", "package demo;\n@org.springframework.stereotype.Component\npublic class ApiImpl implements Api {}\n");
    write(&dir, "src/main/java/demo/Worker.java", "package demo;\n@org.springframework.stereotype.Component\npublic class Worker {\n    Worker(Api service) {}\n}\n");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    git(&repo, &["checkout", "-qb", "feature"]);
    std::fs::remove_file(dir.join("src/main/java/demo/Gone.java")).unwrap();
    write(&dir, "src/main/java/demo/ApiImpl2.java", "package demo;\n@org.springframework.stereotype.Component\npublic class ApiImpl2 implements Api {}\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "feature"]);

    let mut s = Session::open(&dir).unwrap();
    let mb = grounds_engine::git::merge_base(&dir, "main").unwrap();
    let opts = CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base: Some(mb) };
    let r = s.check(&opts);
    let mut pending: Vec<(&str, u32, &str)> = r.pending.iter().map(|f| (f.path.as_str(), f.line, f.rule)).collect();
    pending.sort();
    assert_eq!(
        pending,
        vec![
            ("src/main/java/demo/SamePackage.java", 3, "removed-class-still-used"),
            ("src/main/java/demo/Worker.java", 4, "ambiguous-bean"),
            ("src/main/java/other/UseGone.java", 2, "removed-class-still-used"),
        ],
        "{:?}",
        r.pending
    );
    // the new bean is blamed on its declaration
    let breaks: Vec<(&str, u32)> = r.breaks.iter().map(|b| (b.finding.path.as_str(), b.finding.line)).collect();
    assert_eq!(breaks, vec![("src/main/java/demo/ApiImpl2.java", 3)], "{:?}", r.breaks);
    let sarif: serde_json::Value = serde_json::from_str(&grounds_engine::report::sarif_report(&r, &dir)).unwrap();
    let rules: Vec<&str> = sarif["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert!(rules.contains(&"Removed class still used"), "{rules:?}");
    let _ = std::fs::remove_dir_all(&repo);
}
