//! Prototype: javac disposes of what grounds proposes. Every source file of a
//! project is attributed by the JDK's own compiler (javac-helper/, a
//! long-lived process speaking JSON lines), and each call, constructor call
//! and field access grounds is certain of is compared, by source position,
//! with the symbol javac attributed there. A file is
//!
//! - confirmed: javac attributed every site grounds was certain of and
//!   agrees on each (declaring class, name, erased descriptor, static);
//! - disagrees: javac attributed a site to something else;
//! - not attributable: javac could not attribute a site grounds was certain
//!   of (its symbol, a type in it, the receiver or an argument is
//!   erroneous, its answer rests on a class with a missing supertype, or a
//!   type in it has the name of a single-type import javac could not
//!   resolve, which javac then binds to another class of that name),
//!   or produced no symbol at that position, or crashed.
//!
//! Every rule runs over the project as `grounds check` runs them (preview
//! rules included), and each finding is counted under its file's verdict.
//! With the class files of the project beside it (`<dir>/classes`), the
//! helper itself is checked against what javac recorded in them (each call
//! it attributed without error against the invoke instruction on the line
//! of its `(`), as the bytecode oracle does for grounds.
//!
//!   javac_verify [--jobs N] [--out DIR] [--lombok JAR] [--lombok-roots]
//!       [--files-from LIST] [--no-rules] [--no-bytecode] [--reuse]
//!       [--tag T] <dir>...
//!
//! Each `<dir>` holds `proj/` (a grounds project, sources under
//! src/main/java) and optionally `classes/`. `--jobs` helper processes
//! attribute files in parallel (one by default). `--lombok` puts the jar on
//! the processor path; `--lombok-roots` also enters, in every task, each
//! source file that imports from `lombok`, so that javac sees the members
//! Lombok generates in classes other than the attributed file's (javac does
//! not run processors on classes it completes from the sourcepath).
//! `--files-from` restricts to the listed files. `--reuse` keeps javac's
//! context between files (JavacTaskPool; measured unsound, see the helper).
//! Writes `<out>/<tag>.json` (totals, every disagreement, every file's
//! verdict and timings) and prints a summary. The project's `java_release`
//! is the `--release`, at least 8 (JDK 21's javac has no 6 or 7).
//!
//! javac's conventions that differ from grounds' naming and are allowed:
//! a method of Object called on an interface (javac names Object's unless
//! the interface declares it again), a signature-polymorphic method (JLS
//! 15.12.3: javac's symbol has the call's own descriptor), `length` and
//! `clone()` of an array (javac's owner is its internal class `Array`), and
//! a method inherited as more than one abstract declaration (Map.clear() and
//! a Put.clear() of the library: JLS 15.12.2.5 lets the compiler name any of
//! them; the same method runs). A constructor call of an anonymous class is
//! not compared: javac's symbol is the anonymous class's own constructor.
//!
//! A file's verdict covers what grounds resolved in it, not what a rule
//! claims about declarations: a compile-break finding in a confirmed file
//! can still be false. For each, the output says whether javac reports an
//! error at its position.

use grounds_classfile::{parse_references, ClassContainer, RefKind};
use grounds_index::{ClassLookup, Flags, MethodInfo, Prim, Ty, TypeParam};
use grounds_semantic::{FileModel, Symbol};
use grounds_syntax::{Node, NodeExt};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// The rules stable in `grounds check` (crates/grounds-rules/tests/fixtures.rs `STABLE`).
const STABLE: &[&str] = &[
    "assertion-argument-order",
    "public-static-mutable-container",
    "public-static-non-final-field",
    "redundant-null-check",
    "static-non-thread-safe-field",
    "unused-local-variable",
    "unused-private-field",
    "unused-private-method",
];
/// The compile-break rules (`STABLE_IN_CHANGED` in grounds-rules' framework.rs).
const COMPILE_BREAK: &[&str] = &["missing-jdk-import", "override-matches-nothing", "unimplemented-abstract-method", "unresolved-constructor", "unresolved-field", "unresolved-method"];

// ----- descriptors (as in the bytecode_oracle example) -----

fn desc(t: &Ty) -> Option<String> {
    Some(match t {
        Ty::Prim(p) => match p {
            Prim::Boolean => "Z",
            Prim::Byte => "B",
            Prim::Char => "C",
            Prim::Short => "S",
            Prim::Int => "I",
            Prim::Long => "J",
            Prim::Float => "F",
            Prim::Double => "D",
        }
        .to_string(),
        Ty::Void => "V".to_string(),
        Ty::Class { name, .. } => format!("L{};", name.replace('.', "/")),
        Ty::Array(e) => format!("[{}", desc(e)?),
        _ => return None,
    })
}

fn erased(t: &Ty, scope: &[&TypeParam], depth: u8) -> Option<String> {
    match t {
        Ty::TypeVar(v) if depth < 8 => match scope.iter().find(|p| p.name == *v)?.bounds.first() {
            Some(b) => erased(b, scope, depth + 1),
            None => Some("Ljava/lang/Object;".to_string()),
        },
        Ty::Array(e) => Some(format!("[{}", erased(e, scope, depth)?)),
        Ty::Class { .. } | Ty::Prim(_) | Ty::Void => desc(&t.erasure()),
        _ => None,
    }
}

