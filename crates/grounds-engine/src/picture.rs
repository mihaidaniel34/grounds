//! The build picture: which source files the build compiles, from evidence,
//! without running the build. A claim that code does not compile is only
//! worth making in a file the build compiles; test data, samples and stale
//! modules sit next to the code with the conventional layout and do not
//! compile, and nothing builds them. Each source file is in one of three
//! states (`grounds_index::InBuild`): in the build, not in the build, or
//! unknown.
//!
//! The evidence is a module's build output: the directories Maven and
//! Gradle compile Java into (`target/classes`, `target/test-classes`,
//! `build/classes/<language>/<source set>`), read for the names of their
//! class files and their times. Nothing is run. The rules:
//!
//! - A file belongs to its nearest enclosing module (the workspace's). Its
//!   source root is its directory less the package's directories, when they
//!   spell its package: such files establish the module's roots. A build
//!   compiles directory trees, and a package other than the directories (or
//!   the default package below a root) is legal Java, so a file whose
//!   package does not spell its directories belongs to the nearest root
//!   enclosing it that is established otherwise: a source set's
//!   conventional directory of the module (`src/<set>/java`), or a root a
//!   matching file establishes. Only when there is none is its directory its
//!   root. (A compact source file, with methods at the top level, declares
//!   an implicit class named after it, which the stubs do not record: it is
//!   no evidence for its root, and takes its state from the enclosing one.)
//! - A root is **in the build** when its module's output holds the class file
//!   of a top-level class declared in that root and in no other root of the
//!   module, or, for a source set's conventional directory
//!   (`src/<set>/java`), of a class it declares in that set's own output
//!   directory. Every file of such a root is in the build, also one newer
//!   than every class file or without one of its own (a file just created or
//!   edited: the build compiles the root, so it compiles the file).
//! - A class declared in several roots of a module says nothing more (test
//!   data copying a class of the module): a root whose classes with class
//!   files are all declared in roots that are in the build has no class file
//!   of its own; one with such a class otherwise is unknown.
//! - A root with no class file of its own, in a module whose output holds
//!   class files, is **not in the build** for a file older than the newest
//!   class file of the module (it was there when the build ran and was not
//!   compiled), and **unknown** for a newer one (a source root added since).
//!   Except a root that is a source set's conventional directory
//!   (`src/<set>/java`) whose own output directory holds no class file: that
//!   set was never compiled, or its compilation failed (tests that do not
//!   compile while main code does), and it is unknown.
//! - A module whose output holds no class file (never built, cleaned, a
//!   failed first build), or too many files to read, is unknown throughout.
//!   So is a project with no build output at all: grounds then guesses the
//!   build as it always has.
//!
//! Main and test output count alike for the evidence (one module's tests may
//! shadow a main class; the unique-class rule keeps them apart), and so do
//! generated sources under the output tree (`target/generated-sources`),
//! whose classes are compiled with the rest. Class files left from an old
//! build count as long as they are there: a root whose classes they name is
//! in the build. Times compare as the file system gives them; a file as old
//! as the newest class file is unknown, not older.
//!
//! A session follows the evidence (`Picture::refresh`): the output
//! directories are stamped as they are read, before each is listed, so a
//! build that adds or removes class files, or makes or deletes an output
//! directory, is noticed at the next sync by their times. A build that only
//! rewrites class files in place moves no directory's time; that changes the
//! newest class file's time, which matters only to files newer than it in a
//! root with no class file, so a module that has such files is read again
//! at every sync until it has none.

use crate::project::FileStamp;
use crate::workspace::Workspace;
use grounds_index::{FileStub, InBuild};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// What one module's build output shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Output {
    /// The top-level classes it holds a class file of, as `a/b/C`, with the
    /// source sets whose output directory holds it.
    classes: FxHashMap<Box<str>, Vec<Box<str>>>,
    /// The newest class file's time; `None`: no class file.
    newest: Option<SystemTime>,
    /// The source sets whose own output directory holds a class file:
    /// `main` (`target/classes`), `test` (`target/test-classes`), a Gradle
    /// set by name (`build/classes/java/<set>`).
    sets: FxHashSet<String>,
    /// More files than are read (`grounds_classfile::DIR_LIMIT`): no evidence.
    oversized: bool,
}

