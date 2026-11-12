//! `grounds selfcheck`: what grounds resolves from a project's sources,
//! compared with what javac recorded in the project's own build output
//! (`crate::oracle` makes the comparisons). A disagreement shows something
//! this project uses that grounds' model gets wrong, before a rule reports a
//! false finding because of it.
//!
//! Each module's Java source sets are paired with the directory a build
//! compiles them into (`classpath::JAVA_OUTPUTS`). A source file is compared
//! only when the newest class file compiled from it is at least as new as
//! it; one edited since (newer than its class files), or declaring a class
//! no class file has (added since the build, or not compiled), is not.
//!
//! A class file reflects the whole build: a file not edited can still
//! disagree because another changed after the build (a method renamed and
//! not yet recompiled). So the files edited, added and removed since the
//! build are counted, and a disagreement about a class declared in one of
//! them (or about a subclass of one) is listed apart as possibly stale. It
//! is still a disagreement.

use crate::classpath::JAVA_OUTPUTS;
use crate::daemon::Outcome;
use crate::oracle::{compare_file, Bytecode, Counts, Disagreement, Tally, CATEGORIES};
use crate::workspace::BuildSystem;
use crate::Project;
use rayon::prelude::*;
use std::collections::{HashSet, VecDeque};
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// A source set of a module, the directory its build output was looked for
/// in, and what was compared. Paths are relative to the project root.
pub struct SetReport {
    /// `main` or `test`.
    pub name: &'static str,
    pub sources: PathBuf,
    /// The build output compared with; `None`: none found.
    pub classes: Option<PathBuf>,
    pub looked_in: Vec<PathBuf>,
    /// Source files compared.
    pub compared: usize,
    /// Newer than the class files compiled from them: not compared.
    pub edited: Vec<PathBuf>,
    /// Declaring a class no class file was compiled from: not compared.
    pub added: Vec<PathBuf>,
    /// Source files class files were compiled from that are not there any
    /// more.
    pub removed: Vec<PathBuf>,
    pub unreadable: Vec<String>,
}

pub struct ModuleReport {
    /// Relative to the project root (`.` for the root).
    pub dir: PathBuf,
    pub build_system: BuildSystem,
    pub sets: Vec<SetReport>,
    pub counts: [Counts; 5],
}

/// A disagreement, at a path relative to the project root.
pub struct Found {
    pub path: PathBuf,
    pub disagreement: Disagreement,
    /// It is about a class declared in a file edited, added or removed since
    /// the build, or a subclass of one.
    pub possibly_stale: bool,
}

pub struct Report {
    pub modules: Vec<ModuleReport>,
    pub counts: [Counts; 5],
    pub found: Vec<Found>,
}

/// What each category is called in the text output.
const LABELS: [&str; 5] = ["calls", "constructors", "fields", "method reference names", "members"];

const MEANING: &str = "A disagreement means grounds resolved something differently from the compiler on this project, so rules that depend on it may be wrong there: please report it with the line shown.";

const HINT: &str = "No build output found: selfcheck compares with the class files of a build. Build first (`./gradlew compileJava compileTestJava` or `mvn test-compile`), then run it again.";

/// Compare the project at `root` with its build output.
pub fn run(root: &Path) -> anyhow::Result<Report> {
    Ok(compare(&load(root)?))
}

/// The project at `root` as selfcheck compares it: without its modules'
/// build output on the classpath, where `grounds check` has it by default.
/// That output is compiled from these very sources, and a class grounds
/// found there instead of in a source (a local class that javac and grounds
/// number alike) would agree with its class file by being it, hiding what
/// grounds makes of the source. (A module's output listed in a classpath
/// file, as `grounds classpath` may list a sibling module's, is still read.)
pub fn load(root: &Path) -> anyhow::Result<Project> {
    let mut config = crate::Config::load(root)?;
    config.classpath.build_outputs = false;
    Project::load_with(root, config)
}

