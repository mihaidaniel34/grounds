//! Probe: compare what grounds resolves with what javac recorded in the
//! class files compiled from the same sources. A class file names, for every
//! call, the method javac chose (owner, name, descriptor), for every field
//! access the field, and the source line of each, and it lists the members
//! javac gave each class. Where grounds is certain of something else, grounds
//! is wrong, and every rule that relies on it inherits the mistake.
//! scripts/bytecode_oracle.py fetches sources and classes of published
//! libraries and runs this on them, and fails when a disagreement appears
//! that its baseline does not list.
//!
//!   bytecode_oracle [--json] <project root> <classes directory>
//!
//! The sources are under `<project root>/src/main/java`; the classes
//! directory holds the class files compiled from them (the library's jar,
//! unpacked). Method bodies are read with `grounds_classfile::parse_references`,
//! members with `parse_class`.
//!
//! Each comparison prints what it compared, agreed on, disagreed on and did
//! not compare, then each disagreement on one line; with `--json`, one JSON
//! object with the counts of each comparison and each disagreement as file,
//! line, subject (the name; for members the class and the member) and the
//! two answers, for the script's baseline. The comparisons:
//!
//! - Method calls grounds is certain of, matched by file, line, name and
//!   number of parameters: javac gives the invoke instruction the line of the
//!   `(`. Two calls of one name on a line are not told apart, and calls javac
//!   compiles to something else (string concatenation, accessors for private
//!   members before Java 11) are not compared.
//! - Constructors grounds is certain of (`new T(..)`, `this(..)`, `super(..)`).
//!   javac gives `this(..)` and `super(..)` the line of the `(`, but `new` has
//!   no line entry of its own: its constructor call has the line of whatever
//!   came before it in the statement, so it is looked for on the lines of the
//!   statement up to the end of the expression it is part of (`window`).
//!   javac adds parameters to some constructors (the outer instance first for
//!   an inner class, name and ordinal first for an enum, captured variables
//!   last for a local class): only a call whose parameter count is grounds'
//!   is compared, the others are not.
//! - Fields an identifier or field access refers to, against the get or put
//!   instruction of that name in the same window. javac names the class the
//!   access is made on (the declaring class or a subclass), the erased type,
//!   and a static or instance instruction. A constant is inlined and leaves
//!   no instruction, as does a private field of another class read through
//!   an accessor method before Java 11, so a field with no instruction of its
//!   own is not compared; an instruction that fits another access of that
//!   name in the window, or a field declaration's initializer, is that one's.
//! - Method references: grounds' model resolves a method reference to its
//!   result type, not to one method, so javac's method is not compared with
//!   grounds'. What is compared is whether grounds knows a method of that
//!   name on the receiver's type, which a "has no method" claim rests on,
//!   against the method handle javac recorded on the reference's lines. A
//!   name grounds knows no method of on a receiver some of whose supertypes
//!   it does not have is not compared: it makes no such claim there.
//! - Declared members of every class declared in the sources, as the index
//!   holds them (generated Lombok members included), against its class file:
//!   methods by name and erased parameter descriptors, fields by name. A
//!   parameter type grounds does not know, or knows only by a name no class
//!   has (the sources come without their dependencies), matches any. Not
//!   members in the language sense, so not compared: synthetic and bridge
//!   members, `<clinit>`, an inner class constructor's outer instance, an
//!   enum constructor's name and ordinal, and the private static
//!   `$default$<field>()` Lombok writes for a `@Builder.Default` initializer.
//!   Local and anonymous classes are not compared. A member only the class
//!   file has is listed apart for a class marked `MEMBERS_UNKNOWN`, where
//!   grounds does not claim to know them all.
//!
//! javac numbers local and anonymous classes its own way and grounds
//! differently (JLS 13.1 leaves the numbers to the compiler), so owners are
//! compared without those numbers.

use grounds_classfile::classfile::access;
use grounds_classfile::{parse_class, parse_references, ClassContainer, ClassFile, RefKind};
use grounds_index::{ClassLookup, Flags, MethodInfo, Prim, Ty, TypeParam};
use grounds_semantic::{FileModel, Symbol};
use grounds_syntax::{Node, NodeExt, ParsedFile};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};

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
        _ => return None, // a type variable (its bound decides) or unknown
    })
}

/// The erased descriptor of a declared type (JLS 4.6): a type variable is the
/// erasure of its first bound, found in `scope` (the member's own type
/// parameters before its class's and the enclosing classes'). None for a
/// type grounds does not know.
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

/// Type parameters in scope in a class: its own, then its enclosing classes'.
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
    let Some((ps, ret)) = d.strip_prefix('(').and_then(|d| d.split_once(')')) else { return (Vec::new(), String::new()) };
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

/// The simple name of a binary (`a.b.C$D`) or internal (`a/b/C$1D`) name: a
/// local class's has javac's number in front, which is dropped.
fn simple(name: &str) -> &str {
    let last = name.rsplit(['.', '/', '$']).next().unwrap_or(name);
    last.trim_start_matches(|c: char| c.is_ascii_digit())
}

/// A binary or internal name as a binary name without the numbers of local
/// and anonymous classes (`a.B$#`, `a.B$#Local`): JLS 13.1 leaves them to
/// the compiler, and grounds numbers them otherwise than javac does.
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