impl Output {
    /// Whether the output is evidence at all.
    fn evidence(&self) -> bool {
        self.newest.is_some() && !self.oversized
    }
}

/// A module's output, and the directories it was read from, each stamped
/// before it was listed (`None`: not there).
#[derive(Clone, Debug, Default)]
struct ModuleOutput {
    output: Output,
    stamps: Vec<(PathBuf, Option<FileStamp>)>,
}

/// What a source root's own class files show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Root {
    /// It is compiled.
    Compiled,
    /// The module was built, and no class file is of this root: a file
    /// older than the build was not compiled.
    NoClassFile,
    /// No evidence either way.
    Unknown,
}

/// The build picture of a workspace: each module's output, and what it shows
/// of each source root.
#[derive(Clone, Debug, Default)]
pub struct Picture {
    /// Per module, in the workspace's order.
    modules: Vec<ModuleOutput>,
    /// Per (module, source root).
    roots: FxHashMap<(usize, PathBuf), Root>,
    /// Per module, the source roots files whose package spells their
    /// directories establish (see `root_of`).
    established: FxHashMap<usize, FxHashSet<PathBuf>>,
    /// The modules with a file newer than their newest class file in a root
    /// without one: read again at every sync (see the module doc).
    pending: FxHashSet<usize>,
}

/// The output directories of a module directory, with the source set each
/// is the output of. `build/classes` holds one directory per language, and
/// one per set in each.
const MAVEN_OUTPUTS: [(&str, &str); 2] = [("target/classes", "main"), ("target/test-classes", "test")];
const GRADLE_OUTPUTS: &str = "build/classes";

impl Picture {
    /// Read the build output of every module of `ws`.
    pub fn read(ws: &Workspace) -> Picture {
        use rayon::prelude::*;
        let modules = ws.modules.par_iter().map(|m| read_module(&m.dir)).collect();
        Picture { modules, roots: FxHashMap::default(), established: FxHashMap::default(), pending: FxHashSet::default() }
    }

    /// Whether any module has build output to go by.
    pub fn any_evidence(&self) -> bool {
        self.modules.iter().any(|m| m.output.evidence())
    }

    /// How many modules holding source files have no build output to go
    /// by: their picture of the build is guessed.
    pub fn guessed_modules(&self, ws: &Workspace) -> usize {
        let with_code: FxHashSet<usize> = ws.files.iter().filter(|f| f.kind != crate::workspace::SourceKind::Generated).map(|f| f.module).collect();
        with_code.into_iter().filter(|m| !self.module_has_evidence(*m)).count()
    }

    /// Whether module `m` has build output to go by.
    pub fn module_has_evidence(&self, m: usize) -> bool {
        self.modules.get(m).is_some_and(|m| m.output.evidence())
    }

    /// Read again the output of each module whose directories changed since
    /// they were read (`clock`: the time before this look), or that has files
    /// newer than its newest class file in a root without one. Whether what
    /// any of them shows changed; the states are then to be placed again.
    pub fn refresh(&mut self, ws: &Workspace, clock: SystemTime) -> bool {
        let mut changed = false;
        for (i, m) in self.modules.iter_mut().enumerate() {
            let moved = self.pending.contains(&i)
                || m.stamps.iter_mut().any(|(p, s)| match s {
                    Some(s) => s.changed(p, clock),
                    None => p.exists(),
                });
            if !moved {
                continue;
            }
            let Some(dir) = ws.modules.get(i).map(|w| &w.dir) else { continue };
            let again = read_module(dir);
            crate::trace::log(|| format!("sync: build output of {} read again", dir.display()));
            changed |= again.output != m.output;
            *m = again;
        }
        changed
    }

