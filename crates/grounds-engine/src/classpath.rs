//! Classpath providers. None of them run a build tool on the hot path.
//!
//! - explicit entries / classpath files from config (e.g. a one-off Gradle
//!   dump produced by `grounds classpath --gradle`)
//! - module build outputs (`build/classes/**`, `target/classes`)
//! - a heuristic scan of local Gradle/Maven caches (newest version of each
//!   artifact)
//!
//! Imprecision is acceptable by design: analysis never reports a problem
//! based on a type it cannot see.

use crate::config::Config;
use crate::workspace::Workspace;
use grounds_index::Index;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Classpath files in use: those listed in grounds.toml, then the default
/// dump if it exists and is not listed.
fn classpath_files(ws: &Workspace, cfg: &Config) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = cfg.classpath.files.iter().map(|f| if f.is_absolute() { f.clone() } else { ws.root.join(f) }).collect();
    let default_dump = ws.root.join(".grounds/classpath.txt");
    if default_dump.exists() && !files.contains(&default_dump) {
        files.push(default_dump);
    }
    files
}

/// Entries of a classpath file: one per line or `:`-separated.
fn read_list(f: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(f) else { return Vec::new() };
    text.lines().flat_map(|l| l.split(':')).map(str::trim).filter(|p| !p.is_empty() && !p.starts_with('#')).map(PathBuf::from).collect()
}

pub fn collect(ws: &Workspace, cfg: &Config) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for e in &cfg.classpath.entries {
        out.push(if e.is_absolute() { e.clone() } else { ws.root.join(e) });
    }
    for f in classpath_files(ws, cfg) {
        out.extend(read_list(&f));
    }
    if cfg.classpath.build_outputs {
        for m in &ws.modules {
            for rel in ["build/classes/java/main", "build/classes/kotlin/main", "target/classes", "build/classes/groovy/main"] {
                let d = m.dir.join(rel);
                if d.is_dir() {
                    out.push(d);
                }
            }
        }
    }
    if cfg.classpath.scan_caches {
        out.extend(scan_caches());
    }
    let mut seen = rustc_hash::FxHashSet::default();
    out.retain(|p| p.exists() && seen.insert(p.clone()));
    out
}

/// A classpath problem that makes calls into libraries stay unresolved, so
/// most rules go silent on them without anyone noticing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// A classpath file listed in grounds.toml does not exist.
    MissingFile(String),
    /// A classpath file is older than a build file it was generated from.
    Stale { file: String, build_file: String },
    /// Entries of a classpath file outside the project no longer exist
    /// (the cache was cleaned).
    MissingEntries { file: String, missing: usize, total: usize },
    /// No classpath file, and no jar has most of the library types the
    /// sources import.
    FewJars { missing: usize, imported: usize, packages: Vec<String> },
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Warning::MissingFile(file) => write!(f, "grounds: warning: classpath file {file} (grounds.toml) does not exist; run `grounds classpath` to create it"),
            Warning::Stale { file, build_file } => write!(f, "grounds: warning: {file} is older than {build_file}; re-run `grounds classpath` if dependencies changed"),
            Warning::MissingEntries { file, missing, total } => write!(f, "grounds: warning: {missing} of {total} jars in {file} no longer exist; re-run `grounds classpath`"),
            Warning::FewJars { missing, imported, packages } => write!(
                f,
                "grounds: warning: no jar found for {missing} of {imported} imported library types ({}); calls into them stay unresolved and most checks skip them. Build the project once or run `grounds classpath` (silence: `[classpath] warn = false`)",
                packages.join(", ")
            ),
        }
    }
}

