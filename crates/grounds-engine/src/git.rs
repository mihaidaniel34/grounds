//! Minimal git integration: which files changed, which lines changed, and the
//! committed version of a file. Uses the `git` CLI (fast, no libgit2).
//!
//! A git command that fails is a `GitError`, never an empty answer: an empty
//! list of changed files reads as a clean tree, no version of a file at a
//! revision as a file the change added. The answers that are "no" are values
//! (`Diff::Untracked`, `Ok(None)` for a file absent at a revision); each
//! caller decides what a failure means for it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A git command that could not run or did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError {
    /// The arguments, as given to git.
    pub args: String,
    /// What it said (its error output, or why it did not run).
    pub why: String,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`git {}` failed: {}", self.args, self.why)
    }
}

impl std::error::Error for GitError {}

fn git(root: &Path, args: &[&str]) -> Result<String, GitError> {
    let error = |why: String| GitError { args: args.join(" "), why };
    let out = Command::new("git").arg("-C").arg(root).args(args).output().map_err(|e| error(e.to_string()))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(error(if said.is_empty() { out.status.to_string() } else { said }));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether `rev` is HEAD before the first commit, which names no commit and
/// holds no files: git's answer (`rev-parse --verify` exits with 1, quietly),
/// an error when git cannot say. Any other revision that names nothing is
/// an error of the commands that use it.
fn unborn(root: &Path, rev: &str) -> Result<bool, GitError> {
    if rev != "HEAD" {
        return Ok(false);
    }
    let args = ["rev-parse", "--verify", "--quiet", "HEAD"];
    let error = |why: String| GitError { args: args.join(" "), why };
    let out = Command::new("git").arg("-C").arg(root).args(args).output().map_err(|e| error(e.to_string()))?;
    match out.status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(error(String::from_utf8_lossy(&out.stderr).trim().to_string())),
    }
}

/// Whether `root` is inside a git work tree: `false` where no repository
/// holds it (no `.git` there or above), an error when one does and git
/// cannot answer.
pub fn is_repo(root: &Path) -> Result<bool, GitError> {
    if crate::workspace::git_dir(root).is_none() {
        return Ok(false);
    }
    git(root, &["rev-parse", "--is-inside-work-tree"]).map(|s| s.trim() == "true")
}

/// The top of the work tree holding `root`; `None` outside one, or when git
/// cannot answer (the callers then take `root` itself as the top).
pub fn toplevel(root: &Path) -> Option<PathBuf> {
    git(root, &["rev-parse", "--show-toplevel"]).ok().map(|s| PathBuf::from(s.trim()))
}

fn top(root: &Path) -> Result<PathBuf, GitError> {
    git(root, &["rev-parse", "--show-toplevel"]).map(|s| PathBuf::from(s.trim()))
}

