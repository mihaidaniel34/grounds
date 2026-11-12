//! The whole-program declaration index: classpath classes (lazy, from jars)
//! layered under source classes (from stubs, resolved lazily).

use crate::binary;
use crate::hierarchy::{ClassLookup, Tri};
use crate::model::*;
use crate::stubs::*;
use crate::stubs::fold_binary;

thread_local! {
    /// (class, simple name) member-type lookups in progress on this thread
    static MEMBER_LOOKUPS: std::cell::RefCell<Vec<(Name, Name)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// (class, field) constant initializers being evaluated on this thread
    static CONSTANTS_IN_PROGRESS: std::cell::RefCell<Vec<(Name, Name)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Longest qualified type name resolved, in segments. Resolution tries every
/// split into package and nested classes, which is quadratic in the length:
/// a file cut off in `@a.x.x.x...` (a million segments) never finished. No
/// name in the corpus has 21.
pub(crate) const MAX_NAME_SEGMENTS: usize = 256;

use dashmap::DashMap;
use grounds_classfile::ClassContainer;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// How much of the JDK's API the classpath holds for the project's release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JdkApi {
    /// The JDK's signatures for the release (or the JDK's own release, which
    /// `open_ct_sym` cannot read exactly).
    Complete,
    /// The JDK home has no `lib/ct.sym` that can be read: a JRE, or a JDK older than 9.
    Absent { home: std::path::PathBuf },
    /// The project compiles for a release newer than the installed JDK, whose
    /// API `open_ct_sym` can only approximate with the newest older release.
    /// Members added since then may be missing from JDK classes.
    Newer { requested: u32, installed: u32 },
    /// The project compiles for the JDK's own release, but its runtime image
    /// (`lib/modules`) cannot be read (a compressed image, a parse error), so
    /// its API is approximated from `ct.sym` like [`JdkApi::Newer`].
    Unreadable { release: u32 },
}

/// Another release's JDK API, from ct.sym: its classes and packages.
struct OtherRelease {
    container: ClassContainer,
    classes: FxHashSet<Name>,
    packages: FxHashSet<Name>,
}

/// Immutable classpath: JDK symbols plus dependency jars and class dirs.
pub struct Classpath {
    containers: Vec<ClassContainer>,
    names: FxHashMap<Name, u32>,
    packages: FxHashSet<Name>,
    cache: DashMap<Name, Option<Arc<ClassInfo>>>,
    pub jdk: JdkApi,
    /// The release the JDK's classes (the first container) were read for.
    jdk_release: Option<u32>,
    /// Whether the JDK home has `lib/ct.sym`, which only a full JDK has: a
    /// runtime image cut down by jlink lacks modules, and their packages.
    jdk_full: bool,
    /// Every package of the JDK: those of the API loaded, and those of the
    /// installed JDK's runtime image, exported or not.
    jdk_packages: FxHashSet<Name>,
    /// The packages of the JDK API loaded (exported ones), and of the jars.
    api_packages: FxHashSet<Name>,
    jar_packages: FxHashSet<Name>,
    jdk_home: Option<std::path::PathBuf>,
    /// The classes and packages of other releases' APIs, read from ct.sym when asked.
    other_releases: DashMap<u32, Option<Arc<OtherRelease>>>,
    /// Directory entries too large to be directories of classes
    /// (`grounds_classfile::DIR_LIMIT`); nothing was read from them.
    pub oversized: Vec<std::path::PathBuf>,
}

impl std::fmt::Debug for Classpath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Classpath({} containers, {} classes)", self.containers.len(), self.names.len())
    }
}

impl Classpath {
    pub fn empty() -> Classpath {
        Classpath { containers: Vec::new(), names: FxHashMap::default(), packages: FxHashSet::default(), cache: DashMap::new(), jdk: JdkApi::Complete, jdk_release: None, jdk_full: false, jdk_packages: FxHashSet::default(), api_packages: FxHashSet::default(), jar_packages: FxHashSet::default(), jdk_home: None, other_releases: DashMap::new(), oversized: Vec::new() }
    }

    /// Build from containers in priority order (first definition wins).
    pub fn from_containers(containers: Vec<ClassContainer>) -> Classpath {
        Self::from_containers_within(containers, grounds_classfile::DIR_LIMIT)
    }

    fn from_containers_within(containers: Vec<ClassContainer>, dir_limit: usize) -> Classpath {
        let lists: Vec<Option<Vec<String>>> = {
            use rayon::prelude::*;
            containers.par_iter().map(|c| c.class_names_within(dir_limit)).collect()
        };
        let mut names = FxHashMap::default();
        let mut packages = FxHashSet::default();
        let mut oversized: Vec<(u32, std::path::PathBuf)> = Vec::new();
        for (i, list) in lists.into_iter().enumerate() {
            let Some(list) = list else {
                if let ClassContainer::Dir(d) = &containers[i] {
                    oversized.push((i as u32, d.clone()));
                }
                continue;
            };
            for n in list {
                let b = binary::internal_to_binary(&n);
                if let Some(pos) = b.rfind('.') {
                    let pkg = &b[..pos];
                    if !packages.contains(pkg) {
                        // register all parent packages too
                        let mut p = pkg;
                        loop {
                            packages.insert(Arc::from(p));
                            match p.rfind('.') {
                                Some(j) => p = &p[..j],
                                None => break,
                            }
                        }
                    }
                }
                // a directory that was not listed still comes first for a
                // class it holds: a later entry's copy is not the one in use
                let from = oversized.iter().find(|(_, d)| d.join(format!("{n}.class")).is_file()).map_or(i as u32, |(j, _)| *j);
                names.entry(Arc::from(b.as_str())).or_insert(from);
            }
        }
        let oversized = oversized.into_iter().map(|(_, d)| d).collect();
        Classpath { containers, names, packages, cache: DashMap::new(), jdk: JdkApi::Complete, jdk_release: None, jdk_full: false, jdk_packages: FxHashSet::default(), api_packages: FxHashSet::default(), jar_packages: FxHashSet::default(), jdk_home: None, other_releases: DashMap::new(), oversized }
    }

    /// JDK classes for a release (see `ClassContainer::open_jdk`), followed by the given jars/dirs.
    pub fn build(java_home: Option<&Path>, release: u32, entries: &[std::path::PathBuf]) -> Classpath {
        let mut containers = Vec::new();
        let mut jdk = JdkApi::Complete;
        let mut jdk_release = None;
        let mut jdk_full = false;
        let mut jdk_packages: FxHashSet<Name> = FxHashSet::default();
        if let Some(home) = java_home {
            // a release newer than the JDK: the JDK's own API is the closest at hand
            let read_as = grounds_classfile::jdk_feature_release(home).filter(|&i| release > i).unwrap_or(release);
            let opened = ClassContainer::open_jdk(home, read_as);
            // the image lists every package, the internal ones too
            let mut image_packages = |j: &grounds_classfile::jimage::JImage| jdk_packages.extend(j.all_packages().map(|p| Arc::from(p.replace('/', ".").as_str())));
            match &opened {
                Ok(ClassContainer::JImage(j)) => image_packages(j),
                _ => {
                    if let Ok(j) = grounds_classfile::jimage::JImage::open(&home.join("lib/modules")) {
                        image_packages(&j);
                    }
                }
            }
            if let Ok(c) = opened {
                jdk_release = Some(release);
                jdk_full = home.join("lib/ct.sym").is_file();
                // the JDK's own release is read exactly from its image; a newer one has no signatures anywhere
                let unreadable = grounds_classfile::jdk_is_own_release(home, release) && !matches!(c, ClassContainer::JImage(_));
                containers.push(c);
                if unreadable {
                    jdk = JdkApi::Unreadable { release };
                } else if let Some(installed) = grounds_classfile::jdk_feature_release(home).filter(|&v| release > v) {
                    jdk = JdkApi::Newer { requested: release, installed };
                }
            } else {
                jdk = JdkApi::Absent { home: home.to_path_buf() };
            }
        }
        for e in entries {
            if e.is_dir() {
                containers.push(ClassContainer::open_dir(e));
            } else if let Ok(c) = ClassContainer::open_jar(e) {
                containers.push(c);
            }
        }
        let mut cp = Classpath::from_containers(containers);
        let with_parents = |set: &mut FxHashSet<Name>, n: &str| {
            let mut p = n;
            while let Some(i) = p.rfind('.') {
                p = &p[..i];
                if !set.insert(Arc::from(p)) {
                    break;
                }
            }
        };
        let (mut api_packages, mut jar_packages) = (FxHashSet::default(), FxHashSet::default());
        for (n, &i) in &cp.names {
            if jdk_release.is_some() && i == 0 {
                with_parents(&mut api_packages, n);
            } else {
                with_parents(&mut jar_packages, n);
            }
        }
        // the API's packages: a JDK older than 9 has no image
        jdk_packages.extend(api_packages.iter().cloned());
        cp.api_packages = api_packages;
        cp.jar_packages = jar_packages;
        cp.jdk_home = java_home.map(Path::to_path_buf);
        cp.jdk = jdk;
        cp.jdk_release = jdk_release;
        cp.jdk_full = jdk_full;
        cp.jdk_packages = jdk_packages;
        cp
    }

    /// Whether the JDK's API (an exported package of the release loaded)
    /// has the class. javac does not compile a source class in such a
    /// package together with the code ("package exists in another module"),
    /// so the code compiles against the JDK's.
    pub fn is_jdk_class(&self, name: &str) -> bool {
        self.jdk_release.is_some() && self.names.get(name) == Some(&0)
    }

    /// Whether the JDK's API (the release loaded) has the package: javac
    /// refuses a source file in it when compiling the code.
    pub fn is_jdk_api_package(&self, package: &str) -> bool {
        self.api_packages.contains(package)
    }

    /// Whether a package belongs to the JDK, exported or not.
    pub fn is_jdk_package(&self, package: &str) -> bool {
        self.jdk_packages.contains(package)
    }

    /// Whether a jar or class directory (not the JDK) has the class.
    pub fn contains_outside_jdk(&self, name: &str) -> bool {
        self.names.get(name).is_some_and(|&i| self.jdk_release.is_none() || i != 0)
    }

    /// Whether a jar or class directory (not the JDK) has the package.
    pub fn has_package_outside_jdk(&self, package: &str) -> bool {
        self.jar_packages.contains(package)
    }

    /// Whether the JDK API of `release` has the public class. The loaded
    /// release when its API is exact, another from the JDK's ct.sym (8 up to
    /// the JDK's own); None when that API cannot be read.
    pub fn jdk_has_class(&self, release: u32, name: &str) -> Option<bool> {
        if self.exact_jdk_release() == Some(release) {
            return Some(self.is_jdk_class(name) && self.contains(name));
        }
        Some(self.other_release(release)?.classes.contains(name))
    }

