//! Which source set a file belongs to, and whether it holds tests: one
//! classification for the workspace, the rules and the DI model.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Source set names known to hold tests besides the naming heuristics: from
/// `test_source_sets` in grounds.toml and from the source set map written by
/// `grounds classpath` (source sets some `Test` task runs).
static EXTRA_TEST_SETS: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Roots of the loaded projects: module directories are judged by the path
/// below the root only, so a checkout under `~/tests/` stays main code.
static PROJECT_ROOTS: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

pub fn set_extra_test_sets(names: Vec<String>) {
    if let Ok(mut w) = EXTRA_TEST_SETS.write() {
        *w = names.into_iter().map(|n| n.to_ascii_lowercase()).collect();
    }
}

pub fn register_project_root(root: &Path) {
    if let Ok(mut w) = PROJECT_ROOTS.write() {
        if !w.iter().any(|r| r == root) {
            w.push(root.to_path_buf());
        }
    }
}

/// The source set a file belongs to (`main`, `test`, `integrationTest`, ...),
/// i.e. the directory after the last `src`.
pub fn source_set_of(path: &Path) -> Option<&str> {
    let comps: Vec<&str> = path.components().filter_map(|c| c.as_os_str().to_str()).collect();
    comps.windows(2).rev().find(|w| w[0] == "src").map(|w| w[1])
}

pub fn is_test_set(set: &str) -> bool {
    let set = set.to_ascii_lowercase();
    const HINTS: &[&str] = &["test", "integration", "jmh", "e2e", "functional", "acceptance", "contract", "benchmark"];
    set == "it" || HINTS.iter().any(|h| set.contains(h)) || EXTRA_TEST_SETS.read().is_ok_and(|e| e.contains(&set))
}

/// A file in a test source set.
pub fn is_test_path(path: &str) -> bool {
    source_set_of(Path::new(path)).is_some_and(is_test_set)
}

/// A file of a module that exists only for testing: a directory between the
/// project root and the module's `src` is named exactly `tests`, `test`, `it`,
/// `integration-tests` or `testing` (`<root>/webserver/tests/grpc/src/main/java`).
/// Packages (below `src`) and the directories above the root don't count; an
/// absolute path outside every loaded project is not judged.
pub fn in_test_module(path: &Path) -> bool {
    const DIRS: &[&str] = &["tests", "test", "it", "integration-tests", "testing"];
    let rel = if path.is_absolute() {
        let roots = PROJECT_ROOTS.read();
        let Some(root) = roots.as_ref().ok().and_then(|rs| rs.iter().filter(|r| path.starts_with(r)).max_by_key(|r| r.as_os_str().len()).cloned()) else {
            return false;
        };
        path.strip_prefix(root).unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    };
    let comps: Vec<&str> = rel.components().filter_map(|c| c.as_os_str().to_str()).collect();
    let Some(src) = comps.iter().rposition(|c| *c == "src") else { return false };
    comps[..src].iter().any(|c| DIRS.contains(c))
}

/// A file holding test code: in a test source set or a test-only module.
pub fn is_test_code(path: &str) -> bool {
    is_test_path(path) || in_test_module(Path::new(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_only_modules() {
        register_project_root(Path::new("/work/test/proj"));
        let test = |p: &str| is_test_code(&format!("/work/test/proj/{p}"));
        assert!(test("webserver/tests/grpc/src/main/java/a/b/C.java"));
        assert!(test("service/tests/toolbox/src/main/java/C.java"));
        assert!(test("tests/src/main/java/C.java"));
        assert!(test("integration-tests/app/src/main/java/C.java"));
        assert!(test("testing/junit5/src/main/java/C.java"));
        assert!(test("core/it/src/main/java/C.java"));
        assert!(test("core/src/test/java/C.java"));
        // production modules: the package or a module name merely containing `test`
        assert!(!test("core/src/main/java/com/x/test/C.java"));
        assert!(!test("core/src/main/java/com/x/tests/C.java"));
        assert!(!test("testing-utils/src/main/java/C.java"));
        assert!(!test("test-support/src/main/java/C.java"));
        assert!(!test("contest/src/main/java/C.java"));
        // the directories above the root don't count, nor do other checkouts
        assert!(!test("src/main/java/C.java"));
        assert!(!is_test_code("/elsewhere/tests/x/src/main/java/C.java"));
        assert!(is_test_code("tests/x/src/main/java/C.java"));
        assert!(!is_test_path("/work/test/proj/tests/x/src/main/java/C.java"));
    }
}
