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
         @Value class V { String a; @NonFinal String b; @PackagePrivate String c; static int d; }
         @Value @NonFinal class W { String a; }
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
    assert!(v.flags.is_final());
    assert!(v.field("a").unwrap().flags.is_final() && v.field("a").unwrap().flags.is_private());
    let b = v.field("b").unwrap();
    assert!(!b.flags.is_final() && b.flags.is_private());
    let c = v.field("c").unwrap();
    assert!(c.flags.is_final() && !c.flags.is_private());
    assert!(!v.field("d").unwrap().flags.is_final());
    assert_eq!(v.constructors().next().unwrap().params.len(), 3);
    let w = idx.class("q.W").unwrap();
    assert!(!w.flags.is_final() && w.field("a").unwrap().flags.is_final());
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

#[test]
fn constants_that_refer_to_each_other_are_not_constants() {
    // j2cl's circularcompiletimeconstant test: javac compiles it, and none of
    // the cycle is a constant. This overflowed the stack.
    let idx = index_of(&[(
        "Main.java",
        "package q; public class Main {
           public static class A { static final int A = B.B; static final int C = B.B + A; static final int D = 1 + 2; }
           public static class B { static final int B = A.A + 1; }
           static final int X = Y.Y + Y.Y + Y.Y + Y.Y;
           static class Y { static final int Y = Main.X + Main.X + Main.X + Main.X; }
         }",
    )]);
    let a = idx.class("q.Main$A").unwrap();
    assert_eq!(a.field("A").unwrap().constant, None);
    assert_eq!(a.field("C").unwrap().constant, None);
    assert_eq!(a.field("D").unwrap().constant, Some(Constant::Int(3)));
    assert_eq!(idx.class("q.Main$B").unwrap().field("B").unwrap().constant, None);
    assert_eq!(idx.class("q.Main").unwrap().field("X").unwrap().constant, None);
}

#[test]
fn a_chain_of_constants_across_classes_is_constant() {
    // C0.V = C1.V; ... C40.V = 7: longer than the 32-level evaluation budget,
    // still a constant: past the budget, resolve_constant_name falls back to
    // the constant of the lowered class, which has a budget of its own
    let n = 40;
    let mut src = String::from("package c; class Chain {");
    for i in 0..n {
        src.push_str(&format!(" static class C{i} {{ static final int V = C{}.V; }}", i + 1));
    }
    src.push_str(&format!(" static class C{n} {{ static final int V = 7; }} }}"));
    let idx = index_of(&[("Chain.java", &src)]);
    assert_eq!(idx.class("c.Chain$C0").unwrap().field("V").unwrap().constant, Some(Constant::Int(7)));
}

#[test]
fn constants_that_refer_to_themselves_by_simple_name_terminate() {
    // does not compile (forward references), but is a state between edits
    let idx = index_of(&[("F.java", "package f; class F { static final int A = B + B + B + B; static final int B = A + A + A + A; static final int C = 7; }")]);
    let f = idx.class("f.F").unwrap();
    assert_eq!(f.field("A").unwrap().constant, None);
    assert_eq!(f.field("C").unwrap().constant, Some(Constant::Int(7)));
}

#[test]
fn without_a_jdk_no_hierarchy_is_complete() {
    // an interface declares no supertypes, but has Object's methods
    let src = "package p; interface I {} class C implements I {}";
    let pf = ParsedFile::parse(src);
    let mut bare = Index::new(Arc::new(Classpath::empty()));
    bare.set_file(Arc::from("p/I.java"), Arc::new(extract(&pf)));
    assert!(!bare.hierarchy_complete("p.I"));
    assert!(!bare.hierarchy_complete("p.C"));
    let with_jdk = index_of(&[("p/I.java", src)]);
    assert!(with_jdk.hierarchy_complete("p.I"));
}

#[test]
fn release_newer_than_the_jdk_leaves_jdk_subclasses_incomplete() {
    let Some(home) = grounds_classfile::find_java_home() else { return };
    let Some(installed) = grounds_classfile::jdk_feature_release(&home) else { return };
    let src = "package p; class Meter extends java.util.concurrent.CopyOnWriteArrayList<String> {} class Plain {}";
    let index = |release: u32| {
        let mut idx = Index::new(Arc::new(Classpath::build(Some(&home), release, &[])));
        idx.set_file(Arc::from("p/M.java"), Arc::new(extract(&ParsedFile::parse(src))));
        idx
    };
    // the JDK's own release and older ones are exact: nothing to hide
    let exact = index(installed);
    assert_eq!(exact.classpath.jdk, JdkApi::Complete);
    assert!(exact.hierarchy_complete("p.Meter"));
    // a newer release: JDK classes may have members the JDK does not have yet,
    // but a class without JDK supertypes is still complete
    let newer = index(installed + 1);
    assert_eq!(newer.classpath.jdk, JdkApi::Newer { requested: installed + 1, installed });
    assert!(!newer.hierarchy_complete("p.Meter"));
    assert!(newer.hierarchy_complete("p.Plain"));
}

