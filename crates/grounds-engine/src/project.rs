//! A loaded project: workspace layout, classpath and declaration index.

use crate::cache::{content_hash, StubCache};
use crate::config::Config;
use crate::workspace::{SourceKind, SourceSets, Workspace};
use grounds_index::lombok::LombokConfig;
use grounds_index::stubs::extract_with;
use grounds_index::{Classpath, FileStub, Index};
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
    pub lombok: LombokConfigs,
    /// classes listed in META-INF/spring.factories and META-INF/spring/*.imports
    pub registered: Vec<String>,
    /// the auto-configurations among them
    pub auto_registered: Vec<String>,
}

/// Class names registered through Spring's META-INF files in any module, and
/// the auto-configurations among them (`EnableAutoConfiguration` in
/// spring.factories, `AutoConfiguration.imports`).
fn spring_registrations(ws: &Workspace) -> (Vec<String>, Vec<String>) {
    let (mut out, mut auto) = (Vec::new(), Vec::new());
    for m in &ws.modules {
        let meta = m.dir.join("src/main/resources/META-INF");
        if let Ok(t) = std::fs::read_to_string(meta.join("spring.factories")) {
            // key=a.B,\ c.D  (values may continue over lines)
            for (k, v) in t.replace("\r\n", "\n").replace("\\\n", "").lines().filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.to_string()))) {
                let classes: Vec<String> = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                if k == "org.springframework.boot.autoconfigure.EnableAutoConfiguration" {
                    auto.extend(classes.iter().cloned());
                }
                out.extend(classes);
            }
        }
        if let Ok(rd) = std::fs::read_dir(meta.join("spring")) {
            for e in rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "imports")) {
                if let Ok(t) = std::fs::read_to_string(e.path()) {
                    let classes: Vec<String> = t.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect();
                    if e.file_name() == "org.springframework.boot.autoconfigure.AutoConfiguration.imports" {
                        auto.extend(classes.iter().cloned());
                    }
                    out.extend(classes);
                }
            }
        }
    }
    (out, auto)
}

/// Parsed `lombok.config` files by directory.
#[derive(Debug, Default, Clone)]
pub struct LombokConfigs {
    dirs: Vec<(PathBuf, Vec<(String, String, String)>)>,
}

