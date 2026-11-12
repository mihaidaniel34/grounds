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
    let it = idx.as_super(&a.this_type(), "java.lang.Iterable").unwrap();
    assert_eq!(it.to_string(), "java.lang.Iterable<java.lang.String>");
    // `A` without arguments is the raw type, whose supertypes are erased (JLS 4.8)
    let it = idx.as_super(&Ty::class("p.A"), "java.lang.Iterable").unwrap();
    assert_eq!(it.to_string(), "java.lang.Iterable");
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
fn lombok_equals_hash_code_and_can_equal_as_lombok_generates_them() {
    let idx = index_of(&[(
        "E.java",
        "package q; import lombok.*;
         @Data class Open { int a; }
         @Value class Closed { int a; }
         @Value @NonFinal class Reopened { int a; }
         @EqualsAndHashCode final class FinalPlain { int a; }
         @EqualsAndHashCode final class FinalObject extends Object { int a; }
         @Value @EqualsAndHashCode(callSuper = true) class FinalChild extends Open { int b; }
         @Data class OwnBoth { int a; public boolean equals(Object o) { return false; } public int hashCode() { return 0; } }
         @Data class OwnEquals { int a; public boolean equals(Object o) { return false; } }
         @EqualsAndHashCode class OwnHash { int a; public int hashCode() { return 0; } }
         @EqualsAndHashCode class OwnCanEqual { int a; boolean canEqual(Object o) { return true; } }",
    )]);
    let has = |c: &str, m: &str| idx.class(c).unwrap_or_else(|| panic!("{c}")).methods_named(m).next().is_some();
    let generated = |c: &str, m: &str| idx.class(c).unwrap().methods_named(m).any(|x| x.flags.has(Flags::GENERATED));
    for c in ["q.Open", "q.Reopened", "q.FinalChild"] {
        assert!(generated(c, "equals") && generated(c, "hashCode") && generated(c, "canEqual"), "{c}");
    }
    // final and extending Object: no subclass, no canEqual
    for c in ["q.Closed", "q.FinalPlain", "q.FinalObject"] {
        assert!(generated(c, "equals") && generated(c, "hashCode"), "{c}");
        assert!(!has(c, "canEqual"), "{c} has canEqual");
    }
    // equals or hashCode declared: Lombok generates none of the three
    for c in ["q.OwnBoth", "q.OwnEquals", "q.OwnHash"] {
        assert!(!generated(c, "equals") && !generated(c, "hashCode") && !has(c, "canEqual"), "{c}");
    }
    assert!(!has("q.OwnEquals", "hashCode") && !has("q.OwnHash", "equals"));
    // a declared canEqual stops only canEqual
    assert!(generated("q.OwnCanEqual", "equals") && generated("q.OwnCanEqual", "hashCode") && !generated("q.OwnCanEqual", "canEqual"));
}

