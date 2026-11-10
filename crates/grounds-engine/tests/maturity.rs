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
    // (no rule is stable yet, so this is the empty list for now)
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
