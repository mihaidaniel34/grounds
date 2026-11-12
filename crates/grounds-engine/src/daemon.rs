//! A long-lived analysis server keeping a warm [`Session`] per workspace,
//! reached over a Unix socket with one JSON request/response per connection.
//! Every request first re-syncs files that changed (`Session::sync`), so
//! results always reflect what is on disk ("read your writes").

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

/// How a request ended, as the CLI exits with it and the wire carries it:
/// 0, 1 or 2, as before there was a type for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Outcome {
    /// Done, nothing to report (0).
    #[default]
    Clean,
    /// The request could not be carried out (1): what went wrong is on
    /// stderr.
    Failure,
    /// Done, with problems to report (2): hooks hand them to the agent.
    Problems,
}

impl Outcome {
    /// The process exit code, and the number on the wire.
    pub fn code(self) -> i32 {
        match self {
            Outcome::Clean => 0,
            Outcome::Failure => 1,
            Outcome::Problems => 2,
        }
    }

    fn of(code: i32) -> Option<Outcome> {
        [Outcome::Clean, Outcome::Failure, Outcome::Problems].into_iter().find(|o| o.code() == code)
    }
}

impl Serialize for Outcome {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i32(self.code())
    }
}

impl<'de> Deserialize<'de> for Outcome {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Outcome, D::Error> {
        let code = i32::deserialize(d)?;
        Outcome::of(code).ok_or_else(|| serde::de::Error::custom(format!("no outcome has code {code}")))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Response {
    pub code: Outcome,
    pub stdout: String,
    pub stderr: String,
    /// Classpath warnings, once per session (for stderr; hooks drop them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// A daemon's answer to `Ping`: the root it serves and the build it runs
/// (`build_id`). On the wire it is a `Response` as it was before it had a
/// type: `ok <root>` and a newline on stdout, the build on stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pong {
    pub root: PathBuf,
    pub build: String,
}

impl Pong {
    pub fn response(&self) -> Response {
        Response { code: Outcome::Clean, stdout: format!("ok {}\n", self.root.display()), stderr: self.build.clone(), ..Default::default() }
    }

    /// The pong `r` carries, if it is one.
    pub fn read(r: &Response) -> Option<Pong> {
        let root = r.stdout.strip_prefix("ok ")?.strip_suffix('\n')?;
        (r.code == Outcome::Clean).then(|| Pong { root: PathBuf::from(root), build: r.stderr.clone() })
    }
}

/// What a look for the daemon of a workspace found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// No socket, one nothing listens on, or a server that went without
    /// answering.
    Absent,
    /// A socket another user owns, or a server that answers otherwise than a
    /// daemon does.
    Foreign,
    /// A server that took the request and has not answered in the time
    /// given: a daemon still loading its project, or busy with a request.
    NotAnswering,
    /// A daemon of this build.
    Ready(Pong),
    /// A daemon of another build, to be replaced.
    OtherBuild(Pong),
}

/// Why a request to a daemon got no response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unanswered {
    Absent,
    Foreign,
    NotAnswering,
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
            session.project().classpath_warnings.iter().map(|w| w.to_string()).collect()
        }
        Request::Ping | Request::ImpactedTests { .. } | Request::Shutdown => Vec::new(),
    };
    let resp = execute_with(session, req, &warnings);
    Response { warnings, ..resp }
}