/// A field grounds says a name refers to (`node`), or one a declaration
/// initializes (no node).
struct FieldUse<'t> {
    node: Option<Node<'t>>,
    line: usize,
    owner: String,
    name: String,
    is_static: bool,
    /// erased type; None where grounds does not know it
    ty: Option<String>,
    /// a constant variable (JLS 4.12.4) by grounds' evaluation
    constant: bool,
}

/// One reference in a method body. Owner as a binary name; a field's
/// descriptor is its `ret`.
struct Site {
    kind: RefKind,
    owner: String,
    name: String,
    params: Vec<String>,
    ret: String,
}

/// An invoke instruction's owner, parameter descriptors and return descriptor.
type Invoke = (String, Vec<String>, String);

#[derive(Default)]
struct Bytecode {
    /// (file, line, name, number of parameters) -> (owner, parameters, return) of each invoke instruction
    calls: HashMap<(String, usize, String, usize), Vec<Invoke>>,
    /// file -> line -> every reference on it
    lines: HashMap<String, HashMap<usize, Vec<Site>>>,
    /// internal name -> declarations
    classes: HashMap<String, ClassFile>,
    /// internal name -> its source file (`a/b/C.java`)
    source: HashMap<String, String>,
    unreadable: Vec<String>,
}

fn read_classes(dir: &std::path::Path) -> Bytecode {
    // multi-release copies under META-INF would be second copies
    let names: Vec<String> = ClassContainer::open_dir(dir).class_names().into_iter().filter(|n| !n.starts_with("META-INF/")).collect();
    let parsed: Vec<_> = names
        .par_iter()
        .map(|n| {
            let bytes = std::fs::read(dir.join(format!("{n}.class")));
            let r = bytes.map_err(anyhow::Error::from).and_then(|b| Ok((parse_references(&b)?, parse_class(&b)?)));
            (n, r)
        })
        .collect();
    let mut bc = Bytecode::default();
    for (n, r) in parsed {
        let (refs, cf) = match r {
            Ok(x) => x,
            Err(e) => {
                bc.unreadable.push(format!("{n}: {e}"));
                continue;
            }
        };
        if let Some(src) = &refs.source_file {
            let file = match refs.name.rsplit_once('/') {
                Some((pkg, _)) => format!("{pkg}/{src}"),
                None => src.clone(),
            };
            bc.source.insert(n.clone(), file.clone());
            let lines = bc.lines.entry(file.clone()).or_default();
            for m in refs.methods {
                for r in m.refs {
                    let Some(line) = r.line else { continue };
                    let (params, ret) = if r.kind.is_field() { (Vec::new(), r.descriptor) } else { split(&r.descriptor) };
                    let owner = r.owner.replace('/', ".");
                    if matches!(r.kind, RefKind::InvokeVirtual | RefKind::InvokeSpecial | RefKind::InvokeStatic | RefKind::InvokeInterface) {
                        bc.calls.entry((file.clone(), line as usize, r.name.clone(), params.len())).or_default().push((owner.clone(), params.clone(), ret.clone()));
                    }
                    lines.entry(line as usize).or_default().push(Site { kind: r.kind, owner, name: r.name, params, ret });
                }
            }
        }
        bc.classes.insert(n.clone(), cf);
    }
    bc
}

/// Lines where javac may have put the instruction of an expression that has
/// no line entry of its own (a constructor call, a field access): from the
/// start of the statement or declaration it is in, which has an entry, to
/// the end of the part of it that holds the expression (the condition of an
/// `if`, not its body; the body of a lambda).
fn window<'t>(fm: &FileModel<'t>, pf: &ParsedFile, n: Node<'t>) -> (usize, usize) {
    let line = |b: usize| pf.lines.line_of(b as u32) + 1;
    let anchor = |k: &str| {
        k.ends_with("_statement")
            || matches!(k, "local_variable_declaration" | "field_declaration" | "constant_declaration" | "explicit_constructor_invocation" | "lambda_expression" | "switch_rule" | "enum_constant" | "method_declaration" | "constructor_declaration" | "class_body")
    };
    let mut part = n;
    while let Some(p) = fm.parent_of(part) {
        if anchor(p.kind()) {
            return (line(p.start_byte()), line(part.end_byte().saturating_sub(1)));
        }
        part = p;
    }
    (line(n.start_byte()), line(n.end_byte().saturating_sub(1)))
}

/// One disagreement: where, about what, and the two answers. `member` is
/// empty but for declared members, whose `subject` is the class.
struct Dis {
    category: &'static str,
    kind: &'static str,
    file: String,
    line: usize,
    subject: String,
    member: String,
    grounds: String,
    javac: String,
}

