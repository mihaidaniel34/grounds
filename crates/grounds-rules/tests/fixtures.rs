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
        let mut di = grounds_di::DiModel::build(&idx);
        // `// grounds: bean-overriding` stands for allow-bean-definition-overriding=true
        di.bean_overriding_allowed = first.contains("grounds: bean-overriding");
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
