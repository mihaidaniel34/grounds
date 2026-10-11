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

/// The commit `--base <rev>` compares against: the merge base of `rev` and
/// HEAD, so a branch's changes are what it added since it forked, not what
/// landed on `rev` meanwhile. `None` if git cannot find one (e.g. a shallow
/// clone without the base's history).
pub fn merge_base(root: &Path, rev: &str) -> Option<String> {
    git(root, &["merge-base", rev, "HEAD"]).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Files changed relative to `rev` (committed or not, plus untracked files),
/// and files deleted since `rev`; absolute paths. A rename counts as a
/// deletion plus an addition.
pub fn changed_files_since(root: &Path, rev: &str) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let Some(top) = toplevel(root) else { return (Vec::new(), Vec::new()) };
    let (mut changed, mut deleted) = (Vec::new(), Vec::new());
    if let Some(out) = git(root, &["diff", "--name-status", "--no-renames", "-z", rev]) {
        let mut parts = out.split('\0').filter(|p| !p.is_empty());
        while let (Some(status), Some(path)) = (parts.next(), parts.next()) {
            if status.starts_with('D') {
                deleted.push(top.join(path));
            } else {
                changed.push(top.join(path));
            }
        }
    }
    if let Some(out) = git(root, &["ls-files", "--others", "--exclude-standard", "--full-name", "-z"]) {
        changed.extend(out.split('\0').filter(|p| !p.is_empty()).map(|p| top.join(p)));
    }
    for v in [&mut changed, &mut deleted] {
        v.sort();
        v.dedup();
    }
    (changed, deleted)
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
    changed_lines_since(root, files, "HEAD")
}

/// `changed_lines` versus `rev`.
pub fn changed_lines_since(root: &Path, files: &[PathBuf], rev: &str) -> BTreeMap<PathBuf, Option<Vec<(u32, u32)>>> {
    hunks_since(root, files, rev).into_iter().map(|(f, h)| (f, h.map(|h| h.iter().filter(|x| x.new_count > 0).map(|x| (x.new_start, x.new_start + x.new_count - 1)).collect()))).collect()
}

/// One `@@ -old_start,old_count +new_start,new_count @@` hunk of a diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
}

impl Hunk {
    /// The current line standing for line `old` of the old version, when a
    /// hunk changed or removed it (a removal maps to the line after it).
    pub fn new_line_for(hunks: &[Hunk], old: u32) -> Option<u32> {
        hunks.iter().find(|h| h.old_count > 0 && old >= h.old_start && old < h.old_start + h.old_count).map(|h| h.new_start.max(1))
    }
}

/// Diff hunks per file versus `rev`; `None` for untracked files and files
/// git cannot diff (all lines new).
pub fn hunks_since(root: &Path, files: &[PathBuf], rev: &str) -> BTreeMap<PathBuf, Option<Vec<Hunk>>> {
    let mut out = BTreeMap::new();
    let tracked: std::collections::HashSet<PathBuf> = {
        let top = toplevel(root).unwrap_or_else(|| root.to_path_buf());
        git(root, &["ls-files", "--full-name", "-z"]).map(|s| s.split('\0').filter(|p| !p.is_empty()).map(|p| top.join(p)).collect()).unwrap_or_default()
    };
    for f in files {
        if !tracked.contains(f) {
            out.insert(f.clone(), None);
            continue;
        }
        let Some(diff) = git(root, &["diff", "-U0", "--no-color", "--no-ext-diff", rev, "--", &f.to_string_lossy()]) else {
            out.insert(f.clone(), None);
            continue;
        };
        // `-a,b` / `+c,d`; a missing count means 1
        let spec = |s: &str| -> (u32, u32) {
            match s.split_once(',') {
                Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
                None => (s.parse().unwrap_or(0), 1),
            }
        };
        let mut hunks = Vec::new();
        for line in diff.lines() {
            // @@ -a,b +c,d @@
            if let Some(rest) = line.strip_prefix("@@ ") {
                let mut parts = rest.split_whitespace();
                if let (Some(minus), Some(plus)) = (parts.next().and_then(|p| p.strip_prefix('-')), parts.next().and_then(|p| p.strip_prefix('+'))) {
                    let ((old_start, old_count), (new_start, new_count)) = (spec(minus), spec(plus));
                    hunks.push(Hunk { old_start, old_count, new_start, new_count });
                }
            }
        }
        out.insert(f.clone(), Some(hunks));
    }
    out
}

/// Contents of the file at HEAD, if it exists there.
pub fn head_version(root: &Path, file: &Path) -> Option<String> {
    version_at(root, "HEAD", file)
}

/// Contents of the file at commit `rev`, if it exists there.
pub fn version_at(root: &Path, rev: &str, file: &Path) -> Option<String> {
    let top = toplevel(root)?;
    let rel = file.strip_prefix(&top).ok()?;
    git(root, &["show", &format!("{rev}:{}", rel.to_string_lossy())])
}

pub fn in_ranges(line: u32, ranges: &Option<Vec<(u32, u32)>>) -> bool {
    match ranges {
        None => true,
        Some(r) => r.iter().any(|(a, b)| line >= *a && line <= *b),
    }
}
