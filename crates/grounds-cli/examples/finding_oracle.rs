//! Probe: are grounds' findings true? For rules whose claim the class files
//! compiled from the same sources can decide, each finding is checked
//! against what javac recorded, and is confirmed, refuted, or not decidable
//! here (with the reason). No one reads a sample: the class file answers.
//! scripts/finding_oracle.py runs this over published libraries (sources and
//! the classes compiled from them) and adds the answers up per rule.
//!
//!   finding_oracle [--json] <project root> <classes directory>
//!
//! The sources are under `<project root>/src/main/java`, the classes
//! directory holds the class files compiled from them (the library's jar,
//! unpacked). The rules run as `grounds check` runs them: the project's
//! configuration selects them, `analyze::analyze_files` runs them, and the
//! doubt policy withholds what it withholds for a user. A finding is mapped
//! to the member or class it is about by its position and the source model
//! (the declaration node at its range), never by its message.
//!
//! Three oracles:
//!
//! - compiled: the rule says the code does not compile (the compile-break
//!   rules of `STABLE_IN_CHANGED` and the rest of resolution.rs). A class file
//!   whose `SourceFile` is the finding's file means the file compiled, so the
//!   finding is refuted. Without one it is not decidable.
//! - unreferenced: the rule says a private member is not used. The class
//!   files of the member's top-level class and its nested classes (the only
//!   code that may name a private member) are searched for a reference to it:
//!   an invoke, get or put instruction, or a method handle of an
//!   `invokedynamic`. Each rule's own meaning of "used" is mirrored: for
//!   `unused-private-field` a read (a get), not a write; for
//!   `unused-private-method` any call, also from its own body. javac does not
//!   inline methods, but it does inline the reads of a constant variable
//!   (a field with a `ConstantValue`), so those are not decidable. A member
//!   used only by reflection has no reference: the literal claim holds, it is
//!   confirmed, and how many of those carry an annotation or a serialization
//!   name in the class file is counted apart.
//! - declaration: the rule's claim is a fact about a declaration the class
//!   file states exactly (access flags, a descriptor, a supertype, the
//!   methods a class declares). The rule's own condition is recomputed from
//!   the class file. Supertypes outside the classes directory are read from
//!   the JDK the probe runs on; one in neither (a dependency of the library)
//!   leaves the finding not decidable.
//!
//! Local and anonymous classes are not decided: javac numbers them its own
//! way (JLS 13.1), so which class file is which is not known by name.

use grounds_classfile::classfile::access;
use grounds_classfile::{parse_class, parse_references, ClassContainer, ClassFile, ClassReferences, RefKind};
use grounds_engine::analyze::{self, Finding};
use grounds_index::{ClassInfo, ClassLookup, Flags, Prim, Ty, TypeParam};
use grounds_semantic::FileModel;
use grounds_syntax::{Node, NodeExt, ParsedFile};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

type CfMethod = grounds_classfile::MethodInfo;

/// The rules that say the code does not compile: those `grounds changed`
/// holds to the stable promise, and the other two of resolution.rs.
fn is_compile_break(rule: &str) -> bool {
    grounds_rules::STABLE_IN_CHANGED.contains(&rule) || matches!(rule, "unresolved-static-import" | "incompatible-assignment")
}

const UNREFERENCED: &[&str] = &["unused-private-field", "unused-private-method", "unused-private-class"];

const DECLARATION: &[&str] = &[
    "public-static-non-final-field",
    "public-static-mutable-container",
    "static-non-thread-safe-field",
    "private-constructor-class-not-final",
    "field-hides-superclass-field",
    "too-many-parameters",
    "utility-class-instantiable",
    "missing-override",
    "compareto-without-equals",
    "equals-without-hashcode",
    "mutable-enum-field",
    "mutable-exception",
    "volatile-array-or-collection",
    "record-array-component",
];

fn oracle_of(rule: &str) -> Option<&'static str> {
    if is_compile_break(rule) {
        Some("compiled")
    } else if UNREFERENCED.contains(&rule) {
        Some("unreferenced")
    } else if DECLARATION.contains(&rule) {
        Some("declaration")
    } else {
        None
    }
}

// ----- descriptors (as in bytecode_oracle.rs) -----

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

/// The erased descriptor of a declared type (JLS 4.6): a type variable is the
/// erasure of its first bound, found in `scope`. None for a type grounds does
/// not know.
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

/// Parameter descriptors of `(..)R`.
fn params_of(d: &str) -> Vec<String> {
    let Some(ps) = d.strip_prefix('(').and_then(|d| d.split_once(')')).map(|x| x.0) else { return Vec::new() };
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
    out
}

/// The parameters a method declares in the source: javac adds the outer
/// instance first to an inner class's constructors, and name and ordinal
/// first to an enum's.
fn source_params(cf: &ClassFile, m: &CfMethod) -> Vec<String> {
    let mut ps = params_of(&m.descriptor);
    if m.name == "<init>" {
        let inner = cf.access & access::INTERFACE == 0 && cf.inner_classes.iter().any(|ic| ic.inner == cf.this_class && ic.outer.is_some() && ic.access & access::STATIC == 0);
        if cf.access & access::ENUM != 0 && ps.len() >= 2 && ps[0] == "Ljava/lang/String;" && ps[1] == "I" {
            ps.drain(..2);
        } else if inner && !ps.is_empty() {
            ps.remove(0);
        }
    }
    ps
}

/// `a/b/Outer` of `a/b/Outer$Inner$Deeper`.
fn top_of(internal: &str) -> &str {
    let start = internal.rfind('/').map_or(0, |i| i + 1);
    match internal[start..].find('$') {
        Some(i) => &internal[..start + i],
        None => internal,
    }
}

// ----- the class files -----

struct Entry {
    cf: Arc<ClassFile>,
    refs: ClassReferences,
}

/// One reference found in a body: in which class and method, on which line.
struct At {
    class: String,
    method: String,
    line: Option<u32>,
}

impl At {
    fn text(&self) -> String {
        let simple = self.class.rsplit('/').next().unwrap_or(&self.class);
        match self.line {
            Some(l) => format!("{simple}.{}:{l}", self.method),
            None => format!("{simple}.{}", self.method),
        }
    }
}

struct Classes {
    /// internal name -> declarations and references
    dir: HashMap<String, Entry>,
    /// top-level internal name -> it and its nested classes
    nests: HashMap<String, Vec<String>>,
    /// source file (`a/b/C.java`) -> the classes compiled from it
    by_source: HashMap<String, Vec<String>>,
    /// the JDK's classes, for supertypes the directory does not have
    jdk: Option<ClassContainer>,
    jdk_cache: Mutex<HashMap<String, Option<Arc<ClassFile>>>>,
    unreadable: usize,
}