/// Build files a classpath dump is generated from.
const BUILD_FILES: &[&str] = &["pom.xml", "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "gradle.properties", "gradle/libs.versions.toml"];

/// A classpath file this much older than a build file is stale; checkouts
/// write files in some order, so a few seconds apart means nothing.
const STALE_AFTER: Duration = Duration::from_secs(2);

/// Classpath problems worth telling the user about, cheaply: file existence,
/// mtimes, and the sources' imports against the loaded index.
pub fn warnings(ws: &Workspace, cfg: &Config, index: &Index) -> Vec<Warning> {
    if !cfg.classpath.warn {
        return Vec::new();
    }
    let rel = |p: &Path| p.strip_prefix(&ws.root).unwrap_or(p).to_string_lossy().to_string();
    let mut out = Vec::new();
    for f in &cfg.classpath.files {
        let f = if f.is_absolute() { f.clone() } else { ws.root.join(f) };
        if !f.exists() {
            out.push(Warning::MissingFile(rel(&f)));
        }
    }
    let files = classpath_files(ws, cfg);
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let mut dirs: Vec<&Path> = vec![&ws.root];
    for m in &ws.modules {
        if !dirs.contains(&m.dir.as_path()) {
            dirs.push(&m.dir);
        }
    }
    let newest_build_file = dirs.iter().flat_map(|d| BUILD_FILES.iter().map(move |b| d.join(b))).filter_map(|p| Some((mtime(&p)?, p))).max();
    for f in files.iter().filter(|f| f.exists()) {
        if let (Some(t), Some((bt, b))) = (mtime(f), &newest_build_file) {
            if bt.duration_since(t).is_ok_and(|d| d > STALE_AFTER) {
                out.push(Warning::Stale { file: rel(f), build_file: rel(b) });
            }
        }
        // entries inside the project (sibling modules' outputs) come and go
        // with builds; those outside are jars in a cache
        let outside: Vec<PathBuf> = read_list(f).into_iter().filter(|e| !e.starts_with(&ws.root)).collect();
        let missing = outside.iter().filter(|e| !e.exists()).count();
        if missing > 0 {
            out.push(Warning::MissingEntries { file: rel(f), missing, total: outside.len() });
        }
    }
    let build_tool = BUILD_FILES[..5].iter().any(|b| ws.root.join(b).exists());
    if files.is_empty() && build_tool {
        let (missing, imported, packages) = missing_imports(index);
        // conservative: a few unknown types are generated code or optional
        // dependencies; only most of them missing means missing jars
        if missing >= 10 && missing * 3 >= imported {
            out.push(Warning::FewJars { missing, imported, packages });
        }
    }
    out
}

/// Library types imported by the sources (`java.*` and project types
/// aside) that neither the sources nor the classpath define: (missing,
/// imported, the packages most of the missing ones are in).
fn missing_imports(index: &Index) -> (usize, usize, Vec<String>) {
    let source_packages: FxHashSet<&str> = index.files().map(|(_, s)| &*s.package).collect();
    let mut imported: FxHashSet<&str> = FxHashSet::default();
    for (_, stub) in index.files() {
        for i in stub.imports.iter().filter(|i| !i.from_module) {
            let path: &str = &i.path;
            // `import static a.B.m;` names a member, `import a.b.*;` a package
            let class = match (i.is_static, i.on_demand) {
                (false, true) => continue,
                (true, false) => match path.rfind('.') {
                    Some(j) => &path[..j],
                    None => continue,
                },
                _ => path,
            };
            imported.insert(class);
        }
    }
    let mut total = 0;
    let mut by_package: FxHashMap<String, usize> = FxHashMap::default();
    for class in imported {
        if class.starts_with("java.") {
            continue;
        }
        // the package: segments before the first capitalized one
        let segs: Vec<&str> = class.split('.').collect();
        let n = segs.iter().take_while(|s| s.starts_with(|c: char| c.is_lowercase())).count();
        if n == 0 || n == segs.len() || source_packages.contains(segs[..n].join(".").as_str()) {
            continue;
        }
        total += 1;
        // `a.B.C` is found when `a.B` is
        let found = (n + 1..=segs.len()).any(|k| index.class_exists(&segs[..k].join(".")));
        if !found {
            *by_package.entry(segs[..n.min(2)].join(".")).or_default() += 1;
        }
    }
    let missing = by_package.values().sum();
    let mut packages: Vec<(String, usize)> = by_package.into_iter().collect();
    packages.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    (missing, total, packages.into_iter().take(3).map(|(p, _)| p).collect())
}

/// Parse a version string into comparable components.
fn version_key(v: &str) -> Vec<(u64, String)> {
    v.split(['.', '-', '_'])
        .map(|part| {
            let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            let n = digits.parse::<u64>().unwrap_or(0);
            // pre-release qualifiers sort before releases
            let rest = part[digits.len()..].to_ascii_lowercase();
            (n, rest)
        })
        .collect()
}

fn is_prerelease(v: &str) -> bool {
    let l = v.to_ascii_lowercase();
    ["alpha", "beta", "-rc", ".rc", "snapshot", "-m", "milestone", "preview", "-ea"].iter().any(|q| l.contains(q))
}

fn newer(a: &str, b: &str) -> bool {
    // prefer releases over pre-releases
    match (is_prerelease(a), is_prerelease(b)) {
        (false, true) => return true,
        (true, false) => return false,
        _ => {}
    }
    version_key(a) > version_key(b)
}

/// Newest jar per `group:artifact` from `~/.gradle/caches/modules-2` and `~/.m2`.
pub fn scan_caches() -> Vec<PathBuf> {
    let Some(home) = home_dir() else { return Vec::new() };
    let mut best: FxHashMap<String, (String, PathBuf)> = FxHashMap::default();
    let gradle = home.join(".gradle/caches/modules-2/files-2.1");
    if let Ok(groups) = std::fs::read_dir(&gradle) {
        for g in groups.flatten() {
            let group = g.file_name().to_string_lossy().to_string();
            let Ok(arts) = std::fs::read_dir(g.path()) else { continue };
            for a in arts.flatten() {
                let artifact = a.file_name().to_string_lossy().to_string();
                let Ok(versions) = std::fs::read_dir(a.path()) else { continue };
                for v in versions.flatten() {
                    let version = v.file_name().to_string_lossy().to_string();
                    let Ok(hashes) = std::fs::read_dir(v.path()) else { continue };
                    for h in hashes.flatten() {
                        let Ok(fs) = std::fs::read_dir(h.path()) else { continue };
                        for f in fs.flatten() {
                            let n = f.file_name().to_string_lossy().to_string();
                            if n.ends_with(".jar") && !n.ends_with("-sources.jar") && !n.ends_with("-javadoc.jar") && !n.ends_with("-tests.jar") {
                                consider(&mut best, format!("{group}:{artifact}"), &version, f.path());
                            }
                        }
                    }
                }
            }
        }
    }
    let m2 = home.join(".m2/repository");
    if m2.is_dir() {
        scan_m2(&m2, &m2, &mut best, 0);
    }
    let mut out: Vec<PathBuf> = best.into_values().map(|(_, p)| p).collect();
    out.sort();
    out
}

fn consider(best: &mut FxHashMap<String, (String, PathBuf)>, key: String, version: &str, path: PathBuf) {
    match best.get(&key) {
        Some((v, _)) if !newer(version, v) => {}
        _ => {
            best.insert(key, (version.to_string(), path));
        }
    }
}

fn scan_m2(root: &Path, dir: &Path, best: &mut FxHashMap<String, (String, PathBuf)>, depth: usize) {
    if depth > 12 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut subdirs = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            subdirs.push(p);
        } else if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
            if n.ends_with(".jar") && !n.ends_with("-sources.jar") && !n.ends_with("-javadoc.jar") && !n.ends_with("-tests.jar") {
                // layout: group/path/artifact/version/artifact-version.jar
                let Some(vdir) = p.parent() else { continue };
                let Some(adir) = vdir.parent() else { continue };
                let Some(gdir) = adir.parent() else { continue };
                let version = vdir.file_name().unwrap().to_string_lossy().to_string();
                let artifact = adir.file_name().unwrap().to_string_lossy().to_string();
                if !n.starts_with(&format!("{artifact}-{version}")) {
                    continue;
                }
                let group = gdir.strip_prefix(root).map(|g| g.to_string_lossy().replace('/', ".")).unwrap_or_default();
                consider(best, format!("{group}:{artifact}"), &version, p.clone());
            }
        }
    }
    for s in subdirs {
        scan_m2(root, &s, best, depth + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grounds_index::Classpath;
    use std::sync::Arc;
    use std::time::SystemTime;

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("grounds-classpath-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn set_mtime(p: &Path, t: SystemTime) {
        std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
    }

    /// Warnings for a project, with `lib` (class files by name) as the only
    /// classpath entry besides the sources.
    fn warnings_of(root: &Path, cfg: &Config, lib: &[&str]) -> Vec<Warning> {
        let ws = Workspace::discover(root, &[]);
        let dir = root.join("lib-classes");
        for c in lib {
            write(&dir, &format!("{}.class", c.replace('.', "/")), "");
        }
        let mut index = Index::new(Arc::new(Classpath::from_containers(vec![grounds_classfile::ClassContainer::open_dir(&dir)])));
        for f in &ws.files {
            let pf = grounds_syntax::ParsedFile::parse(std::fs::read_to_string(&f.path).unwrap());
            index.set_file(crate::project::path_key(&f.path), Arc::new(grounds_index::stubs::extract(&pf)));
        }
        warnings(&ws, cfg, &index)
    }

    fn with_files(files: &[&str]) -> Config {
        let mut cfg = Config::default();
        cfg.classpath.files = files.iter().map(PathBuf::from).collect();
        cfg
    }

    #[test]
    fn listed_file_missing() {
        let dir = temp("missing");
        write(&dir, "pom.xml", "<project/>");
        let w = warnings_of(&dir, &with_files(&[".grounds/classpath.txt"]), &[]);
        assert_eq!(w, vec![Warning::MissingFile(".grounds/classpath.txt".into())]);
        assert!(w[0].to_string().contains("run `grounds classpath`"));
        let mut quiet = with_files(&[".grounds/classpath.txt"]);
        quiet.classpath.warn = false;
        assert!(warnings_of(&dir, &quiet, &[]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_older_than_build_files() {
        let dir = temp("stale");
        write(&dir, "settings.gradle", "include 'app'\n");
        write(&dir, "app/build.gradle", "plugins { id 'java' }\n");
        write(&dir, "app/src/main/java/a/A.java", "package a;\nclass A {}\n");
        write(&dir, ".grounds/classpath.txt", "");
        let now = SystemTime::now();
        set_mtime(&dir.join("settings.gradle"), now - Duration::from_secs(3600));
        set_mtime(&dir.join("app/build.gradle"), now - Duration::from_secs(3600));
        set_mtime(&dir.join(".grounds/classpath.txt"), now - Duration::from_secs(60));
        // the default dump is used without being listed
        assert!(warnings_of(&dir, &Config::default(), &[]).is_empty());
        // a module's build file changed after the dump
        set_mtime(&dir.join("app/build.gradle"), now);
        assert_eq!(warnings_of(&dir, &Config::default(), &[]), vec![Warning::Stale { file: ".grounds/classpath.txt".into(), build_file: "app/build.gradle".into() }]);
        // a version catalog too
        set_mtime(&dir.join("app/build.gradle"), now - Duration::from_secs(3600));
        write(&dir, "gradle/libs.versions.toml", "[versions]\n");
        assert_eq!(warnings_of(&dir, &Config::default(), &[]), vec![Warning::Stale { file: ".grounds/classpath.txt".into(), build_file: "gradle/libs.versions.toml".into() }]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn listed_jars_missing() {
        let dir = temp("entries");
        let cache = temp("entries-cache");
        write(&cache, "a.jar", "");
        write(&dir, "pom.xml", "<project/>");
        // a cleaned cache counts; a sibling module's output in the project does not
        let listed = [cache.join("a.jar"), cache.join("gone-1.jar"), cache.join("gone-2.jar"), dir.join("core/target/classes")];
        write(&dir, "cp.txt", &listed.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
        set_mtime(&dir.join("pom.xml"), SystemTime::now() - Duration::from_secs(3600));
        let w = warnings_of(&dir, &with_files(&["cp.txt"]), &[]);
        assert_eq!(w, vec![Warning::MissingEntries { file: "cp.txt".into(), missing: 2, total: 3 }]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn imported_library_types_without_jars() {
        let dir = temp("imports");
        write(&dir, "pom.xml", "<project/>");
        let mut imports: Vec<String> = (0..8).map(|i| format!("import org.springframework.web.T{i};")).collect();
        imports.extend((0..4).map(|i| format!("import com.fasterxml.jackson.databind.J{i};")));
        // the JDK, the project's own types and on-demand imports are not counted
        imports.extend(["import java.util.List;", "import app.other.B;", "import app.gen.Generated;", "import org.slf4j.*;"].map(String::from));
        // found: nested and static imports resolve through their class
        imports.extend(["import org.lib.Found.Inner;", "import static org.lib.Util.call;"].map(String::from));
        write(&dir, "src/main/java/app/A.java", &format!("package app;\n{}\nclass A {{}}\n", imports.join("\n")));
        write(&dir, "src/main/java/app/other/B.java", "package app.other;\nclass B {}\n");
        write(&dir, "src/main/java/app/gen/X.java", "package app.gen;\nclass X {}\n");
        let lib = ["org.lib.Found", "org.lib.Found$Inner", "org.lib.Util"];
        let w = warnings_of(&dir, &Config::default(), &lib);
        assert_eq!(w, vec![Warning::FewJars { missing: 12, imported: 14, packages: vec!["org.springframework".into(), "com.fasterxml".into()] }]);
        assert!(w[0].to_string().contains("[classpath] warn = false"));

        // not a Maven or Gradle project: nothing to dump a classpath from
        std::fs::remove_file(dir.join("pom.xml")).unwrap();
        assert!(warnings_of(&dir, &Config::default(), &lib).is_empty());
        write(&dir, "pom.xml", "<project/>");

        // most imported types found: a few unknown ones are generated code
        let many: Vec<String> = (0..30).map(|i| format!("org.lib.L{i}")).collect();
        let found: Vec<String> = (0..30).map(|i| format!("import org.lib.L{i};")).collect();
        write(&dir, "src/main/java/app/C.java", &format!("package app;\n{}\nclass C {{}}\n", found.join("\n")));
        let lib: Vec<&str> = lib.iter().copied().chain(many.iter().map(String::as_str)).collect();
        assert!(warnings_of(&dir, &Config::default(), &lib).is_empty());
        std::fs::remove_file(dir.join("src/main/java/app/C.java")).unwrap();

        // a classpath file is in use: its own warnings cover it
        write(&dir, ".grounds/classpath.txt", "");
        assert!(warnings_of(&dir, &Config::default(), &lib).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn versions() {
        assert!(newer("1.10.0", "1.9.3"));
        assert!(newer("2.0.0", "2.0.0-RC1"));
        assert!(!newer("3.0.0-M1", "2.7.5"));
        assert!(newer("5.10.2", "5.9.0"));
    }
}