fn execute_with(session: &mut Session, req: &Request, warnings: &[String]) -> Response {
    match req {
        Request::Ping => Pong { root: session.root().to_path_buf(), build: build_id() }.response(),
        Request::Shutdown => Response { code: Outcome::Clean, stdout: "bye\n".into(), ..Default::default() },
        Request::Check { mode, files, all_lines, baseline, max, min_severity, json, color, base, sarif } => {
            let min = Severity::parse(min_severity).unwrap_or(Severity::Warning);
            let base = match base {
                None => None,
                Some(rev) => match crate::git::merge_base(session.root(), rev) {
                    Some(mb) => Some(mb),
                    None => {
                        let msg = format!("grounds: no merge base of `{rev}` and HEAD; fetch enough history (actions/checkout: `fetch-depth: 0`, or `--base HEAD^1` on a pull request's merge commit with `fetch-depth: 2`)\n");
                        return Response { code: Outcome::Failure, stderr: msg, ..Default::default() };
                    }
                },
            };
            let opts = CheckOptions { mode: *mode, files: files.clone(), all_lines: *all_lines, use_baseline: *baseline, min_severity: min, base };
            // (git failing: the check cannot tell what changed, and says so)
            let report = match session.check(&opts) {
                Ok(report) => report,
                Err(e) => return Response { code: Outcome::Failure, stderr: format!("grounds: {e}\n"), ..Default::default() },
            };
            let problems = !report.findings.is_empty() || !report.pending.is_empty();
            let text = if *sarif {
                crate::report::sarif_report(&report, session.root(), warnings)
            } else if *json {
                serde_json::to_string_pretty(&report).unwrap_or_default()
            } else {
                render(&report, *max, crate::report::Style { color: *color })
            };
            Response { code: if problems { Outcome::Problems } else { Outcome::Clean }, stdout: text, ..Default::default() }
        }
        Request::Baseline { files, merge } => match session.write_baseline(files, *merge) {
            Ok(n) => Response { code: Outcome::Clean, stdout: format!("baseline: {n} accepted finding(s)\n"), ..Default::default() },
            Err(e) => Response { code: Outcome::Failure, stderr: format!("baseline failed: {e}\n"), ..Default::default() },
        },
        Request::SessionStart => execute_with(session, &Request::Baseline { files: Vec::new(), merge: true }, warnings),
        Request::ImpactedTests { files } => {
            let tests = match session.impacted_tests(files) {
                Ok(tests) => tests,
                Err(e) => return Response { code: Outcome::Failure, stderr: format!("grounds: {e}\n"), ..Default::default() },
            };
            let mut out = String::new();
            for t in &tests {
                out.push_str(&format!("{}\t{}\t{}\n", t.module, t.task, t.class));
            }
            Response { code: Outcome::Clean, stdout: out, ..Default::default() }
        }
    }
}

/// An advisory lock on the file beside a socket path (`<socket>.lock`), for
/// as long as it is held. A server holds it while it looks at the path,
/// removes a stale socket and binds its own, and while it removes its own:
/// two servers starting on one stale socket otherwise both found it stale,
/// and the second removed the socket the first had just bound, leaving the
/// first serving no one until its idle exit.
fn lock(sock: &Path) -> std::io::Result<std::fs::File> {
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(sock.with_extension("lock"))?;
    file.lock()?;
    Ok(file)
}

/// The socket a server bound, owned by it: dropped, on every way out of
/// `serve` (an error after binding included), it removes the socket file,
/// and only if the path still names the socket it bound (the same device
/// and inode). A file put in its place, another server's socket among them,
/// is left alone.
pub struct Bound {
    listener: UnixListener,
    path: PathBuf,
    /// The device and inode of the socket file bound.
    id: (u64, u64),
}

impl Bound {
    /// Bind `sock`, unless a server answers there (`for_root`: named in the
    /// error); a socket nothing listens on is stale and goes first.
    pub fn bind(sock: &Path, for_root: &Path) -> anyhow::Result<Bound> {
        use std::os::unix::fs::MetadataExt;
        let _lock = lock(sock)?;
        if sock.exists() {
            // another daemon alive?
            if UnixStream::connect(sock).is_ok() {
                anyhow::bail!("a daemon is already running for {}", for_root.display());
            }
            let _ = std::fs::remove_file(sock);
        }
        let listener = UnixListener::bind(sock)?;
        match std::fs::symlink_metadata(sock) {
            Ok(m) => Ok(Bound { listener, path: sock.to_path_buf(), id: (m.dev(), m.ino()) }),
            Err(e) => {
                // (bound under the lock just now: what is there is ours)
                let _ = std::fs::remove_file(sock);
                Err(e.into())
            }
        }
    }

    pub fn listener(&self) -> &UnixListener {
        &self.listener
    }
}

impl Drop for Bound {
    fn drop(&mut self) {
        use std::os::unix::fs::MetadataExt;
        // (under the lock: no server removes or binds between the look and
        // the removal; without it, the look still holds)
        let _lock = lock(&self.path);
        if std::fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.id) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Serve requests until idle for `idle` or asked to shut down.
pub fn serve(root: &Path, idle: Duration) -> anyhow::Result<()> {
    let Some(sock) = socket_path(root) else { anyhow::bail!("no private directory for the daemon socket") };
    let bound = Bound::bind(&sock, root)?;
    let listener = bound.listener();
    listener.set_nonblocking(true)?;
    crate::trace::log(|| format!("daemon {}: listening", root.display()));
    let mut session = Session::open(root)?;
    crate::trace::log(|| format!("daemon {}: loaded, serving", root.display()));
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
            let resp = Response { code: Outcome::Failure, stderr: format!("bad request: {e}\n"), ..Default::default() };
            let _ = writeln!(&stream, "{}", serde_json::to_string(&resp).unwrap_or_default());
            return false;
        }
    };
    let stop = matches!(req, Request::Shutdown);
    crate::trace::log(|| format!("request {}", line.chars().take(300).collect::<String>()));
    let resp = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(session, &req))).unwrap_or_else(|_| Response { code: Outcome::Failure, stderr: "internal error\n".into(), ..Default::default() });
    let _ = writeln!(&stream, "{}", serde_json::to_string(&resp).unwrap_or_default());
    crate::trace::log(|| format!("answered, code {}", resp.code.code()));
    stop
}