/// Compare a project (as `load` loads it) with its build output.
pub fn compare(p: &Project) -> Report {
    let root = &p.workspace.root;
    let shown = |path: &Path| -> PathBuf {
        match path.strip_prefix(root) {
            Ok(r) if r.as_os_str().is_empty() => PathBuf::from("."),
            Ok(r) => r.to_path_buf(),
            Err(_) => path.to_path_buf(),
        }
    };
    let mut modules = Vec::new();
    let mut found: Vec<(PathBuf, Disagreement)> = Vec::new();
    // what changed since the build: source files (absolute), and the
    // classes the class files say were compiled from them
    let mut edited_files: HashSet<PathBuf> = HashSet::new();
    let mut edited_classes: HashSet<String> = HashSet::new();
    for (mi, m) in p.workspace.modules.iter().enumerate() {
        let mut sets = Vec::new();
        let mut counts = [Counts::default(); 5];
        // the source files of the module, as the class files name them
        // (`a/b/C.java`, by package): a class file compiled from one of them
        // in another directory (generated sources) names a file that is there
        let named: HashSet<String> = p.workspace.files.iter().filter(|f| f.module == mi).filter_map(|f| package_path(p, &f.path)).collect();
        for (name, src, outputs) in JAVA_OUTPUTS {
            let sources = m.dir.join(src);
            let files: Vec<&PathBuf> = p.workspace.files.iter().filter(|f| f.module == mi && f.path.starts_with(&sources)).map(|f| &f.path).collect();
            if files.is_empty() {
                continue;
            }
            // the build system's own directory first
            let mut looked_in: Vec<PathBuf> = outputs.iter().map(|o| m.dir.join(o)).collect();
            if m.build_system == BuildSystem::Maven {
                looked_in.reverse();
            }
            let classes = looked_in.iter().find(|d| d.is_dir()).cloned();
            let mut set = SetReport { name, sources: shown(&sources), classes: classes.as_deref().map(shown), looked_in: looked_in.iter().map(|d| shown(d)).collect(), compared: 0, edited: Vec::new(), added: Vec::new(), removed: Vec::new(), unreadable: Vec::new() };
            let Some(classes) = classes else {
                sets.push(set);
                continue;
            };
            let bc = Bytecode::read(&classes);
            let mut compared: Vec<(&PathBuf, String)> = Vec::new();
            for f in files {
                let rel = f.strip_prefix(&sources).unwrap_or(f).to_string_lossy().to_string();
                let modified = std::fs::metadata(f).and_then(|m| m.modified()).ok();
                match bc.written(&rel) {
                    // (javac writes every class file of a source it compiles:
                    // the newest is from the last compilation of it; an older
                    // one may be left from before, of a class since removed)
                    Some(written) if modified.is_some_and(|t| t <= written) => compared.push((f, rel)),
                    Some(_) => {
                        set.edited.push(shown(f));
                        edited_files.insert(f.clone());
                    }
                    None if declares_a_class(p, f) => {
                        set.added.push(shown(f));
                        edited_files.insert(f.clone());
                    }
                    // nothing that compiles to a class file of its own
                    // (`package-info.java` without annotations): nothing to compare
                    None => compared.push((f, rel)),
                }
            }
            let mut removed: HashSet<&str> = HashSet::new();
            for (class, rel) in bc.sources() {
                let gone = rel.ends_with(".java") && !named.contains(rel) && !sources.join(rel).exists();
                if gone && removed.insert(rel) {
                    set.removed.push(shown(&sources.join(rel)));
                }
                if gone || edited_files.contains(&sources.join(rel)) {
                    edited_classes.insert(class.replace('/', "."));
                }
            }
            set.removed.sort();
            let tallies: Vec<Tally> = compared.par_iter().map(|(f, rel)| compare_file(p, &bc, f, rel)).collect();
            let mut tally = Tally::default();
            for t in tallies {
                tally.add(t);
            }
            for (c, n) in counts.iter_mut().zip(tally.counts()) {
                c.add(n);
            }
            found.extend(tally.disagreements().map(|d| (shown(&sources.join(&d.file)), d.clone())));
            set.compared = compared.len();
            set.unreadable = bc.unreadable.clone();
            sets.push(set);
        }
        if !sets.is_empty() {
            modules.push(ModuleReport { dir: shown(&m.dir), build_system: m.build_system, sets, counts });
        }
    }
    // the classes declared in the files edited or added since the build, as
    // the sources declare them now
    for n in p.index.source_class_names() {
        if p.index.source_file_of(n).is_some_and(|f| edited_files.contains(Path::new(&**f))) {
            edited_classes.insert(n.to_string());
        }
    }
    let found = found
        .into_iter()
        .map(|(path, d)| {
            let possibly_stale = !edited_classes.is_empty() && d.concerns.iter().any(|c| with_supertypes(p, c).iter().any(|s| edited_classes.contains(s)));
            Found { path, disagreement: d, possibly_stale }
        })
        .collect();
    let mut counts = [Counts::default(); 5];
    for m in &modules {
        for (c, n) in counts.iter_mut().zip(m.counts) {
            c.add(n);
        }
    }
    Report { modules, counts, found }
}

