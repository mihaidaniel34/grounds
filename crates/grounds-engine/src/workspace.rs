//! Workspace discovery: source files, modules and source sets, without
//! running any build tool.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

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
}

const SKIP_DIRS: &[&str] = &["node_modules", ".git", ".gradle", ".idea", "out", ".grounds", "bin"];

/// Source set names known to hold tests besides the naming heuristics: from
/// `test_source_sets` in grounds.toml and from the source set map written by
/// `grounds classpath` (source sets some `Test` task runs).
static EXTRA_TEST_SETS: RwLock<Vec<String>> = RwLock::new(Vec::new());

pub fn set_extra_test_sets(names: Vec<String>) {
    if let Ok(mut w) = EXTRA_TEST_SETS.write() {
        *w = names.into_iter().map(|n| n.to_ascii_lowercase()).collect();
    }
}

/// The source set a file belongs to (`main`, `test`, `integrationTest`, ...),
/// i.e. the directory after the last `src`.
pub fn source_set_of(path: &Path) -> Option<&str> {
    let comps: Vec<&str> = path.components().filter_map(|c| c.as_os_str().to_str()).collect();
    comps.windows(2).rev().find(|w| w[0] == "src").map(|w| w[1])
}

fn is_test_set(set: &str) -> bool {
    let set = set.to_ascii_lowercase();
    const HINTS: &[&str] = &["test", "integration", "jmh", "e2e", "functional", "acceptance", "contract", "benchmark"];
    set == "it" || HINTS.iter().any(|h| set.contains(h)) || EXTRA_TEST_SETS.read().is_ok_and(|e| e.contains(&set))
}

pub fn classify(path: &Path) -> SourceKind {
    let s = path.to_string_lossy();
    if s.contains("/build/generated") || s.contains("/target/generated-sources") || s.contains("/generated-sources/") {
        return SourceKind::Generated;
    }
    match source_set_of(path) {
        Some(set) if is_test_set(set) => SourceKind::Test,
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
        if let Ok(text) = std::fs::read_to_string(root.join(".grounds/source-sets.txt")) {
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
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mut modules: Vec<Module> = Vec::new();
        let mut files = Vec::new();
        let excludes: Vec<glob::Pattern> = exclude.iter().filter_map(|g| glob::Pattern::new(g).ok()).collect();

        let walker = ignore::WalkBuilder::new(&root)
            .hidden(true)
            .git_ignore(true)
            .git_exclude(true)
            .parents(true)
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
                        if excludes.iter().any(|g| g.matches_path(rel)) {
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
        Workspace { root, modules, files }
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
