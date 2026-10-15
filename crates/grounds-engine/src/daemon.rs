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
    Check {
        mode: Mode,
        files: Vec<PathBuf>,
        all_lines: bool,
        baseline: bool,
        max: usize,
        min_severity: String,
        json: bool,
        /// ANSI colors (humans at a terminal only, never hooks)
        #[serde(default)]
        color: bool,
        /// Compare with the merge base of this revision and HEAD (CI).
        #[serde(default)]
        base: Option<String>,
        #[serde(default)]
        sarif: bool,
    },
    Baseline { files: Vec<PathBuf>, merge: bool },
    /// A new agent session: baseline the uncommitted work and report the
    /// classpath warnings (again, whether or not the daemon showed them).
    SessionStart,
    ImpactedTests { files: Vec<PathBuf> },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Response {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    /// Classpath warnings, once per session (for stderr; hooks drop them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Identity of the running executable (path + size + mtime), so a client can
/// tell when a daemon was started by an older build.
pub fn build_id() -> String {
    let exe = std::env::current_exe().ok();
    let meta = exe.as_ref().and_then(|e| std::fs::metadata(e).ok());
    let mtime = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    format!("{}:{}:{}", exe.map(|e| e.display().to_string()).unwrap_or_default(), meta.map(|m| m.len()).unwrap_or(0), mtime)
}

/// Per-user directory for daemon sockets: `$XDG_RUNTIME_DIR` or
/// `$TMPDIR/grounds-<uid>`, owned by us and closed to everyone else. `None`
/// (no daemon, analysis in-process) if that cannot be ensured, e.g. another
/// user pre-created the directory in a shared /tmp.
fn socket_dir() -> Option<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let uid = unsafe { libc::geteuid() };
    let dir = match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|d| d.is_absolute()) {
        Some(d) => d.join("grounds"),
        None => std::env::temp_dir().join(format!("grounds-{uid}")),
    };
    let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir);
    let meta = std::fs::symlink_metadata(&dir).ok()?;
    if !meta.is_dir() || meta.uid() != uid {
        return None;
    }
    if meta.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(dir)
}

pub fn socket_path(root: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let h = xxhash_rust::xxh3::xxh3_64(root.to_string_lossy().as_bytes());
    Some(socket_dir()?.join(format!("{h:016x}.sock")))
}

/// Only talk to sockets we own.
fn owned_by_us(sock: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(sock).is_ok_and(|m| m.uid() == unsafe { libc::geteuid() })
}

/// Largest request accepted (paths of changed files).
const MAX_REQUEST: u64 = 4 << 20;

/// Execute a request against a session (used by the daemon and in-process).
pub fn execute(session: &mut Session, req: &Request) -> Response {
    let warnings = match req {
        Request::Check { .. } | Request::Baseline { .. } => session.take_warnings(),
        Request::SessionStart => {
            session.take_warnings();
            session.project.classpath_warnings.iter().map(|w| w.to_string()).collect()
        }
        Request::Ping | Request::ImpactedTests { .. } | Request::Shutdown => Vec::new(),
    };
    let resp = execute_with(session, req, &warnings);
    Response { warnings, ..resp }
}

fn execute_with(session: &mut Session, req: &Request, warnings: &[String]) -> Response {
    match req {
        Request::Ping => Response { code: 0, stdout: format!("ok {}\n", session.root().display()), stderr: build_id(), ..Default::default() },
        Request::Shutdown => Response { code: 0, stdout: "bye\n".into(), ..Default::default() },
        Request::Check { mode, files, all_lines, baseline, max, min_severity, json, color, base, sarif } => {
            let min = Severity::parse(min_severity).unwrap_or(Severity::Warning);
            let base = match base {
                None => None,
                Some(rev) => match crate::git::merge_base(session.root(), rev) {
                    Some(mb) => Some(mb),
                    None => {
                        let msg = format!("grounds: no merge base of `{rev}` and HEAD; fetch enough history (actions/checkout: `fetch-depth: 0`, or `--base HEAD^1` on a pull request's merge commit with `fetch-depth: 2`)\n");
                        return Response { code: 1, stderr: msg, ..Default::default() };
                    }
                },
            };
            let opts = CheckOptions { mode: *mode, files: files.clone(), all_lines: *all_lines, use_baseline: *baseline, min_severity: min, base };
            let report = session.check(&opts);
            let problems = !report.findings.is_empty() || !report.pending.is_empty();
            let text = if *sarif {
                crate::report::sarif_report(&report, session.root(), warnings)
            } else if *json {
                serde_json::to_string_pretty(&report).unwrap_or_default()
            } else {
                render(&report, *max, crate::report::Style { color: *color })
            };
            Response { code: if problems { 2 } else { 0 }, stdout: text, ..Default::default() }
        }
        Request::Baseline { files, merge } => match session.write_baseline(files, *merge) {
            Ok(n) => Response { code: 0, stdout: format!("baseline: {n} accepted finding(s)\n"), ..Default::default() },
            Err(e) => Response { code: 1, stderr: format!("baseline failed: {e}\n"), ..Default::default() },
        },
        Request::SessionStart => execute_with(session, &Request::Baseline { files: Vec::new(), merge: true }, warnings),
        Request::ImpactedTests { files } => {
            let tests = session.impacted_tests(files);
            let mut out = String::new();
            for t in &tests {
                out.push_str(&format!("{}\t{}\t{}\n", t.module, t.task, t.class));
            }
            Response { code: 0, stdout: out, ..Default::default() }
        }
    }
}

