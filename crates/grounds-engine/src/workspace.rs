//! Workspace discovery: source files, modules and source sets, without
//! running any build tool.

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
}

const SKIP_DIRS: &[&str] = &["node_modules", ".git", ".gradle", ".idea", "out", ".grounds", "bin"];

pub fn classify(path: &Path) -> SourceKind {
    let s = path.to_string_lossy();
    if s.contains("/build/generated") || s.contains("/target/generated-sources") || s.contains("/generated-sources/") {
        return SourceKind::Generated;
    }
    let comps: Vec<&str> = path.components().filter_map(|c| c.as_os_str().to_str()).collect();
    for w in comps.windows(2) {
        if w[0] == "src" {
            let set = w[1].to_ascii_lowercase();
            if set.contains("test") || set == "it" || set.contains("integration") || set.contains("jmh") {
                return SourceKind::Test;
            }
        }
    }
    SourceKind::Main
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
                    let s = p.to_string_lossy();
                    // skip non-generated build outputs
                    if (s.contains("/build/") || s.contains("/target/")) && classify(p) != SourceKind::Generated {
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
