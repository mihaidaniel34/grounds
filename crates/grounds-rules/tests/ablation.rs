//! Hiding information from the analyzer may only remove findings.
//!
//! grounds stays silent about what it cannot see: an unknown type, method or
//! annotation is not evidence of anything. So a finding that appears only when
//! the JDK or a library is hidden rests on something being invisible, and it
//! is a false positive in every project where a jar is simply not found (an
//! unresolved Maven dependency, a classpath grounds could not build, a JDK it
//! could not locate).
//!
//! Each `tests/fixtures/*.java` file is analyzed with all rules, the JDK
//! (found through `JAVA_HOME`) and every `tests/support/**.java` stub visible.
//! Then it is analyzed again with less, once per ablation:
//!
//! - `no-jdk`: the JDK is hidden;
//! - `no-libraries`: every support stub is hidden;
//! - `nothing`: both are hidden;
//! - `no-package:<package>`: the stubs of one package are hidden, for each
//!   package whose name the fixture's text contains;
//! - `no-file:<path>`: one support file is hidden, for each file that declares
//!   a type the fixture mentions by simple name (`<path>` is relative to
//!   `tests/support`).
//!
//! A finding is the pair (rule, line); its message is not compared, so a
//! finding that only lists fewer candidates is the same claim. A violation is
//! a finding of an ablated run that the full run does not have.
//!
//! `tests/ablation_baseline.rs` lists the violations known when this test was
//! written, one per (ablation, rule, fixture, line), sorted. The test fails in
//! two ways:
//!
//! - a violation the baseline does not list: a rule now claims something
//!   because information is missing. Make the rule silent when what it needs
//!   is unknown. Only if the claim truly holds without that information,
//!   regenerate the baseline (below) and say why in the commit.
//! - a baseline entry that no longer occurs: a fix removed it. Delete those
//!   lines from `tests/ablation_baseline.rs`, or regenerate it.
//!
//! Both at once, for the same rule and fixture, usually means the fixture was
//! edited and its lines moved; regenerate the baseline.
//!
//! To regenerate the baseline, run
//! `GROUNDS_ABLATION_BLESS=1 cargo test -p grounds-rules --test ablation`.
//! It rewrites `tests/ablation_baseline.rs` from what it finds and passes;
//! review the diff before committing it.

use grounds_index::{extract, Classpath, FileStub, Index};
use grounds_rules::framework::{FileInfo, RuleOptions};
use grounds_rules::{all_rules, RuleSet};
use grounds_semantic::FileModel;
use grounds_syntax::ParsedFile;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

include!("ablation_baseline.rs");

const BLESS: &str = "GROUNDS_ABLATION_BLESS";

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

/// One `tests/support` file.
struct Support {
    key: Arc<str>,
    stub: Arc<FileStub>,
    package: String,
    /// path relative to `tests/support`, with `/` separators
    rel: String,
    /// simple names of the types it declares
    names: BTreeSet<String>,
}

/// (ablation, rule, fixture stem, line)
type Entry = (String, String, String, usize);

fn index(cp: &Arc<Classpath>, support: &[Support], keep: &dyn Fn(&Support) -> bool) -> Index {
    let mut idx = Index::new(cp.clone());
    // the fixtures compile for exactly the release whose API is loaded
    idx.set_exact_releases(vec![("fixtures/".to_string(), Some(21))]);
    for s in support.iter().filter(|s| keep(s)) {
        idx.set_file(s.key.clone(), s.stub.clone());
    }
    idx
}

/// Every finding of every rule on the fixture: (rule, line) to the smallest
/// message at that place.
fn findings(text: &str, stem: &str, options: &RuleOptions, mut idx: Index) -> BTreeMap<(String, usize), String> {
    let first = text.lines().next().unwrap_or("");
    let key: Arc<str> = Arc::from(format!("fixtures/{stem}.java").as_str());
    let pf = ParsedFile::parse(text.to_string());
    idx.set_file(key.clone(), Arc::new(extract(&pf)));
    let fm = FileModel::new(&pf, &idx, key.clone());
    let set = RuleSet::new(all_rules());
    let info = FileInfo { path: key.to_string(), is_test: first.contains("grounds: test"), is_generated: false };
    // `// grounds: bean-overriding` stands for allow-bean-definition-overriding=true
    let settings = grounds_di::Settings { bean_overriding_allowed: first.contains("grounds: bean-overriding"), ..Default::default() };
    let di = grounds_di::DiModel::build_with(&idx, &[], &[], settings);
    let mut out: BTreeMap<(String, usize), String> = BTreeMap::new();
    for d in set.check(&fm, &info, Some(&di), options) {
        let at = (d.rule.to_string(), pf.lines.line_of(d.range.start) + 1);
        match out.get_mut(&at) {
            Some(m) if *m <= d.message => {}
            Some(m) => *m = d.message.clone(),
            None => {
                out.insert(at, d.message.clone());
            }
        }
    }
    out
}

