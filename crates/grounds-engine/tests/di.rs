//! DI model on multi-package projects (not expressible as one-file fixtures).

use grounds_engine::analyze::{analyze_files, build_ruleset};
use grounds_engine::Project;
use std::path::Path;

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn di_findings(dir: &Path) -> Vec<String> {
    let mut project = Project::load(dir).unwrap();
    project.config.rules.enable = ["bean-not-found", "ambiguous-bean", "qualifier-not-found", "bean-name-conflict"].iter().map(|s| s.to_string()).collect();
    let (rules, options) = build_ruleset(&project);
    let files: Vec<_> = project.workspace.files.iter().map(|f| f.path.clone()).collect();
    analyze_files(&project, &rules, &options, &files).into_iter().map(|f| format!("{}:{} {}", f.path, f.line, f.rule)).collect()
}

#[test]
fn imported_and_unscanned_configurations() {
    let dir = std::env::temp_dir().join(format!("grounds-di-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/app/App.java", "package app;\nimport org.springframework.boot.autoconfigure.SpringBootApplication;\nimport org.springframework.context.annotation.Import;\n@SpringBootApplication\n@Import(other.ExternalConfig.class)\npublic class App {}\n");
    write(&dir, "src/main/java/app/Consumer.java", "package app;\nimport org.springframework.stereotype.Service;\n@Service\nclass Consumer {\n    Consumer(other.Store store,\n             other.Cache cache,\n             other.Missing missing) {}\n}\n");
    // imported explicitly: a real bean
    write(&dir, "src/main/java/other/ExternalConfig.java", "package other;\nimport org.springframework.context.annotation.*;\n@Configuration\npublic class ExternalConfig {\n    @Bean public Store store() { return new Store(); }\n}\n");
    write(&dir, "src/main/java/other/Store.java", "package other;\npublic class Store {}\n");
    // outside the scan and not imported: may still be registered (auto-configuration)
    write(&dir, "src/main/java/other/CacheConfig.java", "package other;\nimport org.springframework.context.annotation.*;\n@Configuration\npublic class CacheConfig {\n    @Bean public Cache cache() { return new Cache(); }\n}\n");
    write(&dir, "src/main/java/other/Cache.java", "package other;\npublic class Cache {}\n");
    write(&dir, "src/main/java/other/Missing.java", "package other;\npublic interface Missing {}\n");
    let found = di_findings(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(found, vec!["src/main/java/app/Consumer.java:7 bean-not-found".to_string()], "{found:?}");
}

#[test]
fn registration_follows_spring() {
    let dir = std::env::temp_dir().join(format!("grounds-di-reg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/app/App.java", "package app;\nimport org.springframework.boot.autoconfigure.SpringBootApplication;\nimport org.springframework.context.annotation.Import;\n@SpringBootApplication\n@Import(other.Plain.class)\npublic class App {}\n");
    // @Import on a class that is never registered has no effect
    write(&dir, "src/main/java/app/Unused.java", "package app;\n@org.springframework.context.annotation.Import(other.External.class)\nclass Unused {}\n");
    write(&dir, "src/main/java/app/Store.java", "package app;\npublic interface Store {}\n");
    write(&dir, "src/main/java/app/DbStore.java", "package app;\n@org.springframework.stereotype.Service\nclass DbStore implements Store {}\n");
    write(&dir, "src/main/java/app/Consumer.java", "package app;\n@org.springframework.stereotype.Service\nclass Consumer {\n    Consumer(Store store,\n             other.Plain plain) {}\n}\n");
    write(&dir, "src/main/java/other/External.java", "package other;\nimport org.springframework.context.annotation.*;\n@Configuration\npublic class External {\n    @Bean public app.Store otherStore() { return null; }\n}\n");
    // a plain class registered by @Import is a bean
    write(&dir, "src/main/java/other/Plain.java", "package other;\npublic class Plain {}\n");
    // an unscanned component's own dependencies are not judged
    write(&dir, "src/main/java/other/Disabled.java", "package other;\ninterface Missing {}\n@org.springframework.stereotype.Service\npublic class Disabled {\n    public Disabled(Missing dependency) {}\n}\n");
    let found = di_findings(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(found.is_empty(), "{found:?}");
}
