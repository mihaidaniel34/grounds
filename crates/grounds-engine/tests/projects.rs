//! Patterns from real Spring projects (thingsboard, dolphinscheduler, mall,
//! halo, start.spring.io, eladmin) that were wrongly reported, as small
//! multi-file projects.

use grounds_engine::analyze::{analyze_files, build_ruleset};
use grounds_engine::Project;
use std::path::{Path, PathBuf};

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grounds-projects-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    for (rel, text) in files {
        write(&dir, rel, text);
    }
    dir
}

fn findings(dir: &Path, rules: &[&str]) -> Vec<String> {
    let mut project = Project::load(dir).unwrap();
    project.config.rules.enable = rules.iter().map(|s| s.to_string()).collect();
    let (set, options) = build_ruleset(&project);
    let files: Vec<_> = project.workspace.files.iter().map(|f| f.path.clone()).collect();
    let out = analyze_files(&project, &set, &options, &files).into_iter().map(|f| format!("{}:{} {}", f.path, f.line, f.rule)).collect();
    let _ = std::fs::remove_dir_all(dir);
    out
}

const COMPILE: &[&str] = &["unresolved-method", "unresolved-constructor", "unresolved-field", "override-matches-nothing", "unimplemented-abstract-method"];
const DI: &[&str] = &["bean-not-found", "ambiguous-bean", "qualifier-not-found", "bean-name-conflict", "circular-constructor-dependency"];