impl Classes {
    fn read(dir: &std::path::Path) -> Classes {
        // multi-release copies under META-INF would be second copies
        let names: Vec<String> = ClassContainer::open_dir(dir).class_names().into_iter().filter(|n| !n.starts_with("META-INF/")).collect();
        let parsed: Vec<_> = names
            .par_iter()
            .map(|n| {
                let bytes = std::fs::read(dir.join(format!("{n}.class")));
                let r = bytes.map_err(anyhow::Error::from).and_then(|b| Ok((parse_references(&b)?, parse_class(&b)?)));
                (n.clone(), r)
            })
            .collect();
        let jdk = grounds_classfile::find_java_home().and_then(|h| {
            let release = grounds_classfile::jdk_feature_release(&h)?;
            ClassContainer::open_jdk(&h, release).ok()
        });
        let mut c = Classes { dir: HashMap::new(), nests: HashMap::new(), by_source: HashMap::new(), jdk, jdk_cache: Mutex::new(HashMap::new()), unreadable: 0 };
        for (n, r) in parsed {
            let Ok((refs, cf)) = r else {
                c.unreadable += 1;
                continue;
            };
            if let Some(src) = &cf.source_file {
                let file = match n.rsplit_once('/') {
                    Some((pkg, _)) => format!("{pkg}/{src}"),
                    None => src.clone(),
                };
                c.by_source.entry(file).or_default().push(n.clone());
            }
            c.nests.entry(top_of(&n).to_string()).or_default().push(n.clone());
            c.dir.insert(n, Entry { cf: Arc::new(cf), refs });
        }
        for v in c.by_source.values_mut().chain(c.nests.values_mut()) {
            v.sort();
        }
        c
    }

    /// A class by internal name: from the directory, else from the JDK.
    fn class(&self, name: &str) -> Option<Arc<ClassFile>> {
        if let Some(e) = self.dir.get(name) {
            return Some(e.cf.clone());
        }
        let jdk = self.jdk.as_ref()?;
        let mut cache = self.jdk_cache.lock().unwrap();
        cache.entry(name.to_string()).or_insert_with(|| jdk.read_class(name).map(Arc::new)).clone()
    }

    /// The superclasses of `cf`, nearest first, up to `java/lang/Object`;
    /// Err with the first one neither the directory nor the JDK has.
    fn superclasses(&self, cf: &ClassFile) -> Result<Vec<Arc<ClassFile>>, String> {
        let mut out = Vec::new();
        let mut next = cf.super_class.clone();
        while let Some(s) = next {
            let Some(c) = self.class(&s) else { return Err(s) };
            next = c.super_class.clone();
            out.push(c);
            if out.len() > 64 {
                break;
            }
        }
        Ok(out)
    }

    /// Every supertype of `cf`, classes and interfaces; Err with one that is
    /// not available.
    fn supertypes(&self, cf: &ClassFile) -> Result<Vec<Arc<ClassFile>>, String> {
        let mut out: Vec<Arc<ClassFile>> = Vec::new();
        let mut todo: Vec<String> = cf.super_class.iter().chain(&cf.interfaces).cloned().collect();
        let mut seen = std::collections::HashSet::new();
        while let Some(s) = todo.pop() {
            if !seen.insert(s.clone()) {
                continue;
            }
            let Some(c) = self.class(&s) else { return Err(s) };
            todo.extend(c.super_class.iter().chain(&c.interfaces).cloned());
            out.push(c);
            if out.len() > 256 {
                break;
            }
        }
        Ok(out)
    }

    /// Whether `name` is `target` or a subclass of it (superclasses only).
    fn extends(&self, name: &str, target: &str) -> Result<bool, String> {
        if name == target {
            return Ok(true);
        }
        let Some(c) = self.class(name) else { return Err(name.to_string()) };
        Ok(self.superclasses(&c)?.iter().any(|s| s.this_class == target))
    }

    /// Every reference in the bodies of the classes of `internal`'s nest
    /// (its top-level class and all classes nested in that) that `want`
    /// accepts.
    fn nest_refs(&self, internal: &str, want: impl Fn(&grounds_classfile::Reference) -> bool) -> Vec<At> {
        let mut out = Vec::new();
        for n in self.nests.get(top_of(internal)).into_iter().flatten() {
            let Some(e) = self.dir.get(n) else { continue };
            for m in &e.refs.methods {
                for r in m.refs.iter().filter(|r| want(r)) {
                    out.push(At { class: n.clone(), method: m.name.clone(), line: r.line });
                }
            }
        }
        out
    }
}

// ----- answers -----

#[derive(Clone)]
enum Verdict {
    Confirmed,
    Refuted,
    /// why not, from a short fixed list
    Undecided(&'static str),
}

struct Answer {
    verdict: Verdict,
    /// what the class file shows
    detail: String,
    /// a confirmed unreferenced member the rule's own exemptions should have
    /// covered: an annotation or a serialization name in the class file
    exempt: Option<&'static str>,
}

fn confirmed(detail: String) -> Answer {
    Answer { verdict: Verdict::Confirmed, detail, exempt: None }
}
fn refuted(detail: String) -> Answer {
    Answer { verdict: Verdict::Refuted, detail, exempt: None }
}
fn undecided(why: &'static str) -> Answer {
    Answer { verdict: Verdict::Undecided(why), detail: String::new(), exempt: None }
}

macro_rules! need {
    ($e:expr, $why:expr) => {
        match $e {
            Some(x) => x,
            None => return undecided($why),
        }
    };
}

/// What one file's oracles need: its tree and model, and the class files.
struct Cx<'a, 't> {
    fm: &'a FileModel<'t>,
    pf: &'t ParsedFile,
    cls: &'a Classes,
}

impl<'a, 't> Cx<'a, 't> {
    fn line(&self, n: Node) -> usize {
        self.pf.lines.line_of(n.start_byte() as u32) + 1
    }

    /// The class file of a class grounds declares in this file.
    fn class_file(&self, ci: &ClassInfo) -> Result<(String, Arc<ClassFile>), &'static str> {
        if ci.flags.has(Flags::LOCAL | Flags::ANON) {
            return Err("local or anonymous class");
        }
        let internal = ci.name.replace('.', "/");
        match self.cls.dir.get(&internal) {
            Some(e) => Ok((internal, e.cf.clone())),
            None => Err("no class file for the class"),
        }
    }

