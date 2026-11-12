//! `[rules] preview`: whether preview rules run, and that naming a rule by id
//! runs it whatever its maturity.

use grounds_engine::analyze::{analyze_files, build_ruleset, selected_rules};
use grounds_engine::Project;
use grounds_rules::Maturity;
use std::path::PathBuf;

fn project(name: &str, toml: Option<&str>) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grounds-maturity-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/main/java/app")).unwrap();
    std::fs::write(dir.join("build.gradle"), "plugins { id 'java' }\n").unwrap();
    // `s == "x"` is reported by `string-reference-equality`, a preview rule
    std::fs::write(dir.join("src/main/java/app/A.java"), "package app;\nclass A {\n    boolean m(String s) { return s == \"x\"; }\n}\n").unwrap();
    if let Some(t) = toml {
        std::fs::write(dir.join("grounds.toml"), t).unwrap();
    }
    dir
}

fn ids(project: &Project) -> Vec<&'static str> {
    selected_rules(project).iter().map(|r| r.meta().id).collect()
}

fn rules_found(project: &Project) -> Vec<&'static str> {
    let (set, options) = build_ruleset(project);
    let files: Vec<_> = project.workspace.files.iter().map(|f| f.path.clone()).collect();
    let mut out: Vec<_> = analyze_files(project, &set, &options, &files).into_iter().map(|f| f.rule).collect();
    out.dedup();
    out
}

