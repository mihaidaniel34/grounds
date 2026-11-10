//! Workspace discovery: source files, modules and source sets, without
//! running any build tool.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SourceKind {
    Main,
    Test,
    /// Generated sources (annotation processor output); indexed, not analyzed.
    Generated,
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    pub module: usize,
    pub kind: SourceKind,
}

#[derive(Clone, Debug)]
pub struct Module {
    pub dir: PathBuf,
    pub name: String,
    pub build_system: BuildSystem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildSystem {
    Gradle,
    Maven,
    None,
}

#[derive(Debug)]
pub struct Workspace {
    pub root: PathBuf,
    pub modules: Vec<Module>,
    pub files: Vec<SourceFile>,
}

const SKIP_DIRS: &[&str] = &["node_modules", ".git", ".gradle", ".idea", "out", ".grounds", "bin"];

pub use grounds_index::sources::{in_test_module, is_test_path, is_test_set, register_project_root, set_extra_test_sets, source_set_of};

/// A file under the resources of a source set (`src/<set>/resources/**`),
/// `rel` relative to the project root. Such Java files are data that tests
/// read or copy (a whole fixture project, a stub of a library class), not
/// sources of the build: they are neither analyzed nor indexed.
pub fn is_resource(rel: &Path) -> bool {
    let comps: Vec<&std::ffi::OsStr> = rel.components().map(|c| c.as_os_str()).collect();
    comps.windows(3).any(|w| w[0] == "src" && w[2] == "resources")
}

pub fn classify(path: &Path) -> SourceKind {
    let s = path.to_string_lossy();
    if s.contains("/build/generated") || s.contains("/target/generated-sources") || s.contains("/generated-sources/") {
        return SourceKind::Generated;
    }
    match source_set_of(path) {
        Some(set) if is_test_set(set) => SourceKind::Test,
        // `src/main` of a module that exists only for testing
        _ if in_test_module(path) => SourceKind::Test,
        _ => SourceKind::Main,
    }
}

/// Source sets reported by the build (`.grounds/source-sets.txt`, written by
/// `grounds classpath --gradle`): which `Test` tasks run each source set.
#[derive(Debug, Default, Clone)]
pub struct SourceSets {
    /// (module name, source set) -> test task names
    tasks: HashMap<(String, String), Vec<String>>,
}

impl SourceSets {
    pub fn load(root: &Path) -> SourceSets {
        let mut tasks = HashMap::new();
        if let Ok(text) = crate::fsutil::read_text(root.join(".grounds/source-sets.txt")) {
            for line in text.lines() {
                let mut parts = line.split('\t');
                let (Some(module), Some(set)) = (parts.next(), parts.next()) else { continue };
                let module = if module == "." || module.is_empty() { ":".to_string() } else { module.to_string() };
                let ts: Vec<String> = parts.next().unwrap_or("").split(',').filter(|t| !t.is_empty()).map(String::from).collect();
                tasks.insert((module, set.to_string()), ts);
            }
        }
        SourceSets { tasks }
    }

    /// Source sets that some test task runs.
    pub fn test_sets(&self) -> Vec<String> {
        self.tasks.iter().filter(|(_, t)| !t.is_empty()).map(|((_, s), _)| s.clone()).collect()
    }

