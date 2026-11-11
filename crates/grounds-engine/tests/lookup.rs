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
    findings_of(dir, COMPILE)
}

/// `path:line rule` of every finding of `rules`, sorted.
fn findings_of(dir: &Path, rules: &[&str]) -> Vec<String> {
    let mut project = Project::load(dir).unwrap();
    project.config.rules.enable = rules.iter().map(|s| s.to_string()).collect();
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

#[test]
fn a_copy_outside_a_source_root_is_never_the_certain_one() {
    let dir = project(
        "duplicate-copy",
        &[
            // guava: plain `src` trees and a GWT super-source copy without the field
            ("core/src/p/Cache.java", "package p;\npublic class Cache {\n    int[] segments;\n}\n"),
            ("gwt/src-super/p/Cache.java", "package p;\npublic class Cache {\n}\n"),
            ("tests/bench/p/Bench.java", "package p;\nclass Bench {\n    int n(Cache c) { return c.segments.length; }\n}\n"),
            // javaparser: old copies of the sources kept as test data
            ("core/src/main/java/q/Config.java", "package q;\npublic class Config {\n    public Config setResolver(Object r) { return this; }\n}\n"),
            ("solver/src/test/test_sourcecode/q/Config.java", "package q;\npublic class Config {\n}\n"),
            ("solver/src/test/java/t/UseTest.java", "package t;\n\nimport q.Config;\n\nclass UseTest {\n    void f() { new Config().setResolver(null); }\n}\n"),
            // still judged: the copy of the file's own module and source set
            ("app/src/main/java/r/Model.java", "package r;\npublic class Model {\n    public void save() {}\n}\n"),
            ("lib/src/main/java/r/Model.java", "package r;\npublic class Model {\n    public void store() {}\n}\n"),
            ("app/src/main/java/r/Service.java", "package r;\nclass Service {\n    void run(Model m) { m.save(); m.store(); }\n}\n"),
        ],
    );
    assert_eq!(findings(&dir), vec!["app/src/main/java/r/Service.java:3 unresolved-method".to_string()]);
}

#[test]
fn member_types_of_a_class_declared_twice_come_from_the_copy_read() {
    let dir = project(
        "duplicate-member-types",
        &[
            // glassfish: default-package WebTest classes, one nesting an HttpClient
            ("a/WebTest.java", "public class WebTest {\n    private static final class HttpClient { }\n}\n"),
            ("b/WebTest.java", "import p.*;\npublic class WebTest {\n    void run() { HttpClient.builder(); HttpClient.noSuchMethod(); }\n}\n"),
            ("lib/p/HttpClient.java", "package p;\npublic class HttpClient { public static HttpClient builder() { return new HttpClient(); } }\n"),
            // micronaut-data: two example.Pet, one with a nested PetType, the
            // other next to a top-level one
            ("one/src/main/java/example/Pet.java", "package example;\npublic class Pet {\n    enum PetType { DOG }\n    public void setType(PetType t) {}\n}\n"),
            ("two/src/main/java/example/Pet.java", "package example;\npublic class Pet {\n    public void setType(PetType t) {}\n}\nenum PetType { CAT }\n"),
            ("two/src/test/java/example/PetTest.java", "package example;\nclass PetTest {\n    void f(Pet p) { p.setType(PetType.CAT); p.setKind(PetType.CAT); }\n}\n"),
        ],
    );
    assert_eq!(findings(&dir), vec!["b/WebTest.java:3 unresolved-method".to_string(), "two/src/test/java/example/PetTest.java:3 unresolved-method".to_string()]);
}

#[test]
fn a_default_package_class_is_seen_only_from_its_own_directory() {
    if !has_jdk() {
        return;
    }
    let dir = project(
        "default-package",
        &[
            // checker-framework: test inputs compiled one directory at a time
            ("tests/calledmethods/Use.java", "import java.io.*;\n\nclass Use {\n    void f(Closeable r) throws IOException { r.close(); }\n    void g(Error e) { new Error(\"boom\"); }\n}\n"),
            ("tests/issue141/Closeable.java", "interface Closeable {}\n"),
            ("tests/issue820/Error.java", "class Error {}\n"),
            // javacc: a Node that JJTree generates in each example, one written out
            ("examples/Interpreter/Node.java", "public interface Node {\n    void interpret();\n}\n"),
            ("examples/Transformer/SimpleNode.java", "public class SimpleNode implements Node {\n    public void jjtOpen() {}\n}\n"),
            // the same directory still compiles together
            ("examples/Interpreter/ASTStart.java", "public class ASTStart implements Node {\n    public void interpret() {}\n    void run(Node n) { n.interpret(); n.noSuchMethod(); }\n}\n"),
            ("examples/Interpreter/ASTEnd.java", "public class ASTEnd implements Node {\n}\n"),
        ],
    );
    assert_eq!(findings(&dir), vec!["examples/Interpreter/ASTEnd.java:1 unimplemented-abstract-method".to_string(), "examples/Interpreter/ASTStart.java:3 unresolved-method".to_string()]);
}

#[test]
fn a_class_of_a_variant_source_set_is_seen_only_from_that_set() {
    let dir = project(
        "variant-set",
        &[
            // amaze-file-manager: the F-Droid flavor's stand-in for a proprietary SDK
            ("app/src/fdroid/java/com/cloud/services/Dropbox.java", "package com.cloud.services;\n\npublic class Dropbox {\n    public Dropbox(Object ctx, String a, String b, String c, String d) { }\n}\n"),
            ("app/src/play/java/app/Loader.java", "package app;\n\nimport com.cloud.services.Dropbox;\n\nclass Loader {\n    Object load(Object ctx) { return new Dropbox(ctx, \"key\", \"secret\"); }\n}\n"),
            // jmonkeyengine: stubs that let a library build without AndroidX
            ("lib/src/androidx-stubs/java/androidx/fragment/app/Fragment.java", "package androidx.fragment.app;\n\npublic class Fragment {\n}\n"),
            ("examples/src/main/java/demo/JmeFragment.java", "package demo;\n\nimport androidx.fragment.app.Fragment;\n\npublic class JmeFragment extends Fragment {\n    Object args() { return getArguments(); }\n}\n"),
            // within the set it is the class
            ("app/src/fdroid/java/app/FdroidLoader.java", "package app;\n\nimport com.cloud.services.Dropbox;\n\nclass FdroidLoader {\n    Object load(Object ctx) { return new Dropbox(ctx, \"key\", \"secret\"); }\n}\n"),
        ],
    );
    assert_eq!(findings(&dir), vec!["app/src/fdroid/java/app/FdroidLoader.java:6 unresolved-constructor".to_string()]);
}

#[test]
fn files_git_tracks_are_read_whatever_gitignore_says() {
    let dir = project(
        "gitignored-tracked",
        &[
            (".gitignore", "/a/src/main/java/p/\n"),
            ("a/src/main/java/p/Logic.java", "package p;\npublic class Logic {\n    static void serializeParent(String s) {}\n}\n"),
            ("a/src/test/java/p/LogicTest.java", "package p;\npublic class LogicTest {\n    void t() { Logic.serializeParent(\"x\"); Logic.serializeChild(\"x\"); }\n}\n"),
            ("b/src/main/java/p/Logic.java", "package p;\npublic class Logic {\n    static void serializeCar(String s) {}\n}\n"),
            // ignored and not tracked: not part of the project
            ("a/src/main/java/p/Scratch.java", "package p;\nclass Scratch {\n    void t() { Logic.nothingHere(); }\n}\n"),
        ],
    );
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git").arg("-C").arg(&dir).args(["-c", "commit.gpgsign=false", "-c", "core.hooksPath=.no-hooks"]).args(args).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").status().is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["add", "-f", "a/src/main/java/p/Logic.java"]);
    git(&["commit", "-qm", "init"]);
    assert_eq!(findings(&dir), vec!["a/src/test/java/p/LogicTest.java:3 unresolved-method".to_string()]);
}