    /// The class file method of a method or constructor declaration: the one
    /// whose name and erased parameters are those of the declaration. A
    /// parameter type grounds does not know matches any.
    fn method(&self, ci: &ClassInfo, cf: &'a ClassFile, decl: Node<'t>) -> Result<&'a CfMethod, &'static str>
    where
        't: 'a,
    {
        let name = decl.field("name").ok_or("declaration without a name")?;
        let r = (name.start_byte() as u32, name.end_byte() as u32);
        let m = ci.methods.iter().find(|m| m.range == Some(r)).ok_or("grounds has no method at the declaration")?;
        let scope_owned = class_type_params(self.fm, &ci.name);
        let scope: Vec<&TypeParam> = m.type_params.iter().chain(&scope_owned).collect();
        let want: Vec<Option<String>> = m.params.iter().map(|q| erased(&q.ty, &scope, 0)).collect();
        let jname = if matches!(decl.kind(), "constructor_declaration" | "compact_constructor_declaration") { "<init>" } else { &*m.name };
        let known = |d: &Option<String>| {
            d.as_deref().is_some_and(|d| match d.trim_start_matches('[').strip_prefix('L').and_then(|c| c.strip_suffix(';')) {
                Some(c) => self.fm.class(&c.replace('/', ".")).is_some(),
                None => true,
            })
        };
        for lenient in [false, true] {
            let fits: Vec<&CfMethod> = cf
                .methods
                .iter()
                .filter(|x| x.access & (access::SYNTHETIC | access::BRIDGE) == 0 && x.name == jname)
                .filter(|x| {
                    let ps = source_params(cf, x);
                    ps.len() == want.len() && want.iter().zip(&ps).all(|(w, p)| w.as_deref() == Some(p.as_str()) || (lenient && !known(w)))
                })
                .collect();
            match fits.len() {
                1 => return Ok(fits[0]),
                0 => continue,
                _ => return Err("more than one method of the class file fits the declaration"),
            }
        }
        Err("no method of the class file fits the declaration")
    }

    /// The field declaration a finding is on and the declarators it names:
    /// the one it is on, or all of the declaration's.
    fn declarators(&self, n: Node<'t>) -> Option<(Node<'t>, Vec<Node<'t>>)> {
        let d = std::iter::once(n).chain(self.fm.ancestors_of(n)).take(3).find(|a| a.kind() == "variable_declarator");
        let decl = std::iter::once(n).chain(self.fm.ancestors_of(n)).find(|a| matches!(a.kind(), "field_declaration" | "constant_declaration"))?;
        let ds = match d {
            Some(d) => vec![d],
            None => decl.fields("declarator"),
        };
        Some((decl, ds))
    }

    /// The class declaration a finding on its name is about.
    fn class_decl(&self, n: Node<'t>) -> Option<Node<'t>> {
        std::iter::once(n).chain(self.fm.ancestors_of(n)).find(|a| grounds_syntax::is_type_declaration(a.kind()))
    }
}

/// The field of the class file `cf` a declarator declares.
fn field_of<'c>(cx: &Cx, d: Node, cf: &'c ClassFile) -> Option<&'c grounds_classfile::FieldInfo> {
    let name = cx.fm.text(d.field("name")?);
    cf.fields.iter().find(|f| f.name == name && f.access & access::SYNTHETIC == 0)
}

fn flags_text(a: u16) -> String {
    let mut v = Vec::new();
    for (bit, s) in [
        (access::PUBLIC, "public"),
        (access::PRIVATE, "private"),
        (access::PROTECTED, "protected"),
        (access::STATIC, "static"),
        (access::FINAL, "final"),
        (access::VOLATILE, "volatile"),
        (access::ABSTRACT, "abstract"),
    ] {
        if a & bit != 0 {
            v.push(s);
        }
    }
    if v.is_empty() {
        "package-private".to_string()
    } else {
        v.join(" ")
    }
}

const SERIAL_FIELDS: &[&str] = &["serialVersionUID", "serialPersistentFields"];
const SERIAL_METHODS: &[&str] = &["writeObject", "readObject", "readObjectNoData", "readResolve", "writeReplace"];

// ----- compiled -----

fn compiled(cx: &Cx, rel: Option<&str>) -> Answer {
    let rel = need!(rel, "file not under src/main/java");
    match cx.cls.by_source.get(rel) {
        Some(names) => refuted(format!("{} class file(s) compiled from this file ({})", names.len(), names[0])),
        None => undecided("no class file compiled from this file"),
    }
}

// ----- unreferenced -----

fn unused_field<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let (decl, ds) = need!(cx.declarators(n), "no field declaration at the finding");
    let d = need!(ds.first().copied(), "no declarator");
    let ci = need!(cx.fm.enclosing_class(decl), "class unknown to grounds");
    let (internal, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let f = need!(field_of(cx, d, &cf), "no field of that name in the class file");
    // a constant variable's reads are inlined: no get names it
    if f.constant.is_some() {
        return undecided("a constant variable: javac inlines its reads");
    }
    let name = f.name.as_str();
    let ours = |r: &grounds_classfile::Reference| r.owner == internal && r.name == name;
    let reads = cx.cls.nest_refs(&internal, |r| ours(r) && matches!(r.kind, RefKind::GetField | RefKind::GetStatic | RefKind::Handle(1 | 2)));
    let writes = cx.cls.nest_refs(&internal, |r| ours(r) && matches!(r.kind, RefKind::PutField | RefKind::PutStatic | RefKind::Handle(3 | 4)));
    if reads.is_empty() {
        let nest = cx.cls.nests.get(top_of(&internal)).map_or(0, |v| v.len());
        let mut a = confirmed(format!("no get of `{name}` in the {nest} class file(s) of its nest; {} put(s)", writes.len()));
        // default serialization reads and writes every non-static,
        // non-transient field of a Serializable class by reflection
        let serialized = f.access & (access::STATIC | access::TRANSIENT) == 0 && cx.cls.supertypes(&cf).is_ok_and(|s| s.iter().any(|c| c.this_class == "java/io/Serializable"));
        a.exempt = if !f.annotations.is_empty() {
            Some("annotated")
        } else if SERIAL_FIELDS.contains(&name) {
            Some("serialization name")
        } else if serialized {
            Some("non-transient field of a Serializable class")
        } else {
            None
        };
        return a;
    }
    // `x++;` and `x += 1;` read and write on one line; the rule counts them
    // as writes, and the class file cannot tell them from `y = x++`
    if reads.iter().all(|r| writes.iter().any(|w| w.class == r.class && w.method == r.method && w.line == r.line)) {
        return undecided("read only where it is also written on the same line (x++, x += ..)");
    }
    refuted(format!("`{name}` is read at {} ({} get(s))", reads[0].text(), reads.len()))
}