/// The commit `--base <rev>` compares against: the merge base of `rev` and
/// HEAD, so a branch's changes are what it added since it forked, not what
/// landed on `rev` meanwhile. `None` if git cannot find one (e.g. a shallow
/// clone without the base's history) or cannot run: the comparison cannot be
/// made either way, and the caller fails the command saying so.
pub fn merge_base(root: &Path, rev: &str) -> Option<String> {
    git(root, &["merge-base", rev, "HEAD"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Files changed relative to `rev` (committed or not, plus untracked files),
/// and files deleted since `rev`; absolute paths. A rename counts as a
/// deletion plus an addition.
pub fn changed_files_since(root: &Path, rev: &str) -> Result<(Vec<PathBuf>, Vec<PathBuf>), GitError> {
    let top = top(root)?;
    let (mut changed, mut deleted) = (Vec::new(), Vec::new());
    // (HEAD before the first commit holds no files: none were deleted since)
    let out = match git(root, &["diff", "--name-status", "--no-renames", "--no-relative", "-z", rev]) {
        Ok(out) => out,
        Err(_) if unborn(root, rev)? => String::new(),
        Err(e) => return Err(e),
    };
    let mut parts = out.split('\0').filter(|p| !p.is_empty());
    while let (Some(status), Some(path)) = (parts.next(), parts.next()) {
        if status.starts_with('D') {
            deleted.push(top.join(path));
        } else {
            changed.push(top.join(path));
        }
    }
    let out = git(root, &["ls-files", "--others", "--exclude-standard", "--full-name", "-z"])?;
    changed.extend(out.split('\0').filter(|p| !p.is_empty()).map(|p| top.join(p)));
    for v in [&mut changed, &mut deleted] {
        v.sort();
        v.dedup();
    }
    Ok((changed, deleted))
}

/// Files modified, added or untracked relative to HEAD (absolute paths).
pub fn changed_files(root: &Path) -> Result<Vec<PathBuf>, GitError> {
    let top = top(root)?;
    let out = git(root, &["status", "--porcelain", "-z", "--untracked-files=all"])?;
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
    Ok(files)
}

/// Changed line ranges (1-based, inclusive) per file versus HEAD. Untracked
/// or new files map to `None` meaning "all lines".
pub fn changed_lines(root: &Path, files: &[PathBuf]) -> Result<BTreeMap<PathBuf, Option<Vec<(u32, u32)>>>, GitError> {
    changed_lines_since(root, files, "HEAD")
}

/// `changed_lines` versus `rev`.
pub fn changed_lines_since(root: &Path, files: &[PathBuf], rev: &str) -> Result<BTreeMap<PathBuf, Option<Vec<(u32, u32)>>>, GitError> {
    Ok(hunks_since(root, files, rev)?
        .into_iter()
        .map(|(f, d)| {
            let ranges = match d {
                Diff::Untracked => None,
                Diff::Hunks(h) => Some(h.iter().filter(|x| x.new_count > 0).map(|x| (x.new_start, x.new_start + x.new_count - 1)).collect()),
            };
            (f, ranges)
        })
        .collect())
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

/// How a file differs from a revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Diff {
    /// git does not track it, or the revision is HEAD before the first
    /// commit: every line is new.
    Untracked,
    /// The hunks of its diff (none: the same).
    Hunks(Vec<Hunk>),
}

/// The diff of each of `files` versus `rev`.
pub fn hunks_since(root: &Path, files: &[PathBuf], rev: &str) -> Result<BTreeMap<PathBuf, Diff>, GitError> {
    let mut out = BTreeMap::new();
    let tracked: std::collections::HashSet<PathBuf> = {
        let top = top(root)?;
        git(root, &["ls-files", "--full-name", "-z"])?.split('\0').filter(|p| !p.is_empty()).map(|p| top.join(p)).collect()
    };
    for f in files {
        if !tracked.contains(f) {
            out.insert(f.clone(), Diff::Untracked);
            continue;
        }
        let diff = match git(root, &["diff", "-U0", "--no-color", "--no-ext-diff", "--no-relative", rev, "--", &f.to_string_lossy()]) {
            Ok(diff) => diff,
            Err(_) if unborn(root, rev)? => {
                out.insert(f.clone(), Diff::Untracked);
                continue;
            }
            Err(e) => return Err(e),
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
        out.insert(f.clone(), Diff::Hunks(hunks));
    }
    Ok(out)
}

/// Contents of the file at HEAD; `Ok(None)` if it is not there.
pub fn head_version(root: &Path, file: &Path) -> Result<Option<String>, GitError> {
    version_at(root, "HEAD", file)
}

/// Contents of the file at commit `rev`; `Ok(None)` if it is not there (or
/// is outside the repository).
pub fn version_at(root: &Path, rev: &str, file: &Path) -> Result<Option<String>, GitError> {
    let top = top(root)?;
    let Ok(rel) = file.strip_prefix(&top) else { return Ok(None) };
    let rel = rel.to_string_lossy();
    match git(root, &["show", &format!("{rev}:{rel}")]) {
        Ok(text) => Ok(Some(text)),
        // `show` fails alike for a path not at `rev` and for git failing:
        // HEAD before the first commit, or a listing of the path at `rev`
        // that succeeds and is empty, tells the first
        Err(_) if unborn(root, rev)? => Ok(None),
        Err(e) => match git(root, &["ls-tree", "-z", "--full-tree", rev, "--", &rel]) {
            Ok(listed) if listed.is_empty() => Ok(None),
            _ => Err(e),
        },
    }
}

pub fn in_ranges(line: u32, ranges: &Option<Vec<(u32, u32)>>) -> bool {
    match ranges {
        None => true,
        Some(r) => r.iter().any(|(a, b)| line >= *a && line <= *b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").output().unwrap();
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8(out.stdout).unwrap()
    }

    fn repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("grounds-git-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        run(&dir, &["init", "-q"]);
        dir.canonicalize().unwrap()
    }

    /// The answers that are "no" are values; git failing is an error.
    #[test]
    fn a_version_absent_at_a_revision_is_none_and_one_git_cannot_read_is_an_error() {
        let dir = repo("version");
        let (a, b) = (dir.join("sub/A.java"), dir.join("sub/B.java"));
        std::fs::write(&a, "class A {}\n").unwrap();
        // HEAD before the first commit holds no files
        assert_eq!(version_at(&dir, "HEAD", &a), Ok(None));
        assert_eq!(changed_files_since(&dir, "HEAD").map(|(_, deleted)| deleted), Ok(Vec::new()));
        run(&dir, &["add", "-A"]);
        assert_eq!(hunks_since(&dir, std::slice::from_ref(&a), "HEAD"), Ok([(a.clone(), Diff::Untracked)].into_iter().collect()));
        run(&dir, &["commit", "-qm", "a"]);
        assert_eq!(version_at(&dir, "HEAD", &a), Ok(Some("class A {}\n".into())));
        std::fs::write(&b, "class B {}\n").unwrap();
        assert_eq!(version_at(&dir, "HEAD", &b), Ok(None), "not at HEAD");
        assert_eq!(hunks_since(&dir, std::slice::from_ref(&b), "HEAD"), Ok([(b.clone(), Diff::Untracked)].into_iter().collect()));
        assert_eq!(changed_files(&dir), Ok(vec![b.clone()]));
        // the blob of A at HEAD gone from the object store: git fails to
        // show it, and that is not a file absent at HEAD
        let blob = run(&dir, &["rev-parse", "HEAD:sub/A.java"]);
        let blob = blob.trim();
        std::fs::remove_file(dir.join(".git/objects").join(&blob[..2]).join(&blob[2..])).unwrap();
        assert!(version_at(&dir, "HEAD", &a).is_err());
        assert!(version_at(&dir, "no-such-revision", &a).is_err(), "a revision that names nothing is no answer");
        // an index git cannot read: no list of changed files
        std::fs::write(dir.join(".git/index"), b"not an index").unwrap();
        assert!(changed_files(&dir).is_err());
        assert!(changed_files_since(&dir, "HEAD").is_err());
        assert!(hunks_since(&dir, std::slice::from_ref(&a), "HEAD").is_err());
        let _ = std::fs::remove_dir_all(&dir);
        // outside a repository: none, not an error
        let outside = std::env::temp_dir().join(format!("grounds-git-outside-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        if crate::workspace::git_dir(&outside).is_none() {
            assert_eq!(is_repo(&outside), Ok(false));
        }
        let _ = std::fs::remove_dir_all(&outside);
    }
}
