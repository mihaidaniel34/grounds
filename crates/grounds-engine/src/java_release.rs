//! The JDK API a project's code compiles against, read from its build files
//! when `grounds.toml` does not set `java_release`: JDK APIs newer than the
//! release must not count (`CharSequence.isEmpty` exists only since Java 15,
//! so a Java 8 class declaring `isEmpty()` overrides nothing), and JDK APIs
//! the code does use must not be missing (`List.getFirst` is Java 21).
//!
//! Each module's build is evidence of one of three kinds ([`Evidence`]):
//! - exact: javac's `--release R` (Maven `<release>`/`maven.compiler.release`,
//!   Gradle `options.release`), or a toolchain of JDK R without it: the code
//!   compiles against R's API and nothing else;
//! - at least: `-source`/`-target` only (`maven.compiler.source`/`target`,
//!   `sourceCompatibility`): javac compiles against the full API of whichever
//!   JDK runs the build, R or newer;
//! - unknown: no setting (Maven's and Gradle's defaults are the JDK running
//!   the build), a value grounds cannot evaluate (a variable, an expression, a
//!   property it cannot resolve, a parent POM outside the repository).
//!
//! A module's tests are compiled by a compile of their own, which the build
//! may give another release ([`Scoped`]): Maven's `testRelease`,
//! `testSource`/`testTarget`; Gradle's `compileTestJava { }`. Its files
//! (`src/test`) take that one, and the other source sets theirs.
//!
//! The code itself is evidence too: a module said to compile for exactly R
//! whose code uses newer syntax (a record before 16, a sealed class before
//! 17) does not, and its API is unknown.
//!
//! One JDK symbol table serves the whole project ([`plan`]): the exact
//! release when every module states one, the installed JDK's (the newest API
//! at hand, so no JDK member the code may use is missing) when some module's
//! is unknown or a lower bound, and a release newer than the installed JDK
//! when some module needs one (then JDK classes may lack members; see
//! `JdkApi::Newer`). A claim that a JDK class or member does not exist in a
//! module's release is only made for modules whose release is exact.

use crate::workspace::{BuildSystem, Workspace};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What a module's build says about the JDK API its code compiles against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence {
    /// `--release R`, or a toolchain of JDK R: exactly R's API.
    Exact(u32),
    /// `-source`/`-target R` only: the API of the JDK running the build, R or newer.
    AtLeast(u32),
    /// No setting, or one grounds cannot evaluate.
    Unknown,
}

impl Evidence {
    /// The lowest release the code may compile for.
    pub fn floor(self) -> Option<u32> {
        match self {
            Evidence::Exact(r) | Evidence::AtLeast(r) => Some(r),
            Evidence::Unknown => None,
        }
    }
}

impl ReleasePlan {
    /// The release a file compiles for exactly, as the index tells it
    /// (`Index::exact_release`): its source set's when that differs, else its
    /// module's (the innermost).
    pub fn exact_release(&self, file: &Path) -> Option<u32> {
        let module = self.exact.iter().filter(|(d, _)| file.starts_with(d)).max_by_key(|(d, _)| d.as_os_str().len());
        let set = self.sets.iter().filter(|(d, _, _)| file.starts_with(d)).max_by_key(|(d, _, _)| d.as_os_str().len());
        match (module, set) {
            (m, Some((d, r, _))) if m.is_none_or(|(m, _)| d.as_os_str().len() > m.as_os_str().len()) => *r,
            (m, _) => m?.1,
        }
    }
}

/// What a module's build says of each of its compiles: the main code's
/// (`src/main`), the tests' (`src/test`), and the other source sets'.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scoped {
    pub main: Evidence,
    pub test: Evidence,
    /// None: the build compiles them with the main code or the tests, which
    /// one is not known (Maven's `build-helper` sources): what both state
    /// when they agree.
    pub other: Option<Evidence>,
}

impl Scoped {
    pub fn all(e: Evidence) -> Scoped {
        Scoped { main: e, test: e, other: Some(e) }
    }

    pub fn get(&self, scope: Scope) -> Evidence {
        let agree = |a: Evidence, b: Evidence| if a == b { a } else { Evidence::Unknown };
        let other = self.other.unwrap_or(agree(self.main, self.test));
        match scope {
            Scope::Main => self.main,
            Scope::Test => self.test,
            Scope::Other => other,
            // a file outside `src/<set>/`: in a source directory the build
            // sets, compiled by one of them
            Scope::Rest => agree(agree(self.main, self.test), other),
        }
    }
}

/// Which of a module's compiles a source file is in, by where it is in the
/// module: `src/main/`, `src/test/`, another `src/<set>/`, or elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Main,
    Test,
    Other,
    Rest,
}

impl Scope {
    pub const ALL: [Scope; 4] = [Scope::Main, Scope::Test, Scope::Other, Scope::Rest];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The scope of `file` in the module at `module`, and the directory of
    /// its source set (`<module>/src/<set>`) when it is in one.
    pub fn of(module: &Path, file: &Path) -> (Scope, Option<PathBuf>) {
        let Ok(rel) = file.strip_prefix(module) else { return (Scope::Rest, None) };
        let comps: Vec<&std::ffi::OsStr> = rel.components().map(|c| c.as_os_str()).collect();
        if comps.len() < 3 || comps[0] != "src" {
            return (Scope::Rest, None);
        }
        let scope = match comps[1].to_str() {
            Some("main") => Scope::Main,
            Some("test") => Scope::Test,
            _ => Scope::Other,
        };
        (scope, Some(module.join("src").join(comps[1])))
    }
}

/// The release whose JDK API is loaded, and the modules whose code compiles
/// against exactly one release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleasePlan {
    pub load: u32,
    /// (module directory, release when exact) for every module: a module
    /// with None stops the lookup of an enclosing module's release.
    /// `missing-jdk-import` judges files of exact modules only. The release
    /// of the module's files outside its source sets (`Scope::Rest`), which
    /// is its main code's when the module's compiles agree.
    pub exact: Vec<(PathBuf, Option<u32>)>,
    /// (module directory, lowest release it may compile for): exact ones and
    /// `source`/`target` lower bounds; None when unknown.
    pub floors: Vec<(PathBuf, Option<u32>)>,
    /// (source set directory, exact release, floor) of the source sets
    /// whose compile states otherwise than the module entry: the tests
    /// compiled for another release than the main code (`testRelease`,
    /// `compileTestJava { options.release }`), or for one grounds cannot read.
    pub sets: Vec<(PathBuf, Option<u32>, Option<u32>)>,
}

/// The evidence of every module of the workspace, in `ws.modules` order.
/// Builds inside a source tree (test fixtures) say nothing about the project.
pub fn module_evidence(ws: &Workspace) -> Vec<Scoped> {
    let mut poms = PomCache::default();
    // Gradle convention plugins (`id("java-conventions")`) and the root build
    // (`subprojects { }`) set it for modules that do not set it themselves
    let mut conventions = GradleSettings::default();
    for dir in ["buildSrc", "build-logic"] {
        let mut scripts = Vec::new();
        gradle_scripts(&ws.root.join(dir), 0, &mut scripts);
        for s in scripts {
            if let Ok(t) = crate::fsutil::read_text(&s) {
                conventions.merge(&GradleSettings::read(&t));
            }
        }
    }
    // the root's `subprojects { }` and `allprojects { }` apply to every
    // module; the rest of the root script to the root project only
    let mut for_subprojects = conventions;
    for f in ["build.gradle", "build.gradle.kts"] {
        if let Ok(t) = crate::fsutil::read_text(ws.root.join(f)) {
            let t = strip_script_comments(&t, false);
            for b in blocks(&t, "subprojects") {
                for_subprojects.merge(&GradleSettings::read(b));
            }
            for b in blocks(&t, "allprojects") {
                for_subprojects.merge(&GradleSettings::read(b));
                conventions.merge(&GradleSettings::read(b));
            }
        }
    }
    // the scripts of the Gradle modules, without their `project(':x') { }`
    // blocks, which are settings of x (merged with x's own: set in both
    // places with different values, the release is not known)
    let mut scripts: Vec<(PathBuf, String)> = Vec::new();
    let mut targeted: Vec<(PathBuf, GradleSettings)> = Vec::new();
    for m in ws.modules.iter().filter(|m| m.build_system == BuildSystem::Gradle) {
        for f in ["build.gradle", "build.gradle.kts"] {
            if let Ok(t) = crate::fsutil::read_text(m.dir.join(f)) {
                let (own, targets) = project_blocks(&strip_script_comments(&t, false));
                for (path, body) in targets {
                    targeted.push((ws.root.join(path), GradleSettings::read(&body)));
                }
                // which modules these reach is the build's business: what they
                // set is unknown for every module below this one (and this
                // one, unless only its subprojects can be reached)
                let (mut own, mut reaching) = reaching_blocks(&own);
                // a nested module's `subprojects { }` and `allprojects { }`
                // likewise (the root's are read as the settings its modules
                // inherit)
                if m.dir != ws.root {
                    for (name, itself) in [("subprojects", false), ("allprojects", true)] {
                        reaching.extend(blocks(&own, name).into_iter().map(|b| (b.to_string(), itself)));
                        own = without_blocks(&own, name);
                    }
                }
                for (body, itself) in reaching {
                    let g = GradleSettings::read(&body).opaque();
                    for d in ws.modules.iter().filter(|o| o.build_system == BuildSystem::Gradle && o.dir.starts_with(&m.dir) && (itself || o.dir != m.dir)) {
                        targeted.push((d.dir.clone(), g));
                    }
                }
                scripts.push((m.dir.clone(), own));
            }
        }
    }
    ws.modules
        .iter()
        .map(|m| {
            if fixture_module(ws, &m.dir) {
                return Scoped::all(Evidence::Unknown);
            }
            match m.build_system {
                BuildSystem::Maven => {
                    let pom = m.dir.join("pom.xml");
                    Scoped { main: poms.evidence(&pom, false), test: poms.evidence(&pom, true), other: None }
                }
                BuildSystem::Gradle => {
                    let mut own = GradleSettings::default();
                    for (_, t) in scripts.iter().filter(|(d, _)| *d == m.dir) {
                        // `subprojects { }` of the root does not apply to the root
                        let t = if m.dir == ws.root { without_blocks(t, "subprojects") } else { t.clone() };
                        own.merge(&GradleSettings::read(&t));
                    }
                    for (_, g) in targeted.iter().filter(|(d, _)| *d == m.dir) {
                        own.merge(g);
                    }
                    own.evidence(if m.dir == ws.root { &conventions } else { &for_subprojects })
                }
                BuildSystem::None => GradleSettings::default().evidence(if m.dir == ws.root { &conventions } else { &for_subprojects }),
            }
        })
        .collect()
}