fn class_type_params(lk: &impl ClassLookup, class: &str) -> Vec<TypeParam> {
    let mut out = Vec::new();
    let mut cur = lk.class(class);
    for _ in 0..16 {
        let Some(c) = cur else { break };
        out.extend(c.type_params.iter().cloned());
        cur = c.outer.as_deref().and_then(|o| lk.class(o));
    }
    out
}

/// Parameter descriptors and the return descriptor of `(..)R`.
fn split(d: &str) -> (Vec<String>, String) {
    let Some((ps, ret)) = d.strip_prefix('(').and_then(|d| d.split_once(')')) else { return (Vec::new(), d.to_string()) };
    let b = ps.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        let s = i;
        while i < b.len() && b[i] == b'[' {
            i += 1;
        }
        if i < b.len() && b[i] == b'L' {
            while i < b.len() && b[i] != b';' {
                i += 1;
            }
        }
        i = (i + 1).min(b.len());
        out.push(ps[s..i].to_string());
    }
    (out, ret.to_string())
}

/// A binary name without the numbers of local and anonymous classes (JLS
/// 13.1 leaves them to the compiler).
fn unnumbered(name: &str) -> String {
    let parts: Vec<String> = name
        .replace('/', ".")
        .split('$')
        .enumerate()
        .map(|(i, s)| {
            let rest = s.trim_start_matches(|c: char| c.is_ascii_digit());
            if i > 0 && rest.len() < s.len() {
                format!("#{rest}")
            } else {
                s.to_string()
            }
        })
        .collect();
    parts.join("$")
}

// ----- the helper process -----

/// Helpers started again after they died.
static RESTARTS: AtomicUsize = AtomicUsize::new(0);

struct Helper {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next: u64,
    classes: PathBuf,
}

impl Helper {
    /// A helper process: a bounded heap and the serial collector keep
    /// several of them, and the machine's other work, within memory.
    fn spawn(classes: &Path) -> anyhow::Result<Helper> {
        let mut child = Command::new("java")
            .args(["--add-exports", "jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED", "-Xss64m", "-Xmx1g", "-XX:+UseSerialGC", "-cp"])
            .arg(classes)
            .arg("JavacHelper")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = BufWriter::new(child.stdin.take().expect("piped"));
        let stdout = BufReader::new(child.stdout.take().expect("piped"));
        Ok(Helper { child, stdin, stdout, next: 0, classes: classes.to_path_buf() })
    }

    /// One request; the answers of its files. A helper that died (killed
    /// for memory) is started again and asked once more.
    fn ask(&mut self, req: Value) -> anyhow::Result<Vec<Value>> {
        match self.ask_once(req.clone()) {
            Ok(v) => Ok(v),
            Err(_) if self.child.try_wait().ok().flatten().is_some() => {
                let classes = self.classes.clone();
                *self = Helper::spawn(&classes)?;
                RESTARTS.fetch_add(1, Ordering::SeqCst);
                self.ask_once(req)
            }
            Err(e) => Err(e),
        }
    }

    fn ask_once(&mut self, mut req: Value) -> anyhow::Result<Vec<Value>> {
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
    // a checkout without the helper's source (an older commit): the one compiled before
    if !src.is_file() && dir.join("JavacHelper.class").is_file() {
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir)?;
    let st = Command::new("javac").args(["--add-exports", "jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED", "-nowarn", "-d"]).arg(&dir).arg(&src).stderr(Stdio::null()).status()?;
    anyhow::ensure!(st.success(), "javac failed on {}", src.display());
    Ok(dir)
}

// ----- one site of javac's answer -----

struct JSite {
    kind: String,
    start: i64,
    ns: i64,
    line: i64,
    owner: String,
    name: String,
    desc: String,
    is_static: bool,
    err: bool,
    argerr: bool,
    hier: bool,
    imp: bool,
    anon: bool,
    qual: Option<String>,
}

fn jsite(v: &Value) -> JSite {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let i = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(-1);
    let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    JSite {
        kind: s("k"),
        start: i("start"),
        ns: i("ns"),
        line: i("line"),
        owner: s("owner"),
        name: s("name"),
        desc: s("desc"),
        is_static: b("static"),
        // javac's error recovery can leave a symbol of another kind (a
        // package) where a member was expected: as good as erroneous
        err: b("err") || v.get("symkind").is_some(),
        argerr: b("argerr"),
        hier: b("hier"),
        imp: b("imp"),
        anon: b("anon"),
        qual: v.get("qual").and_then(Value::as_str).map(str::to_string),
    }
}

struct Diag {
    code: String,
    kind: String,
    start: i64,
    end: i64,
    line: i64,
}

// ----- what grounds is certain of -----

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum SiteKind {
    Call,
    Ctor,
    Field,
}

impl SiteKind {
    fn as_str(self) -> &'static str {
        match self {
            SiteKind::Call => "call",
            SiteKind::Ctor => "ctor",
            SiteKind::Field => "field",
        }
    }
}

struct GSite {
    kind: SiteKind,
    /// the position javac's site is found by: a name's start (calls,
    /// fields), the expression's start (constructors)
    key: usize,
    start: usize,
    end: usize,
    line: usize,
    owner: String,
    name: String,
    /// erased parameters of each declared candidate (None: unknown to grounds)
    params: Vec<Vec<Option<String>>>,
    ret: Option<String>,
    is_static: bool,
    owner_is_interface: bool,
    /// the declared method is generated by grounds' Lombok model
    generated: bool,
}