const OLD_RELEASE: &[(&str, &str)] = &[
        ("settings.gradle", "include 'streams', 'streams:upgrade-tests-22', 'app'\n"),
        ("gradle.properties", "group=org.example\nversion=4.0.0-SNAPSHOT\n"),
        ("gradle/dependencies.gradle", "libs += [\n  streams_22: \"org.example:example-streams:$versions.streams_22\",\n]\n"),
        (
            "build.gradle",
            "apply from: \"$rootDir/gradle/dependencies.gradle\"\nsubprojects { apply plugin: 'java' }\nproject(':streams') {\n  base { archivesName = \"example-streams\" }\n}\nproject(':streams:upgrade-tests-22') {\n  dependencies {\n    // the 2.2 release from Maven Central, not project(':streams')\n    testImplementation libs.streams_22\n  }\n}\nproject(':app') {\n  dependencies { implementation project(':streams') }\n}\n",
        ),
        ("streams/src/main/java/org/example/streams/TimeWindows.java", "package org.example.streams;\n\npublic final class TimeWindows {\n    // of(long) was removed in 4.0\n    public static TimeWindows ofSizeWithNoGrace(long size) { return new TimeWindows(); }\n}\n"),
        ("streams/upgrade-tests-22/src/test/java/org/example/upgrade/SmokeTest.java", "package org.example.upgrade;\n\nimport org.example.streams.TimeWindows;\n\nclass SmokeTest {\n    Object w() { return TimeWindows.of(1000L); }\n    void own() { new SmokeTest().noSuchMethod(); }\n}\n"),
        // a module nested in the marked one is another module
        ("streams/upgrade-tests-22/nested/build.gradle", "\n"),
        ("streams/upgrade-tests-22/nested/src/main/java/org/example/nested/Helper.java", "package org.example.nested;\n\npublic class Helper { }\n"),
        ("streams/upgrade-tests-22/src/test/java/org/example/upgrade/UsesHelper.java", "package org.example.upgrade;\n\nimport org.example.nested.Helper;\n\nclass UsesHelper {\n    void f() { new Helper().gone(); }\n}\n"),
        // a module built against the sources is judged against them
        ("app/src/main/java/org/example/app/Windows.java", "package org.example.app;\n\nimport org.example.streams.TimeWindows;\n\nclass Windows {\n    Object w() { return TimeWindows.of(1000L); }\n}\n"),
];