    /// The state of every source file of `ws` (`stub`: its stub, `mtime`:
    /// its time as read), and what the roots show, kept for `place_file`.
    pub fn place(&mut self, ws: &Workspace, stub: impl Fn(&Path) -> Option<Arc<FileStub>>, mtime: impl Fn(&Path) -> Option<SystemTime>) -> FxHashMap<Arc<str>, InBuild> {
        self.roots.clear();
        self.established.clear();
        self.pending.clear();
        if !self.any_evidence() {
            return FxHashMap::default();
        }
        let files: Vec<(usize, &Path, Option<Arc<FileStub>>)> = ws.files.iter().filter(|f| self.module_has_evidence(f.module)).map(|f| (f.module, f.path.as_path(), stub(&f.path))).collect();
        // the roots files whose package spells their directories establish
        for (m, path, stub) in &files {
            if let Some(root) = stub.as_deref().and_then(|s| package_root(path, &s.package)) {
                self.established.entry(*m).or_default().insert(root);
            }
        }
        // the top-level classes each root of a module declares, as `a/b/C`
        let mut declares: FxHashMap<(usize, PathBuf), FxHashSet<String>> = FxHashMap::default();
        let mut file_root: Vec<(usize, &Path, PathBuf)> = Vec::new();
        for (m, path, stub) in files {
            let root = self.root_of(ws, m, path, stub.as_deref().map_or("", |s| &s.package));
            let classes = declares.entry((m, root.clone())).or_default();
            for c in stub.iter().flat_map(|s| s.classes.iter()).filter(|c| c.outer.is_none()) {
                classes.insert(c.name.replace('.', "/"));
            }
            file_root.push((m, path, root));
        }
        // in how many roots of its module each class is declared
        let mut count: FxHashMap<(usize, &str), u32> = FxHashMap::default();
        for ((m, _), classes) in &declares {
            for c in classes {
                *count.entry((*m, c.as_str())).or_default() += 1;
            }
        }
        let output = |m: usize| &self.modules[m].output;
        // a root is compiled when the output holds the class file of a class
        // only it declares, or, for a source set's conventional directory, of
        // a class it declares in that set's own output (a module's tests may
        // declare a main class again; test data may copy one)
        let compiled: FxHashSet<&(usize, PathBuf)> = declares
            .iter()
            .filter(|((m, root), classes)| {
                let set = conventional_set(&ws.modules[*m].dir, root);
                classes.iter().any(|c| match output(*m).classes.get(c.as_str()) {
                    Some(sets) => count[&(*m, c.as_str())] == 1 || set.is_some_and(|set| sets.iter().any(|s| **s == *set)),
                    None => false,
                })
            })
            .map(|(k, _)| k)
            .collect();
        // the classes the roots in the build declare, per module
        let mut explained: FxHashSet<(usize, &str)> = FxHashSet::default();
        for k in &compiled {
            for c in &declares[*k] {
                explained.insert((k.0, c.as_str()));
            }
        }
        let mut roots: FxHashMap<(usize, PathBuf), Root> = FxHashMap::default();
        for (key, classes) in &declares {
            let m = key.0;
            // a class file of its classes that no root in the build explains,
            // or a source set whose own output holds no class file: unknown
            let unexplained = classes.iter().any(|c| output(m).classes.contains_key(c.as_str()) && !explained.contains(&(m, c.as_str())));
            let set_not_compiled = conventional_set(&ws.modules[m].dir, &key.1).is_some_and(|set| !output(m).sets.contains(set));
            let state = if compiled.contains(key) {
                Root::Compiled
            } else if unexplained || set_not_compiled {
                Root::Unknown
            } else {
                Root::NoClassFile
            };
            roots.insert(key.clone(), state);
        }
        self.roots = roots;
        let mut states = FxHashMap::default();
        for (m, path, root) in file_root {
            let state = self.state(m, &root, mtime(path));
            states.insert(crate::project::path_key(path), state);
        }
        states
    }