fn grounds_sites<'t>(fm: &FileModel<'t>, pf: &'t grounds_syntax::ParsedFile) -> Vec<GSite> {
    let line = |b: usize| pf.lines.line_of(b as u32) as usize + 1;
    let mut out = Vec::new();
    for n in pf.root().descendants() {
        match n.kind() {
            "method_invocation" => {
                let Some(name_node) = n.field("name") else { continue };
                let name = fm.text(name_node).to_string();
                let Some(r) = fm.resolve_call(n) else { continue };
                if r.ambiguous || r.erasure_only {
                    continue;
                }
                let Some(owner) = fm.class(&r.owner) else { continue };
                let scope_owned = class_type_params(fm, &owner.name);
                let subst: Vec<Option<String>> = r.method.params.iter().map(|q| desc(&q.ty.erasure())).collect();
                let cands: Vec<&MethodInfo> = owner
                    .methods_named(&name)
                    .filter(|m| m.params.len() == subst.len())
                    .filter(|m| {
                        m.params.iter().zip(&subst).all(|(q, s)| match (desc(&q.ty), s) {
                            (Some(d), Some(s)) => d == *s,
                            _ => true,
                        })
                    })
                    .collect();
                let (params, ret, generated) = if cands.is_empty() {
                    (vec![subst.iter().map(|_| None).collect()], None, false)
                } else {
                    let ps = cands
                        .iter()
                        .map(|m| {
                            let scope: Vec<&TypeParam> = m.type_params.iter().chain(&scope_owned).collect();
                            m.params.iter().map(|q| erased(&q.ty, &scope, 0)).collect()
                        })
                        .collect();
                    let scope: Vec<&TypeParam> = cands[0].type_params.iter().chain(&scope_owned).collect();
                    let ret = if cands.len() == 1 { erased(&cands[0].ret, &scope, 0) } else { None };
                    (ps, ret, cands.iter().any(|m| m.generated_by.is_some() || m.flags.has(Flags::GENERATED)))
                };
                out.push(GSite {
                    kind: SiteKind::Call,
                    key: name_node.start_byte(),
                    start: n.start_byte(),
                    end: n.end_byte(),
                    line: line(name_node.start_byte()),
                    owner: r.owner.to_string(),
                    name,
                    params,
                    ret,
                    is_static: r.method.flags.is_static(),
                    owner_is_interface: owner.is_interface(),
                    generated,
                });
            }
            "object_creation_expression" | "explicit_constructor_invocation" => {
                let Some(r) = fm.select_method(n) else { continue };
                if r.ambiguous {
                    continue;
                }
                let Some(owner) = fm.class(&r.owner) else { continue };
                let scope_owned = class_type_params(fm, &owner.name);
                let subst: Vec<Option<String>> = r.method.params.iter().map(|q| desc(&q.ty.erasure())).collect();
                let cands: Vec<&MethodInfo> = owner
                    .constructors()
                    .filter(|m| m.params.len() == subst.len())
                    .filter(|m| {
                        m.params.iter().zip(&subst).all(|(q, s)| match (desc(&q.ty), s) {
                            (Some(d), Some(s)) => d == *s,
                            _ => true,
                        })
                    })
                    .collect();
                let params = if cands.is_empty() {
                    vec![subst.iter().map(|_| None).collect()]
                } else {
                    cands
                        .iter()
                        .map(|m| {
                            let scope: Vec<&TypeParam> = m.type_params.iter().chain(&scope_owned).collect();
                            m.params.iter().map(|q| erased(&q.ty, &scope, 0)).collect()
                        })
                        .collect()
                };
                out.push(GSite {
                    kind: SiteKind::Ctor,
                    key: n.start_byte(),
                    start: n.start_byte(),
                    end: n.end_byte(),
                    line: line(n.start_byte()),
                    owner: r.owner.to_string(),
                    name: "<init>".to_string(),
                    params,
                    ret: Some("V".to_string()),
                    is_static: false,
                    owner_is_interface: false,
                    generated: cands.iter().any(|m| m.generated_by.is_some() || m.flags.has(Flags::GENERATED)),
                });
            }
            "identifier" | "field_access" => {
                if n.kind() == "identifier" && fm.parent_of(n).is_some_and(|p| p.kind() == "field_access" && p.field("field") == Some(n)) {
                    continue;
                }
                // a name in an import is no access, and javac gives it no symbol
                if n.ancestors().any(|a| a.kind() == "import_declaration") {
                    continue;
                }
                let Some(Symbol::Field(f)) = fm.symbol(n) else { continue };
                let name_node: Node = if n.kind() == "field_access" { n.field("field").unwrap_or(n) } else { n };
                let scope_owned = class_type_params(fm, &f.owner);
                let scope: Vec<&TypeParam> = scope_owned.iter().collect();
                let ty = fm.class(&f.owner).and_then(|o| o.field(&f.field.name).and_then(|d| erased(&d.ty, &scope, 0)));
                out.push(GSite {
                    kind: SiteKind::Field,
                    key: name_node.start_byte(),
                    start: n.start_byte(),
                    end: n.end_byte(),
                    line: line(n.start_byte()),
                    owner: f.owner.to_string(),
                    name: f.field.name.to_string(),
                    params: Vec::new(),
                    ret: ty,
                    is_static: f.field.flags.is_static(),
                    owner_is_interface: false,
                    generated: false,
                });
            }
            _ => {}
        }
    }
    out
}

