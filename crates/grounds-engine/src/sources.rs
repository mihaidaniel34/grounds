//! The source files a session holds, with one owner: the list of files, the
//! stamp each was read with and its stub in the index change together here,
//! and nowhere else, so that none of them can be out of step with the others.

use crate::project::{path_key, FileStamp, Project, Walk, Why};
use crate::workspace::{classify, SourceFile, Workspace};
use grounds_index::FileStub;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// A source file the session holds: listed in the workspace, and as it was
/// read.
#[derive(Clone, Copy, Debug)]
pub enum Entry {
    /// Its stub is in the index, made from the text this stamp was taken of.
    Indexed(FileStamp),
    /// Its text could not be read (no permission, not UTF-8, its directory
    /// not to be entered): there is no stub, as a fresh load has none, and the
    /// stamp vouches for nothing, so every sync reads it again.
    Unread(FileStamp),
}

impl Entry {
    pub fn stamp(&self) -> &FileStamp {
        match self {
            Entry::Indexed(stamp) | Entry::Unread(stamp) => stamp,
        }
    }

    fn stamp_mut(&mut self) -> &mut FileStamp {
        match self {
            Entry::Indexed(stamp) | Entry::Unread(stamp) => stamp,
        }
    }
}

/// A path a fresh load of the workspace would list as a source file: found
/// by a discovery (`discover`), or already held. Only this module makes one,
/// so a file is installed only through the one filter the discovery applies
/// (resources, excluded paths, files outside the root, hidden and ignored
/// directories, build output).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Admitted(PathBuf);

impl Admitted {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// What a discovery found (`Workspace::discover_in`), its source files as
/// admitted ones: `ws.files` is empty, they are in `files`.
pub struct Found {
    pub ws: Workspace,
    files: Vec<Admitted>,
}

impl Found {
    pub fn files(&self) -> &[Admitted] {
        &self.files
    }

    pub fn into_files(self) -> Vec<Admitted> {
        self.files
    }
}

/// `Workspace::discover_in`, with what it lists as source files admitted.
pub fn discover(root: &Path, start: Option<&Path>, max_depth: Option<usize>, exclude: &[String]) -> Found {
    let mut ws = Workspace::discover_in(root, start, max_depth, exclude);
    let files = std::mem::take(&mut ws.files).into_iter().map(|f| Admitted(f.path)).collect();
    Found { ws, files }
}

/// The stubs of some files and the bean model as they were before a base
/// state was put in (`Sources::lend`), to be put back (`Sources::put_back`).
pub struct Lent {
    stubs: Vec<(Arc<str>, Option<Arc<FileStub>>)>,
    di: grounds_di::DiModel,
}

/// The project of a session, and the source files it holds. The session
/// reads the project through it (`project`) and changes what is held only
/// through `install` and `forget`, which keep the entries, the workspace's
/// list of files and the index's stubs in step, and note that the release
/// plan is to be compared again.
pub struct Sources {
    project: Project,
    entries: FxHashMap<PathBuf, Entry>,
    /// A file was installed or forgotten since the release plan was last
    /// compared with what the sources need (`release_plan_changed`). It
    /// stays set until that comparison is done, also when a panic comes in
    /// between.
    replan: bool,
}

impl Sources {
    /// The project as loaded, and an entry for each file of its workspace.
    pub(crate) fn new(project: Project, entries: FxHashMap<PathBuf, Entry>) -> Sources {
        Sources { project, entries, replan: false }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn get(&self, path: &Path) -> Option<&Entry> {
        self.entries.get(path)
    }

    pub fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.entries.keys()
    }

    pub fn entries(&self) -> impl Iterator<Item = (&PathBuf, &Entry)> {
        self.entries.iter()
    }

    /// A file held, as admitted (it was when it was installed).
    pub fn held(&self, path: &Path) -> Option<Admitted> {
        self.entries.contains_key(path).then(|| Admitted(path.to_path_buf()))
    }

    /// The files that may differ from what their stamps were taken of
    /// (`FileStamp::changed`; `clock`: the time before this look). On all
    /// cores: after a checkout of the whole tree every file is racy and its
    /// content is compared.
    pub fn stale(&mut self, clock: SystemTime) -> Vec<Admitted> {
        use rayon::prelude::*;
        self.entries
            .par_iter_mut()
            .filter_map(|(p, entry)| {
                let stamp = entry.stamp_mut();
                let was = *stamp;
                if !stamp.changed(p, clock) {
                    return None;
                }
                crate::trace::log(|| format!("sync: stale {} recorded {} size {}{} now {}", p.display(), crate::trace::time(was.meta().map(|m| m.mtime)), was.meta().map_or(0, |m| m.size), if was.is_racy() { " racy" } else if was.vouches() { "" } else { " unread" }, crate::trace::time(std::fs::metadata(p).and_then(|m| m.modified()).ok())));
                Some(Admitted(p.clone()))
            })
            .collect()
    }