    /// The state of one source file of module `m`, after a change that left
    /// its module's roots as they were (an edit that changed no declaration):
    /// only its own time may differ.
    pub fn place_file(&mut self, ws: &Workspace, m: usize, path: &Path, stub: Option<&FileStub>, mtime: Option<SystemTime>) -> InBuild {
        if !self.module_has_evidence(m) {
            return InBuild::Unknown;
        }
        let root = self.root_of(ws, m, path, stub.map_or("", |s| &s.package));
        self.state(m, &root, mtime)
    }

    /// The source root of the file at `path` of module `m`, in package
    /// `package`, as the roots were last placed: its directory less the
    /// package's directories when they spell the package. A build compiles
    /// directory trees, and a package other than the directories (or the
    /// default package below a root) is legal Java: such a file belongs to
    /// the nearest root enclosing it that is established otherwise, a source
    /// set's conventional directory of its module (`src/<set>/java`) or a
    /// root that a file whose package spells its directories establishes.
    /// Only when there is none is its directory its root.
    pub fn root_of(&self, ws: &Workspace, m: usize, path: &Path, package: &str) -> PathBuf {
        if let Some(root) = package_root(path, package) {
            return root;
        }
        let dir = path.parent().unwrap_or(Path::new(""));
        let Some(module) = ws.modules.get(m).map(|w| w.dir.as_path()) else { return dir.to_path_buf() };
        let established = self.established.get(&m);
        dir.ancestors()
            .take_while(|a| a.starts_with(module))
            .find(|a| established.is_some_and(|e| e.contains(*a)) || conventional_set(module, a).is_some())
            .unwrap_or(dir)
            .to_path_buf()
    }

    fn state(&mut self, m: usize, root: &Path, mtime: Option<SystemTime>) -> InBuild {
        match self.roots.get(&(m, root.to_path_buf())) {
            Some(Root::Compiled) => InBuild::Yes,
            Some(Root::NoClassFile) => match (mtime, self.modules[m].output.newest) {
                (Some(t), Some(newest)) if t < newest => InBuild::No,
                _ => {
                    self.pending.insert(m);
                    InBuild::Unknown
                }
            },
            Some(Root::Unknown) | None => InBuild::Unknown,
        }
    }
}

/// The warning for a run whose compile-break findings include some made
/// where the build picture is unknown (`Finding::build`), once per run: how
/// many, in how many modules. None when there are none, or classpath
/// warnings are off (`[classpath] warn = false`).
pub fn guessed<'f>(project: &crate::Project, findings: impl IntoIterator<Item = &'f crate::analyze::Finding>) -> Option<crate::classpath::Warning> {
    if !project.config.classpath.warn {
        return None;
    }
    let ws = &project.workspace;
    let mut modules = FxHashSet::default();
    let mut count = 0;
    for f in findings.into_iter().filter(|f| f.build == Some(InBuild::Unknown)) {
        count += 1;
        modules.insert(ws.module_of(&ws.root.join(&f.path)).unwrap_or(0));
    }
    (count > 0).then_some(crate::classpath::Warning::GuessedBuild { modules: modules.len(), findings: count })
}

/// The source root a file in package `package` establishes: its directory
/// less the package's directories, when they spell the package. None for
/// the default package, and for directories that do not match.
fn package_root(path: &Path, package: &str) -> Option<PathBuf> {
    if package.is_empty() {
        return None;
    }
    let mut root = path.parent()?;
    for seg in package.rsplit('.') {
        match (root.file_name(), root.parent()) {
            (Some(n), Some(up)) if n == seg => root = up,
            _ => return None,
        }
    }
    Some(root.to_path_buf())
}

/// The source set whose conventional directory `root` is in module `module`
/// (`<module>/src/<set>/<java|kotlin|groovy|scala>`).
fn conventional_set<'a>(module: &Path, root: &'a Path) -> Option<&'a str> {
    let lang = root.file_name()?.to_str()?;
    if !matches!(lang, "java" | "kotlin" | "groovy" | "scala") {
        return None;
    }
    let set_dir = root.parent()?;
    let src = set_dir.parent()?;
    (src.file_name()? == "src" && src.parent()? == module).then(|| set_dir.file_name()?.to_str())?
}