/// Send a request to a running daemon; `None` if none answered.
pub fn request(root: &Path, req: &Request) -> Option<Response> {
    let sock = socket_path(root)?;
    ask(&sock, req, Duration::from_secs(120)).ok()
}

/// Look for the daemon of `root`, waiting `within` for its answer.
pub fn probe(root: &Path, within: Duration) -> Probe {
    match socket_path(root) {
        Some(sock) => probe_at(&sock, within),
        None => Probe::Absent,
    }
}

fn probe_at(sock: &Path, within: Duration) -> Probe {
    match ask(sock, &Request::Ping, within) {
        Ok(r) => match Pong::read(&r) {
            Some(pong) if pong.build == build_id() => Probe::Ready(pong),
            Some(pong) => Probe::OtherBuild(pong),
            None => Probe::Foreign,
        },
        Err(Unanswered::Absent) => Probe::Absent,
        Err(Unanswered::Foreign) => Probe::Foreign,
        Err(Unanswered::NotAnswering) => Probe::NotAnswering,
    }
}

/// `req` sent to the server at `sock`, and its response within `timeout`.
fn ask(sock: &Path, req: &Request, timeout: Duration) -> Result<Response, Unanswered> {
    if !owned_by_us(sock) {
        return Err(if std::fs::symlink_metadata(sock).is_ok() { Unanswered::Foreign } else { Unanswered::Absent });
    }
    let stream = UnixStream::connect(sock).map_err(|_| Unanswered::Absent)?;
    stream.set_read_timeout(Some(timeout)).map_err(|_| Unanswered::Absent)?;
    stream.set_write_timeout(Some(Duration::from_secs(10))).map_err(|_| Unanswered::Absent)?;
    let req = serde_json::to_string(req).map_err(|_| Unanswered::Absent)?;
    writeln!(&stream, "{req}").map_err(|_| Unanswered::Absent)?;
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => Err(Unanswered::Absent),
        Ok(_) => serde_json::from_str(&line).map_err(|_| Unanswered::Foreign),
        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Err(Unanswered::NotAnswering),
        Err(_) => Err(Unanswered::Absent),
    }
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
    match probe(root, Duration::from_secs(2)) {
        Probe::Ready(_) => {
            if let Some(r) = request(root, req) {
                return Ok(r);
            }
        }
        Probe::OtherBuild(_) => {
            // started by a different build: replace it
            let _ = request(root, &Request::Shutdown);
            std::thread::sleep(Duration::from_millis(50));
        }
        // (a daemon still loading its project is not waited for, and is
        // asked to start again, which it declines: answered in-process, as
        // with no daemon)
        Probe::NotAnswering | Probe::Absent | Probe::Foreign => {}
    }
    if start_daemon {
        crate::trace::log(|| format!("client: no daemon for {}, starting one and answering in-process", root.display()));
        spawn_background(root);
    }
    let mut s = Session::open(root)?;
    Ok(execute(&mut s, req))
}

#[cfg(test)]
mod tests {
    use super::{read_request, Bound, MAX_REQUEST};
    use std::io::Write;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::time::{Duration, Instant};