impl Dis {
    fn new(category: &'static str, kind: &'static str, file: &str, line: usize, subject: &str, grounds: String, javac: String) -> Dis {
        Dis { category, kind, file: file.to_string(), line, subject: subject.to_string(), member: String::new(), grounds, javac }
    }
    fn member(kind: &'static str, file: &str, line: usize, class: &str, member: String, grounds: String, javac: String) -> Dis {
        Dis { member, ..Dis::new("members", kind, file, line, class, grounds, javac) }
    }
    fn text(&self) -> String {
        let prefix = match self.category {
            "constructors" => "CTOR ",
            "fields" => "FIELD ",
            "method references" => "METHODREF ",
            "members" => "MEMBER ",
            _ => "",
        };
        let member = if self.member.is_empty() { String::new() } else { format!(": {}", self.member) };
        format!("{prefix}{}  {}:{} {}{member}: grounds {}, javac {}", self.kind.to_uppercase(), self.file, self.line, self.subject, self.grounds, self.javac)
    }
    fn json(&self) -> serde_json::Value {
        serde_json::json!({"category": self.category, "kind": self.kind, "file": self.file, "line": self.line, "subject": self.subject, "member": self.member, "grounds": self.grounds, "javac": self.javac})
    }
}

/// Compared, agreed, not compared (by why), and the disagreements.
#[derive(Default)]
struct Count {
    compared: usize,
    agree: usize,
    not_compared: BTreeMap<&'static str, usize>,
    disagree: Vec<Dis>,
}

impl Count {
    fn skip(&mut self, why: &'static str) {
        *self.not_compared.entry(why).or_default() += 1;
    }
    fn add(&mut self, o: Count) {
        self.compared += o.compared;
        self.agree += o.agree;
        for (k, v) in o.not_compared {
            *self.not_compared.entry(k).or_default() += v;
        }
        self.disagree.extend(o.disagree);
    }
    fn line(&self, what: &str) -> String {
        let why: Vec<String> = self.not_compared.iter().map(|(k, v)| format!("{k} {v}")).collect();
        let n: usize = self.not_compared.values().sum();
        format!("{what}: compared {}  agree {}  disagree {}  not compared {n}  ({})", self.compared, self.agree, self.disagree.len(), why.join(", "))
    }
    fn json(&self) -> serde_json::Value {
        serde_json::json!({"compared": self.compared, "agree": self.agree, "disagree": self.disagree.len(), "not_compared": self.not_compared.values().sum::<usize>()})
    }
}

#[derive(Default)]
struct Members {
    classes: usize,
    /// declared in the sources, with no class file of that name
    no_class_file: usize,
    /// matched by name and parameters (methods), by name (fields)
    matched: usize,
    grounds_only: Vec<Dis>,
    /// in classes grounds believes it knows every member of
    file_only: Vec<Dis>,
    /// in classes marked `MEMBERS_UNKNOWN`
    file_only_open: Vec<Dis>,
    /// of `matched`, methods the Lombok model generated
    generated_matched: usize,
    skipped: BTreeMap<&'static str, usize>,
}

#[derive(Default)]
struct Tally {
    calls: usize,
    resolved: usize,
    certain: usize,
    compared: usize,
    agree: usize,
    other_overload: Vec<Dis>,
    other_owner: Vec<Dis>,
    other_return: Vec<Dis>,
    /// javac names another method by a rule of its own, not by another choice
    javac_quirk: usize,
    unresolved_known_to_javac: usize,
    guess_compared: usize,
    guess_wrong: usize,
    ctors: Count,
    ctor_unresolved: usize,
    ctor_guessed: usize,
    fields: Count,
    method_refs: Count,
    members: Members,
}

