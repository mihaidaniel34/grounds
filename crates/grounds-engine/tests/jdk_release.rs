//! A project compiling for a release newer than the installed JDK: grounds
//! warns once and does not report compile breaks in classes that inherit
//! from JDK classes, whose newer members it cannot see.

use grounds_engine::session::{CheckOptions, Session};
use grounds_rules::Severity;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").status().unwrap().success();
    assert!(ok, "git {args:?}");
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn release_newer_than_the_jdk_warns_once_and_stays_silent_on_jdk_subclasses() {
    let Some(home) = grounds_classfile::find_java_home() else { return };
    let Some(installed) = grounds_classfile::jdk_feature_release(&home) else { return };
    let dir = std::env::temp_dir().join(format!("grounds-jdk-release-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "grounds.toml", &format!("java_release = {}\n", installed + 1));
    // a JDK 21 method that no JDK has (`getFirst` would only test one release)
    write(&dir, "src/main/java/demo/Meter.java", "package demo;\nimport java.util.concurrent.CopyOnWriteArrayList;\nclass Meter extends CopyOnWriteArrayList<String> {\n    void run() { new Meter().noSuchMethodHere(); }\n}\n");
    // the same call on a class without JDK supertypes is a real break
    write(&dir, "src/main/java/demo/Plain.java", "package demo;\nclass Plain {\n    void run() { new Plain().noSuchMethodHere(); }\n}\n");
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);

    let mut s = Session::open(&dir).unwrap();
    let warnings = s.take_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains(&format!("Java {}", installed + 1)), "{warnings:?}");
    assert!(s.take_warnings().is_empty());

    let opts = CheckOptions { mode: grounds_engine::session::Mode::Edit, files: Vec::new(), all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None };
    let r = s.check(&CheckOptions { files: vec![dir.join("src/main/java/demo/Meter.java"), dir.join("src/main/java/demo/Plain.java")], ..opts });
    let breaks: Vec<&str> = r.findings.iter().filter(|f| f.rule == "unresolved-method").map(|f| f.path.as_str()).collect();
    assert_eq!(breaks.len(), 1, "{:?}", r.findings.iter().map(|f| (f.rule, &f.path)).collect::<Vec<_>>());
    assert!(breaks[0].ends_with("Plain.java"), "{breaks:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn imports_of_jdk_classes_the_release_lacks_are_reported() {
    let Some(home) = grounds_classfile::find_java_home() else { return };
    // Java 17's API needs ct.sym, which a JRE or a jlink image lacks
    if !home.join("lib/ct.sym").is_file() {
        return;
    }
    // Java 17's API from ct.sym, and the JDK's own (21 or newer) from its image
    let Some(installed) = grounds_classfile::jdk_feature_release(&home).filter(|&v| v >= 21) else { return };
    let check = |release: u32| {
        let dir = std::env::temp_dir().join(format!("grounds-jdk-import-{}-{release}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write(&dir, "grounds.toml", &format!("java_release = {release}\n"));
        let imports = [
            "import java.util.SequencedCollection;", // Java 21
            "import java.util.nosuchpackage.*;",
            "import java.util.ImmutableCollections;", // package-private
            "import java.util.List;",
            "import java.util.Map.Entry;",
            "import static java.util.Map.entry;",
            "import static java.util.Map.*;",
            "import java.util.concurrent.*;",
            "import java.net.http.HttpClient;",
            // a module import (JEP 511, Java 25) names a module, not a class
            "import module java.base;",
        ];
        write(&dir, "src/main/java/demo/Uses.java", &format!("package demo;\n{}\nclass Uses {{}}\n", imports.join("\n")));
        // multi-release sources compile for their own release
        write(&dir, "src/main/java99/demo/Later.java", "package demo;\nimport java.util.NoSuchClassYet;\nclass Later {}\n");
        git(&dir, &["init", "-q"]);
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-qm", "init"]);
        let mut s = Session::open(&dir).unwrap();
        let opts = CheckOptions { mode: grounds_engine::session::Mode::Edit, files: vec![dir.join("src/main/java/demo/Uses.java"), dir.join("src/main/java99/demo/Later.java")], all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None };
        let r = s.check(&opts);
        let _ = std::fs::remove_dir_all(&dir);
        r.findings.iter().filter(|f| f.rule == "missing-jdk-import").map(|f| (f.line, f.message.clone())).collect::<Vec<_>>()
    };
    let old = check(17);
    assert_eq!(old.iter().map(|(l, _)| *l).collect::<Vec<_>>(), vec![2, 3, 4], "{old:?}");
    assert!(old[0].1.contains("Java 17"), "{old:?}");
    let own = check(installed);
    assert_eq!(own.iter().map(|(l, _)| *l).collect::<Vec<_>>(), vec![3, 4], "{own:?}");
}

/// `path:line rule` of the findings of `rules` over a whole project.
fn project_findings(dir: &Path, rules: &[&str]) -> Vec<String> {
    let mut project = grounds_engine::Project::load(dir).unwrap();
    project.config.rules.enable = rules.iter().map(|s| s.to_string()).collect();
    let (set, options) = grounds_engine::analyze::build_ruleset(&project);
    let files: Vec<_> = project.workspace.files.iter().map(|f| f.path.clone()).collect();
    let mut out: Vec<String> = grounds_engine::analyze::analyze_files(&project, &set, &options, &files).into_iter().map(|f| format!("{}:{} {}", f.path, f.line, f.rule)).collect();
    out.sort();
    out
}

fn fresh(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("grounds-jdk-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_release_the_build_does_not_state_as_a_literal_leaves_the_newest_api() {
    let Some(home) = grounds_classfile::find_java_home() else { return };
    if grounds_classfile::jdk_feature_release(&home).is_none_or(|v| v < 21) {
        return;
    }
    // gocd: the release every module compiles for is an expression; two
    // plugin modules state 11 for themselves
    let dir = fresh("release-expression");
    write(&dir, "settings.gradle", "include 'app', 'plugin-api'\n");
    write(&dir, "build.gradle", "ext.targetJavaVersion = 21\nsubprojects {\n  apply plugin: 'java'\n  tasks.withType(JavaCompile).configureEach {\n    options.release.set(targetJavaVersion as Integer)\n  }\n}\n");
    write(&dir, "app/build.gradle", "\n");
    write(&dir, "app/src/main/java/app/Stages.java", "package app;\n\nimport java.util.ArrayList;\n\npublic class Stages extends ArrayList<String> {\n    String first() { return getFirst(); }\n    String none() { return noSuchMethod(); }\n}\n");
    write(&dir, "plugin-api/build.gradle", "compileJava {\n  options.release.set(11)\n}\n");
    write(&dir, "plugin-api/src/main/java/api/Plugin.java", "package api;\n\npublic interface Plugin { }\n");
    let found = project_findings(&dir, &["unresolved-method", "missing-jdk-import"]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(found, vec!["app/src/main/java/app/Stages.java:7 unresolved-method".to_string()]);
}

#[test]
fn jdk_classes_a_module_release_lacks_are_reported_where_the_release_is_exact() {
    let Some(home) = grounds_classfile::find_java_home() else { return };
    if !home.join("lib/ct.sym").is_file() || grounds_classfile::jdk_feature_release(&home).is_none_or(|v| v < 21) {
        return;
    }
    let dir = fresh("module-releases");
    let uses = |pkg: &str| format!("package {pkg};\n\nimport java.util.SequencedCollection;\n\nclass Uses {{ }}\n");
    write(&dir, "pom.xml", "<project><groupId>g</groupId><artifactId>root</artifactId><version>1</version><modules><module>exact</module><module>newer</module><module>lower-bound</module><module>records</module></modules></project>");
    // --release 17: Java 21's SequencedCollection does not compile, while
    // another module compiles for 21 and the API loaded is 21's
    write(&dir, "exact/pom.xml", "<project><artifactId>exact</artifactId><properties><maven.compiler.release>17</maven.compiler.release></properties></project>");
    write(&dir, "exact/src/main/java/a/Uses.java", &uses("a"));
    write(&dir, "newer/pom.xml", "<project><artifactId>newer</artifactId><properties><maven.compiler.release>21</maven.compiler.release></properties></project>");
    write(&dir, "newer/src/main/java/b/Uses.java", &uses("b"));
    // -source/-target 17 only: javac compiles against the JDK it runs on
    write(&dir, "lower-bound/pom.xml", "<project><artifactId>lower-bound</artifactId><properties><maven.compiler.source>17</maven.compiler.source><maven.compiler.target>17</maven.compiler.target></properties></project>");
    write(&dir, "lower-bound/src/main/java/c/Uses.java", &uses("c"));
    // --release 11 with records: the build's literal is not what compiles this code
    write(&dir, "records/pom.xml", "<project><artifactId>records</artifactId><properties><maven.compiler.release>11</maven.compiler.release></properties></project>");
    write(&dir, "records/src/main/java/d/Uses.java", &uses("d"));
    write(&dir, "records/src/main/java/d/Point.java", "package d;\n\nrecord Point(int x, int y) { }\n");
    let found = project_findings(&dir, &["missing-jdk-import"]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(found, vec!["exact/src/main/java/a/Uses.java:3 missing-jdk-import".to_string()]);
}

#[test]
fn a_jdk_deprecation_newer_than_the_module_release_is_not_reported() {
    let Some(home) = grounds_classfile::find_java_home() else { return };
    if grounds_classfile::jdk_feature_release(&home).is_none_or(|v| v < 21) {
        return;
    }
    let dir = fresh("deprecation-since");
    // URL's constructors are deprecated since Java 20
    let uses = |pkg: &str| format!("package {pkg};\n\nclass Uses {{\n    Object url() throws Exception {{ return new java.net.URL(\"http://x\"); }}\n}}\n");
    write(&dir, "pom.xml", "<project><groupId>g</groupId><artifactId>root</artifactId><version>1</version><modules><module>old</module><module>new</module></modules></project>");
    write(&dir, "old/pom.xml", "<project><artifactId>old</artifactId><properties><maven.compiler.release>17</maven.compiler.release></properties></project>");
    write(&dir, "old/src/main/java/a/Uses.java", &uses("a"));
    write(&dir, "new/pom.xml", "<project><artifactId>new</artifactId><properties><maven.compiler.release>21</maven.compiler.release></properties></project>");
    write(&dir, "new/src/main/java/b/Uses.java", &uses("b"));
    let found = project_findings(&dir, &["deprecated-api-usage"]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(found, vec!["new/src/main/java/b/Uses.java:4 deprecated-api-usage".to_string()]);
}

#[test]
fn jdk_members_newer_than_a_source_level_are_not_taken_as_existing() {
    let Some(home) = grounds_classfile::find_java_home() else { return };
    if !home.join("lib/ct.sym").is_file() || grounds_classfile::jdk_feature_release(&home).is_none_or(|v| v < 21) {
        return;
    }
    // -source/-target 8: the API loaded is the installed JDK's, so that Java 9+
    // members the code may use are not missing; members it claims exist must
    // exist in Java 8's API
    let dir = fresh("source-level");
    write(&dir, "pom.xml", "<project><groupId>g</groupId><artifactId>a</artifactId><version>1</version><properties><maven.compiler.source>1.8</maven.compiler.source><maven.compiler.target>1.8</maven.compiler.target></properties></project>");
    // CharSequence.isEmpty is Java 15; length() and toString() are Java 8's
    write(&dir, "src/main/java/a/Text.java", "package a;\n\npublic abstract class Text implements CharSequence {\n    public boolean isEmpty() { return length() == 0; }\n    public String toString() { return \"\"; }\n}\n");
    // Thread.getId is deprecated since 19, new Integer(int) since 9
    write(&dir, "src/main/java/a/Uses.java", "package a;\n\nclass Uses {\n    long id() { return Thread.currentThread().getId(); }\n    Object boxed() { return new Integer(1); }\n    Object first(java.util.ArrayList<String> l) { return l.getFirst(); }\n}\n");
    let found = project_findings(&dir, &["missing-override", "deprecated-api-usage", "unresolved-method"]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(found, vec!["src/main/java/a/Text.java:5 missing-override".to_string()]);
}

#[test]
fn a_child_module_of_unknown_release_does_not_inherit_its_parents_exact_one() {
    let Some(home) = grounds_classfile::find_java_home() else { return };
    if !home.join("lib/ct.sym").is_file() || grounds_classfile::jdk_feature_release(&home).is_none_or(|v| v < 21) {
        return;
    }
    // the root states --release 17; the child's release is an expression
    let dir = fresh("child-unknown");
    write(&dir, "pom.xml", "<project><groupId>g</groupId><artifactId>root</artifactId><version>1</version><properties><maven.compiler.release>17</maven.compiler.release></properties><modules><module>child</module></modules></project>");
    write(&dir, "src/main/java/a/Root.java", "package a;\n\nimport java.util.SequencedCollection;\n\nclass Root { }\n");
    write(&dir, "child/pom.xml", "<project><artifactId>child</artifactId><properties><maven.compiler.release>${java.next}</maven.compiler.release></properties></project>");
    write(&dir, "child/src/main/java/b/Child.java", "package b;\n\nimport java.util.SequencedCollection;\n\nclass Child { }\n");
    let found = project_findings(&dir, &["missing-jdk-import"]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(found, vec!["src/main/java/a/Root.java:3 missing-jdk-import".to_string()]);
}

/// `grounds changed --base HEAD~1` over a project whose build files are
/// `build`, after a commit that adds `import java.net.http.HttpClient` (Java
/// 11) to the test `p.MainTest` and, with `in_main`, to the main class
/// `p.Main` too: the `missing-jdk-import` findings, with their maturity.
fn http_client_import_changed(name: &str, build: &[(&str, &str)], in_main: bool) -> Vec<(String, u32, grounds_rules::Maturity)> {
    use grounds_engine::session::Mode;
    let dir = fresh(name);
    for (p, t) in build {
        write(&dir, p, t);
    }
    let main = "package p;\n\npublic class Main {\n    public int one() { return 1; }\n}\n";
    let test = "package p;\n\npublic class MainTest {\n    int one() { return new Main().one(); }\n}\n";
    write(&dir, "src/main/java/p/Main.java", main);
    write(&dir, "src/test/java/p/MainTest.java", test);
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);
    let with_client = |text: &str, class: &str| text.replacen("package p;\n", "package p;\nimport java.net.http.HttpClient;\n", 1).replacen(&format!("public class {class} {{\n"), &format!("public class {class} {{\n    HttpClient client() {{ return HttpClient.newHttpClient(); }}\n"), 1);
    write(&dir, "src/test/java/p/MainTest.java", &with_client(test, "MainTest"));
    if in_main {
        write(&dir, "src/main/java/p/Main.java", &with_client(main, "Main"));
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "use the HTTP client"]);
    let base = String::from_utf8(Command::new("git").arg("-C").arg(&dir).args(["rev-parse", "HEAD~1"]).output().unwrap().stdout).unwrap().trim().to_string();
    let mut s = Session::open(&dir).unwrap();
    let r = s.check(&CheckOptions { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, use_baseline: false, min_severity: Severity::Info, base: Some(base) });
    let _ = std::fs::remove_dir_all(&dir);
    r.findings.iter().chain(&r.pending).filter(|f| f.rule == "missing-jdk-import").map(|f| (f.path.clone(), f.line, f.maturity)).collect()
}

/// The JDK at hand can read Java 8's and 17's API (from its `lib/ct.sym`).
fn jdk_reads_8_and_17() -> bool {
    let Some(home) = grounds_classfile::find_java_home() else { return false };
    home.join("lib/ct.sym").is_file() && grounds_classfile::jdk_feature_release(&home).is_some_and(|v| v >= 17)
}

/// The builds that compile main code for Java 8 and tests for Java 17.
fn main_8_tests_17() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    let compiler = |conf: &'static str| -> &'static str { Box::leak(format!("<project><modelVersion>4.0.0</modelVersion><groupId>g</groupId><artifactId>a</artifactId><version>1</version><build><plugins><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version>{conf}</plugin></plugins></build></project>").into_boxed_str()) };
    vec![
        // prometheus client_java's shape
        ("maven-plugin", vec![("pom.xml", compiler("<configuration><release>8</release><testRelease>17</testRelease></configuration>"))]),
        // the test compile's own execution
        ("maven-execution", vec![("pom.xml", compiler("<configuration><release>8</release></configuration><executions><execution><id>default-testCompile</id><configuration><release>17</release></configuration></execution></executions>"))]),
        ("maven-properties", vec![("pom.xml", "<project><modelVersion>4.0.0</modelVersion><groupId>g</groupId><artifactId>a</artifactId><version>1</version><properties><java.version>8</java.version><test.java.version>17</test.java.version><maven.compiler.release>${java.version}</maven.compiler.release><maven.compiler.testRelease>${test.java.version}</maven.compiler.testRelease></properties></project>")]),
        ("gradle-groovy", vec![("settings.gradle", "rootProject.name = 'a'\n"), ("build.gradle", "plugins { id 'java' }\ncompileJava {\n    options.release = 8\n}\ncompileTestJava {\n    options.release = 17\n}\n")]),
        ("gradle-named", vec![("settings.gradle", "rootProject.name = 'a'\n"), ("build.gradle", "plugins { id 'java' }\ntasks.named('compileJava') { options.release = 8 }\ntasks.named('compileTestJava') {\n    options.release = 17\n}\n")]),
        ("gradle-kotlin", vec![("settings.gradle.kts", "rootProject.name = \"a\"\n"), ("build.gradle.kts", "plugins { java }\ntasks.compileJava {\n    options.release.set(8)\n}\ntasks.named<JavaCompile>(\"compileTestJava\") {\n    options.release.set(17)\n}\n")]),
        // the release of every compile task, then the test compile's own
        ("gradle-each-then-test", vec![("settings.gradle.kts", "rootProject.name = \"a\"\n"), ("build.gradle.kts", "plugins { java }\ntasks.withType<JavaCompile>().configureEach {\n    options.release = 8\n}\ntasks.compileTestJava {\n    options.release = 17\n}\n")]),
        // the main compile's release alone: the tests compile for the JDK running Gradle
        ("gradle-main-only", vec![("settings.gradle", "rootProject.name = 'a'\n"), ("build.gradle", "plugins { id 'java' }\ncompileJava { options.release = 8 }\n")]),
    ]
}

#[test]
fn a_test_compiled_for_a_newer_release_than_main_code_may_import_its_classes() {
    if !jdk_reads_8_and_17() {
        return;
    }
    let wrong: Vec<_> = main_8_tests_17().into_iter().map(|(name, build)| (name, http_client_import_changed(&format!("test-release-{name}"), &build, false))).filter(|(_, found)| !found.is_empty()).collect();
    assert!(wrong.is_empty(), "{wrong:?}");
}

#[test]
fn main_code_compiled_for_an_older_release_than_its_tests_still_may_not() {
    if !jdk_reads_8_and_17() {
        return;
    }
    let want = vec![("src/main/java/p/Main.java".to_string(), 2, grounds_rules::Maturity::Stable)];
    let wrong: Vec<_> = main_8_tests_17().into_iter().map(|(name, build)| (name, http_client_import_changed(&format!("main-release-{name}"), &build, true))).filter(|(_, found)| *found != want).collect();
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// A session answers as a fresh load would after the build changes the
/// tests' release, and after a source set appears that compiles otherwise
/// than the rest of its module.
#[test]
fn a_session_follows_the_tests_release() {
    use grounds_engine::session::Mode;
    let Some(home) = grounds_classfile::find_java_home() else { return };
    if !home.join("lib/ct.sym").is_file() || grounds_classfile::jdk_feature_release(&home).is_none_or(|v| v < 21) {
        return;
    }
    let reported = |s: &mut Session, file: &Path| -> bool {
        let r = s.check(&CheckOptions { mode: Mode::Edit, files: vec![file.to_path_buf()], all_lines: true, use_baseline: false, min_severity: Severity::Info, base: None });
        r.findings.iter().any(|f| f.rule == "missing-jdk-import")
    };
    let pom = |test: u32| format!("<project><groupId>g</groupId><artifactId>a</artifactId><version>1</version><properties><maven.compiler.release>8</maven.compiler.release><maven.compiler.testRelease>{test}</maven.compiler.testRelease></properties></project>");
    let dir = fresh("session-test-release");
    write(&dir, "pom.xml", &pom(8));
    write(&dir, "src/main/java/p/Main.java", "package p;\n\npublic class Main { }\n");
    let test = dir.join("src/test/java/p/MainTest.java");
    write(&dir, "src/test/java/p/MainTest.java", "package p;\n\nimport java.net.http.HttpClient;\n\nclass MainTest { }\n");
    let mut s = Session::open(&dir).unwrap();
    assert!(reported(&mut s, &test));
    write(&dir, "pom.xml", &pom(17));
    assert!(!reported(&mut s, &test));
    assert!(!reported(&mut Session::open(&dir).unwrap(), &test));
    write(&dir, "pom.xml", &pom(8));
    assert!(reported(&mut s, &test));
    let _ = std::fs::remove_dir_all(&dir);

    // Gradle: every compile on JDK 17's toolchain, the tests on 21; a new
    // source set compiles on the toolchain
    let dir = fresh("session-new-set");
    write(&dir, "settings.gradle", "rootProject.name = 'a'\n");
    write(&dir, "build.gradle", "plugins { id 'java' }\njava { toolchain { languageVersion = JavaLanguageVersion.of(17) } }\ncompileTestJava { options.release = 21 }\n");
    write(&dir, "src/main/java/p/Main.java", "package p;\n\npublic class Main { }\n");
    write(&dir, "src/integrationTest/java/p/It.java", "package p;\n\nclass It { }\n");
    // past the margin of a file system clock's tick: the session does not
    // load again for the build files as just written
    std::thread::sleep(std::time::Duration::from_millis(2100));
    let mut s = Session::open(&dir).unwrap();
    let jmh = dir.join("src/jmh/java/p/Bench.java");
    write(&dir, "src/jmh/java/p/Bench.java", "package p;\n\nimport java.util.SequencedCollection;\n\nclass Bench { }\n");
    assert!(reported(&mut Session::open(&dir).unwrap(), &jmh));
    assert!(reported(&mut s, &jmh));
    let _ = std::fs::remove_dir_all(&dir);
}