/// The `// option k=v` lines of a fixture, for the rule it is named after,
/// as `tests/fixtures.rs` reads them.
fn options(text: &str, stem: &str) -> RuleOptions {
    let rule_id = stem.split("__").next().unwrap();
    let mut options = RuleOptions::default();
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("// option ") {
            if let Some((k, v)) = rest.split_once('=') {
                options.insert(format!("{rule_id}.{}", k.trim()), v.trim().to_string());
            }
        }
    }
    options
}

/// The violations of one fixture: findings of an ablated run that the full
/// run does not have, with their message.
fn violations(fx: &Path, jdk: &Arc<Classpath>, no_jdk: &Arc<Classpath>, support: &[Support], packages: &BTreeSet<String>) -> (usize, Vec<(Entry, String)>) {
    let stem = fx.file_stem().unwrap().to_string_lossy().to_string();
    let text = std::fs::read_to_string(fx).unwrap();
    let options = options(&text, &stem);
    let full = findings(&text, &stem, &options, index(jdk, support, &|_| true));
    // every simple identifier the fixture mentions
    let mentioned: BTreeSet<&str> = {
        let pf = ParsedFile::parse(text.clone());
        let names = extract(&pf).referenced_names;
        support.iter().flat_map(|s| s.names.iter()).filter(|n| names.binary_search_by(|m| m.as_ref().cmp(n.as_str())).is_ok()).map(|n| n.as_str()).collect()
    };

    let mut ablations: Vec<(String, Index)> = vec![
        ("no-jdk".into(), index(no_jdk, support, &|_| true)),
        ("no-libraries".into(), index(jdk, support, &|_| false)),
        ("nothing".into(), index(no_jdk, support, &|_| false)),
    ];
    for pkg in packages.iter().filter(|p| !p.is_empty() && text.contains(p.as_str())) {
        ablations.push((format!("no-package:{pkg}"), index(jdk, support, &|s| &s.package != pkg)));
    }
    for s in support.iter().filter(|s| s.names.iter().any(|n| mentioned.contains(n.as_str()))) {
        ablations.push((format!("no-file:{}", s.rel), index(jdk, support, &|t| t.key != s.key)));
    }

    let runs = ablations.len();
    let mut out = Vec::new();
    for (name, idx) in ablations {
        for ((rule, line), msg) in findings(&text, &stem, &options, idx) {
            if !full.contains_key(&(rule.clone(), line)) {
                out.push(((name.clone(), rule, stem.clone(), line), msg));
            }
        }
    }
    (runs, out)
}

fn render_baseline(entries: &BTreeSet<Entry>) -> String {
    let mut s = String::new();
    s.push_str("// The violations of `tests/ablation.rs` known today: findings that appear\n");
    s.push_str("// only when information is hidden from the analyzer. One entry per\n");
    s.push_str("// (ablation, rule, fixture stem, line), sorted. Delete an entry when a fix\n");
    s.push_str("// removes it; regenerate the whole list with\n");
    s.push_str("// `GROUNDS_ABLATION_BLESS=1 cargo test -p grounds-rules --test ablation`.\n");
    s.push_str("// (cargo also builds this file as a test of its own, with no tests in it.)\n\n");
    s.push_str("#[allow(dead_code)]\n");
    s.push_str("#[rustfmt::skip]\n");
    s.push_str("const BASELINE: &[(&str, &str, &str, usize)] = &[\n");
    for (a, r, f, l) in entries {
        s.push_str(&format!("    ({a:?}, {r:?}, {f:?}, {l}),\n"));
    }
    s.push_str("];\n");
    s
}

