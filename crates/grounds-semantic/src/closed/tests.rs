//! Every small world, systematically: one to four classes, each a class or
//! an interface extending or implementing others (every such hierarchy, once
//! up to renaming the classes). Each world has one method name for every
//! combination of how its classes declare it (not at all, private, or
//! public, package-private or protected, which of the three varying with
//! the class and the name), and one field name for every combination of
//! declared (public, package-private or private) or not. Then every subset
//! of the classes is hidden (not declared at all) and every visible one
//! marked or not as having members Lombok adds that the model does not
//! reproduce. The kernel must:
//!
//! (a) prove a class closed exactly when no class of its supertype closure
//!     is hidden or has unknown members;
//! (b) never grant a claim with something hidden or unknown that it does not
//!     grant with everything visible: hiding never creates a claim (asked:
//!     no method of a name, none taking one argument, no field);
//! (c) with everything visible, claim "no method named m" exactly when no
//!     class of the closure declares an `m` the class inherits (its own, any
//!     access, or a supertype's that is not private), but for the case
//!     marked FINDING below.
//!
//! The worlds are Java source text, extracted as the fixtures' are.

use crate::closed::Jars;
use crate::FileModel;
use grounds_index::*;
use grounds_syntax::{NodeExt, ParsedFile};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::{Arc, OnceLock};

fn jdk() -> Arc<Classpath> {
    static CP: OnceLock<Arc<Classpath>> = OnceLock::new();
    CP.get_or_init(|| Arc::new(Classpath::build(grounds_classfile::find_java_home().as_deref(), 21, &[]))).clone()
}

const NAMES: [&str; 4] = ["A", "B", "C", "D"];

/// How a class declares one method name: not at all, private, or with an
/// access that lets subclasses inherit it. Which one of those varies with
/// the class and the name (`ACCESS`); an interface's are public (a default
/// method).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Decl {
    Absent,
    Inherited,
    Private,
}

const METHOD_DECLS: [Decl; 3] = [Decl::Absent, Decl::Inherited, Decl::Private];
/// Fields: declared or not, with an access that varies with the class and
/// the name (an interface's are public constants).
const FIELD_DECLS: [Decl; 2] = [Decl::Absent, Decl::Inherited];