/// `a/b/C.java` for a source file in package `a.b`, as a class file names
/// its source file.
fn package_path(p: &Project, f: &Path) -> Option<String> {
    let stub = p.index.file(&crate::project::path_key(f))?;
    let name = f.file_name()?.to_string_lossy();
    Some(if stub.package.is_empty() { name.to_string() } else { format!("{}/{name}", stub.package.replace('.', "/")) })
}

fn declares_a_class(p: &Project, f: &Path) -> bool {
    p.index.file(&crate::project::path_key(f)).is_some_and(|s| !s.classes.is_empty())
}

/// A class and its supertypes as grounds knows them: a member reached
/// through a class may be declared in any of them.
fn with_supertypes(p: &Project, class: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![class.to_string()];
    let mut queue: VecDeque<String> = VecDeque::from([class.to_string()]);
    while let Some(c) = queue.pop_front() {
        if out.len() > 256 {
            break;
        }
        let Some(info) = p.index.class(&c) else { continue };
        for s in info.superclass.iter().chain(&info.interfaces).filter_map(|t| t.class_name()) {
            if !out.iter().any(|o| o == s) {
                out.push(s.to_string());
                queue.push_back(s.to_string());
            }
        }
    }
    out
}

impl Report {
    /// Whether any module had build output to compare with.
    pub fn any_output(&self) -> bool {
        self.modules.iter().any(|m| m.sets.iter().any(|s| s.classes.is_some()))
    }

    pub fn edited_since_build(&self) -> usize {
        self.modules.iter().flat_map(|m| &m.sets).map(|s| s.edited.len() + s.added.len() + s.removed.len()).sum()
    }

    /// `Problems` when anything disagrees, possibly stale or not.
    pub fn outcome(&self) -> Outcome {
        if self.found.is_empty() {
            Outcome::Clean
        } else {
            Outcome::Problems
        }
    }

    pub fn text(&self) -> String {
        let mut o = String::new();
        let table = |o: &mut String, counts: &[Counts; 5]| {
            let _ = writeln!(o, "  {:<24}{:>10}{:>10}{:>10}{:>14}", "", "compared", "agree", "disagree", "not compared");
            for (label, c) in LABELS.iter().zip(counts) {
                let _ = writeln!(o, "  {label:<24}{:>10}{:>10}{:>10}{:>14}", c.compared, c.agree, c.disagree, c.not_compared);
            }
        };
        let list = |ps: &[PathBuf]| ps.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
        for m in &self.modules {
            let _ = writeln!(o, "module {} ({})", m.dir.display(), match m.build_system {
                BuildSystem::Gradle => "Gradle",
                BuildSystem::Maven => "Maven",
                BuildSystem::None => "no build file",
            });
            for s in &m.sets {
                let Some(classes) = &s.classes else {
                    let _ = writeln!(o, "  {}: {}: no build output found (looked in {})", s.name, s.sources.display(), list(&s.looked_in));
                    continue;
                };
                let mut line = format!("  {}: {} -> {}: {} file(s) compared", s.name, s.sources.display(), classes.display(), s.compared);
                if !s.edited.is_empty() {
                    let _ = write!(line, "; {} edited since the build: not compared", s.edited.len());
                }
                if !s.added.is_empty() {
                    let _ = write!(line, "; {} with no class file (added since the build?): not compared", s.added.len());
                }
                if !s.removed.is_empty() {
                    let _ = write!(line, "; class files of {} source file(s) no longer there", s.removed.len());
                }
                if !s.unreadable.is_empty() {
                    let _ = write!(line, "; {} class file(s) unreadable", s.unreadable.len());
                }
                let _ = writeln!(o, "{line}");
            }
            if m.sets.iter().any(|s| s.classes.is_some()) {
                table(&mut o, &m.counts);
            }
            o.push('\n');
        }
        if !self.any_output() {
            let _ = writeln!(o, "{HINT}");
            return o;
        }
        let _ = writeln!(o, "total");
        table(&mut o, &self.counts);
        o.push('\n');
        let changed: Vec<(&PathBuf, &str)> = self
            .modules
            .iter()
            .flat_map(|m| &m.sets)
            .flat_map(|s| s.edited.iter().map(|p| (p, "edited since the build: not compared")).chain(s.added.iter().map(|p| (p, "no class file (added since the build?): not compared"))).chain(s.removed.iter().map(|p| (p, "removed since the build"))))
            .collect();
        for (p, what) in &changed {
            let _ = writeln!(o, "{}: {what}", p.display());
        }
        if !changed.is_empty() {
            let _ = writeln!(o, "{} file(s) edited since the build; disagreements in other files may come from that\n", changed.len());
        }
        let section = |o: &mut String, title: &str, stale: bool| {
            let of: Vec<&Found> = self.found.iter().filter(|f| f.possibly_stale == stale).collect();
            if of.is_empty() {
                return;
            }
            let _ = writeln!(o, "{title}");
            for (cat, label) in CATEGORIES.iter().zip(LABELS) {
                let here: Vec<&&Found> = of.iter().filter(|f| f.disagreement.category == *cat).collect();
                if here.is_empty() {
                    continue;
                }
                let _ = writeln!(o, "{label} ({})", here.len());
                for f in here {
                    let _ = writeln!(o, "  {}", f.line());
                }
            }
            o.push('\n');
        };
        if self.found.is_empty() {
            let _ = writeln!(o, "no disagreement\n");
        }
        section(&mut o, "disagreements", false);
        section(&mut o, "possibly stale: about a class declared in a file edited since the build", true);
        let _ = writeln!(o, "{MEANING}");
        o
    }

