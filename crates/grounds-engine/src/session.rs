//! Agent-facing operations on a loaded project: incremental refresh,
//! per-edit checks, checkpoint checks, baselines, pending-refactor detection
//! and impacted tests. The CLI runs these in-process; the daemon keeps a
//! `Session` warm and runs them per request.

use crate::analyze::{analyze_text, build_ruleset, Finding};
use crate::git;
use crate::project::{path_key, Project};
use crate::workspace::{classify, SourceKind};
use grounds_index::{simple_name_of, ClassLookup, FileStub, Name};
use grounds_rules::framework::RuleOptions;
use grounds_rules::{RuleSet, Severity, Tier};
use grounds_syntax::{NodeExt, ParsedFile};
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
    /// Classpath warnings already shown in this session.
    warned: bool,
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
    /// One finding per changed line that broke code elsewhere (`pending`),
    /// so the break shows in a diff view.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub breaks: Vec<Break>,
    /// Files whose declarations changed relative to HEAD.
    pub changed_apis: Vec<String>,
    pub files_checked: usize,
    pub suppressed_by_baseline: usize,
    pub outside_changed_lines: usize,
}

/// A changed line in the diff that broke code in other files.
#[derive(Clone, Debug, Serialize)]
pub struct Break {
    pub finding: Finding,
    /// Indexes into `Report::pending`.
    pub broken: Vec<usize>,
}