impl Tally {
    fn add(&mut self, t: Tally) {
        self.calls += t.calls;
        self.resolved += t.resolved;
        self.certain += t.certain;
        self.compared += t.compared;
        self.agree += t.agree;
        self.javac_quirk += t.javac_quirk;
        self.unresolved_known_to_javac += t.unresolved_known_to_javac;
        self.guess_compared += t.guess_compared;
        self.guess_wrong += t.guess_wrong;
        self.other_overload.extend(t.other_overload);
        self.other_owner.extend(t.other_owner);
        self.other_return.extend(t.other_return);
        self.ctors.add(t.ctors);
        self.ctor_unresolved += t.ctor_unresolved;
        self.ctor_guessed += t.ctor_guessed;
        self.fields.add(t.fields);
        self.method_refs.add(t.method_refs);
        let (m, o) = (&mut self.members, t.members);
        m.classes += o.classes;
        m.no_class_file += o.no_class_file;
        m.matched += o.matched;
        m.grounds_only.extend(o.grounds_only);
        m.file_only.extend(o.file_only);
        m.file_only_open.extend(o.file_only_open);
        m.generated_matched += o.generated_matched;
        for (k, v) in o.skipped {
            *m.skipped.entry(k).or_default() += v;
        }
    }
}

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    let json = args.iter().any(|a| a == "--json");
    args.retain(|a| a != "--json");
    anyhow::ensure!(args.len() == 3, "usage: bytecode_oracle [--json] <project root> <classes directory>");
    let root = std::path::PathBuf::from(&args[1]).canonicalize()?;
    let bc = read_classes(&std::path::PathBuf::from(&args[2]).canonicalize()?);
    let p = grounds_engine::Project::load(&root)?;
    let src_root = root.join("src/main/java");
    let files: Vec<_> = p.workspace.files.iter().collect();
    let no_lines = HashMap::new();
    let tallies: Vec<Tally> = files
        .par_iter()
        .map(|f| {
            let mut t = Tally::default();
            let Ok(rel) = f.path.strip_prefix(&src_root) else { return t };
            let rel = rel.to_string_lossy().to_string();
            let Ok(text) = std::fs::read_to_string(&f.path) else { return t };
            let pf = grounds_syntax::ParsedFile::parse(text);
            let fm = grounds_semantic::FileModel::new(&pf, &p.index, grounds_engine::project::path_key(&f.path));
            let line = |b: usize| pf.lines.line_of(b as u32) + 1;
            let lines = bc.lines.get(&rel).unwrap_or(&no_lines);
            let in_window = |(lo, hi): (usize, usize)| (lo..=hi).filter_map(|l| lines.get(&l)).flatten();
            for n in pf.root().descendants().filter(|n| n.kind() == "method_invocation") {
                t.calls += 1;
                let (Some(name), Some(al)) = (n.field("name").map(|x| fm.text(x)), n.field("arguments")) else { continue };
                // javac gives the invoke instruction the line of the `(`
                let line = line(al.start_byte());
                let nargs = fm.args_of(n).len();
                let Some(r) = fm.resolve_call(n) else {
                    if bc.calls.contains_key(&(rel.clone(), line, name.to_string(), nargs)) {
                        t.unresolved_known_to_javac += 1;
                    }
                    continue;
                };
                t.resolved += 1;
                let certain = !r.ambiguous && !r.erasure_only;
                if certain {
                    t.certain += 1;
                }
                // the declared method behind the substituted one
                let Some(owner) = fm.class(&r.owner) else { continue };
                let subst: Vec<Option<String>> = r.method.params.iter().map(|q| desc(&q.ty.erasure())).collect();
                let cands: Vec<&MethodInfo> = owner
                    .methods_named(name)
                    .filter(|m| m.params.len() == subst.len())
                    .filter(|m| m.params.iter().zip(&subst).all(|(q, s)| match (desc(&q.ty), s) {
                        (Some(d), Some(s)) => d == *s,
                        _ => true,
                    }))
                    .collect();
                if cands.is_empty() {
                    continue;
                }
                // javac's descriptor has the declared arity: varargs and fixed alike
                let Some(sites) = bc.calls.get(&(rel.clone(), line, name.to_string(), subst.len())) else { continue };
                let fits = |m: &MethodInfo, ps: &[String]| m.params.iter().zip(ps).all(|(q, d)| desc(&q.ty).is_none_or(|x| x == *d));
                let dis = |kind, grounds: String, javac: String| Dis::new("calls", kind, &rel, line, name, grounds, javac);
                // a signature-polymorphic method (JLS 15.12.3): the call's own
                // argument types are its descriptor
                if matches!(&*r.owner, "java.lang.invoke.MethodHandle" | "java.lang.invoke.VarHandle") {
                    t.javac_quirk += 1;
                    continue;
                }
                let same_params: Vec<&Invoke> = sites.iter().filter(|s| cands.iter().any(|m| fits(m, &s.1))).collect();
                if !certain {
                    t.guess_compared += 1;
                    if same_params.is_empty() {
                        t.guess_wrong += 1;
                    }
                    continue;
                }
                t.compared += 1;
                if same_params.is_empty() {
                    t.other_overload.push(dis("other overload", format!("{}.{}", r.owner, r.method.signature_string()), sites.iter().map(|s| format!("{}.{name}({})", s.0, s.1.join(""))).collect::<Vec<_>>().join(" | ")));
                    continue;
                }
                // javac names the type the call is made on: the declaring class or a subtype
                // a method of Object called on an interface that declares it
                // again (Collection.equals): javac names Object's
                if same_params.iter().all(|s| s.0 == "java.lang.Object") && owner.is_interface() {
                    t.javac_quirk += 1;
                    continue;
                }
                if !same_params.iter().any(|s| !fm.is_subclass(&s.0, &r.owner).is_no() || s.0.starts_with('[')) {
                    t.other_owner.push(dis("other owner", r.owner.to_string(), same_params.iter().map(|s| s.0.as_str()).collect::<Vec<_>>().join(" | ")));
                    continue;
                }
                if !same_params.iter().any(|s| cands.iter().any(|m| fits(m, &s.1) && desc(&m.ret).is_none_or(|x| x == s.2))) {
                    t.other_return.push(dis("other return", format!("{} {}.{}", cands[0].ret, r.owner, r.method.signature_string()), format!("returns {}", same_params[0].2)));
                    continue;
                }
                t.agree += 1;
            }

            // ----- constructors -----
            for n in pf.root().descendants().filter(|n| matches!(n.kind(), "object_creation_expression" | "explicit_constructor_invocation")) {
                let Some(al) = n.field("arguments") else { continue };
                let Some(r) = fm.select_method(n) else {
                    t.ctor_unresolved += 1;
                    continue;
                };
                if r.ambiguous {
                    t.ctor_guessed += 1;
                    continue;
                }
                let c = &mut t.ctors;
                let Some(owner) = fm.class(&r.owner) else {
                    c.skip("class unknown");
                    continue;
                };
                let scope_owned = class_type_params(&fm, &owner.name);
                let subst: Vec<Option<String>> = r.method.params.iter().map(|q| desc(&q.ty.erasure())).collect();
                // the declared constructor behind the substituted one, with its erased parameters
                let cands: Vec<Vec<Option<String>>> = owner
                    .constructors()
                    .filter(|m| m.params.len() == subst.len())
                    .filter(|m| m.params.iter().zip(&subst).all(|(q, s)| match (desc(&q.ty), s) {
                        (Some(d), Some(s)) => d == *s,
                        _ => true,
                    }))
                    .map(|m| {
                        let scope: Vec<&TypeParam> = m.type_params.iter().chain(&scope_owned).collect();
                        m.params.iter().map(|q| erased(&q.ty, &scope, 0)).collect::<Vec<_>>()
                    })
                    .collect();
                if cands.is_empty() {
                    c.skip("declared constructor not found");
                    continue;
                }
                let fits = |ps: &[String]| cands.iter().any(|c| c.iter().zip(ps).all(|(d, p)| d.as_ref().is_none_or(|d| d == p)));
                let explicit = n.kind() == "explicit_constructor_invocation";
                // `this(..)`, `super(..)`: a call, on the line of its `(`
                let win = if explicit { (line(al.start_byte()), line(al.start_byte())) } else { window(&fm, &pf, n) };
                let at = (line(n.start_byte()), simple(&r.owner));
                let inits: Vec<&Site> = in_window(win).filter(|s| s.kind == RefKind::InvokeSpecial && s.name == "<init>").collect();
                let sites: Vec<&Site> = inits.iter().copied().filter(|s| s.params.len() == subst.len()).collect();
                let same_class: Vec<&&Site> = sites.iter().filter(|s| unnumbered(&s.owner) == unnumbered(&r.owner)).collect();
                if same_class.is_empty() {
                    // a class of that name, but another: grounds resolved the name to another class
                    let named: Vec<&&Site> = sites.iter().filter(|s| simple(&s.owner) == simple(&r.owner)).collect();
                    if named.is_empty() {
                        c.skip(if inits.iter().any(|s| unnumbered(&s.owner) == unnumbered(&r.owner)) {
                            "parameters javac adds (outer instance, enum name and ordinal, captured variables)"
                        } else {
                            "no constructor call of that class in the window"
                        });
                    } else {
                        c.compared += 1;
                        c.disagree.push(Dis::new("constructors", "other class", &rel, at.0, at.1, r.owner.to_string(), named.iter().map(|s| s.owner.as_str()).collect::<Vec<_>>().join(" | ")));
                    }
                    continue;
                }
                c.compared += 1;
                if same_class.iter().any(|s| fits(&s.params)) {
                    c.agree += 1;
                } else {
                    c.disagree.push(Dis::new("constructors", "other overload", &rel, at.0, at.1, format!("{}.{}", r.owner, r.method.signature_string()), same_class.iter().map(|s| format!("<init>({})", s.params.join(""))).collect::<Vec<_>>().join(" | ")));
                }
            }

            // ----- fields -----
            // every field grounds says a name refers to, and every field a
            // declaration initializes (its instruction is no other access's)
            let mut uses: Vec<FieldUse> = Vec::new();
            let erased_field = |owner: &str, name: &str| {
                let scope_owned = class_type_params(&fm, owner);
                let scope: Vec<&TypeParam> = scope_owned.iter().collect();
                fm.class(owner).and_then(|o| o.field(name).and_then(|d| erased(&d.ty, &scope, 0)))
            };
            for n in pf.root().descendants() {
                match n.kind() {
                    "identifier" | "field_access" => {
                        // the name in `obj.f` is counted with the field access
                        if n.kind() == "identifier" && fm.parent_of(n).is_some_and(|p| p.kind() == "field_access" && p.field("field") == Some(n)) {
                            continue;
                        }
                        let Some(Symbol::Field(f)) = fm.symbol(n) else { continue };
                        let ty = erased_field(&f.owner, &f.field.name);
                        uses.push(FieldUse { node: Some(n), line: line(n.start_byte()), owner: f.owner.to_string(), name: f.field.name.to_string(), is_static: f.field.flags.is_static(), ty, constant: fm.constant_value(n).is_some() });
                    }
                    "variable_declarator" if n.field("value").is_some() && fm.parent_of(n).is_some_and(|p| matches!(p.kind(), "field_declaration" | "constant_declaration")) => {
                        let (Some(f), Some(owner)) = (fm.field_of_declarator(n), fm.enclosing_class_name(n)) else { continue };
                        let ty = erased_field(&owner, &f.name);
                        uses.push(FieldUse { node: None, line: line(n.start_byte()), owner: owner.to_string(), name: f.name.to_string(), is_static: f.flags.is_static(), ty, constant: false });
                    }
                    _ => {}
                }
            }
            // javac names the class the access is made on: the declaring class or a subclass
            let owner_ok = |u: &FieldUse, s: &Site| unnumbered(&s.owner) == unnumbered(&u.owner) || !fm.is_subclass(&s.owner, &u.owner).is_no();
            let static_ok = |u: &FieldUse, s: &Site| matches!(s.kind, RefKind::GetStatic | RefKind::PutStatic) == u.is_static;
            let type_ok = |u: &FieldUse, s: &Site| u.ty.as_ref().is_none_or(|d| *d == s.ret);
            let fits = |u: &FieldUse, s: &Site| owner_ok(u, s) && static_ok(u, s) && type_ok(u, s);
            for (i, u) in uses.iter().enumerate() {
                let Some(n) = u.node else { continue };
                let c = &mut t.fields;
                // a constant is inlined: javac leaves no instruction for it
                if u.constant {
                    c.skip("a constant, inlined");
                    continue;
                }
                let (lo, hi) = window(&fm, &pf, n);
                let sites: Vec<&Site> = in_window((lo, hi)).filter(|s| s.kind.is_field() && !matches!(s.kind, RefKind::Handle(_)) && s.name == u.name).collect();
                if sites.is_empty() {
                    c.skip("no instruction of that name in the window");
                    continue;
                }
                if sites.iter().any(|s| fits(u, s)) {
                    c.compared += 1;
                    c.agree += 1;
                    continue;
                }
                // what another access of that name in the window explains is
                // that access's: this one may have no instruction (a private
                // field of another class read through an accessor method before
                // Java 11, a constant grounds does not evaluate)
                let sites: Vec<&Site> = sites.into_iter().filter(|s| !uses.iter().enumerate().any(|(j, o)| j != i && o.name == u.name && (lo..=hi).contains(&o.line) && fits(o, s))).collect();
                if sites.is_empty() {
                    c.skip("the instructions of that name are other accesses'");
                    continue;
                }
                c.compared += 1;
                let what = if !sites.iter().any(|s| owner_ok(u, s)) {
                    "other owner"
                } else if !sites.iter().any(|s| owner_ok(u, s) && static_ok(u, s)) {
                    "other kind"
                } else {
                    "other type"
                };
                let javac = sites.iter().map(|s| format!("{:?} {}.{}:{}", s.kind, s.owner, s.name, s.ret)).collect::<Vec<_>>().join(" | ");
                let grounds = format!("{}{}.{}:{}", if u.is_static { "static " } else { "" }, u.owner, u.name, u.ty.as_deref().unwrap_or("?"));
                c.disagree.push(Dis::new("fields", what, &rel, u.line, &u.name, grounds, javac));
            }

            // ----- method references: does grounds know a method of that name -----
            for n in pf.root().descendants().filter(|n| n.kind() == "method_reference") {
                let kids = n.kids();
                // `Type::new` has a single named child
                if kids.len() < 2 || fm.text(n).rsplit("::").next().is_some_and(|r| r.trim_start().starts_with("new")) {
                    continue;
                }
                let (recv, mname) = (kids[0], kids[kids.len() - 1]);
                if mname.kind() != "identifier" || recv.kind() == "super" || recv.kids().iter().any(|k| k.kind() == "super") {
                    continue;
                }
                let mname = fm.text(mname);
                let c = &mut t.method_refs;
                let sites: Vec<&Site> = in_window(window(&fm, &pf, n)).filter(|s| matches!(s.kind, RefKind::Handle(5..=9)) && s.name == mname).collect();
                let cls = match fm.symbol(recv) {
                    Some(Symbol::Type(t)) => fm.class(&t),
                    Some(Symbol::Package(_)) => None,
                    _ => fm.type_of(recv).class_name().and_then(|c| fm.class(c)),
                };
                // javac compiles some references as lambdas (`super::m`, a varargs method): no handle names them
                if sites.is_empty() {
                    c.skip("no method handle of that name in the window");
                    continue;
                }
                let Some(cls) = cls else {
                    c.skip("receiver type unknown");
                    continue;
                };
                let none = fm.methods_named(&cls.this_type(), mname).is_empty();
                // with a supertype grounds does not have (the sources come
                // without their dependencies) it claims no method missing: no
                // "has no method" claim rests on what it does not know
                if none && !fm.hierarchy_complete(&cls.name) {
                    c.skip("no method known, supertypes not all known");
                    continue;
                }
                c.compared += 1;
                if none {
                    let open = if cls.flags.has(Flags::MEMBERS_UNKNOWN) { " (members unknown)" } else { "" };
                    let javac = sites.iter().map(|s| format!("{}.{}{}", s.owner, s.name, s.params.join(""))).collect::<Vec<_>>().join(" | ");
                    c.disagree.push(Dis::new("method references", "no method", &rel, line(n.start_byte()), mname, format!("knows no `{mname}` on {}{open}", cls.name), javac));
                } else {
                    c.agree += 1;
                }
            }

            // ----- declared members -----
            for (decl, c) in fm.declared_classes() {
                let ms = &mut t.members;
                if c.flags.has(Flags::LOCAL | Flags::ANON) {
                    *ms.skipped.entry("local or anonymous class").or_default() += 1;
                    continue;
                }
                let Some(c) = p.index.class(&c.name) else { continue };
                let Some(cf) = bc.classes.get(&c.name.replace('.', "/")) else {
                    ms.no_class_file += 1;
                    continue;
                };
                ms.classes += 1;
                let open = c.flags.has(Flags::MEMBERS_UNKNOWN);
                let at = line(decl.field("name").unwrap_or(decl).start_byte());
                let dis = |kind, member: String, grounds: String, javac: &str| Dis::member(kind, &rel, at, &c.name, member, grounds, javac.to_string());
                let scope_owned = class_type_params(&fm, &c.name);
                // grounds' methods: name, erased parameters, and how it came to be
                let g: Vec<(&str, Vec<Option<String>>, &str)> = c
                    .methods
                    .iter()
                    .map(|m| {
                        let scope: Vec<&TypeParam> = m.type_params.iter().chain(&scope_owned).collect();
                        let how = if m.generated_by.is_some() || m.flags.has(Flags::GENERATED) {
                            " (generated)"
                        } else if m.flags.has(Flags::IMPLICIT) {
                            " (implicit)"
                        } else {
                            ""
                        };
                        (&*m.name, m.params.iter().map(|q| erased(&q.ty, &scope, 0)).collect(), how)
                    })
                    .collect();
                // an inner (non-static member) class's constructors take the outer instance first
                let inner = cf.access & access::INTERFACE == 0 && cf.inner_classes.iter().any(|ic| ic.inner == cf.this_class && ic.outer.is_some() && ic.access & access::STATIC == 0);
                let is_enum = cf.access & access::ENUM != 0;
                let mut b: Vec<(&str, Vec<String>, &str)> = Vec::new();
                for m in &cf.methods {
                    if m.access & (access::SYNTHETIC | access::BRIDGE) != 0 {
                        *ms.skipped.entry("synthetic or bridge method").or_default() += 1;
                        continue;
                    }
                    if m.name == "<clinit>" {
                        *ms.skipped.entry("<clinit>").or_default() += 1;
                        continue;
                    }
                    // Lombok moves a `@Builder.Default` initializer into a private
                    // static `$default$<field>()` the builder calls: an
                    // implementation detail no source can name (`$` names are
                    // the compiler's), not a member grounds should declare
                    if m.name.starts_with("$default$") && m.access & (access::PRIVATE | access::STATIC) == access::PRIVATE | access::STATIC {
                        *ms.skipped.entry("Lombok's private @Builder.Default initializer").or_default() += 1;
                        continue;
                    }
                    let (mut ps, _) = split(&m.descriptor);
                    if m.name == "<init>" {
                        if is_enum && ps.len() >= 2 && ps[0] == "Ljava/lang/String;" && ps[1] == "I" {
                            ps.drain(..2);
                            *ms.skipped.entry("enum constructor's name and ordinal parameters").or_default() += 1;
                        } else if inner && !ps.is_empty() {
                            ps.remove(0);
                            *ms.skipped.entry("inner class constructor's outer instance parameter").or_default() += 1;
                        }
                    }
                    b.push((&m.name, ps, &m.descriptor));
                }
                // a parameter whose type grounds does not know, or knows only by a
                // name no class has (sources without their dependencies), may be any
                let known = |d: &Option<String>| {
                    d.as_deref().is_some_and(|d| match d.trim_start_matches('[').strip_prefix('L').and_then(|c| c.strip_suffix(';')) {
                        Some(c) => fm.class(&c.replace('/', ".")).is_some(),
                        None => true,
                    })
                };
                let (mut used_g, mut used_b) = (vec![false; g.len()], vec![false; b.len()]);
                for lenient in [false, true] {
                    for (i, (gn, gp, _)) in g.iter().enumerate() {
                        if used_g[i] {
                            continue;
                        }
                        let same = |j: usize| {
                            let (bn, bp, _) = &b[j];
                            !used_b[j] && bn == gn && bp.len() == gp.len() && gp.iter().zip(bp).all(|(x, y)| x.as_deref() == Some(y.as_str()) || (lenient && !known(x)))
                        };
                        if let Some(j) = (0..b.len()).find(|&j| same(j)) {
                            used_g[i] = true;
                            used_b[j] = true;
                            ms.matched += 1;
                            if g[i].2 == " (generated)" {
                                ms.generated_matched += 1;
                            }
                        }
                    }
                }
                for (i, (gn, gp, how)) in g.iter().enumerate() {
                    if !used_g[i] {
                        let ps: Vec<&str> = gp.iter().map(|d| d.as_deref().unwrap_or("?")).collect();
                        ms.grounds_only.push(dis("grounds only", format!("method {gn}({})", ps.join("")), format!("declares it{how}"), "does not"));
                    }
                }
                // what a lookup of the name finds: a "has no method" claim rests on none
                let this = c.this_type();
                for (j, (bn, _, d)) in b.iter().enumerate() {
                    if used_b[j] {
                        continue;
                    }
                    let found = if g.iter().any(|(gn, _, _)| gn == bn) {
                        "another overload declared"
                    } else if !fm.methods_named(&this, bn).is_empty() {
                        "one inherited"
                    } else {
                        "no method of that name"
                    };
                    let (member, grounds) = (format!("method {bn}{d}"), format!("does not declare it ({found})"));
                    if open {
                        ms.file_only_open.push(dis("class file only, members unknown", member, grounds, "declares it"));
                    } else {
                        ms.file_only.push(dis("class file only", member, grounds, "declares it"));
                    }
                }
                // fields, by name
                for f in &c.fields {
                    if cf.fields.iter().any(|x| x.name == *f.name && x.access & access::SYNTHETIC == 0) {
                        ms.matched += 1;
                    } else {
                        ms.grounds_only.push(dis("grounds only", format!("field {}", f.name), "declares it".to_string(), "does not"));
                    }
                }
                for x in &cf.fields {
                    if x.access & access::SYNTHETIC != 0 {
                        *ms.skipped.entry("synthetic field").or_default() += 1;
                    } else if !c.fields.iter().any(|f| *f.name == x.name) {
                        let found = if fm.field(&this, &x.name).is_some() { "one inherited" } else { "no field of that name" };
                        let (member, grounds) = (format!("field {}:{}", x.name, x.descriptor), format!("does not declare it ({found})"));
                        if open {
                            ms.file_only_open.push(dis("class file only, members unknown", member, grounds, "declares it"));
                        } else {
                            ms.file_only.push(dis("class file only", member, grounds, "declares it"));
                        }
                    }
                }
            }
            t
        })
        .collect();
    let mut s = Tally::default();
    for t in tallies {
        s.add(t);
    }
    // class files compiled from a source file of the project, of a class grounds
    // does not have: not local, anonymous, inside one, or synthetic (a switch map). (A jar
    // may hold classes of other projects, shaded, whose sources it lacks.)
    let mut no_source: Vec<&String> = bc
        .classes
        .iter()
        .filter(|(n, cf)| cf.access & access::SYNTHETIC == 0 && !unnumbered(n).contains("$#"))
        .filter(|(n, _)| bc.source.get(*n).is_some_and(|f| src_root.join(f).is_file()))
        .filter(|(n, _)| !p.index.class_exists(&n.replace('/', ".")))
        .map(|(n, _)| n)
        .collect();
    no_source.sort();

    let m = &s.members;
    if json {
        let calls = serde_json::json!({"compared": s.compared, "agree": s.agree, "disagree": s.other_overload.len() + s.other_owner.len() + s.other_return.len(), "not_compared": s.resolved - s.compared});
        let members = serde_json::json!({"compared": m.matched + m.grounds_only.len() + m.file_only.len(), "agree": m.matched, "disagree": m.grounds_only.len() + m.file_only.len(), "not_compared": m.file_only_open.len()});
        let all = s.other_overload.iter().chain(&s.other_owner).chain(&s.other_return).chain(&s.ctors.disagree).chain(&s.fields.disagree).chain(&s.method_refs.disagree).chain(&m.file_only).chain(&m.grounds_only);
        let out = serde_json::json!({
            "files": files.len(),
            "categories": {"calls": calls, "constructors": s.ctors.json(), "fields": s.fields.json(), "method references": s.method_refs.json(), "members": members},
            "disagreements": all.map(Dis::json).collect::<Vec<_>>(),
            "class_files_without_source_class": no_source.len(),
            "unreadable_class_files": bc.unreadable.len(),
        });
        println!("{out}");
        return Ok(());
    }
    println!("files {}  calls {}  resolved {}  certain {}", files.len(), s.calls, s.resolved, s.certain);
    println!("certain calls found in bytecode {}  agree {}  other overload {}  other owner {}  other return {}  javac's own naming {}", s.compared, s.agree, s.other_overload.len(), s.other_owner.len(), s.other_return.len(), s.javac_quirk);
    println!("guessed (ambiguous/erasure-only) calls found in bytecode {}  of which javac chose another {}", s.guess_compared, s.guess_wrong);
    println!("unresolved by grounds, resolved by javac {}", s.unresolved_known_to_javac);
    println!("{}  (unresolved {}, guessed {})", s.ctors.line("constructors"), s.ctor_unresolved, s.ctor_guessed);
    println!("{}", s.fields.line("fields"));
    println!("{}", s.method_refs.line("method reference names"));
    println!(
        "members: classes {}  compared {}  agree {}  disagree {}  not compared {}  (agree on {} generated by the Lombok model; grounds only {}, class file only {}, of which no member of that name even inherited {}; class file only in classes marked MEMBERS_UNKNOWN {}; source classes without a class file {}; class files without a source class {})",
        m.classes,
        m.matched + m.grounds_only.len() + m.file_only.len(),
        m.matched,
        m.grounds_only.len() + m.file_only.len(),
        m.file_only_open.len(),
        m.generated_matched,
        m.grounds_only.len(),
        m.file_only.len(),
        m.file_only.iter().filter(|x| x.grounds.ends_with("of that name)")).count(),
        m.file_only_open.len(),
        m.no_class_file,
        no_source.len()
    );
    println!("members not compared: {}", m.skipped.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", "));
    if !bc.unreadable.is_empty() {
        println!("unreadable class files {}", bc.unreadable.len());
    }
    for x in s.other_overload.iter().chain(&s.other_owner).chain(&s.other_return).chain(&s.ctors.disagree).chain(&s.fields.disagree).chain(&s.method_refs.disagree).chain(&m.file_only).chain(&m.grounds_only).chain(&m.file_only_open) {
        println!("{}", x.text());
    }
    for n in no_source {
        println!("CLASS FILE ONLY  {n}");
    }
    for x in &bc.unreadable {
        println!("UNREADABLE  {x}");
    }
    Ok(())
}
