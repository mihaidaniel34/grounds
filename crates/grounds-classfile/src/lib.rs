//! Reading compiled Java declarations: jars, directories of class files and
//! the JDK's `ct.sym` symbol archive.

pub mod classfile;
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
}

impl std::fmt::Debug for ClassContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClassContainer::Jar(z) => write!(f, "Jar({})", z.path.display()),
            ClassContainer::Dir(p) => write!(f, "Dir({})", p.display()),
            ClassContainer::CtSym { archive, .. } => write!(f, "CtSym({})", archive.path.display()),
        }
    }
}

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
    /// package-info).
    pub fn class_names(&self) -> Vec<String> {
        let keep = |n: &str| !n.ends_with("module-info") && !n.ends_with("package-info");
        match self {
            ClassContainer::Jar(z) => z
                .entries()
                .iter()
                .filter_map(|e| e.name.strip_suffix(".class"))
                .filter(|n| !n.starts_with("META-INF/"))
                .filter(|n| keep(n))
                .map(|n| n.to_string())
                .collect(),
            ClassContainer::CtSym { entries, .. } => entries.keys().filter(|n| keep(n)).cloned().collect(),
            ClassContainer::Dir(root) => {
                let mut out = Vec::new();
                let mut stack = vec![root.clone()];
                while let Some(d) = stack.pop() {
                    let Ok(rd) = std::fs::read_dir(&d) else { continue };
                    for e in rd.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            stack.push(p);
                        } else if p.extension().is_some_and(|x| x == "class") {
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
        }
    }

    pub fn read_class_bytes(&self, internal_name: &str) -> Option<Vec<u8>> {
        match self {
            ClassContainer::Jar(z) => z.read(&format!("{internal_name}.class")),
            ClassContainer::Dir(root) => std::fs::read(root.join(format!("{internal_name}.class"))).ok(),
            ClassContainer::CtSym { archive, entries } => {
                let idx = *entries.get(internal_name)?;
                archive.read_entry(&archive.entries()[idx]).ok()
            }
        }
    }

    pub fn read_class(&self, internal_name: &str) -> Option<ClassFile> {
        parse_class(&self.read_class_bytes(internal_name)?).ok()
    }

    /// Read a non-class resource (e.g. Spring's AutoConfiguration.imports).
    pub fn read_resource(&self, name: &str) -> Option<Vec<u8>> {
        match self {
            ClassContainer::Jar(z) => z.read(name),
            ClassContainer::Dir(root) => std::fs::read(root.join(name)).ok(),
            ClassContainer::CtSym { .. } => None,
        }
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
    use super::*;

    #[test]
    fn release_chars() {
        assert_eq!(ct_sym_release_char(8), '8');
        assert_eq!(ct_sym_release_char(11), 'B');
        assert_eq!(ct_sym_release_char(21), 'L');
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