#[test]
fn a_lazy_getter_field_is_an_atomic_reference() {
    // Lombok rewrites `@Getter(lazy = true) private final T x = init;` into
    // `private final AtomicReference<Object> x` and a getter returning T
    let idx = index_of(&[(
        "L.java",
        "package q; import lombok.Getter; import java.nio.file.Path;
         class L {
           @Getter(lazy = true) private final Path path = Path.of(\"/\");
           @Getter(lazy = true) private static final L INSTANCE = new L();
           @Getter(lazy = true) private final String name = \"n\";
           @Getter private final String plain = \"p\";
         }",
    )]);
    let l = idx.class("q.L").unwrap();
    for (f, getter, ty) in [("path", "getPath", "java.nio.file.Path"), ("INSTANCE", "getINSTANCE", "q.L"), ("name", "getName", "java.lang.String")] {
        let field = l.field(f).unwrap();
        assert_eq!(field.ty.to_string(), "java.util.concurrent.atomic.AtomicReference<java.lang.Object>", "{f}");
        assert_eq!(field.constant, None, "{f}");
        assert_eq!(l.methods_named(getter).next().unwrap_or_else(|| panic!("{getter}")).ret.to_string(), ty);
    }
    assert!(l.field("INSTANCE").unwrap().flags.is_static());
    assert_eq!(l.field("plain").unwrap().ty.to_string(), "java.lang.String");
    assert!(l.field("plain").unwrap().constant.is_some());
}

#[test]
fn a_record_declares_equals_hash_code_and_to_string() {
    // JLS 8.10.3: implicit unless declared with the same signature
    let idx = index_of(&[(
        "R.java",
        "package r;
         record Plain(int x) { }
         record Own(int x) {
           public boolean equals(Own o) { return true; }
           @Override public String toString() { return \"\"; }
         }
         record Full(int x) {
           @Override public boolean equals(java.lang.Object o) { return true; }
           @Override public int hashCode() { return 0; }
           public int x() { return x; }
         }",
    )]);
    let methods = |c: &str, m: &str| idx.class(c).unwrap().methods_named(m).cloned().collect::<Vec<_>>();
    for m in ["equals", "hashCode", "toString", "x"] {
        let ms = methods("r.Plain", m);
        assert_eq!(ms.len(), 1, "{m}");
        assert!(ms[0].flags.has(Flags::IMPLICIT) && ms[0].flags.is_public(), "{m}");
    }
    assert_eq!(methods("r.Plain", "equals")[0].params[0].ty.to_string(), "java.lang.Object");
    // `equals(Own)` is an overload: `equals(Object)` stays implicit
    let eq = methods("r.Own", "equals");
    assert_eq!(eq.len(), 2);
    assert!(eq.iter().any(|m| m.flags.has(Flags::IMPLICIT) && m.params[0].ty.to_string() == "java.lang.Object"));
    assert!(methods("r.Own", "toString").iter().all(|m| !m.flags.has(Flags::IMPLICIT)));
    assert_eq!(methods("r.Own", "hashCode").len(), 1);
    for m in ["equals", "hashCode", "x"] {
        let ms = methods("r.Full", m);
        assert!(ms.len() == 1 && !ms[0].flags.has(Flags::IMPLICIT), "{m}");
    }
    assert!(methods("r.Full", "toString")[0].flags.has(Flags::IMPLICIT));
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
    assert_eq!(bare.hierarchy_gap("p.C", false), Some(Gap::NoJdk));
    assert!(!bare.supertypes_known("p.C"));
    let with_jdk = index_of(&[("p/I.java", src)]);
    assert!(with_jdk.hierarchy_complete("p.I"));
    assert!(with_jdk.supertypes_known("p.C"));
}

#[test]
fn a_hierarchy_with_members_lombok_adds_unmodelled_is_not_complete() {
    let idx = index_of(&[(
        "p/D.java",
        "package p; import lombok.experimental.Delegate; import lombok.Data; import java.util.List;
         class Delegating { @Delegate List<String> items; class Inner { } static class Nested { class Deeper { } } }
         class Sub extends Delegating { }
         interface Port { }
         class Outside implements Port { Delegating d; }
         @Data class Known { int a; class Inner { } }",
    )]);
    for n in ["p.Delegating", "p.Sub", "p.Delegating$Inner", "p.Delegating$Nested", "p.Delegating$Nested$Deeper"] {
        assert!(!idx.hierarchy_complete(n), "{n} complete");
        // the members Lombok adds change no supertype
        assert!(idx.supertypes_known(n), "{n} has a supertype not known");
    }
    assert_eq!(idx.hierarchy_gap("p.Sub", true), Some(Gap::MembersUnknown(Arc::from("p.Delegating"))));
    for n in ["p.Port", "p.Outside", "p.Known", "p.Known$Inner"] {
        assert!(idx.hierarchy_complete(n), "{n} not complete");
    }
}

#[test]
fn supertypes_are_known_only_when_every_one_resolves() {
    let idx = index_of(&[("p/S.java", "package p; class Up extends lib.Gone { } class Down extends Up { } class Fine implements java.io.Serializable { }")]);
    // a name that resolves to no class is no class type: Up names an unknown
    assert_eq!(idx.hierarchy_gap("p.Down", false), Some(Gap::UnresolvedSupertype(Arc::from("p.Up"))));
    assert!(!idx.supertypes_known("p.Down"));
    assert!(idx.supertypes_known("p.Fine"));
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
    // a newer JDK class may have supertypes this JDK does not give it
    assert!(!newer.supertypes_known("p.Meter"));
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
    // no name resolves to it, but hierarchy walks see it
    assert!(!cp.contains("java.util.ImmutableCollections"));
    assert!(cp.class("java.util.ImmutableCollections").is_some());
    assert!(cp.contains("java.util.List"));
    assert!(cp.class("java.util.List").is_some());
    assert!(cp.contains("java.awt.Container$AccessibleAWTContainer"));
    assert!(cp.class_names().all(|n| cp.contains(n)));
}

#[test]
fn constants_defined_by_each_other_across_classes_are_bounded() {
    // found by scripts/fuzz_robust.py: each class hop restarted the depth
    // bound, so a cycle overflowed the stack and a chain doubling at each
    // class took 2^n steps. Run on a 2 MiB stack like a rayon worker.
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(|| {
            let idx = index_of(&[
                ("A.java", "package p; class A { static final int X = B.Y * 2 + A.X; }"),
                ("B.java", "package p; class B { static final int Y = A.X + 1; }"),
            ]);
            assert_eq!(idx.class("p.A").unwrap().field("X").unwrap().constant, None);
            assert_eq!(idx.class("p.B").unwrap().field("Y").unwrap().constant, None);

            let srcs: Vec<(String, String)> = (0..80)
                .map(|i| {
                    let init = if i == 0 { "1".to_string() } else { format!("K{p}.V + K{p}.V", p = i - 1) };
                    (format!("K{i}.java"), format!("package p; class K{i} {{ static final int V = {init}; }}"))
                })
                .collect();
            let files: Vec<(&str, &str)> = srcs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
            let idx = index_of(&files);
            let t = std::time::Instant::now();
            // a short chain is still evaluated
            assert_eq!(idx.class("p.K10").unwrap().field("V").unwrap().constant, Some(Constant::Int(1024)));
            let _ = idx.class("p.K79").unwrap().field("V").unwrap().constant.clone();
            assert!(t.elapsed() < std::time::Duration::from_secs(10), "{:?}", t.elapsed());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn string_constants_stop_at_the_class_file_limit() {
    use crate::stubs::{fold_binary, MAX_CONSTANT_STRING};
    let short = fold_binary("+", &Constant::Str(Arc::from("ab")), &Constant::Int(1));
    assert_eq!(short, Some(Constant::Str(Arc::from("ab1"))));
    let half = Constant::Str(Arc::from("x".repeat(MAX_CONSTANT_STRING / 2 + 1).as_str()));
    assert_eq!(fold_binary("+", &half, &half), None);
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

#[test]
fn a_receiver_parameter_is_not_a_parameter() {
    // JLS 8.4: `@A R this` and `R.this` only carry type annotations
    let idx = index_of(&[(
        "R.java",
        "package p; class R {
           boolean ok(@A R this, int x) { return true; }
           boolean plain(R this) { return true; }
           class I { I(@A R R.this, int y) {} }
         }",
    )]);
    let r = idx.class("p.R").unwrap();
    let ok = r.methods_named("ok").next().unwrap();
    assert_eq!(ok.params.iter().map(|p| &*p.name).collect::<Vec<_>>(), ["x"]);
    assert!(r.methods_named("plain").next().unwrap().params.is_empty());
    let i = idx.class("p.R$I").unwrap();
    assert_eq!(i.methods_named("<init>").next().map(|m| m.params.len()), Some(1));
}

#[test]
fn manifold_runtime_imports_mark_their_module_as_extended() {
    let mut idx = index_of(&[
        ("/r/a/src/test/java/p/A.java", "package p; import manifold.ext.rt.api.Jailbreak; class A {}"),
        ("/r/a/src/main/java/p/B.java", "package p; class B {}"),
        ("/r/b/src/main/java/q/C.java", "package q; import manifold.ext.Other; class C {}"),
    ]);
    assert!(idx.language_extended("/r/a/src/main/java/p/B.java"));
    assert!(!idx.language_extended("/r/b/src/main/java/q/C.java"), "no runtime API import in module b");
    // an extension class adds methods in every module
    let pf = ParsedFile::parse("package e; import manifold.ext.rt.api.Extension; @Extension public class MyStringExt {}");
    idx.set_file(Arc::from("/r/c/src/main/java/e/MyStringExt.java"), Arc::new(extract(&pf)));
    assert!(idx.language_extended("/r/b/src/main/java/q/C.java"));
}

fn simple_in(idx: &Index, class: &str, simple: &str) -> Option<String> {
    let scope = idx.scope_for_class(class).unwrap();
    idx.resolve_simple_type(simple, &scope).map(|n| n.to_string())
}

#[test]
fn a_private_member_type_of_a_superclass_is_not_inherited() {
    // JLS 8.2, 8.5: `Value` in r.T is the imported one, not q.Base's private class
    let idx = index_of(&[
        ("q/Base.java", "package q; public class Base { private static class Value { } protected static class Kept { } }"),
        ("lib/Value.java", "package lib; public class Value { }"),
        ("r/T.java", "package r; import lib.Value; import q.Base; class T extends Base { }"),
    ]);
    assert_eq!(simple_in(&idx, "r.T", "Value").as_deref(), Some("lib.Value"));
    assert_eq!(simple_in(&idx, "r.T", "Kept").as_deref(), Some("q.Base$Kept"));
    assert_eq!(simple_in(&idx, "q.Base", "Value").as_deref(), Some("q.Base$Value"));
}

#[test]
fn an_on_demand_import_imports_only_accessible_classes() {
    // JLS 7.5.2: a.Node is package-private, so b sees no `Node` from `a.*`
    let idx = index_of(&[
        ("a/Graph.java", "package a; public class Graph { } class Node { }"),
        ("a/Edge.java", "package a; public class Edge { }"),
        ("a/Other.java", "package a; class Other { }"),
        ("b/Viewer.java", "package b; import a.*; import lib.graph.*; class Viewer { }"),
        ("a/Same.java", "package a; import a.*; class Same { }"),
    ]);
    assert_eq!(simple_in(&idx, "b.Viewer", "Node"), None);
    assert_eq!(simple_in(&idx, "b.Viewer", "Edge").as_deref(), Some("a.Edge"));
    assert_eq!(simple_in(&idx, "a.Same", "Other").as_deref(), Some("a.Other"));
}

#[test]
fn a_package_private_jdk_supertype_is_walked_but_not_named() {
    if grounds_classfile::find_java_home().is_none() {
        return;
    }
    let idx = index_of(&[(
        "p/A.java",
        "package p; import java.lang.AbstractStringBuilder; import java.util.*;
         class A { }",
    )]);
    // StringBuilder's superclass is package-private: its hierarchy is complete
    // and lookups on it see the members it inherits, with nothing in doubt
    assert!(idx.hierarchy_complete("java.lang.StringBuilder"));
    assert!(idx.hierarchy_complete("java.lang.StringBuffer"));
    assert!(idx.is_subclass("java.lang.StringBuilder", "java.lang.AbstractStringBuilder").is_yes());
    let (owners, d) = crate::doubt::scope(|| idx.methods_named(&Ty::class("java.lang.StringBuilder"), "setLength").into_iter().map(|m| m.owner.to_string()).collect::<Vec<_>>());
    assert_eq!(owners, vec!["java.lang.AbstractStringBuilder"]);
    assert!(d.is_empty(), "{d}");
    let ((), d) = crate::doubt::scope(|| {
        idx.methods_named(&Ty::class("java.lang.StringBuilder"), "append");
    });
    assert!(d.is_empty(), "{d}");
    // no name resolves to it: not the simple name java.lang brings in, nor a
    // single-type import, nor the qualified name; nor to another class that
    // is not public, by an on-demand import
    assert_eq!(simple_in(&idx, "p.A", "AbstractStringBuilder"), None);
    assert_eq!(idx.binary_name_for_dotted("java.lang.AbstractStringBuilder"), None);
    assert_eq!(simple_in(&idx, "p.A", "ImmutableCollections"), None);
    assert!(!idx.class_exists("java.lang.AbstractStringBuilder"));
    // the API of an older release, read from ct.sym, carries it the same way
    let cp17 = Classpath::build(grounds_classfile::find_java_home().as_deref(), 17, &[]);
    if cp17.class("java.lang.StringBuilder").is_some() {
        assert!(cp17.class("java.lang.AbstractStringBuilder").is_some());
        assert!(!cp17.contains("java.lang.AbstractStringBuilder"));
        assert!(cp17.contains("java.lang.StringBuilder"));
    }
}

#[test]
fn a_test_class_is_not_imported_into_main_sources_or_another_module() {
    let idx = index_of(&[
        ("m/src/test/java/a/Node.java", "package a; public class Node { }"),
        ("m/src/main/java/a/Graph.java", "package a; public class Graph { }"),
        ("m/src/main/java/b/Main.java", "package b; import a.*; class Main { }"),
        ("m/src/test/java/b/MainTest.java", "package b; import a.*; class MainTest { }"),
        ("n/src/test/java/c/OtherTest.java", "package c; import a.*; class OtherTest { }"),
    ]);
    assert_eq!(simple_in(&idx, "b.Main", "Node"), None);
    assert_eq!(simple_in(&idx, "b.MainTest", "Node").as_deref(), Some("a.Node"));
    assert_eq!(simple_in(&idx, "c.OtherTest", "Node"), None);
    assert_eq!(simple_in(&idx, "b.Main", "Graph").as_deref(), Some("a.Graph"));
}

#[test]
fn an_unresolved_supertype_may_declare_any_member_type() {
    let in_code = |idx: &Index, class: &str, simple: &str| {
        let mut scope = idx.scope_for_class(class).unwrap();
        scope.unknown_if_inherited = true;
        idx.resolve_simple_type(simple, &scope).map(|n| n.to_string())
    };
    // lib.DataSource is not on the classpath: `Attribute` may be its member type
    let idx = index_of(&[
        ("p/Attribute.java", "package p; enum Attribute { NAME }"),
        ("p/Ds.java", "package p; import lib.DataSource; import java.util.List; class Ds implements DataSource { static class Inner { } }"),
        ("p/Plain.java", "package p; class Plain { }"),
        ("p/Module.java", "package p; interface Module { }"),
        ("p/Sub.java", "package p; class Sub extends Ds { }"),
    ]);
    assert_eq!(in_code(&idx, "p.Ds", "Attribute"), None);
    assert_eq!(in_code(&idx, "p.Sub", "Attribute"), None, "through a project superclass too");
    // what is known stays known: member types, single-type imports, java.lang
    assert_eq!(in_code(&idx, "p.Ds", "Inner").as_deref(), Some("p.Ds$Inner"));
    assert_eq!(in_code(&idx, "p.Ds", "List").as_deref(), Some("java.util.List"));
    assert_eq!(in_code(&idx, "p.Ds", "String").as_deref(), Some("java.lang.String"));
    // a same-package class would shadow java.lang's
    assert_eq!(in_code(&idx, "p.Ds", "Module"), None);
    assert_eq!(in_code(&idx, "p.Plain", "Attribute").as_deref(), Some("p.Attribute"));
    // declarations lowered for the whole project keep the package's class
    assert_eq!(simple_in(&idx, "p.Ds", "Attribute").as_deref(), Some("p.Attribute"));
}

#[test]
fn an_inherited_member_type_comes_before_the_class_own_name() {
    let idx = index_of(&[
        ("P.java", "public class P { public static class Q extends P { public static class B { } } }"),
        ("S.java", "public class S extends P { public static class Q extends P { public static class B extends Q.B { } } }"),
    ]);
    assert_eq!(simple_in(&idx, "S$Q", "Q").as_deref(), Some("P$Q"));
    let b = idx.class("S$Q$B").unwrap();
    assert_eq!(b.superclass.as_ref().unwrap().to_string(), "P.Q.B");
    assert_eq!(simple_in(&idx, "S", "S").as_deref(), Some("S"));
}

#[test]
fn inherited_override_equivalent_methods_keep_the_one_a_call_selects() {
    let idx = index_of(&[(
        "p/T.java",
        "package p;
         interface Model { Model setPrefix(String p); }
         interface Prefixed<R> { R setPrefix(String p); }
         interface Ont extends Model, Prefixed<Ont> { }
         interface Named { CharSequence name(); }
         interface Titled { Comparable<String> name(); }
         interface Both extends Named, Titled { }
         interface Data { }
         interface Manager { Data data(); }
         class Cache implements Data { }
         abstract class AbstractManager implements Manager { public Cache data() { return null; } }
         abstract class Holder<H> extends AbstractManager { }
         class User extends Holder<String> implements Manager { }
         interface Impl { private Object help() { return null; } }
         interface Api { String help(); }
         interface Helped extends Impl, Api { }",
    )]);
    let ret = |cls: &str, m: &str| {
        let ms = idx.methods_named(&Ty::class(cls), m);
        assert_eq!(ms.len(), 1, "{cls}.{m}: {ms:?}");
        ms[0].method.ret.to_string()
    };
    assert_eq!(ret("p.Ont", "setPrefix"), "p.Ont");
    assert_eq!(ret("p.User", "data"), "p.Cache");
    // neither return type is a subtype of the other: unknown
    assert_eq!(ret("p.Both", "name"), "?unknown");
    // the abstract method is still the one found once implemented
    assert_eq!(idx.all_methods(&Ty::class("p.User")).iter().filter(|m| &*m.method.name == "data").count(), 1);
    // a private interface method is not inherited, and is no rival
    let ms = idx.methods_named(&Ty::class("p.Helped"), "help");
    assert!(ms.iter().any(|m| &*m.owner == "p.Api" && m.method.ret.to_string() == "java.lang.String"), "{ms:?}");
}

#[test]
fn a_wildcard_for_a_parameter_with_several_bounds_is_unknown() {
    // JLS 4.9: the capture is an intersection, which one type cannot stand for
    let idx = index_of(&[(
        "p/N.java",
        "package p; abstract class Root { } interface Code { }
         class Nodes<N extends Root & Code> { N get() { return null; } }
         class One<N extends Root> { N get() { return null; } }",
    )]);
    let wild = |c: &str| Ty::class_with(c, vec![Ty::Wildcard(None)]);
    assert_eq!(idx.methods_named(&wild("p.Nodes"), "get")[0].method.ret, Ty::Unknown);
    assert_eq!(idx.methods_named(&wild("p.One"), "get")[0].method.ret.to_string(), "p.Root");
    let root = Ty::Wildcard(Some(Box::new(WildcardBound::Super(Ty::class("p.Root")))));
    assert!(idx.methods_named(&Ty::class_with("p.Nodes", vec![root]), "get")[0].method.ret.is_known());
}

#[test]
fn lombok_required_fields_follow_its_non_null_annotations() {
    let idx = index_of(&[(
        "q/R.java",
        "package q; import lombok.*; import javax.annotation.*; import com.example.NotNull;
         @RequiredArgsConstructor class R { @Nonnull String a; @NotNull String b; @javax.annotation.Nonnull String c; String d; }
         @AllArgsConstructor class S { String a; String $b; }",
    )]);
    let ctor = |c: &str| idx.class(c).unwrap().methods_named("<init>").map(|m| m.params.len()).collect::<Vec<_>>();
    // com.example.NotNull is not one Lombok knows
    assert_eq!(ctor("q.R"), vec![2]);
    assert_eq!(ctor("q.S"), vec![1]);
}

#[test]
fn whether_an_interface_is_functional_is_unknown_when_a_supertype_is() {
    let idx = index_of(&[(
        "p/F.java",
        "package p;
         interface F { void f(); }
         interface Two { void a(); void b(); }
         interface Sub extends F { default void g() {} }
         interface OnlyInherited extends q.Gone {}
         interface OneMore extends q.Gone { void run(); }
         class C implements F { public void f() {} }",
    )]);
    let what = |n: &str| match idx.functional_method(&Ty::class(n)) {
        Functional::Method(m) => format!("method {}", m.method.name),
        Functional::Not => "not".to_string(),
        Functional::Unknown => "unknown".to_string(),
    };
    assert_eq!(what("p.F"), "method f");
    assert_eq!(what("p.Sub"), "method f");
    assert_eq!(what("p.Two"), "not");
    assert_eq!(what("p.C"), "not");
    // the unresolved supertype may declare abstract methods, or none
    assert_eq!(what("p.OnlyInherited"), "unknown");
    assert_eq!(what("p.OneMore"), "unknown");
    assert_eq!(what("p.Missing"), "unknown");
    assert!(matches!(idx.functional_method(&Ty::Unknown), Functional::Unknown));
    // abstract methods of different names are two whatever their parameter types
    assert!(matches!(idx.functional_method(&Ty::class_with("java.util.Collection", vec![Ty::Unknown])), Functional::Not));
    assert!(matches!(idx.functional_method(&Ty::int()), Functional::Not));
}

#[test]
fn members_and_supertypes_of_a_raw_type_are_erased() {
    let idx = index_of(&[(
        "p/A.java",
        "package p; import java.util.*;
         public class A<T extends Number> extends ArrayList<T> {
           public T first; public List<T> all() { return null; } public <U> U as(Class<U> c) { return null; }
           public static List<String> names() { return null; } public static final List<String> NAMES = null;
         }
         class B extends A<Integer> {}",
    )]);
    let ret = |t: &Ty, m: &str| idx.methods_named(t, m)[0].method.ret.to_string();
    let raw = Ty::class("p.A");
    assert_eq!(idx.field(&raw, "first").unwrap().field.ty.to_string(), "java.lang.Number");
    assert_eq!(ret(&raw, "all"), "java.util.List");
    assert_eq!(ret(&raw, "as"), "java.lang.Object");
    assert!(idx.methods_named(&raw, "as")[0].method.type_params.is_empty());
    assert_eq!(ret(&raw, "get"), "java.lang.Object");
    assert_eq!(idx.direct_supertypes(&raw)[0].to_string(), "java.util.ArrayList");
    // static members keep their types
    assert_eq!(ret(&raw, "names"), "java.util.List<java.lang.String>");
    assert_eq!(idx.field(&raw, "NAMES").unwrap().field.ty.to_string(), "java.util.List<java.lang.String>");
    // a class that declares no type parameters is not raw
    let b = Ty::class("p.B");
    assert_eq!(idx.field(&b, "first").unwrap().field.ty.to_string(), "java.lang.Integer");
    assert_eq!(ret(&b, "all"), "java.util.List<java.lang.Integer>");
    assert_eq!(ret(&b, "get"), "java.lang.Integer");
}

#[test]
fn two_unknown_types_are_not_known_to_be_the_same() {
    let list = |a: Ty| Ty::class_with("java.util.List", vec![a]);
    assert_eq!(Ty::Unknown.same(&Ty::Unknown), Tri::Unknown);
    assert_eq!(list(Ty::Unknown).same(&list(Ty::Unknown)), Tri::Unknown);
    assert_eq!(list(Ty::Unknown).same(&list(Ty::string())), Tri::Unknown);
    assert_eq!(list(Ty::string()).same(&list(Ty::string())), Tri::Yes);
    // another class, shape or number of arguments differs whatever is unknown
    assert_eq!(list(Ty::Unknown).same(&Ty::class_with("java.util.Set", vec![Ty::Unknown])), Tri::No);
    assert_eq!(list(Ty::Unknown).same(&Ty::class("java.util.List")), Tri::No);
    assert_eq!(Ty::Array(Box::new(Ty::Unknown)).same(&Ty::int()), Tri::No);
    assert_eq!(Ty::Wildcard(Some(Box::new(WildcardBound::Extends(Ty::object())))).same(&Ty::Wildcard(None)), Tri::Yes);
    // a parameter type with an unknown argument is not a parameter that is
    // known to be another, but its erasure may be: `take(List<Gone>)`
    // overrides `I<Gone>.take(List<T>)` (equal erasures, JLS 8.4.2)
    let idx = index_of(&[(
        "p/C.java",
        "package p; import java.util.List;
         interface I<T> { void take(List<T> l); }
         abstract class C implements I<Gone> { public void take(List<Gone> l) {} }",
    )]);
    let takes: Vec<String> = idx.all_methods(&Ty::class("p.C")).into_iter().filter(|m| &*m.method.name == "take").map(|m| m.owner.to_string()).collect();
    assert_eq!(takes, vec!["p.C"]);
    // whether a release's JDK has a method whose parameter type is unknown
    // is unknown, not "no"
    let cp = &idx.classpath;
    let release = cp.exact_jdk_release().unwrap_or(21);
    assert_eq!(cp.jdk_has_method(release, "java.lang.String", "indexOf", &[Ty::string()]), Some(true));
    assert_eq!(cp.jdk_has_method(release, "java.lang.String", "indexOf", &[Ty::Unknown]), None);
    assert_eq!(cp.jdk_has_method(release, "java.lang.String", "indexOf", &[Ty::Unknown, Ty::Unknown, Ty::Unknown, Ty::Unknown]), Some(false));
}
