//! A loaded project: workspace layout, classpath and declaration index.

use crate::cache::{content_hash, StubCache};
use crate::config::Config;
use crate::workspace::{SourceKind, SourceSets, Workspace};
use grounds_index::{extract, Classpath, FileStub, Index};
use grounds_syntax::ParsedFile;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Default, Clone)]
pub struct LoadStats {
    pub files: usize,
    pub parsed: usize,
    pub cached: usize,
    pub classpath_entries: usize,
    pub classpath_classes: usize,
    pub t_discover: Duration,
    pub t_classpath: Duration,
    pub t_stubs: Duration,
    pub t_index: Duration,
}

pub struct Project {
    pub config: Config,
    pub workspace: Workspace,
    pub index: Index,
    pub stats: LoadStats,
    /// Dependency-injection model (rebuilt when declarations change).
    pub di: grounds_di::DiModel,
    pub source_sets: SourceSets,
}

pub fn path_key(p: &Path) -> Arc<str> {
    Arc::from(p.to_string_lossy().as_ref())
}

impl Project {
    pub fn load(root: &Path) -> anyhow::Result<Project> {
        let config = Config::load(root)?;
        Self::load_with(root, config)
    }

    pub fn load_with(root: &Path, config: Config) -> anyhow::Result<Project> {
        let mut stats = LoadStats::default();
        let t = Instant::now();
        let source_sets = SourceSets::load(root);
        crate::workspace::set_extra_test_sets(config.test_source_sets.iter().cloned().chain(source_sets.test_sets()).collect());
        let workspace = Workspace::discover(root, &config.exclude);
        stats.t_discover = t.elapsed();
        stats.files = workspace.files.len();

        let t = Instant::now();
        let entries = crate::classpath::collect(&workspace, &config);
        stats.classpath_entries = entries.len();
        let java_home = config.java_home.clone().or_else(grounds_classfile::find_java_home);
        let classpath = Arc::new(Classpath::build(java_home.as_deref(), config.java_release, &entries));
        stats.classpath_classes = classpath.len();
        stats.t_classpath = t.elapsed();

        let t = Instant::now();
        let cache = StubCache::load(&workspace.root);
        let results: Vec<(PathBuf, u64, Arc<FileStub>, bool)> = workspace
            .files
            .par_iter()
            .filter_map(|f| {
                let text = std::fs::read_to_string(&f.path).ok()?;
                let h = content_hash(&text);
                let key = f.path.to_string_lossy();
                if let Some(s) = cache.get(&key, h) {
                    return Some((f.path.clone(), h, s, true));
                }
                let pf = ParsedFile::parse(text);
                Some((f.path.clone(), h, Arc::new(extract(&pf)), false))
            })
            .collect();
        stats.t_stubs = t.elapsed();

        let t = Instant::now();
        let mut index = Index::new(classpath);
        let mut new_cache = StubCache::load(&workspace.root);
        new_cache.retain(|_| false);
        for (p, h, stub, was_cached) in results {
            if was_cached {
                stats.cached += 1;
            } else {
                stats.parsed += 1;
            }
            new_cache.put(p.to_string_lossy().to_string(), h, stub.clone());
            index.set_file(path_key(&p), stub);
        }
        stats.t_index = t.elapsed();
        if stats.parsed > 0 {
            let _ = new_cache.save(&workspace.root);
        }
        let mut di = grounds_di::DiModel::build(&index);
        di.lombok_copies_qualifiers = lombok_copies_qualifiers(&workspace.root);
        Ok(Project { config, workspace, index, stats, di, source_sets })
    }

    pub fn source_kind(&self, path: &Path) -> SourceKind {
        crate::workspace::classify(path)
    }

    /// Re-read one file and update the index. Returns true if its
    /// declaration signature changed.
    pub fn refresh_file(&mut self, path: &Path) -> bool {
        let key = path_key(path);
        let old = self.index.file(&key).map(|s| s.signature_hash);
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let pf = ParsedFile::parse(text);
                let stub = Arc::new(extract(&pf));
                let changed = old != Some(stub.signature_hash);
                if changed {
                    self.index.set_file(key, stub);
                    self.rebuild_di();
                }
                changed
            }
            Err(_) => {
                self.index.remove_file(&key);
                self.rebuild_di();
                old.is_some()
            }
        }
    }

    pub fn rebuild_di(&mut self) {
        let copies = self.di.lombok_copies_qualifiers;
        self.di = grounds_di::DiModel::build(&self.index);
        self.di.lombok_copies_qualifiers = copies;
    }
}

/// Does a lombok.config in the workspace make Lombok copy @Qualifier/@Value?
fn lombok_copies_qualifiers(root: &Path) -> bool {
    let mut stack = vec![root.to_path_buf()];
    let mut depth = 0;
    while let Some(d) = stack.pop() {
        let cfg = d.join("lombok.config");
        if let Ok(t) = std::fs::read_to_string(&cfg) {
            if t.lines().any(|l| l.contains("copyableAnnotations") && (l.contains("Qualifier") || l.contains("Value"))) {
                return true;
            }
        }
        depth += 1;
        if depth > 200 {
            break;
        }
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if p.is_dir() && !name.starts_with('.') && !matches!(name.as_str(), "build" | "target" | "node_modules" | "out" | "src") {
                    stack.push(p);
                }
            }
        }
    }
    false
}