    /// The Gradle test task for a source set: as reported by the build, else
    /// the convention (`test` for `test`, the suite name for JVM test suites).
    pub fn test_task(&self, module: &str, set: &str) -> String {
        match self.tasks.get(&(module.to_string(), set.to_string())) {
            Some(t) if t.iter().any(|t| t == set) => set.to_string(),
            Some(t) if !t.is_empty() => t[0].clone(),
            _ => set.to_string(),
        }
    }
}

/// The `.java` files under `root` that git tracks although `.gitignore`
/// matches them, relative to `root`. Empty outside a git work tree.
fn tracked_but_ignored(root: &Path) -> Vec<PathBuf> {
    let out = std::process::Command::new("git").arg("-C").arg(root).args(["ls-files", "-z", "--cached", "--ignored", "--exclude-standard", "--", "*.java"]).stderr(std::process::Stdio::null()).output();
    match out {
        Ok(o) if o.status.success() => o.stdout.split(|&b| b == 0).filter(|p| !p.is_empty()).filter_map(|p| std::str::from_utf8(p).ok()).map(PathBuf::from).collect(),
        _ => Vec::new(),
    }
}

impl Workspace {
    pub fn discover(root: &Path, exclude: &[String]) -> Workspace {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mut modules: Vec<Module> = Vec::new();
        let mut files = Vec::new();
        let excludes: Vec<glob::Pattern> = exclude.iter().filter_map(|g| glob::Pattern::new(g).ok()).collect();

        let walker = ignore::WalkBuilder::new(&root)
            .hidden(true)
            .git_ignore(true)
            .git_exclude(true)
            .parents(true)
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                if e.file_type().is_some_and(|t| t.is_dir()) {
                    if SKIP_DIRS.contains(&name.as_ref()) {
                        return false;
                    }
                    // build output except generated sources
                    if name == "build" || name == "target" {
                        return true;
                    }
                }
                true
            })
            .build();
        let mut module_dirs: Vec<(PathBuf, BuildSystem)> = Vec::new();
        let mut java_files: Vec<PathBuf> = Vec::new();
        let source = |p: &Path, name: &str| -> bool {
            if !name.ends_with(".java") || name == "module-info.java" || name == "package-info.java" {
                return false;
            }
            // skip non-generated build outputs: a `build`/`target` directory
            // inside the project and above `src` (not the folders the
            // project sits in, nor a package named `build`)
            let rel = p.strip_prefix(&root).unwrap_or(p);
            let output = rel.components().map(|c| c.as_os_str()).take_while(|c| *c != "src").any(|c| c == "build" || c == "target");
            if output && classify(p) != SourceKind::Generated {
                return false;
            }
            !(is_resource(rel) || excludes.iter().any(|g| g.matches_path(rel)))
        };
        for entry in walker.flatten() {
            let p = entry.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
            if entry.file_type().is_some_and(|t| t.is_file()) {
                match name {
                    "build.gradle" | "build.gradle.kts" => module_dirs.push((p.parent().unwrap().to_path_buf(), BuildSystem::Gradle)),
                    "pom.xml" => module_dirs.push((p.parent().unwrap().to_path_buf(), BuildSystem::Maven)),
                    _ => {}
                }
                if source(p, name) {
                    java_files.push(p.to_path_buf());
                }
            }
        }
        // files git tracks are part of the project whatever .gitignore says
        // (a directory ignored after its files were added): the build compiles them
        for rel in tracked_but_ignored(&root) {
            let skipped = rel.components().any(|c| c.as_os_str().to_str().is_none_or(|n| SKIP_DIRS.contains(&n) || n.starts_with('.')));
            let p = root.join(&rel);
            let Some(name) = rel.file_name().and_then(|n| n.to_str()) else { continue };
            if !skipped && p.is_file() && source(&p, name) {
                java_files.push(p);
            }
        }
        java_files.sort();
        java_files.dedup();
        module_dirs.sort_by(|a, b| a.0.cmp(&b.0));
        module_dirs.dedup_by(|a, b| a.0 == b.0);
        if !module_dirs.iter().any(|(d, _)| *d == root) {
            module_dirs.push((root.clone(), BuildSystem::None));
        }
        for (d, bs) in &module_dirs {
            let name = d.strip_prefix(&root).map(|r| r.to_string_lossy().to_string()).unwrap_or_default();
            modules.push(Module { dir: d.clone(), name: if name.is_empty() { ":".into() } else { name }, build_system: *bs });
        }
        // longest module dir prefix wins
        let mut order: Vec<usize> = (0..modules.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(modules[i].dir.as_os_str().len()));
        java_files.sort();
        for p in java_files {
            let module = order.iter().copied().find(|&i| p.starts_with(&modules[i].dir)).unwrap_or(0);
            let kind = classify(&p);
            files.push(SourceFile { path: p, module, kind });
        }
        Workspace { root, modules, files }
    }

    pub fn module_of(&self, path: &Path) -> Option<usize> {
        let mut best: Option<(usize, usize)> = None;
        for (i, m) in self.modules.iter().enumerate() {
            if path.starts_with(&m.dir) {
                let l = m.dir.as_os_str().len();
                if best.is_none_or(|(_, bl)| l > bl) {
                    best = Some((i, l));
                }
            }
        }
        best.map(|b| b.0)
    }
}