#[test]
fn hiding_information_adds_no_finding() {
    let started = std::time::Instant::now();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let java_home = grounds_classfile::find_java_home();
    assert!(java_home.is_some(), "no JDK found: set JAVA_HOME; the `no-jdk` ablation means nothing without one");
    let jdk = Arc::new(Classpath::build(java_home.as_deref(), 21, &[]));
    let no_jdk = Arc::new(Classpath::build(None, 21, &[]));

    let support_dir = root.join("tests/support");
    let mut files = Vec::new();
    java_files(&support_dir, &mut files);
    files.sort();
    let support: Vec<Support> = files
        .into_iter()
        .map(|f| {
            let text = std::fs::read_to_string(&f).unwrap();
            let stub = extract(&ParsedFile::parse(text));
            let rel = f.strip_prefix(&support_dir).unwrap().components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            Support {
                key: Arc::from(f.to_string_lossy().as_ref()),
                package: stub.package.to_string(),
                names: stub.classes.iter().map(|c| c.simple_name().to_string()).collect(),
                stub: Arc::new(stub),
                rel,
            }
        })
        .collect();
    let packages: BTreeSet<String> = support.iter().map(|s| s.package.clone()).collect();

    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(root.join("tests/fixtures")).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "java")).collect();
    fixtures.sort();

    let per_fixture: Vec<(usize, Vec<(Entry, String)>)> = fixtures.iter().map(|fx| violations(fx, &jdk, &no_jdk, &support, &packages)).collect();
    let runs: usize = per_fixture.iter().map(|p| p.0).sum();
    let found: BTreeMap<Entry, String> = per_fixture.into_iter().flat_map(|p| p.1).collect();
    eprintln!("{} fixtures, {runs} ablated runs, {} violations, {:.1?}", fixtures.len(), found.len(), started.elapsed());

    let observed: BTreeSet<Entry> = found.keys().cloned().collect();
    if std::env::var(BLESS).is_ok_and(|v| v == "1") {
        std::fs::write(root.join("tests/ablation_baseline.rs"), render_baseline(&observed)).unwrap();
        eprintln!("wrote tests/ablation_baseline.rs with {} entries", observed.len());
        return;
    }

    let baseline: BTreeSet<Entry> = BASELINE.iter().map(|(a, r, f, l)| (a.to_string(), r.to_string(), f.to_string(), *l)).collect();
    let mut report = String::new();
    let new: Vec<&Entry> = observed.difference(&baseline).collect();
    if !new.is_empty() {
        report.push_str(&format!(
            "\n{} finding(s) appear only when information is hidden, and tests/ablation_baseline.rs does not list them.\n\
             Each is a claim that rests on something being invisible: a false positive wherever that jar or the JDK is not found.\n\
             Make the rule stay silent when what it needs is unknown. Only if the claim truly holds without that information,\n\
             regenerate the baseline with `{BLESS}=1 cargo test -p grounds-rules --test ablation` and say why in the commit.\n",
            new.len()
        ));
        for e in &new {
            let (a, r, f, l) = e;
            report.push_str(&format!("  {r}  {f}.java:{l}  [{a}]  {}\n", found[*e]));
        }
    }
    let stale: Vec<&Entry> = baseline.difference(&observed).collect();
    if !stale.is_empty() {
        report.push_str(&format!(
            "\n{} entr{} of tests/ablation_baseline.rs no longer occur{}: a fix removed {}.\n\
             Delete these lines from tests/ablation_baseline.rs (or regenerate it with `{BLESS}=1 cargo test -p grounds-rules --test ablation`):\n",
            stale.len(),
            if stale.len() == 1 { "y" } else { "ies" },
            if stale.len() == 1 { "s" } else { "" },
            if stale.len() == 1 { "it" } else { "them" }
        ));
        for (a, r, f, l) in &stale {
            report.push_str(&format!("    ({a:?}, {r:?}, {f:?}, {l}),\n"));
        }
    }
    if !new.is_empty() && !stale.is_empty() {
        report.push_str("\nIf a fixture was edited and its lines moved, the same findings show up both as new and as stale: regenerate the baseline.\n");
    }
    assert!(report.is_empty(), "{report}");
}