/// A script's `project(':path') { ... }` blocks, which configure another
/// module: the script without them, and each block's path (`a/b` for
/// `:a:b`) and body.
pub(crate) fn project_blocks(text: &str) -> (String, Vec<(String, String)>) {
    let mut own = String::new();
    let mut found = Vec::new();
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
        found.push((path.trim_start_matches(':').replace(':', "/"), text[open..end].to_string()));
        own.push_str(&text[last..start - "project(".len()]);
        last = end;
        from = end.max(start);
    }
    own.push_str(&text[last.min(text.len())..]);
    (own, found)
}

/// A script's blocks that configure modules chosen by an expression
/// (`configure(subprojects.findAll { ... }) { }`, `subprojects.each { }`,
/// `allprojects.forEach { }`): the script without them, and their bodies.
pub(crate) fn reaching_blocks(text: &str) -> (String, Vec<(String, bool)>) {
    // the index just past the bracket matching the one at `open`
    let close = |open: usize| -> Option<usize> {
        let (o, c) = match text[open..].chars().next()? {
            '(' => ('(', ')'),
            _ => ('{', '}'),
        };
        let mut depth = 0;
        for (j, ch) in text[open..].char_indices() {
            if ch == o {
                depth += 1;
            } else if ch == c {
                depth -= 1;
                if depth == 0 {
                    return Some(open + j + 1);
                }
            }
        }
        None
    };
    let word_before = |i: usize| text[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.');
    let mut cuts: Vec<(usize, usize, String, bool)> = Vec::new();
    let mut from = 0;
    while from < text.len() {
        let next = ["configure(", "subprojects.", "allprojects."].iter().filter_map(|k| text[from..].find(k).map(|i| (from + i, *k))).min();
        let Some((start, key)) = next else { break };
        from = start + key.len();
        if word_before(start) {
            continue;
        }
        // the block's opening brace: after `configure(...)`, or after `.each` / `.forEach(...)`
        let mut at = if key == "configure(" {
            match close(start + key.len() - 1) {
                Some(e) => e,
                None => break,
            }
        } else {
            from + text[from..].find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(text.len() - from)
        };
        let rest = text[at..].trim_start();
        at = text.len() - rest.len();
        if key != "configure(" && rest.starts_with('(') {
            let Some(e) = close(at) else { break };
            at = e + (text[e..].len() - text[e..].trim_start().len());
        }
        if !text[at..].starts_with('{') {
            continue;
        }
        let Some(mut end) = close(at) else { break };
        let mut body = text[at + 1..end - 1].to_string();
        // a chain (`.findAll { }.each { }`, `.matching { }.configureEach { }`)
        // reaches the same modules: every block of it is cut
        loop {
            let rest = text[end..].trim_start();
            let Some(after) = rest.strip_prefix('.') else { break };
            let name = after.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(after.len());
            if name == 0 {
                break;
            }
            let mut next = text.len() - after.len() + name;
            next += text[next..].len() - text[next..].trim_start().len();
            if text[next..].starts_with('(') {
                let Some(e) = close(next) else { break };
                next = e + (text[e..].len() - text[e..].trim_start().len());
            }
            let block = if text[next..].starts_with('{') { close(next) } else { None };
            match block {
                Some(e) => {
                    body.push('\n');
                    body.push_str(&text[next + 1..e - 1]);
                    end = e;
                }
                // `.each(fn)`: no block, the chain ends here
                None => {
                    end = next.max(end);
                    break;
                }
            }
        }
        // (`subprojects.` never reaches the declaring project itself)
        cuts.push((start, end, body, key != "subprojects."));
        from = end;
    }
    let mut own = String::new();
    let mut last = 0;
    for (s, e, _, _) in &cuts {
        own.push_str(&text[last..*s]);
        last = *e;
    }
    own.push_str(&text[last..]);
    (own, cuts.into_iter().map(|(_, _, b, itself)| (b, itself)).collect())
}

/// A script without its `name { ... }` blocks.
pub(crate) fn without_blocks(text: &str, name: &str) -> String {
    let mut out = text.to_string();
    for b in blocks(text, name).into_iter().rev() {
        let start = b.as_ptr() as usize - text.as_ptr() as usize;
        out.replace_range(start..start + b.len(), "");
    }
    out
}

/// The bodies of `name { ... }` blocks in a script.
pub(crate) fn blocks<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find(name) {
        let start = from + i;
        from = start + name.len();
        if text[..start].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
            continue;
        }
        let rest = text[from..].trim_start();
        if !rest.starts_with('{') {
            continue;
        }
        let open = text.len() - rest.len();
        let mut depth = 0;
        for (j, c) in text[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        out.push(&text[open + 1..open + j]);
                        from = open + j;
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn fixture_module(ws: &Workspace, dir: &Path) -> bool {
    dir.strip_prefix(&ws.root).is_ok_and(|r| r.components().any(|c| c.as_os_str() == "src"))
}

/// Which JDK API to load and which modules compile for an exact release.
/// `configured`: `java_release` from grounds.toml, exact for every module.
/// `needs`: (module index, scope, lowest release its code there needs).
/// `installed`: the installed JDK's release.
pub fn plan(ws: &Workspace, configured: Option<u32>, needs: &[(usize, Scope, u32)], installed: Option<u32>) -> ReleasePlan {
    if let Some(r) = configured {
        // every module listed, with the one release: the list is also how a
        // file's module is told from an enclosing module's (`Index::in_module`)
        let all: Vec<(PathBuf, Option<u32>)> = std::iter::once(ws.root.clone()).chain(ws.modules.iter().map(|m| m.dir.clone())).fold(Vec::new(), |mut v, d| {
            if !v.iter().any(|(e, _)| *e == d) {
                v.push((d, Some(r)));
            }
            v
        });
        return ReleasePlan { load: r, exact: all.clone(), floors: all, sets: Vec::new() };
    }
    let mut evidence = module_evidence(ws);
    // the compiles that hold code (an aggregator pom without sources says
    // nothing), and the source set directories of each module
    let mut with_code = vec![[false; 4]; ws.modules.len()];
    let mut set_dirs: Vec<Vec<(Scope, PathBuf)>> = vec![Vec::new(); ws.modules.len()];
    for f in &ws.files {
        let Some(m) = ws.modules.get(f.module) else { continue };
        let (scope, dir) = Scope::of(&m.dir, &f.path);
        if f.kind != crate::workspace::SourceKind::Generated {
            with_code[f.module][scope.index()] = true;
        }
        if let Some(d) = dir {
            if !set_dirs[f.module].iter().any(|(_, e)| *e == d) {
                set_dirs[f.module].push((scope, d));
            }
        }
    }
    let mut need = 0;
    for &(m, scope, n) in needs {
        need = need.max(n);
        if let Some(Evidence::Exact(r)) = evidence.get(m).map(|e| e.get(scope)) {
            if n > r {
                // the code says the release is not the one stated: the build
                // is not read right, for any of the module's compiles
                evidence[m] = Scoped::all(Evidence::Unknown);
            }
        }
    }
    let any_code = with_code.iter().any(|w| w.iter().any(|&c| c));
    // the evidence of each compile that holds code (of every module's main
    // code when there is none): the API loaded serves them all
    let used: Vec<Evidence> = (0..ws.modules.len())
        .filter(|&m| !fixture_module(ws, &ws.modules[m].dir))
        .flat_map(|m| {
            let e = evidence[m];
            let w = with_code[m];
            Scope::ALL.into_iter().filter(move |s| if any_code { w[s.index()] } else { *s == Scope::Main }).map(move |s| e.get(s))
        })
        .collect();
    let need = used.iter().filter_map(|e| e.floor()).fold(need, u32::max);
    let readable = |r: u32| installed.is_none_or(|i| r <= i);
    let exact_of = |m: usize, e: Evidence| match e {
        Evidence::Exact(r) if readable(r) && !fixture_module(ws, &ws.modules[m].dir) => Some(r),
        _ => None,
    };
    let floor_of = |m: usize, e: Evidence| e.floor().filter(|_| !fixture_module(ws, &ws.modules[m].dir));
    let exact: Vec<(PathBuf, Option<u32>)> = evidence.iter().enumerate().map(|(m, e)| (ws.modules[m].dir.clone(), exact_of(m, e.get(Scope::Rest)))).collect();
    let floors = evidence.iter().enumerate().map(|(m, e)| (ws.modules[m].dir.clone(), floor_of(m, e.get(Scope::Rest)))).collect();
    let mut sets = Vec::new();
    for (m, e) in evidence.iter().enumerate() {
        let rest = (exact_of(m, e.get(Scope::Rest)), floor_of(m, e.get(Scope::Rest)));
        for (scope, dir) in &set_dirs[m] {
            let own = (exact_of(m, e.get(*scope)), floor_of(m, e.get(*scope)));
            if own != rest {
                sets.push((dir.clone(), own.0, own.1));
            }
        }
    }
    let all_exact = !used.is_empty() && used.iter().all(|e| matches!(e, Evidence::Exact(_)));
    let load = match installed {
        Some(i) if need > i => need,
        _ if all_exact => used.iter().filter_map(|e| e.floor()).max().unwrap_or(need),
        Some(i) => i,
        None => need.max(crate::config::DEFAULT_JAVA_RELEASE),
    };
    ReleasePlan { load, exact, floors, sets }
}

/// The highest release the project's builds state, if any: the release
/// whose API [`plan`] loads when the installed JDK is not known.
pub fn infer(ws: &Workspace) -> Option<u32> {
    module_evidence(ws).into_iter().flat_map(|e| [Scope::Main, Scope::Test, Scope::Other].map(|s| e.get(s))).filter_map(Evidence::floor).max()
}

fn gradle_scripts(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    gradle_scripts_in(dir, depth, out, &mut Vec::new());
}

/// The Gradle scripts below `dir` that convention plugins are read from, and
/// (`dirs`) the directories looked into, whose times change when a script
/// is added or removed there.
fn gradle_scripts_in(dir: &Path, depth: u32, out: &mut Vec<PathBuf>, dirs: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    dirs.push(dir.to_path_buf());
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if depth < 8 && !matches!(name.as_str(), "build" | ".gradle") {
                gradle_scripts_in(&p, depth + 1, out, dirs);
            }
        } else if name.ends_with(".gradle") || name.ends_with(".gradle.kts") {
            out.push(p);
        }
    }
}