    /// Whether the JDK API of `release` has the package (see `jdk_has_class`).
    pub fn jdk_has_package(&self, release: u32, package: &str) -> Option<bool> {
        if self.exact_jdk_release() == Some(release) {
            return Some(self.api_packages.contains(package));
        }
        Some(self.other_release(release)?.packages.contains(package))
    }

    /// Whether the JDK class has, in the API of `release`, a method of that
    /// name whose parameters erase to `params` (see `jdk_has_class`).
    /// None too where an unknown parameter type leaves it open.
    pub fn jdk_has_method(&self, release: u32, class: &str, name: &str, params: &[Ty]) -> Option<bool> {
        // a method whose parameters are surely `params`, or may be
        let has = |c: &ClassInfo| -> Tri {
            c.methods
                .iter()
                .filter(|m| &*m.name == name && m.params.len() == params.len())
                .map(|m| m.params.iter().zip(params).fold(Tri::Yes, |acc, (a, b)| acc.and(a.ty.erasure().same(&b.erasure()))))
                .fold(Tri::No, Tri::or)
        };
        let known = |t: Tri| match t {
            Tri::Yes => Some(true),
            Tri::No => Some(false),
            Tri::Unknown => None,
        };
        if self.exact_jdk_release() == Some(release) {
            if !self.is_jdk_class(class) {
                return Some(false);
            }
            return match self.class(class) {
                Some(c) => known(has(&c)),
                None => Some(false),
            };
        }
        let other = self.other_release(release)?;
        let cf = other.container.read_class(&class.replace('.', "/"));
        match cf {
            Some(cf) => known(has(&binary::lower_class(&cf, 0))),
            None => Some(false),
        }
    }

    fn other_release(&self, release: u32) -> Option<Arc<OtherRelease>> {
        if let Some(r) = self.other_releases.get(&release) {
            return r.clone();
        }
        let api = (|| {
            let home = self.jdk_home.as_deref()?;
            // ct.sym holds signatures for releases older than the JDK's own
            let installed = grounds_classfile::jdk_feature_release(home)?;
            if release < 8 || release >= installed {
                return None;
            }
            let ct = ClassContainer::open_ct_sym(&home.join("lib/ct.sym"), release).ok()?;
            let mut classes = FxHashSet::default();
            let mut packages = FxHashSet::default();
            for n in ct.class_names() {
                let b = binary::internal_to_binary(&n);
                let mut p = b.as_str();
                while let Some(i) = p.rfind('.') {
                    p = &p[..i];
                    if !packages.insert(Arc::from(p)) {
                        break;
                    }
                }
                classes.insert(Arc::from(b.as_str()));
            }
            Some(Arc::new(OtherRelease { container: ct, classes, packages }))
        })();
        self.other_releases.insert(release, api.clone());
        api
    }

    /// Whether `name` is a JDK class whose API may be newer than the one
    /// loaded (see [`JdkApi::Newer`] and [`JdkApi::Unreadable`]). The JDK is the first container then.
    pub fn may_lack_members(&self, name: &str) -> bool {
        matches!(self.jdk, JdkApi::Newer { .. } | JdkApi::Unreadable { .. }) && self.names.get(name) == Some(&0)
    }

    /// The release whose JDK API the classpath holds exactly: every public
    /// JDK class of that release, and no other. None without a JDK, or when
    /// the API is approximated ([`JdkApi::Newer`], [`JdkApi::Unreadable`]).
    pub fn exact_jdk_release(&self) -> Option<u32> {
        self.jdk_release.filter(|_| self.jdk == JdkApi::Complete)
    }

    /// Whether the JDK holds all of its modules (see `jdk_full`), so a JDK
    /// package missing from it is missing from the release.
    pub fn jdk_is_full(&self) -> bool {
        self.jdk_full
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
    pub fn containers(&self) -> &[ClassContainer] {
        &self.containers
    }
    /// Whether a class exists that a name may resolve to. The JDK's runtime
    /// image and ct.sym also hold classes that are not public, which code
    /// outside the JDK cannot name: `class` has them, as the supertypes of
    /// public ones (`AbstractStringBuilder` of `StringBuilder`) that
    /// hierarchy walks and member lookup need, but they do not exist for
    /// names. A nested class declared protected is public in its class file,
    /// and accessible.
    pub fn contains(&self, name: &str) -> bool {
        match self.names.get(name) {
            Some(&idx) if matches!(self.containers[idx as usize], ClassContainer::JImage(_) | ClassContainer::CtSym { .. }) => self.class(name).is_some_and(|c| c.flags.is_public() || c.flags.has(Flags::PROTECTED)),
            Some(_) => true,
            None => false,
        }
    }
    pub fn has_package(&self, p: &str) -> bool {
        self.packages.contains(p)
    }
    pub fn class_names(&self) -> impl Iterator<Item = &Name> {
        self.names.keys().filter(|n| self.contains(n))
    }

    pub fn class(&self, name: &str) -> Option<Arc<ClassInfo>> {
        if let Some(c) = self.cache.get(name) {
            return c.clone();
        }
        // a class is lowered once for every thread and file: what lowering
        // observes is not any one rule's (see `doubt`)
        let r = crate::doubt::quiet(|| {
            let idx = *self.names.get(name)?;
            let internal = name.replace('.', "/");
            let cf = self.containers[idx as usize].read_class(&internal)?;
            Some(Arc::new(binary::lower_class(&cf, idx)))
        });
        self.cache.insert(Arc::from(name), r.clone());
        r
    }

    /// Read a resource from all containers (e.g. META-INF/spring.factories).
    pub fn resources(&self, name: &str) -> Vec<Vec<u8>> {
        self.containers.iter().filter_map(|c| c.read_resource(name)).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceClassRef {
    pub idx: u32,
}

/// Name lookup context for resolving type syntax.
#[derive(Clone, Debug, Default)]
pub struct Scope<'a> {
    pub package: &'a str,
    pub imports: &'a Imports,
    /// Enclosing classes, innermost first.
    pub enclosing: Vec<Name>,
    /// Type variables in scope.
    pub type_vars: Vec<Name>,
    /// Local classes visible here: simple name, binary name, and the number
    /// of `enclosing` classes declared inside the block that declares it (the
    /// local class shadows the member types of `enclosing[level]` and outer
    /// ones, and is shadowed by those of the classes nested in its block).
    pub local_types: Vec<(Name, Name, usize)>,
    /// The local and anonymous classes among `enclosing`, which the index
    /// does not hold.
    pub local_classes: Vec<LocalClassScope>,
    /// A name that a supertype which is not resolved may declare as a member
    /// type is unknown, unless an enclosing class, a single-type import or
    /// java.lang provides it. Set for names in code; declarations lowered for
    /// the whole project keep the package's or an on-demand import's class
    /// (the wiring model counts on their types).
    pub unknown_if_inherited: bool,
}

/// What a local or anonymous class brings into scope in its body.
#[derive(Clone, Debug, Default)]
pub struct LocalClassScope {
    pub name: Name,
    /// Member types declared in it (and in local supertypes): simple name ->
    /// binary name.
    pub members: Vec<(Name, Name)>,
    /// Supertypes the index holds, whose member types it inherits.
    pub supers: Vec<Name>,
    /// A supertype is not resolved: any name may be an inherited member type.
    pub unknown_super: bool,
}

/// Whether the build compiles a source file, as far as grounds has evidence
/// of it (its module's build output, or the build tool's list of source
/// directories). Claims that code does not compile are made only in a file
/// that is not `No`; one made under `Unknown` rests on a guessed picture of
/// the build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InBuild {
    /// In the build: there is evidence that the build compiles the file's
    /// source root.
    Yes,
    /// Not in the build: the module has evidence of what it compiles, and
    /// this file is not part of it (test data, samples, a stale module).
    No,
    /// No evidence: the module was never built, nor its source directories
    /// dumped.
    Unknown,
}

impl InBuild {
    /// How output formats name it.
    pub fn as_str(self) -> &'static str {
        match self {
            InBuild::Yes => "in the build",
            InBuild::No => "not in the build",
            InBuild::Unknown => "unknown",
        }
    }
}

/// The result of a member type lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemberType {
    Found(Name),
    NotFound,
    /// Not found, but a supertype is not resolved and may declare it.
    Unknown,
}

pub struct Index {
    pub classpath: Arc<Classpath>,
    files: FxHashMap<Arc<str>, Arc<FileStub>>,
    classes: FxHashMap<Name, (Arc<str>, u32)>,
    packages: FxHashMap<Name, u32>,
    cache: DashMap<Name, Option<Arc<ClassInfo>>>,
    /// Number of source files defining each class name (>1 = duplicated
    /// across modules, e.g. test fixtures).
    defs: FxHashMap<Name, u32>,
    /// The files declaring each duplicated class name (`defs` > 1).
    copies: FxHashMap<Name, Vec<Arc<str>>>,
    /// Directories (ending in `/`) of modules whose build states the release
    /// they compile for exactly, with that release.
    exact_releases: Vec<(String, Option<u32>)>,
    release_floors: Vec<(String, Option<u32>)>,
    /// Number of files in a variant source set (see `in_variant_set`)
    /// declaring each class name.
    variant_defs: FxHashMap<Name, u32>,
    /// Directories (ending in `/`) of modules that compile against a
    /// published release of the project itself, not its sources.
    release_modules: Vec<String>,
    /// Whether the build compiles each source file, as far as there is
    /// evidence (see `in_build`); a file not here is `InBuild::Unknown`.
    built: FxHashMap<Arc<str>, InBuild>,
    pub generation: u64,
    // the generation resolve_ordered last filled the cache at
    resolved_at: AtomicU64,
    // the groups resolve_ordered last used; None: the next fill resolves everything
    plan: parking_lot::Mutex<Option<Plan>>,
    // see `language_extended`: the generation it was computed at, the module
    // directories, and whether the whole project is affected
    extended: parking_lot::Mutex<Option<(u64, Arc<(FxHashSet<String>, bool)>)>>,
}

// file groups in dependency order, and the files edited since without a declaration change
struct Plan {
    groups: Vec<Vec<Arc<str>>>,
    group_of: FxHashMap<Arc<str>, usize>,
    dirty: Vec<Arc<str>>,
}

thread_local! {
    static RESOLVING: RefCell<FxHashSet<Name>> = RefCell::new(FxHashSet::default());
    // while resolve_ordered works on a group on this thread
    static GROUP: RefCell<Option<FxHashSet<Name>>> = const { RefCell::new(None) };
    // source classes this thread had to resolve outside its group due to missing graph edges
    static OUTSIDE: Cell<usize> = const { Cell::new(0) };
}

// marks this thread as working on one group until dropped, also when resolution panics
struct InGroup;

impl InGroup {
    fn enter(classes: FxHashSet<Name>) -> InGroup {
        GROUP.with(|g| *g.borrow_mut() = Some(classes));
        OUTSIDE.with(|o| o.set(0));
        InGroup
    }
}