#[test]
fn a_module_built_against_a_published_release_of_the_project_sees_no_sources_of_other_modules() {
    let dir = project("old-release", OLD_RELEASE);
    assert_eq!(
        findings(&dir),
        vec!["app/src/main/java/org/example/app/Windows.java:6 unresolved-method".to_string(), "streams/upgrade-tests-22/src/test/java/org/example/upgrade/SmokeTest.java:7 unresolved-method".to_string()]
    );
}

#[test]
fn a_configured_release_keeps_the_modules_of_a_module_built_against_a_published_release_apart() {
    // the release from grounds.toml: still one entry per module, which is
    // how a class's module is told from an enclosing module's
    let mut files = OLD_RELEASE.to_vec();
    files.push(("grounds.toml", "java_release = 21\n"));
    let dir = project("old-release-configured", &files);
    assert_eq!(
        findings(&dir),
        vec!["app/src/main/java/org/example/app/Windows.java:6 unresolved-method".to_string(), "streams/upgrade-tests-22/src/test/java/org/example/upgrade/SmokeTest.java:7 unresolved-method".to_string()]
    );
}

#[test]
fn emulations_of_jdk_classes_are_not_judged() {
    if !has_jdk() {
        return;
    }
    // libgdx: GWT emulation of java.nio, compiled by GWT only, never by javac with the code
    let dir = project(
        "gwt-emulation",
        &[
            ("gwt/emu/java/nio/CharSequenceAdapter.java", "package java.nio;\n\nfinal class CharSequenceAdapter extends CharBuffer {\n    CharSequenceAdapter(int cap) { super(cap); }\n    void m() { int p = position; }\n}\n"),
            ("gwt/emu/java/io/BufferedReader.java", "package java.io;\n\npublic class BufferedReader {\n    private int position;\n    public int read() { return position++; }\n}\n"),
            ("src/main/java/app/Use.java", "package app;\n\nclass Use {\n    void f() { new Use().missing(); }\n}\n"),
        ],
    );
    assert_eq!(findings(&dir), vec!["src/main/java/app/Use.java:4 unresolved-method".to_string()]);
}

