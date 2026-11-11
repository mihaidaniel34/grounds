//! Workspace discovery: source files, modules and source sets, without
//! running any build tool.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SourceKind {
    Main,
    Test,
    /// Generated sources (annotation processor output); indexed, not analyzed.
    Generated,
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    pub module: usize,
    pub kind: SourceKind,
}

#[derive(Clone, Debug)]
pub struct Module {
    pub dir: PathBuf,
    pub name: String,
    pub build_system: BuildSystem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildSystem {
    Gradle,
    Maven,
    None,
}

#[derive(Debug)]
pub struct Workspace {
    pub root: PathBuf,
    pub modules: Vec<Module>,
    pub files: Vec<SourceFile>,
    /// The directories the discovery went into: those whose entries it
    /// looked at. Adding, removing or renaming an entry changes a directory's
    /// time, so they are where a source file that appears, or comes back, is
    /// noticed (see `Session::sync`).
    pub dirs: Vec<PathBuf>,
}

/// The files in a directory that decide which files the discovery skips,
/// besides `.git/info/exclude` and git's global excludes file.
pub const IGNORE_FILES: &[&str] = &[".gitignore", ".ignore"];

/// The ignore files that apply to a discovery of `root` besides those in the
/// directories it goes into: those of the directories above it and git's
/// `info/exclude`, whether they exist or not.
pub fn outer_ignore_files(root: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for d in root.ancestors() {
        if d != root {
            v.extend(IGNORE_FILES.iter().map(|n| d.join(n)));
        }
        v.extend(repo_inputs(d));
    }
    v
}

/// What makes `d` the top of a repository for `ignore`, whose ignore files
/// count only inside one: a `.git` directory (1), a `.git` file naming a
/// gitdir (2), a `.jj` (4); `None` for none of them. (Through symlinks, as
/// `ignore` looks.)
pub fn boundary(d: &Path) -> Option<u8> {
    let git = std::fs::metadata(d.join(".git")).ok().map_or(0, |m| if m.is_dir() { 1 } else { 2 });
    let jj = if d.join(".jj").exists() { 4 } else { 0 };
    (git | jj != 0).then_some(git | jj)
}

/// The files a repository at `d` reads its excludes through: its
/// `info/exclude` and, for a `.git` file, that file and the `commondir` of the
/// gitdir it names, whether they exist or not.
pub fn repo_inputs(d: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let dot = d.join(".git");
    if std::fs::metadata(&dot).is_ok_and(|m| m.is_file()) {
        if let Some(gitdir) = gitdir_of(&dot) {
            v.push(gitdir.join("commondir"));
        }
        v.push(dot);
    }
    v.extend(exclude_file(d));
    v
}

/// The gitdir a `.git` file names, as `ignore` reads it.
fn gitdir_of(dot: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dot).ok()?;
    Some(PathBuf::from(text.lines().next()?.strip_prefix("gitdir: ")?))
}

/// The `info/exclude` that applies to a repository at `d` (see `git_common_dir`).
pub fn exclude_file(d: &Path) -> Option<PathBuf> {
    git_common_dir(d).map(|c| c.join("info/exclude"))
}

/// Whether the entries of directory `d` can be listed and looked at: a
/// directory that can be read but not entered (no execute permission) lists
/// names whose files cannot be reached. (The permission is the directory's,
/// so its first entry tells.)
pub fn listable(d: &Path) -> bool {
    match std::fs::read_dir(d) {
        Ok(mut entries) => match entries.next() {
            Some(Ok(e)) => std::fs::symlink_metadata(e.path()).is_ok(),
            Some(Err(_)) => false,
            None => true,
        },
        Err(_) => false,
    }
}

