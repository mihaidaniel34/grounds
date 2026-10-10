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
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn collect(ws: &Workspace, cfg: &Config) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for e in &cfg.classpath.entries {
        out.push(if e.is_absolute() { e.clone() } else { ws.root.join(e) });
    }
    let mut files = cfg.classpath.files.clone();
    let default_dump = ws.root.join(".grounds/classpath.txt");
    if default_dump.exists() {
        files.push(default_dump);
    }
    for f in files {
        let f = if f.is_absolute() { f } else { ws.root.join(f) };
        if let Ok(text) = std::fs::read_to_string(&f) {
            for line in text.lines() {
                for part in line.split(':') {
                    let part = part.trim();
                    if !part.is_empty() && !part.starts_with('#') {
                        out.push(PathBuf::from(part));
                    }
                }
            }
        }
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
    #[test]
    fn versions() {
        assert!(newer("1.10.0", "1.9.3"));
        assert!(newer("2.0.0", "2.0.0-RC1"));
        assert!(!newer("3.0.0-M1", "2.7.5"));
        assert!(newer("5.10.2", "5.9.0"));
    }
}
