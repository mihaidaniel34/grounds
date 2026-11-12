//! `grounds selfcheck` on small projects compiled here with javac: what it
//! compares, what it leaves out as edited since the build, what it lists as
//! possibly stale, and what it says when there is no build output. Skipped,
//! saying so, where no javac is found.

use grounds_engine::selfcheck::{run, Report};
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

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A project of `files` with `build` as its build file, its times a minute
/// in the past, as sources are before a build.
fn project(name: &str, build: (&str, &str), files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grounds-selfcheck-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, build.0, build.1);
    // (no library caches: the result is the same on every machine)
    write(&dir, "grounds.toml", "java_release = 17\n[classpath]\nscan_caches = false\n");
    for (rel, text) in files {
        write(&dir, rel, text);
        touch(&dir.join(rel), SystemTime::now() - Duration::from_secs(120));
    }
    dir.canonicalize().unwrap()
}

const MAVEN: (&str, &str) = ("pom.xml", "<project><modelVersion>4.0.0</modelVersion><groupId>t</groupId><artifactId>t</artifactId><version>1</version></project>\n");
const GRADLE: (&str, &str) = ("build.gradle", "plugins { id 'java' }\n");

fn touch(p: &Path, t: SystemTime) {
    std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
}

/// Compile the Java files below `dir/src` into `dir/out` (with `classpath`),
/// as a build does: the class files a little newer than the sources.
fn compile(javac: &Path, dir: &Path, src: &str, out: &str, classpath: Option<&str>) {
    let sources: Vec<PathBuf> = walk(&dir.join(src)).into_iter().filter(|p| p.extension().is_some_and(|x| x == "java")).collect();
    let mut cmd = Command::new(javac);
    cmd.arg("--release").arg("17").arg("-d").arg(dir.join(out)).args(&sources).env_remove("JAVA_TOOL_OPTIONS");
    if let Some(cp) = classpath {
        cmd.arg("-cp").arg(dir.join(cp));
    }
    let o = cmd.output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for c in walk(&dir.join(out)) {
        touch(&c, SystemTime::now() - Duration::from_secs(60));
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

fn check(dir: &Path) -> Report {
    run(dir).unwrap()
}

/// Overloads, a field access, a constructor call, a lambda and a method
/// reference, in main and test sources.
const SMALL: &[(&str, &str)] = &[
    (
        "src/main/java/demo/Shapes.java",
        "package demo;\n\
         import java.util.ArrayList;\n\
         import java.util.List;\n\
         public class Shapes {\n\
         \x20   int count;\n\
         \x20   final List<String> names = new ArrayList<>();\n\
         \x20   public Shapes() { this(0); }\n\
         \x20   public Shapes(int count) { this.count = count; }\n\
         \x20   public void add(Object o) { count++; }\n\
         \x20   public void add(String s) { names.add(s); count++; }\n\
         \x20   public void add(int n) { count += n; }\n\
         \x20   public Runnable later(String s) { return () -> add(s); }\n\
         \x20   public List<String> upper() { List<String> out = new ArrayList<>(); names.forEach(n -> out.add(n.toUpperCase())); return out; }\n\
         }\n",
    ),
    (
        "src/main/java/demo/Use.java",
        "package demo;\n\
         class Use {\n\
         \x20   int run() {\n\
         \x20       Shapes s = new Shapes(2);\n\
         \x20       s.add(\"circle\");\n\
         \x20       s.add(3);\n\
         \x20       s.add(new Object());\n\
         \x20       s.later(\"square\").run();\n\
         \x20       return s.count + s.upper().size();\n\
         \x20   }\n\
         }\n",
    ),
    ("src/test/java/demo/ShapesTest.java", "package demo;\nclass ShapesTest {\n    void count() {\n        Shapes s = new Shapes();\n        s.add(\"x\");\n        if (s.count != 1) throw new AssertionError(s.count);\n    }\n}\n"),
];

fn maven_build(javac: &Path, dir: &Path) {
    compile(javac, dir, "src/main/java", "target/classes", None);
    compile(javac, dir, "src/test/java", "target/test-classes", Some("target/classes"));
}

#[test]
fn everything_agrees_on_a_small_project() {
    let javac = javac_or_skip!();
    let dir = project("agree", MAVEN, SMALL);
    maven_build(&javac, &dir);
    let r = check(&dir);
    let text = r.text();
    assert!(r.found.is_empty(), "{text}");
    let [calls, ctors, fields, _, members] = r.counts;
    assert!(calls.compared >= 8 && calls.agree == calls.compared, "{text}");
    assert!(ctors.compared >= 4 && ctors.agree == ctors.compared, "{text}");
    assert!(fields.compared >= 4 && fields.agree == fields.compared, "{text}");
    assert!(members.compared >= 10 && members.agree == members.compared, "{text}");
    assert_eq!(r.outcome(), grounds_engine::daemon::Outcome::Clean);
    assert_eq!(r.edited_since_build(), 0);
    assert!(text.contains("main: src/main/java -> target/classes: 2 file(s) compared\n"), "{text}");
    assert!(text.contains("test: src/test/java -> target/test-classes: 1 file(s) compared\n"), "{text}");
    assert!(text.contains("no disagreement"), "{text}");
    assert!(text.trim_end().ends_with("please report it with the line shown."), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_edited_after_the_build_is_not_compared() {
    let javac = javac_or_skip!();
    let dir = project("edited", MAVEN, SMALL);
    maven_build(&javac, &dir);
    // a body edit: the declarations, and so the other files, are as built
    let use_ = dir.join("src/main/java/demo/Use.java");
    let text = std::fs::read_to_string(&use_).unwrap().replace("s.add(3);", "s.add(4);");
    std::fs::write(&use_, text).unwrap();
    let r = check(&dir);
    let text = r.text();
    let main = &r.modules[0].sets[0];
    assert_eq!((main.compared, main.edited.clone()), (1, vec![PathBuf::from("src/main/java/demo/Use.java")]), "{text}");
    assert!(text.contains("src/main/java/demo/Use.java: edited since the build: not compared\n"), "{text}");
    assert!(text.contains("1 file(s) edited since the build; disagreements in other files may come from that"), "{text}");
    assert!(r.found.is_empty(), "{text}");
    // a class file older than its source by its time alone: the same
    touch(&use_, SystemTime::now() - Duration::from_secs(90));
    assert!(check(&dir).modules[0].sets[0].edited.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_rename_after_the_build_makes_the_unedited_caller_s_disagreement_possibly_stale() {
    let javac = javac_or_skip!();
    let dir = project(
        "rename",
        MAVEN,
        &[
            ("src/main/java/demo/Lib.java", "package demo;\npublic class Lib {\n    public void put(Object o) {}\n    public void put(String s) {}\n}\n"),
            ("src/main/java/demo/Caller.java", "package demo;\nclass Caller {\n    void m(Lib lib) {\n        lib.put(\"x\");\n    }\n}\n"),
        ],
    );
    compile(&javac, &dir, "src/main/java", "target/classes", None);
    assert!(check(&dir).found.is_empty());
    // renamed, not compiled again: grounds now resolves the call in the
    // unedited Caller to put(Object), the class file still names put(String)
    let lib = dir.join("src/main/java/demo/Lib.java");
    std::fs::write(&lib, "package demo;\npublic class Lib {\n    public void put(Object o) {}\n    public void putText(String s) {}\n}\n").unwrap();
    let r = check(&dir);
    let text = r.text();
    assert_eq!(r.found.len(), 1, "{text}");
    let f = &r.found[0];
    assert_eq!((f.path.to_str().unwrap(), f.disagreement.line, f.disagreement.category, f.possibly_stale), ("src/main/java/demo/Caller.java", 4, "calls", true), "{text}");
    assert_eq!(r.outcome(), grounds_engine::daemon::Outcome::Problems);
    let stale = text.split("possibly stale: about a class declared in a file edited since the build\n").nth(1).unwrap_or_else(|| panic!("{text}"));
    assert!(stale.starts_with("calls (1)\n  src/main/java/demo/Caller.java:4: put: grounds demo.Lib.put(Object), the class file demo.Lib.put(Ljava/lang/String;) (other overload)\n"), "{text}");
    assert!(!text.contains("\ndisagreements\n"), "listed only as possibly stale: {text}");
    assert_eq!(r.json()["disagreements"][0]["possibly_stale"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_build_output_says_how_to_get_one() {
    let dir = project("none", GRADLE, SMALL);
    let r = check(&dir);
    let text = r.text();
    assert!(!r.any_output());
    assert_eq!(r.outcome(), grounds_engine::daemon::Outcome::Clean, "exit 0");
    assert!(text.contains("main: src/main/java: no build output found (looked in build/classes/java/main, target/classes)\n"), "{text}");
    assert!(text.contains("test: src/test/java: no build output found (looked in build/classes/java/test, target/test-classes)\n"), "{text}");
    assert!(text.contains("`./gradlew compileJava compileTestJava` or `mvn test-compile`"), "{text}");
    assert!(r.json()["hint"].as_str().is_some_and(|h| h.contains("mvn test-compile")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn maven_and_gradle_layouts_are_both_found() {
    let javac = javac_or_skip!();
    let dir = project("gradle", GRADLE, SMALL);
    compile(&javac, &dir, "src/main/java", "build/classes/java/main", None);
    compile(&javac, &dir, "src/test/java", "build/classes/java/test", Some("build/classes/java/main"));
    let r = check(&dir);
    let sets: Vec<_> = r.modules[0].sets.iter().map(|s| (s.name, s.classes.clone(), s.compared)).collect();
    assert_eq!(sets, [("main", Some(PathBuf::from("build/classes/java/main")), 2), ("test", Some(PathBuf::from("build/classes/java/test")), 1)], "{}", r.text());
    assert!(r.found.is_empty(), "{}", r.text());
    let _ = std::fs::remove_dir_all(&dir);
    // a Maven build of two modules
    let dir = project(
        "maven",
        ("pom.xml", "<project><modelVersion>4.0.0</modelVersion><groupId>t</groupId><artifactId>p</artifactId><version>1</version><packaging>pom</packaging><modules><module>app</module></modules></project>\n"),
        &[("app/pom.xml", MAVEN.1), ("app/src/main/java/demo/Shapes.java", SMALL[0].1), ("app/src/main/java/demo/Use.java", SMALL[1].1), ("app/src/test/java/demo/ShapesTest.java", SMALL[2].1)],
    );
    let app = dir.join("app");
    maven_build(&javac, &app);
    let r = check(&dir);
    let sets: Vec<_> = r.modules.iter().flat_map(|m| m.sets.iter().map(|s| (m.dir.clone(), s.name, s.classes.clone(), s.compared))).collect();
    assert_eq!(sets, [(PathBuf::from("app"), "main", Some(PathBuf::from("app/target/classes")), 2), (PathBuf::from("app"), "test", Some(PathBuf::from("app/target/test-classes")), 1)], "{}", r.text());
    assert!(r.found.is_empty(), "{}", r.text());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A case label of an enum switch names the enum's constant (JLS 14.11.1),
/// whatever field of that name is in scope, and grounds resolves it so: the
/// class file agrees. Reduced from reactor-core's
/// reactor/core/publisher/ImmutableSignal.java:142 `ON_COMPLETE`, which
/// grounds resolved to the field, and the gate's baseline listed, until
/// that was fixed.
#[test]
fn an_enum_case_label_agrees_with_the_class_file() {
    let javac = javac_or_skip!();
    let dir = project(
        "enum-case",
        MAVEN,
        &[
            ("src/main/java/demo/SignalType.java", "package demo;\npublic enum SignalType { ON_NEXT, ON_COMPLETE }\n"),
            (
                "src/main/java/demo/Signal.java",
                "package demo;\n\
                 final class Signal {\n\
                 \x20   SignalType type = SignalType.ON_NEXT;\n\
                 \x20   @Override\n\
                 \x20   public String toString() {\n\
                 \x20       switch (this.type) {\n\
                 \x20           case ON_NEXT:\n\
                 \x20               return \"onNext()\";\n\
                 \x20           case ON_COMPLETE:\n\
                 \x20               return \"onComplete()\";\n\
                 \x20           default:\n\
                 \x20               return \"other\";\n\
                 \x20       }\n\
                 \x20   }\n\
                 \x20   private static final Signal ON_COMPLETE = new Signal();\n\
                 }\n",
            ),
        ],
    );
    compile(&javac, &dir, "src/main/java", "target/classes", None);
    let r = check(&dir);
    let text = r.text();
    assert!(r.found.is_empty(), "{text}");
    assert_eq!(r.outcome(), grounds_engine::daemon::Outcome::Clean);
    let _ = std::fs::remove_dir_all(&dir);
}