#[test]
fn a_default_package_name_declared_in_another_directory_is_unknown() {
    if !has_jdk() {
        return;
    }
    // compiled together, the default-package Closeable wins over java.io.*;
    // compiled apart, java.io.Closeable does: which one is not known
    let dir = project(
        "default-package-unknown",
        &[
            ("a/Closeable.java", "interface Closeable {}\n"),
            ("b/Use.java", "import java.io.*;\n\nclass Use implements Closeable {\n}\n"),
            // a name only one directory declares stays judged within it
            ("a/Impl.java", "class Impl implements Runnable {\n}\n"),
        ],
    );
    assert_eq!(findings(&dir), vec!["a/Impl.java:1 unimplemented-abstract-method".to_string()]);
}

#[test]
fn copies_with_different_supertypes_leave_inherited_member_types_unknown() {
    let dir = project(
        "duplicate-headers",
        &[
            ("lib/src/main/java/p/Empty.java", "package p;\npublic class Empty {}\n"),
            ("lib/src/main/java/p/Base.java", "package p;\npublic class Base {\n    public static class Member { public static void ping() {} }\n}\n"),
            ("lib/src/main/java/q/Member.java", "package q;\npublic class Member {}\n"),
            // two copies of p.Owner with the same imports and different supertypes
            ("one/src/main/java/p/Owner.java", "package p;\nimport q.*;\nclass Owner extends Empty {\n}\n"),
            ("two/src/main/java/p/Owner.java", "package p;\nimport q.*;\nclass Owner extends Base {\n    void f() { Member.ping(); }\n}\n"),
        ],
    );
    assert_eq!(findings(&dir), Vec::<String>::new());
}

#[test]
fn a_variant_class_is_seen_in_headers_of_its_own_set() {
    let dir = project(
        "variant-header",
        &[
            ("app/src/fdroid/java/p/Base.java", "package p;\n\npublic class Base { }\n"),
            ("app/src/fdroid/java/p/Broken.java", "package p;\n\nclass Broken extends Base {\n    void f() { new Broken().missing(); }\n}\n"),
        ],
    );
    assert_eq!(findings(&dir), vec!["app/src/fdroid/java/p/Broken.java:4 unresolved-method".to_string()]);
}

#[test]
fn a_class_declared_in_two_modules_is_judged_inside_its_own_files() {
    if !has_jdk() {
        return;
    }
    // a starter for two framework versions: the same class in two modules,
    // each implementing an interface of its framework (no jars here). Which
    // copy other files see is uncertain; inside its own file, `String` is
    // still java.lang's
    let body = "    String f(String s) {\n        return s.replaceAll(\"_\", \".\");\n    }\n}\n";
    let dir = project(
        "copies-own-file",
        &[
            ("starter/src/main/java/p/Post.java", &format!("package p;\nimport fw.v4.Hook;\npublic class Post implements Hook {{\n{body}")),
            ("starter-3/src/main/java/p/Post.java", &format!("package p;\nimport fw.v3.Hook;\npublic class Post implements Hook {{\n{body}")),
        ],
    );
    assert_eq!(findings_of(&dir, &["replaceall-with-literal"]), vec!["starter-3/src/main/java/p/Post.java:5 replaceall-with-literal".to_string(), "starter/src/main/java/p/Post.java:5 replaceall-with-literal".to_string()]);
}

#[test]
fn inside_its_own_file_a_class_reads_its_own_copy() {
    if !has_jdk() {
        return;
    }
    // test inputs: one default-package class name in many directories, with
    // the same imports and different supertypes. Inside each file the copy
    // read is that file's: `Integer` is java.lang's
    let dir = project(
        "own-copy",
        &[
            ("tests/a/WebTest.java", "import java.io.*;\npublic class WebTest extends Base {\n    Object f() { return new Integer(1); }\n}\n"),
            ("tests/a/Base.java", "public class Base {}\n"),
            ("tests/b/WebTest.java", "import java.io.*;\npublic class WebTest {\n    Object f() { return new Integer(2); }\n}\n"),
        ],
    );
    assert_eq!(findings_of(&dir, &["boxed-primitive-constructor"]), vec!["tests/a/WebTest.java:3 boxed-primitive-constructor".to_string(), "tests/b/WebTest.java:3 boxed-primitive-constructor".to_string()]);
}

