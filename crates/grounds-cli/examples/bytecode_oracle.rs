//! Probe: compare what grounds resolves at each call site with what javac
//! recorded in the class files compiled from the same sources. A class file
//! names, for every call, the method javac chose (owner, name, descriptor)
//! and the line of the call: where grounds is certain of another method, it
//! is wrong, and every rule that asks for that call inherits the mistake.
//! scripts/bytecode_oracle.py fetches sources and classes of published
//! libraries and runs this on them.
//!
//!   bytecode_oracle <project root> <bytecode.tsv>
//!
//! The sources are under `<project root>/src/main/java`. bytecode.tsv: source
//! file (relative to that), line, owner, name, descriptor of every invoke
//! instruction (from `javap -c -l -p`).
//!
//! A probe, not a gate: calls are matched by file, line, name and number of
//! parameters, so two calls of one name on a line are not told apart, and
//! calls javac compiles to something else (string concatenation, accessors
//! for private members before Java 11) are not compared.

use grounds_index::{ClassLookup, MethodInfo, Prim, Ty};
use grounds_syntax::NodeExt;
use rayon::prelude::*;
use std::collections::HashMap;

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

/// Parameter descriptors and the return descriptor of `(..)R`.
fn split(d: &str) -> (Vec<String>, String) {
    let (ps, ret) = d[1..].split_once(')').unwrap_or(("", ""));
    let b = ps.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        let s = i;
        while b[i] == b'[' {
            i += 1;
        }
        if b[i] == b'L' {
            while b[i] != b';' {
                i += 1;
            }
        }
        i += 1;
        out.push(ps[s..i].to_string());
    }
    (out, ret.to_string())
}

#[derive(Default)]
struct Tally {
    calls: usize,
    resolved: usize,
    certain: usize,
    compared: usize,
    agree: usize,
    other_overload: Vec<String>,
    other_owner: Vec<String>,
    other_return: Vec<String>,
    /// javac names another method by a rule of its own, not by another choice
    javac_quirk: usize,
    unresolved_known_to_javac: usize,
    guess_compared: usize,
    guess_wrong: usize,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = std::path::PathBuf::from(&args[1]).canonicalize()?;
    // (file, line, name, argc) -> [(owner, params, ret)]
    let mut bc: HashMap<(String, usize, String, usize), Vec<(String, Vec<String>, String)>> = HashMap::new();
    for l in std::fs::read_to_string(&args[2])?.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() != 5 {
            continue;
        }
        let (ps, ret) = split(f[4]);
        bc.entry((f[0].to_string(), f[1].parse()?, f[3].to_string(), ps.len())).or_default().push((f[2].replace('/', "."), ps, ret));
    }
    let p = grounds_engine::Project::load(&root)?;
    let src_root = root.join("src/main/java");
    let files: Vec<_> = p.workspace.files.iter().collect();
    let tallies: Vec<Tally> = files
        .par_iter()
        .map(|f| {
            let mut t = Tally::default();
            let Ok(rel) = f.path.strip_prefix(&src_root) else { return t };
            let rel = rel.to_string_lossy().to_string();
            let Ok(text) = std::fs::read_to_string(&f.path) else { return t };
            let pf = grounds_syntax::ParsedFile::parse(text);
            let fm = grounds_semantic::FileModel::new(&pf, &p.index, grounds_engine::project::path_key(&f.path));
            for n in pf.root().descendants().filter(|n| n.kind() == "method_invocation") {
                t.calls += 1;
                let (Some(name), Some(al)) = (n.field("name").map(|x| fm.text(x)), n.field("arguments")) else { continue };
                // javac gives the invoke instruction the line of the `(`
                let line = pf.lines.line_of(al.start_byte() as u32) + 1;
                let nargs = fm.args_of(n).len();
                let Some(r) = fm.resolve_call(n) else {
                    if bc.contains_key(&(rel.clone(), line, name.to_string(), nargs)) {
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
                let Some(sites) = bc.get(&(rel.clone(), line, name.to_string(), subst.len())) else { continue };
                let fits = |m: &MethodInfo, ps: &[String]| m.params.iter().zip(ps).all(|(q, d)| desc(&q.ty).is_none_or(|x| x == *d));
                let at = format!("{rel}:{line} {name}");
                // a signature-polymorphic method (JLS 15.12.3): the call's own
                // argument types are its descriptor
                if matches!(&*r.owner, "java.lang.invoke.MethodHandle" | "java.lang.invoke.VarHandle") {
                    t.javac_quirk += 1;
                    continue;
                }
                let same_params: Vec<&(String, Vec<String>, String)> = sites.iter().filter(|s| cands.iter().any(|m| fits(m, &s.1))).collect();
                if !certain {
                    t.guess_compared += 1;
                    if same_params.is_empty() {
                        t.guess_wrong += 1;
                    }
                    continue;
                }
                t.compared += 1;
                if same_params.is_empty() {
                    t.other_overload.push(format!("{at}: grounds {}.{}, javac {}", r.owner, r.method.signature_string(), sites.iter().map(|s| format!("{}.{name}({})", s.0, s.1.join(""))).collect::<Vec<_>>().join(" | ")));
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
                    t.other_owner.push(format!("{at}: grounds {}, javac {}", r.owner, same_params.iter().map(|s| s.0.as_str()).collect::<Vec<_>>().join(" | ")));
                    continue;
                }
                if !same_params.iter().any(|s| cands.iter().any(|m| fits(m, &s.1) && desc(&m.ret).is_none_or(|x| x == s.2))) {
                    t.other_return.push(format!("{at}: grounds {} {}.{}, javac returns {}", cands[0].ret, r.owner, r.method.signature_string(), same_params[0].2));
                    continue;
                }
                t.agree += 1;
            }
            t
        })
        .collect();
    let mut s = Tally::default();
    for t in tallies {
        s.calls += t.calls;
        s.resolved += t.resolved;
        s.certain += t.certain;
        s.compared += t.compared;
        s.agree += t.agree;
        s.javac_quirk += t.javac_quirk;
        s.unresolved_known_to_javac += t.unresolved_known_to_javac;
        s.guess_compared += t.guess_compared;
        s.guess_wrong += t.guess_wrong;
        s.other_overload.extend(t.other_overload);
        s.other_owner.extend(t.other_owner);
        s.other_return.extend(t.other_return);
    }
    println!("files {}  calls {}  resolved {}  certain {}", files.len(), s.calls, s.resolved, s.certain);
    println!("certain calls found in bytecode {}  agree {}  other overload {}  other owner {}  other return {}  javac's own naming {}", s.compared, s.agree, s.other_overload.len(), s.other_owner.len(), s.other_return.len(), s.javac_quirk);
    println!("guessed (ambiguous/erasure-only) calls found in bytecode {}  of which javac chose another {}", s.guess_compared, s.guess_wrong);
    println!("unresolved by grounds, resolved by javac {}", s.unresolved_known_to_javac);
    for (title, v) in [("OTHER OVERLOAD", &s.other_overload), ("OTHER OWNER", &s.other_owner), ("OTHER RETURN", &s.other_return)] {
        for x in v {
            println!("{title}  {x}");
        }
    }
    Ok(())
}
