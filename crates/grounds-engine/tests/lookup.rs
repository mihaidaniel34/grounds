//! Which class a name means: the smoke corpus's causes about class lookup
//! (docs/smoke/2026-11-triage.md), as small synthetic projects. Each test
//! also keeps a real break reported, so that a fix is not a rule turned off.

use grounds_engine::analyze::{analyze_files, build_ruleset};
use grounds_engine::Project;
use std::path::{Path, PathBuf};

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A project of the given files, without a build file unless one is given.
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grounds-lookup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (rel, text) in files {
        write(&dir, rel, text);
    }
    dir
}

/// `path:line rule` of every finding of the compile-break rules, sorted.
fn findings(dir: &Path) -> Vec<String> {
    let mut project = Project::load(dir).unwrap();
    project.config.rules.enable = COMPILE.iter().map(|s| s.to_string()).collect();
    let (set, options) = build_ruleset(&project);
    let files: Vec<_> = project.workspace.files.iter().map(|f| f.path.clone()).collect();
    let mut out: Vec<String> = analyze_files(&project, &set, &options, &files).into_iter().map(|f| format!("{}:{} {}", f.path, f.line, f.rule)).collect();
    out.sort();
    let _ = std::fs::remove_dir_all(dir);
    out
}

const COMPILE: &[&str] = &["unresolved-method", "unresolved-constructor", "unresolved-field", "override-matches-nothing", "unimplemented-abstract-method", "missing-jdk-import"];

fn has_jdk() -> bool {
    grounds_classfile::find_java_home().is_some_and(|h| h.join("lib/modules").is_file())
}

#[test]
fn a_source_file_declaring_a_jdk_class_does_not_replace_the_jdks() {
    if !has_jdk() {
        return;
    }
    let dir = project(
        "jdk-standin",
        &[
            // test input of a tool: an empty java.lang.Object
            ("third_party/ijar/test/Object.java", "package java.lang;\n\nclass Object {\n}\n"),
            // a stub of a class the JDK has, without the methods the code calls
            ("stubs/java/lang/invoke/MethodHandle.java", "package java.lang.invoke;\n\npublic abstract class MethodHandle {\n}\n"),
            // a compile-only stub of an internal JDK package (not exported)
            ("stubs/com/sun/tools/javac/util/JCDiagnostic.java", "package com.sun.tools.javac.util;\n\npublic class JCDiagnostic {\n}\n"),
            ("stubs/com/sun/tools/javac/util/Log.java", "package com.sun.tools.javac.util;\n\npublic class Log {\n    public static Log instance() { return new Log(); }\n}\n"),
            (
                "src/main/java/app/Visitor.java",
                "package app;\n\nimport com.sun.tools.javac.util.JCDiagnostic;\nimport com.sun.tools.javac.util.Log;\nimport java.lang.invoke.MethodHandle;\n\nclass Visitor {\n    String name(Object value) { return value.getClass().getName(); }\n    String text(Visitor v) { return v.toString(); }\n    long at(JCDiagnostic d) { return d.getStartPosition(); }\n    Object log() { return Log.instance(0); }\n    Object call(MethodHandle h) throws Throwable { return h.invokeExact(); }\n    void broken() { new Visitor().noSuchMethod(); }\n}\n",
            ),
        ],
    );
    assert_eq!(findings(&dir), vec![format!("{}:13 unresolved-method", "src/main/java/app/Visitor.java")]);
}