/// A JDK home whose runtime image cannot be read: the real `ct.sym` and
/// `release` of `real`, and a `lib/modules` that is not a jimage.
fn jdk_with_unreadable_image(dir: &std::path::Path, real: &std::path::Path) {
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::copy(real.join("lib/ct.sym"), dir.join("lib/ct.sym")).unwrap();
    std::fs::copy(real.join("release"), dir.join("release")).unwrap();
    std::fs::write(dir.join("lib/modules"), b"not a runtime image").unwrap();
}

#[test]
fn unreadable_runtime_image_approximates_the_jdk_api() {
    let Some(home) = grounds_classfile::find_java_home() else { return };
    let Some(installed) = grounds_classfile::jdk_feature_release(&home) else { return };
    let dir = std::env::temp_dir().join(format!("grounds-index-unreadable-jdk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    jdk_with_unreadable_image(&dir, &home);
    let src = "package p; class Meter extends java.util.concurrent.CopyOnWriteArrayList<String> {} class Plain {}";
    let mut idx = Index::new(Arc::new(Classpath::build(Some(&dir), installed, &[])));
    idx.set_file(Arc::from("p/M.java"), Arc::new(extract(&ParsedFile::parse(src))));
    // the JDK's own release, approximated by the release before it
    assert_eq!(idx.classpath.jdk, JdkApi::Unreadable { release: installed });
    assert!(!idx.hierarchy_complete("p.Meter"));
    assert!(idx.hierarchy_complete("p.Plain"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jdk_home_without_ct_sym_is_absent() {
    let dir = std::env::temp_dir().join(format!("grounds-index-absent-jdk-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    let cp = Classpath::build(Some(&dir), 21, &[]);
    assert_eq!(cp.jdk, JdkApi::Absent { home: dir.clone() });
    assert!(cp.is_empty());
    assert!(!Index::new(Arc::new(cp)).hierarchy_complete("p.C"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn package_private_jdk_classes_are_not_visible() {
    let Some(home) = grounds_classfile::find_java_home() else { return };
    let Some(release) = grounds_classfile::jdk_feature_release(&home) else { return };
    let cp = Classpath::build(Some(&home), release, &[]);
    assert!(!cp.contains("java.util.ImmutableCollections"));
    assert!(cp.class("java.util.ImmutableCollections").is_none());
    assert!(cp.contains("java.util.List"));
    assert!(cp.class("java.util.List").is_some());
    assert!(cp.contains("java.awt.Container$AccessibleAWTContainer"));
    assert!(cp.class_names().all(|n| cp.contains(n)));
}

#[test]
fn lombok_auto_singular_names() {
    use crate::lombok::auto_singularize;
    // the answers of Lombok 1.18.36's Singulars.autoSingularize, run on these names
    let cases = [
        ("items", Some("item")), ("statuses", Some("status")), ("addresses", Some("address")), ("boxes", Some("box")), ("matches", Some("match")),
        ("categories", Some("category")), ("children", Some("child")), ("indices", Some("index")), ("heroes", Some("hero")), ("wolves", Some("wolf")),
        ("leaves", Some("leaf")), ("userIds", Some("userId")), ("menuItems", Some("menuItem")), ("Men", Some("Man")), ("chairmen", None),
        ("species", None), ("news", None), ("status", None), ("data", None), ("classes", Some("class")), ("analyses", Some("analysis")),
        ("headers", Some("header")), ("bases", Some("base")), ("caches", Some("cach")),
    ];
    for (plural, singular) in cases {
        assert_eq!(auto_singularize(plural).as_deref(), singular, "{plural}");
    }
}

#[test]
fn lombok_classes_with_unmodelled_annotations_are_marked_open() {
    let idx = index_of(&[
        (
            "A.java",
            "package p; import lombok.*; import lombok.experimental.*; import java.util.List;
             @Data @Builder @FieldNameConstants class Known { @Singular private List<String> tags; @Builder.Default private int n = 1; }
             class Delegating { @Delegate private List<String> items; }
             @Accessors(prefix = \"m\") class Prefixed { @Getter private String mName; }
             class Lazy { @Getter(lazy = true) private final String v = \"\"; }
             @Builder class Guava { @Singular private com.google.common.collect.ImmutableList<String> xs; }
             @Builder class NoSingular { @Singular private List<String> data; }
             @lombok.experimental.WithBy class Qualified { int a; }
             @Unknown class Plain { }",
        ),
        // a single-type import decides: this `Delegate` is not Lombok's
        ("B.java", "package q; import other.Delegate; class Mine { @Delegate String s; }"),
    ]);
    let open = |n: &str| idx.class(n).unwrap_or_else(|| panic!("{n}")).flags.has(Flags::MEMBERS_UNKNOWN);
    for n in ["p.Known", "p.Known$KnownBuilder", "p.Known$Fields", "p.Plain", "q.Mine"] {
        assert!(!open(n), "{n} open");
    }
    for n in ["p.Delegating", "p.Prefixed", "p.Lazy", "p.Guava", "p.NoSingular", "p.Qualified"] {
        assert!(open(n), "{n} not open");
    }
}