    pub fn json(&self) -> serde_json::Value {
        use serde_json::json;
        let counts = |c: &[Counts; 5]| -> serde_json::Map<String, serde_json::Value> { CATEGORIES.iter().zip(c).map(|(k, c)| (k.to_string(), c.json())).collect() };
        let paths = |ps: &[PathBuf]| ps.iter().map(|p| p.display().to_string()).collect::<Vec<_>>();
        let modules: Vec<serde_json::Value> = self
            .modules
            .iter()
            .map(|m| {
                let sets: Vec<serde_json::Value> = m
                    .sets
                    .iter()
                    .map(|s| {
                        json!({
                            "name": s.name,
                            "sources": s.sources.display().to_string(),
                            "classes": s.classes.as_ref().map(|c| c.display().to_string()),
                            "looked_in": paths(&s.looked_in),
                            "compared": s.compared,
                            "edited": paths(&s.edited),
                            "added": paths(&s.added),
                            "removed": paths(&s.removed),
                            "unreadable": s.unreadable,
                        })
                    })
                    .collect();
                let bs = match m.build_system {
                    BuildSystem::Gradle => "gradle",
                    BuildSystem::Maven => "maven",
                    BuildSystem::None => "none",
                };
                json!({"dir": m.dir.display().to_string(), "build_system": bs, "source_sets": sets, "categories": counts(&m.counts)})
            })
            .collect();
        let found: Vec<serde_json::Value> = self
            .found
            .iter()
            .map(|f| {
                let d = &f.disagreement;
                json!({"category": d.category, "kind": d.kind, "path": f.path.display().to_string(), "line": d.line, "subject": d.subject, "member": d.member, "grounds": d.grounds, "class_file": d.javac, "possibly_stale": f.possibly_stale})
            })
            .collect();
        let mut out = json!({
            "modules": modules,
            "categories": counts(&self.counts),
            "edited_since_build": self.edited_since_build(),
            "disagreements": found,
        });
        if !self.any_output() {
            out["hint"] = json!(HINT);
        }
        out
    }
}

impl Found {
    /// `path:line: what grounds says, what the class file says`.
    pub fn line(&self) -> String {
        let d = &self.disagreement;
        let member = if d.member.is_empty() { String::new() } else { format!(" {}", d.member) };
        format!("{}:{}: {}{member}: grounds {}, the class file {} ({})", self.path.display(), d.line, d.subject, d.grounds, d.javac, d.kind)
    }
}
