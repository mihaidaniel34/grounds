//! Agent-facing operations on a loaded project: incremental refresh,
//! per-edit checks, checkpoint checks, baselines, pending-refactor detection
//! and impacted tests. The CLI runs these in-process; the daemon keeps a
//! `Session` warm and runs them per request.

use crate::analyze::{analyze_text, build_ruleset_for, Finding};
use crate::git;
use crate::project::{path_key, recent, FileStamp, Project, Walk};
use crate::workspace::{classify, SourceKind};
use grounds_index::{simple_name_of, ClassLookup, FileStub, Name};
use grounds_rules::framework::RuleOptions;
use grounds_rules::{Maturity, RuleSet, Severity, Tier};
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
    /// The source files as their stubs in the index were made from.
    stamps: FxHashMap<PathBuf, FileStamp>,
    /// The directories the source files were found in, and the ignore files
    /// that applied: a source file that appears is noticed by the time of
    /// its directory, whichever way it came (see `rescan`).
    walked: Walk,
    /// Build and configuration files the project was loaded from (and the
    /// places one may appear), as the load read them: `None` for one that
    /// was not there.
    config: Vec<(PathBuf, Option<FileStamp>)>,
    /// Which version of a configuration file the load read is not known (its
    /// time is that of a write during the load, or of one just before it,
    /// with no stamp from before the load to compare): load again.
    config_unread: bool,
    /// Classpath warnings already shown in this session.
    warned: bool,
    /// Full walks of the workspace by `rescan`.
    full_walks: usize,
}

