//! Reading compiled Java declarations: jars, directories of class files, the
//! JDK's `ct.sym` symbol archive and the JDK's runtime image.

pub mod classfile;
pub mod jimage;
pub mod signature;
pub mod zip;

pub use classfile::{parse_class, Annotation, ClassFile, ConstValue, ElementValue, FieldInfo, MethodInfo};

use anyhow::Result;
use std::path::{Path, PathBuf};

/// A source of class files addressable by internal name (`java/lang/String`).
pub enum ClassContainer {
    Jar(zip::ZipArchive),
    Dir(PathBuf),
    /// `ct.sym` restricted to one release; maps internal names to entries.
    CtSym { archive: zip::ZipArchive, entries: rustc_hash::FxHashMap<String, usize> },
    /// The running JDK's own release, from its runtime image (`lib/modules`).
    JImage(jimage::JImage),
}

impl std::fmt::Debug for ClassContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClassContainer::Jar(z) => write!(f, "Jar({})", z.path.display()),
            ClassContainer::Dir(p) => write!(f, "Dir({})", p.display()),
            ClassContainer::CtSym { archive, .. } => write!(f, "CtSym({})", archive.path.display()),
            ClassContainer::JImage(j) => write!(f, "JImage({})", j.path.display()),
        }
    }
}

/// Files and directories a directory of classes may hold. More than this and
/// the entry is something else (`/`, a home directory, a jar cache): listing
/// it takes minutes and finds no packages. It is then not read at all, since
/// the part listed before the limit would depend on the file system's order.
pub const DIR_LIMIT: usize = 200_000;

/// Release character used by ct.sym directory names: 8, 9, A=10, B=11, ...
pub fn ct_sym_release_char(release: u32) -> char {
    match release {
        0..=9 => char::from_digit(release.max(8), 10).unwrap(),
        r => (b'A' + (r - 10) as u8) as char,
    }
}

impl ClassContainer {
    pub fn open_jar(path: &Path) -> Result<ClassContainer> {
        Ok(ClassContainer::Jar(zip::ZipArchive::open(path)?))
    }

    pub fn open_dir(path: &Path) -> ClassContainer {
        ClassContainer::Dir(path.to_path_buf())
    }

    /// The JDK at `home` for a release. ct.sym has no signatures for the JDK's
    /// own release, so that release is read from the runtime image; any other
    /// release, or an image that cannot be read, comes from ct.sym.
    pub fn open_jdk(home: &Path, release: u32) -> Result<ClassContainer> {
        let ct = home.join("lib/ct.sym");
        if jdk_is_own_release(home, release) {
            if let Ok(image) = jimage::JImage::open(&home.join("lib/modules")) {
                return Ok(ClassContainer::JImage(image));
            }
        }
        Self::open_ct_sym(&ct, release)
    }

