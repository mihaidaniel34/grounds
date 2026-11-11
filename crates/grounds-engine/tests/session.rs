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
    // an application (not a library): its context is complete
    write(&dir, "src/main/java/org/springframework/boot/autoconfigure/SpringBootApplication.java", "package org.springframework.boot.autoconfigure;\npublic @interface SpringBootApplication {}\n");
    write(&dir, "src/main/java/demo/App.java", "package demo;\n@org.springframework.boot.autoconfigure.SpringBootApplication\npublic class App { public static void main(String[] args) {} }\n");
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
    let sarif: serde_json::Value = serde_json::from_str(&grounds_engine::report::sarif_report(&r, &dir, &[])).unwrap();
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

    let sarif: serde_json::Value = serde_json::from_str(&grounds_engine::report::sarif_report(&r, &dir, &[])).unwrap();
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
    // an application (not a library): its context is complete
    write(&dir, "src/main/java/org/springframework/boot/autoconfigure/SpringBootApplication.java", "package org.springframework.boot.autoconfigure;\npublic @interface SpringBootApplication {}\n");
    write(&dir, "src/main/java/demo/App.java", "package demo;\n@org.springframework.boot.autoconfigure.SpringBootApplication\npublic class App { public static void main(java.lang.String[] args) {} }\n");
    write(&dir, "src/main/java/demo/Gone.java", "package demo;\npublic class Gone {\n    public static int value() { return 1; }\n}\n");
    write(&dir, "src/main/java/demo/SamePackage.java", "package demo;\nclass SamePackage {\n    int v() { return Gone.value(); }\n}\n");
    write(&dir, "src/main/java/other/UseGone.java", "package other;\nimport demo.Gone;\nclass UseGone {\n    int v() { return Gone.value(); }\n}\n");
    // another class named Gone is imported explicitly: not the removed one
    write(&dir, "src/main/java/third/Gone.java", "package third;\npublic class Gone {}\n");
    write(&dir, "src/main/java/third/Shadowed.java", "package third;\nimport third.Gone;\nclass Shadowed { Gone g; }\n");
    // same name, other bindings: a nested class, a field, a qualified name,
    // and a same-package class winning over an on-demand import
    write(&dir, "src/main/java/demo/Nested.java", "package demo;\nclass Nested {\n    static class Gone {}\n    Gone value;\n}\n");
    write(&dir, "src/main/java/demo/Field.java", "package demo;\nclass Field {\n    int Gone;\n    int v() { return Gone; }\n}\n");
    write(&dir, "src/main/java/other/Qualified.java", "package other;\nclass Qualified { third.Gone g; }\n");
    write(&dir, "src/main/java/third/OnDemand.java", "package third;\nimport demo.*;\nclass OnDemand { Gone g; }\n");
    // a qualified use breaks too; another on-demand import of a package we
    // cannot see may supply the name once the class is gone
    write(&dir, "src/main/java/other/FullyQualified.java", "package other;\nclass FullyQualified {\n    demo.Gone g;\n}\n");
    write(&dir, "src/main/java/demo/Wildcard.java", "package demo;\nimport somelib.*;\nclass Wildcard { Gone g; }\n");
    // a static on-demand import may supply a nested Gone; a qualifier may be
    // a variable inherited from a supertype we cannot see; qualified in the
    // same package still breaks
    write(&dir, "src/main/java/demo/StaticWildcard.java", "package demo;\nimport static somelib.Holder.*;\nclass StaticWildcard { Gone g; }\n");
    write(&dir, "src/main/java/other/Inherited.java", "package other;\nclass Inherited extends somelib.Base {\n    int v() { return demo.Gone.value(); }\n}\n");
    write(&dir, "src/main/java/demo/SelfQualified.java", "package demo;\nclass SelfQualified {\n    demo.Gone g;\n}\n");
    // the simple name inside a class whose supertype we cannot see: that
    // supertype may declare a member type Gone, which is what javac then binds
    write(&dir, "src/main/java/demo/InheritsName.java", "package demo;\nclass InheritsName extends somelib.Base {\n    Gone value;\n    int v() { return Gone.value(); }\n}\n");
    // the qualifier names a class here (one that may inherit a nested Gone)
    write(&dir, "src/main/java/fourth/demo.java", "package fourth;\nclass demo extends somelib.Base {}\n");
    write(&dir, "src/main/java/fourth/ClassQualifier.java", "package fourth;\nclass ClassQualifier { demo.Gone g; }\n");
    // deleting a class named like a java.lang one: the name falls back to java.lang
    write(&dir, "src/main/java/demo/String.java", "package demo;\npublic class String {}\n");
    write(&dir, "src/main/java/demo/Text.java", "package demo;\nclass Text { String s; }\n");
    write(&dir, "src/main/java/demo/Api.java", "package demo;\npublic interface Api {}\n");
    write(&dir, "src/main/java/demo/ApiImpl.java", "package demo;\n@org.springframework.stereotype.Component\npublic class ApiImpl implements Api {}\n");
    write(&dir, "src/main/java/demo/Worker.java", "package demo;\n@org.springframework.stereotype.Component\npublic class Worker {\n    Worker(Api service) {}\n}\n");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    git(&repo, &["checkout", "-qb", "feature"]);
    std::fs::remove_file(dir.join("src/main/java/demo/Gone.java")).unwrap();
    std::fs::remove_file(dir.join("src/main/java/demo/String.java")).unwrap();
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
            ("src/main/java/demo/SelfQualified.java", 3, "removed-class-still-used"),
            ("src/main/java/demo/Worker.java", 4, "ambiguous-bean"),
            ("src/main/java/other/FullyQualified.java", 3, "removed-class-still-used"),
            ("src/main/java/other/UseGone.java", 2, "removed-class-still-used"),
        ],
        "{:?}",
        r.pending
    );
    // the new bean is blamed on its declaration
    let breaks: Vec<(&str, u32)> = r.breaks.iter().map(|b| (b.finding.path.as_str(), b.finding.line)).collect();
    assert_eq!(breaks, vec![("src/main/java/demo/ApiImpl2.java", 3)], "{:?}", r.breaks);
    let sarif: serde_json::Value = serde_json::from_str(&grounds_engine::report::sarif_report(&r, &dir, &[])).unwrap();
    let rules: Vec<&str> = sarif["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert!(rules.contains(&"Removed class still used"), "{rules:?}");
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn classpath_warnings_once_per_session_and_at_session_start() {
    use grounds_engine::daemon::{execute, Request};
    let dir = std::env::temp_dir().join(format!("grounds-session-warn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "pom.xml", "<project/>\n");
    write(&dir, "grounds.toml", "[classpath]\nfiles = [\".grounds/classpath.txt\"]\n");
    write(&dir, "src/main/java/demo/A.java", "package demo;\nclass A {}\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);

    let mut s = Session::open(&dir).unwrap();
    let check = |sarif| Request::Check { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, baseline: true, max: 15, min_severity: "warning".into(), json: false, color: false, base: None, sarif };
    let warned = execute(&mut s, &check(true));
    assert_eq!(warned.warnings.len(), 1, "{:?}", warned.warnings);
    assert!(warned.warnings[0].contains(".grounds/classpath.txt"));
    let sarif: serde_json::Value = serde_json::from_str(&warned.stdout).unwrap();
    let note = &sarif["runs"][0]["invocations"][0]["toolExecutionNotifications"][0];
    assert_eq!(note["level"], "warning");
    assert!(note["message"]["text"].as_str().unwrap().starts_with("classpath file .grounds/classpath.txt"));
    // once per session; a new agent session hears it again
    let again = execute(&mut s, &check(true));
    assert!(again.warnings.is_empty());
    let sarif: serde_json::Value = serde_json::from_str(&again.stdout).unwrap();
    assert!(sarif["runs"][0].get("invocations").is_none());
    assert_eq!(execute(&mut s, &Request::SessionStart).warnings, warned.warnings);
    assert!(execute(&mut s, &check(false)).warnings.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_body_only_edit_resolves_classes_as_a_fresh_load_does() {
    let dir = std::env::temp_dir().join(format!("grounds-session-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/p/A.java", "package p;\npublic class A extends CA {\n    public static final int X = B.KB;\n    void m() {}\n}\n");
    write(&dir, "src/main/java/p/CA.java", "package p;\npublic class CA { public static final int KA = 1; }\n");
    write(&dir, "src/main/java/p/B.java", "package p;\npublic class B extends CB { public static final int Y = A.KA; }\n");
    write(&dir, "src/main/java/p/CB.java", "package p;\npublic class CB { public static final int KB = 2; }\n");
    write(&dir, "src/main/java/p/Other.java", "package p;\npublic class Other {\n    void m() {}\n}\n");
    let classes = |index: &grounds_index::Index| {
        let _ = index.class("p.B");
        let mut names: Vec<_> = index.source_class_names().cloned().collect();
        names.sort();
        names.iter().map(|n| format!("{:?}", index.class(n))).collect::<Vec<_>>()
    };
    let mut s = Session::open(&dir).unwrap();
    let edits = [
        ("src/main/java/p/Other.java", "package p;\npublic class Other {\n    void m() { int i = 0; }\n}\n"),
        ("src/main/java/p/A.java", "package p;\npublic class A extends CA {\n    public static final int X = B.KB;\n    void m() {\n        int i = 0;\n    }\n}\n"),
    ];
    for (i, (file, text)) in edits.iter().enumerate() {
        write(&dir, file, text);
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5 * (i as u64 + 1));
        std::fs::File::options().write(true).open(dir.join(file)).unwrap().set_modified(later).unwrap();
        s.sync(&[]);
        let fresh = classes(&grounds_engine::Project::load(&dir).unwrap().index);
        assert_eq!(classes(&s.project.index), fresh, "after a body-only edit of {file}, the classes must resolve as a fresh load resolves them");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_changed_build_file_reloads_the_project() {
    let dir = std::env::temp_dir().join(format!("grounds-session-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "settings.gradle", "include 'app'\n");
    write(&dir, "app/build.gradle", "plugins { id 'java' }\n");
    write(&dir, "app/src/main/java/demo/App.java", "package demo;\npublic class App {}\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);
    let mut s = Session::open(&dir).unwrap();
    let modules = |s: &Session| {
        let mut names: Vec<String> = s.project.workspace.modules.iter().map(|m| m.name.clone()).collect();
        names.sort();
        names
    };
    let module_of = |s: &Session, rel: &str| {
        let ws = &s.project.workspace;
        let file = ws.root.join(rel);
        ws.files.iter().find(|f| f.path == file).map(|f| ws.modules[f.module].name.clone())
    };
    assert_eq!(modules(&s), [":", "app"]);

    // a second module: its build file, the settings that include it, a source file
    write(&dir, "lib/build.gradle", "plugins { id 'java-library' }\n");
    write(&dir, "lib/src/main/java/demo/Lib.java", "package demo;\npublic class Lib {}\n");
    write(&dir, "settings.gradle", "include 'app', 'lib'\n");
    // (far enough ahead that the dump written below is older than it however slowly the test runs)
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
    std::fs::File::options().write(true).open(dir.join("settings.gradle")).unwrap().set_modified(later).unwrap();
    s.sync(&[dir.join("lib/src/main/java/demo/Lib.java")]);
    let fresh = Session::open(&dir).unwrap();
    assert_eq!(modules(&s), modules(&fresh), "the modules a fresh load finds");
    assert_eq!(modules(&s), [":", "app", "lib"]);
    assert_eq!(module_of(&s, "lib/src/main/java/demo/Lib.java").as_deref(), Some("lib"), "the new file belongs to the new module");

    // a setting changed: grounds.toml did not exist when the session opened
    assert!(s.project.config.exclude.is_empty());
    write(&dir, "grounds.toml", "exclude = [\"lib/**\"]\n");
    s.sync(&[]);
    assert_eq!(s.project.config.exclude, ["lib/**"]);
    // one reload settles it: the next request, moments later, loads nothing
    let loaded = s.project.load_began;
    s.sync(&[]);
    assert_eq!(s.project.load_began, loaded, "a configuration file just written must not reload the project on every request");
    assert_eq!(module_of(&s, "lib/src/main/java/demo/Lib.java"), None, "the excluded file is gone, as in a fresh load");

    // a registration file that did not exist: Spring's imports of the app module
    assert!(s.project.registered.is_empty());
    write(&dir, "app/src/main/resources/META-INF/spring/org.springframework.boot.autoconfigure.AutoConfiguration.imports", "demo.App\n");
    s.sync(&[]);
    assert_eq!(s.project.registered, ["demo.App"]);

    // the classpath dump `grounds classpath` writes, created after the session opened
    // (a load warns that it is older than the build file above)
    assert!(s.project.classpath_warnings.is_empty(), "{:?}", s.project.classpath_warnings);
    write(&dir, ".grounds/classpath.txt", &format!("{}\n", dir.join("libs/some.jar").display()));
    s.sync(&[]);
    let warnings: Vec<String> = s.project.classpath_warnings.iter().map(|w| w.to_string()).collect();
    assert!(warnings.iter().any(|w| w.contains("classpath.txt")), "the dump is read once it exists: {warnings:?}");

    // the jar it lists appears (a dependency built later): a class from it is known
    assert!(!s.project.index.class_exists("lib.Dep"));
    write(&dir, "libs/src/lib/Dep.java", "package lib;\npublic class Dep {}\n");
    // (older javac does not create the output directory itself)
    std::fs::create_dir_all(dir.join("libs/classes")).unwrap();
    let javac = Command::new("javac").arg("-d").arg(dir.join("libs/classes")).arg(dir.join("libs/src/lib/Dep.java")).status();
    let jar = Command::new("jar").arg("cf").arg(dir.join("libs/some.jar")).arg("-C").arg(dir.join("libs/classes")).arg("lib").status();
    if javac.is_ok_and(|s| s.success()) && jar.is_ok_and(|s| s.success()) {
        s.sync(&[]);
        assert!(s.project.index.class_exists("lib.Dep"), "the jar is read once it is there");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A project whose committed code already has a compile-break claim the
/// model cannot back (`l.gap()`, which stands for a call to a member that a
/// framework feature the model lacks generates): the claim exists before and
/// after a change, so `grounds changed` must not report it, even on a line
/// the change touches, while what the change does break is reported.
fn gap_project(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let repo = std::env::temp_dir().join(format!("grounds-gap-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    let dir = repo.join("app");
    write(&dir, "settings.gradle", "rootProject.name = 'demo'\n");
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/demo/Lib.java", "package demo;\npublic class Lib {\n    public void real() {}\n    public void other() {}\n}\n");
    write(&dir, "src/main/java/demo/Use.java", "package demo;\nclass Use {\n    void a(Lib l) { l.gap(); }\n    void b(Lib l) { l.real(); }\n}\n");
    write(&dir, "src/main/java/demo/Other.java", "package demo;\nclass Other {\n    void c(Lib l) { l.gap(); }\n    void d(Lib l) { l.real(); }\n}\n");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    (repo, dir)
}

fn claims(r: &grounds_engine::session::Report) -> Vec<(String, u32, String)> {
    let mut v: Vec<(String, u32, String)> = r.findings.iter().filter(|f| f.rule == "unresolved-method").map(|f| (f.path.rsplit('/').next().unwrap().to_string(), f.line, f.message.clone())).collect();
    v.sort();
    v
}

fn json(r: &grounds_engine::session::Report) -> String {
    serde_json::to_string_pretty(r).unwrap()
}

#[test]
fn a_pre_existing_break_claim_on_a_changed_line_is_not_reported() {
    let (repo, dir) = gap_project("per-turn");
    let use_java = "src/main/java/demo/Use.java";
    let opts = |base: Option<String>| CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base };
    // a whole-project run reports the gap: that is what `check` is told
    let mut warm = Session::open(&dir).unwrap();
    let all = warm.check(&CheckOptions { all_lines: true, files: vec![dir.join(use_java)], ..opts(None) });
    assert_eq!(claims(&all).len(), 1, "{:?}", all.findings);
    assert_eq!(all.findings.iter().find(|f| f.rule == "unresolved-method").unwrap().maturity, grounds_rules::Maturity::Preview, "on an unchanged line it is not the promise");

    // 1. the change edits the very line of the gap, harmlessly: not reported
    write(&dir, use_java, "package demo;\nclass Use {\n    void a(Lib l) { l.gap(); /* touched */ }\n    void b(Lib l) { l.real(); }\n}\n");
    let r = warm.check(&opts(None));
    assert_eq!(claims(&r), vec![], "{:?}", r.findings);
    assert_eq!(r.outside_changed_lines, 0);
    // the post-edit hook, with the file named
    let edit = warm.check(&CheckOptions { mode: Mode::Edit, files: vec![dir.join(use_java)], ..opts(None) });
    assert_eq!(claims(&edit), vec![]);

    // 2. a new break on a changed line next to it: reported, as stable
    write(&dir, use_java, "package demo;\nclass Use {\n    void a(Lib l) { l.gap(); /* touched */ }\n    void b(Lib l) { l.missing(); }\n}\n");
    let r = warm.check(&opts(None));
    assert_eq!(claims(&r), vec![("Use.java".to_string(), 4, "`Lib` has no method `missing`".to_string())], "{:?}", r.findings);
    let v: serde_json::Value = serde_json::from_str(&json(&r)).unwrap();
    let f = v["findings"].as_array().unwrap().iter().find(|f| f["rule"] == "unresolved-method").unwrap();
    assert_eq!(f["maturity"], "stable");
    assert_eq!(r.findings.iter().find(|f| f.rule == "unresolved-method").unwrap().maturity, grounds_rules::Maturity::Stable);
    let fresh = Session::open(&dir).unwrap().check(&opts(None));
    assert_eq!(json(&fresh), json(&r), "a fresh session (no daemon) answers as the warm one");

    // 3. the gap's line moved to the end of the class and edited: not reported
    write(&dir, use_java, "package demo;\nclass Use {\n    void b(Lib l) { l.real(); }\n\n    void a(Lib l) {\n        l.gap();\n    }\n}\n");
    let r = warm.check(&opts(None));
    assert_eq!(claims(&r), vec![], "{:?}", r.findings);
    assert_eq!(json(&Session::open(&dir).unwrap().check(&opts(None))), json(&r));

    // 4. a new file: everything in it is new, except a claim the project
    // before the change already got (the same call in untouched code)
    write(&dir, use_java, "package demo;\nclass Use {\n    void a(Lib l) { l.gap(); }\n    void b(Lib l) { l.real(); }\n}\n");
    write(&dir, "src/main/java/demo/Fresh.java", "package demo;\nclass Fresh {\n    void e(Lib l) { l.nowhere(); }\n    void f(Lib l) { l.gap(); }\n}\n");
    let r = warm.check(&opts(None));
    assert_eq!(claims(&r), vec![("Fresh.java".to_string(), 3, "`Lib` has no method `nowhere`".to_string())], "{:?}", r.findings);
    assert_eq!(json(&Session::open(&dir).unwrap().check(&opts(None))), json(&r));

    // 5. the same in CI mode, against the merge base, once committed
    git(&repo, &["checkout", "-qb", "feature"]);
    write(&dir, use_java, "package demo;\nclass Use {\n    void a(Lib l) { l.gap(); /* touched */ }\n    void b(Lib l) { l.missing(); }\n}\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "feature"]);
    let mb = grounds_engine::git::merge_base(&dir, "main").unwrap();
    let r = warm.check(&opts(Some(mb.clone())));
    assert_eq!(
        claims(&r),
        vec![("Fresh.java".to_string(), 3, "`Lib` has no method `nowhere`".to_string()), ("Use.java".to_string(), 4, "`Lib` has no method `missing`".to_string())],
        "{:?}",
        r.findings
    );
    assert!(r.pending.is_empty(), "{:?}", r.pending);
    assert_eq!(json(&Session::open(&dir).unwrap().check(&opts(Some(mb)))), json(&r));
    // against HEAD the branch is clean
    assert!(warm.check(&opts(None)).findings.is_empty());
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn per_turn_breaks_in_other_files_are_only_those_the_change_made() {
    let (repo, dir) = gap_project("pending");
    let opts = CheckOptions { mode: Mode::Edit, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base: None };
    let mut warm = Session::open(&dir).unwrap();
    assert!(warm.check(&opts).pending.is_empty());
    // renaming `real` breaks its callers in Use and Other; their `l.gap()`
    // calls were already claimed broken before and are not the change's
    write(&dir, "src/main/java/demo/Lib.java", "package demo;\npublic class Lib {\n    public void real2() {}\n    public void other() {}\n}\n");
    let r = warm.check(&opts);
    let mut pending: Vec<(&str, u32)> = r.pending.iter().map(|f| (f.path.rsplit('/').next().unwrap(), f.line)).collect();
    pending.sort();
    assert_eq!(pending, vec![("Other.java", 4), ("Use.java", 4)], "{:?}", r.pending);
    assert!(r.pending.iter().all(|f| f.maturity == grounds_rules::Maturity::Stable));
    assert_eq!(r.breaks.len(), 1);
    let v: serde_json::Value = serde_json::from_str(&json(&r)).unwrap();
    assert_eq!(v["pending"][0]["maturity"], "stable");
    assert_eq!(v["breaks"][0]["finding"]["maturity"], "stable");
    let sarif: serde_json::Value = serde_json::from_str(&grounds_engine::report::sarif_report(&r, &dir, &[])).unwrap();
    for x in sarif["runs"][0]["results"].as_array().unwrap() {
        assert_eq!(x["properties"]["maturity"], "stable", "{x}");
    }
    for d in sarif["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap() {
        assert_eq!(d["properties"]["maturity"], "stable", "{d}");
    }
    assert_eq!(json(&Session::open(&dir).unwrap().check(&opts)), json(&r));
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn only_the_promised_rules_are_stable_in_pending_and_sarif_rules_take_the_weaker_maturity() {
    let (repo, dir) = gap_project("static");
    write(&dir, "src/main/java/demo/Util.java", "package demo;\npublic class Util {\n    public static void helper() {}\n}\n");
    write(&dir, "src/main/java/demo/UsesUtil.java", "package demo;\nimport static demo.Util.helper;\nclass UsesUtil {\n    void t() { helper(); }\n}\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "util"]);
    let mut s = Session::open(&dir).unwrap();
    // a renamed statically imported member: `unresolved-static-import` is not
    // one of the promised rules, so it stays preview in `pending`
    write(&dir, "src/main/java/demo/Util.java", "package demo;\npublic class Util {\n    public static void helper2() {}\n}\n");
    let r = s.check(&CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base: None });
    assert!(r.pending.iter().any(|f| f.rule == "unresolved-static-import"), "{:?}", r.pending);
    for f in &r.pending {
        let want = if grounds_engine::session::stable_in_changed(f.rule) { grounds_rules::Maturity::Stable } else { grounds_rules::Maturity::Preview };
        assert_eq!(f.maturity, want, "{f:?}");
    }
    // a break on a changed line is as mature as the weakest place under it
    assert!(!r.breaks.is_empty());
    for b in &r.breaks {
        let all_stable = b.broken.iter().all(|&i| r.pending[i].maturity == grounds_rules::Maturity::Stable);
        assert_eq!(b.finding.maturity == grounds_rules::Maturity::Stable, all_stable, "{b:?}");
    }

    // `--all-lines`: the pre-existing gap on an unchanged line (preview) and a
    // new break on a changed line (stable) of the same rule
    write(&dir, "src/main/java/demo/Util.java", "package demo;\npublic class Util {\n    public static void helper() {}\n}\n");
    write(&dir, "src/main/java/demo/Use.java", "package demo;\nclass Use {\n    void a(Lib l) { l.gap(); }\n    void b(Lib l) { l.missing(); }\n}\n");
    let r = s.check(&CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None });
    let m: Vec<_> = r.findings.iter().filter(|f| f.rule == "unresolved-method").map(|f| (f.line, f.maturity)).collect();
    assert!(m.contains(&(3, grounds_rules::Maturity::Preview)) && m.contains(&(4, grounds_rules::Maturity::Stable)), "{m:?}");
    let sarif: serde_json::Value = serde_json::from_str(&grounds_engine::report::sarif_report(&r, &dir, &[])).unwrap();
    let d = sarif["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap().iter().find(|d| d["id"] == "unresolved-method").unwrap().clone();
    assert_eq!(d["properties"]["maturity"], "preview", "{d}");
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn a_file_written_while_the_project_loads_is_read_again() {
    let dir = std::env::temp_dir().join(format!("grounds-session-loadrace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/demo/Api.java", "package demo;\npublic class Api {\n    public void old() {}\n}\n");
    write(&dir, "src/main/java/demo/Caller.java", "package demo;\npublic class Caller {\n    void m(Api a) { a.old(); }\n}\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);
    let classes = |index: &grounds_index::Index| {
        let mut names: Vec<_> = index.source_class_names().cloned().collect();
        names.sort();
        names.iter().map(|n| format!("{:?}", index.class(n))).collect::<Vec<_>>()
    };
    // a daemon starting: the contents are read ...
    let project = grounds_engine::Project::load(&dir).unwrap();
    // ... an edit lands ...
    write(&dir, "src/main/java/demo/Api.java", "package demo;\npublic class Api {\n    public void renamed() {}\n}\n");
    // ... and only then does the session note the files' times
    let mut s = Session::from_project(project);
    s.sync(&[]);
    let fresh = grounds_engine::Project::load(&dir).unwrap();
    assert_eq!(classes(&s.project.index), classes(&fresh.index), "the edit made during the load must not be lost");
    // and what it broke is reported, as a run from scratch reports it
    let opts = CheckOptions { mode: Mode::Edit, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base: None };
    let r = s.check(&opts);
    assert_eq!(r.pending.len(), 1, "the caller of the renamed method: {:?}", r.pending);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Writes `content` and puts the file's time back to `time`: a write in the
/// same tick of a file system that keeps whole seconds.
fn write_at(dir: &std::path::Path, file: &str, content: &str, time: std::time::SystemTime) {
    write(dir, file, content);
    std::fs::File::options().write(true).open(dir.join(file)).unwrap().set_modified(time).unwrap();
}

#[test]
fn a_second_write_at_the_same_time_is_read() {
    let dir = std::env::temp_dir().join(format!("grounds-session-sametime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    // the build file is old, so nothing here loads the project again
    write_at(&dir, "build.gradle", "plugins { id 'java' }\n", long_ago);
    write_at(&dir, "src/main/java/demo/Api.java", "package demo;\npublic class Api {}\n", long_ago);
    let methods = |s: &Session| {
        let api = s.project.index.source_class_names().find(|n| n.ends_with("Api")).cloned().unwrap();
        format!("{:?}", s.project.index.class(&api))
    };
    let mut s = Session::open(&dir).unwrap();
    let loaded = s.project.load_began;
    // (the time is taken after the load, however long that took: the writes below are recent at each sync)
    let now = std::time::SystemTime::now();
    write_at(&dir, "src/main/java/demo/Api.java", "package demo;\npublic class Api {\n    public void first() {}\n}\n", now);
    s.sync(&[]);
    assert!(methods(&s).contains("first"));
    // read, and then written again within the same tick: the time says nothing
    write_at(&dir, "src/main/java/demo/Api.java", "package demo;\npublic class Api {\n    public void second() {}\n}\n", now);
    s.sync(&[]);
    assert!(methods(&s).contains("second"), "{}", methods(&s));
    write_at(&dir, "src/main/java/demo/Api.java", "package demo;\npublic class Api {\n    public void third() {}\n}\n", now);
    s.sync(&[]);
    assert!(methods(&s).contains("third"), "{}", methods(&s));
    assert_eq!(s.project.load_began, loaded, "source files alone never load the project again");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_configuration_file_written_twice_at_the_same_time_is_read() {
    let dir = std::env::temp_dir().join(format!("grounds-session-sametime-config-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    write_at(&dir, "build.gradle", "plugins { id 'java' }\n", long_ago);
    write_at(&dir, "src/main/java/demo/Api.java", "package demo;\npublic class Api {}\n", long_ago);
    let mut s = Session::open(&dir).unwrap();
    let now = std::time::SystemTime::now();
    write_at(&dir, "grounds.toml", "exclude = [\"a/**\"]\n", now);
    s.sync(&[]);
    assert_eq!(s.project.config.exclude, ["a/**"]);
    // written again in the tick it was read in
    write_at(&dir, "grounds.toml", "exclude = [\"b/**\"]\n", now);
    std::thread::sleep(std::time::Duration::from_millis(2100));
    // the tick is over: one more load is right, whatever was written in it
    s.sync(&[]);
    assert_eq!(s.project.config.exclude, ["b/**"]);
    // and that load is the last
    let loaded = s.project.load_began;
    s.sync(&[]);
    assert_eq!(s.project.load_began, loaded);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A launcher whose body starts `Solo` or `Pair` (names of one length: the
/// edit moves nothing); `Pair` imports two
/// configurations with a bean of the same name.
fn launcher_project(dir: &Path, started: &str) {
    write(dir, "build.gradle", "plugins { id 'java' }\n");
    write(dir, "src/main/java/app/Solo.java", "package app;\n@org.springframework.context.annotation.Configuration\npublic class Solo {}\n");
    write(dir, "src/main/java/app/Pair.java", "package app;\n@org.springframework.context.annotation.Configuration\n@org.springframework.context.annotation.Import({B.class, C.class})\npublic class Pair {}\n");
    write(dir, "src/main/java/app/B.java", "package app;\n@org.springframework.context.annotation.Configuration\npublic class B {\n    @org.springframework.context.annotation.Bean\n    public Object job() { return null; }\n}\n");
    write(dir, "src/main/java/app/C.java", "package app;\n@org.springframework.context.annotation.Configuration\npublic class C {\n    @org.springframework.context.annotation.Bean\n    public Object job() { return null; }\n}\n");
    write(dir, "src/main/java/launch/Main.java", &launcher(started));
    // written long ago: a session does not read them again as just written
    let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    for f in ["build.gradle", "src/main/java/app/Solo.java", "src/main/java/app/Pair.java", "src/main/java/app/B.java", "src/main/java/app/C.java", "src/main/java/launch/Main.java"] {
        std::fs::File::options().write(true).open(dir.join(f)).unwrap().set_modified(past).unwrap();
    }
}

fn launcher(started: &str) -> String {
    format!("package launch;\nimport app.*;\nimport org.springframework.boot.SpringApplication;\npublic class Main {{\n    public static void main(String[] args) {{\n        SpringApplication.run({started}.class, args);\n    }}\n}}\n")
}

#[test]
fn a_body_only_edit_of_a_launcher_rebuilds_the_contexts() {
    let dir = std::env::temp_dir().join(format!("grounds-session-launcher-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    launcher_project(&dir, "Solo");
    let mut s = Session::open(&dir).unwrap();
    assert!(s.project.di.name_conflicts().is_empty());
    // a body-only edit: no declaration changes, no bean in the file
    write(&dir, "src/main/java/launch/Main.java", &launcher("Pair"));
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options().write(true).open(dir.join("src/main/java/launch/Main.java")).unwrap().set_modified(later).unwrap();
    s.sync(&[]);
    let fresh = grounds_engine::Project::load(&dir).unwrap();
    assert_eq!(fresh.di.name_conflicts().len(), 1);
    assert_eq!(s.project.di.name_conflicts().len(), 1, "the warm session must see the launcher's new start as a fresh load does");
    let _ = std::fs::remove_dir_all(&dir);
}

fn set_mtime(dir: &Path, rel: &str, t: std::time::SystemTime) {
    std::fs::File::options().write(true).open(dir.join(rel)).unwrap().set_modified(t).unwrap();
}

fn mtime(dir: &Path, rel: &str) -> std::time::SystemTime {
    std::fs::metadata(dir.join(rel)).unwrap().modified().unwrap()
}

/// A project with one class committed, for the tests of change detection
/// below: `changed_apis` of a check against HEAD says whether the session
/// saw the file's declarations change.
fn one_class_project(name: &str, api: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("grounds-session-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/demo/Api.java", api);
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);
    dir
}

fn changed_apis(s: &mut Session, dir: &Path) -> Vec<String> {
    let api = dir.join("src/main/java/demo/Api.java").canonicalize().unwrap();
    let opts = CheckOptions { mode: Mode::Checkpoint, files: vec![api], all_lines: false, use_baseline: false, min_severity: Severity::Info, base: None };
    s.check(&opts).changed_apis
}

#[test]
fn a_file_written_while_the_project_loads_keeps_no_time_of_the_new_version() {
    // The replay's first step: a daemon started at a commit loads the project
    // in the background while the next commit is checked out. It read the
    // file before the checkout wrote it, and took the file's time after: the
    // new version's time with the old version's stub, so no later look found
    // the file changed.
    let rel = "src/main/java/demo/Api.java";
    let dir = one_class_project("load-race", "package demo;\npublic class Api {\n    public void old() {}\n}\n");
    let now = std::time::SystemTime::now();
    // times too old to be racy: this is about the order of time and read
    set_mtime(&dir, rel, now - std::time::Duration::from_secs(7200));
    let project = grounds_engine::Project::load(&dir).unwrap();
    // the checkout, between the read of the sources and the end of the load
    write(&dir, rel, "package demo;\npublic class Api {\n    public void renamed() {}\n}\n");
    set_mtime(&dir, rel, now - std::time::Duration::from_secs(3600));
    let mut s = Session::from_project(project);
    assert_eq!(changed_apis(&mut s, &dir), [rel], "the session holds the stub of the version it read");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_rewrite_that_keeps_time_and_size_is_seen() {
    // A file system keeps times only as fine as its clock ticks: a file
    // rewritten in the tick it was read in keeps its time, and an edit of a
    // string can keep its size. Its time was recent when it was read, so the
    // content decides. (Each rewrite follows the look before it by
    // milliseconds: a time seen unchanged for two seconds is trusted, as no
    // tick is that long.)
    let rel = "src/main/java/demo/Api.java";
    let v1 = "package demo;\npublic class Api {\n    @Deprecated(since = \"1\") public void m() {}\n}\n";
    let v2 = "package demo;\npublic class Api {\n    @Deprecated(since = \"2\") public void m() {}\n}\n";
    let dir = one_class_project("racy", v1);
    set_mtime(&dir, rel, std::time::SystemTime::now() - std::time::Duration::from_secs(3600));
    let mut s = Session::open(&dir).unwrap();
    let t = std::time::SystemTime::now();
    write(&dir, rel, v2);
    set_mtime(&dir, rel, t);
    assert_eq!(changed_apis(&mut s, &dir), [rel], "a new time");
    write(&dir, rel, v1);
    set_mtime(&dir, rel, t);
    assert!(changed_apis(&mut s, &dir).is_empty(), "same time and size, content back to that of HEAD");
    write(&dir, rel, v2);
    set_mtime(&dir, rel, t);
    assert_eq!(changed_apis(&mut s, &dir), [rel], "same time and size, other content again");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_comparison_with_a_base_sees_the_base_s_launcher() {
    let repo = std::env::temp_dir().join(format!("grounds-session-launcher-base-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    launcher_project(&repo, "Solo");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    git(&repo, &["checkout", "-qb", "feature"]);
    write(&repo, "src/main/java/launch/Main.java", &launcher("Pair"));
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "start Pair"]);
    let mut s = Session::open(&repo).unwrap();
    let mb = grounds_engine::git::merge_base(&repo, "main").unwrap();
    let opts = CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base: Some(mb) };
    let r = s.check(&opts);
    let conflicts: Vec<&str> = r.pending.iter().chain(r.findings.iter()).filter(|f| f.rule == "bean-name-conflict").map(|f| f.path.as_str()).collect();
    assert!(!conflicts.is_empty(), "the clash the change introduces is new: {:?} / {:?}", r.pending, r.findings);
    drop(s);
    // against a base that already starts Pair, an edit of the launcher that
    // still starts it is not a new clash: the base state (`at_base`) must
    // read the base's launcher, not take its startups from the head
    write(&repo, "src/main/java/launch/Main.java", &launcher("Pair").replace("public class Main {", "public class Main {\n    // still Pair"));
    let r = base_check(&repo);
    assert!(!rules_of(&r).iter().any(|(_, rule)| *rule == "bean-name-conflict"), "{:?}", r.pending);
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn a_build_file_written_while_the_project_loads_reloads_it() {
    // as above, for the files the project's layout and settings come from
    let dir = std::env::temp_dir().join(format!("grounds-session-load-race-build-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "grounds.toml", "exclude = []\n");
    write(&dir, "src/main/java/demo/App.java", "package demo;\npublic class App {}\n");
    write(&dir, "src/main/java/gen/Gen.java", "package gen;\npublic class Gen {}\n");
    let project = grounds_engine::Project::load(&dir).unwrap();
    write(&dir, "grounds.toml", "exclude = [\"src/main/java/gen/**\"]\n");
    let mut s = Session::from_project(project);
    assert!(s.project.config.exclude.is_empty());
    s.sync(&[]);
    assert_eq!(s.project.config.exclude, ["src/main/java/gen/**"], "the settings written during the load are read");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn times_in_the_future_are_trusted_once_seen_for_the_margin() {
    // A time in the future is always "recent" by the clock; seen unchanged for
    // two seconds, it cannot be shared by a later write any more. Neither the
    // source file nor the build file may be compared, or the project loaded,
    // without end.
    let dir = std::env::temp_dir().join(format!("grounds-session-future-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let future = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
    write_at(&dir, "build.gradle", "plugins { id 'java' }\n", future);
    write_at(&dir, "src/main/java/demo/Api.java", "package demo;\npublic class Api {}\n", future);
    let mut s = Session::open(&dir).unwrap();
    assert_eq!(s.in_doubt(), (2, true), "both compared by content, and which build file the load read is unknown");
    s.sync(&[]);
    let loaded = s.project.load_began;
    assert_eq!(s.in_doubt(), (2, false), "loaded once more, the build file's content now known");
    s.sync(&[]);
    assert_eq!(s.project.load_began, loaded, "and not again");
    std::thread::sleep(std::time::Duration::from_millis(2100));
    s.sync(&[]);
    assert_eq!(s.in_doubt(), (0, false), "seen unchanged for the margin: times alone decide");
    assert_eq!(s.project.load_began, loaded);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_that_could_not_be_read_is_read_once_it_can_be() {
    // A source file that cannot be read (no permission, not UTF-8, unlinked
    // between the look at its time and the read) is not taken as read: when
    // it can be read again with the same time and size, it is read, and the
    // session holds what a fresh load holds.
    let rel = "src/main/java/demo/Api.java";
    let good = "package demo;\npublic class Api {\n    public void m() {}\n}\n";
    let edit = "package demo;\npublic class Api {\n    public void n() {}\n}\n";
    let dir = one_class_project("unreadable", good);
    let t = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    set_mtime(&dir, rel, t);
    // (an old build file: a reload would read everything again and hide what sync does)
    set_mtime(&dir, "build.gradle", t);
    let mut s = Session::open(&dir).unwrap();
    let api = |s: &Session| s.project.index.source_class_names().find(|n| n.ends_with("Api")).map(|n| format!("{:?}", s.project.index.class(n)));
    let fresh = |dir: &Path| { let s = Session::open(dir).unwrap(); api(&s) };
    // the edit lands, but cannot be read: without permission where that
    // stops us, else as bytes that are not UTF-8 (of the same size)
    let u = std::time::SystemTime::now() - std::time::Duration::from_secs(1800);
    write(&dir, rel, edit);
    set_mtime(&dir, rel, u);
    let path = dir.join(rel);
    use std::os::unix::fs::PermissionsExt;
    // (the time is set first: a file without permission cannot be opened to set it)
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let by_permission = std::fs::read(&path).is_err();
    if !by_permission {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut bytes = edit.as_bytes().to_vec();
        bytes[0] = 0xff;
        std::fs::write(&path, bytes).unwrap();
        set_mtime(&dir, rel, u);
    }
    s.sync(&[]);
    assert_eq!(api(&s), fresh(&dir), "an unreadable file: as a fresh load, no stub");
    // readable again, with the same time and size
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    write(&dir, rel, edit);
    set_mtime(&dir, rel, u);
    s.sync(&[]);
    assert!(api(&s).is_some_and(|c| c.contains("\"n\"") || c.contains(" n")), "{:?}", api(&s));
    assert_eq!(api(&s), fresh(&dir), "the file once it can be read again (by {})", if by_permission { "permission" } else { "UTF-8" });

    // the same for a file that could not be read when the project was loaded
    let mut bytes = good.as_bytes().to_vec();
    bytes[0] = 0xff;
    std::fs::write(&path, bytes).unwrap();
    set_mtime(&dir, rel, t);
    let mut s = Session::open(&dir).unwrap();
    assert_eq!(api(&s), None);
    write(&dir, rel, good);
    set_mtime(&dir, rel, t);
    s.sync(&[]);
    assert!(api(&s).is_some(), "read once it can be");
    assert_eq!(api(&s), fresh(&dir));
    let _ = std::fs::remove_dir_all(&dir);
}

/// What a session holds of the sources, to compare with a fresh load: the
/// files with their module and kind, the files in the index, and every class
/// as it resolves.
fn holdings(s: &Session) -> (Vec<String>, Vec<String>, Vec<String>) {
    let ws = &s.project.workspace;
    let mut files: Vec<String> = ws.files.iter().map(|f| format!("{} {} {:?}", f.path.strip_prefix(&ws.root).unwrap_or(&f.path).display(), ws.modules[f.module].name, f.kind)).collect();
    files.sort();
    let index = &s.project.index;
    let mut indexed: Vec<String> = index.files().map(|(p, _)| p.to_string()).collect();
    indexed.sort();
    let mut names: Vec<_> = index.source_class_names().cloned().collect();
    names.sort();
    (files, indexed, names.iter().map(|n| format!("{:?}", index.class(n))).collect())
}

/// After `what`, the session holds what a fresh one holds.
fn holds_as_fresh(s: &mut Session, dir: &Path, what: &str) {
    s.sync(&[]);
    let fresh = Session::open(dir).unwrap();
    assert_eq!(holdings(s), holdings(&fresh), "after {what}, the session must hold the sources a fresh load finds");
}

/// Every file and directory under `rel` gets time `t`, as a restore that
/// keeps times (or a clock that is behind) leaves them.
fn set_tree_mtime(dir: &Path, rel: &str, t: std::time::SystemTime) {
    let p = dir.join(rel);
    if p.is_dir() {
        // (not git's own files: its objects are read-only)
        for e in std::fs::read_dir(&p).unwrap().flatten().filter(|e| e.file_name() != ".git") {
            set_tree_mtime(dir, &format!("{rel}/{}", e.file_name().to_string_lossy()), t);
        }
        std::fs::File::open(&p).unwrap().set_modified(t).unwrap();
    } else {
        set_mtime(dir, rel, t);
    }
}

/// An interface, its implementation in another file, and a class the
/// implementation's signature names; all committed, with old times.
fn restored_project(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("grounds-session-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/demo/Req.java", "package demo;\npublic class Req {}\n");
    write(&dir, "src/main/java/demo/Biz.java", "package demo;\npublic interface Biz {\n    void handle(Req r);\n}\n");
    write(&dir, "src/main/java/demo/BizImpl.java", "package demo;\npublic class BizImpl implements Biz {\n    public void handle(Req r) {}\n}\n");
    write(&dir, "src/main/java/demo/sub/Sub.java", "package demo.sub;\npublic class Sub {}\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);
    // (old: a reload or a recent time would read everything again and hide what sync does)
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    set_tree_mtime(&dir, ".", old);
    // (git's exclude file too: just written, it would have the first sync walk
    // the whole workspace, and the tests pass without listing a directory)
    set_mtime(&dir, ".git/info/exclude", old);
    dir
}

/// The session found what changed by listing the directories that changed,
/// not by walking the whole workspace (which an ignore file's change does).
fn listed_not_walked(s: &Session) {
    assert_eq!(s.full_walks(), 0, "found by the directories that changed, not by a walk of the workspace");
}

#[test]
fn a_deleted_file_that_comes_back_is_read_again() {
    // The hold-out measurement's case: an edit deletes a file, `git reset`
    // brings it back. Equal to HEAD again, it is in no list of changed files,
    // and its time may be any (here an old one): the session must find it
    // all the same, or the classes that name it stay unknown and a break in
    // a caller goes unreported.
    let dir = restored_project("restored");
    let req = "src/main/java/demo/Req.java";
    let text = std::fs::read_to_string(dir.join(req)).unwrap();
    let old = mtime(&dir, req);
    let mut s = Session::open(&dir).unwrap();
    let opts = CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base: None };
    std::fs::remove_file(dir.join(req)).unwrap();
    s.check(&opts);
    holds_as_fresh(&mut s, &dir, "a file was deleted");
    write(&dir, req, &text);
    set_mtime(&dir, req, old);
    holds_as_fresh(&mut s, &dir, "the deleted file came back with its old time");
    listed_not_walked(&s);
    // a method added to the interface, committed, breaks the implementation
    write(&dir, "src/main/java/demo/Biz.java", "package demo;\npublic interface Biz {\n    void handle(Req r);\n    void added();\n}\n");
    git(&dir, &["commit", "-qam", "added"]);
    let opts = CheckOptions { base: Some("HEAD~1".into()), ..opts };
    let warm = s.check(&opts);
    let fresh = Session::open(&dir).unwrap().check(&opts);
    assert!(fresh.pending.iter().any(|f| f.rule == "unimplemented-abstract-method"), "{}", json(&fresh));
    assert_eq!(json(&warm), json(&fresh), "the daemon answers as a fresh run");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_that_appear_in_no_list_of_changed_files_are_found() {
    // A branch switch, a pull, `git stash pop`: files appear (and go) that
    // equal HEAD afterwards, in an existing package and in a new one.
    let dir = restored_project("branch");
    git(&dir, &["checkout", "-qb", "base"]);
    git(&dir, &["checkout", "-qb", "other"]);
    write(&dir, "src/main/java/demo/Added.java", "package demo;\npublic class Added extends Req {}\n");
    write(&dir, "src/main/java/demo/fresh/deep/New.java", "package demo.fresh.deep;\npublic class New {}\n");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "other"]);
    git(&dir, &["checkout", "-q", "base"]);
    let mut s = Session::open(&dir).unwrap();
    git(&dir, &["checkout", "-q", "other"]);
    holds_as_fresh(&mut s, &dir, "a checkout that adds files");
    assert!(s.project.index.class_exists("demo.fresh.deep.New"));
    git(&dir, &["checkout", "-q", "base"]);
    holds_as_fresh(&mut s, &dir, "a checkout that removes them");
    git(&dir, &["checkout", "-q", "other"]);
    git(&dir, &["reset", "-q", "--soft", "base"]);
    git(&dir, &["reset", "-q"]);
    git(&dir, &["stash", "push", "-q", "-u", "--", "src"]);
    holds_as_fresh(&mut s, &dir, "a stash of untracked files");
    git(&dir, &["stash", "pop", "-q"]);
    holds_as_fresh(&mut s, &dir, "a stash pop");
    // a generator writes a tree of sources, with old times
    write(&dir, "build/generated/sources/annotationProcessor/java/main/demo/gen/Gen.java", "package demo.gen;\npublic class Gen {}\n");
    set_tree_mtime(&dir, "build", std::time::SystemTime::now() - std::time::Duration::from_secs(600));
    holds_as_fresh(&mut s, &dir, "a generator wrote sources");
    listed_not_walked(&s);
    assert!(s.project.index.class_exists("demo.gen.Gen"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_directory_that_goes_and_comes_back_is_read_again() {
    let dir = restored_project("dirback");
    let mut s = Session::open(&dir).unwrap();
    let old = mtime(&dir, "src/main/java/demo/sub/Sub.java");
    std::fs::remove_dir_all(dir.join("src/main/java/demo")).unwrap();
    holds_as_fresh(&mut s, &dir, "a package directory was removed");
    git(&dir, &["checkout", "-q", "--", "."]);
    set_tree_mtime(&dir, "src/main/java/demo", old);
    holds_as_fresh(&mut s, &dir, "it came back, with old times");
    listed_not_walked(&s);
    assert!(s.project.index.class_exists("demo.sub.Sub"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_replaced_by_a_directory_and_back() {
    let dir = restored_project("filedir");
    let mut s = Session::open(&dir).unwrap();
    let req = "src/main/java/demo/Req.java";
    let old = mtime(&dir, req);
    std::fs::remove_file(dir.join(req)).unwrap();
    write(&dir, "src/main/java/demo/Req.java/Inner.java", "package demo;\npublic class Inner {}\n");
    holds_as_fresh(&mut s, &dir, "a file was replaced by a directory");
    assert!(s.unread_files().is_empty(), "a directory is no file to read again: {:?}", s.unread_files());
    std::fs::remove_dir_all(dir.join(req)).unwrap();
    git(&dir, &["checkout", "-q", "--", req]);
    set_mtime(&dir, req, old);
    holds_as_fresh(&mut s, &dir, "the directory was replaced by the file");
    listed_not_walked(&s);
    assert!(s.project.index.class_exists("demo.Req"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_module_directory_that_appears_is_a_module() {
    // no settings file names it: its build file alone makes it a module
    let dir = restored_project("newmodule");
    let mut s = Session::open(&dir).unwrap();
    write(&dir, "lib/build.gradle", "plugins { id 'java-library' }\n");
    write(&dir, "lib/src/main/java/lib/Lib.java", "package lib;\npublic class Lib {}\n");
    write(&dir, "lib/src/test/java/lib/LibTest.java", "package lib;\nclass LibTest {}\n");
    set_tree_mtime(&dir, "lib", std::time::SystemTime::now() - std::time::Duration::from_secs(600));
    holds_as_fresh(&mut s, &dir, "a module directory appeared");
    assert!(s.project.workspace.modules.iter().any(|m| m.name == "lib"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_ignore_file_that_changes_changes_the_sources() {
    let dir = restored_project("ignorefile");
    // (untracked: files git tracks are read whatever .gitignore says)
    git(&dir, &["rm", "-r", "-q", "--cached", "src/main/java/demo/sub"]);
    let mut s = Session::open(&dir).unwrap();
    write(&dir, ".gitignore", "src/main/java/demo/sub/\n");
    holds_as_fresh(&mut s, &dir, "a .gitignore appeared");
    assert!(!s.project.index.class_exists("demo.sub.Sub"));
    // edited in place: the directory's time does not change
    write(&dir, ".gitignore", "*.log\n");
    set_mtime(&dir, ".gitignore", std::time::SystemTime::now() - std::time::Duration::from_secs(60));
    holds_as_fresh(&mut s, &dir, "the .gitignore no longer lists it");
    assert!(s.project.index.class_exists("demo.sub.Sub"));
    write(&dir, ".git/info/exclude", "Req.java\n");
    holds_as_fresh(&mut s, &dir, "git's exclude file lists a file");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_directory_renamed_in_case_only_is_held_once() {
    // On a file system that ignores case (macOS by default) the old spelling
    // still answers a look at its time: the listing of the parent decides.
    let dir = restored_project("caserename");
    let mut s = Session::open(&dir).unwrap();
    std::fs::rename(dir.join("src/main/java/demo"), dir.join("src/main/java/DEMO")).unwrap();
    holds_as_fresh(&mut s, &dir, "a package directory renamed in case only");
    holds_as_fresh(&mut s, &dir, "a second sync");
    listed_not_walked(&s);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_symlink_in_place_of_a_directory_or_a_file_holds_no_sources() {
    // The discovery does not follow symlinks; a file's stamp does, so the
    // target's unchanged time must not keep what the link replaced.
    let dir = restored_project("symlinks");
    let away = dir.with_extension("away");
    let _ = std::fs::remove_dir_all(&away);
    std::fs::create_dir_all(&away).unwrap();
    let mut s = Session::open(&dir).unwrap();
    std::fs::rename(dir.join("src/main/java/demo/sub"), away.join("sub")).unwrap();
    std::os::unix::fs::symlink(away.join("sub"), dir.join("src/main/java/demo/sub")).unwrap();
    holds_as_fresh(&mut s, &dir, "a directory replaced by a symlink to it");
    holds_as_fresh(&mut s, &dir, "a second sync");
    std::fs::rename(dir.join("src/main/java/demo/Req.java"), away.join("Req.java")).unwrap();
    std::os::unix::fs::symlink(away.join("Req.java"), dir.join("src/main/java/demo/Req.java")).unwrap();
    holds_as_fresh(&mut s, &dir, "a file replaced by a symlink to it");
    holds_as_fresh(&mut s, &dir, "a second sync");
    listed_not_walked(&s);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&away);
}

#[cfg(unix)]
#[test]
fn a_directory_that_cannot_be_listed_is_listed_once_it_can_be() {
    // (as root a directory without permissions can still be listed: then
    // this only checks that the session holds what a fresh load holds)
    use std::os::unix::fs::PermissionsExt;
    let dir = restored_project("unlistable");
    let mut s = Session::open(&dir).unwrap();
    let sub = dir.join("src/main/java/demo/sub");
    write(&dir, "src/main/java/demo/sub/B.java", "package demo.sub;\npublic class B {}\n");
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
    s.sync(&[]);
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    holds_as_fresh(&mut s, &dir, "the directory could be listed again");
    holds_as_fresh(&mut s, &dir, "a second sync");
    assert!(s.project.index.class_exists("demo.sub.B"));
    listed_not_walked(&s);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_exclude_file_of_a_linked_worktree_is_followed() {
    // In a worktree `.git` is a file; `ignore` reads the main repository's
    // `info/exclude` (its common directory's)
    let main = restored_project("worktree");
    let dir = main.with_extension("wt");
    let _ = std::fs::remove_dir_all(&dir);
    git(&main, &["worktree", "add", "-q", "--detach", dir.to_str().unwrap()]);
    // (untracked: files git tracks are read whatever the exclude file says)
    git(&dir, &["rm", "-q", "--cached", "src/main/java/demo/Req.java"]);
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    set_tree_mtime(&dir, ".", old);
    let mut s = Session::open(&dir).unwrap();
    assert!(s.project.index.class_exists("demo.Req"));
    write(&main, ".git/info/exclude", "Req.java\n");
    set_mtime(&main, ".git/info/exclude", old - std::time::Duration::from_secs(60));
    holds_as_fresh(&mut s, &dir, "the main repository's exclude file lists a file");
    assert!(!s.project.index.class_exists("demo.Req"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&main);
}

#[test]
fn an_ignore_file_that_removes_a_module_reloads_the_project() {
    let dir = restored_project("ignoremodule");
    write(&dir, "lib/build.gradle", "plugins { id 'java-library' }\n");
    write(&dir, "lib/src/main/java/lib/L.java", "package lib;\npublic class L {}\n");
    set_tree_mtime(&dir, "lib", std::time::SystemTime::now() - std::time::Duration::from_secs(3600));
    let mut s = Session::open(&dir).unwrap();
    assert!(s.project.workspace.modules.iter().any(|m| m.name == "lib"));
    write(&dir, ".ignore", "lib/build.gradle\n");
    holds_as_fresh(&mut s, &dir, "an ignore file hides a module's build file");
    assert!(!s.project.workspace.modules.iter().any(|m| m.name == "lib"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_ignore_file_with_a_time_in_the_future_is_trusted_once_seen_for_the_margin() {
    // A time in the future is "recent" by every clock: seen unchanged for the
    // margin it is trusted, or every request walks the whole workspace.
    let dir = restored_project("futureignore");
    write(&dir, ".ignore", "*.log\n");
    set_mtime(&dir, ".ignore", std::time::SystemTime::now() + std::time::Duration::from_secs(3600));
    let mut s = Session::open(&dir).unwrap();
    holds_as_fresh(&mut s, &dir, "an ignore file with a time in the future");
    std::thread::sleep(std::time::Duration::from_millis(2100));
    s.sync(&[]);
    let walks = s.full_walks();
    for _ in 0..3 {
        s.sync(&[]);
    }
    assert_eq!(s.full_walks(), walks, "nothing changed: no walk of the workspace");
    holds_as_fresh(&mut s, &dir, "the syncs since");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_directory_that_can_be_read_but_not_entered_is_listed_once_it_can_be() {
    // Without execute permission a directory lists its names, but its files
    // cannot be reached: it is not taken as listed until they can be.
    // (as root the permission is not enforced: then this only checks that
    // the session holds what a fresh load holds)
    use std::os::unix::fs::PermissionsExt;
    let dir = restored_project("unenterable");
    let mut s = Session::open(&dir).unwrap();
    let sub = dir.join("src/main/java/demo/sub");
    write(&dir, "src/main/java/demo/sub/B.java", "package demo.sub;\npublic class B {}\n");
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o400)).unwrap();
    s.sync(&[]);
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    holds_as_fresh(&mut s, &dir, "the directory could be entered again");
    holds_as_fresh(&mut s, &dir, "a second sync");
    assert!(s.project.index.class_exists("demo.sub.Sub") && s.project.index.class_exists("demo.sub.B"));
    listed_not_walked(&s);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn an_ignore_file_that_cannot_be_read_is_read_once_it_can_be() {
    // (as root the permission is not enforced: then this only checks that
    // the session holds what a fresh load holds)
    use std::os::unix::fs::PermissionsExt;
    let dir = restored_project("unreadableignore");
    let mut s = Session::open(&dir).unwrap();
    write(&dir, ".ignore", "Req.java\n");
    std::fs::set_permissions(dir.join(".ignore"), std::fs::Permissions::from_mode(0o000)).unwrap();
    s.sync(&[]);
    s.sync(&[]);
    std::fs::set_permissions(dir.join(".ignore"), std::fs::Permissions::from_mode(0o644)).unwrap();
    holds_as_fresh(&mut s, &dir, "the ignore file could be read again");
    std::thread::sleep(std::time::Duration::from_millis(2100));
    holds_as_fresh(&mut s, &dir, "past the margin");
    assert!(!s.project.index.class_exists("demo.Req"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_exclude_file_of_a_nested_repository_is_followed() {
    let dir = restored_project("nestedrepo");
    let mut s = Session::open(&dir).unwrap();
    let sub = dir.join("src/main/java/demo/sub");
    git(&sub, &["init", "-q"]);
    holds_as_fresh(&mut s, &dir, "a repository started in a package directory");
    write(&sub, ".git/info/exclude", "Sub.java\n");
    set_mtime(&sub, ".git/info/exclude", std::time::SystemTime::now() - std::time::Duration::from_secs(60));
    holds_as_fresh(&mut s, &dir, "its exclude file lists a file");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_repository_that_starts_or_ends_around_the_workspace_changes_what_is_ignored() {
    // a `.gitignore` only counts inside a repository: `git init` on a
    // workspace that had none, and the reverse, change the sources
    let dir = std::env::temp_dir().join(format!("grounds-session-gitinit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, ".gitignore", "Req.java\n");
    write(&dir, "src/main/java/demo/Req.java", "package demo;\npublic class Req {}\n");
    write(&dir, "src/main/java/demo/Biz.java", "package demo;\npublic class Biz {}\n");
    set_tree_mtime(&dir, ".", std::time::SystemTime::now() - std::time::Duration::from_secs(3600));
    let mut s = Session::open(&dir).unwrap();
    assert!(s.project.index.class_exists("demo.Req"));
    git(&dir, &["init", "-q"]);
    holds_as_fresh(&mut s, &dir, "git init");
    assert!(!s.project.index.class_exists("demo.Req"));
    std::fs::remove_dir_all(dir.join(".git")).unwrap();
    holds_as_fresh(&mut s, &dir, "the repository removed");
    assert!(s.project.index.class_exists("demo.Req"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_jj_repository_counts_as_a_repository() {
    // `ignore` applies `.gitignore` inside a `.jj` repository as inside a git one
    let dir = std::env::temp_dir().join(format!("grounds-session-jj-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, ".gitignore", "Req.java\n");
    write(&dir, "src/main/java/demo/Req.java", "package demo;\npublic class Req {}\n");
    write(&dir, "src/main/java/demo/Biz.java", "package demo;\npublic class Biz {}\n");
    set_tree_mtime(&dir, ".", std::time::SystemTime::now() - std::time::Duration::from_secs(3600));
    let mut s = Session::open(&dir).unwrap();
    std::fs::create_dir(dir.join(".jj")).unwrap();
    holds_as_fresh(&mut s, &dir, "a .jj directory made");
    std::thread::sleep(std::time::Duration::from_millis(2100));
    holds_as_fresh(&mut s, &dir, "past the margin");
    assert!(!s.project.index.class_exists("demo.Req"));
    std::fs::remove_dir(dir.join(".jj")).unwrap();
    holds_as_fresh(&mut s, &dir, "the .jj directory removed");
    assert!(s.project.index.class_exists("demo.Req"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_worktree_whose_git_file_is_rewritten_follows_the_new_exclude_file() {
    // The main repository moves: the worktree's `.git` file names a gitdir
    // that is gone, so no exclude file applies; `git worktree repair` writes
    // the new one, still a file, and the main repository's exclude applies again.
    let main = restored_project("wtrepair");
    write(&main, ".git/info/exclude", "Req.java\n");
    let dir = main.with_extension("wt");
    let _ = std::fs::remove_dir_all(&dir);
    git(&main, &["worktree", "add", "-q", "--detach", dir.to_str().unwrap()]);
    // (untracked: files git tracks are read whatever the exclude file says)
    git(&dir, &["rm", "-q", "--cached", "src/main/java/demo/Req.java"]);
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    set_tree_mtime(&dir, ".", old);
    set_mtime(&main, ".git/info/exclude", old);
    let mut s = Session::open(&dir).unwrap();
    assert!(!s.project.index.class_exists("demo.Req"));
    let moved = main.with_extension("moved");
    let _ = std::fs::remove_dir_all(&moved);
    std::fs::rename(&main, &moved).unwrap();
    holds_as_fresh(&mut s, &dir, "the main repository moved");
    git(&moved, &["worktree", "repair", dir.to_str().unwrap()]);
    holds_as_fresh(&mut s, &dir, "the worktree repaired");
    std::thread::sleep(std::time::Duration::from_millis(2100));
    holds_as_fresh(&mut s, &dir, "past the margin");
    assert!(!s.project.index.class_exists("demo.Req"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&moved);
}

#[test]
fn a_repository_that_starts_or_ends_while_the_walk_runs_leaves_a_walk_to_do() {
    // The walk filtered by where repositories started before it; if that
    // differs by the time it is stamped, it is done again at the next sync.
    use grounds_engine::project::Walk;
    let dir = restored_project("walkrace");
    let root = dir.canonicalize().unwrap();
    let began = std::time::SystemTime::now();
    let before: rustc_hash::FxHashMap<std::path::PathBuf, Option<u8>> = [(root.clone(), Some(1))].into_iter().collect();
    let none = rustc_hash::FxHashMap::default();
    assert!(!Walk::stamp(std::slice::from_ref(&root), Some(&root), began, &none, &before).pending, "the same repository: done");
    std::fs::remove_dir_all(root.join(".git")).unwrap();
    assert!(Walk::stamp(std::slice::from_ref(&root), Some(&root), began, &none, &before).pending, "the repository ended during the walk");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same place, a different claim: a changed return type turns "`Old` has
/// no method `missing`" (there before the change) into "`New` has no method
/// `missing`" on an unchanged line of another file. That is a new break.
#[test]
fn a_different_claim_at_the_same_place_is_a_new_break() {
    let repo = std::env::temp_dir().join(format!("grounds-sameplace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    let dir = repo.join("app");
    write(&dir, "settings.gradle", "rootProject.name = 'demo'\n");
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/demo/Old.java", "package demo;\npublic class Old {\n    public void present() {}\n}\n");
    write(&dir, "src/main/java/demo/New.java", "package demo;\npublic class New {\n    public void present() {}\n}\n");
    write(&dir, "src/main/java/demo/Lib.java", "package demo;\npublic class Lib {\n    public Old api() { return null; }\n    public void helper(int a) {}\n}\n");
    write(&dir, "src/main/java/demo/Caller.java", "package demo;\nclass Caller {\n    void t(Lib l) { l.api().missing(); l.helper(1); }\n}\n");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    let mut s = Session::open(&dir).unwrap();
    // `helper` widened (so Caller is a dependent per turn), `api` now returns `New`
    write(&dir, "src/main/java/demo/Lib.java", "package demo;\npublic class Lib {\n    public New api() { return null; }\n    public void helper(long a) {}\n}\n");
    let opts = |base: Option<String>| CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base };
    let msgs = |r: &grounds_engine::session::Report| r.pending.iter().map(|f| f.message.clone()).collect::<Vec<_>>();
    let r = s.check(&opts(None));
    assert_eq!(msgs(&r), ["`New` has no method `missing`"], "per turn: {:?}", r.pending);
    git(&repo, &["checkout", "-qb", "feature"]);
    git(&repo, &["commit", "-qam", "api"]);
    let mb = grounds_engine::git::merge_base(&dir, "main").unwrap();
    let r = s.check(&opts(Some(mb)));
    assert_eq!(msgs(&r), ["`New` has no method `missing`"], "--base: {:?}", r.pending);
    let _ = std::fs::remove_dir_all(&repo);
}

/// Reordering overloads changes only the order of the `available:` list in an
/// old claim, which is not a break; a different receiver type still is.
#[test]
fn reordered_declarations_do_not_make_an_old_claim_new() {
    let repo = std::env::temp_dir().join(format!("grounds-reorder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    let dir = repo.join("app");
    write(&dir, "settings.gradle", "rootProject.name = 'demo'\n");
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/demo/Lib.java", "package demo;\npublic class Lib {\n    public void f(int a) {}\n    public void f(int a, int b) {}\n}\n");
    write(&dir, "src/main/java/demo/Caller.java", "package demo;\nclass Caller {\n    void t(Lib l) { l.f(\"x\"); }\n}\n");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    let mut s = Session::open(&dir).unwrap();
    let before = s.check(&CheckOptions { mode: Mode::Checkpoint, files: vec![dir.join("src/main/java/demo/Caller.java")], all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None });
    assert!(before.findings.iter().any(|f| f.message.contains("available: f(int), f(int,int)")), "{:?}", before.findings);
    write(&dir, "src/main/java/demo/Lib.java", "package demo;\npublic class Lib {\n    public void f(int a, int b) {}\n    public void f(int a) {}\n}\n");
    let opts = |base: Option<String>| CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base };
    let r = s.check(&opts(Some("HEAD".into())));
    assert!(r.pending.is_empty() && r.breaks.is_empty(), "--base: {:?}", r.pending);
    let r = s.check(&opts(None));
    assert!(r.pending.is_empty() && r.breaks.is_empty(), "per turn: {:?}", r.pending);
    let _ = std::fs::remove_dir_all(&repo);
}

/// A Spring application in a repository, for the wiring claims.
fn spring_project(name: &str, files: &[(&str, &str)]) -> (std::path::PathBuf, std::path::PathBuf) {
    let repo = std::env::temp_dir().join(format!("grounds-claim-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    let dir = repo.join("app");
    write(&dir, "settings.gradle", "rootProject.name = 'demo'\n");
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    for a in ["Service", "Component"] {
        write(&dir, &format!("src/main/java/org/springframework/stereotype/{a}.java"), &format!("package org.springframework.stereotype;\npublic @interface {a} {{}}\n"));
    }
    write(&dir, "src/main/java/org/springframework/boot/autoconfigure/SpringBootApplication.java", "package org.springframework.boot.autoconfigure;\npublic @interface SpringBootApplication {}\n");
    write(&dir, "src/main/java/demo/App.java", "package demo;\n@org.springframework.boot.autoconfigure.SpringBootApplication\npublic class App { public static void main(String[] args) {} }\n");
    for (f, t) in files {
        write(&dir, &format!("src/main/java/demo/{f}"), t);
    }
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    (repo, dir)
}

fn base_check(dir: &std::path::Path) -> grounds_engine::session::Report {
    let mut s = Session::open(dir).unwrap();
    s.check(&CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base: Some("HEAD".into()) })
}

fn rules_of(r: &grounds_engine::session::Report) -> Vec<(String, &'static str)> {
    let mut v: Vec<_> = r.findings.iter().chain(&r.pending).filter(|f| grounds_engine::session::compared_in_changed(f.rule)).map(|f| (f.path.rsplit('/').next().unwrap().to_string(), f.rule)).collect();
    v.sort();
    v
}

/// S5: what an unchanged class is missing changes; the number does not.
#[test]
fn a_different_missing_method_is_a_new_claim() {
    let (repo, dir) = spring_project("unimpl", &[("I.java", "package demo;\npublic interface I {\n    void old();\n}\n"), ("C.java", "package demo;\npublic class C implements I {}\n")]);
    assert_eq!(rules_of(&base_check(&dir)), vec![], "nothing changed yet");
    write(&dir, "src/main/java/demo/I.java", "package demo;\npublic interface I {\n    void fresh();\n}\n");
    let r = base_check(&dir);
    assert_eq!(rules_of(&r), vec![("C.java".into(), "unimplemented-abstract-method")], "{:?}", r.pending);
    assert!(r.pending[0].message.contains("I.fresh()"), "{:?}", r.pending);
    let _ = std::fs::remove_dir_all(&repo);
}

/// S7: other candidates for the same ambiguous injection point.
#[test]
fn other_ambiguous_candidates_are_a_new_claim() {
    let bean = |n: &str| format!("package demo;\n@org.springframework.stereotype.Service\npublic class {n} implements Api {{}}\n");
    let (repo, dir) = spring_project(
        "ambiguous",
        &[("Api.java", "package demo;\npublic interface Api {}\n"), ("One.java", &bean("One")), ("Two.java", &bean("Two")), ("Vet.java", "package demo;\n@org.springframework.stereotype.Component\npublic class Vet {\n    Vet(Api api) {}\n}\n")],
    );
    std::fs::remove_file(dir.join("src/main/java/demo/One.java")).unwrap();
    std::fs::remove_file(dir.join("src/main/java/demo/Two.java")).unwrap();
    write(&dir, "src/main/java/demo/Three.java", &bean("Three"));
    write(&dir, "src/main/java/demo/Four.java", &bean("Four"));
    let r = base_check(&dir);
    assert_eq!(rules_of(&r), vec![("Vet.java".into(), "ambiguous-bean")], "{:?}", r.pending);
    // the same candidates in another order of declaration are the old claim
    write(&dir, "src/main/java/demo/Api.java", "package demo;\n// reordered\npublic interface Api {}\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "three and four"]);
    std::fs::remove_file(dir.join("src/main/java/demo/Four.java")).unwrap();
    write(&dir, "src/main/java/demo/Four.java", &format!("package demo;\n\n{}", &bean("Four")["package demo;\n".len()..]));
    assert_eq!(rules_of(&base_check(&dir)), vec![]);
    let _ = std::fs::remove_dir_all(&repo);
}

/// S8, by decision: which cycle the rule reports can depend on the order of
/// parameters (the wiring rules' claims depend on order in the model), so the
/// cycle rule is not stable: whatever it reports after the change is preview.
#[test]
fn a_cycle_found_after_reordered_parameters_is_preview() {
    let c = |n: &str, ps: &str| format!("package demo;\n@org.springframework.stereotype.Component public class {n} {{ {n}({ps}) {{}} }}\n");
    let (repo, dir) = spring_project("cycle", &[("A.java", &c("A", "B b, C c")), ("B.java", &c("B", "A a")), ("C.java", &c("C", "A a"))]);
    write(&dir, "src/main/java/demo/A.java", &c("A", "C c, B b"));
    let r = base_check(&dir);
    for f in r.findings.iter().chain(&r.pending).filter(|f| f.rule == "circular-constructor-dependency") {
        assert_eq!(f.maturity, grounds_rules::Maturity::Preview, "{f:?}");
    }
    assert!(r.breaks.iter().all(|b| b.finding.maturity == grounds_rules::Maturity::Preview || b.broken.iter().all(|&i| r.pending[i].rule != "circular-constructor-dependency")));
    let _ = std::fs::remove_dir_all(&repo);
}

/// S6, decided: a new injection point of a type whose bean untouched code
/// already missed before the change is the model's gap (limit b), not
/// reported; a missing bean of another type is.
#[test]
fn a_new_injection_point_of_a_type_already_missing_is_the_same_gap() {
    let comp = |n: &str, ps: &str| format!("package demo;\n@org.springframework.stereotype.Component\npublic class {n} {{\n    {n}({ps}) {{}}\n}}\n");
    let (repo, dir) = spring_project("gap", &[("Api.java", "package demo;\npublic interface Api {}\n"), ("Other.java", "package demo;\npublic interface Other {}\n"), ("Old.java", &comp("Old", "Api old"))]);
    write(&dir, "src/main/java/demo/Fresh.java", &comp("Fresh", "Api fresh"));
    assert_eq!(rules_of(&base_check(&dir)), vec![]);
    write(&dir, "src/main/java/demo/Fresh.java", &comp("Fresh", "Other fresh"));
    assert_eq!(rules_of(&base_check(&dir)), vec![("Fresh.java".into(), "bean-not-found")]);
    let _ = std::fs::remove_dir_all(&repo);
}

/// Two types with one simple name: an injection point of `b.Api` missing its
/// bean is not the gap untouched code had for `a.Api`.
#[test]
fn a_missing_bean_of_another_type_with_the_same_simple_name_is_reported() {
    let (repo, dir) = spring_project(
        "samename",
        &[
            ("a/Api.java", "package demo.a;\npublic interface Api {}\n"),
            ("b/Api.java", "package demo.b;\npublic interface Api {}\n"),
            ("Old.java", "package demo;\n@org.springframework.stereotype.Component\npublic class Old {\n    Old(demo.a.Api api) {}\n}\n"),
        ],
    );
    write(&dir, "src/main/java/demo/Fresh.java", "package demo;\n@org.springframework.stereotype.Component\npublic class Fresh {\n    Fresh(demo.b.Api api) {}\n}\n");
    assert_eq!(rules_of(&base_check(&dir)), vec![("Fresh.java".into(), "bean-not-found")]);
    write(&dir, "src/main/java/demo/Fresh.java", "package demo;\n@org.springframework.stereotype.Component\npublic class Fresh {\n    Fresh(demo.a.Api api) {}\n}\n");
    assert_eq!(rules_of(&base_check(&dir)), vec![]);
    let _ = std::fs::remove_dir_all(&repo);
}

/// S9: which of several private overloads is shown depends on declaration
/// order; the claim (every fitting overload is private) does not.
#[test]
fn reordered_private_overloads_are_the_old_claim() {
    let (repo, dir) = spring_project("private", &[("Lib.java", "package demo;\npublic class Lib {\n    private void f(int a) {}\n    private void f(long a) {}\n}\n"), ("Caller.java", "package demo;\nclass Caller {\n    void t(Lib x) { x.f(1); }\n}\n")]);
    let mut s = Session::open(&dir).unwrap();
    let all = s.check(&CheckOptions { mode: Mode::Checkpoint, files: vec![dir.join("src/main/java/demo/Caller.java")], all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None });
    assert!(all.findings.iter().any(|f| f.rule == "unresolved-method" && f.message.contains("is private")), "{:?}", all.findings);
    write(&dir, "src/main/java/demo/Lib.java", "package demo;\npublic class Lib {\n    private void f(long a) {}\n    private void f(int a) {}\n}\n");
    assert_eq!(rules_of(&base_check(&dir)), vec![]);
    let _ = std::fs::remove_dir_all(&repo);
}

/// S10: the same requirement inherited from two interfaces, listed in the
/// other order, is the old claim.
#[test]
fn a_requirement_from_reordered_interfaces_is_the_old_claim() {
    let (repo, dir) = spring_project(
        "twointerfaces",
        &[("I.java", "package demo;\npublic interface I {\n    void f();\n}\n"), ("J.java", "package demo;\npublic interface J {\n    void f();\n}\n"), ("C.java", "package demo;\npublic class C implements I, J {}\n")],
    );
    let mut s = Session::open(&dir).unwrap();
    let all = s.check(&CheckOptions { mode: Mode::Checkpoint, files: vec![dir.join("src/main/java/demo/C.java")], all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None });
    assert!(all.findings.iter().any(|f| f.rule == "unimplemented-abstract-method"), "{:?}", all.findings);
    write(&dir, "src/main/java/demo/C.java", "package demo;\npublic class C implements J, I {}\n");
    assert_eq!(rules_of(&base_check(&dir)), vec![]);
    let _ = std::fs::remove_dir_all(&repo);
}

/// S11: a short cycle next to a long route to the same class is found
/// whatever the order of the search.
#[test]
fn a_short_cycle_beside_a_long_route_is_found() {
    let c = |n: &str, ps: &str| (format!("{n}.java"), format!("package demo;\n@org.springframework.stereotype.Component public class {n} {{ {n}({ps}) {{}} }}\n"));
    let mut files = vec![c("S", "Z z, A a"), c("A", "R r"), c("R", "S s"), c("Z", "Z1 z"), c("Z6", "R r")];
    for i in 1..6 {
        files.push(c(&format!("Z{i}"), &format!("Z{} z", i + 1)));
    }
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let (repo, dir) = spring_project("longroute", &refs);
    let mut s = Session::open(&dir).unwrap();
    let all = s.check(&CheckOptions { mode: Mode::Checkpoint, files: vec![dir.join("src/main/java/demo/S.java")], all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None });
    let cyc: Vec<&str> = all.findings.iter().filter(|f| f.rule == "circular-constructor-dependency").map(|f| f.message.as_str()).collect();
    assert_eq!(cyc, ["Circular constructor dependency: S -> A -> R -> S"], "{:?}", all.findings);
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn a_tracked_file_in_an_ignored_directory_that_comes_back_is_read_again() {
    // A fresh load reads files git tracks in a directory .gitignore matches;
    // the discovery never walks that directory, so the session must watch it
    // all the same: a tracked file deleted there and restored is found again,
    // and an untracked one beside it is not (as for a fresh load).
    let dir = restored_project("tracked-ignored");
    write(&dir, ".gitignore", "/gen/\n");
    write(&dir, "gen/demo/Gen.java", "package demo;\npublic class Gen {\n    Req r;\n}\n");
    git(&dir, &["add", "-f", ".gitignore", "gen/demo/Gen.java"]);
    git(&dir, &["commit", "-qm", "generated"]);
    write(&dir, "gen/demo/Untracked.java", "package demo;\npublic class Untracked {}\n");
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    set_tree_mtime(&dir, ".", old);
    set_mtime(&dir, ".git/info/exclude", old);
    let gen = "gen/demo/Gen.java";
    let text = std::fs::read_to_string(dir.join(gen)).unwrap();
    let mut s = Session::open(&dir).unwrap();
    assert!(holdings(&s).0.iter().any(|f| f.starts_with(gen)), "a fresh load reads the tracked file: {:?}", holdings(&s).0);
    std::fs::remove_file(dir.join(gen)).unwrap();
    holds_as_fresh(&mut s, &dir, "a tracked file in an ignored directory was deleted");
    write(&dir, gen, &text);
    set_mtime(&dir, gen, old);
    holds_as_fresh(&mut s, &dir, "the tracked file in an ignored directory came back");
    listed_not_walked(&s);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stable_rule_is_stable_on_changed_lines_on_unchanged_lines_and_outside_git() {
    use grounds_rules::Maturity;
    let (repo, dir) = gap_project("stable-rule");
    // an unused local before the change, on a line the change does not touch
    write(&dir, "src/main/java/demo/Old.java", "package demo;\nclass Old {\n    void a() { int before = 1; }\n}\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "old"]);
    let mut s = Session::open(&dir).unwrap();
    write(&dir, "src/main/java/demo/Old.java", "package demo;\nclass Old {\n    void a() { int before = 1; }\n    void b() { int added = 1; }\n}\n");
    let opts = |all_lines| CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines, use_baseline: false, min_severity: Severity::Info, base: None };
    let unused = |r: &grounds_engine::session::Report| -> Vec<(u32, Maturity)> {
        let mut v: Vec<_> = r.findings.iter().filter(|f| f.rule == "unused-local-variable").map(|f| (f.line as u32, f.maturity)).collect();
        v.sort_by_key(|x| x.0);
        v
    };
    // changed lines only: the new one, stable
    assert_eq!(unused(&s.check(&opts(false))), [(4, Maturity::Stable)]);
    // `--all-lines`: the old one too, and it is stable as well
    assert_eq!(unused(&s.check(&opts(true))), [(3, Maturity::Stable), (4, Maturity::Stable)]);
    // outside a git repository
    let _ = std::fs::remove_dir_all(repo.join(".git"));
    let r = Session::open(&dir).unwrap().check(&CheckOptions { files: vec![dir.join("src/main/java/demo/Old.java")], ..opts(true) });
    assert_eq!(unused(&r), [(3, Maturity::Stable), (4, Maturity::Stable)]);
    let _ = std::fs::remove_dir_all(&repo);
}

/// What `grounds changed --base HEAD` reports, through a session that has
/// lived through the changes and through a fresh one: the same, and the JDK
/// API the two load.
fn changed_as_fresh(s: &mut Session, dir: &Path, what: &str) -> String {
    let opts = || CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base: Some("HEAD".into()) };
    let warm = json(&s.check(&opts()));
    let mut fresh = Session::open(dir).unwrap();
    assert_eq!(warm, json(&fresh.check(&opts())), "after {what}, the session must report what a fresh run reports");
    assert_eq!(s.project.stats.java_release, fresh.project.stats.java_release, "after {what}, the session must check against the JDK API a fresh run loads");
    assert_eq!(holdings(s), holdings(&fresh), "after {what}");
    warm
}

#[test]
fn a_module_that_gains_its_first_sources_gets_the_jdk_api_a_fresh_load_picks() {
    // The release review's R2: the root module compiles for exactly Java 8,
    // a module `later` for 21 holds no sources yet, so the JDK API loaded is
    // 8's. A source added to `later` makes it count: a fresh load loads 21's,
    // where `ArrayList.getFirst` exists. The session must do the same, and go
    // back when the module has no sources again.
    let dir = std::env::temp_dir().join(format!("grounds-session-release-plan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "settings.gradle", "rootProject.name = 'demo'\ninclude 'later'\n");
    write(&dir, "build.gradle", "plugins { id 'java' }\ntasks.withType(JavaCompile) { options.release = 8 }\n");
    write(&dir, "later/build.gradle", "plugins { id 'java' }\ntasks.withType(JavaCompile) { options.release = 21 }\n");
    write(&dir, "src/main/java/Earlier.java", "class Earlier {}\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);
    let mut s = Session::open(&dir).unwrap();
    changed_as_fresh(&mut s, &dir, "the load");
    let java8 = s.project.stats.java_release;
    write(&dir, "later/src/main/java/Later.java", "class Later extends java.util.ArrayList<String> { void test() { new Later().getFirst(); } }\n");
    let r = changed_as_fresh(&mut s, &dir, "a source was added to a module that had none");
    assert!(!r.contains("unresolved-method"), "{r}");
    let java21 = s.project.stats.java_release;
    // (where a JDK is installed that can tell the two apart)
    if java8 == 8 {
        assert_eq!(java21, 21, "the newly populated module counts");
    }
    std::fs::remove_file(dir.join("later/src/main/java/Later.java")).unwrap();
    changed_as_fresh(&mut s, &dir, "the module lost its only source");
    assert_eq!(s.project.stats.java_release, java8);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_git_starts_or_stops_tracking_against_gitignore_is_read_or_dropped() {
    // The release review's R8: a fresh load reads the files git tracks
    // although .gitignore matches them. `git add -f` of such a file changes
    // only git's index: the session must find the file all the same, and
    // drop it when git stops tracking it.
    let dir = std::env::temp_dir().join(format!("grounds-session-index-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, ".gitignore", "Hidden.java\n");
    write(&dir, "src/main/java/Use.java", "class Use {}\n");
    write(&dir, "src/main/java/Hidden.java", "class Hidden {}\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);
    // (old times: what sync does, not a recent time, must find the file)
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    set_tree_mtime(&dir, ".", old);
    set_mtime(&dir, ".git/info/exclude", old);
    let mut s = Session::open(&dir).unwrap();
    changed_as_fresh(&mut s, &dir, "the load");
    git(&dir, &["add", "-f", "src/main/java/Hidden.java"]);
    git(&dir, &["commit", "-qm", "track"]);
    write(&dir, "src/main/java/Use.java", "class Use { void test(Hidden h) { h.missing(); } }\n");
    let r = changed_as_fresh(&mut s, &dir, "git started tracking an ignored file");
    assert!(r.contains("unresolved-method"), "{r}");
    // and back: untracked (still on disk, still ignored), it goes
    git(&dir, &["rm", "-q", "--cached", "src/main/java/Hidden.java"]);
    let r = changed_as_fresh(&mut s, &dir, "git stopped tracking an ignored file");
    assert!(!r.contains("unresolved-method"), "{r}");
    // a commit that touches neither: nothing changes
    git(&dir, &["commit", "-qm", "untrack"]);
    changed_as_fresh(&mut s, &dir, "a commit");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_git_listing_that_fails_is_asked_again_once_git_answers() {
    // Review of #91 (D1): when git cannot list the files it tracks although
    // ignored (here a corrupt index; or git missing from PATH), which they
    // are is not known. The session must not take the failure for an empty
    // list and accept the index as read: once git answers again, it must
    // hold what a fresh load holds. The corrupt index has the size and time
    // of the good one put back, so only asking again can tell.
    let dir = std::env::temp_dir().join(format!("grounds-session-index-fails-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, ".gitignore", "Hidden.java\nKept.java\n");
    write(&dir, "src/main/java/Use.java", "class Use {}\n");
    write(&dir, "src/main/java/Hidden.java", "class Hidden {}\n");
    write(&dir, "src/main/java/Kept.java", "class Kept {}\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["add", "-f", "src/main/java/Kept.java"]);
    git(&dir, &["commit", "-qm", "init"]);
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    set_tree_mtime(&dir, ".", old);
    set_mtime(&dir, ".git/info/exclude", old);
    set_mtime(&dir, ".git/index", old);
    let mut s = Session::open(&dir).unwrap();
    changed_as_fresh(&mut s, &dir, "the load");
    git(&dir, &["add", "-f", "src/main/java/Hidden.java"]);
    git(&dir, &["commit", "-qm", "track"]);
    write(&dir, "src/main/java/Use.java", "class Use { void test(Hidden h, Kept k) { h.missing(); k.gone(); } }\n");
    let index = dir.join(".git/index");
    let good = std::fs::read(&index).unwrap();
    let mut bad = good.clone();
    bad[..4].copy_from_slice(b"XXXX");
    let at = std::time::SystemTime::now() - std::time::Duration::from_secs(1800);
    std::fs::write(&index, &bad).unwrap();
    set_mtime(&dir, ".git/index", at);
    let opts = CheckOptions { mode: Mode::Checkpoint, files: vec![dir.join("src/main/java/Use.java")], all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None };
    let r = json(&s.check(&opts));
    // (git failed: the files found tracked before are still held)
    assert!(r.contains("`Kept` has no method `gone`"), "{r}");
    // and while git keeps failing, it is asked again without a walk of the workspace
    let walks = s.full_walks();
    s.sync(&[]);
    s.sync(&[]);
    assert_eq!(s.full_walks(), walks, "no walk of the workspace while git cannot list");
    std::fs::write(&index, &good).unwrap();
    set_mtime(&dir, ".git/index", at);
    let r = json(&s.check(&opts));
    assert_eq!(r, json(&Session::open(&dir).unwrap().check(&opts)), "once git answers again, the session must report what a fresh run reports");
    assert!(r.contains("`Hidden` has no method `missing`"), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}