/// The git directory whose `info/exclude` applies to a repository at `d`, as
/// `ignore` finds it: `d/.git`, or for a linked worktree (`.git` a file
/// naming its `gitdir:`) the common directory that gitdir's `commondir` names.
/// (A submodule's gitdir has no `commondir`: `ignore` then reads no exclude
/// file for it, and neither is one watched.)
fn git_common_dir(d: &Path) -> Option<PathBuf> {
    let dot = d.join(".git");
    let meta = std::fs::metadata(&dot).ok()?;
    if meta.is_dir() {
        return Some(dot);
    }
    let gitdir = gitdir_of(&dot)?;
    let common = std::fs::read_to_string(gitdir.join("commondir")).ok()?.lines().next()?.to_string();
    Some(if common.starts_with('.') { gitdir.join(common) } else { PathBuf::from(common) })
}

const SKIP_DIRS: &[&str] = &["node_modules", ".git", ".gradle", ".idea", "out", ".grounds", "bin"];

pub use grounds_index::sources::{in_test_module, is_test_path, is_test_set, register_project_root, set_extra_test_sets, source_set_of};

/// A file under the resources of a source set (`src/<set>/resources/**`),
/// `rel` relative to the project root. Such Java files are data that tests
/// read or copy (a whole fixture project, a stub of a library class), not
/// sources of the build: they are neither analyzed nor indexed.
pub fn is_resource(rel: &Path) -> bool {
    let comps: Vec<&std::ffi::OsStr> = rel.components().map(|c| c.as_os_str()).collect();
    comps.windows(3).any(|w| w[0] == "src" && w[2] == "resources")
}

pub fn classify(path: &Path) -> SourceKind {
    let s = path.to_string_lossy();
    if s.contains("/build/generated") || s.contains("/target/generated-sources") || s.contains("/generated-sources/") {
        return SourceKind::Generated;
    }
    match source_set_of(path) {
        Some(set) if is_test_set(set) => SourceKind::Test,
        // `src/main` of a module that exists only for testing
        _ if in_test_module(path) => SourceKind::Test,
        _ => SourceKind::Main,
    }
}

/// Source sets reported by the build (`.grounds/source-sets.txt`, written by
/// `grounds classpath --gradle`): which `Test` tasks run each source set.
#[derive(Debug, Default, Clone)]
pub struct SourceSets {
    /// (module name, source set) -> test task names
    tasks: HashMap<(String, String), Vec<String>>,
}

impl SourceSets {
    pub fn load(root: &Path) -> SourceSets {
        let mut tasks = HashMap::new();
        if let Ok(text) = crate::fsutil::read_text(root.join(".grounds/source-sets.txt")) {
            for line in text.lines() {
                let mut parts = line.split('\t');
                let (Some(module), Some(set)) = (parts.next(), parts.next()) else { continue };
                let module = if module == "." || module.is_empty() { ":".to_string() } else { module.to_string() };
                let ts: Vec<String> = parts.next().unwrap_or("").split(',').filter(|t| !t.is_empty()).map(String::from).collect();
                tasks.insert((module, set.to_string()), ts);
            }
        }
        SourceSets { tasks }
    }

    /// Source sets that some test task runs.
    pub fn test_sets(&self) -> Vec<String> {
        self.tasks.iter().filter(|(_, t)| !t.is_empty()).map(|((_, s), _)| s.clone()).collect()
    }

    /// The Gradle test task for a source set: as reported by the build, else
    /// the convention (`test` for `test`, the suite name for JVM test suites).
    pub fn test_task(&self, module: &str, set: &str) -> String {
        match self.tasks.get(&(module.to_string(), set.to_string())) {
            Some(t) if t.iter().any(|t| t == set) => set.to_string(),
            Some(t) if !t.is_empty() => t[0].clone(),
            _ => set.to_string(),
        }
    }
}

impl Workspace {
    pub fn discover(root: &Path, exclude: &[String]) -> Workspace {
        Self::discover_in(root, None, None, exclude)
    }