fn unused_method<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let decl = need!(std::iter::once(n).chain(cx.fm.ancestors_of(n)).find(|a| a.kind() == "method_declaration"), "no method declaration at the finding");
    let ci = need!(cx.fm.enclosing_class(decl), "class unknown to grounds");
    let (internal, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let m = match cx.method(&ci, &cf, decl) {
        Ok(m) => m,
        Err(why) => return undecided(why),
    };
    // any call counts for the rule, a recursive one too
    let calls = cx.cls.nest_refs(&internal, |r| {
        r.owner == internal
            && r.name == m.name
            && r.descriptor == m.descriptor
            && matches!(r.kind, RefKind::InvokeVirtual | RefKind::InvokeSpecial | RefKind::InvokeStatic | RefKind::InvokeInterface | RefKind::Handle(5..=9))
    });
    if calls.is_empty() {
        let nest = cx.cls.nests.get(top_of(&internal)).map_or(0, |v| v.len());
        let mut a = confirmed(format!("no invoke or method handle of `{}{}` in the {nest} class file(s) of its nest", m.name, m.descriptor));
        a.exempt = if !m.annotations.is_empty() {
            Some("annotated")
        } else if SERIAL_METHODS.contains(&m.name.as_str()) {
            Some("serialization name")
        } else {
            None
        };
        return a;
    }
    refuted(format!("`{}{}` is called at {} ({} reference(s))", m.name, m.descriptor, calls[0].text(), calls.len()))
}

/// A private nested class is used where another class of its nest extends
/// it, names it as an owner, in a descriptor or a signature, or lists it among
/// its inner classes (javac lists there every nested class its constant pool
/// names; the class's own outer class lists it in any case). Casts,
/// `instanceof` and class literals are not read, so with none of these the
/// finding is not decided.
fn unused_class<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let decl = need!(cx.class_decl(n), "no class declaration at the finding");
    let ci = need!(cx.fm.class_of_decl(decl), "class unknown to grounds");
    let (internal, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let outer = cf.inner_classes.iter().find(|ic| ic.inner == internal).and_then(|ic| ic.outer.clone());
    let mention = format!("L{internal};");
    let own = |c: &str| c == internal || c.starts_with(&format!("{internal}$"));
    for name in cx.cls.nests.get(top_of(&internal)).into_iter().flatten().filter(|c| !own(c)) {
        let Some(e) = cx.cls.dir.get(name) else { continue };
        let o = &e.cf;
        let simple = name.rsplit('/').next().unwrap_or(name);
        if o.super_class.as_deref() == Some(&internal) || o.interfaces.iter().any(|i| *i == internal) {
            return refuted(format!("{simple} extends it"));
        }
        let decls = o.fields.iter().map(|f| (&f.descriptor, &f.signature)).chain(o.methods.iter().map(|m| (&m.descriptor, &m.signature)));
        for (d, s) in decls {
            if d.contains(&mention) || s.as_deref().is_some_and(|s| s.contains(&format!("L{internal}"))) {
                return refuted(format!("a declaration of {simple} names it"));
            }
        }
        for m in &e.refs.methods {
            if let Some(r) = m.refs.iter().find(|r| r.owner == internal || r.descriptor.contains(&mention)) {
                return refuted(format!("{simple}.{} refers to it ({} {})", m.name, r.name, r.descriptor));
            }
        }
        if Some(name) != outer.as_ref() && o.inner_classes.iter().any(|ic| ic.inner == internal) {
            return refuted(format!("{simple} lists it among the classes its constant pool names"));
        }
    }
    undecided("no reference read, but casts, instanceof and class literals are not read")
}

// ----- declaration -----

fn public_static_non_final<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let (decl, ds) = need!(cx.declarators(n), "no field declaration at the finding");
    let ci = need!(cx.fm.enclosing_class(decl), "class unknown to grounds");
    let (_, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let f = need!(ds.first().and_then(|d| field_of(cx, *d, &cf)), "no field of that name in the class file");
    let a = f.access;
    if cf.access & access::INTERFACE == 0 && a & access::PUBLIC != 0 && a & access::STATIC != 0 && a & access::FINAL == 0 {
        confirmed(format!("`{}` is {}", f.name, flags_text(a)))
    } else {
        refuted(format!("`{}` is {}{}", f.name, flags_text(a), if cf.access & access::INTERFACE != 0 { " in an interface" } else { "" }))
    }
}

const MUTABLE_CONTAINERS: &[&str] = &[
    "java/util/ArrayList",
    "java/util/HashMap",
    "java/util/HashSet",
    "java/util/LinkedList",
    "java/util/TreeMap",
    "java/util/TreeSet",
    "java/util/LinkedHashMap",
    "java/util/LinkedHashSet",
    "java/util/Date",
];

/// The rule's condition: public and static (or in an interface), and an
/// array or a value created as one of the mutable containers. What the
/// initializer creates is the last call before the `putstatic` of the field
/// on the declarator's lines in `<clinit>`: a constructor of that class, or a
/// method returning it. A zero-length array (which the rule exempts) is not
/// told from another: its length is an operand, not a reference.
fn public_static_mutable_container<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let (decl, ds) = need!(cx.declarators(n), "no field declaration at the finding");
    let d = need!(ds.first().copied(), "no declarator");
    let ci = need!(cx.fm.enclosing_class(decl), "class unknown to grounds");
    let (internal, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let f = need!(field_of(cx, d, &cf), "no field of that name in the class file");
    let iface = cf.access & access::INTERFACE != 0;
    if !iface && f.access & (access::PUBLIC | access::STATIC) != access::PUBLIC | access::STATIC {
        return refuted(format!("`{}` is {}", f.name, flags_text(f.access)));
    }
    if f.descriptor.starts_with('[') {
        return confirmed(format!("`{}` is {} {}", f.name, flags_text(f.access), f.descriptor));
    }
    let (lo, hi) = (cx.line(d) as u32, cx.pf.lines.line_of(d.end_byte().saturating_sub(1) as u32) as u32 + 1);
    let e = need!(cx.cls.dir.get(&internal), "no class file for the class");
    for m in e.refs.methods.iter().filter(|m| m.name == "<clinit>") {
        let Some(i) = m.refs.iter().position(|r| r.kind == RefKind::PutStatic && r.owner == internal && r.name == f.name && r.line.is_some_and(|l| (lo..=hi).contains(&l))) else { continue };
        let Some(prev) = m.refs[..i].iter().rev().find(|r| !r.kind.is_field()) else {
            return refuted(format!("`{}` ({}) is stored with no call before it", f.name, f.descriptor));
        };
        let made = if prev.name == "<init>" { prev.owner.clone() } else { prev.descriptor.rsplit(')').next().unwrap_or("").trim_start_matches('L').trim_end_matches(';').to_string() };
        return if MUTABLE_CONTAINERS.contains(&made.as_str()) {
            confirmed(format!("`{}` holds a new {made} ({} {}.{})", f.name, flags_text(f.access), prev.owner, prev.name))
        } else {
            refuted(format!("`{}` holds what {}.{}{} returns", f.name, prev.owner, prev.name, prev.descriptor))
        };
    }
    undecided("no putstatic of the field on the declarator's lines")
}