impl Drop for InGroup {
    fn drop(&mut self) {
        GROUP.with(|g| *g.borrow_mut() = None);
    }
}

/// Evaluate the initializer of `owner.field` with `f`, or `None` if it is
/// already being evaluated further up on this thread: initializers that refer
/// to each other (`A.X = B.Y; B.Y = A.X + 1;` compiles, and neither is a
/// constant) would otherwise recurse until the stack overflows.
fn evaluating_constant<T>(owner: &str, field: &Name, f: impl FnOnce() -> Option<T>) -> Option<T> {
    let key: (Name, Name) = (Arc::from(owner), field.clone());
    if CONSTANTS_IN_PROGRESS.with(|c| c.borrow().contains(&key)) {
        return None;
    }
    CONSTANTS_IN_PROGRESS.with(|c| c.borrow_mut().push(key));
    // leave also when `f` panics (the rule runner catches panics)
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            CONSTANTS_IN_PROGRESS.with(|c| c.borrow_mut().pop());
        }
    }
    let _leave = Leave;
    f()
}

/// Marks a class as being resolved on this thread for as long as it lives,
/// also when resolution panics (rule and daemon panics are caught and the
/// thread is reused). Borrows the name: nothing to clone.
struct Resolving<'a>(&'a str);

impl<'a> Resolving<'a> {
    /// `None` if the class is already being resolved further up (a cycle).
    fn enter(name: &'a str) -> Option<Resolving<'a>> {
        // one lookup: entering is the common case, a cycle rare
        RESOLVING.with(|r| r.borrow_mut().insert(Arc::from(name))).then(|| Resolving(name))
    }
}

impl Drop for Resolving<'_> {
    fn drop(&mut self) {
        RESOLVING.with(|r| {
            r.borrow_mut().remove(self.0);
        });
    }
}

impl Index {
    pub fn new(classpath: Arc<Classpath>) -> Index {
        Index { classpath, files: FxHashMap::default(), classes: FxHashMap::default(), packages: FxHashMap::default(), cache: DashMap::new(), defs: FxHashMap::default(), copies: FxHashMap::default(), variant_defs: FxHashMap::default(), exact_releases: Vec::new(), release_floors: Vec::new(), release_modules: Vec::new(), built: FxHashMap::default(), generation: 0, resolved_at: AtomicU64::new(u64::MAX), plan: parking_lot::Mutex::new(None), extended: parking_lot::Mutex::new(None) }
    }

    pub fn set_file(&mut self, path: Arc<str>, stub: Arc<FileStub>) {
        let body_only = self.files.get(&path).is_some_and(|old| old.signature_hash == stub.signature_hash);
        self.remove_file_inner(&path);
        for (i, c) in stub.classes.iter().enumerate() {
            // a stand-in for a class of the JDK's API is not the class
            if self.is_jdk_standin(&c.name) {
                continue;
            }
            if let Some((other, _)) = self.classes.get(&c.name) {
                let files = self.copies.entry(c.name.clone()).or_default();
                if files.is_empty() {
                    files.push(other.clone());
                }
                if !files.contains(&path) {
                    files.push(path.clone());
                    files.sort();
                }
            }
            // a class defined in several files resolves to the smallest path,
            // whatever order files were loaded, refreshed or swapped in
            match self.classes.get(&c.name) {
                Some((p, _)) if **p <= *path => {}
                _ => {
                    self.classes.insert(c.name.clone(), (path.clone(), i as u32));
                }
            }
            *self.defs.entry(c.name.clone()).or_insert(0) += 1;
            if in_variant_set(&path) {
                *self.variant_defs.entry(c.name.clone()).or_insert(0) += 1;
            }
        }
        let mut p: &str = &stub.package;
        loop {
            *self.packages.entry(Arc::from(p)).or_insert(0) += 1;
            match p.rfind('.') {
                Some(j) => p = &p[..j],
                None => break,
            }
        }
        let plan = self.plan.get_mut();
        match plan {
            // same declarations: only the file's group resolves differently (positions)
            Some(p) if body_only && p.group_of.contains_key(&path) => {
                for c in &stub.classes {
                    if self.classes.get(&c.name).is_some_and(|(owner, _)| *owner == path) {
                        self.cache.remove(&c.name);
                    }
                }
                p.dirty.push(path.clone());
            }
            _ => {
                *plan = None;
                self.cache.clear();
            }
        }
        self.files.insert(path, stub);
        self.generation += 1;
    }

    pub fn remove_file(&mut self, path: &str) {
        self.remove_file_inner(path);
        *self.plan.get_mut() = None;
        self.cache.clear();
        self.generation += 1;
    }

    fn remove_file_inner(&mut self, path: &str) {
        if let Some(old) = self.files.remove(path) {
            for c in &old.classes {
                if in_variant_set(path) {
                    if let Some(v) = self.variant_defs.get_mut(&c.name) {
                        *v -= 1;
                        if *v == 0 {
                            self.variant_defs.remove(&c.name);
                        }
                    }
                }
                if let Some(d) = self.defs.get_mut(&c.name) {
                    *d -= 1;
                    let left = *d;
                    if left == 0 {
                        self.defs.remove(&c.name);
                    }
                    if left <= 1 {
                        self.copies.remove(&c.name);
                    } else if let Some(files) = self.copies.get_mut(&c.name) {
                        files.retain(|f| &**f != path);
                    }
                }
                if self.classes.get(&c.name).is_some_and(|(p, _)| &**p == path) {
                    self.classes.remove(&c.name);
                    // another file may define the same class name: the smallest path
                    let next = self.files.iter().filter_map(|(fp, st)| st.classes.iter().position(|x| x.name == c.name).map(|i| (fp, i))).min_by(|a, b| a.0.cmp(b.0));
                    if let Some((fp, i)) = next {
                        self.classes.insert(c.name.clone(), (fp.clone(), i as u32));
                    }
                }
            }
            let mut p: &str = &old.package;
            loop {
                if let Some(cnt) = self.packages.get_mut(p) {
                    *cnt -= 1;
                    if *cnt == 0 {
                        self.packages.remove(p);
                    }
                }
                match p.rfind('.') {
                    Some(j) => p = &p[..j],
                    None => break,
                }
            }
        }
    }

    /// Every module with the release it compiles for exactly, None when
    /// that is not known: (directory, release).
    pub fn set_exact_releases(&mut self, modules: Vec<(String, Option<u32>)>) {
        self.exact_releases = innermost_first(modules);
    }

    /// The release a file's module states exactly (`--release`, a
    /// toolchain), the code not contradicting it. None when its build leaves
    /// the JDK API to the JDK running it, or grounds cannot tell; the module
    /// nearest the file decides, not an enclosing one.
    pub fn exact_release(&self, file: &str) -> Option<u32> {
        self.exact_releases.iter().find(|(d, _)| file.starts_with(d.as_str()))?.1
    }

    /// The lowest release a file's module may compile for: its exact
    /// release, or the `source`/`target` it states. A JDK member newer than
    /// that may not exist where the code is built, though the API loaded has it.
    pub fn release_floor(&self, file: &str) -> Option<u32> {
        self.release_floors.iter().find(|(d, _)| file.starts_with(d.as_str()))?.1
    }

    /// Every module's floor (see `release_floor`): (directory, release).
    pub fn set_release_floors(&mut self, modules: Vec<(String, Option<u32>)>) {
        self.release_floors = innermost_first(modules);
    }

    /// Whether some source file is in the package (or one below it).
    pub fn has_source_package(&self, package: &str) -> bool {
        self.packages.contains_key(package)
    }

    /// Mark the modules whose build compiles against a published release of
    /// the project itself (kafka's `streams/upgrade-system-tests-22` against
    /// `kafka-streams:2.2`): their code does not see the project's sources.
    pub fn set_modules_built_against_release(&mut self, dirs: Vec<String>) {
        let sep = std::path::MAIN_SEPARATOR;
        self.release_modules = dirs.into_iter().map(|d| if d.ends_with(['/', sep]) { d } else { format!("{d}{sep}") }).collect();
        // (innermost first: a module nested in another is its own)
        self.release_modules.sort_by_key(|d| std::cmp::Reverse(d.len()));
    }

    /// For a file of a module that compiles against a published release of
    /// the project, the module's directory: a project class declared outside
    /// it may differ from the one the file compiles against.
    pub fn built_against_release(&self, file: &str) -> Option<&str> {
        self.release_modules.iter().find(|d| file.starts_with(d.as_str())).map(String::as_str)
    }

    /// Whether the build compiles the source file at `path`: what its
    /// module's build output (or the build tool's own list of source
    /// directories) shows, `Unknown` where there is no evidence. Filled by
    /// the engine (`grounds_engine::picture`); a lookup, nothing more.
    pub fn in_build(&self, path: &str) -> InBuild {
        self.built.get(path).copied().unwrap_or(InBuild::Unknown)
    }

    /// Set whether the build compiles the file at `path` (`Unknown`: no
    /// evidence either way).
    pub fn set_in_build(&mut self, path: Arc<str>, state: InBuild) {
        match state {
            InBuild::Unknown => {
                self.built.remove(&path);
            }
            state => {
                self.built.insert(path, state);
            }
        }
    }

    /// Replace what is known of every file (see `set_in_build`).
    pub fn set_all_in_build(&mut self, states: FxHashMap<Arc<str>, InBuild>) {
        self.built = states;
        self.built.retain(|_, s| *s != InBuild::Unknown);
    }

    /// The files declaring `name`, when it is declared in more than one.
    pub fn copies_of(&self, name: &str) -> &[Arc<str>] {
        self.copies.get(name).map_or(&[], |v| v.as_slice())
    }

    /// Whether `file` belongs to the module at `dir` itself, not to a module
    /// nested below it (module directories come from `set_exact_releases`).
    pub fn in_module(&self, file: &str, dir: &str) -> bool {
        file.starts_with(dir) && self.exact_releases.iter().find(|(d, _)| file.starts_with(d.as_str())).is_none_or(|(d, _)| d.len() <= dir.len())
    }

    pub fn file(&self, path: &str) -> Option<&Arc<FileStub>> {
        self.files.get(path)
    }