    /// Open `ct.sym` for a release. ct.sym only carries signatures for
    /// releases older than the JDK itself (the JDK's own release is a bare
    /// `system-modules` marker pointing at `lib/modules`), so fall back to the
    /// newest release with signatures at or below the requested one, else the
    /// newest available.
    pub fn open_ct_sym(path: &Path, release: u32) -> Result<ClassContainer> {
        let archive = zip::ZipArchive::open(path)?;
        let requested = ct_sym_release_char(release);
        // release chars ('8', '9', 'A'=10, ...) sort in release order
        let with_sigs: std::collections::BTreeSet<char> = archive
            .entries()
            .iter()
            .filter(|e| e.name.ends_with(".sig"))
            .filter_map(|e| e.name.split('/').next())
            .flat_map(|d| d.chars().collect::<Vec<_>>())
            .filter(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
            .collect();
        let want = if with_sigs.contains(&requested) {
            requested
        } else {
            with_sigs.range(..requested).next_back().or_else(|| with_sigs.iter().next_back()).copied().unwrap_or(requested)
        };
        let mut entries = rustc_hash::FxHashMap::default();
        for (i, e) in archive.entries().iter().enumerate() {
            let Some(rest) = e.name.strip_suffix(".sig") else { continue };
            let mut parts = rest.splitn(3, '/');
            let (Some(rel), Some(_module), Some(name)) = (parts.next(), parts.next(), parts.next()) else { continue };
            if rel.contains(want) {
                entries.insert(name.to_string(), i);
            }
        }
        Ok(ClassContainer::CtSym { archive, entries })
    }

    /// All internal class names in this container (excluding module-info and
    /// package-info). A directory over [`DIR_LIMIT`] has none.
    pub fn class_names(&self) -> Vec<String> {
        self.class_names_within(DIR_LIMIT).unwrap_or_default()
    }

    /// As [`ClassContainer::class_names`]; None for a directory with more
    /// than `limit` files and directories below it.
    pub fn class_names_within(&self, limit: usize) -> Option<Vec<String>> {
        let keep = |n: &str| !n.ends_with("module-info") && !n.ends_with("package-info");
        Some(match self {
            ClassContainer::Jar(z) => z
                .entries()
                .iter()
                .filter_map(|e| e.name.strip_suffix(".class"))
                .filter(|n| !n.starts_with("META-INF/"))
                .filter(|n| keep(n))
                .map(|n| n.to_string())
                .collect(),
            ClassContainer::CtSym { entries, .. } => entries.keys().filter(|n| keep(n)).cloned().collect(),
            ClassContainer::JImage(j) => j.class_names().filter(|n| keep(n)).map(str::to_string).collect(),
            ClassContainer::Dir(root) => {
                let mut out = Vec::new();
                // Every directory is listed under each path that leads to
                // it, links included, as a compiler finds it (build tools
                // link output directories). Only a directory that is already
                // above the path it would be listed under is not entered:
                // `up -> ..`, or `/proc/self/root` under `/`, never ended.
                struct Above {
                    real: PathBuf,
                    up: Option<std::rc::Rc<Above>>,
                }
                let within = |real: &Path, mut above: Option<&std::rc::Rc<Above>>| {
                    while let Some(a) = above {
                        if a.real == real {
                            return true;
                        }
                        above = a.up.as_ref();
                    }
                    false
                };
                let top = std::rc::Rc::new(Above { real: root.canonicalize().unwrap_or_else(|_| root.clone()), up: None });
                let mut todo = vec![(root.clone(), top)];
                let mut seen = 0usize;
                while let Some((d, above)) = todo.pop() {
                    let Ok(rd) = std::fs::read_dir(&d) else { continue };
                    for e in rd.flatten() {
                        seen += 1;
                        if seen > limit {
                            return None;
                        }
                        let p = e.path();
                        // the entry's own type costs no call; only a link is followed
                        let Ok(kind) = e.file_type() else { continue };
                        let link = kind.is_symlink();
                        if kind.is_dir() || (link && p.is_dir()) {
                            let real = if link { p.canonicalize().ok() } else { Some(above.real.join(e.file_name())) };
                            if let Some(real) = real.filter(|r| !within(r, Some(&above))) {
                                todo.push((p, std::rc::Rc::new(Above { real, up: Some(above.clone()) })));
                            }
                        // a regular file: a pipe named `A.class` would block the read
                        } else if (kind.is_file() || (link && p.is_file())) && p.extension().is_some_and(|x| x == "class") {
                            if let Ok(rel) = p.strip_prefix(root) {
                                let s = rel.with_extension("").to_string_lossy().replace('\\', "/");
                                if keep(&s) {
                                    out.push(s);
                                }
                            }
                        }
                    }
                }
                out
            }
        })
    }

    pub fn read_class_bytes(&self, internal_name: &str) -> Option<Vec<u8>> {
        match self {
            ClassContainer::Jar(z) => z.read(&format!("{internal_name}.class")),
            ClassContainer::Dir(root) => std::fs::read(root.join(format!("{internal_name}.class"))).ok(),
            ClassContainer::CtSym { archive, entries } => {
                let idx = *entries.get(internal_name)?;
                archive.read_entry(&archive.entries()[idx]).ok()
            }
            ClassContainer::JImage(j) => j.read_class_bytes(internal_name),
        }
    }

    pub fn read_class(&self, internal_name: &str) -> Option<ClassFile> {
        let mut class = parse_class(&self.read_class_bytes(internal_name)?).ok()?;
        if matches!(self, ClassContainer::JImage(_)) {
            // ct.sym carries only public classes (a protected nested class is public in its class file)
            // and only their public and protected members; the image has everything
            if class.access & classfile::access::PUBLIC == 0 {
                return None;
            }
            let api = |access: u16| access & (classfile::access::PUBLIC | classfile::access::PROTECTED) != 0;
            class.fields.retain(|f| api(f.access));
            class.methods.retain(|m| api(m.access));
        }
        Some(class)
    }

    /// Read a non-class resource (e.g. Spring's AutoConfiguration.imports).
    pub fn read_resource(&self, name: &str) -> Option<Vec<u8>> {
        match self {
            ClassContainer::Jar(z) => z.read(name),
            ClassContainer::Dir(root) => std::fs::read(root.join(name)).ok(),
            ClassContainer::CtSym { .. } | ClassContainer::JImage(_) => None,
        }
    }
}

/// The JDK's feature release (21 for `JAVA_VERSION="21.0.12.1"`, 8 for
/// `"1.8.0_292"`), from `<home>/release`.
pub fn jdk_feature_release(home: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(home.join("release")).ok()?;
    let version = text.lines().find_map(|l| l.strip_prefix("JAVA_VERSION="))?;
    let nums: Vec<u32> = version.split(|c: char| !c.is_ascii_digit()).filter_map(|s| s.parse().ok()).collect();
    match nums.as_slice() {
        [1, v, ..] => Some(*v),
        [v, ..] => Some(*v),
        [] => None,
    }
}

/// Whether `release` is the JDK's own release, whose classes are in its runtime image.
pub fn jdk_is_own_release(home: &Path, release: u32) -> bool {
    match jdk_feature_release(home) {
        Some(v) => v == release,
        // no `release` file: ct.sym marks the JDK's own release with a bare `system-modules` entry
        None => zip::ZipArchive::open(&home.join("lib/ct.sym")).is_ok_and(|z| z.contains(&format!("{}/system-modules", ct_sym_release_char(release)))),
    }
}

/// Locate the JDK home: `$JAVA_HOME`, then macOS `java_home`, then `java` on PATH.
pub fn find_java_home() -> Option<PathBuf> {
    if let Ok(h) = std::env::var("JAVA_HOME") {
        let p = PathBuf::from(h);
        if p.join("lib/ct.sym").exists() || p.join("lib/modules").exists() {
            return Some(p);
        }
    }
    if let Ok(out) = std::process::Command::new("/usr/libexec/java_home").output() {
        if out.status.success() {
            let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if p.exists() {
                return Some(p);
            }
        }
    }
    // `java` on PATH (Nix, SDKMAN, asdf, distro alternatives): follow the
    // symlinks to the real binary; the home is its parent dir's parent, or
    // `lib/openjdk` under the package (nixpkgs layout)
    let is_home = |p: &Path| p.join("lib/ct.sym").exists() || p.join("lib/modules").exists();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let Ok(java) = dir.join("java").canonicalize() else { continue };
            let Some(home) = java.parent().and_then(Path::parent) else { continue };
            for cand in [home.to_path_buf(), home.join("lib/openjdk")] {
                if is_home(&cand) {
                    return Some(cand);
                }
            }
        }
    }
    for dir in ["/usr/lib/jvm/default-java", "/usr/lib/jvm/default"] {
        let p = PathBuf::from(dir);
        if p.join("lib/ct.sym").exists() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_class_directory_with_a_symlink_loop_is_listed_once() {
        // found by scripts/fuzz_robust.py: `[classpath] entries = ["/"]` never ended
        let dir = std::env::temp_dir().join(format!("grounds-classdir-loop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("p")).unwrap();
        std::fs::write(dir.join("p/A.class"), b"").unwrap();
        std::os::unix::fs::symlink("..", dir.join("p/up")).unwrap();
        std::os::unix::fs::symlink(".", dir.join("self")).unwrap();
        // aliases of a package directory are packages too, whichever read_dir lists first
        for alias in ["a", "z"] {
            std::os::unix::fs::symlink("p", dir.join(alias)).unwrap();
        }
        // two links that lead to each other
        std::fs::create_dir_all(dir.join("q/r")).unwrap();
        std::os::unix::fs::symlink("../q", dir.join("p/toq")).unwrap();
        std::os::unix::fs::symlink("../../p", dir.join("q/r/top")).unwrap();
        let mut names = super::ClassContainer::open_dir(&dir).class_names();
        names.sort();
        // (`q/r/top` is `p` again, and its `toq` would be `q` inside `q`)
        assert_eq!(names, ["a/A", "p/A", "q/r/top/A", "z/A"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_package_linked_to_a_directory_inside_the_class_directory_is_listed() {
        // a store beside the links into it; the walk used to list each real directory once, under the store's name
        let dir = std::env::temp_dir().join(format!("grounds-classdir-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".store/lib")).unwrap();
        std::fs::write(dir.join(".store/lib/Lib.class"), b"").unwrap();
        std::os::unix::fs::symlink(".store/lib", dir.join("lib")).unwrap();
        // not a file: reading it would wait for a writer
        let fifo = std::process::Command::new("mkfifo").arg(dir.join(".store/lib/Pipe.class")).status();
        let mut names = super::ClassContainer::open_dir(&dir).class_names();
        names.sort();
        assert_eq!(names, [".store/lib/Lib", "lib/Lib"], "{fifo:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_over_the_limit_is_not_listed() {
        // found by scripts/fuzz_robust.py: `[classpath] entries = ["/"]` listed the whole file system
        let dir = std::env::temp_dir().join(format!("grounds-classdir-limit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("p/q")).unwrap();
        for f in ["p/A.class", "p/B.class", "p/q/C.class", "p/notes.txt"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let c = super::ClassContainer::open_dir(&dir);
        // p, q and four files
        let mut names = c.class_names_within(6).unwrap();
        names.sort();
        assert_eq!(names, vec!["p/A".to_string(), "p/B".to_string(), "p/q/C".to_string()]);
        assert_eq!(c.class_names_within(5), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    #[test]
    fn release_chars() {
        assert_eq!(ct_sym_release_char(8), '8');
        assert_eq!(ct_sym_release_char(11), 'B');
        assert_eq!(ct_sym_release_char(21), 'L');
    }

    #[test]
    fn reads_jdk_own_release_from_image() {
        let Some(home) = find_java_home() else { return };
        let Some(release) = jdk_feature_release(&home) else { return };
        let c = ClassContainer::open_jdk(&home, release).unwrap();
        assert!(matches!(c, ClassContainer::JImage(_)), "{c:?}");
        let s = c.read_class("java/lang/String").expect("String in the runtime image");
        assert_eq!(s.this_class, "java/lang/String");
        let list = c.read_class("java/util/List").expect("List in the runtime image");
        // private members are not part of the API ct.sym carries
        assert!(c.read_class("java/util/ArrayList").is_some_and(|a| !a.methods.iter().any(|m| m.name == "grow")));
        if release >= 21 {
            // SequencedCollection and List.getFirst arrived in 21
            assert!(c.read_class("java/util/SequencedCollection").is_some_and(|s| s.methods.iter().any(|m| m.name == "reversed")));
            assert!(list.methods.iter().any(|m| m.name == "getFirst"));
        }
        // only exported packages: ct.sym does not carry jdk.internal.misc
        assert!(c.read_class("jdk/internal/misc/Unsafe").is_none());
        // ...while the image knows every package, internal ones included
        let ClassContainer::JImage(image) = &c else { unreachable!() };
        let packages: Vec<&str> = image.all_packages().collect();
        assert!(packages.contains(&"jdk/internal/misc") && packages.contains(&"com/sun/tools/javac/util") && packages.contains(&"java/lang"), "{}", packages.len());
        // only public classes: ImmutableCollections is package-private; a protected nested class is public
        assert!(c.read_class("java/util/ImmutableCollections").is_none());
        assert!(c.read_class("java/awt/Container$AccessibleAWTContainer").is_some());
        assert!(c.class_names().len() > 1000);
    }

    #[test]
    fn feature_release_from_version_string() {
        let dir = std::env::temp_dir().join(format!("grounds-jdk-release-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (version, want) in [("21.0.12.1", 21), ("1.8.0_292", 8), ("25-ea", 25)] {
            std::fs::write(dir.join("release"), format!("IMPLEMENTOR=\"x\"\nJAVA_VERSION=\"{version}\"\n")).unwrap();
            assert_eq!(jdk_feature_release(&dir), Some(want), "{version}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_jdk_ct_sym() {
        let Some(home) = find_java_home() else { return };
        let ct = home.join("lib/ct.sym");
        if !ct.exists() {
            return;
        }
        // 21 may be the JDK's own release (marker only): falls back to 20
        let c = ClassContainer::open_ct_sym(&ct, 21).unwrap();
        let s = c.read_class("java/lang/String").expect("String in ct.sym");
        assert_eq!(s.this_class, "java/lang/String");
        assert!(s.methods.iter().any(|m| m.name == "isBlank"));
        let names = c.class_names();
        assert!(names.len() > 1000);
    }
}