const NOT_THREAD_SAFE: &[&str] = &["java/text/DateFormat", "java/util/Calendar", "java/text/MessageFormat", "java/text/NumberFormat"];

/// Static and of a subclass of one of the formatters. Whether every use is
/// synchronized (an exemption of the rule) is not read.
fn static_non_thread_safe<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let (decl, ds) = need!(cx.declarators(n), "no field declaration at the finding");
    let ci = need!(cx.fm.enclosing_class(decl), "class unknown to grounds");
    let (_, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let scalar: Vec<Node> = ds.into_iter().filter(|d| d.field("dimensions").is_none()).collect();
    let f = need!(scalar.first().and_then(|d| field_of(cx, *d, &cf)), "no field of that name in the class file");
    if f.access & access::STATIC == 0 {
        return refuted(format!("`{}` is {}", f.name, flags_text(f.access)));
    }
    let Some(ty) = f.descriptor.strip_prefix('L').and_then(|d| d.strip_suffix(';')) else { return refuted(format!("`{}` is {}", f.name, f.descriptor)) };
    for b in NOT_THREAD_SAFE {
        match cx.cls.extends(ty, b) {
            Ok(true) => return confirmed(format!("`{}` is {} {ty}, a {b}", f.name, flags_text(f.access))),
            Ok(false) => {}
            Err(_) => return undecided("a supertype is in neither the classes nor the JDK"),
        }
    }
    refuted(format!("`{}` is a {ty}, not a formatter", f.name))
}

/// Not final, not abstract, no class of the same file extends it, and, as
/// the rule has it, either every constructor private (javac's synthetic
/// access constructors aside) or, for a class that declares none, the class
/// itself private: then no class outside its nest can extend it whatever
/// constructor it gets (Lombok's are public).
fn private_constructor_not_final<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let decl = need!(cx.class_decl(n), "no class declaration at the finding");
    let ci = need!(cx.fm.class_of_decl(decl), "class unknown to grounds");
    let (internal, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let acc = cf.effective_access();
    if acc & (access::FINAL | access::ABSTRACT | access::INTERFACE) != 0 {
        return refuted(format!("the class is {}", flags_text(acc)));
    }
    let ctors: Vec<&CfMethod> = cf.methods.iter().filter(|m| m.name == "<init>" && m.access & access::SYNTHETIC == 0).collect();
    let declares_ctor = decl.field("body").is_some_and(|b| b.kids().iter().any(|k| k.kind() == "constructor_declaration"));
    let nested = cf.inner_classes.iter().any(|ic| ic.inner == internal && ic.outer.is_some());
    if !declares_ctor && nested && acc & access::PRIVATE != 0 {
        // (what the rule calls a private class never extended)
    } else if let Some(c) = ctors.iter().find(|c| c.access & access::PRIVATE == 0) {
        return refuted(format!("constructor {} is {}", c.descriptor, flags_text(c.access)));
    }
    if !cf.permitted_subclasses.is_empty() {
        return refuted("the class is sealed".to_string());
    }
    let src = cf.source_file.as_deref().unwrap_or("");
    let same_file = |o: &ClassFile| o.source_file.as_deref() == Some(src) && top_of(&o.this_class).rsplit_once('/').map(|x| x.0) == internal.rsplit_once('/').map(|x| x.0);
    if let Some((name, _)) = cx.cls.dir.iter().find(|(_, e)| e.cf.super_class.as_deref() == Some(internal.as_str()) && same_file(&e.cf)) {
        return refuted(format!("{name} extends it"));
    }
    confirmed(format!("{} class, constructors {}, not extended", flags_text(acc), ctors.iter().map(|c| flags_text(c.access)).collect::<Vec<_>>().join(", ")))
}

/// An instance field whose name the nearest superclass declaring that name
/// declares non-private and non-static.
fn field_hides<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let (decl, ds) = need!(cx.declarators(n), "no field declaration at the finding");
    let ci = need!(cx.fm.enclosing_class(decl), "class unknown to grounds");
    let (_, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let f = need!(ds.first().and_then(|d| field_of(cx, *d, &cf)), "no field of that name in the class file");
    if f.access & access::STATIC != 0 {
        return refuted(format!("`{}` is static", f.name));
    }
    let mut next = cf.super_class.clone();
    while let Some(s) = next {
        let Some(c) = cx.cls.class(&s) else { return undecided("a superclass is in neither the classes nor the JDK") };
        if let Some(sf) = c.fields.iter().find(|x| x.name == f.name) {
            return if sf.access & (access::PRIVATE | access::STATIC) == 0 {
                confirmed(format!("{s} declares `{}` {}", sf.name, flags_text(sf.access)))
            } else {
                refuted(format!("the nearest declaration of `{}`, in {s}, is {}", sf.name, flags_text(sf.access)))
            };
        }
        next = c.super_class.clone();
    }
    refuted(format!("no superclass declares `{}`", f.name))
}