    pub fn files(&self) -> impl Iterator<Item = (&Arc<str>, &Arc<FileStub>)> {
        self.files.iter()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn source_class_names(&self) -> impl Iterator<Item = &Name> {
        self.classes.keys()
    }

    /// Is this class name defined in more than one source file? A source
    /// class in a package of the JDK counts as declared twice: the JDK has
    /// the package too (`com.sun.tools.javac.util`, not exported, which javac
    /// compiles from sources; a stub of it that a build replaces with the
    /// JDK's), so which declaration the code compiles against is a guess.
    pub fn is_duplicated(&self, name: &str) -> bool {
        self.defs.get(name).is_some_and(|d| *d > 1) || (self.classes.contains_key(name) && name.rfind('.').is_some_and(|i| self.classpath.is_jdk_package(&name[..i])))
    }

    /// A source class in a package of the JDK (exported or not): code that
    /// javac compiles only in special builds (GWT emulation, `--patch-module`,
    /// a compile-only stub).
    pub fn in_jdk_package(&self, name: &str) -> bool {
        self.classes.contains_key(name) && name.rfind('.').is_some_and(|i| self.classpath.is_jdk_package(&name[..i]))
    }

    /// A source class standing in for a class of the JDK's API, or nested in
    /// one: it is not indexed, so every file sees the JDK's. javac refuses a
    /// source class in an exported JDK package ("package exists in another
    /// module"): no build compiles it with the code that uses the class (a
    /// test input, a compile-only stub, a GWT emulation of `java.lang`).
    fn is_jdk_standin(&self, name: &str) -> bool {
        let top = match name.rfind('.') {
            Some(i) => name[i..].find('$').map_or(name, |j| &name[..i + j]),
            None => name.split('$').next().unwrap_or(name),
        };
        self.classpath.is_jdk_class(top)
    }

    /// The copy of a class declared in several files that the file at
    /// `from` compiles against, when that is certain without knowing the
    /// build's module dependencies: the copy in the file's own module and
    /// source set, else the only copy in its module when that one is main
    /// code and the file a test (tests see their module's main classes).
    /// Only files under a source root of a module count (see `source_root`):
    /// elsewhere (`guava/src/com/...`, a GWT `src-super` copy, test data
    /// under `src/test/test_sourcecode`) neither the module nor the source
    /// set is known. None for a class declared once, or when the choice
    /// would be a guess.
    pub fn certain_copy(&self, name: &str, from: &str) -> Option<(&Arc<FileStub>, &RawClass, &Arc<str>)> {
        use crate::sources::is_test_set;
        let files = self.copies.get(name)?;
        let (module, set) = source_root(from)?;
        let mine: Vec<(&Arc<str>, &str)> = files.iter().filter_map(|f| source_root(f).filter(|r| r.0 == module).map(|r| (f, r.1))).collect();
        let same_set: Vec<&Arc<str>> = mine.iter().filter(|f| f.1 == set).map(|f| f.0).collect();
        let pick = match (same_set.as_slice(), mine.as_slice()) {
            ([one], _) => *one,
            ([], [(one, "main")]) if is_test_set(set) => *one,
            _ => return None,
        };
        let stub = self.files.get(&**pick)?;
        let raw = stub.classes.iter().find(|c| &*c.name == name)?;
        Some((stub, raw, pick))
    }

    /// Whether the file at `path` is compiled with a javac plugin that
    /// changes the language, so members, access and types may be other than
    /// the Java the model reads. Manifold (`-Xplugin:Manifold`) adds extension
    /// methods, `@Jailbreak` access to private members, properties, `auto`
    /// types, structural interfaces and delegation; code using them is valid
    /// only with the plugin. Its evidence in the sources: a file of the module
    /// imports a package of its runtime API (`manifold.*.rt.api`). Extension
    /// classes (`manifold.ext.rt.api.Extension`) add methods to classes used
    /// in any module, so one of those makes the whole project extended.
    pub fn language_extended(&self, path: &str) -> bool {
        let mut cached = self.extended.lock();
        let sets = match &*cached {
            Some((g, sets)) if *g == self.generation => sets.clone(),
            _ => {
                let mut modules = FxHashSet::default();
                let mut everywhere = false;
                for (p, stub) in &self.files {
                    for imp in stub.imports.iter() {
                        let api = imp.path.strip_prefix("manifold.").is_some_and(|rest| rest.starts_with("rt.api") || rest.contains(".rt.api"));
                        if api {
                            modules.insert(module_dir(p).to_string());
                            everywhere |= imp.path.starts_with("manifold.ext.rt.api") && (&*imp.path == "manifold.ext.rt.api.Extension" || imp.on_demand);
                        }
                    }
                }
                let sets = Arc::new((modules, everywhere));
                *cached = Some((self.generation, sets.clone()));
                sets
            }
        };
        sets.1 || sets.0.contains(module_dir(path))
    }

    pub fn is_source_class(&self, name: &str) -> bool {
        self.classes.contains_key(name)
    }

    pub fn source_file_of(&self, class: &str) -> Option<&Arc<str>> {
        self.classes.get(class).map(|(p, _)| p)
    }

    pub fn raw_class(&self, name: &str) -> Option<(&Arc<FileStub>, &RawClass)> {
        let (path, idx) = self.classes.get(name)?;
        let stub = self.files.get(path)?;
        Some((stub, &stub.classes[*idx as usize]))
    }

    pub fn class_exists(&self, name: &str) -> bool {
        self.classes.contains_key(name) || self.classpath.contains(name)
    }

    pub fn package_exists(&self, p: &str) -> bool {
        self.packages.contains_key(p) || self.classpath.has_package(p)
    }

    pub fn class(&self, name: &str) -> Option<Arc<ClassInfo>> {
        if self.classes.contains_key(name) {
            if let Some(c) = self.cache.get(name) {
                return c.clone();
            }
            let _resolving = Resolving::enter(name)?;
            // resolving a class of another group => the file graph missed an edge
            if GROUP.with(|g| g.borrow().as_ref().is_some_and(|g| !g.contains(name))) {
                OUTSIDE.with(|o| o.set(o.get() + 1));
            }
            // lowered once for every thread and file: what lowering observes
            // is not any one rule's (see `doubt`)
            let r = crate::doubt::quiet(|| self.resolve_source_class(name).map(Arc::new));
            self.cache.insert(Arc::from(name), r.clone());
            return r;
        }
        self.classpath.class(name)
    }

    // files that need each other form a group => 1 thread
    pub fn resolve_ordered(&self) -> crate::order::ResolveStats {
        use rayon::prelude::*;
        self.cache.clear();
        let (files, edges) = crate::order::file_graph(self);
        let groups = crate::order::components(&edges);
        let waves = crate::order::waves(&groups, &edges);
        let mut group_sizes: Vec<usize> = groups.iter().map(|g| g.len()).collect();
        group_sizes.sort_unstable_by(|a, b| b.cmp(a));
        let mut stats = crate::order::ResolveStats { files: files.len(), waves: waves.len(), group_sizes, outside: 0 };
        for wave in waves {
            stats.outside += wave
                .par_iter()
                .map(|&g| self.resolve_group(groups[g].iter().map(|&f| &files[f as usize])))
                .sum::<usize>();
        }
        let groups: Vec<Vec<Arc<str>>> = groups.iter().map(|g| g.iter().map(|&f| files[f as usize].clone()).collect()).collect();
        let group_of = groups.iter().enumerate().flat_map(|(g, fs)| fs.iter().map(move |f| (f.clone(), g))).collect();
        *self.plan.lock() = Some(Plan { groups, group_of, dirty: Vec::new() });
        self.resolved_at.store(self.generation, Ordering::Release);
        stats
    }

    // resolves the group's classes on this thread, in name order
    fn resolve_group<'p>(&self, paths: impl Iterator<Item = &'p Arc<str>>) -> usize {
        // the classes declared in the group's files - a class declared in several files belongs to the file the index uses for it
        let mut names: Vec<Name> = Vec::new();
        for path in paths {
            for c in self.files.get(path).map_or(&[][..], |s| &s.classes) {
                if self.classes.get(&c.name).is_some_and(|(p, _)| p == path) {
                    names.push(c.name.clone());
                }
            }
        }
        names.sort();
        names.dedup();
        let _group = InGroup::enter(names.iter().cloned().collect());
        for n in &names {
            let _ = self.class(n);
        }
        OUTSIDE.with(|o| o.get())
    }

    // brings the cache up to date the way resolve_ordered fills it: after body-only
    // edits only their groups again, in dependency order, otherwise everything
    pub fn ensure_resolved(&self) {
        if self.resolved_at.load(Ordering::Acquire) == self.generation {
            return;
        }
        let mut plan = self.plan.lock();
        let Some(p) = plan.as_mut() else {
            drop(plan);
            self.resolve_ordered();
            return;
        };
        let mut dirty: Vec<usize> = std::mem::take(&mut p.dirty).iter().filter_map(|f| p.group_of.get(f).copied()).collect();
        dirty.sort_unstable();
        dirty.dedup();
        for &g in &dirty {
            for path in &p.groups[g] {
                for c in self.files.get(path).map_or(&[][..], |s| &s.classes) {
                    if self.classes.get(&c.name).is_some_and(|(owner, _)| owner == path) {
                        self.cache.remove(&c.name);
                    }
                }
            }
        }
        for &g in &dirty {
            self.resolve_group(p.groups[g].iter());
        }
        self.resolved_at.store(self.generation, Ordering::Release);
    }