#[test]
fn lombok_config_fluent_accessors_and_copyable_lazy() {
    let dir = project(
        "lombok-config",
        &[
            ("lombok.config", "config.stopBubbling = true\nlombok.accessors.fluent = true\nlombok.copyableAnnotations += org.springframework.context.annotation.Lazy\n"),
            ("src/main/java/app/Form.java", "package app;\nimport lombok.Getter;\n@Getter\npublic class Form {\n    private String driver;\n}\n"),
            ("src/main/java/app/Page.java", "package app;\nclass Page {\n    String m(Form f) { return f.driver(); }\n}\n"),
            ("src/main/java/app/App.java", "package app;\n@org.springframework.boot.autoconfigure.SpringBootApplication\npublic class App {}\n"),
            // a cycle broken by @Lazy copied onto the Lombok constructor
            ("src/main/java/app/A.java", "package app;\nimport lombok.RequiredArgsConstructor;\n@org.springframework.stereotype.Service\n@RequiredArgsConstructor\nclass A {\n    @org.springframework.context.annotation.Lazy\n    private final B b;\n}\n"),
            ("src/main/java/app/B.java", "package app;\nimport lombok.RequiredArgsConstructor;\n@org.springframework.stereotype.Service\n@RequiredArgsConstructor\nclass B {\n    private final A a;\n}\n"),
        ],
    );
    let found = findings(&dir, &[COMPILE, DI].concat());
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn single_type_import_shadows_same_package_even_when_missing() {
    let dir = project(
        "import-shadow",
        &[
            ("src/main/java/app/JsonPatch.java", "package app;\npublic class JsonPatch {\n    public JsonPatch() {}\n}\n"),
            ("src/main/java/app/Handler.java", "package app;\nimport com.github.fge.jsonpatch.JsonPatch;\nclass Handler {\n    Object m(JsonPatch p, Object node) throws Exception { return p.apply(node); }\n    Object n(Object ops) { return new JsonPatch(ops); }\n}\n"),
        ],
    );
    let found = findings(&dir, COMPILE);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn spring_registration_patterns() {
    let dir = project(
        "registration",
        &[
            ("src/main/java/app/App.java", "package app;\n@org.springframework.boot.autoconfigure.SpringBootApplication\n@org.mybatis.spring.annotation.MapperScan(\"app.mapper\")\npublic class App {}\n"),
            // MyBatis: interfaces in @MapperScan packages are beans
            ("src/main/java/app/mapper/OrderMapper.java", "package app.mapper;\npublic interface OrderMapper { int count(); }\n"),
            // a composed annotation carrying a condition makes beans conditional
            ("src/main/java/app/CoreComponent.java", "package app;\n@java.lang.annotation.Retention(java.lang.annotation.RetentionPolicy.RUNTIME)\n@org.springframework.boot.autoconfigure.condition.ConditionalOnExpression(\"true\")\npublic @interface CoreComponent {}\n"),
            ("src/main/java/app/Provider.java", "package app;\npublic interface Provider {}\n"),
            ("src/main/java/app/CoreProvider.java", "package app;\n@org.springframework.stereotype.Service\n@CoreComponent\nclass CoreProvider implements Provider {}\n"),
            ("src/main/java/app/OtherProvider.java", "package app;\n@org.springframework.stereotype.Service\n@CoreComponent\nclass OtherProvider implements Provider {}\n"),
            // prototype beans may get constructor arguments from getBean(Class, args)
            ("src/main/java/app/Target.java", "package app;\npublic class Target {}\n"),
            ("src/main/java/app/Checker.java", "package app;\n@org.springframework.stereotype.Component\n@org.springframework.context.annotation.Scope(\"prototype\")\nclass Checker {\n    Checker(Target target) {}\n}\n"),
            // @Bean methods inherited from a base configuration
            ("src/main/java/app/Cache.java", "package app;\npublic interface Cache {}\n"),
            ("src/main/java/app/BaseCacheConfig.java", "package app;\npublic class BaseCacheConfig {\n    @org.springframework.context.annotation.Bean\n    public Cache cache() { return null; }\n}\n"),
            ("src/main/java/app/CacheConfig.java", "package app;\n@org.springframework.context.annotation.Configuration\nclass CacheConfig extends BaseCacheConfig {}\n"),
            // @Bean methods in a class with an unrecognized (missing-jar) stereotype
            ("src/main/java/app/Containers.java", "package app;\npublic class Containers {}\n"),
            ("src/main/java/app/GenConfig.java", "package app;\n@io.spring.initializr.generator.project.ProjectGenerationConfiguration\nclass GenConfig {\n    @org.springframework.context.annotation.Bean\n    Containers containers() { return new Containers(); }\n}\n"),
            // an @Bean method replacing a scanned component of the same class
            ("src/main/java/app/HomeController.java", "package app;\n@org.springframework.stereotype.Controller\npublic class HomeController {}\n"),
            ("src/main/java/app/WebConfig.java", "package app;\n@org.springframework.context.annotation.Configuration\nclass WebConfig {\n    @org.springframework.context.annotation.Bean\n    HomeController homeController() { return new HomeController(); }\n}\n"),
            // a factory declared to return an interface, injected by its class
            ("src/main/java/app/Store.java", "package app;\npublic interface Store {}\n"),
            ("src/main/java/app/RedisStore.java", "package app;\npublic class RedisStore implements Store {}\n"),
            ("src/main/java/app/StoreConfig.java", "package app;\n@org.springframework.context.annotation.Configuration\nclass StoreConfig {\n    @org.springframework.context.annotation.Bean\n    Store store() { return new RedisStore(); }\n}\n"),
            ("src/main/java/app/Consumer.java", "package app;\n@org.springframework.stereotype.Service\nclass Consumer {\n    Consumer(app.mapper.OrderMapper orders,\n             Provider provider,\n             Cache cache,\n             Containers containers,\n             RedisStore store) {}\n}\n"),
        ],
    );
    let found = findings(&dir, DI);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn review_regressions_stay_reported() {
    // a plain class with a leftover @Bean method registers nothing
    let dir = project(
        "plain-bean-method",
        &[
            ("src/main/java/app/App.java", "package app;\n@org.springframework.boot.autoconfigure.SpringBootApplication\npublic class App {}\n"),
            ("src/main/java/app/Missing.java", "package app;\npublic class Missing {}\n"),
            ("src/main/java/app/Forgotten.java", "package app;\nclass Forgotten {\n    @org.springframework.context.annotation.Bean\n    Missing missing() { return new Missing(); }\n}\n"),
            ("src/main/java/app/Consumer.java", "package app;\n@org.springframework.stereotype.Component\nclass Consumer {\n    Consumer(Missing missing) {}\n}\n"),
            // autowireCandidate = false factories satisfy nothing
            ("src/main/java/app/Store.java", "package app;\npublic interface Store {}\n"),
            ("src/main/java/app/RedisStore.java", "package app;\npublic class RedisStore implements Store {}\n"),
            ("src/main/java/app/StoreConfig.java", "package app;\n@org.springframework.context.annotation.Configuration\nclass StoreConfig {\n    @org.springframework.context.annotation.Bean(autowireCandidate = false)\n    Store store() { return new RedisStore(); }\n    @org.springframework.context.annotation.Bean\n    String user(RedisStore redis) { return \"ok\"; }\n}\n"),
        ],
    );
    let found = findings(&dir, DI);
    assert_eq!(found, vec!["src/main/java/app/Consumer.java:4 bean-not-found".to_string(), "src/main/java/app/StoreConfig.java:7 bean-not-found".to_string()], "{found:?}");

    // same simple name in two project packages is a real mismatch
    let dir = project(
        "same-simple-name",
        &[
            ("src/main/java/a/Item.java", "package a;\npublic class Item {}\n"),
            ("src/main/java/b/Item.java", "package b;\npublic class Item {}\n"),
            ("src/main/java/app/Target.java", "package app;\nclass Target {\n    void accept(a.Item item) {}\n}\n"),
            ("src/main/java/app/Test.java", "package app;\nclass Test {\n    void f() { new Target().accept(new b.Item()); }\n}\n"),
            ("src/main/java/app/Child.java", "package app;\nclass Child extends Target {\n    @Override void accept(b.Item item) {}\n}\n"),
        ],
    );
    let mut found = findings(&dir, COMPILE);
    found.sort();
    assert_eq!(found, vec!["src/main/java/app/Child.java:3 override-matches-nothing".to_string(), "src/main/java/app/Test.java:3 unresolved-method".to_string()], "{found:?}");
}

#[test]
fn inherited_bean_methods_with_generics_and_overloads() {
    let dir = project(
        "inherited-generic",
        &[
            ("src/main/java/app/Store.java", "package app;\npublic class Store {}\n"),
            ("src/main/java/app/Base.java", "package app;\nabstract class Base<T> {\n    @org.springframework.context.annotation.Bean\n    T value() { return create(); }\n    abstract T create();\n    @org.springframework.context.annotation.Bean\n    Store make(String text) { return new Store(); }\n}\n"),
            ("src/main/java/app/App.java", "package app;\n@org.springframework.context.annotation.Configuration\nclass App extends Base<Store> {\n    Store create() { return new Store(); }\n    public void make(int n) {}\n    @org.springframework.context.annotation.Bean\n    String text() { return \"ok\"; }\n    @org.springframework.context.annotation.Bean\n    String consumer(Store s) { return \"ok\"; }\n}\n"),
        ],
    );
    let found = findings(&dir, DI);
    assert!(found.iter().all(|f| !f.contains("bean-not-found")), "{found:?}");
}

#[test]
fn spring_factories_with_crlf_line_endings() {
    let dir = project(
        "factories-crlf",
        &[
            ("src/main/resources/META-INF/spring.factories", "org.springframework.boot.autoconfigure.EnableAutoConfiguration=\\\r\napp.ext.FactoryConfig\r\n"),
            ("src/main/java/app/App.java", "package app;\n@org.springframework.boot.autoconfigure.SpringBootApplication\npublic class App {}\n"),
            ("src/main/java/app/Store.java", "package app;\npublic class Store {}\n"),
            ("src/main/java/app/ext/FactoryConfig.java", "package app.ext;\npublic class FactoryConfig {\n    @org.springframework.context.annotation.Bean\n    public app.Store store() { return new app.Store(); }\n}\n"),
            ("src/main/java/app/Consumer.java", "package app;\n@org.springframework.stereotype.Service\nclass Consumer {\n    Consumer(Store store) {}\n}\n"),
        ],
    );
    let found = findings(&dir, DI);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn build_output_is_judged_inside_the_project() {
    // the checkout sits in a folder named `build` (a Nix sandbox, CI agents),
    // and a package is named `build`: both are sources. The project's own
    // build/ and target/ outputs are not, except generated sources.
    let dir = std::env::temp_dir().join(format!("grounds-projects-outputs-{}", std::process::id())).join("build").join("app");
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/a/A.java", "package a;\npublic class A {}\n");
    write(&dir, "src/main/java/com/acme/build/B.java", "package com.acme.build;\npublic class B {}\n");
    write(&dir, "build/tmp/copied/C.java", "package c;\npublic class C {}\n");
    write(&dir, "sub/target/classes/D.java", "package d;\npublic class D {}\n");
    write(&dir, "build/generated/sources/annotationProcessor/java/main/g/G.java", "package g;\npublic class G {}\n");
    let project = Project::load(&dir).unwrap();
    let root = dir.canonicalize().unwrap();
    let mut files: Vec<String> = project.workspace.files.iter().map(|f| f.path.strip_prefix(&root).unwrap().to_string_lossy().to_string()).collect();
    files.sort();
    let _ = std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    assert_eq!(files, vec!["build/generated/sources/annotationProcessor/java/main/g/G.java", "src/main/java/a/A.java", "src/main/java/com/acme/build/B.java"]);
}

#[test]
fn main_sources_of_test_only_modules_are_test_code() {
    let print = "package app;\npublic class Tool {\n    void run() { System.out.println(\"x\"); }\n}\n";
    let dir = project(
        "test-modules",
        &[
            ("webserver/tests/grpc/src/main/java/app/Tool.java", print),
            ("webserver/core/src/main/java/app/Tool.java", print),
            ("webserver/core/src/main/java/app/tests/Tool.java", &print.replace("package app;", "package app.tests;")),
            ("testing-utils/src/main/java/app/Tool.java", print),
        ],
    );
    let project = Project::load(&dir).unwrap();
    let test: Vec<_> = project.workspace.files.iter().filter(|f| f.kind == grounds_engine::workspace::SourceKind::Test).map(|f| f.path.strip_prefix(&project.workspace.root).unwrap().to_string_lossy().to_string()).collect();
    assert_eq!(test, ["webserver/tests/grpc/src/main/java/app/Tool.java"]);
    let mut found = findings(&dir, &["system-out-println"]);
    found.sort();
    assert_eq!(found, ["testing-utils/src/main/java/app/Tool.java:3 system-out-println", "webserver/core/src/main/java/app/Tool.java:3 system-out-println", "webserver/core/src/main/java/app/tests/Tool.java:3 system-out-println"]);
}

#[test]
fn renamed_method_hint_names_the_new_method() {
    let dir = project(
        "rename-hint",
        &[
            ("src/main/java/app/Client.java", "package app;\npublic class Client {\n    public String fetchAll() { return \"\"; }\n    public String fetchByName(String name) { return name; }\n    public void watch(String name) {}\n}\n"),
            ("src/main/java/app/Caller.java", "package app;\nclass Caller {\n    String m(Client c) { return c.fetch(\"x\"); }\n}\n"),
        ],
    );
    let mut project = Project::load(&dir).unwrap();
    project.config.rules.enable = vec!["unresolved-method".into()];
    let (set, options) = build_ruleset(&project);
    let files: Vec<_> = project.workspace.files.iter().map(|f| f.path.clone()).collect();
    let found: Vec<_> = analyze_files(&project, &set, &options, &files).into_iter().map(|f| f.message).collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(found, ["`Client` has no method `fetch`; did you mean `fetchByName`?"]);
}

#[test]
fn inherited_harness_of_test_only_module_asserts() {
    let dir = project(
        "test-module-harness",
        &[
            ("tests/support/src/main/java/app/Harness.java", "package app;\npublic abstract class Harness {\n    protected void rewriteRun(String spec) { }\n}\n"),
            ("tests/support/src/test/java/app/RecipeTest.java", "package app;\nimport org.junit.jupiter.api.Test;\nclass RecipeTest extends Harness {\n    @Test\n    void renames() { rewriteRun(\"a\"); }\n}\n"),
        ],
    );
    let found = findings(&dir, &["test-without-assertion"]);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn java_files_under_resources_are_data() {
    // a test copies a whole fixture project out of src/test/resources; its
    // stub of a library class (whose jar is missing here) must not stand in
    // for that class in the build, nor be analyzed itself
    let dir = project(
        "resources",
        &[
            ("core/pom.xml", "<project/>\n"),
            ("core/src/main/java/app/Reader.java", "package app;\nimport org.acme.xml.Dom;\nclass Reader {\n    String m(Dom d) { return d.getChild(\"a\").getValue() + d.getAttribute(\"b\"); }\n    void broken(Reader r) { r.missing(); }\n}\n"),
            ("core/src/test/java/app/ReaderTest.java", "package app;\nimport org.acme.xml.Dom;\nclass ReaderTest {\n    void t(Dom d) { d.addChild(new Dom(\"x\", \"y\")); }\n}\n"),
            ("core/src/test/resources/fixture-1/pom.xml", "<project/>\n"),
            ("core/src/test/resources/fixture-1/src/main/java/org/acme/xml/Dom.java", "package org.acme.xml;\npublic class Dom {\n    public Dom(String name) {}\n    public String getName() { return \"\"; }\n}\n"),
            ("core/src/test/resources/fixture-1/src/main/java/org/acme/Uses.java", "package org.acme;\nclass Uses {\n    void m(org.acme.xml.Dom d) { d.nope(); }\n}\n"),
            ("core/src/main/resources/templates/Tpl.java", "package tpl;\nclass Tpl {\n    void m(Tpl t) { t.nope(); }\n}\n"),
        ],
    );
    let project = Project::load(&dir).unwrap();
    let files: Vec<_> = project.workspace.files.iter().map(|f| f.path.strip_prefix(&project.workspace.root).unwrap().to_string_lossy().to_string()).collect();
    assert_eq!(files, ["core/src/main/java/app/Reader.java", "core/src/test/java/app/ReaderTest.java"]);
    assert!(!project.index.is_source_class("org.acme.xml.Dom"));
    let found = findings(&dir, COMPILE);
    assert_eq!(found, ["core/src/main/java/app/Reader.java:5 unresolved-method"]);
}

#[test]
fn a_class_declared_in_several_modules_is_read_from_the_copy_the_file_sees() {
    // `p.BaseTest` and `p.Component` exist in modules a and b with different
    // members; module c has neither and sees one of them through a module
    // dependency grounds does not know
    let dir = project(
        "duplicates",
        &[
            ("a/pom.xml", "<project/>\n"),
            ("b/pom.xml", "<project/>\n"),
            ("c/pom.xml", "<project/>\n"),
            ("a/src/test/java/p/BaseTest.java", "package p;\npublic abstract class BaseTest {\n    protected String fromA() { return \"\"; }\n}\n"),
            ("b/src/test/java/p/BaseTest.java", "package p;\npublic abstract class BaseTest {\n    protected String fromB() { return \"\"; }\n    protected Object request() { return null; }\n}\n"),
            ("a/src/main/java/p/Component.java", "package p;\npublic interface Component {\n    void run();\n}\n"),
            ("b/src/main/java/p/Component.java", "package p;\npublic interface Component {\n    void start();\n}\n"),
            ("a/src/test/java/p/ATest.java", "package p;\nclass ATest extends BaseTest {\n    void t() { fromA(); fromB(); }\n}\n"),
            ("b/src/test/java/p/BTest.java", "package p;\nclass BTest extends BaseTest {\n    void t() { fromB(); request(); fromA(); }\n}\n"),
            ("b/src/test/java/p/Impl.java", "package p;\nclass Impl implements Component {\n    public void start() {}\n}\n"),
            ("b/src/main/java/p/Started.java", "package p;\nclass Started implements Component {\n    @Override\n    public void start() {}\n    @Override\n    public void stop() {}\n}\n"),
            ("c/src/test/java/p/CTest.java", "package p;\nclass CTest extends BaseTest {\n    void t() { fromA(); fromB(); nowhere(); }\n}\n"),
            ("c/src/main/java/p/CImpl.java", "package p;\nclass CImpl implements Component {\n    public void start() {}\n}\n"),
        ],
    );
    let mut found = findings(&dir, COMPILE);
    found.sort();
    assert_eq!(
        found,
        [
            "a/src/test/java/p/ATest.java:3 unresolved-method",
            "b/src/main/java/p/Started.java:6 override-matches-nothing",
            "b/src/test/java/p/BTest.java:3 unresolved-method",
        ]
    );
}

#[test]
fn files_that_do_not_parse_as_java_ground_no_compile_break_report() {
    // a Velocity template named .java, whose directives hide half of its
    // members from the parser, and a class in a file with a syntax error
    let template = "package ${package};\n\npublic class Location {\n    private final int line;\n#if ( $tracked )\n    private final String modelId;\n#end\n\n    public Location(int line, String modelId) {\n        this.line = line;\n#if ( $tracked )\n        this.modelId = modelId;\n#end\n    }\n\n    public int getLine() {\n        return line;\n    }\n\n#if ( $tracked )\n    public String getModelId() {\n        return modelId;\n    }\n#end\n\n    public String toString() {\n#if ( $tracked )\n        return getModelId() + \":\" + getLine();\n#else\n        return String.valueOf(getLine());\n#end\n    }\n}\n";
    let dir = project(
        "templates",
        &[
            ("src/mdo/java/Location.java", template),
            ("src/main/java/app/Half.java", "package app;\npublic class Half {\n    public int one() { return 1 }\n    public int two() { return 2; }\n}\n"),
            ("src/main/java/app/Uses.java", "package app;\nclass Uses {\n    int m(Half h) { return h.one() + h.two() + h.three(); }\n    void broken(Uses u) { u.missing(); }\n}\n"),
        ],
    );
    let found = findings(&dir, COMPILE);
    assert_eq!(found, ["src/main/java/app/Uses.java:4 unresolved-method"]);
}

#[test]
fn a_resources_file_checked_by_name_is_not_analyzed() {
    let dir = project(
        "resources-by-name",
        &[
            ("src/main/java/p/Real.java", "package p;\npublic class Real {\n    public void a() {}\n}\n"),
            ("src/test/resources/fixture/src/main/java/p/Uses.java", "package p;\nclass Uses {\n    void m(Real r) { r.b(); }\n}\n"),
        ],
    );
    let mut project = Project::load(&dir).unwrap();
    project.config.rules.enable = COMPILE.iter().map(|s| s.to_string()).collect();
    let (set, options) = build_ruleset(&project);
    let file = project.workspace.root.join("src/test/resources/fixture/src/main/java/p/Uses.java");
    let text = std::fs::read_to_string(&file).unwrap();
    let found = grounds_engine::analyze::analyze_text(&project, &set, &options, &file, &text);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(found.is_empty(), "{:?}", found.iter().map(|f| &f.message).collect::<Vec<_>>());
}