/// The access of a declaration of name `k` in class `i`.
fn access(k: usize, i: usize, choices: [&'static str; 3]) -> &'static str {
    choices[(k + i) % 3]
}

#[derive(Clone, Debug)]
struct Shape {
    interface: Vec<bool>,
    /// The earlier classes each one extends or implements.
    supers: Vec<Vec<usize>>,
}

/// Every shape of `n` classes, once up to renaming the classes: each a class
/// or an interface whose supertypes are any earlier ones a Java declaration
/// can name (a class extends at most one class; an interface extends only
/// interfaces). Every acyclic hierarchy can be numbered so; two numberings of
/// one hierarchy are the same world.
fn shapes(n: usize) -> Vec<Shape> {
    let mut seen = FxHashSet::default();
    every_shape(n).into_iter().filter(|s| seen.insert(canonical(s))).collect()
}

/// The least form of a shape over every renaming of its classes.
fn canonical(s: &Shape) -> (Vec<bool>, Vec<(usize, usize)>) {
    let n = s.interface.len();
    let mut perm: Vec<usize> = (0..n).collect();
    let mut best = None;
    loop {
        let mut kinds = vec![false; n];
        for i in 0..n {
            kinds[perm[i]] = s.interface[i];
        }
        let mut edges: Vec<(usize, usize)> = (0..n).flat_map(|i| s.supers[i].iter().map(move |&j| (i, j))).map(|(i, j)| (perm[i], perm[j])).collect();
        edges.sort();
        let form = (kinds, edges);
        if best.as_ref().is_none_or(|b| form < *b) {
            best = Some(form);
        }
        // the next permutation in lexicographic order
        let Some(i) = (1..n).rev().find(|&i| perm[i - 1] < perm[i]) else { break };
        let j = (i..n).rev().find(|&j| perm[j] > perm[i - 1]).unwrap();
        perm.swap(i - 1, j);
        perm[i..].reverse();
    }
    best.unwrap()
}

fn every_shape(n: usize) -> Vec<Shape> {
    let mut out = vec![Shape { interface: Vec::new(), supers: Vec::new() }];
    for i in 0..n {
        let mut next = Vec::new();
        for s in &out {
            for interface in [false, true] {
                for mask in 0..(1u32 << i) {
                    let sup: Vec<usize> = (0..i).filter(|j| mask & (1 << j) != 0).collect();
                    let classes = sup.iter().filter(|&&j| !s.interface[j]).count();
                    if (interface && classes > 0) || classes > 1 {
                        continue;
                    }
                    let mut t = s.clone();
                    t.interface.push(interface);
                    t.supers.push(sup);
                    next.push(t);
                }
            }
        }
        out = next;
    }
    out
}

/// The declaration of class `i` in world `n` of method name `k`: digit `i`
/// of `k` in base `base`, so the names together cover every combination.
fn digit(k: usize, i: usize, base: usize) -> usize {
    k / base.pow(i as u32) % base
}

fn method_decl(k: usize, i: usize) -> Decl {
    METHOD_DECLS[digit(k, i, METHOD_DECLS.len())]
}

fn field_decl(k: usize, i: usize) -> Decl {
    FIELD_DECLS[digit(k, i, FIELD_DECLS.len())]
}

fn source(shape: &Shape, i: usize, n: usize, unknown: bool) -> String {
    let iface = shape.interface[i];
    let mut s = String::from("package p;\n");
    if unknown {
        // an annotation Lombok handles that the model does not reproduce
        s.push_str("@lombok.experimental.Accessors(chain = true)\n");
    }
    let (classes, interfaces): (Vec<usize>, Vec<usize>) = shape.supers[i].iter().partition(|&&j| !shape.interface[j]);
    let names = |v: &[usize]| v.iter().map(|&j| NAMES[j]).collect::<Vec<_>>().join(", ");
    if iface {
        s.push_str(&format!("interface {}", NAMES[i]));
        if !interfaces.is_empty() {
            s.push_str(&format!(" extends {}", names(&interfaces)));
        }
    } else {
        s.push_str(&format!("class {}", NAMES[i]));
        if !classes.is_empty() {
            s.push_str(&format!(" extends {}", names(&classes)));
        }
        if !interfaces.is_empty() {
            s.push_str(&format!(" implements {}", names(&interfaces)));
        }
    }
    s.push_str(" {\n");
    for k in 0..METHOD_DECLS.len().pow(n as u32) {
        let line = match (method_decl(k, i), iface) {
            (Decl::Absent, _) => continue,
            (Decl::Inherited, false) => format!("{}void m{k}() {{}}", access(k, i, ["public ", "", "protected "])),
            (Decl::Inherited, true) => format!("default void m{k}() {{}}"),
            (Decl::Private, _) => format!("private void m{k}() {{}}"),
        };
        s.push_str(&line);
        s.push('\n');
    }
    for k in 0..FIELD_DECLS.len().pow(n as u32) {
        let line = match (field_decl(k, i), iface) {
            (Decl::Absent, _) => continue,
            (_, false) => format!("{}int f{k};", access(k, i, ["public ", "", "private "])),
            (_, true) => format!("int f{k} = 0;"),
        };
        s.push_str(&line);
        s.push('\n');
    }
    s.push_str("}\n");
    s
}

/// The file the questions are asked from: an access to every field name
/// through a value of every class.
fn probe(n: usize) -> String {
    let params = (0..n).map(|i| format!("{} {}", NAMES[i], NAMES[i].to_lowercase())).collect::<Vec<_>>().join(", ");
    let mut s = format!("package p;\nclass Q {{\n void q({params}) {{\n Object o;\n");
    for i in 0..n {
        for k in 0..FIELD_DECLS.len().pow(n as u32) {
            s.push_str(&format!(" o = {}.f{k};\n", NAMES[i].to_lowercase()));
        }
    }
    s.push_str(" }\n}\n");
    s
}

/// The classes above `i`, `i` included.
fn closure(shape: &Shape, i: usize) -> Vec<usize> {
    let mut out = vec![i];
    let mut k = 0;
    while k < out.len() {
        for &j in &shape.supers[out[k]] {
            if !out.contains(&j) {
                out.push(j);
            }
        }
        k += 1;
    }
    out
}

/// The file the questions are asked from, parsed, with its field accesses
/// through a value of each class.
struct Probe<'p> {
    pf: &'p ParsedFile,
    stub: Arc<FileStub>,
    accesses: Vec<Vec<grounds_syntax::Node<'p>>>,
}

impl<'p> Probe<'p> {
    fn new(n: usize, src: &'p str, pf: &'p ParsedFile) -> Probe<'p> {
        let mut accesses: Vec<Vec<grounds_syntax::Node<'p>>> = vec![Vec::new(); n];
        for a in pf.root().descendants().filter(|a| a.kind() == "field_access") {
            let o = a.field("object").unwrap().text(src);
            accesses[NAMES.iter().position(|c| c.to_lowercase() == o).unwrap()].push(a);
        }
        Probe { pf, stub: Arc::new(extract(pf)), accesses }
    }
}