    pub fn scope_for_raw<'a>(&self, stub: &'a FileStub, raw: &RawClass) -> Scope<'a> {
        let mut enclosing = vec![raw.name.clone()];
        let mut type_vars: Vec<Name> = raw.type_params.iter().map(|t| t.name.clone()).collect();
        let mut o = raw.outer.clone();
        while let Some(on) = o {
            enclosing.push(on.clone());
            // the outer class of the same copy, for a class declared in several files
            let outer = stub.classes.iter().find(|c| c.name == on).or_else(|| self.raw_class(&on).map(|(_, c)| c));
            o = match outer {
                Some(oc) => {
                    if !raw.flags.is_static() {
                        type_vars.extend(oc.type_params.iter().map(|t| t.name.clone()));
                    }
                    oc.outer.clone()
                }
                None => None,
            };
        }
        Scope { package: &stub.package, imports: &stub.imports, enclosing, type_vars, local_types: Vec::new(), local_classes: Vec::new(), unknown_if_inherited: false }
    }

    pub fn scope_for_class<'a>(&'a self, name: &str) -> Option<Scope<'a>> {
        let (stub, raw) = self.raw_class(name)?;
        Some(self.scope_for_raw(stub, raw))
    }

    fn resolve_source_class(&self, name: &str) -> Option<ClassInfo> {
        let (path, _) = self.classes.get(name)?;
        let (stub, raw) = self.raw_class(name)?;
        let scope = self.scope_for_raw(stub, raw);
        Some(self.lower_raw_class(raw, &scope, Origin::Source { file: path.clone() }))
    }

    /// Lower a raw class with an explicit scope (also used for local classes).
    pub fn lower_raw_class(&self, raw: &RawClass, scope: &Scope, origin: Origin) -> ClassInfo {
        // Header types must not consult this class's inherited members.
        let header_scope = Scope {
            enclosing: scope.enclosing.iter().skip(1).cloned().collect(),
            // the levels count enclosing classes, one fewer here
            local_types: scope.local_types.iter().map(|(s, b, l)| (s.clone(), b.clone(), l.saturating_sub(1))).collect(),
            ..scope.clone()
        };
        let mut header_scope = header_scope;
        header_scope.type_vars = scope.type_vars.clone();
        let tps = raw
            .type_params
            .iter()
            .map(|t| TypeParam { name: t.name.clone(), bounds: t.bounds.iter().map(|b| self.resolve_type(b, &header_scope)).collect() })
            .collect();
        let superclass = raw.superclass.as_ref().map(|s| self.resolve_type(s, &header_scope));
        let interfaces = raw.interfaces.iter().map(|s| self.resolve_type(s, &header_scope)).collect();
        let permits = raw.permits.iter().map(|s| self.resolve_type(s, &header_scope)).collect();
        let annotations = raw.annotations.iter().map(|a| self.resolve_annotation(a, scope)).collect();
        let mut fields: Vec<FieldInfo> = Vec::with_capacity(raw.fields.len());
        for f in &raw.fields {
            let ty = self.resolve_type(&f.ty, scope);
            let constant = f.constant.clone().or_else(|| {
                let e = f.init.as_ref()?;
                let v = self.eval_const_expr(e, scope, &fields, raw, 0)?;
                coerce_constant(v, &ty)
            });
            fields.push(FieldInfo {
                name: f.name.clone(),
                ty,
                flags: f.flags,
                annotations: f.annotations.iter().map(|a| self.resolve_annotation(a, scope)).collect(),
                constant,
                range: Some(f.range),
            });
        }
        let methods = raw
            .methods
            .iter()
            .map(|m| {
                let mut ms = scope.clone();
                ms.type_vars.extend(m.type_params.iter().map(|t| t.name.clone()));
                MethodInfo {
                    name: m.name.clone(),
                    flags: m.flags,
                    type_params: m
                        .type_params
                        .iter()
                        .map(|t| TypeParam { name: t.name.clone(), bounds: t.bounds.iter().map(|b| self.resolve_type(b, &ms)).collect() })
                        .collect(),
                    params: m
                        .params
                        .iter()
                        .map(|p| ParamInfo {
                            name: p.name.clone(),
                            ty: self.resolve_type(&p.ty, &ms),
                            annotations: p.annotations.iter().map(|a| self.resolve_annotation(a, &ms)).collect(),
                        })
                        .collect(),
                    ret: self.resolve_type(&m.ret, &ms),
                    throws: m.throws.iter().map(|t| self.resolve_type(t, &ms)).collect(),
                    annotations: m.annotations.iter().map(|a| self.resolve_annotation(a, &ms)).collect(),
                    range: Some(m.range),
                    generated_by: m.generated_by.clone(),
                }
            })
            .collect();
        ClassInfo {
            name: raw.name.clone(),
            kind: raw.kind,
            flags: raw.flags,
            type_params: tps,
            superclass: superclass.or_else(|| if raw.kind == ClassKind::Class && &*raw.name != "java.lang.Object" { Some(Ty::object()) } else { None }),
            interfaces,
            fields,
            methods,
            annotations,
            outer: raw.outer.clone(),
            inners: raw.inners.clone(),
            permits,
            origin,
            range: Some(raw.range),
        }
    }

    fn eval_const_expr(&self, e: &ConstExpr, scope: &Scope, done: &[FieldInfo], raw: &RawClass, depth: u32) -> Option<Constant> {
        if depth > 32 {
            return None;
        }
        match e {
            ConstExpr::Lit(c) => Some(c.clone()),
            ConstExpr::Name(n) => {
                if !n.contains('.') {
                    // earlier field of the same class
                    if let Some(f) = done.iter().find(|f| *f.name == **n) {
                        return f.constant.clone();
                    }
                    if let Some(f) = raw.fields.iter().find(|f| *f.name == **n) {
                        if let Some(c) = &f.constant {
                            return Some(c.clone());
                        }
                        if let Some(ie) = &f.init {
                            return evaluating_constant(&raw.name, &f.name, || self.eval_const_expr(ie, scope, done, raw, depth + 1));
                        }
                        return None;
                    }
                }
                match self.resolve_constant_name_at(n, scope, depth + 1)? {
                    AnnotationValue::Const(c) => Some(c),
                    _ => None,
                }
            }
            ConstExpr::Unary(op, v) => {
                let v = self.eval_const_expr(v, scope, done, raw, depth + 1)?;
                crate::stubs::fold_unary(op, v)
            }
            ConstExpr::Binary(op, l, r) => {
                let l = self.eval_const_expr(l, scope, done, raw, depth + 1)?;
                let r = self.eval_const_expr(r, scope, done, raw, depth + 1)?;
                fold_binary(op, &l, &r)
            }
            ConstExpr::Cast(p, v) => {
                let v = self.eval_const_expr(v, scope, done, raw, depth + 1)?;
                coerce_constant(v, &Ty::Prim(*p))
            }
            ConstExpr::CastString(v) => self.eval_const_expr(v, scope, done, raw, depth + 1),
            ConstExpr::Ternary(c, a, b) => {
                let c = self.eval_const_expr(c, scope, done, raw, depth + 1)?.as_bool()?;
                self.eval_const_expr(if c { a } else { b }, scope, done, raw, depth + 1)
            }
        }
    }

    pub fn resolve_annotation(&self, a: &RawAnnotation, scope: &Scope) -> AnnotationInfo {
        let segs: Vec<&str> = a.name.split('.').collect();
        let (name, resolved) = match self.resolve_type_name(&segs, scope) {
            Some(n) => (n, true),
            None => (a.name.clone(), false),
        };
        AnnotationInfo { name, resolved, args: a.args.iter().map(|(k, v)| (k.clone(), self.resolve_ann_value(v, scope))).collect() }
    }

    fn resolve_ann_value(&self, v: &RawAnnValue, scope: &Scope) -> AnnotationValue {
        match v {
            RawAnnValue::Const(c) => AnnotationValue::Const(c.clone()),
            RawAnnValue::ClassLit(t) => AnnotationValue::Class(self.resolve_type(t, scope)),
            RawAnnValue::Annotation(a) => AnnotationValue::Annotation(Box::new(self.resolve_annotation(a, scope))),
            RawAnnValue::Array(v) => AnnotationValue::Array(v.iter().map(|x| self.resolve_ann_value(x, scope)).collect()),
            RawAnnValue::Expr(e) => AnnotationValue::Expr(e.clone()),
            RawAnnValue::Name(n) => self.resolve_constant_name(n, scope).unwrap_or_else(|| AnnotationValue::Expr(n.clone())),
        }
    }

    /// Resolve `CONST`, `Type.CONST` or a static import to an annotation value.
    pub fn resolve_constant_name(&self, n: &str, scope: &Scope) -> Option<AnnotationValue> {
        self.resolve_constant_name_at(n, scope, 0)
    }

    /// `depth`: how deep in constant evaluation this lookup is.
    fn resolve_constant_name_at(&self, n: &str, scope: &Scope, depth: u32) -> Option<AnnotationValue> {
        let segs: Vec<&str> = n.split('.').collect();
        // constants of source classes are read from raw stubs, which also works
        // while the class itself is being lowered
        let raw_constant = |owner: &str, field: &str| -> Option<AnnotationValue> {
            let (_, raw) = self.raw_class(owner)?;
            let f = raw.fields.iter().find(|f| &*f.name == field)?;
            if f.flags.has(Flags::ENUM_CONSTANT) {
                return Some(AnnotationValue::Enum { ty: Arc::from(owner), constant: Arc::from(field) });
            }
            if let Some(c) = &f.constant {
                return Some(AnnotationValue::Const(c.clone()));
            }
            let init = f.init.as_ref()?;
            let owner_scope = self.scope_for_class(owner)?;
            evaluating_constant(owner, &f.name, || self.eval_const_expr(init, &owner_scope, &[], raw, depth + 1)).map(AnnotationValue::Const)
        };
        if segs.len() == 1 {
            for e in &scope.enclosing {
                if let Some(v) = raw_constant(e, segs[0]) {
                    return Some(v);
                }
            }
        } else if let Some((last, rest)) = segs.split_last() {
            if let Some(owner) = self.resolve_type_name(rest, scope) {
                if let Some(v) = raw_constant(&owner, last) {
                    return Some(v);
                }
            }
        }
        let (owner, field) = if segs.len() == 1 {
            let f = segs[0];
            // enclosing classes, then static imports
            let mut owner = None;
            for e in &scope.enclosing {
                if let Some(c) = self.class(e) {
                    if self.field(&c.this_type(), f).is_some() {
                        owner = Some(e.clone());
                        break;
                    }
                }
            }
            if owner.is_none() {
                for imp in scope.imports.iter().filter(|i| i.is_static) {
                    if imp.on_demand || imp.is_named(f) {
                        let cls = if imp.on_demand { imp.path.to_string() } else { imp.path.rsplit_once('.').map(|x| x.0.to_string()).unwrap_or_default() };
                        if let Some(b) = self.binary_name_for_dotted(&cls) {
                            if self.class(&b).is_some_and(|c| c.field(f).is_some()) {
                                owner = Some(b);
                                break;
                            }
                        }
                    }
                }
            }
            (owner?, f)
        } else {
            let (last, rest) = segs.split_last()?;
            (self.resolve_type_name(rest, scope)?, *last)
        };
        let c = self.class(&owner)?;
        let fr = self.field(&c.this_type(), field)?;
        if fr.field.flags.has(Flags::ENUM_CONSTANT) {
            return Some(AnnotationValue::Enum { ty: fr.owner.clone(), constant: Arc::from(field) });
        }
        fr.field.constant.clone().map(AnnotationValue::Const)
    }

    pub fn resolve_type(&self, ts: &TypeSyntax, scope: &Scope) -> Ty {
        match ts {
            TypeSyntax::Prim(p) => Ty::Prim(*p),
            TypeSyntax::Void => Ty::Void,
            TypeSyntax::Var | TypeSyntax::Unknown => Ty::Unknown,
            TypeSyntax::Array(e) => match self.resolve_type(e, scope) {
                Ty::Unknown => Ty::Unknown,
                t => Ty::Array(Box::new(t)),
            },
            TypeSyntax::Wildcard(None) => Ty::Wildcard(None),
            TypeSyntax::Wildcard(Some((ext, b))) => {
                let t = self.resolve_type(b, scope);
                Ty::Wildcard(Some(Box::new(if *ext { WildcardBound::Extends(t) } else { WildcardBound::Super(t) })))
            }
            TypeSyntax::Named(segs) => {
                if segs.len() == 1 && scope.type_vars.iter().any(|v| *v == segs[0].0) {
                    return Ty::TypeVar(segs[0].0.clone());
                }
                let names: Vec<&str> = segs.iter().map(|s| &*s.0).collect();
                match self.resolve_type_name(&names, scope) {
                    Some(n) => {
                        let args = segs.last().map(|s| s.1.iter().map(|a| self.resolve_type(a, scope)).collect()).unwrap_or_default();
                        Ty::Class { name: n, args }
                    }
                    None => Ty::Unknown,
                }
            }
        }
    }

    /// Resolve a (possibly qualified) type name to a binary class name.
    pub fn resolve_type_name(&self, segs: &[&str], scope: &Scope) -> Option<Name> {
        if segs.len() > MAX_NAME_SEGMENTS {
            return None;
        }
        let first = *segs.first()?;
        if let Some(n) = self.resolve_simple_type(first, scope) {
            let mut cur = n;
            for s in &segs[1..] {
                cur = self.member_type(&cur, s)?;
            }
            return Some(cur);
        }
        if segs.len() == 1 {
            return None;
        }
        let dotted = segs.join(".");
        self.binary_name_for_dotted(&dotted).filter(|n| self.variant_visible(n, scope))
    }

    /// `java.util.Map.Entry` -> `java.util.Map$Entry`
    pub fn binary_name_for_dotted(&self, dotted: &str) -> Option<Name> {
        if self.class_exists(dotted) {
            return Some(Arc::from(dotted));
        }
        let segs: Vec<&str> = dotted.split('.').collect();
        if segs.len() > MAX_NAME_SEGMENTS {
            return None;
        }
        for i in (1..segs.len()).rev() {
            let pkg = segs[..i].join(".");
            let top = format!("{}.{}", pkg, segs[i]);
            if self.class_exists(&top) {
                let mut cur: Name = Arc::from(top.as_str());
                let mut ok = true;
                for s in &segs[i + 1..] {
                    match self.member_type(&cur, s) {
                        Some(n) => cur = n,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    return Some(cur);
                }
            }
        }
        if !segs.is_empty() {
            // default package nested: Outer.Inner
            let mut cur: Name = Arc::from(segs[0]);
            if self.class_exists(&cur) {
                for s in &segs[1..] {
                    cur = self.member_type(&cur, s)?;
                }
                return Some(cur);
            }
        }
        None
    }

    /// Member type `simple` of class `owner`, including inherited member types.
    /// None also when that is unknown (see `member_lookup`).
    pub fn member_type(&self, owner: &str, simple: &str) -> Option<Name> {
        match self.member_type_in(owner, simple) {
            MemberType::Found(n) => Some(n),
            _ => None,
        }
    }

    /// Member type `simple` of class `owner`, declared or inherited (JLS
    /// 8.5), and whether an unresolved supertype leaves it unknown.
    pub fn member_type_in(&self, owner: &str, simple: &str) -> MemberType {
        self.member_lookup(owner, simple, None)
    }

    /// Member type `simple` of class `owner`, including inherited ones. For
    /// a class declared in several files the index holds one copy, which need
    /// not be the one a file reads: when the copies disagree about the member
    /// the answer is unknown. `scope`, when `owner` encloses it, narrows the
    /// copies to those with the scope's package and imports (the file's own).
    fn member_lookup(&self, owner: &str, simple: &str, scope: Option<&Scope>) -> MemberType {
        // Resolving a supertype resolves names, which can come back here for
        // the same lookup (classes nested in and extending each other): a
        // re-entrant lookup is "not found" instead of unbounded recursion.
        let key: (Name, Name) = (Arc::from(owner), Arc::from(simple));
        let entered = MEMBER_LOOKUPS.with(|l| {
            let mut l = l.borrow_mut();
            if l.len() > 64 || l.contains(&key) {
                false
            } else {
                l.push(key);
                true
            }
        });
        if !entered {
            return MemberType::NotFound;
        }
        struct Pop;
        impl Drop for Pop {
            fn drop(&mut self) {
                MEMBER_LOOKUPS.with(|l| {
                    l.borrow_mut().pop();
                });
            }
        }
        let _pop = Pop;
        let mut seen = FxHashSet::default();
        self.member_type_rec(owner, simple, &mut seen, 0, scope)
    }

    /// The copies of a class declared in several files that code in `scope`
    /// may read: those with the scope's package and imports when `owner`
    /// encloses the scope and some do, else all. Empty for a class declared once.
    fn copies_read(&self, owner: &str, scope: Option<&Scope>) -> Vec<(&Arc<FileStub>, &RawClass)> {
        let Some(files) = self.copies.get(owner) else { return Vec::new() };
        let all: Vec<(&Arc<FileStub>, &RawClass)> = files.iter().filter_map(|f| self.files.get(f)).filter_map(|st| st.classes.iter().find(|c| &*c.name == owner).map(|c| (st, c))).collect();
        if let Some(scope) = scope.filter(|s| s.enclosing.iter().any(|e| &**e == owner)) {
            // inside its own file a class reads that file's copy, as javac does
            if let Some(&own) = all.iter().find(|(st, _)| std::ptr::eq(&st.imports, scope.imports)) {
                return vec![own];
            }
            let own: Vec<(&Arc<FileStub>, &RawClass)> = all.iter().copied().filter(|(st, _)| *st.package == *scope.package && st.imports.iter().eq(scope.imports.iter())).collect();
            if !own.is_empty() {
                return own;
            }
        }
        // a supertype declared in several files: the copy the scope's file
        // certainly compiles against (its module's), when there is one
        if let Some(file) = scope.and_then(|s| self.scope_file(s)) {
            if let Some((stub, raw, _)) = self.certain_copy(owner, &file) {
                return vec![(stub, raw)];
            }
        }
        all
    }

    /// The file whose stub the scope's imports belong to (cached for the
    /// last scope asked about: a file's lookups ask about the same one).
    fn scope_file(&self, scope: &Scope) -> Option<Arc<str>> {
        // (index address, generation, imports address, file found)
        type LastScope = (usize, u64, usize, Option<Arc<str>>);
        thread_local! {
            static LAST: std::cell::RefCell<Option<LastScope>> = const { std::cell::RefCell::new(None) };
        }
        let key = (self as *const Index as usize, self.generation, scope.imports as *const _ as *const u8 as usize);
        if let Some(hit) = LAST.with(|l| l.borrow().as_ref().filter(|(i, g, p, _)| (*i, *g, *p) == key).map(|(_, _, _, f)| f.clone())) {
            return hit;
        }
        let file = self.files.iter().find(|(_, st)| std::ptr::eq(&st.imports, scope.imports)).map(|(f, _)| f.clone());
        LAST.with(|l| *l.borrow_mut() = Some((key.0, key.1, key.2, file.clone())));
        file
    }

    /// Whether the copies of `owner` code in `scope` may read disagree about
    /// a member type `simple`: some declare it and some do not, or their
    /// supertypes differ.
    fn copies_disagree(&self, owner: &str, simple: &str, scope: &Scope) -> bool {
        let copies = self.copies_read(owner, Some(scope));
        if copies.len() < 2 {
            return false;
        }
        let direct = format!("{owner}${simple}");
        let declaring = copies.iter().filter(|(st, _)| st.classes.iter().any(|c| *c.name == *direct)).count();
        (declaring > 0 && declaring < copies.len()) || !copies.windows(2).all(|w| w[0].1.superclass == w[1].1.superclass && w[0].1.interfaces == w[1].1.interfaces)
    }

    fn member_type_rec(&self, owner: &str, simple: &str, seen: &mut FxHashSet<Name>, depth: u32, scope: Option<&Scope>) -> MemberType {
        if depth > 20 {
            return MemberType::Unknown;
        }
        if !seen.insert(Arc::from(owner)) {
            return MemberType::NotFound;
        }
        let direct = format!("{owner}${simple}");
        // private member types are not inherited (JLS 8.2, 8.5)
        let private = || depth > 0 && self.raw_class(&direct).map(|(_, r)| r.flags).or_else(|| self.class(&direct).map(|c| c.flags)).is_some_and(|f| f.is_private());
        let copies = self.copies_read(owner, scope);
        if copies.is_empty() {
            if self.class_exists(&direct) && !private() {
                return MemberType::Found(Arc::from(direct.as_str()));
            }
        } else {
            let declaring = copies.iter().filter(|(st, _)| st.classes.iter().any(|c| *c.name == *direct)).count();
            if declaring == copies.len() && !private() {
                return MemberType::Found(Arc::from(direct.as_str()));
            }
            if declaring > 0 && declaring < copies.len() {
                return MemberType::Unknown;
            }
            // what the supertypes provide depends on the copy: absent from one
            // copy's supertypes is no answer for the others
            if !copies.windows(2).all(|w| w[0].1.superclass == w[1].1.superclass && w[0].1.interfaces == w[1].1.interfaces) {
                return MemberType::Unknown;
            }
        }
        let mut sups: Vec<Name> = Vec::new();
        let mut unknown = false;
        let mut push = |t: &Ty| match t {
            Ty::Class { name, .. } => sups.push(name.clone()),
            _ => unknown = true,
        };
        let header = |push: &mut dyn FnMut(&Ty), stub: &FileStub, raw: &RawClass| {
            let mut scope = self.scope_for_raw(stub, raw);
            scope.enclosing.remove(0);
            for ts in raw.superclass.iter().chain(raw.interfaces.iter()) {
                push(&self.resolve_type(ts, &scope));
            }
        };
        match (copies.as_slice(), self.class(owner)) {
            // the one copy read, which need not be the index's
            ([(stub, raw)], _) => header(&mut push, stub, raw),
            (_, Some(c)) => {
                if let Some(t) = &c.superclass {
                    push(t);
                }
                for i in &c.interfaces {
                    push(i);
                }
            }
            (_, None) => {
                // the class is being lowered right now: resolve its header from the stub
                let Some((stub, raw)) = self.raw_class(owner) else { return MemberType::Unknown };
                header(&mut push, stub, raw);
            }
        }
        for s in sups {
            match self.member_type_rec(&s, simple, seen, depth + 1, scope) {
                MemberType::Found(n) => return MemberType::Found(n),
                MemberType::Unknown => unknown = true,
                MemberType::NotFound => {}
            }
        }
        if unknown {
            MemberType::Unknown
        } else {
            MemberType::NotFound
        }
    }

    /// The type a simple name denotes where `scope` applies (JLS 6.4.1, 6.5.5):
    /// local classes, then the member types of each enclosing class, its own
    /// and inherited ones (JLS 8.5), and the class's own name one level out;
    /// then single-type imports, the package, on-demand imports, `java.lang`.
    /// None when not found, or when a supertype that is not resolved may
    /// declare the name (unknown).
    pub fn resolve_simple_type(&self, simple: &str, scope: &Scope) -> Option<Name> {
        self.resolve_simple_type_anywhere(simple, scope).filter(|n| self.variant_visible(n, scope))
    }

    /// Whether code in `scope` sees the class `name`. A class declared only
    /// in variant source sets (see `in_variant_set`: an Android flavor, a set
    /// of stubs for a library some builds lack) is seen only from those sets:
    /// elsewhere the build may compile against something else under that name.
    fn variant_visible(&self, name: &str, scope: &Scope) -> bool {
        if self.variant_defs.is_empty() {
            return true;
        }
        let top = |n: &str| -> String { n.rfind('.').map_or_else(|| n.split('$').next().unwrap_or(n).to_string(), |i| n[i..].find('$').map_or(n.to_string(), |j| n[..i + j].to_string())) };
        let name = top(name);
        let Some(&variants) = self.variant_defs.get(name.as_str()) else { return true };
        let files = |n: &str| -> Vec<&Arc<str>> {
            match self.copies.get(n) {
                Some(files) => files.iter().collect(),
                None => self.source_file_of(n).into_iter().collect(),
            }
        };
        let there = files(&name);
        // a copy in main or test code: the ordinary rules apply
        if there.len() as u32 > variants {
            return true;
        }
        let sets: Vec<(&str, &str)> = there.iter().filter_map(|f| source_root(f)).collect();
        // in a class header the class is not yet enclosing: the file whose
        // imports the scope reads
        let here: Vec<&Arc<str>> = match scope.enclosing.last() {
            Some(e) => files(&top(e)),
            None => self.files.iter().filter(|(_, st)| std::ptr::eq(&st.imports, scope.imports)).map(|(f, _)| f).collect(),
        };
        !here.is_empty() && here.iter().all(|f| source_root(f).is_some_and(|r| sets.contains(&r)))
    }

    fn resolve_simple_type_anywhere(&self, simple: &str, scope: &Scope) -> Option<Name> {
        let local = |level: &dyn Fn(usize) -> bool| scope.local_types.iter().find(|(s, _, l)| &**s == simple && level(*l)).map(|(_, b, _)| b.clone());
        // a member type may be inherited from a supertype that is not resolved
        let mut uncertain = false;
        for (i, e) in scope.enclosing.iter().enumerate() {
            if let Some(b) = local(&|l| l == i) {
                return Some(b);
            }
            // a local or anonymous class: from what it declares and inherits
            if let Some(lc) = scope.local_classes.iter().find(|l| l.name == *e) {
                if let Some((_, b)) = lc.members.iter().find(|(s, _)| &**s == simple) {
                    return Some(b.clone());
                }
                for sup in &lc.supers {
                    match self.member_type_in(sup, simple) {
                        MemberType::Found(n) => return Some(n),
                        MemberType::Unknown => uncertain = true,
                        MemberType::NotFound => {}
                    }
                }
                uncertain |= lc.unknown_super;
                continue;
            }
            match self.member_lookup(e, simple, Some(scope)) {
                MemberType::Found(n) => return Some(n),
                // copies of a class declared twice disagree: not an import's class
                // either (an unresolved supertype is the case below)
                MemberType::Unknown if self.copies_disagree(e, simple, scope) => return None,
                MemberType::Unknown => uncertain = true,
                MemberType::NotFound => {}
            }
            // the class's own name is in scope one level out: a member type of
            // that name it inherits comes first
            if simple_name_of(e) == simple && !e.contains("$$") {
                return Some(e.clone());
            }
        }
        if let Some(b) = local(&|l| l >= scope.enclosing.len()) {
            return Some(b);
        }
        // single-type imports
        for imp in scope.imports.single_named(simple) {
            if imp.is_static {
                // static import of a member type
                if let Some((cls, _)) = imp.path.rsplit_once('.') {
                    if let Some(b) = self.binary_name_for_dotted(cls) {
                        if let Some(n) = self.member_type(&b, simple) {
                            return Some(n);
                        }
                    }
                }
                continue;
            }
            if let Some(b) = self.binary_name_for_dotted(&imp.path) {
                return Some(b);
            }
            // the import shadows same-package and on-demand classes even
            // when its jar is missing: unknown, never a same-named class
            return None;
        }
        // an enclosing class inherits from a class that is not resolved: the
        // name may be a member type it inherits, which would shadow the
        // package and on-demand imports
        // same package
        let same = if scope.package.is_empty() { simple.to_string() } else { format!("{}.{}", scope.package, simple) };
        // the default package of another directory: compiled with this file
        // or not (test inputs are compiled one directory at a time), so its
        // class and whatever an import or java.lang provides are both
        // possible, and the name is unknown
        if scope.package.is_empty() && self.classes.contains_key(simple) && self.default_package_visible(simple, scope) != Some(true) {
            return None;
        }
        if uncertain && scope.unknown_if_inherited {
            // java.lang is kept unless the package or an on-demand import has
            // the name too, which would shadow it
            if self.class_exists(&same) || self.resolve_on_demand(simple, scope).is_some() {
                return None;
            }
            let jl = format!("java.lang.{simple}");
            return self.class_exists(&jl).then(|| Arc::from(jl.as_str()));
        }
        if self.class_exists(&same) {
            // a same-package class from another module (a test fixture in a
            // module this one does not depend on) loses to an on-demand
            // import that provides the name; javac would not see it
            let module = |n: &str| self.source_file_of(n).map(|f| module_dir(f).to_string());
            let here = scope.enclosing.last().and_then(|e| module(e));
            let other_module = matches!((module(&same), here), (Some(a), Some(b)) if a != b);
            if !other_module {
                return Some(Arc::from(same.as_str()));
            }
            if let Some(n) = self.resolve_on_demand(simple, scope) {
                return Some(n);
            }
            return Some(Arc::from(same.as_str()));
        }
        if let Some(n) = self.resolve_on_demand(simple, scope) {
            return Some(n);
        }
        let jl = format!("java.lang.{simple}");
        if self.class_exists(&jl) {
            return Some(Arc::from(jl.as_str()));
        }
        None
    }

    /// Whether the default-package class `simple` is declared in the
    /// directory of the code in `scope`: that of the files declaring its
    /// top-level class, or, in a class header, of the file whose imports the
    /// scope reads. Visible when the file cannot be told. None when copies of
    /// either class sit in directories that only partly match.
    fn default_package_visible(&self, simple: &str, scope: &Scope) -> Option<bool> {
        let dir = |f: &str| f.rfind(['/', '\\']).map_or("", |i| &f[..i]).to_string();
        let dirs = |name: &str| -> Vec<String> {
            let files: Vec<&Arc<str>> = match self.copies.get(name) {
                Some(files) => files.iter().collect(),
                None => self.source_file_of(name).into_iter().collect(),
            };
            files.into_iter().map(|f| dir(f)).collect()
        };
        let here: Vec<String> = match scope.enclosing.last() {
            Some(top) => dirs(&top[..top.find('$').unwrap_or(top.len())]),
            None => self.files.iter().filter(|(_, st)| st.package.is_empty() && std::ptr::eq(&st.imports, scope.imports)).map(|(f, _)| dir(f)).collect(),
        };
        let there = dirs(simple);
        if here.is_empty() {
            return Some(true);
        }
        let shared = here.iter().filter(|d| there.contains(d)).count();
        match shared {
            0 => Some(false),
            n if n == here.len() => Some(true),
            _ => None,
        }
    }

    fn resolve_on_demand(&self, simple: &str, scope: &Scope) -> Option<Name> {
        // the file the scope belongs to, for what its module can see
        let here = scope.enclosing.last().and_then(|e| self.source_file_of(e));
        let lookup = |imp: &Import| -> Option<Name> {
            if self.package_exists(&imp.path) && !imp.is_static {
                let c = format!("{}.{}", imp.path, simple);
                if self.class_exists(&c) && self.importable(&c, &imp.path, scope.package, here.map(|h| &**h)) {
                    return Some(Arc::from(c.as_str()));
                }
            }
            self.binary_name_for_dotted(&imp.path).and_then(|b| self.member_type(&b, simple))
        };
        // the file's own on-demand imports shadow module imports (JEP 511)
        if let Some(n) = scope.imports.iter().filter(|i| i.on_demand && !i.from_module).find_map(lookup) {
            return Some(n);
        }
        // module imports: a name exported by two packages is ambiguous
        let mut found: Option<Name> = None;
        for n in scope.imports.iter().filter(|i| i.on_demand && i.from_module).filter_map(lookup) {
            match &found {
                Some(f) if *f != n => return None,
                _ => found = Some(n),
            }
        }
        found
    }

    /// Whether an on-demand import of `pkg` imports the top-level class `c`
    /// into a file of package `from` (at path `here`): only accessible types
    /// are imported (JLS 7.5.2, 6.6.1), so not a class that is not public in
    /// another package. A class of a test source set (`test`, `integrationTest`,
    /// `testFixtures`...) is not visible from main sources, nor from another
    /// module.
    fn importable(&self, c: &str, pkg: &str, from: &str, here: Option<&str>) -> bool {
        if pkg != from {
            let flags = self.raw_class(c).map(|(_, r)| r.flags).or_else(|| self.classpath.class(c).map(|k| k.flags));
            if flags.is_some_and(|f| !f.is_public()) {
                return false;
            }
        }
        let is_test = crate::sources::is_test_path;
        match (self.source_file_of(c), here) {
            (Some(f), Some(h)) if is_test(f) => is_test(h) && module_dir(f) == module_dir(h),
            _ => true,
        }
    }

    /// All source classes whose direct supertypes include `name`.
    pub fn direct_source_subclasses(&self, name: &str) -> Vec<Name> {
        let mut out = Vec::new();
        for n in self.classes.keys() {
            if let Some(c) = self.class(n) {
                let mut hit = c.superclass.as_ref().and_then(|t| t.class_name()) == Some(name);
                hit |= c.interfaces.iter().any(|t| t.class_name() == Some(name));
                if hit {
                    out.push(n.clone());
                }
            }
        }
        out
    }
}