#[test]
fn a_group_is_the_projects_only_for_the_module_it_is_stated_or_inferred_for() {
    let dir = project(
        "group-evidence",
        &[
            ("settings.gradle", "include 'guava', 'lib', 'all', 'app', 'old', 'older'\n"),
            // a version catalog names dependencies' groups, not the project's
            ("gradle/libs.versions.toml", "[libraries]\nguava = { group = \"com.google.guava\", name = \"guava\", version = \"33.0.0-jre\" }\n"),
            ("build.gradle", "subprojects { apply plugin: 'java' }\n"),
            // a module that happens to be named guava, with a group of its own
            // and a shim under com/google/guava: Google's guava is not the project's
            ("guava/build.gradle", "group = 'org.acme'\n"),
            ("guava/src/main/java/com/google/guava/Shim.java", "package com.google.guava;\npublic class Shim {}\n"),
            // no group stated: lib's own sources say org.acme
            ("lib/build.gradle", "\n"),
            ("lib/src/main/java/org/acme/lib/Thing.java", "package org.acme.lib;\npublic class Thing {}\n"),
            ("app/build.gradle", "dependencies {\n  implementation 'com.google.guava:guava:33.0.0-jre'\n  implementation project(':lib')\n}\n"),
            ("app/src/main/java/org/acme/app/App.java", "package org.acme.app;\nimport org.acme.lib.Thing;\nclass App {\n    void f() { new Thing().missing(); }\n}\n"),
            // an aggregator: no package of its own, its module name says org.acme
            ("all/build.gradle", "\n"),
            ("all/src/main/java/module-info.java", "module org.acme.all {\n  requires transitive org.acme.lib;\n}\n"),
            // a published release of lib (or of the aggregator): its classes are not today's sources
            ("old/build.gradle", "dependencies { implementation 'org.acme:lib:0.9' }\n"),
            ("older/build.gradle", "dependencies { implementation 'org.acme:all:0.8' }\n"),
            ("older/src/main/java/org/acme/older/Older.java", "package org.acme.older;\nimport org.acme.lib.Thing;\nclass Older {\n    void f() { new Thing().missing(); }\n}\n"),
            ("old/src/main/java/org/acme/old/Old.java", "package org.acme.old;\nimport org.acme.lib.Thing;\nclass Old {\n    void f() { new Thing().missing(); }\n}\n"),
        ],
    );
    assert_eq!(findings(&dir), vec!["app/src/main/java/org/acme/app/App.java:4 unresolved-method".to_string()]);
}

#[test]
fn a_build_that_states_its_group_is_not_given_another_one_by_its_packages() {
    let dir = project(
        "group-stated",
        &[
            ("settings.gradle", "include 'guava', 'app'\n"),
            ("build.gradle", "allprojects { group = 'org.example' }\nsubprojects { apply plugin: 'java' }\n"),
            ("guava/build.gradle", "\n"),
            ("guava/src/main/java/com/google/guava/Shim.java", "package com.google.guava;\npublic class Shim {}\n"),
            ("guava/src/main/java/org/example/Thing.java", "package org.example;\npublic class Thing {}\n"),
            ("app/build.gradle", "dependencies {\n  implementation 'com.google.guava:guava:33.0.0-jre'\n  implementation project(':guava')\n}\n"),
            ("app/src/main/java/org/example/app/App.java", "package org.example.app;\nimport org.example.Thing;\nclass App {\n    void f() { new Thing().missing(); }\n}\n"),
        ],
    );
    assert_eq!(findings(&dir), vec!["app/src/main/java/org/example/app/App.java:4 unresolved-method".to_string()]);
}
