//! Which source set a file belongs to, and whether it holds tests: one
//! classification for the workspace, the rules and the DI model.

use std::path::Path;
use std::sync::RwLock;

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

pub fn is_test_set(set: &str) -> bool {
    let set = set.to_ascii_lowercase();
    const HINTS: &[&str] = &["test", "integration", "jmh", "e2e", "functional", "acceptance", "contract", "benchmark"];
    set == "it" || HINTS.iter().any(|h| set.contains(h)) || EXTRA_TEST_SETS.read().is_ok_and(|e| e.contains(&set))
}

/// A file in a test source set.
pub fn is_test_path(path: &str) -> bool {
    source_set_of(Path::new(path)).is_some_and(is_test_set)
}
