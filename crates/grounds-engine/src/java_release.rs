//! The Java release a project compiles for, read from its build files when
//! `grounds.toml` does not set `java_release`: JDK APIs newer than the
//! project's target must not count (`CharSequence.isEmpty` exists only since
//! Java 15, so a Java 8 class declaring `isEmpty()` overrides nothing).
//!
//! Maven: `maven.compiler.release`/`target`/`source` properties and the
//! maven-compiler-plugin's `<release>`/`<target>`/`<source>`, with `${...}`
//! resolved through parent poms inside the repository. Gradle (Groovy and
//! Kotlin DSL): `options.release`, `targetCompatibility`,
//! `sourceCompatibility` and `toolchain { languageVersion }`.
//!
//! One JDK symbol table serves the whole project, so with modules on
//! different releases the highest one is used: a lower one hides JDK members
//! the newer modules use (a project class implementing `Stream` has no
//! `toList()` before Java 16, so its calls would be reported as not
//! compiling), while a higher one only lets older modules see a few newer
//! supertype methods.

use crate::workspace::{BuildSystem, Workspace};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The highest release the project's builds compile for, if any build says.
pub fn infer(ws: &Workspace) -> Option<u32> {
    let mut poms = PomCache::default();
    let mut found: Option<u32> = None;
    let mut note = |v: Option<u32>| found = found.max(v);
    for m in &ws.modules {
        // builds inside a source tree are test fixtures, not the project's
        if m.dir.strip_prefix(&ws.root).is_ok_and(|r| r.components().any(|c| c.as_os_str() == "src")) {
            continue;
        }
        match m.build_system {
            BuildSystem::Maven => note(poms.release(&m.dir.join("pom.xml"))),
            BuildSystem::Gradle => {
                for f in ["build.gradle", "build.gradle.kts"] {
                    note(std::fs::read_to_string(m.dir.join(f)).ok().and_then(|t| gradle_release(&t)));
                }
            }
            BuildSystem::None => {}
        }
    }
    // Gradle convention plugins (`id("java-conventions")`) set it for every module
    for dir in ["buildSrc", "build-logic"] {
        let mut scripts = Vec::new();
        gradle_scripts(&ws.root.join(dir), 0, &mut scripts);
        for s in scripts {
            note(std::fs::read_to_string(&s).ok().and_then(|t| gradle_release(&t)));
        }
    }
    found
}

fn gradle_scripts(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if depth < 8 && !matches!(name.as_str(), "build" | ".gradle") {
                gradle_scripts(&p, depth + 1, out);
            }
        } else if name.ends_with(".gradle") || name.ends_with(".gradle.kts") {
            out.push(p);
        }
    }
}

/// A Java version as written in a build file: `1.8`, `8`, `"17"`,
/// `JavaVersion.VERSION_1_8`, `JavaVersion.VERSION_17`,
/// `JavaVersion.toVersion(11)`, `JavaLanguageVersion.of(21)`.
pub fn parse_version(s: &str) -> Option<u32> {
    let s = s.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    let s = s.strip_prefix("JavaVersion.").unwrap_or(s);
    let s = s.strip_prefix("VERSION_").map(|v| v.replace('_', ".")).unwrap_or_else(|| s.to_string());
    let s = ["toVersion(", "JavaLanguageVersion.of("].iter().find_map(|p| s.strip_prefix(p)).map(|v| v.trim_end_matches(')').to_string()).unwrap_or(s);
    let s = s.trim().trim_matches(|c| c == '"' || c == '\'');
    let s = s.strip_prefix("1.").unwrap_or(s);
    let v: u32 = s.split('.').next()?.parse().ok()?;
    (5..=99).contains(&v).then_some(v)
}

/// Release a Gradle build script declares: `options.release` first, then
/// `targetCompatibility`, `sourceCompatibility`, the toolchain's
/// `languageVersion`; the highest when one is set several times.
pub fn gradle_release(text: &str) -> Option<u32> {
    let text = strip_comments(text, "//", "/*", "*/");
    let keys: [&[&str]; 4] = [&["options.release", "release"], &["targetCompatibility"], &["sourceCompatibility"], &["languageVersion"]];
    keys.iter().find_map(|names| names.iter().filter_map(|k| gradle_assignments(&text, k)).max())
}