#[test]
fn preview_rules_run_by_default() {
    let dir = project("default", None);
    let p = Project::load(&dir).unwrap();
    assert!(p.config.rules.preview);
    let expected: Vec<_> = grounds_rules::catalog().iter().filter(|m| m.default_enabled).map(|m| m.id).collect();
    assert_eq!(ids(&p), expected);
    assert_eq!(rules_found(&p), ["string-reference-equality"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_false_turns_preview_rules_off() {
    let dir = project("off", Some("[rules]\npreview = false\n"));
    let p = Project::load(&dir).unwrap();
    assert!(!p.config.rules.preview);
    let expected: Vec<_> = grounds_rules::catalog().iter().filter(|m| m.default_enabled && m.maturity == Maturity::Stable).map(|m| m.id).collect();
    assert_eq!(ids(&p), expected);
    assert!(rules_found(&p).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_preview_rule_named_explicitly_runs_anyway() {
    // by id in grounds.toml
    let dir = project("named", Some("[rules]\npreview = false\nenable = [\"string-reference-equality\", \"bean-not-found\"]\n"));
    let p = Project::load(&dir).unwrap();
    assert_eq!(ids(&p), ["string-reference-equality", "bean-not-found"]);
    assert_eq!(rules_found(&p), ["string-reference-equality"]);
    let _ = std::fs::remove_dir_all(&dir);

    // by id as `--rules` sets it, and by one of its source ids
    let dir = project("named-cli", Some("[rules]\npreview = false\n"));
    let mut p = Project::load(&dir).unwrap();
    p.config.rules.enable = vec!["string-reference-equality".into()];
    assert_eq!(rules_found(&p), ["string-reference-equality"]);
    p.config.rules.enable = vec!["sonar:S4973".into()];
    assert!(ids(&p).contains(&"string-reference-equality"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_category_does_not_name_its_preview_rules() {
    let dir = project("category", Some("[rules]\npreview = false\nenable = [\"bug\"]\n"));
    let p = Project::load(&dir).unwrap();
    let got = ids(&p);
    // `string-reference-equality` is a bug rule, and preview
    assert!(!got.contains(&"string-reference-equality"), "{got:?}");
    assert!(got.iter().all(|id| grounds_rules::meta_of(id).unwrap().maturity == Maturity::Stable), "{got:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn json_findings_carry_the_maturity() {
    let dir = project("json", None);
    let p = Project::load(&dir).unwrap();
    let (set, options) = build_ruleset(&p);
    let files: Vec<_> = p.workspace.files.iter().map(|f| f.path.clone()).collect();
    let found = analyze_files(&p, &set, &options, &files);
    let v: serde_json::Value = serde_json::from_str(&grounds_engine::report::json(&found)).unwrap();
    let f = &v[0];
    assert_eq!(f["rule"], "string-reference-equality");
    assert_eq!(f["maturity"], "preview");
    // the fields from before, plus `maturity` (this rule offers a fix)
    let mut keys: Vec<_> = f.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    keys.sort();
    assert_eq!(keys, ["category", "col", "end_col", "end_line", "fingerprint", "fix", "line", "maturity", "message", "path", "rule", "severity"]);
    let _ = std::fs::remove_dir_all(&dir);
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").status().unwrap().success();
    assert!(ok, "git {args:?}");
}

#[test]
fn the_break_rules_are_stable_in_changed_and_preview_in_check() {
    use grounds_engine::analyze::selected_rules_for;
    let dir = project("modes", Some("[rules]\npreview = false\n"));
    let p = Project::load(&dir).unwrap();
    // `grounds check`: exactly the stable rules
    let stable: Vec<_> = grounds_rules::catalog().iter().filter(|m| m.default_enabled && m.maturity == Maturity::Stable).map(|m| m.id).collect();
    assert!(!stable.is_empty());
    let mut check: Vec<_> = selected_rules_for(&p, false).iter().map(|r| r.meta().id).collect();
    check.sort();
    let mut want = stable.clone();
    want.sort();
    assert_eq!(check, want);
    // `grounds changed`: those, plus the compile-break rules of `STABLE_IN_CHANGED`
    let mut changed: Vec<_> = selected_rules_for(&p, true).iter().map(|r| r.meta().id).collect();
    changed.sort();
    let mut expected: Vec<_> = grounds_rules::STABLE_IN_CHANGED.iter().copied().filter(|id| grounds_rules::meta_of(id).unwrap().default_enabled).chain(stable.iter().copied()).collect();
    expected.sort();
    assert_eq!(changed, expected);
    // every stable rule is compared with the base; the wiring rules are
    // compared too, but are not stable (and so do not run with `preview = false`)
    use grounds_engine::session::{compared_in_changed, stable_in_changed};
    assert!(grounds_rules::STABLE_IN_CHANGED.iter().all(|id| compared_in_changed(id)));
    for id in ["bean-not-found", "ambiguous-bean", "qualifier-not-found", "bean-name-conflict", "circular-constructor-dependency"] {
        assert!(compared_in_changed(id) && !stable_in_changed(id), "{id}");
        assert!(!changed.contains(&id), "{id}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// With `preview = false`, `grounds changed` still reports what a change
/// breaks (and only that), as stable, while `grounds check` does not run the
/// rules at all; with the default, `check` reports the same finding as preview.
#[test]
fn preview_false_keeps_the_breaks_in_changed_only() {
    use grounds_engine::session::{CheckOptions, Mode, Session};
    for preview in [false, true] {
        let dir = project(&format!("switch-{preview}"), Some(&format!("[rules]\npreview = {preview}\n")));
        git(&dir, &["init", "-q"]);
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-qm", "init"]);
        // a new break and a preview rule's finding on changed lines
        std::fs::write(dir.join("src/main/java/app/A.java"), "package app;\nclass A {\n    boolean m(String s) { return s == \"y\"; }\n    void n(A a) { a.nothing(); }\n}\n").unwrap();

        let mut s = Session::open(&dir).unwrap();
        let r = s.check(&CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: grounds_rules::Severity::Info, base: None }).unwrap();
        let got: Vec<(&str, Maturity)> = r.findings.iter().map(|f| (f.rule, f.maturity)).collect();
        if preview {
            assert_eq!(got, [("unresolved-method", Maturity::Stable), ("string-reference-equality", Maturity::Preview)]);
        } else {
            assert_eq!(got, [("unresolved-method", Maturity::Stable)]);
        }

        let p = Project::load(&dir).unwrap();
        let found = rules_found(&p);
        if preview {
            // `check` reports the break too, as preview
            let (set, options) = build_ruleset(&p);
            let files: Vec<_> = p.workspace.files.iter().map(|f| f.path.clone()).collect();
            let all = analyze_files(&p, &set, &options, &files);
            let f = all.iter().find(|f| f.rule == "unresolved-method").unwrap();
            assert_eq!(f.maturity, Maturity::Preview);
            let v: serde_json::Value = serde_json::from_str(&grounds_engine::report::json(&all)).unwrap();
            assert!(v.as_array().unwrap().iter().all(|f| f["maturity"] == "preview"));
            let sarif: serde_json::Value = serde_json::from_str(&grounds_engine::report::sarif(&all, &dir, &[])).unwrap();
            for d in sarif["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap() {
                assert_eq!(d["properties"]["maturity"], "preview", "{d}");
            }
        } else {
            assert!(found.is_empty(), "{found:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