/// More parameters than the maximum (the number in the message): decided by
/// the methods of that name alone when they all agree, else by the one whose
/// erased parameters are the declaration's.
fn too_many_parameters<'t>(cx: &Cx<'_, 't>, n: Node<'t>, message: &str) -> Answer {
    let max: usize = need!(message.split("(max ").nth(1).and_then(|s| s.split(')').next()).and_then(|s| s.parse().ok()), "no maximum in the message");
    let decl = need!(std::iter::once(n).chain(cx.fm.ancestors_of(n)).find(|a| matches!(a.kind(), "method_declaration" | "constructor_declaration")), "no declaration at the finding");
    let ci = need!(cx.fm.enclosing_class(decl), "class unknown to grounds");
    let (_, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let jname = if decl.kind() == "constructor_declaration" { "<init>".to_string() } else { cx.fm.text(need!(decl.field("name"), "no name")).to_string() };
    let counts: Vec<usize> = cf.methods.iter().filter(|m| m.name == jname && m.access & (access::SYNTHETIC | access::BRIDGE) == 0).map(|m| source_params(&cf, m).len()).collect();
    if counts.is_empty() {
        return undecided("no method of that name in the class file");
    }
    if counts.iter().all(|&c| c > max) {
        return confirmed(format!("every `{jname}` of the class file takes more than {max} ({counts:?})"));
    }
    if counts.iter().all(|&c| c <= max) {
        return refuted(format!("no `{jname}` of the class file takes more than {max} ({counts:?})"));
    }
    match cx.method(&ci, &cf, decl) {
        Ok(m) => {
            let k = source_params(&cf, m).len();
            if k > max {
                confirmed(format!("`{jname}{}` takes {k}", m.descriptor))
            } else {
                refuted(format!("`{jname}{}` takes {k}", m.descriptor))
            }
        }
        Err(why) => undecided(why),
    }
}

/// A class of static members only, with a constructor that is not private:
/// not abstract, extending Object, implementing nothing, no instance method
/// or field, at least one static member, no `main`. The message says the
/// implicit constructor is public; one of a package-private class is
/// package-private, and the rule's point, that code outside the class can
/// instantiate it, holds for that too. A private class's is private (JLS
/// 8.8.9): then the claim is false.
fn utility_class<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let decl = need!(cx.class_decl(n), "no class declaration at the finding");
    let ci = need!(cx.fm.class_of_decl(decl), "class unknown to grounds");
    let (_, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let acc = cf.effective_access();
    if acc & (access::ABSTRACT | access::INTERFACE | access::ENUM) != 0 || cf.access & access::ENUM != 0 {
        return refuted(format!("the class is {}", flags_text(acc)));
    }
    if cf.super_class.as_deref() != Some("java/lang/Object") || !cf.interfaces.is_empty() {
        return refuted(format!("the class extends {} and implements {}", cf.super_class.as_deref().unwrap_or("?"), cf.interfaces.len()));
    }
    let own = |a: u16| a & access::SYNTHETIC == 0;
    if let Some(m) = cf.methods.iter().find(|m| own(m.access) && m.access & access::STATIC == 0 && m.name != "<init>") {
        return refuted(format!("instance method {}{}", m.name, m.descriptor));
    }
    if let Some(f) = cf.fields.iter().find(|f| own(f.access) && f.access & access::STATIC == 0) {
        return refuted(format!("instance field {}", f.name));
    }
    if cf.methods.iter().any(|m| m.name == "main" && m.descriptor == "([Ljava/lang/String;)V" && m.access & access::STATIC != 0) {
        return refuted("it has a main method".to_string());
    }
    let statics = cf.methods.iter().filter(|m| own(m.access) && m.access & access::STATIC != 0 && m.name != "<clinit>").count() + cf.fields.iter().filter(|f| own(f.access)).count();
    if statics == 0 {
        return refuted("no static member".to_string());
    }
    let ctors: Vec<&CfMethod> = cf.methods.iter().filter(|m| m.name == "<init>" && own(m.access)).collect();
    match ctors.iter().find(|c| c.access & access::PRIVATE == 0) {
        Some(c) => confirmed(format!("{statics} static member(s), constructor {} {}", flags_text(c.access), c.descriptor)),
        None => refuted("every constructor is private".to_string()),
    }
}

/// Whether a supertype declares a method the declaration overrides: one of
/// that name, not static or private, with the same erased parameters, or a
/// bridge javac generated for it in the class. With only a method of that
/// name and arity whose erasure differs (a generic method an interface
/// overrides, which javac bridges only in classes) it is not decided.
fn missing_override<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let decl = need!(std::iter::once(n).chain(cx.fm.ancestors_of(n)).find(|a| a.kind() == "method_declaration"), "no method declaration at the finding");
    let ci = need!(cx.fm.enclosing_class(decl), "class unknown to grounds");
    let (_, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let m = match cx.method(&ci, &cf, decl) {
        Ok(m) => m,
        Err(why) => return undecided(why),
    };
    let ps = params_of(&m.descriptor);
    if cf.methods.iter().any(|b| b.access & access::BRIDGE != 0 && b.name == m.name && params_of(&b.descriptor).len() == ps.len()) {
        return confirmed(format!("javac bridges `{}` in the class: it overrides a generic method", m.name));
    }
    let supers = match cx.cls.supertypes(&cf) {
        Ok(s) => s,
        Err(_) => return undecided("a supertype is in neither the classes nor the JDK"),
    };
    let inheritable = |x: &CfMethod| x.name == m.name && x.access & (access::STATIC | access::PRIVATE) == 0;
    // (an interface inherits Object's public methods, JLS 9.2)
    let object = if cf.access & access::INTERFACE != 0 { cx.cls.class("java/lang/Object") } else { None };
    let all: Vec<&Arc<ClassFile>> = supers.iter().chain(object.iter()).collect();
    if let Some(s) = all.iter().find(|s| s.methods.iter().any(|x| inheritable(x) && params_of(&x.descriptor) == ps)) {
        return confirmed(format!("{} declares `{}{}`", s.this_class, m.name, m.descriptor));
    }
    if all.iter().any(|s| s.methods.iter().any(|x| inheritable(x) && params_of(&x.descriptor).len() == ps.len())) {
        return undecided("a supertype method of that name and arity with another erasure");
    }
    refuted(format!("no supertype declares `{}` with {} parameter(s)", m.name, ps.len()))
}

/// Whether a superclass other than Object declares a non-abstract method of
/// that name and descriptor (`overrides_object_method`). Err when one is not
/// available.
fn superclass_declares(cx: &Cx, cf: &ClassFile, name: &str, descriptor: &str) -> Result<Option<String>, ()> {
    let supers = cx.cls.superclasses(cf).map_err(|_| ())?;
    Ok(supers
        .iter()
        .filter(|s| s.this_class != "java/lang/Object")
        .find(|s| s.methods.iter().any(|m| m.name == name && m.descriptor == descriptor && m.access & access::ABSTRACT == 0))
        .map(|s| s.this_class.clone()))
}

fn compareto_without_equals<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let ci = need!(std::iter::once(n).chain(cx.fm.ancestors_of(n)).filter(|a| grounds_syntax::is_type_declaration(a.kind())).find_map(|d| cx.fm.class_of_decl(d)), "class unknown to grounds");
    let (_, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    if cf.access & access::ABSTRACT != 0 {
        return refuted("the class is abstract".to_string());
    }
    let own = |m: &&CfMethod| m.access & (access::SYNTHETIC | access::BRIDGE) == 0;
    if !cf.methods.iter().filter(own).any(|m| m.name == "compareTo" && params_of(&m.descriptor).len() == 1) {
        return refuted("the class declares no compareTo".to_string());
    }
    if cf.methods.iter().any(|m| m.name == "equals" && m.descriptor == "(Ljava/lang/Object;)Z" && m.access & access::ABSTRACT == 0) {
        return refuted("the class declares equals(Object)".to_string());
    }
    match superclass_declares(cx, &cf, "equals", "(Ljava/lang/Object;)Z") {
        Err(()) => undecided("a superclass is in neither the classes nor the JDK"),
        Ok(Some(s)) => refuted(format!("{s} declares equals(Object)")),
        Ok(None) => confirmed("compareTo declared, equals(Object) declared by no class but Object".to_string()),
    }
}

/// The class declares one of `equals(Object)` and `hashCode()` and neither
/// it nor a superclass other than Object declares the other.
fn equals_without_hashcode<'t>(cx: &Cx<'_, 't>, n: Node<'t>, message: &str) -> Answer {
    let ci = need!(cx.fm.enclosing_class(n), "class unknown to grounds");
    let (_, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let (has, missing) =
        if message.contains("overrides `equals()` but not") { (("equals", "(Ljava/lang/Object;)Z"), ("hashCode", "()I")) } else { (("hashCode", "()I"), ("equals", "(Ljava/lang/Object;)Z")) };
    let declares = |(name, d): (&str, &str)| cf.methods.iter().any(|m| m.name == name && m.descriptor == d && m.access & access::STATIC == 0);
    if !declares(has) {
        return refuted(format!("the class declares no {}{}", has.0, has.1));
    }
    if declares(missing) {
        return refuted(format!("the class declares {}{}", missing.0, missing.1));
    }
    match superclass_declares(cx, &cf, missing.0, missing.1) {
        Err(()) => undecided("a superclass is in neither the classes nor the JDK"),
        Ok(Some(s)) => refuted(format!("{s} declares {}{}", missing.0, missing.1)),
        Ok(None) => confirmed(format!("{}{} declared, {}{} by no class but Object", has.0, has.1, missing.0, missing.1)),
    }
}

/// A non-static, non-final field that can change after construction: not
/// private, or written by a method other than a constructor in its nest.
/// `throwable`: the class must extend Throwable (mutable-exception), else be
/// an enum (mutable-enum-field).
fn mutable_field<'t>(cx: &Cx<'_, 't>, n: Node<'t>, throwable: bool) -> Answer {
    let (decl, ds) = need!(cx.declarators(n), "no field declaration at the finding");
    let ci = need!(cx.fm.enclosing_class(decl), "class unknown to grounds");
    let (internal, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let f = need!(ds.first().and_then(|d| field_of(cx, *d, &cf)), "no field of that name in the class file");
    if throwable {
        match cx.cls.extends(&internal, "java/lang/Throwable") {
            Ok(true) => {}
            Ok(false) => return refuted("the class does not extend Throwable".to_string()),
            Err(_) => return undecided("a superclass is in neither the classes nor the JDK"),
        }
    } else if cf.access & access::ENUM == 0 {
        return refuted("the class is not an enum".to_string());
    }
    if f.access & (access::STATIC | access::FINAL) != 0 {
        return refuted(format!("`{}` is {}", f.name, flags_text(f.access)));
    }
    if f.access & access::PRIVATE == 0 {
        return confirmed(format!("`{}` is {}, open to writes", f.name, flags_text(f.access)));
    }
    let writes = cx.cls.nest_refs(&internal, |r| r.owner == internal && r.name == f.name && matches!(r.kind, RefKind::PutField | RefKind::Handle(3)));
    match writes.iter().find(|w| w.method != "<init>") {
        Some(w) => confirmed(format!("`{}` is written at {}", f.name, w.text())),
        None => refuted(format!("`{}` is private and written only by constructors ({} put(s))", f.name, writes.len())),
    }
}

const VOLATILE_CONTAINERS: &[&str] = &["Ljava/util/ArrayList;", "Ljava/util/HashMap;", "Ljava/util/HashSet;", "Ljava/util/LinkedList;", "Ljava/util/List;", "Ljava/util/Map;", "Ljava/util/Set;"];

fn volatile_container<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let (decl, ds) = need!(cx.declarators(n), "no field declaration at the finding");
    let ci = need!(cx.fm.enclosing_class(decl), "class unknown to grounds");
    let (_, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let f = need!(ds.first().and_then(|d| field_of(cx, *d, &cf)), "no field of that name in the class file");
    if f.access & access::VOLATILE != 0 && (f.descriptor.starts_with('[') || VOLATILE_CONTAINERS.contains(&f.descriptor.as_str())) {
        confirmed(format!("`{}` is {} {}", f.name, flags_text(f.access), f.descriptor))
    } else {
        refuted(format!("`{}` is {} {}", f.name, flags_text(f.access), f.descriptor))
    }
}

/// A record with an array component whose `equals` is the one javac
/// generates: that one is an `invokedynamic` of `ObjectMethods` whose
/// bootstrap arguments are the components' getField handles.
fn record_array_component<'t>(cx: &Cx<'_, 't>, n: Node<'t>) -> Answer {
    let decl = need!(cx.class_decl(n), "no class declaration at the finding");
    let ci = need!(cx.fm.class_of_decl(decl), "class unknown to grounds");
    let (internal, cf) = match cx.class_file(&ci) {
        Ok(x) => x,
        Err(why) => return undecided(why),
    };
    let Some(c) = cf.record_components.iter().find(|c| c.1.starts_with('[')) else { return refuted("no array component".to_string()) };
    let e = need!(cx.cls.dir.get(&internal), "no class file for the class");
    let eq = need!(e.refs.methods.iter().find(|m| m.name == "equals" && m.descriptor == "(Ljava/lang/Object;)Z"), "no equals(Object) in the class file");
    if eq.refs.iter().any(|r| r.kind == RefKind::Handle(1) && r.owner == internal) {
        confirmed(format!("component `{}` {}; equals is javac's ObjectMethods", c.0, c.1))
    } else {
        refuted(format!("component `{}` {}; equals is written in the class", c.0, c.1))
    }
}