/// What `module_evidence` reads of Gradle's convention plugins besides the
/// modules' build files: the scripts in `buildSrc` and `build-logic`, the
/// directories they were looked for in, and those two directories, whether
/// they exist or not. (For a session, which loads the project again when one
/// of them changes.)
pub fn convention_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in ["buildSrc", "build-logic"] {
        let (mut scripts, mut dirs) = (Vec::new(), Vec::new());
        gradle_scripts_in(&root.join(dir), 0, &mut scripts, &mut dirs);
        out.push(root.join(dir));
        out.extend(scripts);
        out.extend(dirs);
    }
    out
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
    let text = strip_script_comments(text, false);
    let keys: [&[&str]; 4] = [&["options.release", "release"], &["targetCompatibility"], &["sourceCompatibility"], &["languageVersion"]];
    keys.iter().find_map(|names| names.iter().filter_map(|k| gradle_assignments(&text, k)).max())
}

/// One kind of setting in Gradle scripts: the highest literal assigned, and
/// whether some assignment is not a literal (`goVersions.targetJavaVersion as
/// Integer`, `jdkVersions.baseline`, `JavaVersion.current()`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Setting {
    literal: Option<u32>,
    opaque: bool,
    /// Different literals: set per task or under a condition
    /// (`if (project.name == ...) options.release = 11`).
    mixed: bool,
}

impl Setting {
    fn merge(&mut self, o: Setting) {
        self.mixed |= o.mixed || matches!((self.literal, o.literal), (Some(a), Some(b)) if a != b);
        self.literal = self.literal.max(o.literal);
        self.opaque |= o.opaque;
    }
    fn is_set(self) -> bool {
        self.literal.is_some() || self.opaque
    }
}

/// The Java settings of Gradle scripts for one compile task (or for all).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Kinds {
    release: Setting,
    toolchain: Setting,
    compat: Setting,
}

impl Kinds {
    /// The same settings, each one that is set made opaque.
    fn opaque(self) -> Kinds {
        let o = |s: Setting| if s.is_set() { Setting { literal: None, opaque: true, mixed: false } } else { s };
        Kinds { release: o(self.release), toolchain: o(self.toolchain), compat: o(self.compat) }
    }

    fn merge(&mut self, o: &Kinds) {
        self.release.merge(o.release);
        self.toolchain.merge(o.toolchain);
        self.compat.merge(o.compat);
    }

    fn with(mut self, o: &Kinds) -> Kinds {
        self.merge(o);
        self
    }

    /// Each kind of setting from `self`, else from `inherited`.
    fn or(&self, inherited: &Kinds) -> Kinds {
        let pick = |a: Setting, b: Setting| if a.is_set() { a } else { b };
        Kinds { release: pick(self.release, inherited.release), toolchain: pick(self.toolchain, inherited.toolchain), compat: pick(self.compat, inherited.compat) }
    }

    /// `--release` decides the API; without it the toolchain's JDK does;
    /// `sourceCompatibility` alone leaves it to the JDK running Gradle.
    /// Several different values, or a release other than the source level,
    /// are set under conditions or for other compile tasks: unknown.
    fn evidence(&self) -> Evidence {
        if matches!((self.release.literal, self.compat.literal), (Some(r), Some(c)) if r != c) {
            return Evidence::Unknown;
        }
        for (s, exact) in [(self.release, true), (self.toolchain, true), (self.compat, false)] {
            match (s.literal, s.opaque || s.mixed) {
                (_, true) => return Evidence::Unknown,
                (Some(r), false) if exact => return Evidence::Exact(r),
                (Some(r), false) => return Evidence::AtLeast(r),
                (None, false) => {}
            }
        }
        Evidence::Unknown
    }
}

/// The Java settings of Gradle scripts, by the compile tasks they apply to:
/// every one (`java { }`, `tasks.withType(JavaCompile)`, a toolchain), the
/// main compile (`compileJava { }`), the test compile (`compileTestJava { }`)
/// and the compiles of other source sets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct GradleSettings {
    all: Kinds,
    main: Kinds,
    test: Kinds,
    other: Kinds,
}

impl GradleSettings {
    fn read(text: &str) -> GradleSettings {
        let text = strip_script_comments(text, false);
        let mut g = GradleSettings::default();
        for (keys, kind) in [(&["options.release"][..], 0), (&["languageVersion", "jvmToolchain"][..], 1), (&["targetCompatibility", "sourceCompatibility"][..], 2)] {
            for k in keys {
                for (at, literal) in gradle_setting(&text, k) {
                    let s = match literal {
                        Some(r) => Setting { literal: Some(r), ..Setting::default() },
                        None => Setting { opaque: true, ..Setting::default() },
                    };
                    let opaque = Setting { opaque: true, ..Setting::default() };
                    let put = |kinds: &mut Kinds, s: Setting| match kind {
                        0 => kinds.release.merge(s),
                        1 => kinds.toolchain.merge(s),
                        _ => kinds.compat.merge(s),
                    };
                    match task_scope(&text, at) {
                        TaskScope::All => put(&mut g.all, s),
                        TaskScope::Main => put(&mut g.main, s),
                        TaskScope::Test => put(&mut g.test, s),
                        // another source set's compile (`compileJava9Java`):
                        // which files it compiles is the build's business, so
                        // it counts with the main compile's settings (as
                        // conflicting with them, when it differs) and leaves
                        // the other source sets unknown
                        TaskScope::Other => {
                            put(&mut g.main, s);
                            put(&mut g.other, opaque);
                        }
                        // chosen by a condition (`if (name == 'compileTestJava')`)
                        TaskScope::Unreadable => put(&mut g.all, opaque),
                    }
                }
            }
        }
        g
    }

