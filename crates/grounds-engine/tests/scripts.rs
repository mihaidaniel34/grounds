//! The scripts that drive the corpus checks, where they take arguments a
//! test can check without a corpus.

use std::path::Path;
use std::process::Command;

fn script(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts").join(name)
}

#[test]
fn replay_rejects_a_timeout_that_is_not_a_finite_positive_number() {
    // `--timeout inf` passed `> 0` and crashed in subprocess's polling with
    // OverflowError once a query ran (#100, C54); argparse rejects it, as it
    // does such a `--budget`, before anything is run.
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("no python3: skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("grounds-replay-args-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for bad in ["inf", "-inf", "nan", "0", "-1"] {
        let out = Command::new("python3").arg("-I").arg(script("replay.py")).arg(&dir).arg(format!("--timeout={bad}")).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "--timeout {bad}: {err}");
        assert!(err.contains("--timeout must be a positive number of seconds"), "--timeout {bad}: {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
