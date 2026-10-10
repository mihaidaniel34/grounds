use super::*;
use grounds_syntax::ParsedFile;
use std::sync::Arc;

fn jdk() -> Arc<Classpath> {
    let home = grounds_classfile::find_java_home();
    Arc::new(Classpath::build(home.as_deref(), 21, &[]))
}

fn index_of(files: &[(&str, &str)]) -> Index {
    let mut idx = Index::new(jdk());
    for (p, src) in files {
        let pf = ParsedFile::parse(*src);
        idx.set_file(Arc::from(*p), Arc::new(extract(&pf)));
    }
    idx
}

#[test]
fn resolves_source_and_jdk_types() {
    let idx = index_of(&[(
        "A.java",
        "package p; import java.util.*; import java.util.function.Function;
         public class A<T> extends ArrayList<String> implements Comparable<A<T>> {
           private Map<String, List<T>> m;
           public Map.Entry<String,Integer> e() { return null; }
           static class Inner { Inner self; }
           Function<String, Integer> f;
           public int compareTo(A<T> o) { return 0; }
         }",
    )]);
    let a = idx.class("p.A").expect("class");
    assert_eq!(a.superclass.as_ref().unwrap().to_string(), "java.util.ArrayList<java.lang.String>");
    assert_eq!(a.field("m").unwrap().ty.to_string(), "java.util.Map<java.lang.String, java.util.List<T>>");
    let e = a.methods_named("e").next().unwrap();
    assert_eq!(e.ret.to_string(), "java.util.Map.Entry<java.lang.String, java.lang.Integer>");
    let inner = idx.class("p.A$Inner").unwrap();
    assert_eq!(inner.field("self").unwrap().ty.to_string(), "p.A.Inner");
    assert!(idx.is_subclass("p.A", "java.util.Collection").is_yes());
    assert!(idx.is_subclass("p.A", "java.lang.Runnable").is_no());
    let it = idx.as_super(&Ty::class("p.A"), "java.lang.Iterable").unwrap();
    assert_eq!(it.to_string(), "java.lang.Iterable<java.lang.String>");
    let get = idx.methods_named(&a.superclass.clone().unwrap(), "get");
    assert!(get.iter().any(|m| m.method.ret.is_string()));
}

#[test]
fn lombok_members() {
    let idx = index_of(&[(
        "P.java",
        "package q; import lombok.*;
         @Data @Builder public class P { private final String name; private boolean active; private int age; }
         @Value class V { String a; }
         @Slf4j @RequiredArgsConstructor class S { private final P p; private String x; }",
    )]);
    let p = idx.class("q.P").unwrap();
    for m in ["getName", "isActive", "setActive", "getAge", "setAge", "builder", "toString", "equals", "hashCode"] {
        assert!(p.methods_named(m).next().is_some(), "missing {m}");
    }
    assert!(p.methods_named("setName").next().is_none());
    let b = idx.class("q.P$PBuilder").unwrap();
    assert!(b.methods_named("name").next().is_some());
    assert_eq!(b.methods_named("build").next().unwrap().ret.to_string(), "q.P");
    let v = idx.class("q.V").unwrap();
    assert!(v.field("a").unwrap().flags.is_final());
    assert_eq!(v.constructors().next().unwrap().params.len(), 1);
    let s = idx.class("q.S").unwrap();
    assert_eq!(s.field("log").unwrap().ty.to_string(), "org.slf4j.Logger".replace("org.slf4j.Logger", "?unknown"));
    let ctor = s.constructors().next().unwrap();
    assert_eq!(ctor.params.len(), 1);
    assert_eq!(&*ctor.params[0].name, "p");
}

#[test]
fn enums_records_interfaces() {
    let idx = index_of(&[(
        "E.java",
        "package r; enum Color { RED, GREEN; int x() { return 1; } }
         record Point(int x, int y) { }
         interface Shape { double area(); default String n() { return \"\"; } int K = 3; }",
    )]);
    let c = idx.class("r.Color").unwrap();
    assert!(c.field("RED").unwrap().flags.has(Flags::ENUM_CONSTANT));
    assert!(c.methods_named("values").next().is_some());
    assert!(idx.methods_named(&Ty::class("r.Color"), "ordinal").len() == 1);
    let p = idx.class("r.Point").unwrap();
    assert!(p.methods_named("x").next().is_some());
    assert_eq!(p.constructors().next().unwrap().params.len(), 2);
    let s = idx.class("r.Shape").unwrap();
    assert!(s.methods_named("area").next().unwrap().flags.is_abstract());
    assert!(!s.methods_named("n").next().unwrap().flags.is_abstract());
    assert!(s.field("K").unwrap().flags.is_static());
    assert_eq!(s.field("K").unwrap().constant, Some(Constant::Int(3)));
}
