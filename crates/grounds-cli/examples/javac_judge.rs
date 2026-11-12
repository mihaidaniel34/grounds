//! Prototype: javac judges grounds' compile-break findings. For each finding
//! (`path:line rule: message`, as scripts/smoke.py lists them) the file is
//! attributed by the JDK's compiler (javac-helper/) in the world grounds
//! judged it in, and the finding is
//!
//! - confirmed: javac reports an error of the claimed kind at that line
//!   (naming the member, for the unresolved-* rules), and nothing the claim
//!   rests on is a class javac could not fully see;
//! - refuted: javac attributes the line without such an error, and the
//!   classes the claim is about are fully visible to javac (for a member:
//!   javac resolved that very name at that line, to the member it gives);
//! - cannot judge, with the reason: a class the claim rests on has a missing
//!   supertype, an erroneous receiver or argument, a failed import, javac
//!   crashed, the file is in no source root, or javac said something else;
//! - not a compile claim: the wiring rules.
//!
//! The world: grounds indexes every Java file of the workspace by the class
//! names it declares, whatever directory it is in. javac finds a class by
//! its package's directory under a source root, so each indexed file whose
//! directory ends in its package's path gives a root (the directory minus
//! that path), and the sourcepath of a file is every such root: its own
//! first, then its module's, then the rest (javac takes the first copy of a
//! class declared in several roots; grounds prefers the copy of the file's
//! own module). A file whose directory does not match its package is in
//! no root: such files (up to 400 of them; beyond, their findings are not
//! judged) are entered in every task, so that javac sees their classes, and
//! one of them with findings is attributed as given. The classpath is the jars grounds loaded, the
//! release the one grounds chose for the file's module (at least 8, at most
//! the running JDK's).
//!
//!   javac_judge [--jobs N] [--max-roots N] <project root> <findings list> <out.json>

use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// Files in no root are entered in every task up to this many.
const MAX_UNROOTED: usize = 400;

const WIRING: &[&str] = &["bean-not-found", "ambiguous-bean", "qualifier-not-found", "bean-name-conflict", "circular-constructor-dependency"];

struct Helper {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Helper {
    fn spawn(classes: &Path) -> anyhow::Result<Helper> {
        let mut child = Command::new("java")
            .args(["--add-exports", "jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED", "-Xss64m", "-Xmx1500m", "-XX:+UseSerialGC", "-cp"])
            .arg(classes)
            .arg("JavacHelper")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = BufWriter::new(child.stdin.take().expect("piped"));
        let stdout = BufReader::new(child.stdout.take().expect("piped"));
        Ok(Helper { child, stdin, stdout, next: 0 })
    }