/// Highest version assigned to `key`: `key = V`, `key.set(V)`, `key V`.
fn gradle_assignments(text: &str, key: &str) -> Option<u32> {
    let mut best = None;
    let mut from = 0;
    while let Some(i) = text[from..].find(key) {
        let start = from + i;
        from = start + key.len();
        // a whole word: not `jvmTarget`, not `release` inside `options.release`
        // when looking for bare `release` (that one is found by its own key)
        let before = text[..start].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || (key == "release" && c == '.')) {
            continue;
        }
        let rest = text[from..].trim_start();
        let value = if let Some(r) = rest.strip_prefix(".set(") {
            Some(r)
        } else if let Some(r) = rest.strip_prefix('=') {
            Some(r)
        } else if key != "release" && (rest.starts_with(['\'', '"']) || rest.starts_with(|c: char| c.is_ascii_digit()) || rest.starts_with("JavaVersion")) {
            // Groovy's `sourceCompatibility 1.8`
            Some(rest)
        } else {
            None
        };
        // the value runs to the end of the line (or a closing brace / `;`)
        let v = value.and_then(|v| {
            let v = v.trim_start();
            let end = v.find(['\n', ';', '}']).unwrap_or(v.len());
            let v = v[..end].trim();
            // `.set(17)` closes one parenthesis more than the value opens
            let v = if v.matches(')').count() > v.matches('(').count() { &v[..v.rfind(')').unwrap_or(v.len())] } else { v };
            // Kotlin's own `languageVersion = "1.9"` is not a Java version
            if key == "languageVersion" && !v.contains("JavaLanguageVersion") {
                return None;
            }
            parse_version(v)
        });
        best = best.max(v);
    }
    best
}

fn strip_comments(text: &str, line: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let l = if line.is_empty() { None } else { rest.find(line) };
        let b = rest.find(open);
        match (l, b) {
            (Some(i), b) if b.is_none_or(|b| i < b) => {
                out.push_str(&rest[..i]);
                rest = rest[i..].find('\n').map(|e| &rest[i + e..]).unwrap_or("");
            }
            (_, Some(i)) => {
                out.push_str(&rest[..i]);
                rest = rest[i + open.len()..].find(close).map(|e| &rest[i + open.len() + e + close.len()..]).unwrap_or("");
            }
            _ => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

/// The text of each `<tag>...</tag>` element (not nested in itself).
fn elements<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = xml[from..].find(&open) {
        let start = from + i + open.len();
        let Some(e) = xml[start..].find(&close) else { break };
        out.push(&xml[start..start + e]);
        from = start + e + close.len();
    }
    out
}

fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    elements(xml, tag).into_iter().next().map(str::trim)
}

/// Child elements `<name>value</name>` of a block, by name.
fn children(block: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = block;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let Some(gt) = rest.find('>') else { break };
        let name = &rest[..gt];
        if name.starts_with('/') || name.ends_with('/') || name.contains(' ') {
            rest = &rest[gt + 1..];
            continue;
        }
        let close = format!("</{name}>");
        let Some(e) = rest[gt + 1..].find(&close) else { break };
        out.insert(name.to_string(), rest[gt + 1..gt + 1 + e].trim().to_string());
        rest = &rest[gt + 1 + e + close.len()..];
    }
    out
}

struct Pom {
    props: HashMap<String, String>,
    /// maven-compiler-plugin configuration (build plugins and plugin management)
    compiler: HashMap<String, String>,
    parent: Option<PathBuf>,
    spring_boot_parent: bool,
}

#[derive(Default)]
struct PomCache {
    poms: HashMap<PathBuf, Option<Pom>>,
}

