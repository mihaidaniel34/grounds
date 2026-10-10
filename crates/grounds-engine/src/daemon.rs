//! A long-lived analysis server keeping a warm [`Session`] per workspace,
//! reached over a Unix socket with one JSON request/response per connection.
//! Every request first re-syncs files whose mtime changed, so results always
//! reflect what is on disk ("read your writes").

use crate::session::{render, CheckOptions, Mode, Session};
use grounds_rules::Severity;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    Ping,
    Check { mode: Mode, files: Vec<PathBuf>, all_lines: bool, baseline: bool, max: usize, min_severity: String, json: bool },
    Baseline { files: Vec<PathBuf>, merge: bool },
    ImpactedTests { files: Vec<PathBuf> },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Response {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Identity of the running executable (path + size + mtime), so a client can
/// tell when a daemon was started by an older build.
pub fn build_id() -> String {
    let exe = std::env::current_exe().ok();
    let meta = exe.as_ref().and_then(|e| std::fs::metadata(e).ok());
    let mtime = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    format!("{}:{}:{}", exe.map(|e| e.display().to_string()).unwrap_or_default(), meta.map(|m| m.len()).unwrap_or(0), mtime)
}

pub fn socket_path(root: &Path) -> PathBuf {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let h = xxhash_rust::xxh3::xxh3_64(root.to_string_lossy().as_bytes());
    std::env::temp_dir().join(format!("grounds-{h:016x}.sock"))
}

/// Execute a request against a session (used by the daemon and in-process).
pub fn execute(session: &mut Session, req: &Request) -> Response {
    match req {
        Request::Ping => Response { code: 0, stdout: format!("ok {}\n", session.root().display()), stderr: build_id() },
        Request::Shutdown => Response { code: 0, stdout: "bye\n".into(), stderr: String::new() },
        Request::Check { mode, files, all_lines, baseline, max, min_severity, json } => {
            let min = Severity::parse(min_severity).unwrap_or(Severity::Warning);
            let opts = CheckOptions { mode: *mode, files: files.clone(), all_lines: *all_lines, use_baseline: *baseline, min_severity: min };
            let report = session.check(&opts);
            let problems = !report.findings.is_empty() || !report.pending.is_empty();
            let text = if *json { serde_json::to_string_pretty(&report).unwrap_or_default() } else { render(&report, *max) };
            Response { code: if problems { 2 } else { 0 }, stdout: text, stderr: String::new() }
        }
        Request::Baseline { files, merge } => match session.write_baseline(files, *merge) {
            Ok(n) => Response { code: 0, stdout: format!("baseline: {n} accepted finding(s)\n"), stderr: String::new() },
            Err(e) => Response { code: 1, stdout: String::new(), stderr: format!("baseline failed: {e}\n") },
        },
        Request::ImpactedTests { files } => {
            let tests = session.impacted_tests(files);
            let mut out = String::new();
            for (m, t) in &tests {
                out.push_str(&format!("{m}\t{t}\n"));
            }
            Response { code: 0, stdout: out, stderr: String::new() }
        }
    }
}

/// Serve requests until idle for `idle` or asked to shut down.
pub fn serve(root: &Path, idle: Duration) -> anyhow::Result<()> {
    let sock = socket_path(root);
    if sock.exists() {
        // another daemon alive?
        if UnixStream::connect(&sock).is_ok() {
            anyhow::bail!("a daemon is already running for {}", root.display());
        }
        let _ = std::fs::remove_file(&sock);
    }
    let listener = UnixListener::bind(&sock)?;
    listener.set_nonblocking(true)?;
    let mut session = Session::open(root)?;
    let mut last = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                last = Instant::now();
                let _ = stream.set_nonblocking(false);
                let stop = handle(&mut session, stream);
                if stop {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if last.elapsed() > idle {
                    break;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let _ = std::fs::remove_file(&sock);
    Ok(())
}

fn handle(session: &mut Session, stream: UnixStream) -> bool {
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return false;
    }
    let req: Request = match serde_json::from_str(&line) {
        Ok(r) => r,
        Err(e) => {
            let resp = Response { code: 1, stdout: String::new(), stderr: format!("bad request: {e}\n") };
            let _ = writeln!(&stream, "{}", serde_json::to_string(&resp).unwrap_or_default());
            return false;
        }
    };
    let stop = matches!(req, Request::Shutdown);
    let resp = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(session, &req))).unwrap_or_else(|_| Response { code: 1, stdout: String::new(), stderr: "internal error\n".into() });
    let _ = writeln!(&stream, "{}", serde_json::to_string(&resp).unwrap_or_default());
    stop
}

/// Send a request to a running daemon; `None` if none is listening.
pub fn request(root: &Path, req: &Request) -> Option<Response> {
    let sock = socket_path(root);
    let stream = UnixStream::connect(&sock).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(120))).ok()?;
    writeln!(&stream, "{}", serde_json::to_string(req).ok()?).ok()?;
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

/// Start a daemon in the background (detached) for later requests.
pub fn spawn_background(root: &Path) {
    let Ok(exe) = std::env::current_exe() else { return };
    let _ = std::process::Command::new(exe)
        .arg("daemon")
        .arg("--root")
        .arg(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Run a request through the daemon if one is running, otherwise in-process
/// (and optionally start a daemon for next time).
pub fn run(root: &Path, req: &Request, start_daemon: bool) -> anyhow::Result<Response> {
    if let Some(p) = request(root, &Request::Ping) {
        if p.stderr == build_id() {
            if let Some(r) = request(root, req) {
                return Ok(r);
            }
        } else {
            // started by a different build: replace it
            let _ = request(root, &Request::Shutdown);
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    if start_daemon {
        spawn_background(root);
    }
    let mut s = Session::open(root)?;
    Ok(execute(&mut s, req))
}