/// What a project is loaded from besides its Java sources: the files that
/// decide its modules, classpath, settings and beans.
/// - build files and `lombok.config` of the root and of every module, and
///   Gradle's convention scripts (`buildSrc`, `build-logic`);
/// - `grounds.toml`, the classpath dumps, the source sets a dump recorded and
///   every jar on the classpath (a snapshot jar is rebuilt in place);
/// - per module, Spring's registration files and `application*` settings,
///   and the directories they sit in, whose own times change when a file is
///   added or removed.
///
/// Listed whether they exist or not, so that one appearing counts as a
/// change. Not followed: class directories on the classpath (a build writes
/// into them without touching the directory itself, and the project's own
/// classes are read from source anyway), and a `lombok.config` created later
/// in a package directory that had none.
fn config_files(project: &Project) -> Vec<PathBuf> {
    let ws = &project.workspace;
    let cfg = &project.config;
    let at_root = |p: &PathBuf| if p.is_absolute() { p.clone() } else { ws.root.join(p) };
    let mut paths: Vec<PathBuf> = ["grounds.toml", ".grounds/config.toml", ".grounds/classpath.txt", ".grounds/source-sets.txt"].iter().map(|n| ws.root.join(n)).collect();
    paths.extend(cfg.classpath.files.iter().chain(&cfg.classpath.entries).map(at_root));
    // the jars in use, and those a dump lists that are not there (yet)
    let listed = crate::classpath::classpath_files(ws, cfg).into_iter().flat_map(|f| crate::classpath::read_list(&f));
    paths.extend(project.classpath_entries.iter().cloned().chain(listed).filter(|e| !e.is_dir()));
    let mut dirs: Vec<&Path> = vec![&ws.root];
    dirs.extend(ws.modules.iter().map(|m| m.dir.as_path()));
    for d in dirs {
        paths.extend(crate::classpath::BUILD_FILES.iter().chain(&["lombok.config"]).map(|n| d.join(n)));
    }
    paths.extend(project.lombok.files());
    // Gradle convention plugins: the release they set is every module's
    paths.extend(crate::java_release::convention_files(&ws.root));
    for m in &ws.modules {
        let resources = m.dir.join("src/main/resources");
        let meta = resources.join("META-INF");
        let listed = |dir: &Path, keep: &dyn Fn(&str) -> bool| -> Vec<PathBuf> {
            std::fs::read_dir(dir).into_iter().flatten().flatten().filter(|e| keep(&e.file_name().to_string_lossy())).map(|e| e.path()).collect()
        };
        paths.extend(listed(&resources, &|n| n.starts_with("application")));
        paths.extend(listed(&meta.join("spring"), &|n| n.ends_with(".imports")));
        paths.extend([meta.join("spring.factories"), meta.join("spring"), meta, resources]);
    }
    paths.sort();
    paths.dedup();
    paths
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

/// Whether `rule` is a compile-break rule whose findings `grounds changed`
/// holds to the stable promise when they are new with the change
/// (`grounds_rules::STABLE_IN_CHANGED`).
pub fn stable_in_changed(rule: &str) -> bool {
    grounds_rules::STABLE_IN_CHANGED.contains(&rule)
}

/// Whether `rule` claims that code would not compile or an application would
/// not start, so that `grounds changed` reports its findings on changed lines
/// only when the project before the change did not get the same claim. That
/// comparison only removes reports, so it runs for the wiring rules too; they
/// are not stable (their claims depend on the order of maps and annotations
/// in the wiring model), so their findings stay preview.
pub fn compared_in_changed(rule: &str) -> bool {
    stable_in_changed(rule) || CROSS_FILE_RULES.contains(&rule)
}

/// What a compile-break or wiring finding claims, independent of where it
/// is: the rule and the assertion. The fingerprint is no use here: it hashes
/// the text of the line, which an edit to that line changes. The rules hand
/// the assertion over on the diagnostic (`Diagnostic::claim`), with
/// qualified names where the message shows simple ones (`a.Api` and `b.Api`
/// are different claims) and whole sets where the message cuts them. Per rule,
/// what is asserted (kept, sets sorted) and what is presentation (left out):
///
/// | rule | asserted | presentation |
/// |---|---|---|
/// | `unresolved-method` | qualified receiver class, member, arity or argument types; or the qualified method that is private, or an instance method called on the type | `; did you mean ...?`, `; available: <overloads>` |
/// | `unresolved-constructor` | qualified class and arity | `; available: <constructors>` (first 4) |
/// | `unresolved-field` | qualified class and field | none |
/// | `override-matches-nothing` | the qualified class and the method's erased, qualified signature | none |
/// | `missing-jdk-import` | the qualified import and the release | none |
/// | `unimplemented-abstract-method` | the qualified class and every missing method (qualified owner and signature), sorted | the first 3 shown and "and N more" |
/// | `bean-not-found` | the qualified type, the injection point and its qualified owner | none |
/// | `ambiguous-bean` | qualified type, injection point and owner, and every candidate (name, qualified class, factory method), sorted | their count, the first 5 shown, `; add a @Qualifier or @Primary` |
/// | `qualifier-not-found` | the qualified type and the qualifier | `; available: <beans>` (first 5) |
/// | `bean-name-conflict` | the bean name, both definitions (qualified) and the exception | none |
/// | `circular-constructor-dependency` | the message: the cycle as the search meets it | none |
///
/// The last five (wiring) rows are compared too, but those rules are not
/// stable: which pair, qualifier or cycle they report depends on the order of
/// maps and annotations in the wiring model, so their findings stay preview.
/// Without a claim on the diagnostic (`missing-jdk-import`, whose message is
/// already qualified, and the cycle rule), the message less its recognised
/// presentation suffixes is the claim.
fn claim_key(f: &Finding) -> (&'static str, String) {
    if let Some(c) = &f.claim {
        return (f.rule, c.clone());
    }
    let mut m = f.message.as_str();
    let presentation: &[&str] = match f.rule {
        "unresolved-method" => &["; did you mean `", "; available: "],
        "unresolved-constructor" | "qualifier-not-found" => &["; available: "],
        _ => &[],
    };
    for p in presentation {
        if let Some(i) = m.find(p) {
            m = &m[..i];
        }
    }
    (f.rule, m.to_string())
}

/// The gap in the model a claim stands for, where that is less than the
/// claim: a decision for limit (b), not for comparing places. A missing or
/// ambiguous bean is a fact about the type (and its candidates), whichever
/// injection point asks for it; when untouched code that ran before the
/// change already got it, a new injection point of the same type meets the
/// same gap, so it is not reported either. For every other rule the claim
/// itself names no place, so it is its own gap.
fn gap_key(f: &Finding) -> Option<(&'static str, String)> {
    match f.rule {
        // claim: "No bean of type `a.T`; injection of `p` into `a.Owner`"
        "bean-not-found" => f.claim.as_deref().and_then(|c| c.split_once("; injection of ")).map(|(t, _)| (f.rule, t.to_string())),
        // claim: "beans of type `a.T` match `p` into `a.Owner`: <candidates>"
        "ambiguous-bean" => f.claim.as_deref().and_then(|c| {
            let (ty, rest) = c.split_once(" match `")?;
            let (_, candidates) = rest.split_once("`: ")?;
            Some((f.rule, format!("{ty}: {candidates}")))
        }),
        _ => None,
    }
}

/// A finding in a file the change did not touch, as the comparison with the
/// project before the change sees it: the place and what is claimed there.
/// The fingerprint names the line, not the call on it (`a.foo(); a.bar();`
/// breaking foo while fixing bar keeps the fingerprint, not the column), and
/// the same place can carry a different claim after the change (a changed
/// return type: "`Old` has no method `x`" becomes "`New` has no method `x`").
type PlaceClaim = (String, u32, (&'static str, String));

fn place_claim(f: &Finding) -> PlaceClaim {
    (f.fingerprint.clone(), f.col, claim_key(f))
}

/// The identifiers a message quotes in `backticks`, before any `;` (the
/// hint or list of candidates that follows).
fn quoted_names(message: &str) -> Vec<String> {
    let head = message.split(';').next().unwrap_or(message);
    let mut v: Vec<String> = head
        .split('`')
        .skip(1)
        .step_by(2)
        .flat_map(|q| q.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).filter(|w| !w.is_empty() && !w.chars().next().is_some_and(|c| c.is_ascii_digit())).map(str::to_string).collect::<Vec<_>>())
        .collect();
    v.sort();
    v.dedup();
    v
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
    /// Compare with this commit instead of HEAD (the merge base for
    /// `--base`): changed files and lines are the branch's, and breaks in
    /// unchanged files are found by analyzing them against both versions.
    pub base: Option<String>,
}

impl Session {
    pub fn open(root: &Path) -> anyhow::Result<Session> {
        crate::trace::log(|| format!("load {}: start", root.display()));
        let project = Project::load(root)?;
        crate::trace::log(|| format!("load {}: done", root.display()));
        Ok(Self::from_project(project))
    }

    pub fn from_project(project: Project) -> Session {
        Self::from_load(project, None)
    }

    /// A session of a project just loaded; `before`: stamps of the
    /// configuration files taken before the load began, if it was a reload.
    fn from_load(mut project: Project, before: Option<&FxHashMap<PathBuf, Option<FileStamp>>>) -> Session {
        // the rules `grounds changed` runs: there the compile-break
        // rules are stable, so `preview = false` keeps them
        let (all_rules, options) = build_ruleset_for(&project, true);
        // same configuration as full checks, narrowed to edit-tier rules
        let edit_rules = RuleSet::new(crate::analyze::selected_rules_for(&project, true).into_iter().filter(|r| r.meta().tier == Tier::Edit).collect());
        let resolution_rules = RuleSet::new(crate::analyze::selected_rules_for(&project, true).into_iter().filter(|r| r.meta().id.starts_with("unresolved-")).collect());
        // Source files: stamped as the load read them, each before its read.
        let stamps = std::mem::take(&mut project.stamps);
        let walked = std::mem::take(&mut project.walked);
        // Configuration files are read all over the load and are only listed
        // after it, so they are stamped now. One the same now as before the
        // load (time, size and, for a recent time, content) is what the load
        // read. One not stamped before (the first load, or one the load
        // newly lists) whose time is recent as of the start of the load may
        // have been written while the load ran: which version it read is not
        // known, and the project is loaded again at the next sync, that time
        // with a stamp from before.
        //
        // Known limit: equal stamps say what the file was before and after
        // the load, not what the load read in between. A file written A, B
        // and A again (the same size) within one tick of a coarse file-system
        // clock, while the load runs, has equal stamps, and the load may have
        // read B. Telling would take a hash of the bytes each reader of
        // configuration consumed; a second load for every recent change, as
        // before, would cost more than two rewrites inside one tick during a
        // reload, the second restoring the first exactly.
        let began = project.load_began;
        let mut config_unread = false;
        let config: Vec<(PathBuf, Option<FileStamp>)> = config_files(&project)
            .into_iter()
            .map(|p| {
                let earlier = before.and_then(|b| b.get(&p));
                let now = FileStamp::of(&p, earlier.is_some_and(|e| e.is_some_and(|e| e.hash.is_some())));
                let read = match (earlier, &now) {
                    (Some(Some(e)), Some(n)) => e.same(n),
                    (Some(None), None) | (None, None) => true,
                    (Some(_), _) => false,
                    (None, Some(n)) => !recent(n.mtime, began),
                };
                if !read {
                    crate::trace::log(|| format!("load: {} may have been written during the load", p.display()));
                    config_unread = true;
                }
                let now = now.map(|n| n.after(earlier.and_then(|e| e.as_ref())));
                (p, now)
            })
            .collect();
        Session { project, all_rules, edit_rules, resolution_rules, options, stamps, walked, config, config_unread, warned: false, full_walks: 0 }
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

    /// Source files that could not be read and are tried again at every
    /// sync (see `FileStamp::unread`).
    pub fn unread_files(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.stamps.iter().filter(|(_, s)| s.unread).map(|(p, _)| p.clone()).collect();
        v.sort();
        v
    }

    /// Files whose content is compared at every sync, their time being too
    /// recent to trust (see `FileStamp`), and whether the project is to be
    /// loaded again: what a sync costs beyond looking at times.
    pub fn in_doubt(&self) -> (usize, bool) {
        let racy = self.stamps.values().chain(self.config.iter().filter_map(|(_, s)| s.as_ref())).filter(|s| s.racy).count();
        (racy, self.config_unread)
    }

    /// Source files that appeared since they were last looked for, and files
    /// that a walk would no longer find though they are there, and whether a
    /// module appeared (a build file in a directory that was no module), for
    /// which the project is loaded again.
    ///
    /// A file is created, removed or renamed in a directory by an entry of
    /// that directory, which changes the directory's time; so every directory
    /// the discovery went into is stamped as a file is (with the hash of its
    /// names while its time is recent), and one that changed is listed again,
    /// as the discovery lists it. A directory in it that is new is walked
    /// whole; one that went is forgotten with all below it, its files being
    /// noticed by their own stamps. This finds a file whatever brought it
    /// (an editor, `git checkout`, `git stash pop`, a generator) and whatever
    /// time it carries, in no more than a look at the time of each
    /// directory. Which files the discovery skips is decided by ignore files:
    /// when one in a directory walked, above the root or in `.git/info`
    /// changes, appears or goes, the whole workspace is walked again.
    ///
    /// Not followed: git's global excludes file (`core.excludesFile`), and
    /// file systems that do not change a directory's time when an entry is
    /// added to it.
    fn rescan(&mut self) -> (Vec<PathBuf>, FxHashSet<PathBuf>, bool) {
        use crate::workspace::{boundary, listable, Workspace, IGNORE_FILES};
        let clock = SystemTime::now();
        let root = self.project.workspace.root.clone();
        let exclude = self.project.config.exclude.clone();
        let walk = |start: &Path, depth| Workspace::discover_in(&root, Some(start), depth, &exclude);
        let files = |w: Workspace| w.files.into_iter().map(|f| f.path);
        let modules = |w: &Workspace| w.modules.iter().map(|m| m.dir.clone()).collect::<Vec<_>>();
        let mut full = self.walked.ignores.iter_mut().any(|(p, stamp)| match stamp {
            Some(stamp) => stamp.changed(p, clock),
            None => FileStamp::of(p, false).is_some(),
        });
        // a repository that starts or ends above the root (`git init`): the
        // ignore files count only inside one; or one that did during the
        // last walk
        full |= self.walked.pending || self.walked.outer_gits.iter().any(|(a, kind)| boundary(a) != *kind);
        // git's index or HEAD changed (`git add`, a commit, a checkout): the
        // files git tracks although ignored, which a fresh load reads, may
        // be others. Asked of git only then; when the list differs, the
        // workspace is walked again, which reads the files that became
        // tracked and drops those that are no longer. (Stamped before git
        // is asked: a change while it runs shows at the next sync.)
        if !full && self.walked.git.iter_mut().any(|(p, stamp)| match stamp {
            Some(stamp) => stamp.changed(p, clock),
            None => FileStamp::of(p, false).is_some(),
        }) {
            let stamps: Vec<(PathBuf, Option<FileStamp>)> = self.walked.git.iter().map(|(p, old)| (p.clone(), FileStamp::of(p, true).map(|s| s.after(old.as_ref())))).collect();
            let tracked = crate::workspace::tracked_but_ignored_cached(&root, true);
            if tracked == self.walked.tracked {
                crate::trace::log(|| "sync: git's index changed, the same files tracked although ignored".into());
                self.walked.git = stamps;
            } else {
                crate::trace::log(|| "sync: git tracks other ignored files, walking the workspace".into());
                full = true;
            }
        }
        let (mut found, mut found_modules) = (Vec::new(), Vec::new());
        let mut gone: FxHashSet<PathBuf> = FxHashSet::default();
        // a directory the discovery would not go into (any more): it and all
        // below it are forgotten, and the source files held there are gone
        // (their own stamps may not show it: a stamp follows a symlink)
        let forget = |s: &mut Session, gone: &mut FxHashSet<PathBuf>, d: &Path| {
            s.walked.dirs.retain(|p, _| !p.starts_with(d));
            s.walked.ignores.retain(|(p, _)| !p.starts_with(d));
            s.walked.gits.retain(|p, _| !p.starts_with(d));
            gone.extend(s.stamps.keys().filter(|p| p.starts_with(d)).cloned());
        };
        if !full {
            let mut changed: Vec<PathBuf> = self.walked.dirs.par_iter_mut().filter_map(|(d, stamp)| stamp.changed(d, clock).then(|| d.clone())).collect();
            // (parents first: a directory that went takes those below it along)
            changed.sort();
            for d in changed {
                let Some(old) = self.walked.dirs.get(&d).copied() else { continue };
                // the stamp first, then the listing it vouches for; not
                // through a symlink, which the discovery does not follow
                let stamp = FileStamp::of(&d, true).filter(|_| std::fs::symlink_metadata(&d).is_ok_and(|m| m.is_dir()));
                let Some(mut stamp) = stamp else {
                    crate::trace::log(|| format!("sync: directory gone {}", d.display()));
                    forget(self, &mut gone, &d);
                    continue;
                };
                // an ignore file that appeared or went
                let known = |n: &&str| self.walked.ignores.iter().any(|(p, s)| s.is_some() && *p == d.join(n));
                if IGNORE_FILES.iter().any(|n| known(n) != std::fs::symlink_metadata(d.join(n)).is_ok()) {
                    full = true;
                    break;
                }
                // a repository that starts or ends here, or a `.git` of
                // another kind: which ignore files count, and which exclude
                // file, is decided again
                if boundary(&d) != self.walked.gits.get(&d).copied() {
                    full = true;
                    break;
                }
                crate::trace::log(|| format!("sync: directory changed {}", d.display()));
                // one that cannot be listed or entered (no permission) is
                // listed again at every sync until it can, as an unreadable
                // file is read
                stamp.unread = !listable(&d);
                self.walked.dirs.insert(d.clone(), stamp.after(Some(&old)));
                let began = SystemTime::now();
                let listed = walk(&d, Some(1));
                // what was held below it by a name it no longer lists exactly
                // (gone, now a symlink, renamed in case only on a file system
                // that ignores case) is forgotten
                let (dirs_now, files_now): (FxHashSet<&PathBuf>, FxHashSet<&PathBuf>) = (listed.dirs.iter().collect(), listed.files.iter().map(|f| &f.path).collect());
                let lost_dirs: Vec<PathBuf> = self.walked.dirs.keys().filter(|p| p.parent() == Some(d.as_path()) && !dirs_now.contains(p)).cloned().collect();
                gone.extend(self.stamps.keys().filter(|p| p.parent() == Some(d.as_path()) && !files_now.contains(p)).cloned());
                for l in lost_dirs {
                    crate::trace::log(|| format!("sync: directory no longer listed {}", l.display()));
                    forget(self, &mut gone, &l);
                }
                let new_dirs: Vec<&PathBuf> = listed.dirs.iter().filter(|s| **s != d && !self.walked.dirs.contains_key(*s)).collect();
                for sub in new_dirs {
                    let w = walk(sub, None);
                    let new = Walk::stamp(&w.dirs, None, began, &FxHashMap::default(), &FxHashMap::default());
                    self.walked.dirs.extend(new.dirs);
                    self.walked.ignores.extend(new.ignores);
                    self.walked.gits.extend(new.gits);
                    found_modules.extend(modules(&w));
                    found.extend(files(w));
                }
                found_modules.extend(modules(&listed));
                found.extend(files(listed));
            }
        }
        if full {
            crate::trace::log(|| "sync: an ignore file changed, walking the workspace".into());
            self.full_walks += 1;
            // ignore files stamped before the walk: one unchanged by its end is
            // what it read, and keeps the time it was first seen
            // (and git's index and HEAD, which the list of tracked files was read with)
            let before: FxHashMap<PathBuf, FileStamp> = self.walked.ignores.iter().chain(&self.walked.git).filter_map(|(p, old)| FileStamp::of(p, true).map(|s| (p.clone(), s.after(old.as_ref())))).collect();
            // and where repositories start, in the directories known before
            // the walk and above the root: one that starts or ends while it
            // runs leaves another walk to do
            let mut bounds: FxHashMap<PathBuf, Option<u8>> = self.walked.dirs.par_iter().map(|(d, _)| (d.clone(), boundary(d))).collect();
            bounds.extend(root.ancestors().skip(1).map(|a| (a.to_path_buf(), boundary(a))));
            let began = SystemTime::now();
            let w = walk(&root, None);
            self.walked = Walk::of_workspace(&w, began, &before, &bounds);
            // the modules as a fresh load finds them: one added, gone or of
            // another build system reloads the project
            let layout = |ms: &[crate::workspace::Module]| {
                let mut v: Vec<(PathBuf, crate::workspace::BuildSystem)> = ms.iter().map(|m| (m.dir.clone(), m.build_system)).collect();
                v.sort_by(|a, b| a.0.cmp(&b.0));
                v
            };
            if layout(&w.modules) != layout(&self.project.workspace.modules) {
                crate::trace::log(|| "sync: reload, the walk finds other modules".into());
                return (Vec::new(), FxHashSet::default(), true);
            }
            found = files(w).collect();
            let all: FxHashSet<&PathBuf> = found.iter().collect();
            gone = self.stamps.keys().filter(|p| !all.contains(p)).cloned().collect();
        }
        let module = found_modules.iter().find(|d| !self.project.workspace.modules.iter().any(|m| m.dir == **d));
        if let Some(d) = module {
            crate::trace::log(|| format!("sync: reload, a module appeared at {}", d.display()));
            return (Vec::new(), FxHashSet::default(), true);
        }
        found.retain(|p| !self.stamps.contains_key(p));
        found.sort();
        found.dedup();
        (found, gone, false)
    }

    /// How many times a sync walked the whole workspace (an ignore file
    /// changed); a directory that changed is listed alone.
    pub fn full_walks(&self) -> usize {
        self.full_walks
    }

    /// Re-read files that changed since they were read (and the given extra
    /// files, which may be new): by time and size, and by content for those
    /// whose time was too recent to tell (see `FileStamp`). Returns files
    /// whose declarations changed.
    pub fn sync(&mut self, extra: &[PathBuf]) -> Vec<PathBuf> {
        // A build, configuration or bean registration file changed, appeared
        // or went: the modules, the classpath, the settings and the beans come
        // from those, so the project is loaded again, as a new session would
        // load it. (Without
        // this a daemon kept the layout it started with: after a module was
        // added or renamed its files counted as the root module's.)
        let trace = crate::trace::enabled();
        let sig = |s: &Session, f: &Path| s.project.index.file(&path_key(f)).map(|s| format!("{:016x}", s.signature_hash)).unwrap_or_else(|| "-".into());
        let sigs: Vec<String> = if trace { extra.iter().map(|e| sig(self, e)).collect() } else { Vec::new() };
        let clock = SystemTime::now();
        let mut reload = self.config_unread || self.config.iter_mut().any(|(p, stamp)| {
            let changed = match stamp {
                Some(stamp) => stamp.changed(p, clock),
                None => FileStamp::of(p, false).is_some(),
            };
            if changed {
                crate::trace::log(|| format!("sync: reload, {} changed", p.display()));
            }
            changed
        });
        // source files that appeared or that the walk no longer finds, and
        // whether a module appeared
        let (mut appeared, mut gone) = (Vec::new(), FxHashSet::default());
        if !reload {
            (appeared, gone, reload) = self.rescan();
        }
        if reload {
            (appeared, gone) = (Vec::new(), FxHashSet::default());
            if self.config_unread {
                crate::trace::log(|| "sync: reload, the version of a configuration file the load read is not known".into());
            }
            self.reload();
        }
        // (on all cores: after a checkout of the whole tree every file is racy
        // and its content is compared)
        let clock = SystemTime::now();
        let mut stale: Vec<PathBuf> = self
            .stamps
            .par_iter_mut()
            .filter_map(|(p, stamp)| {
                let was = *stamp;
                if !stamp.changed(p, clock) {
                    return None;
                }
                crate::trace::log(|| format!("sync: stale {} recorded {} size {}{} now {}", p.display(), crate::trace::time(Some(was.mtime)), was.size, if was.racy { " racy" } else { "" }, crate::trace::time(std::fs::metadata(p).and_then(|m| m.modified()).ok())));
                Some(p.clone())
            })
            .collect();
        // (appeared: not in `stamps`, so not in `stale` yet; gone: some may be)
        let mut queued: FxHashSet<PathBuf> = appeared.iter().cloned().collect();
        for p in appeared {
            crate::trace::log(|| format!("sync: appeared {}", p.display()));
            stale.push(p.clone());
            let module = self.project.workspace.module_of(&p).unwrap_or(0);
            self.project.workspace.files.push(crate::workspace::SourceFile { kind: classify(&p), path: p, module });
        }
        if !gone.is_empty() {
            queued.extend(stale.iter().cloned());
            stale.extend(gone.iter().filter(|p| queued.insert((*p).clone())).cloned());
        }
        for e in extra {
            let e = e.canonicalize().unwrap_or_else(|_| e.clone());
            if e.extension().is_some_and(|x| x == "java") && !self.stamps.contains_key(&e) && !queued.contains(&e) {
                stale.push(e.clone());
                let module = self.project.workspace.module_of(&e).unwrap_or(0);
                self.project.workspace.files.push(crate::workspace::SourceFile { path: e.clone(), module, kind: classify(&e) });
            }
        }
        let mut changed = Vec::new();
        let mut any_decl_change = false;
        let mut di_stale = false;
        let mut forgotten: FxHashSet<PathBuf> = FxHashSet::default();
        let any_stale = !stale.is_empty();
        for p in stale {
            // the stamp first, then the text it vouches for; a path that is
            // no longer a file (a directory now), or that the walk no longer
            // finds, is gone, as for a fresh load
            let file = std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_file());
            let (stamp, text) = if file && !gone.contains(&p) { FileStamp::read(&p) } else { (None, None) };
            // refresh_file rebuilds the DI model itself; batch it below instead
            let key = path_key(&p);
            let old = self.project.index.file(&key).map(|s| s.signature_hash);
            // (the context starts a launcher makes are read with its stub)
            let old_startups = self.project.index.file(&key).map(|s| s.startups.clone());
            if let Some(stamp) = stamp {
                // a time seen at an earlier look keeps the margin it began then
                let stamp = stamp.after(self.stamps.get(&p));
                if let Some(text) = text {
                    let pf = ParsedFile::parse(text);
                    let stub = Arc::new(self.project.extract(&p, &pf));
                    // install the new stub even when signatures are unchanged:
                    // referenced names (for broken callers) and declaration
                    // positions change with body-only edits
                    let decl_change = old != Some(stub.signature_hash);
                    // the DI model stores positions of beans and injection points
                    di_stale |= decl_change || old_startups.as_ref() != Some(&stub.startups) || self.project.di.beans.iter().any(|b| b.file.as_deref() == Some(&*key)) || self.project.di.points.iter().any(|pt| pt.file.as_deref() == Some(&*key));
                    self.project.index.set_file(key, stub);
                    if decl_change {
                        any_decl_change = true;
                        changed.push(p.clone());
                    }
                } else if old.is_some() {
                    // could not be read: no stub, as a fresh load has none;
                    // the stamp is `unread`, so the next sync tries again
                    self.project.index.remove_file(&key);
                    any_decl_change = true;
                    changed.push(p.clone());
                }
                self.stamps.insert(p.clone(), stamp);
            } else {
                self.project.index.remove_file(&key);
                self.stamps.remove(&p);
                forgotten.insert(p.clone());
                any_decl_change = true;
                changed.push(p.clone());
            }
        }
        if !forgotten.is_empty() {
            self.project.workspace.files.retain(|f| !forgotten.contains(&f.path));
        }
        // The JDK API, the modules' releases and all that is resolved against
        // them follow from the release plan, which also depends on the
        // sources (which modules hold code, what release the code needs): one
        // a fresh load would make differently now is made by loading again.
        if any_stale && self.project.release_plan_changed() {
            crate::trace::log(|| "sync: reload, the sources change the release plan".into());
            self.reload();
            return changed;
        }
        if any_decl_change || di_stale {
            self.project.rebuild_di();
        }
        self.project.index.ensure_resolved();
        if trace {
            for (e, b) in extra.iter().zip(&sigs) {
                let now = std::fs::metadata(e).and_then(|m| m.modified()).ok();
                let text = crate::fsutil::read_text(e).ok().map(|t| format!("{:016x}", crate::cache::content_hash(&t))).unwrap_or_else(|| "-".into());
                crate::trace::log(|| format!("sync: checked {} stub {} -> {} recorded {} now {} content {}", e.display(), b, sig(self, e), crate::trace::time(self.stamps.get(e).map(|s| s.mtime)), crate::trace::time(now), text));
            }
        }
        changed
    }

    /// Load the project again, as a new session would load it.
    fn reload(&mut self) {
        let root = self.root().to_path_buf();
        // stamped before the load begins, to tell afterwards whether the
        // load read them as they are then
        let before: FxHashMap<PathBuf, Option<FileStamp>> = self.config.iter().map(|(p, _)| (p.clone(), FileStamp::of(p, false))).collect();
        if let Ok(project) = Project::load(&root) {
            // warnings already shown are not shown again for the same state
            let shown = self.warned.then(|| self.project.classpath_warnings.clone());
            *self = Session::from_load(project, Some(&before));
            self.warned = shown.is_some_and(|w| w == self.project.classpath_warnings);
        }
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
        self.project.index.ensure_resolved();
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
        let repo = git::is_repo(self.root());
        // keyed by resolved path: git reports `/private/var/..` for a root
        // reached through the `/var` symlink
        let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let ranges = repo.then(|| git::changed_lines_since(self.root(), &files, rev).into_iter().map(|(p, r)| (canon(&p), r)).collect::<FxHashMap<_, _>>());
        let root = self.root().to_path_buf();
        let on_changed_line = |f: &Finding| match ranges.as_ref().and_then(|r| r.get(&canon(&root.join(&f.path)))) {
            Some(r) => git::in_ranges(f.line, r),
            None => true,
        };

        // A compile-break or wiring finding on a changed line is a claim about
        // the change only if the project before the change did not get the
        // same claim: a gap in the model is there before and after alike.
        // Those that are new are stable when the rule is one of the promised
        // compile-break rules; on unchanged lines (`--all-lines`) they stay
        // what they are in a whole-project check.
        if repo {
            let claims: Vec<usize> = (0..findings.len()).filter(|&i| compared_in_changed(findings[i].rule) && on_changed_line(&findings[i])).collect();
            let mut drop: FxHashSet<usize> = FxHashSet::default();
            if !claims.is_empty() {
                let touched = self.touched(opts, &files, &deleted, rev);
                let before = self.claims_before(&findings, &claims, &files, &touched, rev);
                drop = claims.iter().copied().filter(|&i| before.contains(&claim_key(&findings[i])) || gap_key(&findings[i]).is_some_and(|g| before.contains(&g))).collect();
            }
            for &i in &claims {
                if stable_in_changed(findings[i].rule) {
                    findings[i].maturity = Maturity::Stable;
                }
            }
            let mut i = 0;
            findings.retain(|_| {
                i += 1;
                !drop.contains(&(i - 1))
            });
        }

        // changed-lines filter
        if !opts.all_lines && ranges.is_some() {
            let before = findings.len();
            findings.retain(|f| on_changed_line(f));
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
        if repo {
            let (changed_apis, names) = self.api_changes(&files, rev);
            report.changed_apis = changed_apis;
            let mut pending = if opts.base.is_some() {
                // CI: every unchanged file, against both versions of the changed ones
                self.cross_file_breaks(&files, &deleted, rev)
            } else {
                // per turn: quick, only unresolved references to changed names,
                // and of those only the ones the change introduced
                let exclude: FxHashSet<PathBuf> = files.iter().cloned().collect();
                let deps = self.dependents(&names, &exclude);
                let now = if deps.is_empty() { Vec::new() } else { self.analyze(&self.resolution_rules, &deps) };
                if now.is_empty() {
                    now
                } else {
                    // (files deleted since HEAD are no longer in the index, so
                    // no dependent is analyzed against a stub of another version)
                    let touched = self.touched(opts, &files, &deleted, rev);
                    let before: FxHashSet<PlaceClaim> = self.at_base(&touched, rev, |s| s.analyze(&s.resolution_rules, &deps)).iter().map(place_claim).collect();
                    now.into_iter().filter(|f| !before.contains(&place_claim(f))).collect()
                }
            };
            // the promise covers the compile-break rules: places broken by a
            // missing or ambiguous bean, and `unresolved-static-import` from the
            // per-turn scan, stay preview
            for f in pending.iter_mut().filter(|f| stable_in_changed(f.rule)) {
                f.maturity = Maturity::Stable;
            }
            pending.retain(|f| !f.in_baseline(&base));
            pending.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
            report.pending = pending;
            report.breaks = self.attribute_breaks(&report.pending, &files, rev);
        }
        report
    }

    /// The files whose version at `rev` makes up the project before the
    /// change: the changed and deleted files, and per turn (no explicit
    /// files) also the files deleted since HEAD. With explicit files (the
    /// post-edit hook) only those: the change judged is the edit to them.
    fn touched(&self, opts: &CheckOptions, files: &[PathBuf], deleted: &[PathBuf], rev: &str) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = files.iter().chain(deleted).cloned().collect();
        if opts.base.is_none() && opts.files.is_empty() {
            let root = self.root().canonicalize().unwrap_or_else(|_| self.root().to_path_buf());
            out.extend(git::changed_files_since(self.root(), rev).1.into_iter().filter(|f| f.extension().is_some_and(|x| x == "java") && (f.starts_with(&root) || f.starts_with(self.root()))));
        }
        out.sort();
        out.dedup();
        out
    }

    /// The compile-break and wiring claims the project made before the
    /// change that match `findings[claims]`: in the version at `rev` of the
    /// files they are in, and in files the change did not touch that mention
    /// every name the claim quotes. A claim the model already made about code
    /// before the change (which compiled) is one it cannot back: the same
    /// gap, whether the line holding it was edited, moved or newly written.
    fn claims_before(&mut self, findings: &[Finding], claims: &[usize], files: &[PathBuf], touched: &[PathBuf], rev: &str) -> FxHashSet<(&'static str, String)> {
        let rules = RuleSet::new(crate::analyze::selected_rules_for(&self.project, true).into_iter().filter(|r| compared_in_changed(r.meta().id)).collect());
        // a finding's path is relative to the root, as `analyze_text` makes it
        let paths: FxHashSet<&str> = claims.iter().map(|&i| findings[i].path.as_str()).collect();
        let in_files: Vec<PathBuf> = files.iter().filter(|f| paths.contains(f.strip_prefix(self.root()).unwrap_or(f).to_string_lossy().as_ref())).cloned().collect();
        let skip: FxHashSet<&PathBuf> = touched.iter().collect();
        let mut elsewhere: FxHashSet<PathBuf> = FxHashSet::default();
        for &i in claims {
            // the names of the gap where it is less than the claim: another
            // injection point of the type, not of the same name
            let f = &findings[i];
            // (the message's simple names: files mention those, not qualified ones)
            // (the bean rules' gap is the type, quoted first: its simple name)
            let names = if gap_key(f).is_some() { f.message.split('`').nth(1).map(|t| quoted_names(&format!("`{t}`"))).unwrap_or_default() } else { quoted_names(&f.message) };
            if names.is_empty() {
                continue;
            }
            for (p, stub) in self.project.index.files() {
                let pb = PathBuf::from(&**p);
                if !skip.contains(&pb) && classify(&pb) != SourceKind::Generated && names.iter().all(|n| stub.referenced_names.binary_search_by(|x| (**x).cmp(n.as_str())).is_ok()) {
                    elsewhere.insert(pb);
                }
            }
        }
        let mut elsewhere: Vec<PathBuf> = elsewhere.into_iter().collect();
        elsewhere.sort();
        let old: Vec<(PathBuf, String)> = in_files.iter().filter_map(|f| git::version_at(self.root(), rev, f).map(|t| (f.clone(), t))).collect();
        let found = self.at_base(touched, rev, |s| {
            s.project.index.ensure_resolved();
            let mut v: Vec<Finding> = old.par_iter().flat_map_iter(|(f, t)| analyze_text(&s.project, &rules, &s.options, f, t)).collect();
            v.extend(s.analyze(&rules, &elsewhere));
            v
        });
        found.iter().flat_map(|f| std::iter::once(claim_key(f)).chain(gap_key(f))).collect()
    }

    /// Run `f` on the project as it was at `rev` for the `touched` files
    /// (their version there, or none), then put the current stubs and bean
    /// model back as they were, from memory, also when `f` panics: the daemon
    /// survives a panic in a request, and no file's time would tell a later
    /// sync that the index holds the base version.
    fn at_base<R>(&mut self, touched: &[PathBuf], rev: &str, f: impl FnOnce(&Session) -> R) -> R {
        let saved: Vec<(Arc<str>, Option<Arc<FileStub>>)> = touched.iter().map(|p| {
            let key = path_key(p);
            let stub = self.project.index.file(&key).cloned();
            (key, stub)
        }).collect();
        let mut base_di = grounds_di::DiModel::default();
        base_di.lombok_copies_qualifiers = self.project.di.lombok_copies_qualifiers;
        base_di.bean_overriding_allowed = self.project.di.bean_overriding_allowed;
        let di = std::mem::replace(&mut self.project.di, base_di);
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for p in touched {
                let key = path_key(p);
                match git::version_at(self.root(), rev, p) {
                    Some(t) => {
                        let stub = Arc::new(self.project.extract(p, &ParsedFile::parse(t)));
                        self.project.index.set_file(key, stub);
                    }
                    None => self.project.index.remove_file(&key),
                }
            }
            self.project.rebuild_di();
            f(self)
        }));
        for (key, stub) in saved {
            match stub {
                Some(stub) => self.project.index.set_file(key, stub),
                None => self.project.index.remove_file(&key),
            }
        }
        self.project.di = di;
        match out {
            Ok(r) => r,
            Err(e) => std::panic::resume_unwind(e),
        }
    }

    /// Findings of the cross-file rules in files outside `changed`/`deleted`
    /// that the change introduced: analyzed with the current version of the
    /// changed files and again with their version at `rev`, keeping what is
    /// new. The unchanged files' text is the same both times, so fingerprints
    /// match exactly.
    fn cross_file_breaks(&mut self, changed: &[PathBuf], deleted: &[PathBuf], rev: &str) -> Vec<Finding> {
        let rules = RuleSet::new(crate::analyze::selected_rules_for(&self.project, true).into_iter().filter(|r| CROSS_FILE_RULES.contains(&r.meta().id)).collect());
        let touched: Vec<PathBuf> = changed.iter().chain(deleted).cloned().collect();
        let skip: FxHashSet<&PathBuf> = touched.iter().collect();
        let others: Vec<PathBuf> = crate::analyze::all_source_files(&self.project).into_iter().filter(|f| !skip.contains(f)).collect();
        if touched.is_empty() || others.is_empty() {
            return Vec::new();
        }
        let now = self.analyze(&rules, &others);
        let (before, base_classes) = self.at_base(&touched, rev, |s| {
            let classes: Vec<Name> = touched.iter().filter_map(|f| s.project.index.file(&path_key(f))).flat_map(|stub| stub.classes.iter().map(|c| c.name.clone())).collect();
            let before: FxHashSet<PlaceClaim> = s.analyze(&rules, &others).iter().map(place_claim).collect();
            (before, classes)
        });
        let mut found: Vec<Finding> = now.into_iter().filter(|f| !before.contains(&place_claim(f))).collect();
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
                    return i.is_named(name) && (i.is_static || *i.path != *format!("{pkg}.{name}"));
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
                maturity: Maturity::Stable,
                fingerprint,
                legacy_fingerprint: None,
                fix: None,
                claim: None,
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
                // as mature as the weakest place it stands for
                let maturity = if broken.iter().all(|&i| pending[i].maturity == Maturity::Stable) { Maturity::Stable } else { Maturity::Preview };
                let finding = Finding { path, line, col: 1, end_line: line, end_col: 1, rule: "breaks-other-files", severity, category: "bug", message, maturity, fingerprint, legacy_fingerprint: None, fix: None, claim: None, start: 0, end: 0 };
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
                if crate::workspace::is_test_path(&pb.to_string_lossy()) {
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
            if crate::workspace::is_test_path(&f.to_string_lossy()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").status().unwrap().success();
        assert!(ok, "git {args:?}");
    }

    fn finding(rule: &'static str, message: &str, claim: Option<&str>) -> Finding {
        Finding { path: "A.java".into(), line: 1, col: 1, end_line: 1, end_col: 1, rule, severity: Severity::Error, category: "bug", message: message.into(), maturity: Maturity::Preview, fingerprint: String::new(), legacy_fingerprint: None, fix: None, claim: claim.map(str::to_string), start: 0, end: 0 }
    }

    /// Per rule, both sides: what the message asserts tells claims apart,
    /// and what is presentation (hint, candidates, truncation, order) does not.
    #[test]
    fn a_claim_is_what_the_message_asserts() {
        let key = |rule, m: &str| claim_key(&finding(rule, m, None)).1;
        let same = |rule, a: &str, b: &str| assert_eq!(key(rule, a), key(rule, b), "{rule}: {a} | {b}");
        let differ = |rule, a: &str, b: &str| assert_ne!(key(rule, a), key(rule, b), "{rule}: {a} | {b}");
        let r = "unresolved-method";
        same(r, "`Lib` has no method `fo`; did you mean `foo`?", "`Lib` has no method `fo`; did you mean `fob`?");
        same(r, "No `f` in `Lib` accepts (String); available: f(int), f(int,int)", "No `f` in `Lib` accepts (String); available: f(int,int), f(int)");
        same(r, "No `f` in `Lib` takes 3 argument(s); available: f(int)", "No `f` in `Lib` takes 3 argument(s); available: f(long)");
        differ(r, "`Old` has no method `missing`", "`New` has no method `missing`");
        differ(r, "`Lib` has no method `a`", "`Lib` has no method `b`");
        differ(r, "No `f` in `Lib` accepts (String); available: f(int)", "No `f` in `Lib` accepts (long); available: f(int)");
        differ(r, "No `f` in `Lib` takes 3 argument(s); available: f(int)", "No `f` in `Lib` takes 2 argument(s); available: f(int)");
        differ(r, "`f(int)` is private in `Lib`", "`f(int)` is private in `Other`");
        let r = "unresolved-constructor";
        same(r, "No constructor of `A` takes 3 argument(s); available: A(int), A()", "No constructor of `A` takes 3 argument(s); available: A(), A(long)");
        differ(r, "No constructor of `A` takes 3 argument(s); available: A()", "No constructor of `B` takes 3 argument(s); available: A()");
        differ(r, "No constructor of `A` takes 3 argument(s); available: A()", "No constructor of `A` takes 2 argument(s); available: A()");
        differ("unresolved-field", "`A` has no field `x`", "`A` has no field `y`");
        differ("override-matches-nothing", "`f(int)` is marked @Override but overrides no supertype method", "`f(long)` is marked @Override but overrides no supertype method");
        differ("missing-jdk-import", "`java.util.SequencedCollection` is not public API in Java 17", "`java.util.SequencedCollection` is not public API in Java 11");
        let r = "qualifier-not-found";
        same(r, "No `Api` bean is qualified `x`; available: 'a' (A), 'b' (B)", "No `Api` bean is qualified `x`; available: 'b' (B)");
        differ(r, "No `Api` bean is qualified `x`; available: 'a' (A)", "No `Api` bean is qualified `y`; available: 'a' (A)");
        // the part after `;` is asserted here: kept
        let r = "bean-not-found";
        differ(r, "No bean of type `Api` is defined; injection of `old` fails at startup", "No bean of type `Api` is defined; injection of `fresh` fails at startup");
        differ(r, "No bean of type `Api` is defined; injection of `a` fails at startup", "No bean of type `Other` is defined; injection of `a` fails at startup");
        let r = "bean-name-conflict";
        differ(r, "Bean name 'a' is also used by x.A; Spring fails at startup (ConflictingBeanDefinitionException). Give one an explicit name", "Bean name 'a' is also used by @Bean x.Cfg.a(); Spring Boot fails at startup (BeanDefinitionOverrideException) unless bean overriding is enabled. Give one an explicit name");
        // the rules that pass the whole claim: the message may show less
        let with = |rule, m: &str, c: &str| claim_key(&finding(rule, m, Some(c))).1;
        let r = "unimplemented-abstract-method";
        assert_ne!(with(r, "`C` does not implement `I.old()`", "`C` does not implement `I.old()`"), with(r, "`C` does not implement `I.fresh()`", "`C` does not implement `I.fresh()`"));
        assert_eq!(with(r, "`C` does not implement `I.a()`, `I.b()`, `I.c()` and 1 more", "`C` does not implement `I.a()`, `I.b()`, `I.c()`, `I.d()`"), with(r, "`C` does not implement `I.d()`, `I.c()`, `I.b()` and 1 more", "`C` does not implement `I.a()`, `I.b()`, `I.c()`, `I.d()`"));
        let r = "ambiguous-bean";
        assert_ne!(with(r, "2 beans of type `Api` match `api`: 'one' (One), 'two' (Two); add a @Qualifier or @Primary", "beans of type `Api` match `api`: 'one' (One), 'two' (Two)"), with(r, "2 beans of type `Api` match `api`: 'three' (Three), 'four' (Four); add a @Qualifier or @Primary", "beans of type `Api` match `api`: 'four' (Four), 'three' (Three)"));
        // the cycle rule hands over no claim: the message, the cycle as the
        // search meets it, is the claim (the rule is preview for that reason)
        let r = "circular-constructor-dependency";
        differ(r, "Circular constructor dependency: A -> B -> A", "Circular constructor dependency: B -> A -> B");
        same(r, "Circular constructor dependency: A -> B -> A", "Circular constructor dependency: A -> B -> A");
        // claims name types by their qualified names; the message may not
        assert_ne!(with("unresolved-method", "`Lib` has no method `x`", "`a.Lib` has no method `x`"), with("unresolved-method", "`Lib` has no method `x`", "`b.Lib` has no method `x`"));
        assert_ne!(with("unimplemented-abstract-method", "`Foo` does not implement `I.m(Bar)`", "`a.Foo` does not implement a.I.m(a.Bar)"), with("unimplemented-abstract-method", "`Foo` does not implement `I.m(Bar)`", "`a.Foo` does not implement a.I.m(b.Bar)"));
        // the gap: a missing or ambiguous bean is about the type, not the point
        let gap = |rule, c: &str| gap_key(&finding(rule, "", Some(c)));
        assert_eq!(gap("bean-not-found", "No bean of type `a.Api`; injection of `old` into `a.Old`"), gap("bean-not-found", "No bean of type `a.Api`; injection of `fresh` into `a.Fresh`"));
        assert_ne!(gap("bean-not-found", "No bean of type `a.Api`; injection of `x` into `a.X`"), gap("bean-not-found", "No bean of type `b.Api`; injection of `x` into `a.X`"));
        assert_eq!(gap("ambiguous-bean", "beans of type `a.Api` match `x` into `a.X`: 'a' a.A, 'b' a.B"), gap("ambiguous-bean", "beans of type `a.Api` match `y` into `a.Y`: 'a' a.A, 'b' a.B"));
        assert_ne!(gap("ambiguous-bean", "beans of type `a.Api` match `x` into `a.X`: 'a' a.A, 'b' a.B"), gap("ambiguous-bean", "beans of type `a.Api` match `x` into `a.X`: 'a' a.A, 'b' b.B"));
        assert_eq!(gap_key(&finding("unresolved-method", "`Lib` has no method `gap`", Some("`a.Lib` has no method `gap`"))), None);
    }

    /// A panic while the project is in its base state (the daemon catches it
    /// and goes on) must not leave the base version of the files behind: no
    /// file's time changed, so no later sync would repair it.
    #[test]
    fn a_panic_in_the_base_state_leaves_the_session_as_it_was() {
        let dir = std::env::temp_dir().join(format!("grounds-atbase-panic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let w = |rel: &str, t: &str| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, t).unwrap();
            // older than the window in which `sync` reads a recent file again,
            // which would repair the index by chance
            let old = SystemTime::now() - std::time::Duration::from_secs(60);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(old).unwrap();
        };
        w("build.gradle", "plugins { id 'java' }\n");
        w("src/main/java/demo/Owner.java", "package demo;\npublic class Owner {\n    public void addPet(String p) {}\n}\n");
        w("src/main/java/demo/Clinic.java", "package demo;\nclass Clinic {\n    void m(Owner o) { o.addPet(\"rex\"); }\n}\n");
        git(&dir, &["init", "-q"]);
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-qm", "init"]);
        w("src/main/java/demo/Owner.java", "package demo;\npublic class Owner {\n    public void registerPet(String p) {}\n}\n");
        let owner = dir.join("src/main/java/demo/Owner.java").canonicalize().unwrap();
        let opts = CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base: None };
        let mut s = Session::open(&dir).unwrap();
        let first = serde_json::to_string(&s.check(&opts)).unwrap();
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.at_base(&[owner.clone()], "HEAD", |_| -> () { panic!("in the base state") })));
        std::panic::set_hook(hook);
        assert!(r.is_err());
        let after = serde_json::to_string(&s.check(&opts)).unwrap();
        let fresh = serde_json::to_string(&Session::open(&dir).unwrap().check(&opts)).unwrap();
        assert_eq!(after, fresh);
        assert_eq!(after, first);
        assert!(after.contains("Clinic.java"), "{after}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Git's index and HEAD are those of the work tree's own git directory:
    /// a linked worktree's, for a project below the top of its repository.
    /// A sync with neither changed runs no git; staging an ignored file, or
    /// unstaging it, is seen as a fresh load sees it.
    #[test]
    fn the_work_tree_s_own_index_decides_the_tracked_ignored_files() {
        let base = std::env::temp_dir().join(format!("grounds-index-worktree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let main = base.join("main");
        let w = |rel: &str, text: &str| {
            let p = main.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        w(".gitignore", "Hidden.java\n");
        w("app/build.gradle", "plugins { id 'java' }\n");
        w("app/src/main/java/Use.java", "class Use { void test(Hidden h) { h.missing(); } }\n");
        w("app/src/main/java/Hidden.java", "class Hidden {}\n");
        std::fs::create_dir_all(&main).unwrap();
        git(&main, &["init", "-q"]);
        git(&main, &["add", "-A"]);
        git(&main, &["commit", "-qm", "init"]);
        let tree = base.join("tree");
        git(&main, &["worktree", "add", "-q", tree.to_str().unwrap()]);
        std::fs::write(tree.join("app/src/main/java/Hidden.java"), "class Hidden {}\n").unwrap();
        let dir = tree.join("app").canonicalize().unwrap();
        let opts = CheckOptions { mode: Mode::Checkpoint, files: vec![dir.join("src/main/java/Use.java")], all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None };
        let report = |s: &mut Session| serde_json::to_string(&s.check(&opts)).unwrap();
        let as_fresh = |s: &mut Session, what: &str| {
            let warm = report(s);
            assert_eq!(warm, report(&mut Session::open(&dir).unwrap()), "after {what}");
            warm
        };
        let mut s = Session::open(&dir).unwrap();
        assert!(!as_fresh(&mut s, "the load").contains("unresolved-method"));
        // (the index was written just before the load: the first sync asks git once)
        s.sync(&[]);
        let runs = crate::workspace::git_listings(&dir);
        s.sync(&[]);
        s.sync(&[]);
        assert_eq!(crate::workspace::git_listings(&dir), runs, "a sync with git's index and HEAD unchanged runs no git");
        // staged in the main work tree: not this work tree's index
        git(&main, &["add", "-f", "app/src/main/java/Hidden.java"]);
        assert!(!as_fresh(&mut s, "a file staged in another work tree").contains("unresolved-method"));
        git(&tree, &["add", "-f", "app/src/main/java/Hidden.java"]);
        assert!(as_fresh(&mut s, "a file staged in this work tree").contains("unresolved-method"));
        git(&tree, &["rm", "-q", "--cached", "app/src/main/java/Hidden.java"]);
        assert!(!as_fresh(&mut s, "the file unstaged").contains("unresolved-method"));
        let _ = std::fs::remove_dir_all(&base);
    }

}
