//! "Overrides nothing" in an anonymous class created with the diamond: its
//! supertype's type arguments are what javac infers from the context, and
//! grounds does not infer them. What depends on them is open; what no
//! argument could change is still claimed; with the arguments written, the
//! answer is as before, a type variable of an enclosing class or method
//! among them erasing to its bound.

use crate::closed::{Jars, Open};
use crate::FileModel;
use grounds_index::*;
use grounds_syntax::{NodeExt, ParsedFile};
use std::sync::{Arc, OnceLock};

fn jdk() -> Arc<Classpath> {
    static CP: OnceLock<Arc<Classpath>> = OnceLock::new();
    CP.get_or_init(|| Arc::new(Classpath::build(grounds_classfile::find_java_home().as_deref(), 21, &[]))).clone()
}

const SRC: &str = "package p;
interface PostProcessor<T> { <O extends T> O postProcess(O object); }
interface Handler<T> { void handle(T value); }
interface Sink<T> { void put(String key, T value); }
interface Manager { }
class Config {
    Object diamond() {
        return new PostProcessor<>() {
            @Override public Manager postProcess(Manager object) { return object; }
            @Override public Manager postProcess(Manager object, int times) { return object; }
            @Override public Manager postProces(Manager object) { return object; }
        };
    }
    Object handler() {
        return new Handler<>() {
            @Override public void handle(String value) { }
        };
    }
    Object sink() {
        return new Sink<>() {
            @Override public void put(String key, Manager value) { }
            @Override public void put(Integer key, Manager value) { }
        };
    }
    Object explicit() {
        return new PostProcessor<Manager>() {
            @Override public Manager postProcess(Manager object) { return object; }
            @Override public String postProcess(String object) { return object; }
        };
    }
}
abstract class Queue<T> { protected abstract boolean lessThan(T a, T b); }
class Sub { }
class Merger<T extends Sub> {
    Object queue() {
        return new Queue<T>() {
            @Override protected boolean lessThan(Sub a, Sub b) { return true; }
        };
    }
    <S extends Sub> Object local() {
        return new Queue<S>() {
            @Override protected boolean lessThan(Sub a, Sub b) { return true; }
        };
    }
}
";

/// What the kernel answers for each method declared in an anonymous class,
/// in order: open (with the reason), overrides something (`None`), or the
/// claim that it overrides nothing.
fn answers() -> Option<Vec<(String, Result<bool, Open>)>> {
    let mut idx = Index::new(jdk());
    let pf = ParsedFile::parse(SRC);
    idx.set_file(Arc::from("p/Config.java"), Arc::new(extract(&pf)));
    idx.resolve_ordered();
    if idx.class("java.lang.Object").is_none() {
        return None;
    }
    let fm = FileModel::new(&pf, &idx, Arc::from("p/Config.java"));
    let mut out = Vec::new();
    for decl in pf.root().descendants().filter(|n| n.kind() == "method_declaration") {
        let Some(cls) = fm.enclosing_class(decl) else { continue };
        if !cls.name.contains('$') {
            continue;
        }
        let what = format!("{} {}", fm.text(decl.field("name").unwrap()), fm.text(decl.field("parameters").unwrap()));
        let closed = fm.closed(&cls.name, Jars::Exact).unwrap_or_else(|e| panic!("{what}: {e:?}"));
        out.push((what, closed.overrides_nothing(decl).map(|c| c.is_some())));
    }
    Some(out)
}

#[test]
fn a_diamond_leaves_open_only_what_depends_on_its_arguments() {
    let Some(got) = answers() else { return };
    let open = || Err(Open::TypeArgumentUnknown(Name::from("p.PostProcessor")));
    let want: Vec<(&str, Result<bool, Open>)> = vec![
        // through a type variable of the method bounded by the inferred argument
        ("postProcess (Manager object)", open()),
        // no supertype method of the name and parameter count: whatever the arguments
        ("postProcess (Manager object, int times)", Ok(true)),
        ("postProces (Manager object)", Ok(true)),
        // through the inferred argument itself: may be the same type
        ("handle (String value)", Ok(false)),
        ("put (String key, Manager value)", Ok(false)),
        // a parameter that differs without involving it: whatever the arguments
        ("put (Integer key, Manager value)", Ok(true)),
        // the arguments written: known, as before
        ("postProcess (Manager object)", Ok(false)),
        ("postProcess (String object)", Ok(true)),
        // an argument that is a type variable of the enclosing class or
        // method erases to its bound there: overrides by erasure
        ("lessThan (Sub a, Sub b)", Ok(false)),
        ("lessThan (Sub a, Sub b)", Ok(false)),
    ];
    let got: Vec<(&str, Result<bool, Open>)> = got.iter().map(|(w, r)| (w.as_str(), r.clone())).collect();
    assert_eq!(got, want);
}
