//! Fixture-driven rule tests.
//!
//! Each `tests/fixtures/<rule-id>.java` file is analyzed with only that rule
//! enabled. Lines ending with `// expect` (or `// expect: <rule-id>`) must
//! produce a diagnostic; all other lines must not. A first-line comment
//! `// grounds: test` marks the fixture as test code; `// grounds:
//! bean-overriding` enables Spring bean definition overriding; `// grounds:
//! no-findings` marks a fixture that must produce nothing.
//! `tests/support/**.java` provides library stubs (Spring, JUnit, SLF4J...).

use grounds_index::{extract, Classpath, Index};
use grounds_rules::framework::FileInfo;
use grounds_rules::{all_rules, RuleSet};
use grounds_semantic::FileModel;
use grounds_syntax::ParsedFile;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn java_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            java_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "java") {
            out.push(p);
        }
    }
}

fn base_index() -> Index {
    use std::sync::OnceLock;
    static CP: OnceLock<Arc<Classpath>> = OnceLock::new();
    static STUBS: OnceLock<Vec<(Arc<str>, Arc<grounds_index::FileStub>)>> = OnceLock::new();
    let cp = CP.get_or_init(|| Arc::new(Classpath::build(grounds_classfile::find_java_home().as_deref(), 21, &[]))).clone();
    let stubs = STUBS.get_or_init(|| {
        let mut files = Vec::new();
        java_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support"), &mut files);
        files
            .into_iter()
            .map(|f| {
                let text = std::fs::read_to_string(&f).unwrap();
                let pf = ParsedFile::parse(text);
                (Arc::from(f.to_string_lossy().as_ref()), Arc::new(extract(&pf)))
            })
            .collect()
    });
    let mut idx = Index::new(cp);
    // the fixtures compile for exactly the release whose API is loaded
    idx.set_exact_releases(vec![("fixtures/".to_string(), Some(21))]);
    for (k, s) in stubs {
        idx.set_file(k.clone(), s.clone());
    }
    idx
}

fn expected_lines(text: &str, rule: &str) -> BTreeSet<usize> {
    let mut s = BTreeSet::new();
    for (i, line) in text.lines().enumerate() {
        if let Some(pos) = line.find("// expect") {
            let rest = line[pos + 9..].trim();
            let rest = rest.strip_prefix(':').map(|r| r.trim()).unwrap_or(rest);
            if rest.is_empty() || rest.split([',', ' ']).any(|r| r == rule) {
                s.insert(i + 1);
            }
        }
    }
    s
}

#[test]
fn rule_fixtures() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let filter = std::env::var("RULE").ok();
    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "java")).collect();
    fixtures.sort();
    let rules = all_rules();
    let ids: BTreeSet<&str> = rules.iter().map(|r| r.meta().id).collect();
    let mut failures = Vec::new();
    let mut checked = 0;
    for fx in &fixtures {
        let stem = fx.file_stem().unwrap().to_string_lossy().to_string();
        let rule_id = stem.split("__").next().unwrap().to_string();
        if let Some(f) = &filter {
            if !rule_id.contains(f.as_str()) {
                continue;
            }
        }
        if !ids.contains(rule_id.as_str()) {
            failures.push(format!("{stem}: no rule with id {rule_id}"));
            continue;
        }
        let text = std::fs::read_to_string(fx).unwrap();
        let first = text.lines().next().unwrap_or("");
        let is_test = first.contains("grounds: test");
        let mut idx = base_index();
        let key: Arc<str> = Arc::from(format!("fixtures/{stem}.java").as_str());
        let pf = ParsedFile::parse(text.clone());
        idx.set_file(key.clone(), Arc::new(extract(&pf)));
        let fm = FileModel::new(&pf, &idx, key.clone());
        let selected: Vec<Box<dyn grounds_rules::Rule>> = all_rules().into_iter().filter(|r| r.meta().id == rule_id).collect();
        let set = RuleSet::new(selected);
        let mut options = grounds_rules::framework::RuleOptions::default();
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("// option ") {
                if let Some((k, v)) = rest.split_once('=') {
                    options.insert(format!("{rule_id}.{}", k.trim()), v.trim().to_string());
                }
            }
        }
        let info = FileInfo { path: key.to_string(), is_test, is_generated: false };
        // `// grounds: bean-overriding` stands for allow-bean-definition-overriding=true
        let settings = grounds_di::Settings { bean_overriding_allowed: first.contains("grounds: bean-overriding"), ..Default::default() };
        let di = grounds_di::DiModel::build_with(&idx, &[], &[], settings);
        let diags = set.check(&fm, &info, Some(&di), &options);
        let actual: BTreeSet<usize> = diags.iter().map(|d| pf.lines.line_of(d.range.start) + 1).collect();
        let expected = expected_lines(&text, &rule_id);
        checked += 1;
        if expected.is_empty() && !first.contains("grounds: no-findings") {
            failures.push(format!("{stem}: fixture has no `// expect` markers (or `// grounds: no-findings`)"));
        }
        if actual != expected {
            let missing: Vec<_> = expected.difference(&actual).collect();
            let extra: Vec<_> = actual.difference(&expected).collect();
            let mut msg = format!("{stem}: missing lines {missing:?}, unexpected lines {extra:?}");
            for d in &diags {
                let l = pf.lines.line_of(d.range.start) + 1;
                if extra.contains(&&l) {
                    msg.push_str(&format!("\n    line {l}: {}", d.message));
                }
            }
            failures.push(msg);
        }
    }
    eprintln!("checked {checked} fixtures");
    if !failures.is_empty() {
        panic!("{} fixture failures:\n  {}", failures.len(), failures.join("\n  "));
    }
}