fn answer<'t>(cx: &Cx<'_, 't>, f: &Finding, rel: Option<&str>) -> Answer {
    if is_compile_break(f.rule) {
        return compiled(cx, rel);
    }
    let n = need!(cx.pf.root().named_descendant_for_byte_range(f.start as usize, f.end as usize), "no node at the finding");
    match f.rule {
        "unused-private-field" => unused_field(cx, n),
        "unused-private-method" => unused_method(cx, n),
        "unused-private-class" => unused_class(cx, n),
        "public-static-non-final-field" => public_static_non_final(cx, n),
        "public-static-mutable-container" => public_static_mutable_container(cx, n),
        "static-non-thread-safe-field" => static_non_thread_safe(cx, n),
        "private-constructor-class-not-final" => private_constructor_not_final(cx, n),
        "field-hides-superclass-field" => field_hides(cx, n),
        "too-many-parameters" => too_many_parameters(cx, n, &f.message),
        "utility-class-instantiable" => utility_class(cx, n),
        "missing-override" => missing_override(cx, n),
        "compareto-without-equals" => compareto_without_equals(cx, n),
        "equals-without-hashcode" => equals_without_hashcode(cx, n, &f.message),
        "mutable-enum-field" => mutable_field(cx, n, false),
        "mutable-exception" => mutable_field(cx, n, true),
        "volatile-array-or-collection" => volatile_container(cx, n),
        "record-array-component" => record_array_component(cx, n),
        _ => undecided("no oracle"),
    }
}