/// Serve requests until idle for `idle` or asked to shut down.
pub fn serve(root: &Path, idle: Duration) -> anyhow::Result<()> {
    let Some(sock) = socket_path(root) else { anyhow::bail!("no private directory for the daemon socket") };
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

/// Read one request line within a total deadline (a per-read timeout alone
/// lets a client trickle bytes forever) and a size limit.
fn read_request(stream: &UnixStream, within: Duration) -> Option<String> {
    use std::io::Read;
    let deadline = Instant::now() + within;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        stream.set_read_timeout(Some(left)).ok()?;
        let n = (&*stream).read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.iter().position(|b| *b == b'\n') {
            if end as u64 > MAX_REQUEST {
                return None;
            }
            return Some(String::from_utf8_lossy(&buf[..end]).into_owned());
        }
        if buf.len() as u64 > MAX_REQUEST {
            return None;
        }
    }
}

fn handle(session: &mut Session, stream: UnixStream) -> bool {
    // requests are handled one at a time: a client that stalls, trickles or
    // floods must not block the hooks waiting behind it
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let Some(line) = read_request(&stream, Duration::from_secs(5)) else { return false };
    let req: Request = match serde_json::from_str(&line) {
        Ok(r) => r,
        Err(e) => {
            let resp = Response { code: 1, stderr: format!("bad request: {e}\n"), ..Default::default() };
            let _ = writeln!(&stream, "{}", serde_json::to_string(&resp).unwrap_or_default());
            return false;
        }
    };
    let stop = matches!(req, Request::Shutdown);
    let resp = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(session, &req))).unwrap_or_else(|_| Response { code: 1, stderr: "internal error\n".into(), ..Default::default() });
    let _ = writeln!(&stream, "{}", serde_json::to_string(&resp).unwrap_or_default());
    stop
}

/// Send a request to a running daemon; `None` if none is listening.
pub fn request(root: &Path, req: &Request) -> Option<Response> {
    request_with_timeout(root, req, Duration::from_secs(120))
}

fn request_with_timeout(root: &Path, req: &Request, timeout: Duration) -> Option<Response> {
    let sock = socket_path(root)?;
    if !owned_by_us(&sock) {
        return None;
    }
    let stream = UnixStream::connect(&sock).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok()?;
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
    // a busy or stuck daemon must not hold up a hook: fall back to in-process
    if let Some(p) = request_with_timeout(root, &Request::Ping, Duration::from_secs(2)) {
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

#[cfg(test)]
mod tests {
    use super::{read_request, MAX_REQUEST};
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    #[test]
    fn request_reading_is_bounded() {
        // a normal request
        let (mut a, b) = UnixStream::pair().unwrap();
        a.write_all(b"{\"cmd\":\"ping\"}\n").unwrap();
        assert_eq!(read_request(&b, Duration::from_secs(1)).as_deref(), Some("{\"cmd\":\"ping\"}"));

        // a line longer than the limit, even if its newline arrives in the
        // chunk that crosses it
        let (mut a, b) = UnixStream::pair().unwrap();
        let writer = std::thread::spawn(move || {
            let mut big = vec![b' '; MAX_REQUEST as usize];
            big.extend_from_slice(b"    \n");
            let _ = a.write_all(&big);
        });
        assert_eq!(read_request(&b, Duration::from_secs(5)), None);
        drop(b);
        let _ = writer.join();

        // a client trickling bytes is cut off at the total deadline
        let (mut a, b) = UnixStream::pair().unwrap();
        let writer = std::thread::spawn(move || {
            for _ in 0..20 {
                if a.write_all(b" ").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let t = Instant::now();
        assert_eq!(read_request(&b, Duration::from_millis(500)), None);
        assert!(t.elapsed() < Duration::from_millis(900), "{:?}", t.elapsed());
        drop(b);
        let _ = writer.join();
    }
}
