//! Minimal git integration: which files changed, which lines changed, and the
//! committed version of a file. Uses the `git` CLI (fast, no libgit2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn is_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"]).is_some_and(|s| s.trim() == "true")
}

pub fn toplevel(root: &Path) -> Option<PathBuf> {
    git(root, &["rev-parse", "--show-toplevel"]).map(|s| PathBuf::from(s.trim()))
}

/// Files modified, added or untracked relative to HEAD (absolute paths).
pub fn changed_files(root: &Path) -> Vec<PathBuf> {
    let Some(top) = toplevel(root) else { return Vec::new() };
    let Some(out) = git(root, &["status", "--porcelain", "-z", "--untracked-files=all"]) else { return Vec::new() };
    let mut files = Vec::new();
    let mut parts = out.split('\0');
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let status = &entry[..2];
        let path = &entry[3..];
        if status.starts_with('R') || status.starts_with('C') {
            // rename: next field is the original path
            parts.next();
        }
        if status.contains('D') {
            continue;
        }
        files.push(top.join(path));
    }
    files.sort();
    files.dedup();
    files
}

/// Changed line ranges (1-based, inclusive) per file versus HEAD. Untracked
/// or new files map to `None` meaning "all lines".
pub fn changed_lines(root: &Path, files: &[PathBuf]) -> BTreeMap<PathBuf, Option<Vec<(u32, u32)>>> {
    let mut out = BTreeMap::new();
    let tracked: std::collections::HashSet<PathBuf> = {
        let top = toplevel(root).unwrap_or_else(|| root.to_path_buf());
        git(root, &["ls-files", "-z"]).map(|s| s.split('\0').filter(|p| !p.is_empty()).map(|p| top.join(p)).collect()).unwrap_or_default()
    };
    for f in files {
        if !tracked.contains(f) {
            out.insert(f.clone(), None);
            continue;
        }
        let Some(diff) = git(root, &["diff", "-U0", "--no-color", "HEAD", "--", &f.to_string_lossy()]) else {
            out.insert(f.clone(), None);
            continue;
        };
        let mut ranges = Vec::new();
        for line in diff.lines() {
            // @@ -a,b +c,d @@
            if let Some(rest) = line.strip_prefix("@@ ") {
                if let Some(plus) = rest.split_whitespace().find(|p| p.starts_with('+')) {
                    let spec = &plus[1..];
                    let (start, count) = match spec.split_once(',') {
                        Some((s, c)) => (s.parse::<u32>().unwrap_or(0), c.parse::<u32>().unwrap_or(0)),
                        None => (spec.parse::<u32>().unwrap_or(0), 1),
                    };
                    if count > 0 {
                        ranges.push((start, start + count - 1));
                    }
                }
            }
        }
        out.insert(f.clone(), Some(ranges));
    }
    out
}

/// Contents of the file at HEAD, if it exists there.
pub fn head_version(root: &Path, file: &Path) -> Option<String> {
    let top = toplevel(root)?;
    let rel = file.strip_prefix(&top).ok()?;
    git(root, &["show", &format!("HEAD:{}", rel.to_string_lossy())])
}

pub fn in_ranges(line: u32, ranges: &Option<Vec<(u32, u32)>>) -> bool {
    match ranges {
        None => true,
        Some(r) => r.iter().any(|(a, b)| line >= *a && line <= *b),
    }
}