    /// The discovery of the workspace at `root` in one of its directories,
    /// `start` (canonical), and `max_depth` levels below it: the source files
    /// and build files there, as `discover` finds them (with the ignore files
    /// of `start` and the directories above it), and the directories it went
    /// into.
    pub fn discover_in(root: &Path, start: Option<&Path>, max_depth: Option<usize>, exclude: &[String]) -> Workspace {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mut modules: Vec<Module> = Vec::new();
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        let excludes: Vec<glob::Pattern> = exclude.iter().filter_map(|g| glob::Pattern::new(g).ok()).collect();

        let walker = ignore::WalkBuilder::new(start.unwrap_or(&root))
            .hidden(true)
            .git_ignore(true)
            .git_exclude(true)
            .parents(true)
            .max_depth(max_depth)
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                if e.file_type().is_some_and(|t| t.is_dir()) {
                    if SKIP_DIRS.contains(&name.as_ref()) {
                        return false;
                    }
                    // build output except generated sources
                    if name == "build" || name == "target" {
                        return true;
                    }
                }
                true
            })
            .build();
        let mut module_dirs: Vec<(PathBuf, BuildSystem)> = Vec::new();
        let mut java_files: Vec<PathBuf> = Vec::new();
        for entry in walker.flatten() {
            let p = entry.path();
            if entry.file_type().is_some_and(|t| t.is_dir()) {
                dirs.push(p.to_path_buf());
            }
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
            if entry.file_type().is_some_and(|t| t.is_file()) {
                match name {
                    "build.gradle" | "build.gradle.kts" => module_dirs.push((p.parent().unwrap().to_path_buf(), BuildSystem::Gradle)),
                    "pom.xml" => module_dirs.push((p.parent().unwrap().to_path_buf(), BuildSystem::Maven)),
                    _ => {}
                }
                if name.ends_with(".java") && name != "module-info.java" && name != "package-info.java" {
                    // skip non-generated build outputs: a `build`/`target` directory
                    // inside the project and above `src` (not the folders the
                    // project sits in, nor a package named `build`)
                    let rel = p.strip_prefix(&root).unwrap_or(p);
                    let output = rel.components().map(|c| c.as_os_str()).take_while(|c| *c != "src").any(|c| c == "build" || c == "target");
                    if output && classify(p) != SourceKind::Generated {
                        continue;
                    }
                    if let Ok(rel) = p.strip_prefix(&root) {
                        if is_resource(rel) || excludes.iter().any(|g| g.matches_path(rel)) {
                            continue;
                        }
                    }
                    java_files.push(p.to_path_buf());
                }
            }
        }
        module_dirs.sort_by(|a, b| a.0.cmp(&b.0));
        module_dirs.dedup_by(|a, b| a.0 == b.0);
        if !module_dirs.iter().any(|(d, _)| *d == root) {
            module_dirs.push((root.clone(), BuildSystem::None));
        }
        for (d, bs) in &module_dirs {
            let name = d.strip_prefix(&root).map(|r| r.to_string_lossy().to_string()).unwrap_or_default();
            modules.push(Module { dir: d.clone(), name: if name.is_empty() { ":".into() } else { name }, build_system: *bs });
        }
        // longest module dir prefix wins
        let mut order: Vec<usize> = (0..modules.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(modules[i].dir.as_os_str().len()));
        java_files.sort();
        for p in java_files {
            let module = order.iter().copied().find(|&i| p.starts_with(&modules[i].dir)).unwrap_or(0);
            let kind = classify(&p);
            files.push(SourceFile { path: p, module, kind });
        }
        Workspace { root, modules, files, dirs }
    }

    pub fn module_of(&self, path: &Path) -> Option<usize> {
        let mut best: Option<(usize, usize)> = None;
        for (i, m) in self.modules.iter().enumerate() {
            if path.starts_with(&m.dir) {
                let l = m.dir.as_os_str().len();
                if best.is_none_or(|(_, bl)| l > bl) {
                    best = Some((i, l));
                }
            }
        }
        best.map(|b| b.0)
    }
}
