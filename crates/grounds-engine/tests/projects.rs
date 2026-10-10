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