// ----- the per-file result -----

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Verdict {
    Confirmed,
    Disagrees,
    NotAttributable,
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Verdict::Confirmed => "confirmed",
            Verdict::Disagrees => "disagrees",
            Verdict::NotAttributable => "not attributable",
        }
    }
}

#[derive(Default)]
struct ACount {
    calls: usize,
    compared: usize,
    agree: usize,
    flagged_compared: usize,
    flagged_disagree: usize,
    not_compared: BTreeMap<&'static str, usize>,
    disagree: Vec<Value>,
}

struct FileOut {
    rel: String,
    verdict: Verdict,
    certain: usize,
    agree: usize,
    conventions: BTreeMap<&'static str, usize>,
    disagree: Vec<Value>,
    /// sites javac could not attribute: why -> count
    not_attr: BTreeMap<String, usize>,
    /// diagnostic codes blamed for the sites javac could not attribute
    not_attr_codes: BTreeMap<String, usize>,
    no_site_examples: Vec<String>,
    crash: Option<String>,
    diags: Vec<Diag>,
    a: ACount,
    /// sites reaching members grounds' Lombok model generated: agree, disagree, javac erroneous
    generated: [usize; 3],
    javac_members: usize,
    rt_ms: f64,
    helper_ms: f64,
    grounds_ms: f64,
}

/// What javac recorded in the class files: (file, line, name, number of
/// parameters) -> (owner, descriptor) of each invoke instruction.
type Invokes = HashMap<(String, usize, String, usize), Vec<(String, String)>>;

fn read_invokes(dir: &Path) -> Invokes {
    let mut out: Invokes = HashMap::new();
    for n in ClassContainer::open_dir(dir).class_names().into_iter().filter(|n| !n.starts_with("META-INF/")) {
        let Ok(bytes) = std::fs::read(dir.join(format!("{n}.class"))) else { continue };
        let Ok(refs) = parse_references(&bytes) else { continue };
        let Some(src) = &refs.source_file else { continue };
        let file = match refs.name.rsplit_once('/') {
            Some((pkg, _)) => format!("{pkg}/{src}"),
            None => src.clone(),
        };
        for m in refs.methods {
            for r in m.refs {
                let Some(line) = r.line else { continue };
                if matches!(r.kind, RefKind::InvokeVirtual | RefKind::InvokeSpecial | RefKind::InvokeStatic | RefKind::InvokeInterface) {
                    let (ps, _) = split(&r.descriptor);
                    out.entry((file.clone(), line as usize, r.name.clone(), ps.len())).or_default().push((r.owner.replace('/', "."), r.descriptor.clone()));
                }
            }
        }
    }
    out
}