impl LombokConfigs {
    pub fn discover(root: &Path) -> LombokConfigs {
        let mut dirs = Vec::new();
        let walker = ignore::WalkBuilder::new(root)
            .hidden(false)
            .filter_entry(|e| !matches!(e.file_name().to_str(), Some(".git" | "build" | "target" | "node_modules" | ".gradle" | "out" | "src")))
            .build();
        for e in walker.flatten() {
            if e.file_name() == "lombok.config" {
                if let (Some(dir), Ok(text)) = (e.path().parent(), std::fs::read_to_string(e.path())) {
                    let mut entries = Vec::new();
                    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                        if let Some(key) = line.strip_prefix("clear ") {
                            entries.push((key.trim().to_ascii_lowercase(), "clear".to_string(), String::new()));
                        } else if let Some((k, v)) = line.split_once("+=") {
                            entries.push((k.trim().to_ascii_lowercase(), "+=".to_string(), v.trim().to_string()));
                        } else if let Some((k, v)) = line.split_once("-=") {
                            entries.push((k.trim().to_ascii_lowercase(), "-=".to_string(), v.trim().to_string()));
                        } else if let Some((k, v)) = line.split_once('=') {
                            entries.push((k.trim().to_ascii_lowercase(), "=".to_string(), v.trim().to_string()));
                        }
                    }
                    dirs.push((dir.to_path_buf(), entries));
                }
            }
        }
        LombokConfigs { dirs }
    }

    /// Settings for a source file: configs from the outermost directory to
    /// the nearest one, stopping at `config.stopBubbling = true`.
    pub fn for_file(&self, file: &Path) -> LombokConfig {
        let mut chain: Vec<&Vec<(String, String, String)>> = Vec::new();
        let mut dir = file.parent();
        while let Some(d) = dir {
            if let Some((_, entries)) = self.dirs.iter().find(|(p, _)| p == d) {
                chain.push(entries);
                if entries.iter().any(|(k, op, v)| k == "config.stopbubbling" && op == "=" && v.eq_ignore_ascii_case("true")) {
                    break;
                }
            }
            dir = d.parent();
        }
        let mut cfg = LombokConfig::default();
        for entries in chain.into_iter().rev() {
            for (k, op, v) in entries {
                let truthy = v.eq_ignore_ascii_case("true");
                match (k.as_str(), op.as_str()) {
                    ("lombok.accessors.fluent", "=") => cfg.fluent = truthy,
                    ("lombok.accessors.fluent", "clear") => cfg.fluent = false,
                    ("lombok.accessors.chain", "=") => cfg.chain = truthy,
                    ("lombok.accessors.chain", "clear") => cfg.chain = false,
                    ("lombok.accessors.prefix", "+=") => cfg.prefixes.push(v.clone()),
                    ("lombok.accessors.prefix", "-=") => cfg.prefixes.retain(|p| p != v),
                    ("lombok.accessors.prefix", "clear") => cfg.prefixes.clear(),
                    ("lombok.copyableannotations", "+=") => cfg.copyable.push(v.clone()),
                    ("lombok.copyableannotations", "-=") => cfg.copyable.retain(|p| p != v),
                    ("lombok.copyableannotations", "clear") => cfg.copyable.clear(),
                    _ => {}
                }
            }
        }
        cfg
    }

    /// Mixed into stub cache keys so a config change re-extracts.
    pub fn key_for(&self, file: &Path) -> u64 {
        let cfg = self.for_file(file);
        if cfg == LombokConfig::default() {
            0
        } else {
            xxhash_rust::xxh3::xxh3_64(format!("{cfg:?}").as_bytes())
        }
    }
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
        if !classpath.contains("java.lang.Object") {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                eprintln!("grounds: warning: no JDK found (set JAVA_HOME, put `java` on PATH, or `java_home` in grounds.toml); checks that need the JDK's classes stay silent");
            });
        }
        stats.classpath_classes = classpath.len();
        stats.t_classpath = t.elapsed();

        let t = Instant::now();
        let lombok = LombokConfigs::discover(&workspace.root);
        let cache = StubCache::load(&workspace.root);
        let results: Vec<(PathBuf, u64, Arc<FileStub>, bool)> = workspace
            .files
            .par_iter()
            .filter_map(|f| {
                let text = std::fs::read_to_string(&f.path).ok()?;
                let h = content_hash(&text) ^ lombok.key_for(&f.path);
                let key = f.path.to_string_lossy();
                if let Some(s) = cache.get(&key, h) {
                    return Some((f.path.clone(), h, s, true));
                }
                let pf = ParsedFile::parse(text);
                Some((f.path.clone(), h, Arc::new(extract_with(&pf, &lombok.for_file(&f.path))), false))
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
        let (registered, auto_registered) = spring_registrations(&workspace);
        let mut di = grounds_di::DiModel::build_with(&index, &registered, &auto_registered);
        di.lombok_copies_qualifiers = lombok_copies_qualifiers(&workspace.root);
        di.bean_overriding_allowed = bean_overriding_allowed(&workspace);
        Ok(Project { config, workspace, index, stats, di, source_sets, lombok, registered, auto_registered })
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
                let stub = Arc::new(self.extract(path, &pf));
                // always install: referenced names and positions change with
                // body-only edits; the DI model stores positions too
                let changed = old != Some(stub.signature_hash);
                self.index.set_file(key, stub);
                self.rebuild_di();
                changed
            }
            Err(_) => {
                self.index.remove_file(&key);
                self.rebuild_di();
                old.is_some()
            }
        }
    }

    /// Declaration stub of a file, with the lombok.config settings for it.
    pub fn extract(&self, path: &Path, pf: &ParsedFile) -> FileStub {
        extract_with(pf, &self.lombok.for_file(path))
    }

    pub fn rebuild_di(&mut self) {
        let (copies, overriding) = (self.di.lombok_copies_qualifiers, self.di.bean_overriding_allowed);
        self.di = grounds_di::DiModel::build_with(&self.index, &self.registered, &self.auto_registered);
        self.di.lombok_copies_qualifiers = copies;
        self.di.bean_overriding_allowed = overriding;
    }
}

/// Does a lombok.config in the workspace make Lombok copy @Qualifier/@Value?
/// Whether any module's application config enables bean definition
/// overriding (any profile counts: unknown means silent).
fn bean_overriding_allowed(ws: &Workspace) -> bool {
    ws.modules.iter().any(|m| {
        let dir = m.dir.join("src/main/resources");
        let Ok(rd) = std::fs::read_dir(&dir) else { return false };
        rd.flatten().any(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let config = name.starts_with("application") && (name.ends_with(".properties") || name.ends_with(".yml") || name.ends_with(".yaml"));
            config
                && std::fs::read_to_string(e.path()).is_ok_and(|t| {
                    t.lines().any(|l| {
                        let l = l.trim().to_ascii_lowercase();
                        // `spring.main.allow-bean-definition-overriding=true` or
                        // `allow-bean-definition-overriding: true` (YAML, nested)
                        l.contains("allow-bean-definition-overriding") && l.split(['=', ':']).nth(1).is_some_and(|v| v.trim() == "true")
                    })
                })
        })
    })
}

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
