//! The build picture on small projects compiled here with javac: which files
//! the build output shows the build compiles, that no claim that code does
//! not compile is made in a file it does not, and how a finding says which
//! picture it was made in. Skipped, saying so, where no javac is found.

use grounds_engine::analyze::{analyze_files, build_ruleset, Finding};
use grounds_engine::Project;
use grounds_index::InBuild;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

/// javac from the JDK grounds finds, else from PATH.
fn javac() -> Option<PathBuf> {
    let home = grounds_classfile::find_java_home().map(|h| h.join("bin/javac"));
    home.into_iter().chain([PathBuf::from("javac")]).find(|c| Command::new(c).arg("-version").env_remove("JAVA_TOOL_OPTIONS").output().is_ok_and(|o| o.status.success()))
}

macro_rules! javac_or_skip {
    () => {
        match javac() {
            Some(j) => j,
            None => {
                eprintln!("skipped: no javac (set JAVA_HOME or put javac on PATH)");
                return;
            }
        }
    };
}

const POM: &str = "<project><modelVersion>4.0.0</modelVersion><groupId>t</groupId><artifactId>t</artifactId><version>1</version></project>\n";
const GRADLE: &str = "plugins { id 'java' }\n";

fn ago(secs: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(secs)
}

fn touch(p: &Path, t: SystemTime) {
    std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
}

/// Write `text` at `rel` with time `t`.
fn write_at(dir: &Path, rel: &str, text: &str, t: SystemTime) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, text).unwrap();
    touch(&p, t);
}

/// A project of `files`, written two minutes ago, as sources are before a
/// build.
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grounds-picture-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // (no library caches: the result is the same on every machine)
    write_at(&dir, "grounds.toml", "java_release = 17\n[classpath]\nscan_caches = false\n", ago(120));
    for (rel, text) in files {
        write_at(&dir, rel, text, ago(120));
    }
    dir.canonicalize().unwrap()
}

/// Compile `sources` (relative to `dir`) into `dir/out` with `classpath`, as
/// a build does: the class files a minute old, newer than the sources.
fn compile(javac: &Path, dir: &Path, sources: &[&str], out: &str, classpath: Option<&str>) {
    let mut cmd = Command::new(javac);
    cmd.arg("--release").arg("17").arg("-d").arg(dir.join(out)).args(sources.iter().map(|s| dir.join(s))).env_remove("JAVA_TOOL_OPTIONS");
    if let Some(cp) = classpath {
        cmd.arg("-cp").arg(dir.join(cp));
    }
    let o = cmd.output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for c in walk(&dir.join(out)) {
        touch(&c, ago(60));
    }
}