/// The helper against the class files: each call it attributed without
/// error, against the invoke instructions of that name on the line of its `(`.
fn check_against_bytecode(rel: &str, sites: &[JSite], inv: &Invokes, a: &mut ACount) {
    for s in sites.iter().filter(|s| s.kind == "call") {
        a.calls += 1;
        if s.err {
            *a.not_compared.entry("erroneous").or_default() += 1;
            continue;
        }
        let (ps, _) = split(&s.desc);
        let Some(found) = inv.get(&(rel.to_string(), s.line as usize, s.name.clone(), ps.len())) else {
            *a.not_compared.entry("no invoke of that name and arity on the line").or_default() += 1;
            continue;
        };
        a.compared += 1;
        let flagged = s.argerr || s.hier || s.imp;
        if flagged {
            a.flagged_compared += 1;
        }
        // an array's qualifier is its descriptor; the instruction's owner was read with dots
        let owners_ok = |o: &str| o == s.owner || Some(o.replace('.', "/").as_str()) == s.qual.as_deref().map(|q| q.replace('.', "/")).as_deref() || (s.owner == "java.lang.Object");
        if found.iter().any(|(o, d)| *d == s.desc && owners_ok(o)) {
            a.agree += 1;
            continue;
        }
        let same_desc: Vec<&(String, String)> = found.iter().filter(|(_, d)| *d == s.desc).collect();
        let kind = if same_desc.is_empty() {
            if s.owner == "java.lang.invoke.MethodHandle" || s.owner == "java.lang.invoke.VarHandle" {
                "descriptor: signature-polymorphic"
            } else {
                "descriptor: other"
            }
        } else if same_desc.iter().all(|(o, _)| o == "java.lang.Object") {
            "owner: Object named for a method an interface redeclares"
        } else {
            "owner: other"
        };
        if flagged {
            a.flagged_disagree += 1;
        }
        a.disagree.push(json!({"kind": kind, "file": rel, "line": s.line, "name": s.name, "helper": format!("{}.{}{}", s.owner, s.name, s.desc), "qual": s.qual, "flagged": flagged, "bytecode": found.iter().map(|(o, d)| format!("{o}.{}{d}", s.name)).collect::<Vec<_>>()}));
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_file(p: &grounds_engine::Project, helper: &mut Helper, base_req: &Value, path: &Path, rel: &str, inv: Option<&Invokes>) -> FileOut {
    let mut req = base_req.clone();
    req["files"] = json!([path.to_string_lossy()]);
    let t = Instant::now();
    let answer = helper.ask(req);
    let rt_ms = t.elapsed().as_secs_f64() * 1e3;
    let mut fo = FileOut {
        rel: rel.to_string(),
        verdict: Verdict::Confirmed,
        certain: 0,
        agree: 0,
        conventions: BTreeMap::new(),
        disagree: Vec::new(),
        not_attr: BTreeMap::new(),
        not_attr_codes: BTreeMap::new(),
        no_site_examples: Vec::new(),
        crash: None,
        diags: Vec::new(),
        a: ACount::default(),
        generated: [0; 3],
        javac_members: 0,
        rt_ms,
        helper_ms: 0.0,
        grounds_ms: 0.0,
    };
    let v = match answer {
        Ok(mut v) if v.len() == 1 => v.remove(0),
        Ok(v) => {
            fo.crash = Some(format!("{} answers", v.len()));
            Value::Null
        }
        Err(e) => {
            fo.crash = Some(e.to_string());
            Value::Null
        }
    };
    if let Some(c) = v.get("crash").and_then(Value::as_str) {
        fo.crash = Some(c.lines().next().unwrap_or("").to_string());
    }
    fo.helper_ms = v.pointer("/ms/total").and_then(Value::as_f64).unwrap_or(0.0);
    let jsites: Vec<JSite> = v.get("sites").and_then(Value::as_array).map(|a| a.iter().map(jsite).collect()).unwrap_or_default();
    fo.diags = v
        .get("diags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|d| Diag {
                    code: d.get("code").and_then(Value::as_str).unwrap_or("").to_string(),
                    kind: d.get("kind").and_then(Value::as_str).unwrap_or("").to_string(),
                    start: d.get("start").and_then(Value::as_i64).unwrap_or(-1),
                    end: d.get("end").and_then(Value::as_i64).unwrap_or(-1),
                    line: d.get("line").and_then(Value::as_i64).unwrap_or(-1),
                })
                .collect()
        })
        .unwrap_or_default();
    fo.javac_members = v.get("classes").and_then(Value::as_array).map(|a| a.iter().map(|c| c.get("members").and_then(Value::as_array).map_or(0, |m| m.len())).sum()).unwrap_or(0);
    if let Some(inv) = inv {
        check_against_bytecode(rel, &jsites, inv, &mut fo.a);
    }

    // grounds' side, timed apart
    let t = Instant::now();
    let Ok(text) = std::fs::read_to_string(path) else {
        fo.crash = Some("unreadable".into());
        fo.verdict = Verdict::NotAttributable;
        return fo;
    };
    let pf = grounds_syntax::ParsedFile::parse(text);
    let fm = FileModel::new(&pf, &p.index, grounds_engine::project::path_key(path));
    let gsites = grounds_sites(&fm, &pf);
    let mut by_name: HashMap<(&str, i64), &JSite> = HashMap::new();
    for s in &jsites {
        let k = match s.kind.as_str() {
            "ctor" => ("ctor", s.start),
            "call" => ("call", s.ns),
            "field" | "unres" => ("field", s.ns),
            _ => continue,
        };
        // a resolved field site wins over an unresolved name at the same place
        if s.kind != "unres" || !by_name.contains_key(&k) {
            by_name.insert(k, s);
        }
    }
    // the diagnostic blamed for a site javac could not attribute: an error
    // within the site, else the file's most frequent error
    let diags = std::mem::take(&mut fo.diags);
    let mut file_codes: BTreeMap<&str, usize> = BTreeMap::new();
    for d in diags.iter().filter(|d| d.kind == "ERROR") {
        *file_codes.entry(&d.code).or_default() += 1;
    }
    let top_code = file_codes.iter().max_by_key(|x| *x.1).map(|x| x.0.to_string());
    let blame = |g: &GSite| -> String {
        diags
            .iter()
            .filter(|d| d.kind == "ERROR" && d.start >= g.start as i64 && d.start < g.end as i64)
            .map(|d| d.code.clone())
            .next()
            .or_else(|| top_code.as_ref().map(|c| format!("elsewhere in file: {c}")))
            .unwrap_or_else(|| "no error reported".to_string())
    };
    let mut not_attr: Vec<(String, String)> = Vec::new();
    for g in &gsites {
        fo.certain += 1;
        let k = (g.kind.as_str(), g.key as i64);
        let Some(s) = by_name.get(&k) else {
            if fo.crash.is_some() {
                not_attr.push(("javac crashed".into(), String::new()));
                continue;
            }
            not_attr.push(("no javac symbol at that position".into(), String::new()));
            if fo.no_site_examples.len() < 3 {
                fo.no_site_examples.push(format!("{rel}:{} {} {}.{}", g.line, g.kind.as_str(), g.owner, g.name));
            }
            continue;
        };
        let gen_slot = |fo: &mut FileOut, i: usize| {
            if g.generated {
                fo.generated[i] += 1;
            }
        };
        if s.kind == "unres" || s.err {
            not_attr.push(("erroneous symbol".into(), blame(g)));
            gen_slot(&mut fo, 2);
            continue;
        }
        if s.argerr {
            not_attr.push(("erroneous receiver or argument".into(), blame(g)));
            gen_slot(&mut fo, 2);
            continue;
        }
        if s.hier {
            not_attr.push(("rests on a class with a missing supertype".into(), blame(g)));
            gen_slot(&mut fo, 2);
            continue;
        }
        if s.imp {
            not_attr.push(("a failed import may have rebound a name".into(), blame(g)));
            gen_slot(&mut fo, 2);
            continue;
        }
        if g.kind == SiteKind::Ctor && s.anon {
            *fo.conventions.entry("anonymous class constructor (not compared)").or_default() += 1;
            continue;
        }
        // a field's descriptor is its type
        let (jps, jret) = if g.kind == SiteKind::Field { (Vec::new(), s.desc.clone()) } else { split(&s.desc) };
        let params_ok = g.kind == SiteKind::Field || g.params.iter().any(|ps| ps.len() == jps.len() && ps.iter().zip(&jps).all(|(a, b)| a.as_deref().is_none_or(|a| a == b)));
        let ret_ok = g.kind == SiteKind::Ctor || g.ret.as_deref().is_none_or(|r| r == jret);
        let owner_ok = unnumbered(&s.owner) == unnumbered(&g.owner);
        let name_ok = s.name == g.name || (g.kind == SiteKind::Ctor && s.name == "<init>");
        let static_ok = s.is_static == g.is_static;
        if owner_ok && name_ok && params_ok && ret_ok && static_ok {
            fo.agree += 1;
            gen_slot(&mut fo, 0);
            continue;
        }
        let abstract_in = |cls: &str, ps: &[String]| {
            fm.class(cls)
                .is_some_and(|c| c.methods_named(&g.name).any(|m| m.params.len() == ps.len() && (m.flags.is_abstract() || (c.is_interface() && !m.flags.has(Flags::DEFAULT) && !m.flags.is_static()))))
        };
        // javac's conventions
        let conv = if name_ok && s.owner == "java.lang.Object" && g.owner_is_interface && params_ok {
            Some("a method of Object called on an interface")
        } else if name_ok && matches!(&*s.owner, "java.lang.invoke.MethodHandle" | "java.lang.invoke.VarHandle") && s.owner == g.owner {
            Some("signature-polymorphic method")
        } else if name_ok && s.owner == "Array" {
            Some("array member (length, clone)")
        } else if name_ok && params_ok && ret_ok && static_ok && g.kind == SiteKind::Call && abstract_in(&s.owner, &jps) && abstract_in(&g.owner, &jps) {
            // more than one abstract declaration of the method is inherited
            // (JLS 15.12.2.5: one is chosen arbitrarily); the same method runs
            Some("override-equivalent abstract declarations (javac names another)")
        } else {
            None
        };
        if let Some(c) = conv {
            *fo.conventions.entry(c).or_default() += 1;
            gen_slot(&mut fo, 0);
            continue;
        }
        gen_slot(&mut fo, 1);
        let what = if !name_ok {
            "other name"
        } else if !owner_ok {
            "other owner"
        } else if !params_ok {
            "other parameters"
        } else if !ret_ok {
            "other return"
        } else {
            "other static"
        };
        let gp: Vec<String> = g.params.iter().map(|ps| ps.iter().map(|x| x.as_deref().unwrap_or("?")).collect::<String>()).collect();
        fo.disagree.push(json!({
            "file": rel, "line": g.line, "kind": g.kind.as_str(), "what": what,
            "grounds": format!("{}{}.{}({}){}", if g.is_static { "static " } else { "" }, g.owner, g.name, gp.join("|"), g.ret.as_deref().unwrap_or("?")),
            "javac": format!("{}{}.{}{}", if s.is_static { "static " } else { "" }, s.owner, s.name, s.desc),
        }));
    }
    for (why, code) in &not_attr {
        *fo.not_attr.entry(why.clone()).or_default() += 1;
        if !code.is_empty() {
            *fo.not_attr_codes.entry(code.clone()).or_default() += 1;
        }
    }
    fo.diags = diags;
    fo.verdict = if !fo.disagree.is_empty() {
        Verdict::Disagrees
    } else if fo.crash.is_some() || !not_attr.is_empty() {
        Verdict::NotAttributable
    } else {
        Verdict::Confirmed
    };
    fo.grounds_ms = t.elapsed().as_secs_f64() * 1e3;
    fo
}