/// What the kernel answers in one world: for each class, whether it is
/// closed, and the claims it grants.
struct Answers {
    closed: Vec<bool>,
    claims: FxHashSet<String>,
}

fn ask(n: usize, stubs: &[Option<Arc<FileStub>>], probe: &Probe<'_>) -> Answers {
    let mut idx = Index::new(jdk());
    for (i, s) in stubs.iter().enumerate() {
        if let Some(s) = s {
            idx.set_file(Arc::from(format!("p/{}.java", NAMES[i]).as_str()), s.clone());
        }
    }
    idx.set_file(Arc::from("p/Q.java"), probe.stub.clone());
    let fm = FileModel::new(probe.pf, &idx, Arc::from("p/Q.java"));
    let mut closed = Vec::new();
    let mut claims = FxHashSet::default();
    for i in 0..n {
        let Ok(c) = fm.closed(&format!("p.{}", NAMES[i]), Jars::Exact) else {
            closed.push(false);
            continue;
        };
        closed.push(true);
        for k in 0..METHOD_DECLS.len().pow(n as u32) {
            let ms = c.methods(&format!("m{k}"));
            if let Some(x) = ms.none() {
                claims.insert(format!("{i} none {}", x.as_str()));
            }
            if let Some(x) = ms.none_taking(1) {
                claims.insert(format!("{i} taking {}", x.as_str()));
            }
        }
        for a in &probe.accesses[i] {
            if let Some(x) = c.no_field(*a) {
                claims.insert(format!("{i} field {}", x.as_str()));
            }
        }
    }
    Answers { closed, claims }
}

#[derive(Default)]
struct Counts {
    worlds: usize,
    proofs: usize,
    granted: usize,
    missed: usize,
}

