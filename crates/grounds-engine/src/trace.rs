//! A log of what the daemon saw, for finding out after the fact why it
//! answered as it did: with `GROUNDS_DAEMON_LOG=<file>` set, each request,
//! each project load and each file `Session::sync` found changed (or not) is
//! appended to that file, one line each, with the time and process id. Off,
//! it costs one environment lookup per process.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

fn target() -> Option<&'static PathBuf> {
    static T: OnceLock<Option<PathBuf>> = OnceLock::new();
    T.get_or_init(|| std::env::var_os("GROUNDS_DAEMON_LOG").filter(|v| !v.is_empty()).map(PathBuf::from)).as_ref()
}

pub fn enabled() -> bool {
    target().is_some()
}

/// Append a line (built only when logging is on).
pub fn log(line: impl FnOnce() -> String) {
    let Some(p) = target() else { return };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    // one record per line, whatever the paths in it
    let line = line().replace('\n', "\\n").replace('\r', "\\r");
    let text = format!("{}.{:09} pid {} {line}\n", now.as_secs(), now.subsec_nanos(), std::process::id());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = f.write_all(text.as_bytes());
    }
}

/// A modification time as seconds.nanoseconds, for the log.
pub fn time(t: Option<SystemTime>) -> String {
    match t.and_then(|t| t.duration_since(UNIX_EPOCH).ok()) {
        Some(d) => format!("{}.{:09}", d.as_secs(), d.subsec_nanos()),
        None => "-".into(),
    }
}