    /// Of `named`, the files a fresh load would list: those in a directory the
    /// discovery went into (`walked`), listed there as the discovery lists
    /// them. A directory is listed once for all the files named in it.
    pub fn admit(&self, named: &[PathBuf], walked: &Walk) -> Vec<Admitted> {
        let mut by_dir: std::collections::BTreeMap<&Path, Vec<&PathBuf>> = Default::default();
        for p in named {
            if let Some(d) = p.parent().filter(|d| walked.dirs.contains_key(*d)) {
                by_dir.entry(d).or_default().push(p);
            }
        }
        let ws = &self.project.workspace;
        let mut out = Vec::new();
        for (d, named) in by_dir {
            let listed = discover(&ws.root, Some(d), Some(1), &self.project.config.exclude);
            out.extend(listed.into_files().into_iter().filter(|f| named.contains(&&f.0)));
        }
        out
    }

    /// Hold `file` as read now: with its stub, or (`None`) as one whose text
    /// could not be read, whose stub, if it had one, goes. A stamp taken again
    /// of a time seen at an earlier look keeps the margin it began then.
    pub fn install(&mut self, file: Admitted, stamp: FileStamp, stub: Option<Arc<FileStub>>) {
        let Admitted(path) = file;
        let stamp = stamp.after(self.entries.get(&path).map(Entry::stamp));
        let key = path_key(&path);
        let entry = match stub {
            Some(stub) => {
                self.project.index.set_file(key, stub);
                Entry::Indexed(stamp)
            }
            None => {
                if self.project.index.file(&key).is_some() {
                    self.project.index.remove_file(&key);
                }
                Entry::Unread(stamp.recheck(Why::ReadFailed))
            }
        };
        if self.entries.insert(path.clone(), entry).is_none() {
            let ws = &mut self.project.workspace;
            let module = ws.module_of(&path).unwrap_or(0);
            ws.files.push(SourceFile { kind: classify(&path), path, module });
        }
        self.replan = true;
    }

    /// Hold none of `gone` any more: no entry, no stub, not in the list.
    pub fn forget(&mut self, gone: &FxHashSet<PathBuf>) {
        if gone.is_empty() {
            return;
        }
        for p in gone {
            self.project.index.remove_file(&path_key(p));
            self.entries.remove(p);
        }
        self.project.workspace.files.retain(|f| !gone.contains(&f.path));
        self.replan = true;
    }

    /// Whether the release plan a load would make now, from the sources as
    /// held, differs from the one the project was loaded with
    /// (`Project::release_plan_changed`); compared only when a file was
    /// installed or forgotten since the last comparison.
    pub fn release_plan_changed(&mut self) -> bool {
        if !self.replan {
            return false;
        }
        let changed = self.project.release_plan_changed();
        self.replan = false;
        changed
    }

    /// Follow the build picture after a sync (`clock`: the time before its
    /// look): read the build output again where it changed, and place every
    /// file again when it did or when `declarations` changed (a class added,
    /// renamed or removed changes which root a class file is evidence for, a
    /// file added or gone which roots there are); else place again only the
    /// files `installed`, whose time is all that may differ. As a fresh load
    /// would place them, from the stamps the files were read with.
    pub fn follow_build(&mut self, clock: SystemTime, declarations: bool, installed: &[PathBuf]) {
        let Sources { project, entries, .. } = self;
        let mtime = |p: &Path| entries.get(p).and_then(|e| e.stamp().meta()).map(|m| m.mtime);
        let mut moved = project.picture.refresh(clock);
        // (a file added may be the first of a unit found by its source set)
        if declarations {
            moved |= project.picture.add_candidates(&project.workspace);
        }
        if moved || declarations {
            let index = &project.index;
            let states = project.picture.place(&project.workspace, |p| index.file(&path_key(p)).cloned(), mtime);
            project.index.set_all_in_build(states);
            return;
        }
        for p in installed {
            let key = path_key(p);
            let state = project.picture.place_file(p, project.index.file(&key).map(|s| &**s), mtime(p));
            project.index.set_in_build(key, state);
        }
    }

    /// See `Project::rebuild_di`.
    pub fn rebuild_di(&mut self) {
        self.project.rebuild_di();
    }

    /// Set aside the stubs `touched` have and the bean model, for a base
    /// state to be put in their place (`hold_base`) until `put_back`.
    pub(crate) fn lend(&mut self, touched: &[PathBuf]) -> Lent {
        let stubs = touched.iter().map(|p| {
            let key = path_key(p);
            let stub = self.project.index.file(&key).cloned();
            (key, stub)
        }).collect();
        Lent { stubs, di: std::mem::take(&mut self.project.di) }
    }

    /// Hold `stub` (`None`: no stub) for a file whose own stub `lent` set
    /// aside, until it is put back; any other file is left as it is.
    pub(crate) fn hold_base(&mut self, lent: &Lent, key: Arc<str>, stub: Option<Arc<FileStub>>) {
        if !lent.stubs.iter().any(|(k, _)| *k == key) {
            return;
        }
        match stub {
            Some(stub) => self.project.index.set_file(key, stub),
            None => self.project.index.remove_file(&key),
        }
    }

    /// Put back the stubs and the bean model `lent` set aside.
    pub(crate) fn put_back(&mut self, lent: Lent) {
        for (key, stub) in lent.stubs {
            match stub {
                Some(stub) => self.project.index.set_file(key, stub),
                None => self.project.index.remove_file(&key),
            }
        }
        self.project.di = lent.di;
    }
}