#[derive(Default)]
struct RuleCount {
    findings: usize,
    confirmed: usize,
    refuted: usize,
    undecided: BTreeMap<&'static str, usize>,
    exempt: BTreeMap<&'static str, usize>,
}

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    let json = args.iter().any(|a| a == "--json");
    args.retain(|a| a != "--json");
    anyhow::ensure!(args.len() == 3, "usage: finding_oracle [--json] <project root> <classes directory>");
    let root = std::path::PathBuf::from(&args[1]).canonicalize()?;
    let t = std::time::Instant::now();
    let classes = Classes::read(&std::path::PathBuf::from(&args[2]).canonicalize()?);
    let t_classes = t.elapsed();

    // as `grounds check` runs: the configured rules, every source file
    let t = std::time::Instant::now();
    let project = grounds_engine::Project::load(&root)?;
    let (ruleset, options) = analyze::build_ruleset(&project);
    let files = analyze::expand_paths(&project, &[]);
    let findings = analyze::analyze_files(&project, &ruleset, &options, &files);
    let t_check = t.elapsed();

    let t = std::time::Instant::now();
    let mut by_file: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, f) in findings.iter().enumerate() {
        if oracle_of(f.rule).is_some() {
            by_file.entry(f.path.as_str()).or_default().push(i);
        }
    }
    let src_root = project.workspace.root.join("src/main/java");
    let answers: Vec<(usize, Answer)> = by_file
        .par_iter()
        .flat_map_iter(|(path, idx)| {
            let abs = project.workspace.root.join(path);
            let rel = abs.strip_prefix(&src_root).ok().map(|r| r.to_string_lossy().to_string());
            let text = std::fs::read_to_string(&abs).unwrap_or_default();
            let pf = ParsedFile::parse(text);
            let fm = FileModel::new(&pf, &project.index, grounds_engine::project::path_key(&abs));
            let cx = Cx { fm: &fm, pf: &pf, cls: &classes };
            idx.iter().map(|&i| (i, answer(&cx, &findings[i], rel.as_deref()))).collect::<Vec<_>>()
        })
        .collect();
    let t_oracle = t.elapsed();

    let mut per: BTreeMap<&str, RuleCount> = BTreeMap::new();
    for f in &findings {
        per.entry(f.rule).or_default().findings += 1;
    }
    let mut decided: Vec<(usize, &Answer)> = Vec::new();
    for (i, a) in &answers {
        let c = per.entry(findings[*i].rule).or_default();
        match &a.verdict {
            Verdict::Confirmed => {
                c.confirmed += 1;
                if let Some(e) = a.exempt {
                    *c.exempt.entry(e).or_default() += 1;
                }
            }
            Verdict::Refuted => c.refuted += 1,
            Verdict::Undecided(why) => *c.undecided.entry(why).or_default() += 1,
        }
        if !matches!(a.verdict, Verdict::Undecided(_)) {
            decided.push((*i, a));
        }
    }
    decided.sort_by_key(|(i, _)| (&findings[*i].path, findings[*i].line, findings[*i].rule));
    let verdict_name = |v: &Verdict| match v {
        Verdict::Confirmed => "confirmed",
        Verdict::Refuted => "refuted",
        Verdict::Undecided(_) => "undecided",
    };

    if json {
        let rules: serde_json::Map<String, serde_json::Value> = per
            .iter()
            .map(|(r, c)| {
                let v = serde_json::json!({
                    "oracle": oracle_of(r),
                    "findings": c.findings,
                    "confirmed": c.confirmed,
                    "refuted": c.refuted,
                    "undecided": c.undecided,
                    "exempt": c.exempt,
                });
                (r.to_string(), v)
            })
            .collect();
        let answers: Vec<serde_json::Value> = decided
            .iter()
            .map(|(i, a)| {
                let f = &findings[*i];
                serde_json::json!({"rule": f.rule, "file": f.path, "line": f.line, "verdict": verdict_name(&a.verdict), "detail": a.detail, "exempt": a.exempt, "message": f.message})
            })
            .collect();
        let out = serde_json::json!({
            "files": files.len(),
            "class_files": classes.dir.len(),
            "unreadable_class_files": classes.unreadable,
            "jdk": classes.jdk.is_some(),
            "findings": findings.len(),
            "rules": rules,
            "answers": answers,
            "seconds": {"classes": t_classes.as_secs_f64(), "check": t_check.as_secs_f64(), "oracle": t_oracle.as_secs_f64()},
        });
        println!("{out}");
        return Ok(());
    }
    println!("files {}  class files {}  findings {}  (classes {:?}, check {:?}, oracles {:?})", files.len(), classes.dir.len(), findings.len(), t_classes, t_check, t_oracle);
    for (r, c) in per.iter().filter(|(r, _)| oracle_of(r).is_some()) {
        let why: Vec<String> = c.undecided.iter().map(|(k, v)| format!("{k} {v}")).collect();
        let n: usize = c.undecided.values().sum();
        println!("{r} [{}]: findings {}  confirmed {}  refuted {}  not decidable {n}  ({})", oracle_of(r).unwrap_or(""), c.findings, c.confirmed, c.refuted, why.join(", "));
    }
    for (i, a) in &decided {
        if matches!(a.verdict, Verdict::Refuted) {
            let f = &findings[*i];
            println!("REFUTED  {}:{} {}: {}", f.path, f.line, f.rule, a.detail);
        }
    }
    Ok(())
}