impl PomCache {
    fn load(&mut self, path: &Path) -> Option<&Pom> {
        let path = path.canonicalize().ok()?;
        if !self.poms.contains_key(&path) {
            let pom = std::fs::read_to_string(&path).ok().map(|t| parse_pom(&t, &path));
            self.poms.insert(path.clone(), pom);
        }
        self.poms.get(&path)?.as_ref()
    }

    /// `key` from the pom's properties or its in-repository parents'.
    fn property(&mut self, path: &Path, key: &str, depth: u32) -> Option<String> {
        let pom = self.load(path)?;
        if let Some(v) = pom.props.get(key) {
            return Some(v.clone());
        }
        let parent = pom.parent.clone()?;
        if depth > 10 {
            return None;
        }
        self.property(&parent, key, depth + 1)
    }

    fn compiler(&mut self, path: &Path, key: &str, depth: u32) -> Option<String> {
        let pom = self.load(path)?;
        if let Some(v) = pom.compiler.get(key) {
            return Some(v.clone());
        }
        let parent = pom.parent.clone()?;
        if depth > 10 {
            return None;
        }
        self.compiler(&parent, key, depth + 1)
    }

    /// Resolve `${name}` references against the pom's properties.
    fn resolve(&mut self, path: &Path, value: &str) -> Option<String> {
        let mut v = value.trim().to_string();
        for _ in 0..10 {
            let Some(i) = v.find("${") else { return Some(v) };
            let e = v[i..].find('}')? + i;
            let name = v[i + 2..e].to_string();
            let sub = self.property(path, &name, 0)?;
            v.replace_range(i..=e, sub.trim());
        }
        None
    }

    fn release(&mut self, path: &Path) -> Option<u32> {
        // the plugin's configuration wins over the property it defaults to
        for (param, prop) in [("release", "maven.compiler.release"), ("target", "maven.compiler.target"), ("source", "maven.compiler.source")] {
            let raw = self.compiler(path, param, 0).or_else(|| self.property(path, prop, 0));
            if let Some(v) = raw.and_then(|r| self.resolve(path, &r)).and_then(|r| parse_version(&r)) {
                return Some(v);
            }
        }
        // Spring Boot's parent compiles for `java.version`
        if self.load(path)?.spring_boot_parent || self.root_has_spring_boot_parent(path) {
            return self.property(path, "java.version", 0).and_then(|r| self.resolve(path, &r)).and_then(|r| parse_version(&r));
        }
        None
    }

    fn root_has_spring_boot_parent(&mut self, path: &Path) -> bool {
        let mut cur = path.to_path_buf();
        for _ in 0..10 {
            let Some(pom) = self.load(&cur) else { return false };
            if pom.spring_boot_parent {
                return true;
            }
            let Some(p) = pom.parent.clone() else { return false };
            cur = p;
        }
        false
    }
}