    fn socket_in(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("grounds-bound-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("d.sock")
    }

    /// The socket a server bound goes when the server does, and only while
    /// the path is still that socket.
    #[test]
    fn a_bound_socket_is_removed_when_dropped_and_only_if_it_is_still_the_one_bound() {
        let sock = socket_in("own");
        let root = sock.parent().unwrap();
        drop(Bound::bind(&sock, root).unwrap());
        assert!(!sock.exists(), "its own socket goes");
        // a server answers there: no second one, and its socket stays
        let first = Bound::bind(&sock, root).unwrap();
        assert!(Bound::bind(&sock, root).is_err_and(|e| e.to_string().contains("already running")));
        assert!(sock.exists());
        // a file put in its place stays
        std::fs::remove_file(&sock).unwrap();
        std::fs::write(&sock, "another's").unwrap();
        drop(first);
        assert_eq!(std::fs::read_to_string(&sock).unwrap(), "another's");
        std::fs::remove_file(&sock).unwrap();
        // and so does another server's socket bound there since
        let bound = Bound::bind(&sock, root).unwrap();
        std::fs::remove_file(&sock).unwrap();
        let other = UnixListener::bind(&sock).unwrap();
        drop(bound);
        assert!(sock.exists() && UnixStream::connect(&sock).is_ok(), "the other server's socket");
        drop(other);
        // a socket nothing listens on is stale: bound over
        assert!(sock.exists());
        drop(Bound::bind(&sock, root).unwrap());
        assert!(!sock.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// The outcome and the pong are what the wire carried before they had
    /// types: the same bytes either way.
    #[test]
    fn outcomes_and_pongs_are_the_same_bytes_on_the_wire() {
        use super::{Outcome, Pong, Response};
        for (o, n) in [(Outcome::Clean, 0), (Outcome::Failure, 1), (Outcome::Problems, 2)] {
            let r = Response { code: o, stdout: "x".into(), ..Default::default() };
            assert_eq!(serde_json::to_string(&r).unwrap(), format!("{{\"code\":{n},\"stdout\":\"x\",\"stderr\":\"\"}}"));
            assert_eq!(serde_json::from_str::<Response>(&format!("{{\"code\":{n},\"stdout\":\"\",\"stderr\":\"\"}}")).unwrap().code, o);
            assert_eq!(o.code(), n);
        }
        assert!(serde_json::from_str::<Response>("{\"code\":3,\"stdout\":\"\",\"stderr\":\"\"}").is_err());
        let pong = Pong { root: "/w/p".into(), build: "/bin/grounds:1:2".into() };
        assert_eq!(serde_json::to_string(&pong.response()).unwrap(), "{\"code\":0,\"stdout\":\"ok /w/p\\n\",\"stderr\":\"/bin/grounds:1:2\"}");
        assert_eq!(Pong::read(&pong.response()), Some(pong));
        assert_eq!(Pong::read(&Response { stdout: "bye\n".into(), ..Default::default() }), None);
    }

    /// A probe tells a daemon still loading (it has bound its socket and
    /// accepts nothing yet) from no daemon, from another program, and from
    /// a daemon of another build.
    #[test]
    fn a_probe_tells_what_it_found() {
        use super::{build_id, probe_at, Pong, Probe};
        let sock = socket_in("probe");
        let within = Duration::from_millis(300);
        assert_eq!(probe_at(&sock, within), Probe::Absent, "no socket");
        let loading = UnixListener::bind(&sock).unwrap();
        assert_eq!(probe_at(&sock, within), Probe::NotAnswering, "bound, not answering yet");
        drop(loading);
        assert_eq!(probe_at(&sock, within), Probe::Absent, "a socket nothing listens on");
        std::fs::remove_file(&sock).unwrap();
        // a server answering each connection with `answer`
        let serve = |answer: String| {
            let listener = UnixListener::bind(&sock).unwrap();
            std::thread::spawn(move || {
                if let Ok((stream, _)) = listener.accept() {
                    let _ = super::read_request(&stream, Duration::from_secs(5));
                    let _ = writeln!(&stream, "{answer}");
                }
            })
        };
        let cases = [
            ("not json".to_string(), Probe::Foreign),
            (serde_json::to_string(&Pong { root: "/r".into(), build: "an older build".into() }.response()).unwrap(), Probe::OtherBuild(Pong { root: "/r".into(), build: "an older build".into() })),
            (serde_json::to_string(&Pong { root: "/r".into(), build: build_id() }.response()).unwrap(), Probe::Ready(Pong { root: "/r".into(), build: build_id() })),
        ];
        for (answer, found) in cases {
            let server = serve(answer);
            assert_eq!(probe_at(&sock, Duration::from_secs(5)), found);
            server.join().unwrap();
            std::fs::remove_file(&sock).unwrap();
        }
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    }

    /// A daemon whose project cannot be loaded fails after it bound its
    /// socket: the socket goes with it, rather than staying for clients to
    /// find nothing behind.
    #[test]
    fn a_daemon_that_cannot_load_its_project_leaves_no_socket() {
        let root = std::env::temp_dir().join(format!("grounds-serve-fails-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("grounds.toml"), "this is = = not toml\n").unwrap();
        let Some(sock) = super::socket_path(&root) else { return };
        assert!(super::serve(&root, Duration::from_secs(60)).is_err());
        assert!(!sock.exists(), "{}", sock.display());
        let _ = std::fs::remove_dir_all(&root);
    }

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