struct Opts {
    jobs: usize,
    out: PathBuf,
    lombok: Option<PathBuf>,
    lombok_roots: bool,
    files_from: Option<Vec<PathBuf>>,
    rules: bool,
    bytecode: bool,
    reuse: bool,
    tag: String,
    dirs: Vec<PathBuf>,
}

fn opts() -> anyhow::Result<Opts> {
    let mut o =
        Opts { jobs: 1, out: PathBuf::from("javac-verify-out"), lombok: None, lombok_roots: false, files_from: None, rules: true, bytecode: true, reuse: false, tag: "run".into(), dirs: Vec::new() };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or_else(|| anyhow::anyhow!("{a} needs a value"));
        match a.as_str() {
            "--jobs" => o.jobs = val()?.parse()?,
            "--out" => o.out = PathBuf::from(val()?),
            "--lombok" => o.lombok = Some(PathBuf::from(val()?).canonicalize()?),
            "--lombok-roots" => o.lombok_roots = true,
            "--files-from" => {
                let list = std::fs::read_to_string(val()?)?;
                o.files_from = Some(list.lines().filter(|l| !l.trim().is_empty()).map(|l| PathBuf::from(l.trim())).collect());
            }
            "--no-rules" => o.rules = false,
            "--no-bytecode" => o.bytecode = false,
            "--reuse" => o.reuse = true,
            "--tag" => o.tag = val()?,
            _ => o.dirs.push(PathBuf::from(&a).canonicalize()?),
        }
    }
    anyhow::ensure!(!o.dirs.is_empty(), "usage: javac_verify [--jobs N] [--out DIR] [--lombok JAR] [--lombok-roots] [--files-from LIST] [--no-rules] [--no-bytecode] [--reuse] [--tag T] <dir>...");
    Ok(o)
}