/// Rules whose findings in one file can be caused by a change in another:
/// a removed method, a changed supertype, a removed or duplicated bean.
pub const CROSS_FILE_RULES: &[&str] = &[
    "unresolved-method",
    "unresolved-constructor",
    "unresolved-field",
    "override-matches-nothing",
    "unimplemented-abstract-method",
    "bean-not-found",
    "ambiguous-bean",
    "qualifier-not-found",
    "bean-name-conflict",
    "circular-constructor-dependency",
];

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
    /// Compare with this commit instead of HEAD (the merge base for
    /// `--base`): changed files and lines are the branch's, and breaks in
    /// unchanged files are found by analyzing them against both versions.
    pub base: Option<String>,
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
        let mut s = Session { project, all_rules, edit_rules, resolution_rules, options, mtimes: FxHashMap::default(), warned: false };
        for f in &s.project.workspace.files {
            if let Ok(m) = std::fs::metadata(&f.path).and_then(|m| m.modified()) {
                s.mtimes.insert(f.path.clone(), m);
            }
        }
        s
    }

    /// Classpath warnings, the first time they are asked for: agents and
    /// hooks run checks over and over, one mention is enough.
    pub fn take_warnings(&mut self) -> Vec<String> {
        if std::mem::replace(&mut self.warned, true) {
            return Vec::new();
        }
        self.project.classpath_warnings.iter().map(|w| w.to_string()).collect()
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
        self.targets(opts).0
    }

    /// Java files to check, and Java files deleted since `opts.base`.
    fn targets(&self, opts: &CheckOptions) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let (files, deleted) = match (&opts.base, opts.files.is_empty()) {
            (Some(rev), true) => git::changed_files_since(self.root(), rev),
            (None, true) => (git::changed_files(self.root()), Vec::new()),
            (_, false) => (opts.files.iter().map(|f| f.canonicalize().unwrap_or_else(|_| f.clone())).collect(), Vec::new()),
        };
        // git lists the whole repository; the project may be a subdirectory
        let root = self.root().canonicalize().unwrap_or_else(|_| self.root().to_path_buf());
        let java = |f: &PathBuf| f.extension().is_some_and(|x| x == "java") && classify(f) != SourceKind::Generated && (!opts.files.is_empty() || f.starts_with(&root) || f.starts_with(self.root()));
        (files.into_iter().filter(|f| java(f) && f.exists()).collect(), deleted.into_iter().filter(|f| java(f)).collect())
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

    /// Classes and member names whose declarations differ from `rev`.
    fn api_changes(&self, files: &[PathBuf], rev: &str) -> (Vec<String>, FxHashSet<Name>) {
        let mut changed_files = Vec::new();
        let mut names: FxHashSet<Name> = FxHashSet::default();
        for f in files {
            let Some(cur) = self.project.index.file(&path_key(f)).cloned() else { continue };
            let head: Option<FileStub> = git::version_at(self.root(), rev, f).map(|t| self.project.extract(f, &ParsedFile::parse(t)));
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
        let rev = opts.base.as_deref().unwrap_or("HEAD");
        let (files, deleted) = self.targets(opts);
        self.sync(&files);
        let rules = match opts.mode {
            Mode::Edit => &self.edit_rules,
            Mode::Checkpoint => &self.all_rules,
        };
        let mut findings = self.analyze(rules, &files);
        let mut report = Report { files_checked: files.len(), ..Default::default() };

        // changed-lines filter
        if !opts.all_lines && git::is_repo(self.root()) {
            // keyed by resolved path: git reports `/private/var/..` for a root
            // reached through the `/var` symlink
            let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            let ranges: FxHashMap<PathBuf, Option<Vec<(u32, u32)>>> = git::changed_lines_since(self.root(), &files, rev).into_iter().map(|(p, r)| (canon(&p), r)).collect();
            let before = findings.len();
            findings.retain(|f| match ranges.get(&canon(&self.root().join(&f.path))) {
                Some(r) => git::in_ranges(f.line, r),
                None => true,
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
            let (changed_apis, names) = self.api_changes(&files, rev);
            report.changed_apis = changed_apis;
            let mut pending = if opts.base.is_some() {
                // CI: every unchanged file, against both versions of the changed ones
                self.cross_file_breaks(&files, &deleted, rev)
            } else {
                // per turn: quick, only unresolved references to changed names
                let exclude: FxHashSet<PathBuf> = files.iter().cloned().collect();
                let deps = self.dependents(&names, &exclude);
                if deps.is_empty() { Vec::new() } else { self.analyze(&self.resolution_rules, &deps) }
            };
            pending.retain(|f| !f.in_baseline(&base));
            pending.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
            report.pending = pending;
            report.breaks = self.attribute_breaks(&report.pending, &files, rev);
        }
        report
    }

    /// Findings of the cross-file rules in files outside `changed`/`deleted`
    /// that the change introduced: analyzed with the current version of the
    /// changed files and again with their version at `rev`, keeping what is
    /// new. The unchanged files' text is the same both times, so fingerprints
    /// match exactly.
    fn cross_file_breaks(&mut self, changed: &[PathBuf], deleted: &[PathBuf], rev: &str) -> Vec<Finding> {
        let rules = RuleSet::new(crate::analyze::selected_rules(&self.project).into_iter().filter(|r| CROSS_FILE_RULES.contains(&r.meta().id)).collect());
        let touched: Vec<PathBuf> = changed.iter().chain(deleted).cloned().collect();
        let skip: FxHashSet<&PathBuf> = touched.iter().collect();
        let others: Vec<PathBuf> = crate::analyze::all_source_files(&self.project).into_iter().filter(|f| !skip.contains(f)).collect();
        if touched.is_empty() || others.is_empty() {
            return Vec::new();
        }
        let now = self.analyze(&rules, &others);
        let install = |s: &mut Session, f: &PathBuf, text: Option<String>| {
            let key = path_key(f);
            match text {
                Some(t) => {
                    let stub = Arc::new(s.project.extract(f, &ParsedFile::parse(t)));
                    s.project.index.set_file(key, stub);
                }
                None => s.project.index.remove_file(&key),
            }
        };
        let mut base_classes: Vec<Name> = Vec::new();
        for f in &touched {
            let old = git::version_at(self.root(), rev, f);
            install(self, f, old);
            if let Some(stub) = self.project.index.file(&path_key(f)) {
                base_classes.extend(stub.classes.iter().map(|c| c.name.clone()));
            }
        }
        self.project.rebuild_di();
        // the fingerprint names the line, not the call on it: `a.foo(); a.bar();`
        // breaking foo while fixing bar keeps the fingerprint, not the column
        let before: FxHashSet<(String, u32)> = self.analyze(&rules, &others).into_iter().map(|f| (f.fingerprint, f.col)).collect();
        for f in &touched {
            let cur = std::fs::read_to_string(f).ok();
            install(self, f, cur);
        }
        self.project.rebuild_di();
        let mut found: Vec<Finding> = now.into_iter().filter(|f| !before.contains(&(f.fingerprint.clone(), f.col))).collect();
        // classes the change deleted or renamed: references to them are unknown
        // types, which the rules leave alone (a missing jar looks the same)
        let removed: Vec<Name> = base_classes.into_iter().filter(|c| self.project.index.class(c).is_none()).collect();
        if !removed.is_empty() {
            found.extend(others.par_iter().flat_map_iter(|f| self.removed_class_uses(f, &removed)).collect::<Vec<_>>());
        }
        found
    }

    /// Uses of `removed` classes in `file`: an import of one, or its simple
    /// name in the same package or under an on-demand import of its package.
    fn removed_class_uses(&self, file: &Path, removed: &[Name]) -> Vec<Finding> {
        let Some(stub) = self.project.index.file(&path_key(file)) else { return Vec::new() };
        // how the file names a removed class: its import, its simple name,
        // or its qualified name
        enum Use {
            Import(String),
            Simple,
            Qualified(String),
        }
        // an import other than of `pkg` that may supply `name`: one naming it,
        // or an on-demand one (nested types under a static one) we cannot see
        // all of; only the JDK defines `java.*`, so those are complete once the
        // JDK's classes are loaded at all
        let idx = &self.project.index;
        let jdk = idx.class_exists("java.lang.Object");
        let may_supply = |name: &str, pkg: &str| {
            stub.imports.iter().any(|i| {
                if !i.on_demand {
                    return i.last() == name && (i.is_static || *i.path != *format!("{pkg}.{name}"));
                }
                *i.path != *pkg && (!jdk || !i.path.starts_with("java.") || idx.class_exists(&format!("{}.{name}", i.path)) || idx.class_exists(&format!("{}${name}", i.path)))
            })
        };
        // ordered: the first of them found in the file is reported
        let mut hits: Vec<(Name, Vec<Use>)> = Vec::new();
        for c in removed {
            let simple = simple_name_of(c);
            if stub.referenced_names.binary_search_by(|n| (**n).cmp(simple)).is_err() {
                continue;
            }
            let dotted = c.replace('$', ".");
            let pkg = dotted.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
            let top_pkg = c.split('$').next().unwrap_or(c).rsplit_once('.').map(|(p, _)| p).unwrap_or("");
            let exact = stub.imports.iter().any(|i| !i.is_static && !i.on_demand && *i.path == *dotted);
            let reachable = !c.contains('$') && (&*stub.package == top_pkg || stub.imports.iter().any(|i| i.on_demand && !i.is_static && *i.path == *pkg));
            let mut uses = Vec::new();
            if exact {
                uses.push(Use::Import(dotted.clone()));
            }
            // another import, or a class of that name it brings, wins now
            if reachable && !may_supply(simple, pkg) {
                uses.push(Use::Simple);
            }
            // `p.Gone`: `p` must not be able to name anything but the package
            let first = dotted.split('.').next().unwrap_or("");
            if !may_supply(first, &stub.package) {
                uses.push(Use::Qualified(dotted.clone()));
            }
            if !uses.is_empty() {
                hits.push((c.clone(), uses));
            }
        }
        if hits.is_empty() {
            return Vec::new();
        }
        let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
        let pf = ParsedFile::parse(text);
        let fm = grounds_semantic::FileModel::new(&pf, &self.project.index, path_key(file));
        let rel = file.strip_prefix(self.root()).unwrap_or(file).to_string_lossy().to_string();
        let mut out = Vec::new();
        // a variable named like the qualifier may come from a supertype we
        // cannot see
        fn enclosing_complete<'t>(fm: &grounds_semantic::FileModel<'t>, n: grounds_syntax::Node<'t>) -> bool {
            let mut cur = fm.enclosing_class_node(n);
            while let Some(c) = cur {
                if !fm.class_name_of_decl(c).is_some_and(|cn| fm.hierarchy_complete(cn)) {
                    return false;
                }
                cur = fm.enclosing_class_node(c);
            }
            true
        }
        for (c, uses) in hits {
            let simple = simple_name_of(&c);
            // the import line (importing a class that exists nowhere does not
            // compile), or else the first use of the bare name that resolves to
            // nothing now: a nested class, a field or variable of that name, a
            // qualified `q.Gone` or java.lang taking over is not the removed class
            let node = uses.iter().find_map(|how| pf.root().descendants().find(|n| match how {
                Use::Import(path) => n.kind() == "import_declaration" && pf.text[n.byte_range()].contains(path.as_str()),
                // `p.Gone` as a type, or as the receiver of a call or field,
                // that resolves to nothing now and does not start with a variable
                Use::Qualified(dotted) => {
                    if !matches!(n.kind(), "scoped_type_identifier" | "scoped_identifier" | "field_access") || n.ancestors().any(|a| matches!(a.kind(), "import_declaration" | "package_declaration")) {
                        return false;
                    }
                    let text: String = pf.text[n.byte_range()].chars().filter(|c| !c.is_whitespace()).collect();
                    if text != *dotted {
                        return false;
                    }
                    let mut first = *n;
                    while let Some(k) = first.kids().first().copied() {
                        first = k;
                    }
                    // the qualifier names no type either (`class p` with a nested
                    // or inherited `Gone`)
                    let qualifier_free = || fm.resolve_type_node(first).is_unknown() && !idx.class_exists(&format!("{}.{}", stub.package, &pf.text[first.byte_range()]));
                    if n.kind() == "scoped_type_identifier" {
                        return fm.resolve_type_node(*n).is_unknown() && qualifier_free() && enclosing_complete(&fm, *n);
                    }
                    let receiver = n.parent().is_some_and(|p| matches!(p.kind(), "method_invocation" | "field_access") && p.field("object") == Some(*n));
                    receiver && first.kind() == "identifier" && fm.symbol(first).is_none() && qualifier_free() && fm.type_of(*n).is_unknown() && enclosing_complete(&fm, *n)
                }
                Use::Simple => {
                    if &pf.text[n.byte_range()] != simple || n.ancestors().any(|a| matches!(a.kind(), "import_declaration" | "package_declaration")) {
                        return false;
                    }
                    let qualified = n.parent().is_some_and(|p| matches!(p.kind(), "scoped_type_identifier" | "scoped_identifier") && p.kids().first() != Some(n));
                    match n.kind() {
                        "type_identifier" => !qualified && fm.resolve_type_node(*n).is_unknown(),
                        // `Gone.value()` / `Gone.FIELD`: the receiver names no variable or type
                        "identifier" => {
                            let receiver = n.parent().is_some_and(|p| matches!(p.kind(), "method_invocation" | "field_access") && p.field("object") == Some(*n));
                            receiver && fm.symbol(*n).is_none() && fm.type_of(*n).is_unknown()
                        }
                        _ => false,
                    }
                }
            }));
            let Some(node) = node else { continue };
            let (s, e) = (pf.lines.line_col(&pf.text, node.start_byte() as u32), pf.lines.line_col(&pf.text, node.end_byte() as u32));
            let dotted = c.replace('$', ".");
            let fingerprint = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(format!("removed-class-still-used|{rel}|{dotted}").as_bytes()));
            out.push(Finding {
                path: rel.clone(),
                line: s.line,
                col: s.col,
                end_line: e.line,
                end_col: e.col,
                rule: "removed-class-still-used",
                severity: Severity::Error,
                category: "bug",
                message: format!("`{dotted}` was removed or renamed by this change but is still used here"),
                fingerprint,
                legacy_fingerprint: None,
                fix: None,
                start: node.start_byte() as u32,
                end: node.end_byte() as u32,
            });
        }
        out
    }

    /// Anchor each broken place in `pending` on the changed line that caused
    /// it: a declaration named in its message that was changed or removed
    /// (mapped through the diff), one that was added, or else the first change
    /// in a file declaring a class it names. Unattributable breaks (say, a
    /// deleted file) are left out; they are still in `pending`.
    fn attribute_breaks(&self, pending: &[Finding], changed: &[PathBuf], rev: &str) -> Vec<Break> {
        if pending.is_empty() || changed.is_empty() {
            return Vec::new();
        }
        let hunks = git::hunks_since(self.root(), changed, rev);
        // (file, simple name, declaring class for members, line in the current version)
        let mut removed: Vec<(&PathBuf, String, Option<String>, u32)> = Vec::new();
        let mut added: Vec<(&PathBuf, String, Option<String>, u32)> = Vec::new();
        let mut first: FxHashMap<&PathBuf, u32> = FxHashMap::default();
        let mut classes: Vec<(&PathBuf, String)> = Vec::new();
        let decls = |stub: &FileStub, pf: &ParsedFile| -> Vec<(String, Option<String>, u32)> {
            let line = |(a, _): (u32, u32)| pf.lines.line_of(a) as u32 + 1;
            let mut out = Vec::new();
            for c in &stub.classes {
                let owner = simple_name_of(&c.name).to_string();
                out.push((owner.clone(), None, line(c.range)));
                // a new bean is blamed for "multiple beans of type `Api`" through
                // its supertypes
                for sup in c.superclass.iter().chain(&c.interfaces) {
                    if let Some(n) = sup.last_name() {
                        out.push((n.to_string(), None, line(c.range)));
                    }
                }
                for m in &c.methods {
                    let n = if &*m.name == "<init>" { owner.clone() } else { m.name.to_string() };
                    out.push((n, Some(owner.clone()), line(m.range)));
                }
                for fl in &c.fields {
                    out.push((fl.name.to_string(), Some(owner.clone()), line(fl.range)));
                }
            }
            out
        };
        for f in changed {
            let Some(Some(h)) = hunks.get(f) else { continue };
            let Some(start) = h.iter().map(|x| x.new_start.max(1)).min() else { continue };
            first.insert(f, start);
            if let Some(text) = git::version_at(self.root(), rev, f) {
                let pf = ParsedFile::parse(text);
                let stub = self.project.extract(f, &pf);
                for (n, owner, l) in decls(&stub, &pf) {
                    if owner.is_none() {
                        classes.push((f, n.clone()));
                    }
                    if let Some(now) = git::Hunk::new_line_for(h, l) {
                        removed.push((f, n, owner, now));
                    }
                }
            }
            if let (Some(stub), Ok(text)) = (self.project.index.file(&path_key(f)), std::fs::read_to_string(f)) {
                let pf = ParsedFile::parse(text);
                for (n, owner, l) in decls(stub, &pf) {
                    if owner.is_none() {
                        classes.push((f, n.clone()));
                    }
                    if h.iter().any(|x| x.new_count > 0 && l >= x.new_start && l < x.new_start + x.new_count) {
                        added.push((f, n, owner, l));
                    }
                }
            }
        }
        // names in a message: the identifiers inside `backticks`
        let names_of = |m: &str| -> FxHashSet<String> {
            m.split('`').skip(1).step_by(2).flat_map(|q| q.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).filter(|w| !w.is_empty()).map(str::to_string).collect::<Vec<_>>()).collect()
        };
        let mut groups: std::collections::BTreeMap<(String, u32), Vec<usize>> = Default::default();
        for (i, p) in pending.iter().enumerate() {
            let names = names_of(&p.message);
            // a member of a class the message names (`B` has no method `foo`) before
            // a same-named member elsewhere, then classes
            let pick = |v: &[(&PathBuf, String, Option<String>, u32)], want: fn(&Option<String>, &FxHashSet<String>) -> bool| {
                v.iter().find(|(_, n, o, _)| names.contains(n) && want(o, &names)).map(|(f, _, _, l)| ((*f).clone(), *l))
            };
            let owned: fn(&Option<String>, &FxHashSet<String>) -> bool = |o, names| o.as_ref().is_some_and(|o| names.contains(o));
            let member: fn(&Option<String>, &FxHashSet<String>) -> bool = |o, _| o.is_some();
            let class: fn(&Option<String>, &FxHashSet<String>) -> bool = |o, _| o.is_none();
            let anchor = pick(&removed, owned)
                .or_else(|| pick(&added, owned))
                .or_else(|| pick(&removed, member))
                .or_else(|| pick(&added, member))
                .or_else(|| pick(&removed, class))
                .or_else(|| pick(&added, class))
                .or_else(|| classes.iter().find(|(_, n)| names.contains(n)).and_then(|(f, _)| first.get(f).map(|l| ((*f).clone(), *l))));
            if let Some((f, l)) = anchor {
                let rel = f.strip_prefix(self.root()).unwrap_or(&f).to_string_lossy().to_string();
                groups.entry((rel, l)).or_default().push(i);
            }
        }
        groups
            .into_iter()
            .map(|((path, line), broken)| {
                // file names: the related locations carry the full paths
                let shown: Vec<String> = broken.iter().take(5).map(|&i| format!("{}:{}", pending[i].path.rsplit('/').next().unwrap_or(&pending[i].path), pending[i].line)).collect();
                let more = if broken.len() > 5 { format!(" (+{} more)", broken.len() - 5) } else { String::new() };
                let severity = broken.iter().map(|&i| pending[i].severity).min().unwrap_or(Severity::Error);
                let message = format!("This change breaks {} place(s) in other files: {}{more}", broken.len(), shown.join(", "));
                let fingerprint = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(format!("breaks-other-files|{path}|{}", broken.iter().map(|&i| pending[i].fingerprint.as_str()).collect::<Vec<_>>().join(",")).as_bytes()));
                let finding = Finding { path, line, col: 1, end_line: line, end_col: 1, rule: "breaks-other-files", severity, category: "bug", message, fingerprint, legacy_fingerprint: None, fix: None, start: 0, end: 0 };
                Break { finding, broken }
            })
            .collect()
    }

    /// Record current findings of the given (or changed) files as accepted.
    pub fn write_baseline(&mut self, files: &[PathBuf], merge: bool) -> anyhow::Result<usize> {
        let opts = CheckOptions { mode: Mode::Checkpoint, files: files.to_vec(), all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None };
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
        let opts = CheckOptions { mode: Mode::Checkpoint, files: files.to_vec(), all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None };
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
        // a deleted bean or a changed supertype breaks things too, not only
        // changed member signatures
        let header = if report.changed_apis.is_empty() {
            format!("grounds: changes broke {} place(s) in {} other file(s) not yet updated:", report.pending.len(), files.len())
        } else {
            format!("grounds: API changes in {} broke {} place(s) in {} other file(s) not yet updated:", report.changed_apis.join(", "), report.pending.len(), files.len())
        };
        out.push_str(&style.bold(&header));
        out.push('\n');
        for f in report.pending.iter().take(max) {
            out.push_str(&format!("{}:{}:{}: {}\n", f.path, f.line, f.col, f.message));
        }
        if report.pending.len() > max {
            out.push_str(&format!("... {} more\n", report.pending.len() - max));
        }
    }
    out
}
