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
    // the suppressed findings come first in the file: they must not take
    // the place of the others under the cap
    let src = format!("package p;\n@SuppressWarnings(\"stray-semicolon\") class B {{ int y; {semis} }}\nclass A {{ int x; {semis} }}\n");
    let dir = project("wide", &[("src/main/java/p/A.java", &src)]);
    let t = Instant::now();
    let fs = check(&dir, &["stray-semicolon"]);
    assert!(t.elapsed() < Duration::from_secs(30), "{:?}", t.elapsed());
    assert_eq!(fs.len(), grounds_rules::runner::MAX_PER_RULE);
    assert!(fs.iter().all(|f| f.line == 3), "B's findings are suppressed, A's are kept");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_run_of_semicolons_left_in_an_error_node_is_checked_in_time() {
    // reduced from a fuzzed file (17 s for 100,000 `;`): the parser leaves the
    // semicolons as unnamed tokens of one node, and looking for that node's
    // modifiers skipped over all of them, once per finding
    let src = format!("Boolean m(int x) {{{}) {{}} }}\n", ";".repeat(100_000));
    let dir = project("error-semicolons", &[("src/main/java/p/A.java", &src)]);
    let t = Instant::now();
    let fs = check(&dir, &["stray-semicolon"]);
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    assert_eq!(fs.len(), grounds_rules::runner::MAX_PER_RULE);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn string_constants_doubling_over_classes_and_huge_qualified_names_are_bounded() {
    // `K{i}.V = K{i-1}.V + "x" + K{i-1}.V` doubles the string at each class
    // (2^50 bytes over 50 classes), and a file cut off in `x.x.x...` makes a
    // qualified name of a million segments, resolved in quadratic time
    let mut files: Vec<(String, String)> = (0..60)
        .map(|i| {
            let init = if i == 0 { "\"a\"".to_string() } else { format!("K{p}.V + \"x\" + K{p}.V", p = i - 1) };
            (format!("src/main/java/p/K{i}.java"), format!("package p;\npublic class K{i} {{ static final String V = {init}; }}\n"))
        })
        .collect();
    let dots = "x.".repeat(300_000);
    files.push(("src/main/java/p/T.java".into(), format!("package p;\n@{dots}A class T {{ void m() throws {dots}E {{ ;; }} }}\n")));
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let dir = project("doubling", &refs);
    let t = Instant::now();
    let _ = check(&dir, &["stray-semicolon"]);
    assert!(t.elapsed() < Duration::from_secs(30), "{:?}", t.elapsed());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_with_a_constant_cut_off_at_the_nesting_bound_is_not_judged() {
    // 500 final locals, each the one before plus "": a constant, but nested
    // past the bound of constant_value, which answers "not a constant". A
    // rule that reports concatenations of variables then reported this one,
    // and a type computed from the answer stays cached for other rules. So
    // nothing is said about the file; the next one is checked as usual.
    let mut body = String::from("final String v0 = \"a\";\n");
    for i in 1..500 {
        body.push_str(&format!("final String v{i} = v{} + \"\";\n", i - 1));
    }
    let deep = format!("package p;\nclass A {{\n  void deep(StringBuilder sb) {{\n{body}sb.append(v499 + \"b\");\n  }}\n  void plain(StringBuilder sb, String s) {{\n    sb.append(s + \"b\");\n  }}\n}}\n");
    let plain = "package p;\nclass B {\n  void plain(StringBuilder sb, String s) {\n    sb.append(s + \"b\");\n  }\n}\n";
    let dir = project("constant-cutoff", &[("src/main/java/p/A.java", &deep), ("src/main/java/p/B.java", plain)]);
    // (a debug build needs the stack the CLI gives its threads, and more)
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).stack_size(256 << 20).build().unwrap();
    let fs = pool.install(|| check(&dir, &["stringbuilder-append-concat"]));
    let at: Vec<(&str, u32)> = fs.iter().map(|f| (f.path.as_str(), f.line)).collect();
    assert_eq!(at, [("src/main/java/p/B.java", 4)], "{fs:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_daemon_session_stamps_pipes_and_devices_without_reading_them() {
    // A session stamps its configuration files, with a hash of the content
    // while their time is recent: a named pipe just made, or a device the
    // classpath dump lists, would block or never end a plain read.
    let dir = project("stamp-fifo", &[("src/main/java/p/B.java", "package p;\nclass B {}\n"), (".grounds/classpath.txt", "/dev/zero\n")]);
    if !std::process::Command::new("mkfifo").arg(dir.join("lombok.config")).status().is_ok_and(|s| s.success()) {
        return;
    }
    let d = dir.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut s = grounds_engine::session::Session::open(&d).unwrap();
        s.sync(&[]);
        std::fs::write(d.join("src/main/java/p/B.java"), "package p;\nclass B { void m() {} }\n").unwrap();
        s.sync(&[]);
        let _ = tx.send(());
    });
    assert!(rx.recv_timeout(Duration::from_secs(60)).is_ok(), "a session blocked on a pipe or a device");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_package_name_of_half_a_million_segments_is_not_indexed() {
    // found by scripts/fuzz_robust.py (seed 2 case 456 once the fixtures it
    // draws from changed): a file cut off in `package a.x.x.x...`. The index
    // counted every prefix of the package name, 500,000 strings of 500 KB on
    // average: two minutes and tens of gigabytes. The other file is checked.
    let garbage = format!("package a.{}x;\nclass Lost {{ void m() {{ ;; }} }}\n", "x.".repeat(500_000));
    let dir = project("package-name", &[("src/main/java/a/Lost.java", &garbage), ("src/main/java/p/B.java", "package p;\nclass B { void m() { ;; } }\n")]);
    let t = Instant::now();
    let loaded = Project::load(&dir).unwrap();
    assert!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
    // the file declares nothing that can be named
    assert!(!loaded.index.source_class_names().any(|n| n.ends_with("Lost")));
    assert!(loaded.index.class_exists("p.B"));
    let fs = check(&dir, &["stray-semicolon"]);
    assert!(fs.iter().any(|f| f.path.ends_with("p/B.java")), "{fs:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_source_file_replaced_by_a_pipe_is_refused_and_not_taken_as_read() {
    // A pipe is no source file: a fresh load does not list it, so for the
    // session the file is gone (never read, which would block, nor kept as
    // one to read again); a regular file there again is found by the time
    // of its directory, and read
    let dir = project("source-fifo", &[("src/main/java/p/B.java", "package p;\nclass B {}\n"), ("src/main/java/p/Pipe.java", "package p;\nclass Pipe {}\n")]);
    // old times: nothing here loads the project again, which would not list the pipe at all
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    for f in ["grounds.toml", "src/main/java/p/B.java"] {
        std::fs::File::options().write(true).open(dir.join(f)).unwrap().set_modified(old).unwrap();
    }
    // (as the session names it: on macOS the temporary directory is reached through the /var symlink)
    let pipe = dir.canonicalize().unwrap().join("src/main/java/p/Pipe.java");
    let d = dir.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut s = grounds_engine::session::Session::open(&d).unwrap();
        assert!(s.project.index.class_exists("p.Pipe"));
        std::fs::remove_file(d.join("src/main/java/p/Pipe.java")).unwrap();
        assert!(std::process::Command::new("mkfifo").arg(d.join("src/main/java/p/Pipe.java")).status().unwrap().success());
        s.sync(&[]);
        let first = (s.project.index.class_exists("p.Pipe"), s.unread_files());
        s.sync(&[]);
        let second = s.unread_files();
        // a regular file again
        std::fs::remove_file(d.join("src/main/java/p/Pipe.java")).unwrap();
        std::fs::write(d.join("src/main/java/p/Pipe.java"), "package p;\nclass Pipe {}\n").unwrap();
        s.sync(&[]);
        let _ = tx.send((first, second, s.project.index.class_exists("p.Pipe"), s.unread_files()));
    });
    let got = rx.recv_timeout(Duration::from_secs(60));
    assert!(got.is_ok(), "a session blocked on a pipe");
    let ((stub, unread), still, back, none) = got.unwrap();
    assert!(!stub, "no stub for a pipe, as a fresh load");
    assert!(unread.is_empty() && still.is_empty(), "a pipe is no file to read again: {unread:?} {still:?} ({})", pipe.display());
    assert!(back && none.is_empty(), "read once it is a file again");
    let _ = std::fs::remove_dir_all(&dir);
}