impl ClassLookup for Index {
    fn class(&self, name: &str) -> Option<Arc<ClassInfo>> {
        Index::class(self, name)
    }
    fn may_lack_members(&self, name: &str) -> bool {
        self.classpath.may_lack_members(name)
    }
}

/// The module directory of a source file: the path before the `src` of its
/// source set (`<module>/src/<set>/java/...`), else before its last `/src/`.
/// Not the first `/src/`: the checkout itself may sit below one.
pub fn module_dir(path: &str) -> &str {
    const LANGS: &[&str] = &["java", "kotlin", "groovy", "scala"];
    let starts: Vec<usize> = path.match_indices("/src/").map(|(i, _)| i).collect();
    let set_root = starts.iter().rev().find(|&&i| path[i + 5..].split('/').nth(1).is_some_and(|l| LANGS.contains(&l)));
    match set_root.or(starts.last()) {
        Some(&i) => &path[..i],
        None => "",
    }
}

/// Module directories ending in `/`, the innermost first.
fn innermost_first<T>(modules: Vec<(String, T)>) -> Vec<(String, T)> {
    let sep = std::path::MAIN_SEPARATOR;
    let mut modules: Vec<(String, T)> = modules.into_iter().map(|(d, r)| (if d.ends_with(['/', sep]) { d } else { format!("{d}{sep}") }, r)).collect();
    modules.sort_by_key(|(d, _)| std::cmp::Reverse(d.len()));
    modules
}