#[test]
fn every_rule_has_a_fixture_and_valid_metadata() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut missing = Vec::new();
    let mut seen = BTreeSet::new();
    for r in all_rules() {
        let m = r.meta();
        assert!(seen.insert(m.id), "duplicate rule id {}", m.id);
        assert!(m.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'), "bad id {}", m.id);
        assert!(!m.description.is_empty(), "{} has no description", m.id);
        assert!(!m.sources.is_empty(), "{} has no sources", m.id);
        if !dir.join(format!("{}.java", m.id)).exists() {
            missing.push(m.id);
        }
    }
    assert!(missing.is_empty(), "rules without fixtures: {missing:?}");
}

/// The stable rules, in `grounds check` and `grounds changed` alike. Each has
/// at least 20 judged findings with at most 1 false on the 60 projects of the
/// three hold-out sets, which the rules were never tuned on
/// (docs/holdout/2026-11.md, 2026-11b.md, 2026-11c.md: 8 sampled findings
/// per rule and set). `fixme-comment` meets the same bar but only marks a
/// comment, so it stays preview. A rule moves here only with such evidence;
/// this list makes that a deliberate change.
const STABLE: &[&str] = &[
    "assertion-argument-order",
    "public-static-mutable-container",
    "public-static-non-final-field",
    "redundant-null-check",
    "static-non-thread-safe-field",
    "unused-local-variable",
    "unused-private-field",
    "unused-private-method",
];

/// The rules stable in `grounds changed` only, where their findings are
/// reported when new with the change: the compile-break rules. A replay of
/// 514 commits of the 40 hold-out projects (docs/holdout/2026-11-replay.md)
/// got no break report from them, against 246 false compile-break and wiring
/// findings from a whole-project run of the same projects. The wiring rules
/// are left out: their claims depend on the order of maps and annotations in
/// the wiring model (review of #74). Like `STABLE`, this list changes only
/// with evidence.
const STABLE_IN_CHANGED: &[&str] = &[
    "missing-jdk-import",
    "override-matches-nothing",
    "unimplemented-abstract-method",
    "unresolved-constructor",
    "unresolved-field",
    "unresolved-method",
];

#[test]
fn stable_rules_are_exactly_the_intended_set() {
    use grounds_rules::Maturity;
    // `maturity` is a required field of `declare_rule!`, so every rule has one
    let mut stable: Vec<_> = all_rules().iter().map(|r| r.meta()).filter(|m| m.maturity == Maturity::Stable).map(|m| m.id).collect();
    stable.sort();
    assert_eq!(stable, STABLE, "the set of stable rules changed; update STABLE (and the README) only with evidence");
    let preview = all_rules().iter().filter(|r| r.meta().maturity == Maturity::Preview).count();
    assert_eq!(preview + STABLE.len(), all_rules().len());

    // stable in `grounds changed`, preview in `grounds check`
    let mut in_changed: Vec<_> = all_rules().iter().map(|r| r.meta()).filter(|m| m.maturity == Maturity::Preview && m.stable_in_changed()).map(|m| m.id).collect();
    in_changed.sort();
    assert_eq!(in_changed, STABLE_IN_CHANGED, "the set of rules stable in `grounds changed` changed; update STABLE_IN_CHANGED (and the README) only with evidence");
    for m in all_rules().iter().map(|r| r.meta()) {
        let (check, changed) = (m.maturity_in(false), m.maturity_in(true));
        assert_eq!(check, m.maturity, "{}", m.id);
        assert_eq!(changed == Maturity::Stable, STABLE.contains(&m.id) || STABLE_IN_CHANGED.contains(&m.id), "{}", m.id);
    }
    // every id names a rule
    let ids: Vec<_> = all_rules().iter().map(|r| r.meta().id).collect();
    assert!(grounds_rules::STABLE_IN_CHANGED.iter().all(|id| ids.contains(id)), "{:?}", grounds_rules::STABLE_IN_CHANGED);
}
