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
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Default, Clone)]
pub struct LoadStats {
    pub files: usize,
    pub parsed: usize,
    pub cached: usize,
    pub classpath_entries: usize,
    pub classpath_classes: usize,
    /// The Java release whose JDK API is checked against.
    pub java_release: u32,
    pub t_discover: Duration,
    pub t_classpath: Duration,
    pub t_stubs: Duration,
    pub t_index: Duration,
    pub t_resolve: Duration,
    pub t_di: Duration,
    pub resolve: grounds_index::order::ResolveStats,
}

pub struct Project {
    pub config: Config,
    pub workspace: Workspace,
    pub index: Index,
    pub stats: LoadStats,
    /// Dependency-injection model (rebuilt when declarations change).
    pub di: grounds_di::DiModel,
    /// What the configuration says about beans, which every model of this
    /// project is built with.
    pub di_settings: grounds_di::Settings,
    pub source_sets: SourceSets,
    pub lombok: LombokConfigs,
    /// classes listed in META-INF/spring.factories and META-INF/spring/*.imports
    pub registered: Vec<String>,
    /// the auto-configurations among them
    pub auto_registered: Vec<String>,
    /// Classpath problems found at load (missing, stale, too few jars).
    pub classpath_warnings: Vec<crate::classpath::Warning>,
    /// The jars and class directories the classpath was built from.
    pub classpath_entries: Vec<PathBuf>,
    /// The JDK API loaded and the release of each module
    /// (`java_release::plan`): the classpath, the index's exact releases and
    /// release floors, and resolution depend on it.
    pub releases: crate::java_release::ReleasePlan,
    /// What the plan took from the sources (see `release_inputs`): a sync
    /// that changes it plans again.
    pub release_inputs: ReleaseInputs,
}

/// What the release plan depends on in the sources, per module (in
/// `ws.modules` order): whether it holds code (`java_release::plan` counts
/// only modules that do) and the lowest release its code needs. The rest of
/// what the plan reads (build files, `grounds.toml`, the installed JDK) a
/// session follows as configuration, loading the project again when it
/// changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReleaseInputs {
    pub with_code: Vec<bool>,
    pub needs: Vec<Option<u32>>,
}

impl ReleaseInputs {
    /// Of the workspace's files, with the stub each has (none for a file that
    /// could not be read, which needs nothing, as at load).
    pub fn of<'a>(ws: &Workspace, stub: impl Fn(&Path) -> Option<&'a FileStub>) -> ReleaseInputs {
        let mut r = ReleaseInputs { with_code: vec![false; ws.modules.len()], needs: vec![None; ws.modules.len()] };
        for f in &ws.files {
            if f.kind != SourceKind::Generated {
                if let Some(w) = r.with_code.get_mut(f.module) {
                    *w = true;
                }
            }
            if let (Some(n), Some(need)) = (r.needs.get_mut(f.module), stub(&f.path).and_then(|s| code_needs(&f.path, s))) {
                *n = (*n).max(Some(need));
            }
        }
        r
    }

    /// As `java_release::plan` takes them: (module, lowest release needed).
    pub fn needs(&self) -> Vec<(usize, u32)> {
        self.needs.iter().enumerate().filter_map(|(m, n)| n.map(|n| (m, n))).collect()
    }
}