/// Read a module's output directories: the class files' names and times,
/// each directory stamped before it is listed.
fn read_module(dir: &Path) -> ModuleOutput {
    let mut out = ModuleOutput::default();
    let mut files = 0usize;
    for (rel, set) in MAVEN_OUTPUTS {
        read_tree(&dir.join(rel), &mut |_| Some(set.to_string()), &mut out, &mut files);
    }
    let gradle = dir.join(GRADLE_OUTPUTS);
    // build/classes/<language>/<set>/...
    let set_of = |p: &Path| p.strip_prefix(&gradle).ok().and_then(|r| r.components().nth(1)).map(|c| c.as_os_str().to_string_lossy().to_string());
    read_tree(&gradle, &mut |p| set_of(p), &mut out, &mut files);
    out
}

/// Walk the output tree at `top`: every directory stamped before it is
/// listed (`top` also when it is not there), every class file counted.
fn read_tree(top: &Path, set_of: &mut dyn FnMut(&Path) -> Option<String>, out: &mut ModuleOutput, files: &mut usize) {
    let mut stack = vec![top.to_path_buf()];
    while let Some(d) = stack.pop() {
        let stamp = FileStamp::of(&d, false);
        let present = stamp.is_some();
        out.stamps.push((d.clone(), stamp));
        if !present {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            *files += 1;
            if *files > grounds_classfile::DIR_LIMIT {
                out.output.oversized = true;
                return;
            }
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                stack.push(p);
                continue;
            }
            let name = e.file_name();
            let Some(name) = name.to_str().and_then(|n| n.strip_suffix(".class")) else { continue };
            if matches!(name, "module-info" | "package-info") {
                continue;
            }
            let Ok(t) = e.metadata().and_then(|m| m.modified()) else { continue };
            let o = &mut out.output;
            o.newest = o.newest.max(Some(t));
            let set = set_of(&p);
            if !name.contains('$') {
                // the class's path below the set's directory: `a/b/C`
                if let Some(class) = class_path(top, &p, name) {
                    let sets = o.classes.entry(class.into()).or_default();
                    if let Some(set) = &set {
                        if !sets.iter().any(|s| **s == **set) {
                            sets.push(set.as_str().into());
                        }
                    }
                }
            }
            if let Some(set) = set {
                o.sets.insert(set);
            }
        }
    }
}

/// `a/b/C` for the class file `<set dir>/a/b/C.class` under `top`: below
/// `target/classes` directly, below `build/classes/<language>/<set>`.
fn class_path(top: &Path, file: &Path, name: &str) -> Option<String> {
    let rel = file.parent()?.strip_prefix(top).ok()?;
    let skip = if top.ends_with(GRADLE_OUTPUTS) { 2 } else { 0 };
    let mut parts: Vec<String> = rel.components().skip(skip).map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    if top.ends_with(GRADLE_OUTPUTS) && rel.components().count() < 2 {
        return None;
    }
    parts.push(name.to_string());
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_root_is_the_directory_less_the_package() {
        assert_eq!(package_root(Path::new("/m/src/main/java/a/b/C.java"), "a.b").as_deref(), Some(Path::new("/m/src/main/java")));
        // the default package, and a package the directories do not spell,
        // establish no root
        assert_eq!(package_root(Path::new("/m/data/C.java"), ""), None);
        assert_eq!(package_root(Path::new("/m/data/x/C.java"), "a.b"), None);
        assert_eq!(conventional_set(Path::new("/m"), Path::new("/m/src/test/java")), Some("test"));
        assert_eq!(conventional_set(Path::new("/m"), Path::new("/m/src/test/test_sourcecode")), None);
        assert_eq!(conventional_set(Path::new("/m"), Path::new("/m/sub/src/main/java")), None);
    }
}