/// A file in a source set that is neither main code nor tests
/// (`app/src/fdroid/java`, `src/androidx-stubs/java`): an Android flavor or
/// a set that some builds use instead of a library, or not at all.
pub fn in_variant_set(path: &str) -> bool {
    source_root(path).is_some_and(|(_, set)| set != "main" && !crate::sources::is_test_set(set))
}

/// The module directory and source set of a file under a source root of a
/// module, `<module>/src/<set>/<java|kotlin|groovy|scala>/...` (the last such
/// root: the checkout itself may sit below a directory named `src`). None
/// for a file anywhere else.
pub fn source_root(path: &str) -> Option<(&str, &str)> {
    const LANGS: &[&str] = &["java", "kotlin", "groovy", "scala"];
    let starts = path.match_indices("src/").map(|(i, _)| i).filter(|&i| i == 0 || path[..i].ends_with('/'));
    let i = starts.filter(|&i| path[i + 4..].split('/').nth(1).is_some_and(|l| LANGS.contains(&l)) && path[i + 4..].split('/').count() > 2).last()?;
    let set = path[i + 4..].split('/').next()?;
    Some((path[..i].trim_end_matches('/'), set))
}

/// Convert a folded constant to the representation of the declared type.
pub fn coerce_constant(v: Constant, ty: &Ty) -> Option<Constant> {
    Some(match ty {
        Ty::Prim(Prim::Int) => Constant::Int(match v {
            Constant::Float(f) => f as i32 as i64,
            c => c.as_i64()? as i32 as i64,
        }),
        Ty::Prim(Prim::Long) => Constant::Long(match v {
            Constant::Float(f) => f as i64,
            c => c.as_i64()?,
        }),
        Ty::Prim(Prim::Short) => Constant::Int(v.as_i64()? as i16 as i64),
        Ty::Prim(Prim::Byte) => Constant::Int(v.as_i64()? as i8 as i64),
        Ty::Prim(Prim::Char) => Constant::Char(char::from_u32(v.as_i64()? as u32)?),
        Ty::Prim(Prim::Float) | Ty::Prim(Prim::Double) => Constant::Float(v.as_f64()?),
        Ty::Prim(Prim::Boolean) => Constant::Bool(v.as_bool()?),
        _ => v,
    })
}

