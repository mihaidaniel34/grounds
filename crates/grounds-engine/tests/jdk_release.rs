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