    fn ask(&mut self, mut req: Value) -> anyhow::Result<Vec<Value>> {
        self.next += 1;
        req["id"] = json!(self.next);
        writeln!(self.stdin, "{req}")?;
        self.stdin.flush()?;
        let mut out = Vec::new();
        loop {
            let mut line = String::new();
            anyhow::ensure!(self.stdout.read_line(&mut line)? > 0, "helper exited");
            let v: Value = serde_json::from_str(&line)?;
            if v.get("done").is_some() {
                if let Some(e) = v.get("error") {
                    anyhow::bail!("helper: {e}");
                }
                return Ok(out);
            }
            out.push(v);
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn compile_helper(out: &Path) -> anyhow::Result<PathBuf> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../javac-helper/JavacHelper.java");
    let dir = out.join("helper-classes");
    std::fs::create_dir_all(&dir)?;
    let st = Command::new("javac").args(["--add-exports", "jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED", "-nowarn", "-d"]).arg(&dir).arg(&src).stderr(Stdio::null()).status()?;
    anyhow::ensure!(st.success(), "javac failed on {}", src.display());
    Ok(dir)
}

struct Finding {
    path: String,
    line: i64,
    rule: String,
    message: String,
}

fn parse_finding(s: &str) -> Option<Finding> {
    let (head, message) = s.split_once(": ")?;
    let (loc, rule) = head.rsplit_once(' ')?;
    let (path, line) = loc.rsplit_once(':')?;
    Some(Finding { path: path.to_string(), line: line.parse().ok()?, rule: rule.to_string(), message: message.to_string() })
}

/// The member a finding names: "has no method `m`", "Method `m` is not
/// defined", "No `m` in `C` accepts", "has no field `f`", "no constructor".
fn member_of(f: &Finding) -> Option<String> {
    let m = &f.message;
    let after = |pat: &str| m.find(pat).map(|i| m[i + pat.len()..].split('`').next().unwrap_or("").to_string());
    after("has no method `")
        .or_else(|| after("Method `"))
        .or_else(|| after("No `"))
        .or_else(|| after("has no field `"))
        .or_else(|| after("Field `"))
        .or_else(|| after("field `"))
        .or_else(|| after("method `"))
        .or_else(|| m.split('`').nth(1).map(str::to_string))
}

fn word_in(text: &str, w: &str) -> bool {
    !w.is_empty()
        && text.match_indices(w).any(|(i, _)| {
            let before = text[..i].chars().last();
            let after = text[i + w.len()..].chars().next();
            !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
}

/// javac's codes for the claim of each rule.
fn codes_for(rule: &str) -> &'static [&'static str] {
    match rule {
        "unresolved-method" => &[
            "compiler.err.cant.resolve.location.args",
            "compiler.err.cant.resolve.args",
            "compiler.err.cant.resolve.location.args.params",
            "compiler.err.cant.apply.symbol",
            "compiler.err.cant.apply.symbols",
        ],
        "unresolved-field" => &["compiler.err.cant.resolve.location", "compiler.err.cant.resolve"],
        "unresolved-constructor" => &["compiler.err.cant.apply.symbol", "compiler.err.cant.apply.symbols", "compiler.err.cant.resolve.location.args"],
        "override-matches-nothing" => &["compiler.err.method.does.not.override.superclass"],
        "unimplemented-abstract-method" => &["compiler.err.does.not.override.abstract"],
        "missing-jdk-import" => {
            &["compiler.err.cant.resolve.location", "compiler.err.doesnt.exist", "compiler.err.cant.resolve", "compiler.err.not.def.public.cant.access", "compiler.err.package.not.visible"]
        }
        _ => &[],
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn i(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(-1)
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// The verdict of one finding against javac's answer for its file.
fn judge(f: &Finding, ans: &Value) -> (String, String, Value) {
    if WIRING.contains(&f.rule.as_str()) {
        return ("not a compile claim".into(), String::new(), Value::Null);
    }
    if let Some(c) = ans.get("crash").and_then(Value::as_str) {
        return ("cannot judge".into(), "javac crashed".into(), json!(c.lines().next().unwrap_or("")));
    }
    let empty = Vec::new();
    let diags: Vec<&Value> = ans.get("diags").and_then(Value::as_array).unwrap_or(&empty).iter().filter(|d| s(d, "kind") == "ERROR").collect();
    let sites: Vec<&Value> = ans.get("sites").and_then(Value::as_array).unwrap_or(&empty).iter().collect();
    let classes: Vec<&Value> = ans.get("classes").and_then(Value::as_array).unwrap_or(&empty).iter().collect();
    let at_line: Vec<&&Value> = diags.iter().filter(|d| i(d, "line") == f.line).collect();
    let codes = codes_for(&f.rule);
    let member = member_of(f).unwrap_or_default();
    let named = |d: &Value| match f.rule.as_str() {
        "unresolved-method" | "unresolved-field" => word_in(s(d, "msg"), &member),
        "missing-jdk-import" => true,
        _ => true,
    };
    let matching: Vec<&&&Value> = at_line.iter().filter(|d| codes.contains(&s(d, "code")) && named(d)).collect();
    // the innermost class around the line, and the one declared on it
    let around = classes.iter().filter(|c| i(c, "line") <= f.line && f.line <= i(c, "end_line")).min_by_key(|c| i(c, "end_line") - i(c, "line"));
    let declared_here = classes.iter().find(|c| i(c, "line") == f.line).or(around);
    let detail = |extra: Value| json!({"javac_errors_on_line": at_line.iter().map(|d| format!("{} {}", s(d, "code"), s(d, "msg"))).collect::<Vec<_>>(), "more": extra});
    match f.rule.as_str() {
        "unresolved-method" | "unresolved-field" | "unresolved-constructor" => {
            let kinds: &[&str] = match f.rule.as_str() {
                "unresolved-method" => &["call"],
                "unresolved-field" => &["field", "unres"],
                _ => &["ctor"],
            };
            let site = sites
                .iter()
                .filter(|x| i(x, "line") == f.line && kinds.contains(&s(x, "k")))
                .find(|x| f.rule == "unresolved-constructor" || s(x, "name") == member || (s(x, "k") == "unres" && s(x, "name").is_empty()));
            let flags = |x: &Value| -> Option<String> {
                if b(x, "hier") {
                    Some(format!("rests on a class with a missing supertype: {}", s(x, "missing")))
                } else if b(x, "argerr") {
                    Some("erroneous receiver or argument".into())
                } else if b(x, "imp") {
                    Some("a failed import may have rebound a name".into())
                } else if b(x, "simp") {
                    Some("a static import javac could not resolve may name it".into())
                } else {
                    None
                }
            };
            if !matching.is_empty() {
                // an error of the claimed kind; is it explained by what javac could not see?
                if let Some(why) = site.and_then(|x| flags(x)) {
                    return ("cannot judge".into(), why, detail(json!(site)));
                }
                if site.is_none() {
                    if let Some(c) = around.filter(|c| !s(c, "missing").is_empty()) {
                        return ("cannot judge".into(), format!("rests on a class with a missing supertype: {}", s(c, "missing")), detail(Value::Null));
                    }
                }
                return ("confirmed".into(), s(matching[0], "code").to_string(), detail(json!(site)));
            }
            match site {
                Some(x) if !b(x, "err") && s(x, "k") != "unres" => match flags(x) {
                    Some(why) => ("cannot judge".into(), why, detail(json!(x))),
                    None => ("refuted".into(), format!("javac resolved it to {}.{}{}", s(x, "owner"), s(x, "name"), s(x, "desc")), detail(json!(x))),
                },
                Some(x) => match flags(x) {
                    Some(why) => ("cannot judge".into(), why, detail(json!(x))),
                    None => ("cannot judge".into(), "javac: erroneous there, with another error".into(), detail(json!(x))),
                },
                None if at_line.is_empty() => ("cannot judge".into(), "no javac symbol of that name on the line, and no error".into(), detail(Value::Null)),
                None => ("cannot judge".into(), "another javac error on the line".into(), detail(Value::Null)),
            }
        }
        "override-matches-nothing" | "unimplemented-abstract-method" => {
            let c = if f.rule == "unimplemented-abstract-method" { declared_here } else { around };
            let missing = c.map(|c| s(c, "missing")).unwrap_or("");
            if !missing.is_empty() {
                return ("cannot judge".into(), format!("rests on a class with a missing supertype: {missing}"), detail(json!(c.map(|c| s(c, "name")))));
            }
            if c.is_none() {
                return ("cannot judge".into(), "no class of javac's around the line".into(), detail(Value::Null));
            }
            // javac gives does.not.override.abstract the class's line; the
            // override error the method's (the annotation's line or the name's)
            let near = |d: &Value| codes.contains(&s(d, "code")) && (i(d, "line") - f.line).abs() <= 1;
            if diags.iter().any(|d| near(d)) {
                ("confirmed".into(), codes[0].to_string(), detail(Value::Null))
            } else {
                ("refuted".into(), "javac reports no such error, the class's hierarchy complete".into(), detail(json!(c.map(|c| s(c, "name")))))
            }
        }
        "missing-jdk-import" => {
            if !matching.is_empty() {
                ("confirmed".into(), s(matching[0], "code").to_string(), detail(Value::Null))
            } else if at_line.is_empty() {
                ("refuted".into(), "javac resolves the import".into(), detail(Value::Null))
            } else {
                ("cannot judge".into(), "another javac error on the line".into(), detail(Value::Null))
            }
        }
        _ => ("cannot judge".into(), "rule unknown to the judge".into(), Value::Null),
    }
}

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut jobs = 1usize;
    if let Some(k) = args.iter().position(|a| a == "--jobs") {
        jobs = args[k + 1].parse()?;
        args.drain(k..k + 2);
    }
    anyhow::ensure!(args.len() == 3, "usage: javac_judge [--jobs N] <project root> <findings list> <out.json>");
    let root = PathBuf::from(&args[0]).canonicalize()?;
    let findings: Vec<Finding> = std::fs::read_to_string(&args[1])?.lines().filter_map(parse_finding).collect();
    let out_path = PathBuf::from(&args[2]);
    let out_dir = out_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let classes = compile_helper(&out_dir)?;
    let t = Instant::now();
    let p = grounds_engine::Project::load(&root)?;
    let load_ms = t.elapsed().as_secs_f64() * 1e3;

    // ----- the world: a root per indexed file whose directory matches its package -----
    let mut root_of: HashMap<PathBuf, Option<PathBuf>> = HashMap::new();
    let mut roots: BTreeMap<PathBuf, (usize, usize)> = BTreeMap::new(); // root -> (module, files)
    let mut unrooted = 0usize;
    let mut unrooted_files: Vec<PathBuf> = Vec::new();
    for f in &p.workspace.files {
        let key = grounds_engine::project::path_key(&f.path);
        let pkg = p.index.file(&key).map(|st| st.package.to_string()).unwrap_or_default();
        let dir = f.path.parent().unwrap_or(Path::new("")).to_path_buf();
        let r = if pkg.is_empty() {
            Some(dir)
        } else {
            let tail: PathBuf = pkg.split('.').collect();
            dir.ends_with(&tail).then(|| dir.ancestors().nth(tail.components().count()).unwrap_or(Path::new("")).to_path_buf())
        };
        match &r {
            Some(r) => roots.entry(r.clone()).or_insert((f.module, 0)).1 += 1,
            None => {
                unrooted += 1;
                unrooted_files.push(f.path.clone());
            }
        }
        root_of.insert(f.path.clone(), r);
    }
    let classpath: Vec<String> = p.classpath_entries.iter().map(|e| e.to_string_lossy().to_string()).collect();
    let max_release: u32 = 21;
    let release_of = |f: &Path| -> u32 {
        let exact = p.releases.exact.iter().filter(|(d, _)| f.starts_with(d)).max_by_key(|(d, _)| d.as_os_str().len()).and_then(|(_, r)| *r);
        exact.unwrap_or(p.releases.load).clamp(8, max_release)
    };
    let modules: HashMap<&Path, usize> = p.workspace.files.iter().map(|x| (x.path.as_path(), x.module)).collect();
    let module_of = |f: &Path| modules.get(f).copied();

    // ----- the files to attribute -----
    let mut by_file: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (k, f) in findings.iter().enumerate() {
        by_file.entry(f.path.clone()).or_default().push(k);
    }
    let files: Vec<String> = by_file.keys().cloned().collect();
    let results: Mutex<Vec<(String, Value, f64, Value)>> = Mutex::new(Vec::new());
    let next = AtomicUsize::new(0);
    let mut helpers: Vec<Helper> = (0..jobs).map(|_| Helper::spawn(&classes)).collect::<anyhow::Result<_>>()?;
    let t_all = Instant::now();
    std::thread::scope(|sc| {
        for h in helpers.iter_mut() {
            let (files, next, results, roots, root_of, classpath, root, classes, unrooted_files) = (&files, &next, &results, &roots, &root_of, &classpath, &root, &classes, &unrooted_files);
            let (release_of, module_of) = (&release_of, &module_of);
            sc.spawn(move || loop {
                let k = next.fetch_add(1, Ordering::SeqCst);
                let Some(rel) = files.get(k) else { break };
                let abs = root.join(rel);
                if !root_of.contains_key(&abs) {
                    results.lock().expect("lock").push((rel.clone(), json!({"no_root": "file not in grounds' workspace"}), 0.0, json!({})));
                    continue;
                }
                // a file in no root is attributed as given; the classes of
                // the other files in no root reach javac as entered roots
                let own = root_of.get(&abs).cloned().flatten();
                if own.is_none() && unrooted_files.len() > MAX_UNROOTED {
                    results.lock().expect("lock").push((rel.clone(), json!({"no_root": "file in no source root (its directory does not match its package), with too many such files to enter"}), 0.0, json!({})));
                    continue;
                }
                let own = own.unwrap_or_else(|| abs.parent().unwrap_or(Path::new("")).to_path_buf());
                let module = module_of(&abs);
                let mut sp: Vec<&PathBuf> = vec![&own];
                sp.extend(roots.iter().filter(|(r, (m, _))| **r != own && Some(*m) == module).map(|(r, _)| r));
                sp.extend(roots.iter().filter(|(r, (m, _))| **r != own && Some(*m) != module).map(|(r, _)| r));
                let release = release_of(&abs);
                let extra: Vec<&PathBuf> = if unrooted_files.len() <= MAX_UNROOTED { unrooted_files.iter().filter(|f| **f != abs).collect() } else { Vec::new() };
                let req = json!({"sourcepath": sp, "classpath": classpath, "release": release.to_string(), "files": [abs], "roots": extra, "members": true});
                let t = Instant::now();
                let ans = match h.ask(req) {
                    Ok(mut v) if v.len() == 1 => v.remove(0),
                    Ok(v) => json!({"crash": format!("{} answers", v.len())}),
                    Err(e) => {
                        // a dead helper (killed for memory): another for the next file
                        if let Ok(n) = Helper::spawn(classes) {
                            *h = n;
                        }
                        json!({"crash": e.to_string()})
                    }
                };
                let ms = t.elapsed().as_secs_f64() * 1e3;
                let meta = json!({"release": release, "own_root": own, "rooted": root_of.get(&abs).is_some_and(|r| r.is_some()), "sourcepath_roots": sp.len(), "entered_unrooted": extra.len(), "helper_ms": ans.pointer("/ms/total")});
                results.lock().expect("lock").push((rel.clone(), ans, ms, meta));
            });
        }
    });
    let wall_ms = t_all.elapsed().as_secs_f64() * 1e3;
    let results = results.into_inner().expect("lock");
    let mut out_findings = Vec::new();
    let mut files_out = Vec::new();
    for (rel, ans, ms, meta) in &results {
        files_out.push(json!({"file": rel, "rt_ms": ms, "meta": meta, "crash": ans.get("crash"), "no_root": ans.get("no_root"),
            "errors": ans.get("diags").and_then(Value::as_array).map_or(0, |d| d.iter().filter(|x| s(x, "kind") == "ERROR").count())}));
        for &k in &by_file[rel] {
            let f = &findings[k];
            let (verdict, reason, detail) = if let Some(why) = ans.get("no_root").and_then(Value::as_str) {
                if WIRING.contains(&f.rule.as_str()) {
                    ("not a compile claim".to_string(), String::new(), Value::Null)
                } else {
                    ("cannot judge".to_string(), why.to_string(), Value::Null)
                }
            } else {
                judge(f, ans)
            };
            out_findings.push(json!({"path": f.path, "line": f.line, "rule": f.rule, "message": f.message, "verdict": verdict, "reason": reason, "detail": detail}));
        }
    }
    let out = json!({
        "root": root, "load_ms": load_ms, "wall_ms": wall_ms, "jobs": jobs, "indexed_files": p.workspace.files.len(), "roots": roots.len(), "unrooted_files": unrooted,
        "classpath": classpath, "release_load": p.releases.load, "findings": out_findings, "files": files_out,
    });
    std::fs::write(&out_path, serde_json::to_string(&out)?)?;
    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    for f in &out_findings {
        *tally.entry(format!("{} {}", s(f, "verdict"), s(f, "reason").split(':').next().unwrap_or(""))).or_default() += 1;
    }
    println!("{}: findings {} files {} roots {} unrooted files {} classpath {} wall {:.0} ms", root.display(), findings.len(), files.len(), roots.len(), unrooted, classpath.len(), wall_ms);
    for (k, v) in tally {
        println!("  {v:5} {k}");
    }
    Ok(())
}