/// Directories of modules whose build compiles against a published release
/// of the project itself: a dependency on an artifact the repository builds
/// (same group and artifact), not on its sources. kafka's
/// `streams/upgrade-system-tests-22` compiles against `kafka-streams:2.2`
/// from Maven Central, while the repository's `streams` sources are today's.
/// For code in such a module, the project's classes from other modules are
/// not what it compiles against.
pub fn modules_built_against_own_release(ws: &Workspace) -> Vec<PathBuf> {
    use crate::java_release::{element, elements, strip_comments, strip_script_comments};
    let fixture = |p: &Path| p.strip_prefix(&ws.root).is_ok_and(|r| r.components().any(|c| c.as_os_str() == "src" || c.as_os_str() == "buildSrc" || c.as_os_str() == "build-logic"));
    let read = |p: &Path| crate::fsutil::read_text(p).ok();
    let mut groups: Vec<String> = Vec::new();
    let mut artifacts: Vec<String> = Vec::new();
    let mut out: Vec<PathBuf> = Vec::new();

    // Maven: the coordinates of every pom, then dependencies on one of them at another version
    struct Pom {
        dir: PathBuf,
        version: Option<String>,
        deps: Vec<(String, String, Option<String>)>,
    }
    let mut poms: Vec<Pom> = Vec::new();
    // the version each artifact of the repository has: (group, artifact, version)
    let mut versions: Vec<(String, String, Option<String>)> = Vec::new();
    for m in ws.modules.iter().filter(|m| m.build_system == BuildSystem::Maven && !fixture(&m.dir)) {
        let Some(text) = read(&m.dir.join("pom.xml")) else { continue };
        let text = strip_comments(&text, "", "<!--", "-->");
        let parent = element(&text, "parent").unwrap_or("").to_string();
        let mut own = text.clone();
        for block in ["parent", "dependencies", "dependencyManagement", "build", "profiles", "reporting", "plugins"] {
            let (open, close) = (format!("<{block}>"), format!("</{block}>"));
            while let (Some(s), Some(e)) = (own.find(&open), own.find(&close)) {
                if e < s {
                    break;
                }
                own.replace_range(s..e + close.len(), "");
            }
        }
        let group = element(&own, "groupId").or_else(|| element(&parent, "groupId")).map(str::to_string);
        let version = element(&own, "version").or_else(|| element(&parent, "version")).map(str::to_string);
        if let (Some(g), Some(a)) = (group, element(&own, "artifactId")) {
            versions.push((g.clone(), a.to_string(), version.clone()));
            groups.push(g);
            artifacts.push(a.to_string());
        }
        let mut main = text.clone();
        for block in ["dependencyManagement", "profiles", "build"] {
            let (open, close) = (format!("<{block}>"), format!("</{block}>"));
            while let (Some(s), Some(e)) = (main.find(&open), main.find(&close)) {
                if e < s {
                    break;
                }
                main.replace_range(s..e + close.len(), "");
            }
        }
        // a runtime dependency says nothing about what the code compiles against
        let deps = elements(&main, "dependency")
            .into_iter()
            .filter(|d| element(d, "scope") != Some("runtime"))
            .filter_map(|d| Some((element(d, "groupId")?.to_string(), element(d, "artifactId")?.to_string(), element(d, "version").map(str::to_string))))
            .collect();
        poms.push(Pom { dir: m.dir.clone(), version, deps });
    }
    for pom in &poms {
        let old = pom.deps.iter().any(|(g, a, v)| {
            // a sibling at its own version comes from the reactor, built from the sources
            let Some((_, _, own_version)) = versions.iter().find(|(og, oa, _)| og == g && oa == a) else { return false };
            let literal = v.as_deref().filter(|v| !v.contains("${"));
            let current = own_version.as_deref().or(pom.version.as_deref());
            literal.is_some_and(|v| Some(v) != current)
        });
        if old {
            out.push(pom.dir.clone());
        }
    }

    // Gradle: the group, the project names and archive names, then external
    // coordinates of those, written out or through an alias
    let mut scripts: Vec<(PathBuf, String)> = Vec::new();
    let mut seen_dirs: Vec<&Path> = Vec::new();
    for m in ws.modules.iter().filter(|m| m.build_system == BuildSystem::Gradle && !fixture(&m.dir)) {
        seen_dirs.push(&m.dir);
        for f in ["build.gradle", "build.gradle.kts"] {
            if let Some(t) = read(&m.dir.join(f)) {
                scripts.push((m.dir.clone(), strip_script_comments(&t, false)));
            }
        }
    }
    if scripts.is_empty() {
        return out;
    }
    let mut extra: Vec<String> = Vec::new();
    for f in ["gradle/libs.versions.toml", "gradle/dependencies.gradle", "settings.gradle", "settings.gradle.kts"] {
        if let Some(t) = read(&ws.root.join(f)) {
            extra.push(strip_script_comments(&t, f.ends_with(".toml")));
        }
    }
    if let Some(t) = read(&ws.root.join("gradle.properties")) {
        groups.extend(t.lines().filter_map(|l| l.trim().strip_prefix("group")).filter_map(|r| r.trim_start().strip_prefix('=')).map(|g| g.trim().to_string()));
    }
    let quoted = |text: &str| -> Vec<(usize, String)> {
        let mut v = Vec::new();
        let b = text.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\'' || b[i] == b'"' {
                if let Some(e) = text[i + 1..].find(b[i] as char) {
                    let s = &text[i + 1..i + 1 + e];
                    if !s.contains('\n') {
                        v.push((i, s.to_string()));
                        i += e + 2;
                        continue;
                    }
                }
            }
            i += 1;
        }
        v
    };
    // the word before `=`, `:` or a space that precedes a string at `at`
    let key_before = |text: &str, at: usize| -> Option<String> {
        let before = text[..at].trim_end();
        let before = before.strip_suffix('=').or_else(|| before.strip_suffix(':')).unwrap_or(before).trim_end();
        let start = before.rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.')).map_or(0, |i| i + 1);
        let key = &before[start..];
        (!key.is_empty()).then(|| key.to_string())
    };
    let all: Vec<&str> = scripts.iter().map(|(_, t)| t.as_str()).chain(extra.iter().map(String::as_str)).collect();
    for text in &all {
        for (at, s) in quoted(text) {
            match key_before(text, at).as_deref() {
                Some("group") if s.contains('.') && !s.contains(' ') => groups.push(s),
                Some(k) if k.ends_with("archivesName") || k.ends_with("archivesBaseName") || k.ends_with("artifactId") => artifacts.push(s),
                _ => {}
            }
        }
    }
    // project names: the last segment of every included path, and module directory names
    for text in &extra {
        if let Some(i) = text.find("include") {
            artifacts.extend(quoted(&text[i..]).into_iter().map(|(_, s)| s.rsplit(':').next().unwrap_or("").to_string()).filter(|s| !s.is_empty()));
        }
    }
    artifacts.extend(seen_dirs.iter().filter_map(|d| d.file_name().and_then(|n| n.to_str()).map(str::to_string)));
    // a group the build does not state: by convention, a package prefix of the sources
    let package_prefix = |g: &str| {
        let dir = format!("/java/{}/", g.replace('.', "/"));
        g.contains('.') && ws.files.iter().any(|f| f.path.to_string_lossy().contains(&dir))
    };
    let own = |coordinate: &str| -> bool {
        let mut parts = coordinate.split(':');
        let (Some(g), Some(a)) = (parts.next(), parts.next()) else { return false };
        artifacts.iter().any(|oa| oa == a) && (groups.iter().any(|og| og == g) || package_prefix(g))
    };
    // aliases of external coordinates: `name: "g:a:v"`, `name = "g:a:v"`, `name = { module = "g:a", ... }`
    let mut aliases: Vec<String> = Vec::new();
    for text in &all {
        for (at, s) in quoted(text) {
            if s.split(':').count() < 2 || s.contains(' ') || !own(&s) {
                continue;
            }
            let key = match key_before(text, at) {
                Some(k) if k == "module" => {
                    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
                    text[line_start..at].split('=').next().map(|k| k.trim().to_string())
                }
                k => k,
            };
            if let Some(k) = key.filter(|k| !k.is_empty() && !matches!(k.as_str(), "implementation" | "api" | "compile")) {
                aliases.push(k.replace(['-', '_'], "."));
                aliases.push(k);
            }
        }
    }
    // a module's declarations: its own script, and `project(':path') { ... }` blocks anywhere
    let mut declarations: Vec<(PathBuf, String)> = Vec::new();
    for (dir, text) in &scripts {
        let mut own_text = String::new();
        let mut last = 0;
        let mut from = 0;
        while let Some(i) = text[from..].find("project(") {
            let start = from + i + "project(".len();
            from = start;
            let Some(close) = text[start..].find(')') else { break };
            let path = text[start..start + close].trim().trim_matches(['\'', '"']);
            let after = text[start + close + 1..].trim_start();
            if !path.starts_with(':') || !after.starts_with('{') {
                continue;
            }
            let open = text.len() - after.len();
            let mut depth = 0;
            let mut end = text.len();
            for (j, c) in text[open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + j;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            declarations.push((ws.root.join(path.trim_start_matches(':').replace(':', "/")), text[open..end].to_string()));
            own_text.push_str(&text[last..start - "project(".len()]);
            last = end;
            from = end.max(start);
        }
        own_text.push_str(&text[last.min(text.len())..]);
        declarations.push((dir.clone(), own_text));
    }
    // `testImplementation "g:a:v"`, `implementation("g:a:v")`, `api libs.x`
    let configuration_before = |text: &str, at: usize| -> bool {
        let before = text[..at].trim_end();
        let before = before.strip_suffix('(').unwrap_or(before).trim_end();
        let start = before.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
        let word = before[start..].to_ascii_lowercase();
        // not `runtimeOnly`: what the code compiles against is the question
        ["implementation", "api", "compileonly", "compile"].iter().any(|c| word.ends_with(c))
    };
    for (dir, text) in declarations {
        let direct = quoted(&text).into_iter().any(|(at, s)| s.split(':').count() >= 2 && !s.contains(' ') && own(&s) && configuration_before(&text, at));
        let through_alias = aliases.iter().any(|a| {
            text.match_indices(&format!(".{a}")).any(|(i, m)| {
                let whole = text[i + m.len()..].chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '.'));
                let start = text[..i].rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).map_or(0, |j| j + 1);
                whole && configuration_before(&text, start)
            })
        });
        if (direct || through_alias) && !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_modules_depending_on_an_older_release_of_the_project() {
        let dir = std::env::temp_dir().join(format!("grounds-own-release-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pom = |groupless: bool, artifact: &str, deps: &str| {
            let group = if groupless { String::new() } else { "<groupId>org.example</groupId>".to_string() };
            format!("<project><parent><groupId>org.example</groupId><artifactId>parent</artifactId><version>3.0-SNAPSHOT</version></parent>{group}<artifactId>{artifact}</artifactId><dependencies>{deps}</dependencies></project>")
        };
        let dep = |artifact: &str, version: &str| format!("<dependency><groupId>org.example</groupId><artifactId>{artifact}</artifactId><version>{version}</version></dependency>");
        for (rel, text) in [
            ("pom.xml", "<project><groupId>org.example</groupId><artifactId>parent</artifactId><version>3.0-SNAPSHOT</version></project>".to_string()),
            ("core/pom.xml", pom(true, "core", "")),
            ("app/pom.xml", pom(false, "app", &dep("core", "${project.version}"))),
            ("pinned/pom.xml", pom(true, "pinned", &dep("core", "3.0-SNAPSHOT"))),
            ("compat/pom.xml", pom(true, "compat", &(dep("core", "2.1.0") + &dep("junit", "4.13")))),
            ("other/pom.xml", pom(true, "other", "<dependency><groupId>com.acme</groupId><artifactId>core</artifactId><version>1.0</version></dependency>")),
        ] {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let ws = Workspace::discover(&dir, &[]);
        let found: Vec<String> = modules_built_against_own_release(&ws).iter().map(|d| d.strip_prefix(&ws.root).unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(found, vec!["compat"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reactor_sibling_at_its_own_version_or_a_runtime_dependency_is_no_published_release() {
        let dir = std::env::temp_dir().join(format!("grounds-own-release-2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let write = |rel: &str, text: &str| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        // Maven: app at version 2 depends on its sibling lib, whose own version is 1
        write("pom.xml", "<project><groupId>g</groupId><artifactId>root</artifactId><version>1</version><modules><module>lib</module><module>app</module></modules></project>");
        write("lib/pom.xml", "<project><groupId>g</groupId><artifactId>lib</artifactId><version>1</version></project>");
        write("app/pom.xml", "<project><groupId>g</groupId><artifactId>app</artifactId><version>2</version><dependencies><dependency><groupId>g</groupId><artifactId>lib</artifactId><version>1</version></dependency></dependencies></project>");
        // Gradle: the project dependency compiles; a published copy at run time only
        write("gr/settings.gradle", "include 'lib', 'app'\n");
        write("gr/build.gradle", "allprojects { group = 'org.example' }\n");
        write("gr/app/build.gradle", "dependencies {\n  implementation project(':lib')\n  runtimeOnly 'org.example:lib:0.1'\n}\n");
        write("gr/lib/build.gradle", "\n");
        let ws = Workspace::discover(&dir, &[]);
        assert!(modules_built_against_own_release(&ws).is_empty(), "{:?}", modules_built_against_own_release(&ws));
        let ws = Workspace::discover(&dir.join("gr"), &[]);
        assert!(modules_built_against_own_release(&ws).is_empty(), "{:?}", modules_built_against_own_release(&ws));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