/// Class names registered through Spring's META-INF files in any module, and
/// the auto-configurations among them (`EnableAutoConfiguration` in
/// spring.factories, `AutoConfiguration.imports`).
fn spring_registrations(ws: &Workspace) -> (Vec<String>, Vec<String>) {
    let (mut out, mut auto) = (Vec::new(), Vec::new());
    for m in &ws.modules {
        let meta = m.dir.join("src/main/resources/META-INF");
        if let Ok(t) = crate::fsutil::read_text(meta.join("spring.factories")) {
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
                if let Ok(t) = crate::fsutil::read_text(e.path()) {
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
    /// The `lombok.config` files found.
    pub fn files(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.dirs.iter().map(|(d, _)| d.join("lombok.config"))
    }

    pub fn discover(root: &Path) -> LombokConfigs {
        let mut dirs = Vec::new();
        let walker = ignore::WalkBuilder::new(root)
            .hidden(false)
            .filter_entry(|e| !matches!(e.file_name().to_str(), Some(".git" | "build" | "target" | "node_modules" | ".gradle" | "out" | "src")))
            .build();
        for e in walker.flatten() {
            if e.file_name() == "lombok.config" {
                if let (Some(dir), Ok(text)) = (e.path().parent(), crate::fsutil::read_text(e.path())) {
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
        // the unmodelled keys whose effective value is not their default: a
        // nearer config setting one back to its default, or clearing it, counts
        let mut unmodelled: Vec<&str> = Vec::new();
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
                    ("lombok.getter.noisprefix", "=") => cfg.no_is_prefix = truthy,
                    ("lombok.getter.noisprefix", "clear") => cfg.no_is_prefix = false,
                    ("lombok.builder.classname", "=") => cfg.builder_class_name = Some(v.clone()),
                    ("lombok.builder.classname", "clear") => cfg.builder_class_name = None,
                    ("lombok.log.fieldname", "=") => cfg.log_field_name = Some(v.clone()),
                    ("lombok.log.fieldname", "clear") => cfg.log_field_name = None,
                    ("lombok.fieldnameconstants.innertypename", "=") => cfg.field_name_constants_type = Some(v.clone()),
                    ("lombok.fieldnameconstants.innertypename", "clear") => cfg.field_name_constants_type = None,
                    ("lombok.fieldnameconstants.uppercase", "=") => cfg.field_name_constants_uppercase = truthy,
                    ("lombok.fieldnameconstants.uppercase", "clear") => cfg.field_name_constants_uppercase = false,
                    // keys that change members in ways not modelled, at a
                    // value other than their default
                    (k, "=") if grounds_index::lombok::UNMODELLED_KEYS.contains(&k) => {
                        let default = match k {
                            "lombok.accessors.capitalization" => v.eq_ignore_ascii_case("basic"),
                            "lombok.accessors.javabeansspeccapitalization" | "lombok.singular.useguava" | "lombok.noargsconstructor.extraprivate" => v.eq_ignore_ascii_case("false"),
                            "lombok.log.fieldisstatic" => truthy,
                            _ => false,
                        };
                        unmodelled.retain(|u| *u != k);
                        if !default {
                            unmodelled.push(k);
                        }
                    }
                    (k, "clear") => unmodelled.retain(|u| *u != k),
                    _ => {}
                }
            }
        }
        cfg.unmodelled = !unmodelled.is_empty();
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

/// The lowest release a file's declarations need: a record needs Java 16,
/// a sealed class 17, a module import 25. Multi-release sources
/// (`src/main/java17`) compile for their own release and say nothing about
/// their module's.
fn code_needs(path: &Path, stub: &FileStub) -> Option<u32> {
    use grounds_index::{ClassKind, Flags};
    let comps: Vec<_> = path.components().collect();
    if comps.windows(3).any(|w| w[0].as_os_str() == "src" && w[2].as_os_str().to_str().and_then(|d| d.strip_prefix("java")).is_some_and(|v| !v.is_empty() && v.trim_start_matches('-').chars().all(|c| c.is_ascii_digit()))) {
        return None;
    }
    let mut need = None;
    for c in &stub.classes {
        if c.kind == ClassKind::Record {
            need = need.max(Some(16));
        }
        if c.flags.has(Flags::SEALED) || c.flags.has(Flags::NON_SEALED) || !c.permits.is_empty() {
            need = need.max(Some(17));
        }
    }
    if stub.imports.iter().any(|i| i.from_module) {
        need = need.max(Some(25));
    }
    need
}

pub fn path_key(p: &Path) -> Arc<str> {
    Arc::from(p.to_string_lossy().as_ref())
}

/// How long a file's time stays in doubt. A time names a tick of the file
/// system's clock (seconds, on some), and a write later in the same tick
/// leaves the time as it was: a file read during the tick of its own time
/// may be written again without showing it. The tick is over once the time is
/// this old by our clock, or once it has been seen unchanged for this long
/// (which holds whatever the file system's clock says, a time in the future
/// included).
pub const MARGIN: Duration = Duration::from_secs(2);

/// Whether a write after `look` could still leave a file's time at `time`.
pub fn recent(time: SystemTime, look: SystemTime) -> bool {
    time >= look.checked_sub(MARGIN).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// What a look at a file's metadata found: its modification time, change
/// time, permissions and size, and when that time was first seen (right
/// after it was read; a stamp taken again keeps the earliest look at the same
/// time, see `FileStamp::after`).
///
/// The change time and the permissions are there for what leaves the
/// modification time and the content alone but changes what can be read:
/// `chmod`, `chown`, an ACL. A directory that can no longer be listed or
/// entered, or an ignore file that can no longer be read, is then looked at
/// again, as a fresh load would find it.
#[derive(Clone, Copy, Debug)]
pub struct Meta {
    pub mtime: SystemTime,
    /// The inode's change time (`st_ctime`; the epoch where there is none).
    pub ctime: SystemTime,
    /// The permission bits and file type (`st_mode`; on other systems, only
    /// whether the file is read-only).
    pub mode: u32,
    pub size: u64,
    pub seen: Instant,
}

impl Meta {
    /// What a look at a file's metadata stamps.
    fn of(meta: &std::fs::Metadata) -> Option<Meta> {
        let mtime = meta.modified().ok()?;
        #[cfg(unix)]
        let (ctime, mode) = {
            use std::os::unix::fs::MetadataExt;
            let ctime = u64::try_from(meta.ctime()).map_or(SystemTime::UNIX_EPOCH, |s| SystemTime::UNIX_EPOCH + Duration::new(s, meta.ctime_nsec().clamp(0, 999_999_999) as u32));
            (ctime, meta.mode())
        };
        #[cfg(not(unix))]
        let (ctime, mode) = (SystemTime::UNIX_EPOCH, meta.permissions().readonly() as u32);
        Some(Meta { mtime, ctime, mode, size: meta.len(), seen: Instant::now() })
    }

    /// The later of the modification and the change time: what a write, or a
    /// change of permissions, last moved.
    pub fn time(&self) -> SystemTime {
        self.mtime.max(self.ctime)
    }

    /// The same times, mode and size, whenever they were seen.
    fn same(&self, other: &Meta) -> bool {
        (self.mtime, self.ctime, self.mode, self.size) == (other.mtime, other.ctime, other.mode, other.size)
    }
}

/// A file as it was read: its metadata, taken before its content was read,
/// and a hash of that content (of the names in it, for a directory).
///
/// Taken in that order, a write after the read changes the time or the size,
/// except a write in the same tick of the file system's clock, which can
/// keep both. So, as git does for its index, a time that is recent when it is
/// taken is not trusted alone: while the stamp is `Racy`, every look at the
/// file compares its content with the hash too, until the time is older than
/// `MARGIN` or has been seen unchanged for that long. That is the files
/// written in the last seconds, not the project.
#[derive(Clone, Copy, Debug)]
pub enum FileStamp {
    /// The metadata decides. `hash`: of the content, when it was read (asked
    /// for, or the time was recent); `None` when it was not asked for and the
    /// time was old, or when it could not be read, and then the metadata is
    /// all there is to go by (an ignore file that cannot be read is read again
    /// cheaply, see `read_otherwise`). With a hash, a change of the change
    /// time alone is no change while the content still reads the same.
    Settled { meta: Meta, hash: Option<u64> },
    /// The time was recent when taken: every look compares the content with
    /// `hash` too, until the time is old or has been seen unchanged for
    /// `MARGIN`, and the stamp is `Settled`.
    Racy { meta: Meta, hash: u64 },
    /// Nothing was read that this stamp could vouch for: every sync takes the
    /// file as changed and looks again. `meta`: what the look at its
    /// metadata found, if there was one.
    Recheck { meta: Option<Meta>, why: Why },
}

/// Why a stamp vouches for nothing (`FileStamp::Recheck`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// A source file whose text could not be read: no permission, not UTF-8,
    /// gone between the look at its time and the read.
    ReadFailed,
    /// A file a listing names but that cannot be looked at (its directory can
    /// be read but not entered), or that went since it was listed.
    Unreachable,
    /// A directory that cannot be listed or entered (no permission).
    Unlistable,
    /// Stamped after a walk that may have read it otherwise: its time is
    /// recent as of the start of the walk, or its content could be read before
    /// the walk and not after it, or the reverse.
    Raced,
    /// git's index or HEAD, whose content could not be read: which files git
    /// tracks is asked again.
    GitUnreadable,
}

/// The hash a stamp keeps: of the bytes of a file, of the sorted names in a
/// directory. A source file's is `content_hash` of its text.
fn content_of(path: &Path, dir: bool) -> (Option<u64>, Option<Vec<u8>>) {
    if dir {
        let Ok(entries) = std::fs::read_dir(path) else { return (None, None) };
        let mut names: Vec<_> = entries.flatten().map(|e| e.file_name()).collect();
        names.sort();
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        for n in &names {
            h.update(n.as_encoded_bytes());
            h.update(b"\0");
        }
        return (Some(h.digest()), None);
    }
    // (not a pipe or a device: one listed as a jar would block, or never end)
    match crate::fsutil::read_bytes(path) {
        Ok(bytes) => (Some(xxhash_rust::xxh3::xxh3_64(&bytes)), Some(bytes)),
        Err(_) => (None, None),
    }
}

impl FileStamp {
    /// Time and size, then, if `always` or the time is recent, the content.
    fn take(path: &Path, always: bool) -> (Option<FileStamp>, Option<Vec<u8>>) {
        let clock = SystemTime::now();
        let Ok(m) = std::fs::metadata(path) else { return (None, None) };
        let Some(meta) = Meta::of(&m) else { return (None, None) };
        let racy = recent(meta.time(), clock);
        let (hash, bytes) = if always || racy { content_of(path, m.is_dir()) } else { (None, None) };
        // (a recent time whose content cannot be read leaves nothing to
        // compare: the metadata decides)
        let stamp = match hash {
            Some(hash) if racy => FileStamp::Racy { meta, hash },
            hash => FileStamp::Settled { meta, hash },
        };
        (Some(stamp), bytes)
    }

    /// What the look at the file's metadata found, if there was one.
    pub fn meta(&self) -> Option<&Meta> {
        match self {
            FileStamp::Settled { meta, .. } | FileStamp::Racy { meta, .. } => Some(meta),
            FileStamp::Recheck { meta, .. } => meta.as_ref(),
        }
    }

    /// See `Meta::time`.
    pub fn time(&self) -> Option<SystemTime> {
        self.meta().map(Meta::time)
    }

    /// The hash of the content this stamp vouches for, if it has one.
    pub fn hash(&self) -> Option<u64> {
        match *self {
            FileStamp::Settled { hash, .. } => hash,
            FileStamp::Racy { hash, .. } => Some(hash),
            FileStamp::Recheck { .. } => None,
        }
    }

    pub fn is_racy(&self) -> bool {
        matches!(self, FileStamp::Racy { .. })
    }

    /// Whether the stamp vouches for what was read (is not `Recheck`).
    pub fn vouches(&self) -> bool {
        !matches!(self, FileStamp::Recheck { .. })
    }

    /// This stamp as one that vouches for nothing, for `why` (one that
    /// already vouches for nothing keeps its first reason).
    pub fn recheck(self, why: Why) -> FileStamp {
        match self {
            FileStamp::Recheck { .. } => self,
            _ => FileStamp::Recheck { meta: self.meta().copied(), why },
        }
    }

    /// The stamp of a source file and its text (if it is UTF-8), the stamp
    /// first. No stamp if the file is gone; one that vouches for nothing if
    /// there is no text.
    pub fn read(path: &Path) -> (Option<FileStamp>, Option<String>) {
        let (stamp, bytes) = Self::take(path, true);
        let text = bytes.and_then(|b| String::from_utf8(b).ok());
        let stamp = if text.is_some() { stamp } else { stamp.map(|s| s.recheck(Why::ReadFailed)) };
        (stamp, text)
    }

    /// The stamp of a file or directory, with a hash of its content when its
    /// time is recent or `with_hash`. (Not of every jar on a classpath.)
    /// Never `Recheck`: what could not be read is left to the caller.
    pub fn of(path: &Path, with_hash: bool) -> Option<FileStamp> {
        Self::take(path, with_hash).0
    }

    /// The same metadata, and the same content where both have a hash. (A
    /// stamp without one was taken of a time that was already old, or of a
    /// file that could not be read.) A stamp that vouches for nothing is the
    /// same as none.
    pub fn same(&self, other: &FileStamp) -> bool {
        match (self.vouches() && other.vouches(), self.meta(), other.meta()) {
            (true, Some(a), Some(b)) => a.same(b) && (self.hash().is_none() || other.hash().is_none() || self.hash() == other.hash()),
            _ => false,
        }
    }

    /// This stamp, taken again of a file stamped `old` before: a time already
    /// seen then has been seen since then, and its margin counts from there.
    pub fn after(mut self, old: Option<&FileStamp>) -> FileStamp {
        let Some(old) = old.and_then(FileStamp::meta).copied() else { return self };
        let meta = match &mut self {
            FileStamp::Settled { meta, .. } | FileStamp::Racy { meta, .. } | FileStamp::Recheck { meta: Some(meta), .. } => meta,
            FileStamp::Recheck { meta: None, .. } => return self,
        };
        if (meta.mtime, meta.ctime) == (old.mtime, old.ctime) {
            meta.seen = meta.seen.min(old.seen);
            if let FileStamp::Racy { meta, hash } = self {
                if meta.seen.elapsed() >= MARGIN {
                    return FileStamp::Settled { meta, hash: Some(hash) };
                }
            }
        }
        self
    }

    /// Whether the file may differ from what this stamp was taken of
    /// (`clock`: the time before this look). A racy stamp whose content is
    /// found unchanged stays, and is settled once its margin is over.
    pub fn changed(&mut self, path: &Path, clock: SystemTime) -> bool {
        let (meta, hash) = match *self {
            FileStamp::Recheck { .. } => return true,
            FileStamp::Settled { meta, hash } => (meta, hash),
            FileStamp::Racy { meta, hash } => (meta, Some(hash)),
        };
        let Ok(m) = std::fs::metadata(path) else { return true };
        let Some(now) = Meta::of(&m) else { return true };
        if (now.mtime, now.mode, now.size) != (meta.mtime, meta.mode, meta.size) {
            return true;
        }
        if now.ctime != meta.ctime {
            // only the change time moved (a hard link, `chown`, an ACL, an
            // extended attribute): a file whose content is still read as it
            // was is the same file. (Not a directory: whether it can be
            // entered is not in its names, and listing it again is cheap.)
            let Some(hash) = hash.filter(|_| !m.is_dir()) else { return true };
            if content_of(path, false).0 != Some(hash) {
                return true;
            }
            let meta = Meta { ctime: now.ctime, seen: Instant::now(), ..meta };
            *self = if recent(meta.time(), clock) { FileStamp::Racy { meta, hash } } else { FileStamp::Settled { meta, hash: Some(hash) } };
            return false;
        }
        let FileStamp::Racy { meta, hash } = *self else { return false };
        if content_of(path, m.is_dir()).0 != Some(hash) {
            return true;
        }
        if !(recent(meta.time(), clock) && meta.seen.elapsed() < MARGIN) {
            *self = FileStamp::Settled { meta, hash: Some(hash) };
        }
        false
    }

    /// For an ignore file stamped as a walk read it: whether a walk now
    /// could read it otherwise. One that could not be read counted for the
    /// walk as if it were not there, whatever its content, as it does for a
    /// fresh load; so it is read again, and only a read that succeeds or a
    /// change of its metadata says so. That costs a look and a failed open
    /// per sync, not a walk of the workspace. (One whose read the walk may
    /// have raced vouches for nothing, and says so until walked again.)
    pub fn read_otherwise(&mut self, path: &Path, clock: SystemTime) -> bool {
        let FileStamp::Settled { meta, hash: None } = *self else { return self.changed(path, clock) };
        match FileStamp::of(path, true) {
            Some(now) => now.hash().is_some() || !now.meta().is_some_and(|n| n.same(&meta)),
            None => true,
        }
    }
}

/// Stamps of what a walk for source files read: the directories it listed
/// and the ignore files that applied (see `Workspace::discover_in`).
#[derive(Clone, Debug, Default)]
pub struct Walk {
    pub dirs: rustc_hash::FxHashMap<PathBuf, FileStamp>,
    pub ignores: Vec<(PathBuf, Option<FileStamp>)>,
    /// The directories walked where a repository starts, and how
    /// (`workspace::boundary`): that decides whether ignore files count and
    /// which exclude file applies.
    pub gits: rustc_hash::FxHashMap<PathBuf, u8>,
    /// The same for the directories above the root, a repository there or not.
    pub outer_gits: Vec<(PathBuf, Option<u8>)>,
    /// A repository started or ended while the walk ran: it may have skipped
    /// files by the other one, and the next sync walks again.
    pub pending: bool,
    /// Of a walk of the whole workspace: git's index and HEAD for the work
    /// tree (`workspace::index_files`; `None`: not there), and the files git
    /// tracks although ignored that the walk read with them
    /// (`Workspace::tracked`). Staging, committing or checking out changes
    /// which files those are, and no directory's time shows it. While they
    /// are `Unknown` (git could not list them when last asked), every sync
    /// asks again, without walking the workspace, until git answers.
    pub git: Vec<(PathBuf, Option<FileStamp>)>,
    pub tracked: crate::workspace::Tracked,
}

impl Walk {
    /// Stamp `dirs`, which a walk that began at `began` listed, and the ignore
    /// files in them. The walk does not say when it listed each directory, so
    /// the stamps are taken after it: one whose time is recent as of `began`
    /// may have been written while the walk ran, after its listing, and
    /// vouches for nothing (`Why::Raced`), to be looked at again at the next
    /// sync; so does a directory that cannot be listed (no permission), until
    /// it can. `before`: ignore files stamped before the walk began; one the
    /// same before and after it is what the walk read, and keeps the time it
    /// was first seen (a time in the future is trusted once seen for the
    /// margin). An ignore file whose content cannot be read is stamped without
    /// a hash (see `FileStamp::read_otherwise`). With `root`, the walk was of
    /// the whole workspace there: the ignore files and repositories above it
    /// are stamped too, and git's index and HEAD (as ignore files, and one
    /// whose content cannot be read vouches for nothing; `of_workspace` adds
    /// the tracked-but-ignored files read with them). `bounds`: where repositories
    /// started (`workspace::boundary`) just before the walk began, in the
    /// directories known then; one that differs now makes the walk `pending`.
    pub fn stamp(dirs: &[PathBuf], root: Option<&Path>, began: SystemTime, before: &rustc_hash::FxHashMap<PathBuf, FileStamp>, bounds: &rustc_hash::FxHashMap<PathBuf, Option<u8>>) -> Walk {
        use crate::workspace::{boundary, index_files, listable, outer_ignore_files, repo_inputs, IGNORE_FILES};
        let mark = |p: &Path, s: FileStamp| match before.get(p).filter(|b| b.same(&s)) {
            Some(b) => s.after(Some(b)),
            None if s.time().is_some_and(|t| recent(t, began)) => s.recheck(Why::Raced),
            None => s,
        };
        let dir_mark = |d: &Path, s: FileStamp| {
            let s = mark(d, s);
            if listable(d) { s } else { s.recheck(Why::Unlistable) }
        };
        // (with the hash of its content, when it can be read: one that
        // cannot is read again at each sync, and walked again once it can be,
        // see `FileStamp::read_otherwise`; one read before the walk and not
        // after it, or the reverse, was read by the walk either way)
        let ignore = |p: &Path| {
            FileStamp::of(p, true).map(|s| {
                let flipped = before.get(p).is_some_and(|b| b.hash().is_some() != s.hash().is_some());
                let s = mark(p, s);
                if flipped { s.recheck(Why::Raced) } else { s }
            })
        };
        let dirs: rustc_hash::FxHashMap<PathBuf, FileStamp> = dirs.par_iter().filter_map(|d| FileStamp::of(d, false).map(|s| (d.clone(), dir_mark(d, s)))).collect();
        let gits: rustc_hash::FxHashMap<PathBuf, u8> = dirs.par_iter().filter_map(|(d, _)| boundary(d).map(|k| (d.clone(), k))).collect();
        // (an entry that is there but cannot be looked at, a symlink whose
        // target is gone or out of reach, is kept without a stamp: a fresh
        // load reads none, and the first sync that can look at it walks)
        let mut ignores: Vec<(PathBuf, Option<FileStamp>)> = dirs
            .par_iter()
            .flat_map_iter(|(d, _)| {
                IGNORE_FILES.iter().filter_map(|n| {
                    let p = d.join(n);
                    match ignore(&p) {
                        Some(s) => Some((p, Some(s))),
                        None => std::fs::symlink_metadata(&p).is_ok().then_some((p, None)),
                    }
                })
            })
            .collect();
        // what every repository the walk went into reads its excludes
        // through, there or not
        let mut listed: Vec<PathBuf> = gits.keys().flat_map(|d| repo_inputs(d)).collect();
        let (mut outer_gits, mut git) = (Vec::new(), Vec::new());
        if let Some(root) = root {
            listed.extend(outer_ignore_files(root));
            outer_gits = root.ancestors().skip(1).map(|a| (a.to_path_buf(), boundary(a))).collect();
            // (one that cannot be read leaves git asked again at each sync)
            git = index_files(root).into_iter().map(|p| { let s = ignore(&p).map(|s| if s.hash().is_some() { s } else { s.recheck(Why::GitUnreadable) }); (p, s) }).collect();
        }
        let pending = bounds.par_iter().any(|(d, b)| boundary(d) != *b);
        ignores.extend(listed.iter().map(|p| (p.clone(), ignore(p))));
        ignores.sort_by(|a, b| a.0.cmp(&b.0));
        ignores.dedup_by(|a, b| a.0 == b.0);
        Walk { dirs, ignores, gits, outer_gits, pending, git, tracked: Default::default() }
    }

    /// The same, of a walk of the whole workspace that found `ws`.
    pub fn of_workspace(ws: &Workspace, began: SystemTime, before: &rustc_hash::FxHashMap<PathBuf, FileStamp>, bounds: &rustc_hash::FxHashMap<PathBuf, Option<u8>>) -> Walk {
        Walk { tracked: ws.tracked.clone(), ..Walk::stamp(&ws.dirs, Some(&ws.root), began, before, bounds) }
    }
}

/// What a load read, for the `Session` that keeps its project and tells
/// later what changed: taken by value, once (`Session::from_project`).
pub struct Loaded {
    /// What each source file was when its stub was made.
    stamps: rustc_hash::FxHashMap<PathBuf, FileStamp>,
    /// The directories the discovery of source files went into, and the
    /// ignore files that decided what it skipped (`None`: not there), for
    /// telling later that a source file appeared.
    walked: Walk,
    /// When loading began. A file written after this may have been read
    /// before or after the write.
    began: SystemTime,
    /// For a load again: the files stamped before it began.
    before: Option<Before>,
}

impl Loaded {
    /// The source stamps, the walk, when the load began, and the files
    /// stamped before it.
    pub(crate) fn into_parts(self) -> (rustc_hash::FxHashMap<PathBuf, FileStamp>, Walk, SystemTime, Option<Before>) {
        (self.stamps, self.walked, self.began, self.before)
    }
}

/// Files stamped (`None`: not there) just before a load began, to tell after
/// it whether the load read them as they are then. `Project::reload` takes
/// one, so the look cannot come after the load it is compared with.
pub struct Before(rustc_hash::FxHashMap<PathBuf, Option<FileStamp>>);

impl Before {
    /// Stamp `paths` now, with the hash of their content.
    pub fn take<'a>(paths: impl Iterator<Item = &'a Path>) -> Before {
        Before(paths.map(|p| (p.to_path_buf(), FileStamp::of(p, true))).collect())
    }

    /// The stamp `p` had, if it was stamped: `Some(None)` if it was not there.
    pub fn get(&self, p: &Path) -> Option<&Option<FileStamp>> {
        self.0.get(p)
    }
}

impl Project {
    pub fn load(root: &Path) -> anyhow::Result<Project> {
        Ok(Self::load_stamped(root)?.0)
    }

    /// The project, and what the load read for a session to keep.
    pub fn load_stamped(root: &Path) -> anyhow::Result<(Project, Loaded)> {
        Self::begin(root, None)
    }

    /// The project loaded again, after the files in `before` were stamped.
    pub fn reload(root: &Path, before: Before) -> anyhow::Result<(Project, Loaded)> {
        Self::begin(root, Some(before))
    }

    fn begin(root: &Path, before: Option<Before>) -> anyhow::Result<(Project, Loaded)> {
        // before grounds.toml is read: it is one of the files that may be
        // written while the load runs
        let began = std::time::SystemTime::now();
        let config = Config::load(root)?;
        let (project, mut loaded) = Self::load_from(root, config, began)?;
        loaded.before = before;
        Ok((project, loaded))
    }

    pub fn load_with(root: &Path, config: Config) -> anyhow::Result<Project> {
        Ok(Self::load_from(root, config, std::time::SystemTime::now())?.0)
    }

    fn load_from(root: &Path, config: Config, load_began: std::time::SystemTime) -> anyhow::Result<(Project, Loaded)> {
        let mut stats = LoadStats::default();
        let t = Instant::now();
        let source_sets = SourceSets::load(root);
        crate::workspace::set_extra_test_sets(config.test_source_sets.iter().cloned().chain(source_sets.test_sets()).collect());
        crate::workspace::register_project_root(&root.canonicalize().unwrap_or_else(|_| root.to_path_buf()));
        let workspace = Workspace::discover(root, &config.exclude);
        let walked = Walk::of_workspace(&workspace, load_began, &Default::default(), &Default::default());
        stats.t_discover = t.elapsed();
        stats.files = workspace.files.len();

        let t = Instant::now();
        let lombok = LombokConfigs::discover(&workspace.root);
        let cache = StubCache::load(&workspace.root);
        // (with the stamp of each file, taken before it is read)
        let read: Vec<(PathBuf, Option<FileStamp>, Option<(u64, Arc<FileStub>, bool)>)> = workspace
            .files
            .par_iter()
            .map(|f| {
                let (stamp, text) = FileStamp::read(&f.path);
                let stub = text.map(|text| {
                    let h = stamp.and_then(|s| s.hash()).unwrap_or_else(|| content_hash(&text)) ^ lombok.key_for(&f.path);
                    let key = f.path.to_string_lossy();
                    if let Some(s) = cache.get(&key, h) {
                        return (h, s, true);
                    }
                    let pf = ParsedFile::parse(text);
                    (h, Arc::new(extract_with(&pf, &lombok.for_file(&f.path))), false)
                });
                (f.path.clone(), stamp, stub)
            })
            .collect();
        let mut stamps = rustc_hash::FxHashMap::default();
        let mut results: Vec<(PathBuf, u64, Arc<FileStub>, bool)> = Vec::with_capacity(read.len());
        for (p, stamp, stub) in read {
            // (a file listed that cannot be looked at, its directory not to
            // be entered, or gone since: tried again at the next sync)
            stamps.insert(p.clone(), stamp.unwrap_or(FileStamp::Recheck { meta: None, why: Why::Unreachable }));
            if let Some((h, s, cached)) = stub {
                results.push((p, h, s, cached));
            }
        }
        stats.t_stubs = t.elapsed();

        let t = Instant::now();
        let entries = crate::classpath::collect(&workspace, &config);
        stats.classpath_entries = entries.len();
        let java_home = config.java_home.clone().or_else(grounds_classfile::find_java_home);
        // the release the build states, checked against what the code uses
        let stubs: std::collections::HashMap<&Path, &FileStub> = results.iter().map(|(p, _, stub, _)| (p.as_path(), &**stub)).collect();
        let release_inputs = ReleaseInputs::of(&workspace, |p| stubs.get(p).copied());
        drop(stubs);
        let releases = config.release_plan(&workspace, &release_inputs.needs());
        stats.java_release = releases.load;
        let classpath = Arc::new(Classpath::build(java_home.as_deref(), stats.java_release, &entries));
        // a JDK home without lib/ct.sym is reported by the classpath warnings
        if !classpath.contains("java.lang.Object") && java_home.is_none() {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                eprintln!("grounds: warning: no JDK found (set JAVA_HOME, put `java` on PATH, or `java_home` in grounds.toml); checks that need the JDK's classes stay silent");
            });
        }
        stats.classpath_classes = classpath.len();
        stats.t_classpath = t.elapsed();

        let t = Instant::now();
        let mut index = Index::new(classpath);
        index.set_release_floors(releases.floors.iter().map(|(d, r)| (d.to_string_lossy().to_string(), *r)).collect());
        index.set_exact_releases(releases.exact.iter().map(|(d, r)| (d.to_string_lossy().to_string(), *r)).collect());
        index.set_modules_built_against_release(crate::workspace::modules_built_against_own_release(&workspace).iter().map(|d| d.to_string_lossy().to_string()).collect());
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
        let di_settings = grounds_di::Settings { lombok_copies_qualifiers: lombok_copies_qualifiers(&workspace.root), bean_overriding_allowed: bean_overriding_allowed(&workspace) };
        // resolve every class on all cores before di model asks for them one by one
        let t = Instant::now();
        stats.resolve = index.resolve_ordered();
        stats.t_resolve = t.elapsed();
        let t = Instant::now();
        let di =grounds_di::DiModel::build_with(&index, &registered, &auto_registered, di_settings);
        stats.t_di = t.elapsed();
        let classpath_warnings = crate::classpath::warnings(&workspace, &config, &index);
        let project = Project { config, workspace, index, stats, di, di_settings, source_sets, lombok, registered, auto_registered, classpath_warnings, classpath_entries: entries, releases, release_inputs };
        Ok((project, Loaded { stamps, walked, began: load_began, before: None }))
    }

    /// Whether the release plan a load would make now, from the sources as
    /// the index holds them, differs from the one this project was loaded
    /// with. Plans again only when what the sources tell it changed (a
    /// module gained its first source or lost its last, a file needs a newer
    /// release): otherwise no more than a pass over the files in memory.
    pub fn release_plan_changed(&mut self) -> bool {
        let inputs = ReleaseInputs::of(&self.workspace, |p| self.index.file(&p.to_string_lossy()).map(|s| &**s));
        if inputs == self.release_inputs {
            return false;
        }
        let plan = self.config.release_plan(&self.workspace, &inputs.needs());
        self.release_inputs = inputs;
        plan != self.releases
    }

    pub fn source_kind(&self, path: &Path) -> SourceKind {
        crate::workspace::classify(path)
    }

    /// Re-read one file and update the index. Returns true if its
    /// declaration signature changed.
    pub fn refresh_file(&mut self, path: &Path) -> bool {
        if crate::workspace::is_resource(path.strip_prefix(&self.workspace.root).unwrap_or(path)) {
            return false;
        }
        let key = path_key(path);
        let old = self.index.file(&key).map(|s| s.signature_hash);
        match crate::fsutil::read_text(path) {
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
        self.index.ensure_resolved();
        self.di = grounds_di::DiModel::build_with(&self.index, &self.registered, &self.auto_registered, self.di_settings);
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
                && crate::fsutil::read_text(e.path()).is_ok_and(|t| {
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
        if let Ok(t) = crate::fsutil::read_text(&cfg) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, bytes: &[u8], time: SystemTime) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("grounds-stamp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("A.java");
        std::fs::write(&p, bytes).unwrap();
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(time).unwrap();
        p
    }

    /// A read that finds no text leaves a stamp that vouches for nothing:
    /// every look takes the file as changed while its metadata stays as it
    /// was, and no stamp taken without that read is the same as it.
    #[test]
    fn a_stamp_from_a_failed_read_never_vouches_for_the_file() {
        let old = SystemTime::now() - Duration::from_secs(3600);
        // (not UTF-8: a read that fails whoever runs the test)
        let p = file("failedread", &[0xff, b'a'], old);
        let (stamp, text) = FileStamp::read(&p);
        assert_eq!(text, None);
        let mut stamp = stamp.unwrap();
        assert!(matches!(stamp, FileStamp::Recheck { meta: Some(_), why: Why::ReadFailed }), "{stamp:?}");
        assert_eq!(stamp.hash(), None);
        for _ in 0..2 {
            assert!(stamp.changed(&p, SystemTime::now()), "the same metadata does not vouch for a failed read");
        }
        let now = FileStamp::of(&p, true).unwrap();
        assert!(!stamp.same(&now) && !now.same(&stamp));
        // what a stamp taken again keeps of it is when its time was seen
        let (again, _) = FileStamp::read(&p);
        assert_eq!(again.unwrap().after(Some(&stamp)).meta().unwrap().seen, stamp.meta().unwrap().seen);
        // a file listed that cannot be looked at: no metadata at all
        let mut unreachable = FileStamp::Recheck { meta: None, why: Why::Unreachable };
        assert!(unreachable.changed(&p, SystemTime::now()));
        assert_eq!(now.after(Some(&unreachable)).meta().unwrap().seen, now.meta().unwrap().seen);
        // readable, with the same time: the stamp vouches for that text
        std::fs::write(&p, b"ba").unwrap();
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(old).unwrap();
        let (stamp, text) = FileStamp::read(&p);
        assert_eq!(text.as_deref(), Some("ba"));
        let mut stamp = stamp.unwrap();
        // (racy: the write moved the change time)
        assert!(stamp.vouches() && stamp.hash().is_some(), "{stamp:?}");
        assert!(!stamp.changed(&p, SystemTime::now()));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    /// A recent time is not trusted alone: the stamp keeps the hash its
    /// content is compared with, and is settled once the time has been seen
    /// unchanged for the margin.
    #[test]
    fn a_racy_stamp_compares_content_until_its_time_is_seen_for_the_margin() {
        // (a time in the future is recent however slowly the test runs)
        let future = SystemTime::now() + Duration::from_secs(3600);
        let p = file("racy", b"one", future);
        let mut stamp = FileStamp::of(&p, false).unwrap();
        let FileStamp::Racy { meta, hash } = stamp else { panic!("{stamp:?}") };
        assert!(!stamp.changed(&p, SystemTime::now()), "the same content");
        // the same time and size, other content
        std::fs::write(&p, b"two").unwrap();
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(future).unwrap();
        assert!(stamp.changed(&p, SystemTime::now()));
        // seen at a look the margin ago: no write since can have kept the time
        let Some(long_ago) = Instant::now().checked_sub(MARGIN * 2) else { return };
        let earlier = FileStamp::Racy { meta: Meta { seen: long_ago, ..meta }, hash };
        let settled = FileStamp::Racy { meta, hash }.after(Some(&earlier));
        assert!(matches!(settled, FileStamp::Settled { hash: Some(h), .. } if h == hash), "{settled:?}");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