    /// The same settings, each one that is set made opaque.
    fn opaque(self) -> GradleSettings {
        GradleSettings { all: self.all.opaque(), main: self.main.opaque(), test: self.test.opaque(), other: self.other.opaque() }
    }

    fn merge(&mut self, o: &GradleSettings) {
        self.all.merge(&o.all);
        self.main.merge(&o.main);
        self.test.merge(&o.test);
        self.other.merge(&o.other);
    }

    /// What applies to each compile: the settings of every compile and its own.
    fn scopes(&self) -> [Kinds; 3] {
        [self.all.with(&self.main), self.all.with(&self.test), self.all.with(&self.other)]
    }

    /// The evidence of each compile, from the module's own settings
    /// (`self`), else for each kind of setting the `inherited` ones.
    fn evidence(&self, inherited: &GradleSettings) -> Scoped {
        let (own, inh) = (self.scopes(), inherited.scopes());
        let ev = |i: usize| own[i].or(&inh[i]).evidence();
        Scoped { main: ev(0), test: ev(1), other: Some(ev(2)) }
    }
}

/// Which compile tasks a setting in a Gradle script applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskScope {
    All,
    Main,
    Test,
    /// Another source set's compile task.
    Other,
    /// Tasks chosen by a condition grounds does not evaluate.
    Unreadable,
}

/// The compile task a setting at `at` configures: the one a dotted prefix
/// names (`compileTestJava.options.release`), else the one the innermost
/// enclosing block naming a task does (`compileTestJava { }`,
/// `tasks.named('compileTestJava') { }`, `tasks.compileTestJava { }`), else
/// every one.
fn task_scope(text: &str, at: usize) -> TaskScope {
    if let Some(before) = text[..at].strip_suffix('.') {
        if let Some(s) = header_scope(chain_before(before)) {
            return s;
        }
    }
    let mut depth = 0usize;
    for (i, c) in text[..at].char_indices().rev() {
        match c {
            '}' => depth += 1,
            '{' if depth > 0 => depth -= 1,
            '{' => {
                if let Some(s) = header_scope(chain_before(&text[..i])) {
                    return s;
                }
            }
            _ => {}
        }
    }
    TaskScope::All
}

/// The expression that ends `text` (a block's header, or what is before a
/// `.`): identifiers joined by dots, with their arguments, type arguments
/// and closures (`tasks.withType(JavaCompile).matching { ... }.configureEach`,
/// `if (name == 'compileTestJava')`).
fn chain_before(text: &str) -> &str {
    let b = text.as_bytes();
    let mut i = text.trim_end().len();
    let end = i;
    loop {
        let mut grouped = false;
        // arguments, type arguments, closures
        while i > 0 && matches!(b[i - 1], b')' | b'}' | b']' | b'>') {
            let close = b[i - 1];
            let open = match close {
                b')' => b'(',
                b'}' => b'{',
                b']' => b'[',
                _ => b'<',
            };
            let mut depth = 0usize;
            let mut j = i;
            while j > 0 {
                j -= 1;
                if b[j] == close {
                    depth += 1;
                } else if b[j] == open {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            if depth != 0 {
                return &text[i..end];
            }
            i = j;
            grouped = true;
            while i > 0 && b[i - 1].is_ascii_whitespace() {
                i -= 1;
            }
        }
        let ident_end = i;
        while i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') {
            i -= 1;
        }
        if i == ident_end && !grouped {
            break;
        }
        let mut k = i;
        while k > 0 && b[k - 1].is_ascii_whitespace() {
            k -= 1;
        }
        if k > 0 && b[k - 1] == b'.' {
            i = k - 1;
            while i > 0 && b[i - 1].is_ascii_whitespace() {
                i -= 1;
            }
        } else {
            break;
        }
    }
    &text[i..end]
}

/// The compile task a block header or a dotted prefix names, if any.
fn header_scope(header: &str) -> Option<TaskScope> {
    let words: Vec<&str> = header.split(|c: char| !(c.is_alphanumeric() || c == '_')).collect();
    let mut tasks: Vec<&str> = words.iter().copied().filter(|w| w.len() > "compileJava".len() - 1 && w.starts_with("compile") && w.ends_with("Java")).collect();
    tasks.sort_unstable();
    tasks.dedup();
    if tasks.is_empty() {
        return None;
    }
    // a task chosen by a condition, or several tasks
    let conditional = words.iter().any(|w| matches!(*w, "if" | "when" | "unless" | "matching" | "filter" | "findAll" | "find" | "each" | "forEach" | "all" | "takeIf")) || ["==", "!=", "!", "contains", "startsWith", "endsWith"].iter().any(|o| header.contains(o));
    if conditional || tasks.len() > 1 {
        return Some(TaskScope::Unreadable);
    }
    Some(match tasks[0] {
        "compileJava" => TaskScope::Main,
        "compileTestJava" => TaskScope::Test,
        _ => TaskScope::Other,
    })
}

/// The assignments of `key` in a script (comments stripped): where each
/// is, and its value when that is a literal.
fn gradle_setting(text: &str, key: &str) -> Vec<(usize, Option<u32>)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find(key) {
        let start = from + i;
        from = start + key.len();
        if text[..start].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') || text[from..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let rest = text[from..].trim_start();
        let value = if let Some(r) = rest.strip_prefix(".set(").or_else(|| rest.strip_prefix('(')) {
            r
        } else if let Some(r) = rest.strip_prefix('=').filter(|r| !r.starts_with('=')) {
            r
        } else if key.ends_with("Compatibility") && (rest.starts_with(['\'', '"']) || rest.starts_with(|c: char| c.is_ascii_digit()) || rest.starts_with("JavaVersion")) {
            // Groovy's `sourceCompatibility 1.8`
            rest
        } else {
            // read, not set: `if (java.toolchain.languageVersion ...)`
            continue;
        };
        let v = value.trim_start();
        let end = v.find(['\n', ';', '}']).unwrap_or(v.len());
        let v = v[..end].trim();
        let v = if v.matches(')').count() > v.matches('(').count() { v[..v.rfind(')').unwrap_or(v.len())].trim() } else { v };
        // Kotlin's compiler has a `languageVersion` too: it is Java's only
        // in a toolchain (`java { toolchain { } }`, `java.toolchain.`,
        // `jvmToolchain { }`) or when its value is a JavaLanguageVersion
        if key == "languageVersion" && !v.contains("JavaLanguageVersion") && !in_toolchain(text, start) {
            continue;
        }
        out.push((start, parse_version(v)));
    }
    out
}