/// The project's `java_release`, at least 8: the oldest `--release` JDK 21's
/// javac accepts (6 and 7 are read as 8; their language is a subset).
fn release_of(proj: &Path) -> Option<String> {
    let toml = std::fs::read_to_string(proj.join("grounds.toml")).ok()?;
    let r: u32 = toml.lines().find_map(|l| l.trim().strip_prefix("java_release").and_then(|r| r.trim_start_matches([' ', '=']).trim().parse().ok()))?;
    Some(r.max(8).to_string())
}

fn add<K: Ord + Clone>(into: &mut BTreeMap<K, usize>, from: &BTreeMap<K, usize>) {
    for (k, v) in from {
        *into.entry(k.clone()).or_default() += v;
    }
}

fn main() -> anyhow::Result<()> {
    let o = opts()?;
    std::fs::create_dir_all(&o.out)?;
    let classes = compile_helper(&o.out)?;
    // the helpers, started together; cold start is the time to the first answer
    let t_spawn = Instant::now();
    let mut helpers: Vec<Helper> = (0..o.jobs).map(|_| Helper::spawn(&classes)).collect::<anyhow::Result<_>>()?;
    let mut cold_ms: Option<f64> = None;
    let mut libs: Vec<Value> = Vec::new();
    let mut all_files: Vec<Value> = Vec::new();
    let t_all = Instant::now();
    for dir in &o.dirs {
        let lib = dir.file_name().unwrap_or_default().to_string_lossy().to_string();
        let proj = dir.join("proj");
        let src_root = proj.join("src/main/java").canonicalize()?;
        let t = Instant::now();
        let mut p = grounds_engine::Project::load(&proj)?;
        let load_ms = t.elapsed().as_secs_f64() * 1e3;
        let mut files: Vec<PathBuf> = p.workspace.files.iter().map(|f| f.path.clone()).filter(|f| f.starts_with(&src_root)).collect();
        files.sort();
        if let Some(list) = &o.files_from {
            files.retain(|f| list.contains(f));
            if files.is_empty() {
                continue;
            }
        }
        // findings, as `grounds check` makes them, with every rule
        let mut findings = Vec::new();
        if o.rules {
            p.config.rules.preview = true;
            let (ruleset, options) = grounds_engine::analyze::build_ruleset(&p);
            let all = grounds_engine::analyze::expand_paths(&p, &[]);
            findings = grounds_engine::analyze::analyze_files(&p, &ruleset, &options, &all);
        }
        let inv = if o.bytecode && dir.join("classes").is_dir() { Some(read_invokes(&dir.join("classes"))) } else { None };
        let mut base = json!({"sourcepath": [src_root.to_string_lossy()], "classpath": [], "members": true, "reuse": o.reuse});
        if let Some(r) = release_of(&proj) {
            base["release"] = json!(r);
        }
        if let Some(l) = &o.lombok {
            base["processorpath"] = json!([l.to_string_lossy()]);
            if o.lombok_roots {
                let roots: Vec<String> = p
                    .workspace
                    .files
                    .iter()
                    .filter(|f| f.path.starts_with(&src_root))
                    .filter(|f| std::fs::read_to_string(&f.path).is_ok_and(|t| t.contains("import lombok.")))
                    .map(|f| f.path.to_string_lossy().to_string())
                    .collect();
                base["roots"] = json!(roots);
            }
        }
        // the first answer of the run: a cold helper
        if cold_ms.is_none() {
            let f = &files[0];
            let mut req = base.clone();
            req["files"] = json!([f.to_string_lossy()]);
            helpers[0].ask(req)?;
            cold_ms = Some(t_spawn.elapsed().as_secs_f64() * 1e3);
            // warm the others on the same file
            for h in helpers.iter_mut().skip(1) {
                let mut req = base.clone();
                req["files"] = json!([f.to_string_lossy()]);
                h.ask(req)?;
            }
        }
        let next = AtomicUsize::new(0);
        let results: Mutex<Vec<FileOut>> = Mutex::new(Vec::new());
        let t = Instant::now();
        std::thread::scope(|sc| {
            for h in helpers.iter_mut() {
                let (p, files, next, results, base, inv, src_root) = (&p, &files, &next, &results, &base, inv.as_ref(), &src_root);
                sc.spawn(move || loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(f) = files.get(i) else { break };
                    let rel = f.strip_prefix(src_root).unwrap_or(f).to_string_lossy().to_string();
                    let fo = verify_file(p, h, base, f, &rel, inv);
                    results.lock().expect("lock").push(fo);
                });
            }
        });
        let wall_ms = t.elapsed().as_secs_f64() * 1e3;
        let mut results = results.into_inner().expect("lock");
        results.sort_by(|a, b| a.rel.cmp(&b.rel));

        // ----- totals -----
        let mut verdicts: BTreeMap<&str, usize> = BTreeMap::new();
        let (mut certain, mut agree) = (0, 0);
        let mut conventions: BTreeMap<&str, usize> = BTreeMap::new();
        let mut not_attr: BTreeMap<String, usize> = BTreeMap::new();
        let mut not_attr_codes: BTreeMap<String, usize> = BTreeMap::new();
        let mut disagree: Vec<Value> = Vec::new();
        let mut no_site: Vec<String> = Vec::new();
        let mut a = ACount::default();
        let mut generated = [0usize; 3];
        let mut javac_members = 0;
        let mut crashes = Vec::new();
        let verdict_of: HashMap<String, (Verdict, usize)> = results.iter().enumerate().map(|(i, r)| (format!("src/main/java/{}", r.rel), (r.verdict, i))).collect();
        for r in &results {
            *verdicts.entry(r.verdict.as_str()).or_default() += 1;
            certain += r.certain;
            agree += r.agree;
            add(&mut conventions, &r.conventions);
            add(&mut not_attr, &r.not_attr);
            add(&mut not_attr_codes, &r.not_attr_codes);
            disagree.extend(r.disagree.iter().cloned());
            no_site.extend(r.no_site_examples.iter().cloned());
            a.calls += r.a.calls;
            a.compared += r.a.compared;
            a.agree += r.a.agree;
            a.flagged_compared += r.a.flagged_compared;
            a.flagged_disagree += r.a.flagged_disagree;
            add(&mut a.not_compared, &r.a.not_compared);
            a.disagree.extend(r.a.disagree.iter().cloned());
            for i in 0..3 {
                generated[i] += r.generated[i];
            }
            javac_members += r.javac_members;
            if let Some(c) = &r.crash {
                crashes.push(format!("{}: {c}", r.rel));
            }
            all_files.push(json!({"lib": lib, "file": r.rel, "verdict": r.verdict.as_str(), "certain": r.certain, "rt_ms": r.rt_ms, "helper_ms": r.helper_ms, "grounds_ms": r.grounds_ms, "not_attr": r.not_attr, "codes": r.not_attr_codes}));
        }
        // findings by the verdict of their file
        let mut by_verdict: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
        let mut breaks: Vec<Value> = Vec::new();
        for f in &findings {
            let Some((v, i)) = verdict_of.get(&f.path) else {
                *by_verdict.entry("file not verified".into()).or_default().entry("all".into()).or_default() += 1;
                continue;
            };
            let e = by_verdict.entry(v.as_str().to_string()).or_default();
            *e.entry("all".into()).or_default() += 1;
            *e.entry(format!("severity {}", f.severity.as_str())).or_default() += 1;
            if STABLE.contains(&f.rule) {
                *e.entry("stable".into()).or_default() += 1;
                *e.entry(format!("stable {}", f.rule)).or_default() += 1;
            }
            if COMPILE_BREAK.contains(&f.rule) {
                *e.entry("compile-break".into()).or_default() += 1;
                *e.entry(format!("compile-break {}", f.rule)).or_default() += 1;
                let r = &results[*i];
                let (fs, fe) = (f.start as i64, f.end as i64);
                let at = r.diags.iter().filter(|d| d.kind == "ERROR").find(|d| d.start < fe.max(fs + 1) && fs < d.end.max(d.start + 1));
                let on_line = r.diags.iter().filter(|d| d.kind == "ERROR").find(|d| d.line == f.line as i64);
                breaks.push(json!({"file": f.path, "line": f.line, "rule": f.rule, "verdict": v.as_str(), "message": f.message,
                    "javac_at_position": at.map(|d| d.code.clone()), "javac_on_line": on_line.map(|d| d.code.clone())}));
            }
        }
        let rt: f64 = results.iter().map(|r| r.rt_ms).sum();
        let hm: f64 = results.iter().map(|r| r.helper_ms).sum();
        let gm: f64 = results.iter().map(|r| r.grounds_ms).sum();
        let lib_out = json!({
            "lib": lib, "files": results.len(), "load_ms": load_ms, "wall_ms": wall_ms, "sum_rt_ms": rt, "sum_helper_ms": hm, "sum_grounds_ms": gm,
            "verdicts": verdicts, "certain": certain, "agree": agree, "conventions": conventions, "disagree_n": disagree.len(),
            "not_attributable": not_attr, "not_attributable_codes": not_attr_codes, "no_javac_site_examples": no_site,
            "disagreements": disagree, "crashes": crashes,
            "generated_sites": {"agree": generated[0], "disagree": generated[1], "javac_erroneous": generated[2]}, "javac_members": javac_members,
            "bytecode": {"calls": a.calls, "compared": a.compared, "agree": a.agree, "disagree": a.disagree.len(), "flagged_compared": a.flagged_compared, "flagged_disagree": a.flagged_disagree, "not_compared": a.not_compared, "disagreements": a.disagree},
            "findings": by_verdict, "compile_break_findings": breaks,
        });
        let na: usize = not_attr.values().sum();
        println!(
            "{lib}: files {} ({}) certain {certain} agree {agree} conv {} disagree {} not-attr {na} ({:.1}%) | bytecode compared {} agree {} disagree {} | wall {:.0} ms (helper {:.0}, grounds {:.0})",
            results.len(),
            verdicts.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", "),
            conventions.values().sum::<usize>(),
            disagree.len(),
            100.0 * na as f64 / certain.max(1) as f64,
            a.compared,
            a.agree,
            a.disagree.len(),
            wall_ms,
            hm,
            gm
        );
        libs.push(lib_out);
    }
    let out = json!({"tag": o.tag, "jobs": o.jobs, "helper_restarts": RESTARTS.load(Ordering::SeqCst), "cold_start_ms": cold_ms, "total_ms": t_all.elapsed().as_secs_f64() * 1e3, "libs": libs, "files": all_files});
    let path = o.out.join(format!("{}.json", o.tag));
    std::fs::write(&path, serde_json::to_string(&out)?)?;
    println!("cold start {:.0} ms; wrote {}", cold_ms.unwrap_or(0.0), path.display());
    Ok(())
}
