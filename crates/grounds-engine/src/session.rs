//! Agent-facing operations on a loaded project: incremental refresh,
//! per-edit checks, checkpoint checks, baselines, pending-refactor detection
//! and impacted tests. The CLI runs these in-process; the daemon keeps a
//! `Session` warm and runs them per request.

use crate::analyze::{analyze_text, build_ruleset, Finding};
use crate::git;
use crate::project::{path_key, Project};
use crate::workspace::{classify, SourceKind};
use grounds_index::{simple_name_of, FileStub, Name};
use grounds_rules::framework::RuleOptions;
use grounds_rules::{RuleSet, Severity, Tier};
use grounds_syntax::ParsedFile;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

pub struct Session {
    pub project: Project,
    pub all_rules: RuleSet,
    pub edit_rules: RuleSet,
    pub resolution_rules: RuleSet,
    pub options: RuleOptions,
    mtimes: FxHashMap<PathBuf, SystemTime>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Baseline {
    pub fingerprints: Vec<String>,
}

impl Baseline {
    pub fn path(root: &Path) -> PathBuf {
        root.join(".grounds/baseline.json")
    }
    pub fn load(root: &Path) -> Baseline {
        std::fs::read_to_string(Self::path(root)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save(&self, root: &Path) -> anyhow::Result<()> {
        let p = Self::path(root);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// A caller broken by a signature change elsewhere.
#[derive(Clone, Debug, Serialize)]
pub struct PendingBreak {
    pub finding: Finding,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
    /// Callers in other files broken by declaration changes in the changed files.
    pub pending: Vec<Finding>,
    /// Files whose declarations changed relative to HEAD.
    pub changed_apis: Vec<String>,
    pub files_checked: usize,
    pub suppressed_by_baseline: usize,
    pub outside_changed_lines: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// After an edit: high-confidence rules, changed lines only.
    Edit,
    /// End of turn: all rules on changed files, changed lines only.
    Checkpoint,
}

pub struct CheckOptions {
    pub mode: Mode,
    /// Explicit files (otherwise git-changed files).
    pub files: Vec<PathBuf>,
    /// Report findings on all lines, not only changed ones.
    pub all_lines: bool,
    pub use_baseline: bool,
    pub min_severity: Severity,
}

impl Session {
    pub fn open(root: &Path) -> anyhow::Result<Session> {
        let project = Project::load(root)?;
        Ok(Self::from_project(project))
    }

    pub fn from_project(project: Project) -> Session {
        let (all_rules, options) = build_ruleset(&project);
        // same configuration as full checks, narrowed to edit-tier rules
        let edit_rules = RuleSet::new(crate::analyze::selected_rules(&project).into_iter().filter(|r| r.meta().tier == Tier::Edit).collect());
        let resolution_rules = RuleSet::new(crate::analyze::selected_rules(&project).into_iter().filter(|r| r.meta().id.starts_with("unresolved-")).collect());
        let mut s = Session { project, all_rules, edit_rules, resolution_rules, options, mtimes: FxHashMap::default() };
        for f in &s.project.workspace.files {
            if let Ok(m) = std::fs::metadata(&f.path).and_then(|m| m.modified()) {
                s.mtimes.insert(f.path.clone(), m);
            }
        }
        s
    }

    pub fn root(&self) -> &Path {
        &self.project.workspace.root
    }

    /// Re-read files whose modification time changed (and the given extra
    /// files, which may be new). Returns files whose declarations changed.
    pub fn sync(&mut self, extra: &[PathBuf]) -> Vec<PathBuf> {
        let mut stale: Vec<PathBuf> = Vec::new();
        for (p, old) in &self.mtimes {
            match std::fs::metadata(p).and_then(|m| m.modified()) {
                Ok(m) if m != *old => stale.push(p.clone()),
                Err(_) => stale.push(p.clone()),
                _ => {}
            }
        }
        for e in extra {
            let e = e.canonicalize().unwrap_or_else(|_| e.clone());
            if e.extension().is_some_and(|x| x == "java") && !self.mtimes.contains_key(&e) {
                stale.push(e.clone());
                let module = self.project.workspace.module_of(&e).unwrap_or(0);
                self.project.workspace.files.push(crate::workspace::SourceFile { path: e.clone(), module, kind: classify(&e) });
            }
        }
        let mut changed = Vec::new();
        let mut any_decl_change = false;
        let mut di_stale = false;
        for p in stale {
            let exists = p.exists();
            // refresh_file rebuilds the DI model itself; batch it below instead
            let key = path_key(&p);
            let old = self.project.index.file(&key).map(|s| s.signature_hash);
            if exists {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    let pf = ParsedFile::parse(text);
                    let stub = Arc::new(self.project.extract(&p, &pf));
                    // install the new stub even when signatures are unchanged:
                    // referenced names (for broken callers) and declaration
                    // positions change with body-only edits
                    let decl_change = old != Some(stub.signature_hash);
                    // the DI model stores positions of beans and injection points
                    di_stale |= decl_change || self.project.di.beans.iter().any(|b| b.file.as_deref() == Some(&*key)) || self.project.di.points.iter().any(|pt| pt.file.as_deref() == Some(&*key));
                    self.project.index.set_file(key, stub);
                    if decl_change {
                        any_decl_change = true;
                        changed.push(p.clone());
                    }
                }
                if let Ok(m) = std::fs::metadata(&p).and_then(|m| m.modified()) {
                    self.mtimes.insert(p.clone(), m);
                }
            } else {
                self.project.index.remove_file(&key);
                self.mtimes.remove(&p);
                any_decl_change = true;
                changed.push(p.clone());
            }
        }
        if any_decl_change || di_stale {
            self.project.rebuild_di();
        }
        changed
    }

    fn target_files(&self, opts: &CheckOptions) -> Vec<PathBuf> {
        let files = if opts.files.is_empty() { git::changed_files(self.root()) } else { opts.files.iter().map(|f| f.canonicalize().unwrap_or_else(|_| f.clone())).collect() };
        files.into_iter().filter(|f| f.extension().is_some_and(|x| x == "java") && f.exists() && classify(f) != SourceKind::Generated).collect()
    }

    fn analyze(&self, rules: &RuleSet, files: &[PathBuf]) -> Vec<Finding> {
        files
            .par_iter()
            .flat_map_iter(|p| match std::fs::read_to_string(p) {
                Ok(text) => analyze_text(&self.project, rules, &self.options, p, &text),
                Err(_) => Vec::new(),
            })
            .collect()
    }

    /// Classes and member names whose declarations differ from HEAD.
    fn api_changes(&self, files: &[PathBuf]) -> (Vec<String>, FxHashSet<Name>) {
        let mut changed_files = Vec::new();
        let mut names: FxHashSet<Name> = FxHashSet::default();
        for f in files {
            let Some(cur) = self.project.index.file(&path_key(f)).cloned() else { continue };
            let head: Option<FileStub> = git::head_version(self.root(), f).map(|t| self.project.extract(f, &ParsedFile::parse(t)));
            let Some(head) = head else { continue }; // new file: nothing to break
            if head.signature_hash == cur.signature_hash {
                continue;
            }
            let rel = f.strip_prefix(self.root()).unwrap_or(f).to_string_lossy().to_string();
            changed_files.push(rel);
            // members present at HEAD that no longer exist with the same shape
            let shape = |s: &FileStub| -> FxHashMap<String, Name> {
                let mut m = FxHashMap::default();
                for c in &s.classes {
                    m.insert(format!("class {}", c.name), Arc::from(simple_name_of(&c.name)));
                    for meth in &c.methods {
                        if meth.flags.is_private() {
                            continue;
                        }
                        let sig = format!("{}.{}({:?})", c.name, meth.name, meth.params.iter().map(|p| &p.ty).collect::<Vec<_>>());
                        let n: Name = if &*meth.name == "<init>" { Arc::from(simple_name_of(&c.name)) } else { meth.name.clone() };
                        m.insert(sig, n);
                    }
                    for fld in &c.fields {
                        if !fld.flags.is_private() {
                            m.insert(format!("{}.{}:{:?}", c.name, fld.name, fld.ty), fld.name.clone());
                        }
                    }
                }
                m
            };
            let (h, c) = (shape(&head), shape(&cur));
            for (k, n) in &h {
                if !c.contains_key(k) {
                    names.insert(n.clone());
                }
            }
        }
        (changed_files, names)
    }

    /// Files (other than `exclude`) that mention any of `names`.
    fn dependents(&self, names: &FxHashSet<Name>, exclude: &FxHashSet<PathBuf>) -> Vec<PathBuf> {
        if names.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (p, stub) in self.project.index.files() {
            let pb = PathBuf::from(&**p);
            if exclude.contains(&pb) || classify(&pb) == SourceKind::Generated {
                continue;
            }
            if names.iter().any(|n| stub.referenced_names.binary_search(n).is_ok()) {
                out.push(pb);
            }
        }
        out.sort();
        out
    }

    pub fn check(&mut self, opts: &CheckOptions) -> Report {
        let files = self.target_files(opts);
        self.sync(&files);
        let rules = match opts.mode {
            Mode::Edit => &self.edit_rules,
            Mode::Checkpoint => &self.all_rules,
        };
        let mut findings = self.analyze(rules, &files);
        let mut report = Report { files_checked: files.len(), ..Default::default() };

        // changed-lines filter
        if !opts.all_lines && git::is_repo(self.root()) {
            let ranges = git::changed_lines(self.root(), &files);
            let before = findings.len();
            findings.retain(|f| {
                let abs = self.root().join(&f.path);
                match ranges.get(&abs) {
                    Some(r) => git::in_ranges(f.line, r),
                    None => true,
                }
            });
            report.outside_changed_lines = before - findings.len();
        }
        let baseline = if opts.use_baseline { Baseline::load(self.root()) } else { Baseline::default() };
        let base: FxHashSet<&str> = baseline.fingerprints.iter().map(|s| s.as_str()).collect();
        let before = findings.len();
        findings.retain(|f| !f.in_baseline(&base));
        report.suppressed_by_baseline = before - findings.len();
        findings.retain(|f| f.severity <= opts.min_severity);
        findings.sort_by(|a, b| (a.severity, &a.path, a.line).cmp(&(b.severity, &b.path, b.line)));
        report.findings = findings;

        // callers broken by API changes in these files
        if git::is_repo(self.root()) {
            let (changed_apis, names) = self.api_changes(&files);
            report.changed_apis = changed_apis;
            let exclude: FxHashSet<PathBuf> = files.iter().cloned().collect();
            let deps = self.dependents(&names, &exclude);
            if !deps.is_empty() {
                let mut pending = self.analyze(&self.resolution_rules, &deps);
                pending.retain(|f| !f.in_baseline(&base));
                pending.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
                report.pending = pending;
            }
        }
        report
    }

    /// Record current findings of the given (or changed) files as accepted.
    pub fn write_baseline(&mut self, files: &[PathBuf], merge: bool) -> anyhow::Result<usize> {
        let opts = CheckOptions { mode: Mode::Checkpoint, files: files.to_vec(), all_lines: true, use_baseline: false, min_severity: Severity::Info };
        let targets = if files.is_empty() && !git::is_repo(self.root()) { crate::analyze::all_source_files(&self.project) } else { self.target_files(&opts) };
        self.sync(&targets);
        let findings = self.analyze(&self.all_rules, &targets);
        let mut b = if merge { Baseline::load(self.root()) } else { Baseline::default() };
        let mut set: FxHashSet<String> = b.fingerprints.drain(..).collect();
        for f in &findings {
            set.insert(f.fingerprint.clone());
        }
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        let n = v.len();
        Baseline { fingerprints: v }.save(self.root())?;
        Ok(n)
    }

    /// Test classes that reference (transitively, through project classes)
    /// any class declared in the changed files.
    pub fn impacted_tests(&mut self, files: &[PathBuf]) -> Vec<ImpactedTest> {
        let opts = CheckOptions { mode: Mode::Checkpoint, files: files.to_vec(), all_lines: true, use_baseline: false, min_severity: Severity::Info };
        let targets = self.target_files(&opts);
        self.sync(&targets);
        // reverse reference closure over simple class names
        let mut frontier: FxHashSet<Name> = FxHashSet::default();
        for f in &targets {
            if let Some(stub) = self.project.index.file(&path_key(f)) {
                for c in &stub.classes {
                    frontier.insert(Arc::from(simple_name_of(&c.name)));
                }
            }
        }
        let mut seen_files: FxHashSet<PathBuf> = targets.iter().cloned().collect();
        let mut tests: Vec<ImpactedTest> = Vec::new();
        for _depth in 0..3 {
            let mut next: FxHashSet<Name> = FxHashSet::default();
            for (p, stub) in self.project.index.files() {
                let pb = PathBuf::from(&**p);
                if seen_files.contains(&pb) {
                    continue;
                }
                if !frontier.iter().any(|n| stub.referenced_names.binary_search(n).is_ok()) {
                    continue;
                }
                seen_files.insert(pb.clone());
                if classify(&pb) == SourceKind::Test {
                    for c in stub.classes.iter().filter(|c| c.outer.is_none()) {
                        tests.push(self.impacted_test(&pb, c.name.to_string()));
                    }
                } else {
                    for c in &stub.classes {
                        next.insert(Arc::from(simple_name_of(&c.name)));
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        // tests among the changed files themselves
        for f in &targets {
            if classify(f) == SourceKind::Test {
                if let Some(stub) = self.project.index.file(&path_key(f)) {
                    for c in stub.classes.iter().filter(|c| c.outer.is_none()) {
                        tests.push(self.impacted_test(f, c.name.to_string()));
                    }
                }
            }
        }
        tests.sort();
        tests.dedup();
        tests
    }

    fn impacted_test(&self, file: &Path, class: String) -> ImpactedTest {
        let ws = &self.project.workspace;
        let module = ws.module_of(file).map(|m| ws.modules[m].name.clone()).unwrap_or_default();
        let set = crate::workspace::source_set_of(file).unwrap_or("test");
        let task = self.project.source_sets.test_task(&module, set);
        ImpactedTest { module, task, class }
    }
}

/// A test class reaching changed code, with the Gradle test task running it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ImpactedTest {
    pub module: String,
    pub task: String,
    pub class: String,
}

/// Render a report for an agent: new problems first, then broken callers,
/// capped so a large refactor does not flood the context.
pub fn render(report: &Report, max: usize, style: crate::report::Style) -> String {
    let mut out = String::new();
    let e = report.findings.iter().filter(|f| f.severity == Severity::Error).count();
    let w = report.findings.iter().filter(|f| f.severity == Severity::Warning).count();
    let i = report.findings.len() - e - w;
    if !report.findings.is_empty() {
        out.push_str(&style.bold(&format!("grounds: {} new problem(s) in changed code ({e} errors, {w} warnings, {i} info)", report.findings.len())));
        out.push('\n');
        for f in report.findings.iter().take(max) {
            out.push_str(&style.finding(f));
        }
        if report.findings.len() > max {
            let mut by: std::collections::BTreeMap<&str, usize> = Default::default();
            for f in &report.findings[max..] {
                *by.entry(f.rule).or_default() += 1;
            }
            let s: Vec<String> = by.iter().map(|(r, n)| format!("{r} x{n}")).collect();
            out.push_str(&format!("... {} more: {}\n", report.findings.len() - max, s.join(", ")));
        }
    }
    if !report.pending.is_empty() {
        let files: FxHashSet<&str> = report.pending.iter().map(|f| f.path.as_str()).collect();
        out.push_str(&style.bold(&format!(
            "grounds: API changes in {} broke {} call site(s) in {} other file(s) not yet updated:",
            report.changed_apis.join(", "),
            report.pending.len(),
            files.len()
        )));
        out.push('\n');
        for f in report.pending.iter().take(max) {
            out.push_str(&format!("{}:{}:{}: {}\n", f.path, f.line, f.col, f.message));
        }
        if report.pending.len() > max {
            out.push_str(&format!("... {} more call sites\n", report.pending.len() - max));
        }
    }
    out
}