#[cfg(test)]
mod resolving_tests {
    use super::*;

    fn resolving(name: &str) -> bool {
        RESOLVING.with(|r| r.borrow().contains(name))
    }

    #[test]
    fn the_guard_marks_a_cycle_and_leaves_on_panic() {
        {
            let outer = Resolving::enter("p.A").expect("enters");
            assert!(Resolving::enter("p.A").is_none(), "a cycle");
            assert!(resolving("p.A"), "the failed entry did not remove the outer one");
            drop(outer);
        }
        assert!(!resolving("p.A"));
        let r = std::panic::catch_unwind(|| {
            let _g = Resolving::enter("p.B").expect("enters");
            panic!("resolution bug");
        });
        assert!(r.is_err());
        assert!(!resolving("p.B"), "left on panic");
    }

    #[test]
    fn a_body_only_edit_of_a_duplicate_class_keeps_every_class_resolved() {
        use crate::stubs::extract;
        use grounds_syntax::ParsedFile;
        let stub = |src: &str| Arc::new(extract(&ParsedFile::parse(src)));
        let mut idx = Index::new(Arc::new(Classpath::empty()));
        idx.set_file(Arc::from("a/p/B.java"), stub("package p; public class B {}"));
        idx.set_file(Arc::from("b/p/B.java"), stub("package p; public class B { void m() {} }"));
        idx.set_file(Arc::from("a/p/C.java"), stub("package p; public class C extends B {}"));
        idx.resolve_ordered();
        for (path, src) in [("b/p/B.java", "package p; public class B { void m() { int i = 0; } }"), ("a/p/B.java", "package p; public class B {  }")] {
            idx.set_file(Arc::from(path), stub(src));
            idx.ensure_resolved();
            let missing: Vec<_> = idx.source_class_names().filter(|n| !idx.cache.contains_key(*n)).collect();
            assert!(missing.is_empty(), "after a body-only edit of {path}: {missing:?}");
        }
    }

    #[test]
    fn the_copy_a_file_compiles_against_is_chosen_only_when_certain() {
        use crate::stubs::extract;
        use grounds_syntax::ParsedFile;
        let stub = |src: &str| Arc::new(extract(&ParsedFile::parse(src)));
        let mut idx = Index::new(Arc::new(Classpath::empty()));
        let b = "package p; public class B {}";
        idx.set_file(Arc::from("/r/a/src/test/java/p/B.java"), stub(b));
        idx.set_file(Arc::from("/r/b/src/main/java/p/B.java"), stub(b));
        idx.set_file(Arc::from("/r/b/src/test/java/p/B.java"), stub(b));
        let pick = |idx: &Index, from: &str| idx.certain_copy("p.B", from).map(|(_, _, f)| f.to_string());
        // the own module and source set; a test sees its module's main code
        assert_eq!(pick(&idx, "/r/a/src/test/java/p/T.java").as_deref(), Some("/r/a/src/test/java/p/B.java"));
        assert_eq!(pick(&idx, "/r/b/src/main/java/p/M.java").as_deref(), Some("/r/b/src/main/java/p/B.java"));
        assert_eq!(pick(&idx, "/r/b/src/test/java/p/T.java").as_deref(), Some("/r/b/src/test/java/p/B.java"));
        assert_eq!(pick(&idx, "/r/b/src/it/java/p/T.java"), None);
        // main code does not see its module's tests; other modules are a guess
        assert_eq!(pick(&idx, "/r/a/src/main/java/p/M.java"), None);
        assert_eq!(pick(&idx, "/r/c/src/test/java/p/T.java"), None);
        idx.remove_file("/r/b/src/test/java/p/B.java");
        assert_eq!(pick(&idx, "/r/b/src/it/java/p/T.java").as_deref(), Some("/r/b/src/main/java/p/B.java"));
        idx.remove_file("/r/b/src/main/java/p/B.java");
        assert!(!idx.is_duplicated("p.B"));
        assert_eq!(pick(&idx, "/r/a/src/test/java/p/T.java"), None);
        // a checkout below a directory named src: modules stay apart
        let mut idx = Index::new(Arc::new(Classpath::empty()));
        idx.set_file(Arc::from("/w/src/r/a/src/test/java/p/B.java"), stub(b));
        idx.set_file(Arc::from("/w/src/r/b/src/main/java/p/B.java"), stub(b));
        assert_eq!(pick(&idx, "/w/src/r/b/src/test/java/p/T.java").as_deref(), Some("/w/src/r/b/src/main/java/p/B.java"));
        assert_eq!(pick(&idx, "/w/src/r/c/src/test/java/p/T.java"), None);
    }

    #[test]
    fn copies_outside_source_roots_are_never_certain() {
        use crate::stubs::extract;
        use grounds_syntax::ParsedFile;
        let stub = |src: &str| Arc::new(extract(&ParsedFile::parse(src)));
        let pick = |idx: &Index, from: &str| idx.certain_copy("p.C", from).map(|(_, _, f)| f.to_string());
        let c = "package p; public class C {}";
        // guava: plain `src/` and `test/` trees, a GWT super-source copy
        let mut idx = Index::new(Arc::new(Classpath::empty()));
        idx.set_file(Arc::from("/r/core/src/p/C.java"), stub(c));
        idx.set_file(Arc::from("/r/gwt/src-super/p/C.java"), stub(c));
        assert_eq!(pick(&idx, "/r/tests/bench/p/B.java"), None);
        assert_eq!(pick(&idx, "/r/core/src/p/B.java"), None);
        // javaparser: test data under src/test/ next to the module's tests
        let mut idx = Index::new(Arc::new(Classpath::empty()));
        idx.set_file(Arc::from("/r/core/src/main/java/p/C.java"), stub(c));
        idx.set_file(Arc::from("/r/solver/src/test/test_sourcecode/p/C.java"), stub(c));
        assert_eq!(pick(&idx, "/r/solver/src/test/java/t/T.java"), None);
        assert_eq!(pick(&idx, "/r/solver/src/test/test_sourcecode/t/T.java"), None);
        // still certain: the own module's source set, or its main code for a test
        idx.set_file(Arc::from("/r/solver/src/main/java/p/C.java"), stub(c));
        assert_eq!(pick(&idx, "/r/solver/src/test/java/t/T.java").as_deref(), Some("/r/solver/src/main/java/p/C.java"));
        assert_eq!(pick(&idx, "/r/core/src/main/java/t/M.java").as_deref(), Some("/r/core/src/main/java/p/C.java"));
        assert_eq!(source_root("/w/src/r/a/src/test/java/p/B.java"), Some(("/w/src/r/a", "test")));
        assert_eq!(source_root("src/main/kotlin/p/B.java"), Some(("", "main")));
        assert_eq!(source_root("/r/a/src/main/java"), None);
        assert_eq!(source_root("/r/a/src/p/B.java"), None);
    }

    #[test]
    fn module_dirs() {
        assert_eq!(module_dir("/w/src/r/a/src/test/java/p/B.java"), "/w/src/r/a");
        assert_eq!(module_dir("/r/a/src/main/java/com/x/src/C.java"), "/r/a");
        assert_eq!(module_dir("/r/src/mdo/java/C.java"), "/r");
        assert_eq!(module_dir("/r/a/src/C.java"), "/r/a");
        assert_eq!(module_dir("C.java"), "");
    }
}

#[cfg(test)]
mod oversized_tests {
    use super::*;
    #[test]
    fn a_directory_too_large_to_list_still_comes_before_later_entries() {
        // `entries = ["cp", "old"]` with `cp` over the limit: its classes are not
        // listed, but `old`'s copy of a class `cp` holds is not the one in use
        let dir = std::env::temp_dir().join(format!("grounds-index-oversized-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for f in ["cp/lib/Lib.class", "cp/lib/OnlyHere.class", "cp/a.txt", "cp/b.txt", "old/lib/Lib.class", "old/lib/Other.class"] {
            std::fs::create_dir_all(dir.join(f).parent().unwrap()).unwrap();
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let containers = vec![ClassContainer::open_dir(&dir.join("cp")), ClassContainer::open_dir(&dir.join("old"))];
        let cp = Classpath::from_containers_within(containers, 4);
        assert_eq!(cp.oversized, [dir.join("cp")]);
        assert_eq!(cp.names.get("lib.Lib"), Some(&0), "the unlisted directory's copy");
        assert_eq!(cp.names.get("lib.Other"), Some(&1));
        assert!(!cp.contains("lib.OnlyHere"), "unknown, not missing: nothing lists it");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