fn walk(d: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// The compile-break findings of `grounds check` on the project, as
/// (path, build state).
fn breaks(p: &Project) -> Vec<(String, Option<InBuild>)> {
    findings(p).into_iter().map(|f| (f.path, f.build)).collect()
}

fn findings(p: &Project) -> Vec<Finding> {
    let (set, options) = build_ruleset(p);
    let files: Vec<PathBuf> = p.workspace.files.iter().map(|f| f.path.clone()).collect();
    analyze_files(p, &set, &options, &files).into_iter().filter(|f| grounds_rules::meta_of(f.rule).is_some_and(|m| m.absence)).collect()
}

fn state(p: &Project, rel: &str) -> InBuild {
    p.index.in_build(&p.workspace.root.join(rel).to_string_lossy())
}

const LIB: &str = "package app;\npublic class Lib { public void run() {} }\n";
const USE: &str = "package app;\nclass Use { void f(Lib l) { l.run(); } }\n";
/// The same file, after an edit that broke it.
const USE_BROKEN: &str = "package app;\nclass Use { void f(Lib l) { l.missing(); } }\n";
const BROKEN: &str = "package app;\nclass Broken { void f(Lib l) { l.missing(); } }\n";

/// A built module with test data that does not compile: no compile-break
/// claim in the test data, while the same broken call in real code (edited
/// since the build, in a root the build compiles) is reported, in the build.
#[test]
fn test_data_of_a_built_module_gets_no_compile_break_claim() {
    let javac = javac_or_skip!();
    let dir = project("data", &[("pom.xml", POM), ("src/main/java/app/Lib.java", LIB), ("src/main/java/app/Use.java", USE), ("src/test/data/app/Broken.java", BROKEN), ("src/test/java/app/LibTest.java", "package app;\nclass LibTest { void t(Lib l) { l.gone(); } }\n")]);
    compile(&javac, &dir, &["src/main/java/app/Lib.java", "src/main/java/app/Use.java"], "target/classes", None);
    // edited after the build, and a file created after it: both in the root the build compiles
    write_at(&dir, "src/main/java/app/Use.java", USE_BROKEN, SystemTime::now());
    write_at(&dir, "src/main/java/app/Created.java", "package app;\nclass Created { void f(Lib l) { l.absent(); } }\n", SystemTime::now());
    let p = Project::load(&dir).unwrap();
    assert_eq!(state(&p, "src/test/data/app/Broken.java"), InBuild::No);
    assert_eq!(state(&p, "src/main/java/app/Lib.java"), InBuild::Yes);
    assert_eq!(state(&p, "src/main/java/app/Created.java"), InBuild::Yes);
    // the tests' own set was never compiled (or failed to): unknown, not out of the build
    assert_eq!(state(&p, "src/test/java/app/LibTest.java"), InBuild::Unknown);
    let got = breaks(&p);
    assert_eq!(got, [("src/main/java/app/Created.java".to_string(), Some(InBuild::Yes)), ("src/main/java/app/Use.java".to_string(), Some(InBuild::Yes)), ("src/test/java/app/LibTest.java".to_string(), Some(InBuild::Unknown))]);
    let v: serde_json::Value = serde_json::from_str(&grounds_engine::report::json(&findings(&p))).unwrap();
    assert_eq!(v[0]["build"], "in the build");
    assert_eq!(v[2]["build"], "unknown");
    let sarif: serde_json::Value = serde_json::from_str(&grounds_engine::report::sarif(&findings(&p), &dir, &[])).unwrap();
    assert_eq!(sarif["runs"][0]["results"][0]["properties"]["build"], "in the build");
    // one guessed module's line for the run, counting the one finding made under it
    let w = grounds_engine::picture::guessed(&p, &findings(&p)).unwrap().to_string();
    assert!(w.contains("guessed for 1 module(s): 1 compile-break finding(s)"), "{w}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A module never built: findings as before, made under a guessed picture,
/// and one line on stderr for the run saying so.
#[test]
fn a_module_never_built_is_judged_as_before_and_says_so() {
    let dir = project("unbuilt", &[("pom.xml", POM), ("src/main/java/app/Lib.java", LIB), ("src/main/java/app/Use.java", USE_BROKEN), ("src/test/data/app/Broken.java", BROKEN)]);
    let p = Project::load(&dir).unwrap();
    assert_eq!(state(&p, "src/test/data/app/Broken.java"), InBuild::Unknown);
    assert_eq!(breaks(&p), [("src/main/java/app/Use.java".to_string(), Some(InBuild::Unknown)), ("src/test/data/app/Broken.java".to_string(), Some(InBuild::Unknown))]);
    let v: serde_json::Value = serde_json::from_str(&grounds_engine::report::json(&findings(&p))).unwrap();
    assert_eq!(v[0]["build"], "unknown");
    let w = grounds_engine::picture::guessed(&p, &findings(&p)).unwrap().to_string();
    assert_eq!(w, "grounds: warning: the build picture is guessed for 1 module(s): 2 compile-break finding(s) made there are not covered by the stable promise. Build the project so that its class files show grounds which files the build compiles (silence: `[classpath] warn = false`)");
    // off with the classpath warnings
    write_at(&dir, "grounds.toml", "java_release = 17\n[classpath]\nscan_caches = false\nwarn = false\n", SystemTime::now());
    let p = Project::load(&dir).unwrap();
    assert!(grounds_engine::picture::guessed(&p, &findings(&p)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file newer than the build in a root no class file is of: a source
/// root added since the build, maybe; unknown, and judged as before.
#[test]
fn a_file_newer_than_the_build_in_a_root_without_class_files_is_unknown() {
    let javac = javac_or_skip!();
    let dir = project("newer", &[("pom.xml", POM), ("src/main/java/app/Lib.java", LIB), ("src/main/java/app/Use.java", USE)]);
    compile(&javac, &dir, &["src/main/java/app/Lib.java", "src/main/java/app/Use.java"], "target/classes", None);
    write_at(&dir, "src/extra/app/Fresh.java", "package app;\nclass Fresh { void f(Lib l) { l.missing(); } }\n", SystemTime::now());
    let p = Project::load(&dir).unwrap();
    assert_eq!(state(&p, "src/extra/app/Fresh.java"), InBuild::Unknown);
    assert_eq!(breaks(&p), [("src/extra/app/Fresh.java".to_string(), Some(InBuild::Unknown))]);
    // the same file older than the build was there when it ran, and not compiled
    touch(&dir.join("src/extra/app/Fresh.java"), ago(120));
    let p = Project::load(&dir).unwrap();
    assert_eq!(state(&p, "src/extra/app/Fresh.java"), InBuild::No);
    assert!(breaks(&p).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two modules, one built: the built one's test data is out of the build,
/// the other module is unknown throughout. A file belongs to its nearest
/// module, whose output decides.
#[test]
fn a_module_not_built_stays_unknown_beside_one_built() {
    let javac = javac_or_skip!();
    let dir = project(
        "two",
        &[
            ("pom.xml", POM),
            ("a/pom.xml", POM),
            ("a/src/main/java/app/Lib.java", LIB),
            ("a/src/test/data/app/Broken.java", BROKEN),
            ("b/pom.xml", POM),
            ("b/src/main/java/bpp/Lib.java", &LIB.replace("package app", "package bpp")),
            ("b/src/main/java/bpp/Use.java", &USE_BROKEN.replace("package app", "package bpp")),
            ("b/src/test/data/bpp/Broken.java", &BROKEN.replace("package app", "package bpp")),
        ],
    );
    compile(&javac, &dir, &["a/src/main/java/app/Lib.java"], "a/target/classes", None);
    let p = Project::load(&dir).unwrap();
    assert_eq!(state(&p, "a/src/main/java/app/Lib.java"), InBuild::Yes);
    assert_eq!(state(&p, "a/src/test/data/app/Broken.java"), InBuild::No);
    assert_eq!(state(&p, "b/src/main/java/bpp/Use.java"), InBuild::Unknown);
    assert_eq!(state(&p, "b/src/test/data/bpp/Broken.java"), InBuild::Unknown);
    assert_eq!(breaks(&p), [("b/src/main/java/bpp/Use.java".to_string(), Some(InBuild::Unknown)), ("b/src/test/data/bpp/Broken.java".to_string(), Some(InBuild::Unknown))]);
    assert_eq!(p.picture.guessed_modules(&p.workspace), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Gradle's layout: one directory per language and source set.
#[test]
fn gradle_output_is_evidence_per_source_set() {
    let javac = javac_or_skip!();
    let dir = project("gradle", &[("build.gradle", GRADLE), ("src/main/java/app/Lib.java", LIB), ("src/test/java/app/LibTest.java", "package app;\nclass LibTest { void t(Lib l) { l.run(); } }\n"), ("src/test/resources-java/app/Broken.java", BROKEN), ("src/integrationTest/java/app/It.java", "package app;\nclass It { void t(Lib l) { l.gone(); } }\n")]);
    compile(&javac, &dir, &["src/main/java/app/Lib.java"], "build/classes/java/main", None);
    compile(&javac, &dir, &["src/test/java/app/LibTest.java"], "build/classes/java/test", Some("build/classes/java/main"));
    let p = Project::load(&dir).unwrap();
    assert_eq!(state(&p, "src/main/java/app/Lib.java"), InBuild::Yes);
    assert_eq!(state(&p, "src/test/java/app/LibTest.java"), InBuild::Yes);
    assert_eq!(state(&p, "src/test/resources-java/app/Broken.java"), InBuild::No);
    // a source set with no output of its own: never compiled here
    assert_eq!(state(&p, "src/integrationTest/java/app/It.java"), InBuild::Unknown);
    assert_eq!(breaks(&p), [("src/integrationTest/java/app/It.java".to_string(), Some(InBuild::Unknown))]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Maven's layout, main and tests, with generated sources compiled with the
/// main code from below the output tree.
#[test]
fn maven_output_is_evidence_for_main_tests_and_generated_sources() {
    let javac = javac_or_skip!();
    let dir = project(
        "maven",
        &[
            ("pom.xml", POM),
            ("src/main/java/app/Lib.java", LIB),
            ("target/generated-sources/annotations/app/Gen.java", "package app;\nclass Gen { void f(Lib l) { l.run(); } }\n"),
            ("src/test/java/app/LibTest.java", "package app;\nclass LibTest { void t(Lib l) { l.run(); } }\n"),
            ("src/test/files/non-compiling/app/Broken.java", BROKEN),
        ],
    );
    compile(&javac, &dir, &["src/main/java/app/Lib.java", "target/generated-sources/annotations/app/Gen.java"], "target/classes", None);
    compile(&javac, &dir, &["src/test/java/app/LibTest.java"], "target/test-classes", Some("target/classes"));
    let p = Project::load(&dir).unwrap();
    assert_eq!(state(&p, "src/main/java/app/Lib.java"), InBuild::Yes);
    assert_eq!(state(&p, "target/generated-sources/annotations/app/Gen.java"), InBuild::Yes);
    assert_eq!(state(&p, "src/test/java/app/LibTest.java"), InBuild::Yes);
    assert_eq!(state(&p, "src/test/files/non-compiling/app/Broken.java"), InBuild::No);
    assert!(breaks(&p).is_empty());
    // test data copying a class of the module is no evidence for itself
    write_at(&dir, "src/test/files/copy/app/Lib.java", LIB, ago(120));
    let p = Project::load(&dir).unwrap();
    assert_eq!(state(&p, "src/test/files/copy/app/Lib.java"), InBuild::No);
    assert_eq!(state(&p, "src/main/java/app/Lib.java"), InBuild::Yes);
    // the output gone (`mvn clean`): nothing to go by
    std::fs::remove_dir_all(dir.join("target/classes")).unwrap();
    std::fs::remove_dir_all(dir.join("target/test-classes")).unwrap();
    let p = Project::load(&dir).unwrap();
    assert_eq!(state(&p, "src/test/files/non-compiling/app/Broken.java"), InBuild::Unknown);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A session follows a build that runs in it, and answers as a fresh run
/// would: build output that appears, class files rewritten in place (which
/// move no directory's time), and output deleted.
#[test]
fn a_session_follows_a_build_that_runs_while_it_is_open() {
    use grounds_engine::session::{CheckOptions, Mode, Session};
    let javac = javac_or_skip!();
    let dir = project("session", &[("pom.xml", POM), ("src/main/java/app/Lib.java", LIB), ("src/main/java/app/Use.java", USE_BROKEN), ("src/test/data/app/Broken.java", BROKEN)]);
    let files: Vec<PathBuf> = ["src/main/java/app/Use.java", "src/test/data/app/Broken.java", "src/extra/app/Fresh.java"].iter().map(|f| dir.join(f)).collect();
    let opts = || CheckOptions { mode: Mode::Checkpoint, files: files.clone(), all_lines: true, use_baseline: false, min_severity: grounds_rules::Severity::Info, base: None };
    let got = |s: &mut Session| -> Vec<(String, Option<InBuild>)> {
        let mut v: Vec<_> = s.check(&opts()).unwrap().findings.into_iter().filter(|f| f.rule == "unresolved-method").map(|f| (f.path, f.build)).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    let fresh = |dir: &Path| -> Vec<(String, Option<InBuild>)> { got(&mut Session::open(dir).unwrap()) };
    let mut s = Session::open(&dir).unwrap();
    let unbuilt = vec![("src/main/java/app/Use.java".to_string(), Some(InBuild::Unknown)), ("src/test/data/app/Broken.java".to_string(), Some(InBuild::Unknown))];
    assert_eq!(got(&mut s), unbuilt);
    // the agent builds (main code only: `Use` no longer compiles)
    compile(&javac, &dir, &["src/main/java/app/Lib.java"], "target/classes", None);
    let built = vec![("src/main/java/app/Use.java".to_string(), Some(InBuild::Yes))];
    assert_eq!(got(&mut s), built);
    assert_eq!(fresh(&dir), built);
    // a source root added since the build, newer than its class files
    write_at(&dir, "src/extra/app/Fresh.java", "package app;\nclass Fresh { void f(Lib l) { l.missing(); } }\n", ago(30));
    let with_fresh = vec![("src/extra/app/Fresh.java".to_string(), Some(InBuild::Unknown)), ("src/main/java/app/Use.java".to_string(), Some(InBuild::Yes))];
    assert_eq!(got(&mut s), with_fresh);
    assert_eq!(fresh(&dir), with_fresh);
    // built again, its class files rewritten in place: the new root was not compiled
    for c in walk(&dir.join("target/classes")) {
        touch(&c, ago(10));
    }
    assert_eq!(got(&mut s), built);
    assert_eq!(fresh(&dir), built);
    // `mvn clean`: nothing to go by again
    std::fs::remove_dir_all(dir.join("target")).unwrap();
    let cleaned = vec![("src/extra/app/Fresh.java".to_string(), Some(InBuild::Unknown)), ("src/main/java/app/Use.java".to_string(), Some(InBuild::Unknown)), ("src/test/data/app/Broken.java".to_string(), Some(InBuild::Unknown))];
    assert_eq!(got(&mut s), cleaned);
    assert_eq!(fresh(&dir), cleaned);
    let _ = std::fs::remove_dir_all(&dir);
}

/// javaparser's shape: the symbol solver keeps an old snapshot of the core
/// module's sources as test data, so `Node` is declared twice. The snapshot's
/// own code is never compiled and gets no claim (it would be read against
/// today's `Node`); real code of another module that calls the real `Node`
/// wrongly is still judged, the snapshot's copy notwithstanding.
#[test]
fn a_snapshot_of_the_sources_is_no_rival_of_the_class_the_build_compiles() {
    let javac = javac_or_skip!();
    let node = "package p;\npublic class Node { public void visit(int i) {} }\n";
    let old_node = "package p;\npublic class Node { public void visit(String s) {} }\n";
    let call = |pkg: &str, arg: &str| format!("package {pkg};\nimport p.Node;\nclass Use {{ void f(Node n) {{ n.visit({arg}); }} }}\n");
    let dir = project(
        "snapshot",
        &[
            ("pom.xml", POM),
            ("core/pom.xml", POM),
            ("core/src/main/java/p/Node.java", node),
            ("solver/pom.xml", POM),
            ("solver/src/main/java/s/Use.java", &call("s", "1")),
            ("solver/src/test/test_sourcecode/core/p/Node.java", old_node),
            // compiles against the snapshot's `Node`, not today's
            ("solver/src/test/test_sourcecode/solver/s/Use.java", &call("s", "\"x\"")),
        ],
    );
    compile(&javac, &dir, &["core/src/main/java/p/Node.java"], "core/target/classes", None);
    compile(&javac, &dir, &["solver/src/main/java/s/Use.java"], "solver/target/classes", Some("core/target/classes"));
    // the real code broken since the build
    write_at(&dir, "solver/src/main/java/s/Use.java", &call("s", "\"x\""), SystemTime::now());
    let p = Project::load(&dir).unwrap();
    assert_eq!(state(&p, "solver/src/test/test_sourcecode/core/p/Node.java"), InBuild::No);
    assert_eq!(state(&p, "solver/src/test/test_sourcecode/solver/s/Use.java"), InBuild::No);
    assert_eq!(state(&p, "solver/src/main/java/s/Use.java"), InBuild::Yes);
    let got: Vec<(String, String, Option<InBuild>)> = findings(&p).into_iter().map(|f| (f.path, f.message, f.build)).collect();
    assert_eq!(got, [("solver/src/main/java/s/Use.java".to_string(), "No `visit` in `Node` accepts (String); available: visit(int)".to_string(), Some(InBuild::Yes))]);
    // never built: two copies of `Node` and no way to tell which is compiled,
    // so no claim either way, as before
    std::fs::remove_dir_all(dir.join("core/target")).unwrap();
    std::fs::remove_dir_all(dir.join("solver/target")).unwrap();
    let p = Project::load(&dir).unwrap();
    assert!(findings(&p).is_empty(), "{:?}", findings(&p));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A build compiles directory trees, and a package other than the
/// directories is legal Java: a file whose package does not spell its
/// directories belongs to the root enclosing it. In a compiled
/// `src/main/java`, a default-package file and a compact source file (top-level
/// methods, an implicit class) in its subdirectories are in the build; in a
/// sample directory the build does not compile, such files are not in the
/// build with the rest of it (sonar-java's `src/main/files/non-compiling`).
#[test]
fn a_file_whose_package_is_not_its_directories_belongs_to_the_enclosing_root() {
    let javac = javac_or_skip!();
    let dir = project(
        "mismatch",
        &[
            ("pom.xml", POM),
            ("src/main/java/app/Lib.java", LIB),
            ("src/main/java/checks/Default.java", "class Default {\n    void f(Default d) { d.nope(); }\n}\n"),
            ("src/main/java/checks/mainSignature/CompactSource.java", "void main() {\n    System.out.println(\"compact source\");\n}\n"),
            ("src/main/java/checks/Elsewhere.java", "package somewhere.other;\nclass Elsewhere {}\n"),
            ("src/main/files/non-compiling/checks/Sample.java", "package checks;\nclass Sample { void f(Sample s) { s.gone(); } }\n"),
            ("src/main/files/non-compiling/checks/Bare.java", "class Bare { void f(Bare b) { b.gone(); } }\n"),
            ("src/main/files/non-compiling/other/Wrong.java", "package wrong;\nclass Wrong { void f(Wrong w) { w.gone(); } }\n"),
        ],
    );
    compile(&javac, &dir, &["src/main/java/app/Lib.java", "src/main/java/checks/Elsewhere.java"], "target/classes", None);
    let p = Project::load(&dir).unwrap();
    for f in ["src/main/java/app/Lib.java", "src/main/java/checks/Default.java", "src/main/java/checks/mainSignature/CompactSource.java", "src/main/java/checks/Elsewhere.java"] {
        assert_eq!(state(&p, f), InBuild::Yes, "{f}");
    }
    for f in ["src/main/files/non-compiling/checks/Sample.java", "src/main/files/non-compiling/checks/Bare.java", "src/main/files/non-compiling/other/Wrong.java"] {
        assert_eq!(state(&p, f), InBuild::No, "{f}");
    }
    assert_eq!(breaks(&p), [("src/main/java/checks/Default.java".to_string(), Some(InBuild::Yes))]);
    let _ = std::fs::remove_dir_all(&dir);
}
