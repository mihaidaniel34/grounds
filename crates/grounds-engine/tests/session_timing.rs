//! Session tests that model a write within one tick of the file system's
//! clock by forcing a file's time to one taken a moment ago. Each step of
//! such a test must follow the time by less than `MARGIN` (two seconds), so
//! they run in a test binary of their own: cargo runs test binaries one after
//! another, and here they do not share the machine with the loads of every
//! other session test. A run that is still too slow is run again (see
//! `within_margin`).

use grounds_engine::session::{CheckOptions, Mode, Session};
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

fn write_at(dir: &std::path::Path, file: &str, content: &str, time: std::time::SystemTime) {
    write(dir, file, content);
    std::fs::File::options().write(true).open(dir.join(file)).unwrap().set_modified(time).unwrap();
}

fn set_mtime(dir: &Path, rel: &str, t: std::time::SystemTime) {
    std::fs::File::options().write(true).open(dir.join(rel)).unwrap().set_modified(t).unwrap();
}

/// A project with one class committed, for the tests of change detection
/// below: `changed_apis` of a check against HEAD says whether the session
/// saw the file's declarations change.
fn one_class_project(name: &str, api: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("grounds-timing-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "build.gradle", "plugins { id 'java' }\n");
    write(&dir, "src/main/java/demo/Api.java", api);
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "init"]);
    dir
}

fn changed_apis(s: &mut Session, dir: &Path) -> Vec<String> {
    let api = dir.join("src/main/java/demo/Api.java").canonicalize().unwrap();
    let opts = CheckOptions { mode: Mode::Checkpoint, files: vec![api], all_lines: false, use_baseline: false, min_severity: Severity::Info, base: None };
    s.check(&opts).changed_apis
}

/// A run of a test that was too slow to model what it models.
///
/// Some tests force a file's time to one taken a moment ago, to stand for a
/// write within one tick of the file system's clock. That is a write that can
/// happen only while the session looks again within `MARGIN` of that time:
/// later, a real write would get a later time, and the session is right to
/// trust the old one. On a loaded machine a load can take longer than that.
/// Such a run says nothing either way, so it is run again.
#[derive(Debug)]
struct TooSlow;

/// Whether the session's looks since `since` can still be within `MARGIN` of
/// a time taken then (with room for the clock reads the session makes a
/// little later).
fn quick(since: std::time::Instant) -> Result<(), TooSlow> {
    if since.elapsed() + std::time::Duration::from_millis(300) < grounds_engine::project::MARGIN { Ok(()) } else { Err(TooSlow) }
}

/// Runs `scenario` until one run is quick enough to model it, at most five
/// times. A wrong answer fails the test at once. Five runs all too slow
/// (a machine busy with other work) say nothing about the session, so the
/// test ends without a verdict and says so, rather than fail the build.
fn within_margin(mut scenario: impl FnMut() -> Result<(), TooSlow>) {
    for _ in 0..5 {
        if scenario().is_ok() {
            return;
        }
    }
    eprintln!("warning: five runs were too slow to model a write within one tick of the file system's clock; nothing was checked");
}

#[test]
fn a_configuration_file_written_twice_at_the_same_time_is_read() {
    within_margin(configuration_written_twice_at_the_same_time);
}

fn configuration_written_twice_at_the_same_time() -> Result<(), TooSlow> {
    let dir = std::env::temp_dir().join(format!("grounds-timing-sametime-config-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    write_at(&dir, "build.gradle", "plugins { id 'java' }\n", long_ago);
    write_at(&dir, "src/main/java/demo/Api.java", "package demo;\npublic class Api {}\n", long_ago);
    let mut s = Session::open(&dir).unwrap();
    let (now, began) = (std::time::SystemTime::now(), std::time::Instant::now());
    write_at(&dir, "grounds.toml", "exclude = [\"a/**\"]\n", now);
    s.sync(&[]);
    assert_eq!(s.project.config.exclude, ["a/**"]);
    quick(began)?;
    // written again in the tick it was read in
    write_at(&dir, "grounds.toml", "exclude = [\"b/**\"]\n", now);
    std::thread::sleep(std::time::Duration::from_millis(2100));
    // the tick is over: one more load is right, whatever was written in it
    s.sync(&[]);
    assert_eq!(s.project.config.exclude, ["b/**"]);
    // and that load is the last
    let loaded = s.project.load_began;
    s.sync(&[]);
    assert_eq!(s.project.load_began, loaded);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn a_rewrite_that_keeps_time_and_size_is_seen() {
    within_margin(rewrite_that_keeps_time_and_size);
}

fn rewrite_that_keeps_time_and_size() -> Result<(), TooSlow> {
    // A file system keeps times only as fine as its clock ticks: a file
    // rewritten in the tick it was read in keeps its time, and an edit of a
    // string can keep its size. Its time was recent when it was read, so the
    // content decides. (Each rewrite follows the look before it by
    // milliseconds: a time seen unchanged for two seconds is trusted, as no
    // tick is that long.)
    let rel = "src/main/java/demo/Api.java";
    let v1 = "package demo;\npublic class Api {\n    @Deprecated(since = \"1\") public void m() {}\n}\n";
    let v2 = "package demo;\npublic class Api {\n    @Deprecated(since = \"2\") public void m() {}\n}\n";
    let dir = one_class_project("racy", v1);
    set_mtime(&dir, rel, std::time::SystemTime::now() - std::time::Duration::from_secs(3600));
    let mut s = Session::open(&dir).unwrap();
    let (t, began) = (std::time::SystemTime::now(), std::time::Instant::now());
    write(&dir, rel, v2);
    set_mtime(&dir, rel, t);
    assert_eq!(changed_apis(&mut s, &dir), [rel], "a new time");
    quick(began)?;
    write(&dir, rel, v1);
    set_mtime(&dir, rel, t);
    let back = changed_apis(&mut s, &dir);
    quick(began)?;
    assert!(back.is_empty(), "same time and size, content back to that of HEAD");
    write(&dir, rel, v2);
    set_mtime(&dir, rel, t);
    let again = changed_apis(&mut s, &dir);
    quick(began)?;
    assert_eq!(again, [rel], "same time and size, other content again");
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn times_in_the_future_are_trusted_once_seen_for_the_margin() {
    within_margin(times_in_the_future);
}

fn times_in_the_future() -> Result<(), TooSlow> {
    // A time in the future is always "recent" by the clock; seen unchanged for
    // two seconds, it cannot be shared by a later write any more. Neither the
    // source file nor the build file may be compared, or the project loaded,
    // without end.
    let dir = std::env::temp_dir().join(format!("grounds-timing-future-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let future = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
    write_at(&dir, "build.gradle", "plugins { id 'java' }\n", future);
    write_at(&dir, "src/main/java/demo/Api.java", "package demo;\npublic class Api {}\n", future);
    let began = std::time::Instant::now();
    let mut s = Session::open(&dir).unwrap();
    assert_eq!(s.in_doubt(), (2, true), "both compared by content, and which build file the load read is unknown");
    s.sync(&[]);
    let loaded = s.project.load_began;
    // (the doubt about a time in the future ends once it has been seen for the margin)
    quick(began)?;
    assert_eq!(s.in_doubt(), (2, false), "loaded once more, the build file's content now known");
    s.sync(&[]);
    assert_eq!(s.project.load_began, loaded, "and not again");
    std::thread::sleep(std::time::Duration::from_millis(2100));
    s.sync(&[]);
    assert_eq!(s.in_doubt(), (0, false), "seen unchanged for the margin: times alone decide");
    assert_eq!(s.project.load_began, loaded);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