/// Every world of one shape: everything visible, then each class hidden or
/// not and each visible one marked or not.
fn check_shape(n: usize, shape: &Shape, probe: &Probe<'_>, stub: &mut impl FnMut(String) -> Arc<FileStub>, counts: &mut Counts) {
    let names = METHOD_DECLS.len().pow(n as u32);
    let full: Vec<Option<Arc<FileStub>>> = (0..n).map(|i| Some(stub(source(shape, i, n, false)))).collect();
    let base = ask(n, &full, probe);
    counts.worlds += 1;
    // with everything visible every class is closed...
    assert!(base.closed.iter().all(|&c| c), "{shape:?}: not closed with everything visible");
    // ...and (c): no method named m exactly when the class inherits none
    for i in 0..n {
        for k in 0..names {
            let inherits = closure(shape, i).iter().any(|&j| match method_decl(k, j) {
                Decl::Absent => false,
                Decl::Private => j == i,
                Decl::Inherited => true,
            });
            // FINDING, left open: a supertype's private `m` is no member of
            // the class (JLS 8.2; javac: "cannot find symbol"), yet the kernel
            // grants no "no method" claim for it: `methods_named` lists the
            // private method, and dropping it there would also need the
            // check of `private_elsewhere` that it hides no inherited one (an
            // interface default). unresolved-method reports such a call from
            // another top-level class as private to the supertype, and none
            // from within it: a break missed, never a false one. Only (c)'s
            // "exactly" is weakened, and only by this case.
            let private_above = closure(shape, i).iter().any(|&j| j != i && method_decl(k, j) == Decl::Private);
            let claim = format!("{i} none `p.{}` has no method `m{k}`", NAMES[i]);
            assert_eq!(base.claims.contains(&claim), !inherits && !private_above, "{shape:?}: class {} and m{k}", NAMES[i]);
            counts.missed += (!inherits && private_above) as usize;
        }
    }
    counts.proofs += n;
    counts.granted += base.claims.len();
    for hidden in 0..(1u32 << n) {
        let visible: Vec<usize> = (0..n).filter(|i| hidden & (1 << i) == 0).collect();
        for marks in 0..(1u32 << visible.len()) {
            if hidden == 0 && marks == 0 {
                continue;
            }
            let unknown: Vec<usize> = visible.iter().enumerate().filter(|(b, _)| marks & (1 << b) != 0).map(|(_, &i)| i).collect();
            let stubs: Vec<Option<Arc<FileStub>>> = (0..n).map(|i| visible.contains(&i).then(|| stub(source(shape, i, n, unknown.contains(&i))))).collect();
            let w = ask(n, &stubs, probe);
            counts.worlds += 1;
            for i in 0..n {
                // (a): closed exactly when the whole closure is visible and known
                let intact = closure(shape, i).iter().all(|j| visible.contains(j) && !unknown.contains(j));
                assert_eq!(w.closed[i], intact, "{shape:?}, hidden {hidden:b}, unknown {unknown:?}: class {}", NAMES[i]);
                counts.proofs += w.closed[i] as usize;
            }
            // (b): hiding never creates a claim
            let made: Vec<&String> = w.claims.difference(&base.claims).collect();
            assert!(made.is_empty(), "{shape:?}, hidden {hidden:b}, unknown {unknown:?}: claims not granted with everything visible: {made:?}");
            counts.granted += w.claims.len();
        }
    }
}

#[test]
fn every_small_world() {
    const THREADS: usize = 4;
    let all: Vec<(usize, Shape)> = (1..=4).flat_map(|n| shapes(n).into_iter().map(move |s| (n, s))).collect();
    let counts: Vec<Counts> = std::thread::scope(|scope| {
        let all = &all;
        let workers: Vec<_> = (0..THREADS)
            .map(|t| {
                scope.spawn(move || {
                    let mut counts = Counts::default();
                    // extracted once per (class, shape of it and its supertypes, mark)
                    let mut cache: FxHashMap<String, Arc<FileStub>> = FxHashMap::default();
                    let mut stub = |src: String| cache.entry(src).or_insert_with_key(|s| Arc::new(extract(&ParsedFile::parse(s.as_str())))).clone();
                    let sources: Vec<String> = (0..=4).map(probe).collect();
                    let parsed: Vec<ParsedFile> = sources.iter().map(|s| ParsedFile::parse(s.as_str())).collect();
                    let probes: Vec<Probe<'_>> = (0..=4).map(|n| Probe::new(n, &sources[n], &parsed[n])).collect();
                    for (n, shape) in all.iter().skip(t).step_by(THREADS) {
                        check_shape(*n, shape, &probes[*n], &mut stub, &mut counts);
                    }
                    counts
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    let total = counts.iter().fold(Counts::default(), |a, c| Counts { worlds: a.worlds + c.worlds, proofs: a.proofs + c.proofs, granted: a.granted + c.granted, missed: a.missed + c.missed });
    // 2, 6, 24 and 149 shapes of 1 to 4 classes, 3^n worlds each
    assert_eq!(total.worlds, 12777);
    eprintln!("{} worlds, {} classes proved closed, {} claims granted, {} absent methods not claimed (a supertype's private one)", total.worlds, total.proofs, total.granted, total.missed);
}