/// Whether the text at `at` is in a toolchain: a `toolchain.` qualifier
/// or an enclosing `toolchain { }` / `jvmToolchain { }` block.
fn in_toolchain(text: &str, at: usize) -> bool {
    let is_toolchain = |w: &str| w.to_ascii_lowercase().ends_with("toolchain");
    if text[..at].strip_suffix('.').is_some_and(|q| is_toolchain(q.rsplit(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or(""))) {
        return true;
    }
    let mut depth = 0usize;
    for (i, c) in text[..at].char_indices().rev() {
        match c {
            '}' => depth += 1,
            '{' if depth > 0 => depth -= 1,
            '{' => {
                let before = text[..i].trim_end();
                let name = before.rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).next().unwrap_or("");
                if is_toolchain(name.rsplit('.').next().unwrap_or("")) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
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

pub(crate) fn strip_comments(text: &str, line: &str, open: &str, close: &str) -> String {
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

/// A Groovy, Kotlin or TOML script without its comments (`//` and `/* */`,
/// or `#` with `hash`), leaving string literals alone: `'**/*.class'` and
/// `"https://..."` open no comment.
pub(crate) fn strip_script_comments(text: &str, hash: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == q || c == '\n' {
                quote = None;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"' | '\'', _) => {
                quote = Some(c);
                out.push(c);
            }
            ('/', Some('/')) if !hash => {
                while chars.peek().is_some_and(|&n| n != '\n') {
                    chars.next();
                }
            }
            ('#', _) if hash => {
                while chars.peek().is_some_and(|&n| n != '\n') {
                    chars.next();
                }
            }
            ('/', Some('*')) if !hash => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    if n == '\n' {
                        out.push('\n');
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// The text of each `<tag>...</tag>` element (not nested in itself).
pub(crate) fn elements<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
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

pub(crate) fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
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
    /// maven-compiler-plugin configuration (build plugins and plugin
    /// management) of the main compile: the plugin's, under its
    /// `default-compile` execution's
    compiler: HashMap<String, String>,
    /// The same of the test compile: the plugin's, under its
    /// `default-testCompile` execution's.
    test_compiler: HashMap<String, String>,
    /// Another execution of the `testCompile` goal sets a release, source or
    /// target: which tests compile for which release is not known.
    other_test_compile: bool,
    parent: Option<PathBuf>,
    /// The major version of a Spring Boot parent (0 when unread).
    spring_boot_parent: Option<u32>,
    /// A profile configures the compiler plugin's release, source or
    /// target: what applies depends on conditions grounds cannot evaluate
    /// (the JDK running Maven, a flag).
    profile_configures_compiler: bool,
    /// The properties profiles set: a value read from one of them is unknown.
    profile_props: Vec<String>,
}

#[derive(Default)]
pub(crate) struct PomCache {
    poms: HashMap<PathBuf, Option<Pom>>,
}

impl PomCache {
    fn load(&mut self, path: &Path) -> Option<&Pom> {
        let path = path.canonicalize().ok()?;
        if !self.poms.contains_key(&path) {
            let pom = crate::fsutil::read_text(&path).ok().map(|t| parse_pom(&t, &path));
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

    /// The compiler plugin's `key` for the main compile (for the test
    /// compile with `test`), from the pom or its in-repository parents.
    fn compiler(&mut self, path: &Path, key: &str, test: bool, depth: u32) -> Option<String> {
        let pom = self.load(path)?;
        if let Some(v) = if test { &pom.test_compiler } else { &pom.compiler }.get(key) {
            return Some(v.clone());
        }
        let parent = pom.parent.clone()?;
        if depth > 10 {
            return None;
        }
        self.compiler(&parent, key, test, depth + 1)
    }

    /// Whether resolving `value` reads one of `props`, directly or through
    /// other properties.
    fn reads(&mut self, path: &Path, value: &str, props: &[String]) -> bool {
        let mut pending = vec![value.to_string()];
        for _ in 0..20 {
            let Some(v) = pending.pop() else { return false };
            let mut rest = v.as_str();
            while let Some(i) = rest.find("${") {
                let Some(e) = rest[i..].find('}') else { break };
                let name = &rest[i + 2..i + e];
                if props.iter().any(|p| p == name) {
                    return true;
                }
                if let Some(sub) = self.property(path, name, 0) {
                    pending.push(sub);
                }
                rest = &rest[i + e..];
            }
        }
        false
    }

    /// The in-repository parent pom of a pom.
    pub(crate) fn parent_of(&mut self, path: &Path) -> Option<PathBuf> {
        self.load(path)?.parent.clone()
    }

    /// Resolve `${name}` references against the pom's properties.
    pub(crate) fn resolve(&mut self, path: &Path, value: &str) -> Option<String> {
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

    /// The evidence of a pom with its parents in the repository, for its
    /// main compile or (`test`) its test compile. The test compile's release
    /// is `testRelease` (`maven.compiler.testRelease`), else `release`; its
    /// source and target `testSource`/`testTarget`, else `source`/`target`
    /// (maven-compiler-plugin's `TestCompilerMojo`).
    fn evidence(&mut self, path: &Path, test: bool) -> Evidence {
        // a profile of the pom or a parent in the repository may set it otherwise
        let mut profile_props: Vec<String> = Vec::new();
        let mut cur = Some(path.to_path_buf());
        for _ in 0..10 {
            let Some(p) = cur.take() else { break };
            let Some(pom) = self.load(&p) else { break };
            if pom.profile_configures_compiler || (test && pom.other_test_compile) {
                return Evidence::Unknown;
            }
            profile_props.extend(pom.profile_props.iter().cloned());
            cur = pom.parent.clone();
        }
        // a value a profile may change: the property itself, or one it reads
        let value = |c: &mut PomCache, param: &str, prop: &str| -> Option<Option<u32>> {
            let raw = match c.compiler(path, param, test, 0) {
                Some(raw) => raw,
                None if profile_props.iter().any(|p| p == prop) => return Some(None),
                None => c.property(path, prop, 0)?,
            };
            if c.reads(path, &raw, &profile_props) {
                return Some(None);
            }
            Some(c.resolve(path, &raw).and_then(|r| parse_version(&r)))
        };
        // the plugin's configuration wins over the property it defaults to;
        // the test compile's own parameter over the main one's
        let release = match test {
            true => value(self, "testRelease", "maven.compiler.testRelease").or_else(|| value(self, "release", "maven.compiler.release")),
            false => value(self, "release", "maven.compiler.release"),
        };
        match release {
            Some(Some(r)) => return Evidence::Exact(r),
            Some(None) => return Evidence::Unknown,
            None => {}
        }
        let source_target = match test {
            true => [
                value(self, "testTarget", "maven.compiler.testTarget").or_else(|| value(self, "target", "maven.compiler.target")),
                value(self, "testSource", "maven.compiler.testSource").or_else(|| value(self, "source", "maven.compiler.source")),
            ],
            false => [value(self, "target", "maven.compiler.target"), value(self, "source", "maven.compiler.source")],
        };
        if source_target.contains(&Some(None)) {
            return Evidence::Unknown;
        }
        if let Some(r) = source_target.iter().flatten().flatten().max() {
            return Evidence::AtLeast(*r);
        }
        // Spring Boot's parent compiles for `java.version`: with `--release`
        // since Boot 3, with source and target before
        if let Some(major) = self.spring_boot_parent(path) {
            let raw = self.property(path, "java.version", 0).unwrap_or_default();
            if profile_props.iter().any(|p| p == "java.version") || self.reads(path, &raw, &profile_props) {
                return Evidence::Unknown;
            }
            return match self.property(path, "java.version", 0).and_then(|r| self.resolve(path, &r)).and_then(|r| parse_version(&r)) {
                Some(r) if major >= 3 => Evidence::Exact(r),
                Some(r) => Evidence::AtLeast(r),
                None => Evidence::Unknown,
            };
        }
        Evidence::Unknown
    }

    /// The major version of the Spring Boot parent of the pom or of an
    /// ancestor in the repository (0 when it cannot be read).
    fn spring_boot_parent(&mut self, path: &Path) -> Option<u32> {
        let mut cur = path.to_path_buf();
        for _ in 0..10 {
            let pom = self.load(&cur)?;
            if let Some(v) = pom.spring_boot_parent {
                return Some(v);
            }
            cur = pom.parent.clone()?;
        }
        None
    }

    #[cfg(test)]
    fn release(&mut self, path: &Path) -> Option<u32> {
        // the plugin's configuration wins over the property it defaults to
        for (param, prop) in [("release", "maven.compiler.release"), ("target", "maven.compiler.target"), ("source", "maven.compiler.source")] {
            let raw = self.compiler(path, param, false, 0).or_else(|| self.property(path, prop, 0));
            if let Some(v) = raw.and_then(|r| self.resolve(path, &r)).and_then(|r| parse_version(&r)) {
                return Some(v);
            }
        }
        // Spring Boot's parent compiles for `java.version`
        if self.spring_boot_parent(path).is_some() {
            return self.property(path, "java.version", 0).and_then(|r| self.resolve(path, &r)).and_then(|r| parse_version(&r));
        }
        None
    }
}

fn parse_pom(text: &str, path: &Path) -> Pom {
    let text = strip_comments(text, "", "<!--", "-->");
    // profiles switch settings on conditions we cannot evaluate
    let profiles = elements(&text, "profiles");
    let profile_configures_compiler = profiles.iter().flat_map(|p| elements(p, "plugin")).any(|pl| {
        element(pl, "artifactId") == Some("maven-compiler-plugin")
            && (["<release>", "<source>", "<target>", "<testRelease>", "<testSource>", "<testTarget>"].iter().any(|k| pl.contains(k)) || elements(pl, "compilerArgs").iter().any(|a| ["--release", "-release", "-source", "-target", "--source", "--target"].iter().any(|f| a.contains(&format!(">{f}<")) || a.contains(&format!(">{f} ")) || a.contains(&format!(">{f}=")))))
    });
    let profile_props = profiles.iter().flat_map(|p| elements(p, "properties")).flat_map(children).map(|(k, _)| k).collect();
    let mut main = text.clone();
    while let (Some(s), Some(e)) = (main.find("<profiles>"), main.find("</profiles>")) {
        if e < s {
            break;
        }
        main.replace_range(s..e + "</profiles>".len(), "");
    }
    let props = elements(&main, "properties").into_iter().flat_map(children).collect();
    let mut compiler = HashMap::new();
    let mut test_compiler = HashMap::new();
    let mut other_test_compile = false;
    for plugin in elements(&main, "plugin") {
        if element(plugin, "artifactId") != Some("maven-compiler-plugin") {
            continue;
        }
        // executions may compile other sources for other releases (a Java 9
        // module-info next to Java 8 code): only the plugin-level
        // configuration counts, merged under the main compile's
        // (`default-compile`), whose settings win as in Maven's merge
        let executions = elements(plugin, "executions").into_iter().flat_map(|e| elements(e, "execution")).collect::<Vec<_>>();
        let mut own = plugin.to_string();
        while let (Some(s), Some(e)) = (own.find("<executions>"), own.find("</executions>")) {
            if e < s {
                break;
            }
            own.replace_range(s..e + "</executions>".len(), "");
        }
        let settings = |conf: &str| children(conf).into_iter().filter(|(k, _)| matches!(k.as_str(), "release" | "target" | "source" | "testRelease" | "testTarget" | "testSource")).collect::<Vec<_>>();
        for conf in elements(&own, "configuration") {
            for (k, v) in settings(conf) {
                compiler.entry(k.clone()).or_insert(v.clone());
                test_compiler.entry(k).or_insert(v);
            }
        }
        for e in &executions {
            let id = element(e, "id");
            let set = elements(e, "configuration").into_iter().flat_map(settings).collect::<Vec<_>>();
            match id {
                Some("default-compile") => compiler.extend(set),
                Some("default-testCompile") => test_compiler.extend(set),
                // another compile of tests (`<goal>testCompile</goal>`)
                _ if !set.is_empty() && elements(e, "goal").iter().any(|g| g.trim() == "testCompile") => other_test_compile = true,
                _ => {}
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
    let spring_boot_parent = parent_block.filter(|p| element(p, "groupId") == Some("org.springframework.boot")).map(|p| element(p, "version").and_then(|v| v.split('.').next()?.parse().ok()).unwrap_or(0));
    Pom { props, compiler, test_compiler, other_test_compile, parent, spring_boot_parent, profile_configures_compiler, profile_props }
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
                 <profiles><profile><properties><skipTests>true</skipTests></properties></profile></profiles></project>",
            )],
        );
        assert_eq!(infer_dir(&dir), Some(11));
        // the main compile's execution counts, other executions do not; plugin
        // configuration after the executions still counts
        let compiler = |inner: &str| format!("<project><build><plugins><plugin><artifactId>maven-compiler-plugin</artifactId>{inner}</plugin></plugins></build></project>");
        let java9 = "<execution><id>java9</id><configuration><release>9</release></configuration></execution>";
        let dir = temp("exec", &[("pom.xml", &compiler(&format!("<executions><execution><id>default-compile</id><configuration><release>8</release></configuration></execution>{java9}</executions>")))]);
        assert_eq!(infer_dir(&dir), Some(8));
        // the main compile's settings override the plugin level's, which
        // still is the test compile's
        let dir = temp("exec-wins", &[("pom.xml", &compiler("<configuration><release>17</release></configuration><executions><execution><id>default-compile</id><configuration><release>11</release></configuration></execution></executions>"))]);
        assert_eq!(module_evidence(&Workspace::discover(&dir, &[])), vec![Scoped { main: Evidence::Exact(11), test: Evidence::Exact(17), other: None }]);
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
        // without any setting: the installed JDK's API, else the default; the
        // installed release is given here so that the test does not depend on the machine
        let ws = Workspace::discover(&empty, &[]);
        assert_eq!(plan(&ws, None, &[], None).load, crate::config::DEFAULT_JAVA_RELEASE);
        assert_eq!(plan(&ws, None, &[], Some(25)).load, 25);
        // grounds.toml's release wins whatever JDK is installed, newer or older
        let ws = Workspace::discover(&dir, &[]);
        for installed in [None, Some(11), Some(25)] {
            assert_eq!(plan(&ws, Some(17), &[], installed).load, 17, "{installed:?}");
        }
    }

    fn evidence_of(dir: &Path) -> Vec<(String, Evidence)> {
        let ws = Workspace::discover(dir, &[]);
        let ev = module_evidence(&ws);
        ws.modules.iter().zip(ev).map(|(m, e)| (m.name.clone(), e.main)).collect()
    }

    #[test]
    fn what_each_module_states() {
        use Evidence::*;
        // gocd: the root's release is an expression, two plugin modules state 11
        let dir = temp(
            "gocd",
            &[
                ("build.gradle", "subprojects {\n  tasks.withType(JavaCompile).configureEach {\n    options.release.set(goVersions.targetJavaVersion as Integer)\n  }\n}\n"),
                ("server/build.gradle", "dependencies { implementation 'x:y:1' }\n"),
                ("plugin-api/build.gradle", "compileJava {\n  options.release.set(11)\n}\n"),
            ],
        );
        assert_eq!(evidence_of(&dir), vec![(":".into(), Unknown), ("plugin-api".into(), Exact(11)), ("server".into(), Unknown)]);
        // checker-framework: --release 11 for some projects, -source 17 for the others
        let dir = temp(
            "conditional",
            &[
                ("build.gradle", "allprojects {\n  tasks.withType(JavaCompile).configureEach {\n    if (project.name.equals(\"qual\")) {\n      options.release = 11\n    } else {\n      sourceCompatibility = 17\n      targetCompatibility = 17\n    }\n  }\n}\n"),
                ("checker/build.gradle", "dependencies { }\n"),
                ("multi/build.gradle", "compileJava { options.release = 8 }\ncompileJava9Java { options.release = 9 }\n"),
                ("same/build.gradle", "java { sourceCompatibility = 17 }\ncompileJava { options.release = 17 }\n"),
            ],
        );
        assert_eq!(evidence_of(&dir), vec![(":".into(), Unknown), ("checker".into(), Unknown), ("multi".into(), Unknown), ("same".into(), Exact(17))]);
        // different source levels for the main and the test compile: each its own
        let dir = temp("mixed-compat", &[("build.gradle", "compileJava { sourceCompatibility = 11 }\ncompileTestJava { sourceCompatibility = 17 }\n")]);
        assert_eq!(module_evidence(&Workspace::discover(&dir, &[])), vec![Scoped { main: AtLeast(11), test: AtLeast(17), other: Some(Unknown) }]);
        // Maven: a profile that sets the release (activated by the JDK, a flag)
        let dir = temp(
            "profile",
            &[
                ("pom.xml", "<project><properties><maven.compiler.release>11</maven.compiler.release></properties><profiles><profile><id>jdk17</id><activation><jdk>[17,)</jdk></activation><properties><maven.compiler.release>17</maven.compiler.release></properties></profile></profiles><modules><module>a</module></modules></project>"),
                ("a/pom.xml", "<project><parent><artifactId>root</artifactId></parent></project>"),
                ("b/pom.xml", "<project><properties><maven.compiler.release>11</maven.compiler.release></properties><profiles><profile><id>docs</id><properties><skipTests>true</skipTests></properties></profile></profiles></project>"),
            ],
        );
        assert_eq!(evidence_of(&dir), vec![(":".into(), Unknown), ("a".into(), Unknown), ("b".into(), Exact(11))]);
        // a literal release on the compiler plugin wins over a profile's
        // property; a profile that sets what the plugin reads, or configures
        // the plugin itself, still decides
        let plugin = |conf: &str| format!("<build><plugins><plugin><artifactId>maven-compiler-plugin</artifactId><configuration>{conf}</configuration></plugin></plugins></build>");
        let profile = |inner: &str| format!("<profiles><profile><id>p</id>{inner}</profile></profiles>");
        let dir = temp(
            "profile-plugin",
            &[
                ("literal/pom.xml", &format!("<project>{}{}</project>", plugin("<release>17</release>"), profile("<properties><maven.compiler.release>21</maven.compiler.release></properties>"))),
                ("read/pom.xml", &format!("<project><properties><jdk>17</jdk></properties>{}{}</project>", plugin("<release>${jdk}</release>"), profile("<properties><jdk>21</jdk></properties>"))),
                ("configures/pom.xml", &format!("<project>{}{}</project>", plugin("<release>17</release>"), profile(&plugin("<release>21</release>")))),
                ("args/pom.xml", &format!("<project>{}{}</project>", plugin("<release>17</release>"), profile(&plugin("<compilerArgs><arg>--release</arg><arg>21</arg></compilerArgs>")))),
                ("checker/pom.xml", &format!("<project>{}{}</project>", plugin("<release>17</release>"), profile(&plugin("<compilerArgs><arg>-XDcompilePolicy=simple</arg><arg>-Xplugin:Checker</arg></compilerArgs>")))),
            ],
        );
        assert_eq!(
            evidence_of(&dir),
            vec![("args".into(), Unknown), ("checker".into(), Exact(17)), ("configures".into(), Unknown), ("literal".into(), Exact(17)), ("read".into(), Unknown), (":".into(), Unknown)]
        );
        // a toolchain decides the API unless a release is set; Kotlin's languageVersion is not Java's
        let dir = temp(
            "toolchain",
            &[
                ("build.gradle.kts", "java { toolchain { languageVersion.set(JavaLanguageVersion.of(17)) } }\nkotlin { compilerOptions { languageVersion = KOTLIN_2_1 } }\n"),
                ("compat/build.gradle", "java { sourceCompatibility = JavaVersion.VERSION_11 }\n"),
                ("current/build.gradle", "java { sourceCompatibility = JavaVersion.current() }\n"),
                ("release/build.gradle", "java { toolchain { languageVersion = JavaLanguageVersion.of(21) } }\ntasks.withType(JavaCompile) { options.release = 8 }\n"),
                ("kotlin/build.gradle.kts", "kotlin { jvmToolchain(21) }\n"),
            ],
        );
        assert_eq!(evidence_of(&dir), vec![(":".into(), Exact(17)), ("compat".into(), AtLeast(11)), ("current".into(), Unknown), ("kotlin".into(), Exact(21)), ("release".into(), Exact(8))]);
        // a Java toolchain set through a variable: not a literal, and not Kotlin's
        let dir = temp(
            "toolchain-variable",
            &[
                ("build.gradle", "subprojects { java { toolchain { languageVersion = JavaLanguageVersion.of(17) } } }\n"),
                ("child/build.gradle", "def selectedVersion = JavaLanguageVersion.of(21)\njava { toolchain { languageVersion.set(selectedVersion) } }\n"),
            ],
        );
        assert_eq!(evidence_of(&dir), vec![(":".into(), Unknown), ("child".into(), Unknown)]);
        // Kotlin's languageVersion set through a variable is still Kotlin's:
        // only a toolchain block (or JavaLanguageVersion) makes it Java's
        let dir = temp(
            "kotlin-variable",
            &[
                ("build.gradle.kts", "subprojects { java { toolchain { languageVersion = JavaLanguageVersion.of(11) } } }\njava { toolchain { languageVersion.set(JavaLanguageVersion.of(17)) } }\nkotlin {\n  compilerOptions {\n    languageVersion.set(kotlinLanguage)\n    apiVersion.set(kotlinLanguage)\n  }\n}\n"),
                ("dotted/build.gradle.kts", "java.toolchain.languageVersion.set(javaLanguage)\n"),
                ("jvm/build.gradle.kts", "kotlin { jvmToolchain { languageVersion.set(javaLanguage) } }\n"),
            ],
        );
        assert_eq!(evidence_of(&dir), vec![(":".into(), Exact(17)), ("dotted".into(), Unknown), ("jvm".into(), Unknown)]);
        // a root's `project(':x') { }` blocks configure x, not the root; a
        // module set both there and in its own script is set twice
        let dir = temp(
            "project-blocks",
            &[
                ("build.gradle", "project(':lib') {\n  compileJava { options.release = 11 }\n}\nproject(':plugin') {\n  compileJava { options.release = 11 }\n}\n"),
                ("lib/build.gradle", "dependencies { }\n"),
                ("plugin/build.gradle", "compileJava { options.release = 17 }\n"),
            ],
        );
        assert_eq!(evidence_of(&dir), vec![(":".into(), Unknown), ("lib".into(), Exact(11)), ("plugin".into(), Unknown)]);
        // `configure(...) { }` and `subprojects.each { }` reach modules no
        // reader can list: not the script's own settings, and every module
        // below it that could be reached is unknown for what they set
        let dir = temp(
            "configure-blocks",
            &[
                ("build.gradle", "configure(subprojects.findAll { it.name != 'docs' }) {\n  compileJava { options.release = 11 }\n}\n"),
                ("app/build.gradle", "compileJava { options.release = 17 }\n"),
                ("each/build.gradle", "subprojects.each { p -> p.compileJava { options.release = 8 } }\n"),
                ("each/lib/build.gradle", "dependencies { }\n"),
            ],
        );
        assert_eq!(evidence_of(&dir), vec![(":".into(), Unknown), ("app".into(), Unknown), ("each".into(), Unknown), ("each/lib".into(), Unknown)]);
        // a chain after `subprojects.`: every block of it reaches the subprojects
        let dir = temp("chained-blocks", &[("build.gradle", "java { toolchain { languageVersion = JavaLanguageVersion.of(21) } }\nsubprojects.findAll { it.name != 'docs' }.each { p -> p.compileJava { options.release = 11 } }\n"), ("app/build.gradle", "dependencies { }\n")]);
        assert_eq!(evidence_of(&dir), vec![(":".into(), Exact(21)), ("app".into(), Unknown)]);
        // `subprojects { }` and `allprojects { }` of a nested module's script
        // are not its own settings: they reach the modules below it (and
        // itself, for `allprojects`)
        let dir = temp(
            "nested-blocks",
            &[
                ("build.gradle", "java { toolchain { languageVersion = JavaLanguageVersion.of(21) } }\n"),
                ("group/build.gradle", "java { toolchain { languageVersion = JavaLanguageVersion.of(17) } }\nsubprojects { compileJava { options.release = 8 } }\n"),
                ("group/lib/build.gradle", "dependencies { }\n"),
                ("other/build.gradle", "allprojects { compileJava { options.release = 11 } }\n"),
                ("other/lib/build.gradle", "dependencies { }\n"),
            ],
        );
        assert_eq!(evidence_of(&dir), vec![(":".into(), Exact(21)), ("group".into(), Exact(17)), ("group/lib".into(), Unknown), ("other".into(), Unknown), ("other/lib".into(), Unknown)]);
        // Maven: --release is exact, source/target a lower bound, an
        // unresolvable property or a parent outside the repository unknown
        let pom = |inner: &str| format!("<project>{inner}</project>");
        let dir = temp(
            "maven",
            &[
                ("pom.xml", &pom("<properties><jdk>17</jdk></properties><modules><module>a</module></modules>")),
                ("a/pom.xml", &pom("<parent><artifactId>root</artifactId></parent><properties><maven.compiler.release>${jdk}</maven.compiler.release></properties>")),
                ("b/pom.xml", &pom("<properties><maven.compiler.source>1.8</maven.compiler.source><maven.compiler.target>1.8</maven.compiler.target></properties>")),
                ("c/pom.xml", &pom("<properties><maven.compiler.release>${java.level}</maven.compiler.release></properties>")),
                ("d/pom.xml", &pom("<parent><groupId>org.jenkins-ci</groupId><artifactId>jenkins</artifactId><version>1.0</version><relativePath/></parent>")),
                ("e/pom.xml", &pom("<parent><groupId>org.springframework.boot</groupId><artifactId>spring-boot-starter-parent</artifactId><version>3.2.0</version><relativePath/></parent><properties><java.version>17</java.version></properties>")),
                ("f/pom.xml", &pom("<parent><groupId>org.springframework.boot</groupId><artifactId>spring-boot-starter-parent</artifactId><version>2.7.0</version><relativePath/></parent><properties><java.version>11</java.version></properties>")),
            ],
        );
        assert_eq!(
            evidence_of(&dir),
            vec![(":".into(), Unknown), ("a".into(), Exact(17)), ("b".into(), AtLeast(8)), ("c".into(), Unknown), ("d".into(), Unknown), ("e".into(), Exact(17)), ("f".into(), AtLeast(11))]
        );
    }

    #[test]
    fn what_the_test_compile_states() {
        use Evidence::*;
        let scoped = |dir: &Path| {
            let ws = Workspace::discover(dir, &[]);
            ws.modules.iter().zip(module_evidence(&ws)).map(|(m, e)| (m.name.clone(), e.main, e.test)).collect::<Vec<_>>()
        };
        let pom = |inner: &str| format!("<project>{inner}</project>");
        let plugin = |inner: &str| format!("<build><plugins><plugin><artifactId>maven-compiler-plugin</artifactId>{inner}</plugin></plugins></build>");
        let dir = temp(
            "maven-test",
            &[
                ("pom.xml", &pom("<properties><java.version>8</java.version><test.java.version>25</test.java.version></properties><modules><module>a</module></modules>")),
                // prometheus client_java: the root's plugin configuration, read through properties
                ("a/pom.xml", &pom(&format!("<parent><artifactId>root</artifactId></parent>{}", plugin("<configuration><release>${java.version}</release><testRelease>${test.java.version}</testRelease></configuration>")))),
                // testSource/testTarget without a release: a lower bound of their own
                ("b/pom.xml", &pom("<properties><maven.compiler.source>8</maven.compiler.source><maven.compiler.target>8</maven.compiler.target><maven.compiler.testSource>17</maven.compiler.testSource><maven.compiler.testTarget>17</maven.compiler.testTarget></properties>")),
                // with a release, javac takes --release and not the test source level
                ("c/pom.xml", &pom("<properties><maven.compiler.release>11</maven.compiler.release><maven.compiler.testSource>17</maven.compiler.testSource></properties>")),
                // a test release grounds cannot resolve
                ("d/pom.xml", &pom("<properties><maven.compiler.release>11</maven.compiler.release><maven.compiler.testRelease>${tests.jdk}</maven.compiler.testRelease></properties>")),
                // a profile sets the test release
                ("e/pom.xml", &pom(&format!("<properties><maven.compiler.release>11</maven.compiler.release></properties><profiles><profile><id>p</id>{}</profile></profiles>", plugin("<configuration><testRelease>21</testRelease></configuration>")))),
                // another execution compiles tests for another release
                ("f/pom.xml", &pom(&plugin("<configuration><release>11</release></configuration><executions><execution><id>java21-tests</id><goals><goal>testCompile</goal></goals><configuration><release>21</release></configuration></execution></executions>"))),
                // the main compile's execution is not the test compile's
                ("g/pom.xml", &pom(&plugin("<executions><execution><id>default-compile</id><configuration><release>11</release></configuration></execution></executions>"))),
                // a property a profile sets
                ("h/pom.xml", &pom("<properties><maven.compiler.release>11</maven.compiler.release><maven.compiler.testRelease>17</maven.compiler.testRelease></properties><profiles><profile><id>p</id><properties><maven.compiler.testRelease>21</maven.compiler.testRelease></properties></profile></profiles>")),
            ],
        );
        assert_eq!(
            scoped(&dir),
            vec![
                (":".into(), Unknown, Unknown),
                ("a".into(), Exact(8), Exact(25)),
                ("b".into(), AtLeast(8), AtLeast(17)),
                ("c".into(), Exact(11), Exact(11)),
                ("d".into(), Exact(11), Unknown),
                ("e".into(), Unknown, Unknown),
                ("f".into(), Exact(11), Unknown),
                ("g".into(), Exact(11), Unknown),
                ("h".into(), Exact(11), Unknown),
            ]
        );
        let dir = temp(
            "gradle-test",
            &[
                ("build.gradle", "subprojects { java { toolchain { languageVersion = JavaLanguageVersion.of(17) } } }\n"),
                // a dotted assignment, and a release of the test compile over the inherited toolchain
                ("dotted/build.gradle", "compileJava.options.release = 8\ncompileTestJava.options.release = 21\n"),
                ("inherits/build.gradle.kts", "tasks.named<JavaCompile>(\"compileTestJava\") { options.release.set(21) }\n"),
                ("kotlin-tasks/build.gradle.kts", "tasks {\n  compileJava { options.release = 11 }\n  compileTestJava {\n    options.release = 21\n  }\n}\n"),
                // chosen by a condition: neither is known
                ("conditional/build.gradle", "tasks.withType(JavaCompile).configureEach {\n  if (name == 'compileTestJava') {\n    options.release = 21\n  }\n}\n"),
                ("matching/build.gradle.kts", "tasks.withType<JavaCompile>()\n  .matching { it.name == \"compileTestJava\" }\n  .configureEach { options.release = 21 }\n"),
                // a toolchain the test compile's own launcher sets
                ("launcher/build.gradle", "compileJava { options.release = 11 }\ncompileTestJava {\n  javaCompiler = javaToolchains.compilerFor { languageVersion = JavaLanguageVersion.of(21) }\n}\n"),
                // an expression for the test compile
                ("opaque/build.gradle", "compileJava { options.release = 11 }\ncompileTestJava { options.release = testJavaVersion as Integer }\n"),
            ],
        );
        assert_eq!(
            scoped(&dir),
            vec![
                (":".into(), Unknown, Unknown),
                ("conditional".into(), Unknown, Unknown),
                ("dotted".into(), Exact(8), Exact(21)),
                ("inherits".into(), Exact(17), Exact(21)),
                ("kotlin-tasks".into(), Exact(11), Exact(21)),
                ("launcher".into(), Exact(11), Exact(21)),
                ("matching".into(), Unknown, Unknown),
                ("opaque".into(), Exact(11), Unknown),
            ]
        );
    }

    #[test]
    fn test_sources_take_the_test_compiles_release() {
        let dir = temp(
            "plan-tests",
            &[
                ("pom.xml", "<project><properties><maven.compiler.release>8</maven.compiler.release><maven.compiler.testRelease>17</maven.compiler.testRelease></properties><modules><module>a</module><module>b</module></modules></project>"),
                ("a/pom.xml", "<project><parent><artifactId>root</artifactId></parent></project>"),
                ("a/src/main/java/A.java", "class A {}\n"),
                ("a/src/test/java/ATest.java", "class ATest {}\n"),
                // no test code: Java 8 and 17 both, with nothing for 17
                ("b/pom.xml", "<project><parent><artifactId>root</artifactId></parent></project>"),
                ("b/src/main/java/B.java", "class B {}\n"),
            ],
        );
        let ws = Workspace::discover(&dir, &[]);
        let m = |name: &str| ws.modules.iter().position(|m| m.name == name).unwrap();
        let p = plan(&ws, None, &[], Some(21));
        // the API loaded serves the tests' release too
        assert_eq!(p.load, 17);
        let at = |rel: &str| p.exact_release(&dir.join(rel));
        assert_eq!((at("a/src/main/java/A.java"), at("a/src/test/java/ATest.java"), at("a/A.java"), at("b/src/main/java/B.java")), (Some(8), Some(17), None, Some(8)));
        // a record in tests said to compile for 8 would be no news; in main code it is
        let p = plan(&ws, None, &[(m("a"), Scope::Test, 16)], Some(21));
        assert_eq!(p.exact_release(&dir.join("a/src/test/java/ATest.java")), Some(17));
        let p = plan(&ws, None, &[(m("a"), Scope::Main, 16)], Some(21));
        assert_eq!((p.exact_release(&dir.join("a/src/main/java/A.java")), p.exact_release(&dir.join("a/src/test/java/ATest.java"))), (None, None));
    }

    #[test]
    fn the_api_loaded_and_the_exact_modules() {
        let dir = temp(
            "plan",
            &[
                ("build.gradle", "subprojects { apply plugin: 'java' }\n"),
                ("a/build.gradle", "compileJava { options.release = 11 }\n"),
                ("a/src/main/java/A.java", "class A {}\n"),
                ("b/build.gradle", "compileJava { options.release = 17 }\n"),
                ("b/src/main/java/B.java", "class B {}\n"),
                ("c/build.gradle", "java { sourceCompatibility = 11 }\n"),
                ("c/src/main/java/C.java", "class C {}\n"),
            ],
        );
        let ws = Workspace::discover(&dir, &[]);
        let m = |name: &str| ws.modules.iter().position(|m| m.name == name).unwrap();
        // the release of each module's main code
        let exact = |p: &ReleasePlan| p.exact.iter().filter_map(|(d, _)| Some((d.strip_prefix(&ws.root).unwrap().to_string_lossy().to_string(), p.exact_release(&d.join("src/main/java/X.java"))?))).collect::<Vec<_>>();
        // a module on a lower bound: the installed JDK's API; the exact modules stay exact
        let p = plan(&ws, None, &[], Some(21));
        assert_eq!((p.load, exact(&p)), (21, vec![("a".to_string(), 11), ("b".to_string(), 17)]));
        // a module's code needs more than the installed JDK
        assert_eq!(plan(&ws, None, &[(m("c"), Scope::Main, 25)], Some(21)).load, 25);
        // a record in a module said to compile for 11: its release is not exact
        let p = plan(&ws, None, &[(m("a"), Scope::Main, 16)], Some(21));
        assert_eq!(exact(&p), vec![("b".to_string(), 17)]);
        // grounds.toml decides for every module
        let p = plan(&ws, Some(17), &[], Some(21));
        assert_eq!((p.load, p.exact.len(), p.floors.len()), (17, 4, 4));
        assert!(p.exact.iter().chain(&p.floors).all(|(_, r)| *r == Some(17)));
        // every module exact: the highest of them
        std::fs::write(dir.join("c/build.gradle"), "compileJava { options.release = 8 }\n").unwrap();
        let ws = Workspace::discover(&dir, &[]);
        assert_eq!(plan(&ws, None, &[], Some(21)).load, 17);
        // a release newer than the installed JDK
        std::fs::write(dir.join("c/build.gradle"), "compileJava { options.release = 25 }\n").unwrap();
        let ws = Workspace::discover(&dir, &[]);
        let p = plan(&ws, None, &[], Some(21));
        assert_eq!((p.load, exact(&p).len()), (25, 2));
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