fn parse_pom(text: &str, path: &Path) -> Pom {
    let text = strip_comments(text, "", "<!--", "-->");
    // profiles switch settings on conditions we cannot evaluate
    let mut main = text.clone();
    while let (Some(s), Some(e)) = (main.find("<profiles>"), main.find("</profiles>")) {
        if e < s {
            break;
        }
        main.replace_range(s..e + "</profiles>".len(), "");
    }
    let props = elements(&main, "properties").into_iter().flat_map(children).collect();
    let mut compiler = HashMap::new();
    for plugin in elements(&main, "plugin") {
        if element(plugin, "artifactId") != Some("maven-compiler-plugin") {
            continue;
        }
        // executions may compile other sources for other releases (a Java 9
        // module-info next to Java 8 code): the plugin-level configuration
        // counts, else the main compile's (`default-compile`)
        let executions = elements(plugin, "executions").into_iter().flat_map(|e| elements(e, "execution")).collect::<Vec<_>>();
        let mut own = plugin.to_string();
        while let (Some(s), Some(e)) = (own.find("<executions>"), own.find("</executions>")) {
            if e < s {
                break;
            }
            own.replace_range(s..e + "</executions>".len(), "");
        }
        let main_compile = executions.iter().filter(|e| element(e, "id") == Some("default-compile")).flat_map(|e| elements(e, "configuration"));
        for conf in elements(&own, "configuration").into_iter().chain(main_compile) {
            for (k, v) in children(conf) {
                if matches!(k.as_str(), "release" | "target" | "source") {
                    compiler.entry(k).or_insert(v);
                }
            }
        }
    }
    let parent_block = element(&main, "parent");
    let parent = parent_block.and_then(|p| {
        let rel = element(p, "relativePath").unwrap_or("../pom.xml");
        if rel.is_empty() {
            return None;
        }
        let p = path.parent()?.join(rel);
        let p = if p.is_dir() { p.join("pom.xml") } else { p };
        p.exists().then_some(p)
    });
    let spring_boot_parent = parent_block.is_some_and(|p| element(p, "groupId") == Some("org.springframework.boot"));
    Pom { props, compiler, parent, spring_boot_parent }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        for (s, v) in [("1.8", 8), ("8", 8), ("\"17\"", 17), ("'11'", 11), ("JavaVersion.VERSION_1_8", 8), ("JavaVersion.VERSION_17", 17), ("JavaVersion.toVersion(\"21\")", 21), ("JavaLanguageVersion.of(17)", 17), ("25", 25)] {
            assert_eq!(parse_version(s), Some(v), "{s}");
        }
        for s in ["JavaVersion.current()", "${java.version}", "javaVersion", ""] {
            assert_eq!(parse_version(s), None, "{s}");
        }
    }

    #[test]
    fn gradle_groovy_and_kotlin() {
        assert_eq!(gradle_release("java {\n    sourceCompatibility = 1.8\n    targetCompatibility = '1.8'\n}\n"), Some(8));
        assert_eq!(gradle_release("sourceCompatibility = JavaVersion.VERSION_17\n"), Some(17));
        assert_eq!(gradle_release("java {\n    toolchain {\n        languageVersion = JavaLanguageVersion.of(21)\n    }\n}\n"), Some(21));
        assert_eq!(gradle_release("java { toolchain { languageVersion.set(JavaLanguageVersion.of(17)) } }"), Some(17));
        assert_eq!(gradle_release("tasks.withType(JavaCompile) {\n    options.release = 11\n}\n"), Some(11));
        assert_eq!(gradle_release("tasks.withType<JavaCompile> { options.release.set(17) }"), Some(17));
        assert_eq!(gradle_release("java.sourceCompatibility = JavaVersion.VERSION_11"), Some(11));
        // release wins over the toolchain that runs javac
        assert_eq!(gradle_release("java { toolchain { languageVersion = JavaLanguageVersion.of(21) } }\ntasks.withType(JavaCompile) { options.release = 8 }"), Some(8));
        assert_eq!(gradle_release("// sourceCompatibility = 1.6\nkotlin { jvmTarget = '1.6' }\n"), None);
        assert_eq!(gradle_release("plugins { id 'java' }"), None);
        assert_eq!(gradle_release("kotlin { compilerOptions { languageVersion = \"1.9\" } }"), None);
    }

    fn temp(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("grounds-release-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (p, text) in files {
            let p = dir.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        dir
    }

    fn infer_dir(dir: &Path) -> Option<u32> {
        infer(&Workspace::discover(dir, &[]))
    }

    #[test]
    fn maven_properties_and_plugin() {
        let dir = temp("props", &[("pom.xml", "<project><properties><maven.compiler.source>1.8</maven.compiler.source><maven.compiler.target>1.8</maven.compiler.target></properties></project>")]);
        assert_eq!(infer_dir(&dir), Some(8));
        let dir = temp(
            "plugin",
            &[(
                "pom.xml",
                "<project><properties><java.version>11</java.version><!-- <maven.compiler.release>8</maven.compiler.release> --></properties>
                 <build><plugins>
                   <plugin><artifactId>build-helper-maven-plugin</artifactId><configuration><source>src/gen</source></configuration></plugin>
                   <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId>
                     <configuration><release>${java.version}</release></configuration></plugin>
                 </plugins></build>
                 <profiles><profile><properties><maven.compiler.release>21</maven.compiler.release></properties></profile></profiles></project>",
            )],
        );
        assert_eq!(infer_dir(&dir), Some(11));
        // the main compile's execution counts, other executions do not; plugin
        // configuration after the executions still counts
        let compiler = |inner: &str| format!("<project><build><plugins><plugin><artifactId>maven-compiler-plugin</artifactId>{inner}</plugin></plugins></build></project>");
        let java9 = "<execution><id>java9</id><configuration><release>9</release></configuration></execution>";
        let dir = temp("exec", &[("pom.xml", &compiler(&format!("<executions><execution><id>default-compile</id><configuration><release>8</release></configuration></execution>{java9}</executions>")))]);
        assert_eq!(infer_dir(&dir), Some(8));
        let dir = temp("exec-after", &[("pom.xml", &compiler(&format!("<executions>{java9}</executions><configuration><source>1.8</source><target>1.8</target></configuration>")))]);
        assert_eq!(infer_dir(&dir), Some(8));
        let dir = temp("exec-only", &[("pom.xml", &compiler(&format!("<executions>{java9}</executions>")))]);
        assert_eq!(infer_dir(&dir), None);
    }

    #[test]
    fn maven_parent_in_repository() {
        let dir = temp(
            "parent",
            &[
                ("pom.xml", "<project><properties><jdk>1.8</jdk><maven.compiler.target>${jdk}</maven.compiler.target></properties><modules><module>core</module></modules></project>"),
                ("core/pom.xml", "<project><parent><artifactId>root</artifactId></parent></project>"),
            ],
        );
        assert_eq!(infer_dir(&dir), Some(8));
        // Spring Boot's parent turns java.version into the release
        let dir = temp("boot", &[("pom.xml", "<project><parent><groupId>org.springframework.boot</groupId><artifactId>spring-boot-starter-parent</artifactId><relativePath/></parent><properties><java.version>17</java.version></properties></project>")]);
        assert_eq!(infer_dir(&dir), Some(17));
        // without it, java.version is just a property
        let dir = temp("plain", &[("pom.xml", "<project><properties><java.version>17</java.version></properties></project>")]);
        assert_eq!(infer_dir(&dir), None);
    }

    #[test]
    fn grounds_toml_wins() {
        let dir = temp("toml", &[("pom.xml", "<project><properties><maven.compiler.release>8</maven.compiler.release></properties></project>"), ("grounds.toml", "java_release = 17\n")]);
        let ws = Workspace::discover(&dir, &[]);
        assert_eq!(crate::config::Config::load(&dir).unwrap().java_release(&ws), 17);
        assert_eq!(crate::config::Config::default().java_release(&ws), 8);
        let empty = temp("default", &[("build.gradle", "")]);
        assert_eq!(crate::config::Config::default().java_release(&Workspace::discover(&empty, &[])), crate::config::DEFAULT_JAVA_RELEASE);
    }

    #[test]
    fn highest_module_release() {
        let dir = temp(
            "modules",
            &[
                ("build.gradle", "subprojects { apply plugin: 'java'; sourceCompatibility = 1.8 }"),
                ("server/build.gradle.kts", "java { toolchain { languageVersion.set(JavaLanguageVersion.of(17)) } }"),
            ],
        );
        assert_eq!(infer_dir(&dir), Some(17));
        let dir = temp("conventions", &[("build.gradle", "plugins { id 'java-conventions' }"), ("buildSrc/src/main/groovy/java-conventions.gradle", "java { sourceCompatibility = JavaVersion.VERSION_11 }")]);
        assert_eq!(infer_dir(&dir), Some(11));
        // projects under a source tree are test data
        let dir = temp("fixture", &[("pom.xml", "<project><properties><maven.compiler.release>11</maven.compiler.release></properties></project>"), ("src/test/resources/it/pom.xml", "<project><properties><maven.compiler.release>25</maven.compiler.release></properties></project>")]);
        assert_eq!(infer_dir(&dir), Some(11));
        let dir = temp("none", &[("build.gradle", "plugins { id 'java' }")]);
        assert_eq!(infer_dir(&dir), None);
    }
}
