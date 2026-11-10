//! Inputs found by scripts/fuzz_robust.py that made grounds hang or crash,
//! reduced to the smallest project that showed it.

use grounds_engine::analyze::{analyze_files, build_ruleset, Finding};
use grounds_engine::Project;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grounds-robust-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, text) in files.iter().chain(&[("grounds.toml", "[classpath]\nscan_caches = false\n")]) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn check(dir: &Path, rules: &[&str]) -> Vec<Finding> {
    let mut project = Project::load(dir).unwrap();
    project.config.rules.enable = rules.iter().map(|s| s.to_string()).collect();
    let (set, options) = build_ruleset(&project);
    let files: Vec<_> = project.workspace.files.iter().map(|f| f.path.clone()).collect();
    analyze_files(&project, &set, &options, &files)
}

#[test]
fn many_findings_on_one_long_line_are_capped_and_keep_their_fingerprints() {
    // a file cut off in a run of `;`: 100,000 stray semicolons on one line
    // took a minute (each finding copied and hashed the whole line)
    let line = format!("class A {{ void m() {{ int a = 0{}; }} }}", ";".repeat(5000));
    let dir = project("semicolons", &[("src/main/java/p/A.java", &format!("package p;\n{line}\n"))]);
    let t = Instant::now();
    let fs = check(&dir, &["stray-semicolon"]);
    assert!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
    assert_eq!(fs.len(), grounds_rules::runner::MAX_PER_RULE);
    // the fingerprint is still xxh3 of `rule|path|member|line text|n`
    for (i, f) in fs.iter().take(3).enumerate() {
        let want = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(format!("stray-semicolon|src/main/java/p/A.java|A.m()|{line}|{}", i + 1).as_bytes()));
        assert_eq!(f.fingerprint, want);
        assert_eq!((f.line, f.col), (2, 32 + i as u32));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn columns_count_characters_on_lines_with_non_ascii_text() {
    let dir = project("columns", &[("src/main/java/p/A.java", "package p;\nclass A { String s = \"é\";; void m() { String t = \"日本\";; } }\n")]);
    let fs = check(&dir, &["stray-semicolon"]);
    let cols: Vec<u32> = fs.iter().map(|f| f.col).collect();
    assert_eq!(cols, vec![26, 55], "{fs:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_nested_too_deep_is_skipped_and_the_others_are_checked() {
    // 50,000 nested parentheses overflowed the stack of the binder, 5,000
    // made analysis take seconds; the deepest file of the corpus is 491
    let deep = format!("package p;\nclass D {{ int m(int a) {{ return {}a{}; }} }}\n", "(".repeat(50_000), ")".repeat(50_000));
    let dir = project("deep", &[("src/main/java/p/D.java", &deep), ("src/main/java/p/B.java", "package p;\nclass B { void m() { ;; } }\n")]);
    let fs = check(&dir, &["stray-semicolon"]);
    assert_eq!(fs.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), vec!["src/main/java/p/B.java"; 2]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn named_pipes_in_project_files_do_not_block() {
    // a named pipe called lombok.config or grounds.toml blocked the run forever
    let dir = project("fifo", &[("src/main/java/p/B.java", "package p;\nclass B { void m() { ;; } }\n")]);
    for name in ["lombok.config", ".grounds/source-sets.txt"] {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        if !std::process::Command::new("mkfifo").arg(&p).status().is_ok_and(|s| s.success()) {
            return;
        }
    }
    assert_eq!(check(&dir, &["stray-semicolon"]).len(), 2);
    std::fs::remove_file(dir.join("grounds.toml")).unwrap();
    assert!(std::process::Command::new("mkfifo").arg(dir.join("grounds.toml")).status().unwrap().success());
    assert!(Project::load(&dir).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_class_body_of_a_million_members_is_checked_in_time_and_suppressions_hold() {
    // 3,000,000 `;` in a class body timed out: the suppression check asked
    // every ancestor of every finding for its modifiers, which scanned all
    // members of the class body
    let semis = ";".repeat(300_000);
    let src = format!("package p;\nclass A {{ int x; {semis} }}\n@SuppressWarnings(\"stray-semicolon\") class B {{ int y;; @SuppressWarnings(\"unused\") void m() {{ ;; }} }}\n");
    let dir = project("wide", &[("src/main/java/p/A.java", &src)]);
    let t = Instant::now();
    let fs = check(&dir, &["stray-semicolon"]);
    assert!(t.elapsed() < Duration::from_secs(30), "{:?}", t.elapsed());
    assert_eq!(fs.len(), grounds_rules::runner::MAX_PER_RULE);
    assert!(fs.iter().all(|f| f.line == 2), "B's findings are suppressed");
    let _ = std::fs::remove_dir_all(&dir);
}
