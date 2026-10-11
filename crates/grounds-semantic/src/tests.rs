use crate::*;
use grounds_index::*;
use grounds_syntax::{NodeExt, ParsedFile};
use std::sync::{Arc, OnceLock};

fn jdk() -> Arc<Classpath> {
    static CP: OnceLock<Arc<Classpath>> = OnceLock::new();
    CP.get_or_init(|| Arc::new(Classpath::build(grounds_classfile::find_java_home().as_deref(), 21, &[]))).clone()
}

/// Returns the types of all expressions marked with /*T*/ before them.
fn types_of(src: &str) -> Vec<String> {
    let pf = ParsedFile::parse(src);
    let mut idx = Index::new(jdk());
    idx.set_file(Arc::from("T.java"), Arc::new(extract(&pf)));
    let fm = FileModel::new(&pf, &idx, Arc::from("T.java"));
    let mut out = Vec::new();
    let marks: Vec<usize> = src.match_indices("/*T*/").map(|(i, _)| i + 5).collect();
    for m in marks {
        // the largest expression node starting right after the marker
        let mut best = None;
        for n in pf.root().descendants() {
            let mut s = n.start_byte();
            while s > 0 && src.as_bytes()[s - 1] == b' ' && s - 1 >= m {
                s -= 1;
            }
            if n.start_byte() >= m && src[m..n.start_byte()].trim().is_empty() && n.is_named() {
                if best.is_none() {
                    best = Some(n);
                }
            }
        }
        let n = best.expect("node after marker");
        // climb to largest expression with same start
        let mut cur = n;
        while let Some(p) = cur.parent() {
            if p.start_byte() == cur.start_byte() && !matches!(p.kind(), "expression_statement" | "variable_declarator" | "local_variable_declaration" | "block" | "return_statement" | "argument_list") {
                cur = p;
            } else {
                break;
            }
        }
        out.push(fm.type_of(cur).to_string());
    }
    out
}

#[test]
fn literal_and_local_types() {
    let t = types_of(
        "class T { void m() { int a = 1; long b = 2L; var s = \"x\"; String[] arr = {}; 
           Object o = /*T*/ a + b; Object p = /*T*/ s.length(); Object q = /*T*/ arr[0]; Object r = /*T*/ (a > 1 ? 1 : 2.0);
           Object u = /*T*/ s + a; } }",
    );
    assert_eq!(t, vec!["long", "int", "java.lang.String", "double", "java.lang.String"]);
}

#[test]
fn generics_and_inference() {
    let t = types_of(
        "import java.util.*; import java.util.stream.*;
         class T { void m(List<String> xs, Map<String, List<Integer>> m) {
           Object a = /*T*/ xs.get(0);
           Object b = /*T*/ m.get(\"k\");
           Object c = /*T*/ xs.stream().map(x -> x.length()).collect(Collectors.toList());
           Object d = /*T*/ Collections.singletonList(1);
           for (String s : xs) { Object e = /*T*/ s.trim(); }
           Object f = /*T*/ Optional.of(\"a\").map(String::length).orElse(0);
           var g = new ArrayList<String>(); Object h = /*T*/ g.iterator().next();
           List<Long> ll = new ArrayList<>(); Object i = /*T*/ ll.get(0);
           Object j = /*T*/ m.entrySet().iterator().next().getValue();
         } }",
    );
    assert_eq!(t[0], "java.lang.String");
    assert_eq!(t[1], "java.util.List<java.lang.Integer>");
    assert_eq!(t[2], "java.util.List<java.lang.Integer>");
    assert_eq!(t[3], "java.util.List<java.lang.Integer>");
    assert_eq!(t[4], "java.lang.String");
    assert_eq!(t[5], "java.lang.Integer");
    assert_eq!(t[6], "java.lang.String");
    assert_eq!(t[7], "java.lang.Long");
    assert_eq!(t[8], "java.util.List<java.lang.Integer>");
}

#[test]
fn fields_this_and_inheritance() {
    let t = types_of(
        "import java.util.*;
         class Base { protected List<String> names; String name() { return null; } }
         class T extends Base { int count; class Inner { Object x = /*T*/ count; }
           void m() { Object a = /*T*/ names.size(); Object b = /*T*/ this.name(); Object c = /*T*/ super.name().isEmpty();
             Runnable r = new Runnable() { public void run() { Object d = /*T*/ names; } };
             Object e = /*T*/ T.this.count; } }",
    );
    assert_eq!(t, vec!["int", "int", "java.lang.String", "boolean", "java.util.List<java.lang.String>", "int"]);
}

#[test]
fn overloads() {
    let t = types_of(
        "class T { static int f(int x) { return 0; } static String f(Object o) { return null; } static long f(long l) { return 0; }
           void m(Integer boxed) { Object a = /*T*/ f(1); Object b = /*T*/ f(\"s\"); Object c = /*T*/ f(1L); Object d = /*T*/ f(boxed);
             Object e = /*T*/ String.valueOf(3); Object g = /*T*/ Math.max(1, 2L); } }",
    );
    assert_eq!(t, vec!["int", "java.lang.String", "long", "java.lang.String", "java.lang.String", "long"]);
}

#[test]
fn constant_folding_uses_java_integer_semantics() {
    let cases: &[(&str, Constant)] = &[
        ("2147483647 + 1", Constant::Int(-2147483648)),
        ("1 << 32", Constant::Int(1)),
        ("1 << 33", Constant::Int(2)),
        ("1L << 32", Constant::Long(1 << 32)),
        ("-1 >>> 28", Constant::Int(15)),
        ("-1L >>> 60", Constant::Long(15)),
        ("0xFFFFFFFF", Constant::Int(-1)),
        ("-2147483648", Constant::Int(-2147483648)),
        ("2147483647 + 1L", Constant::Long(2147483648)),
        ("65536 * 65536", Constant::Int(0)),
        ("(int) 3000000000L", Constant::Int(-1294967296)),
        ("'a' + 1", Constant::Int(98)),
        ("2147483647 + 1 > 0", Constant::Bool(false)),
        ("(true ? 2147483647 : 0L) + 1", Constant::Long(2147483648)),
    ];
    for (expr, want) in cases {
        let src = format!("class T {{ void m() {{ Object x = {expr}; }} }}");
        let pf = ParsedFile::parse(src.clone());
        let mut idx = Index::new(jdk());
        idx.set_file(Arc::from("T.java"), Arc::new(extract(&pf)));
        let fm = FileModel::new(&pf, &idx, Arc::from("T.java"));
        let d = pf.root().descendants().find(|n| n.kind() == "variable_declarator").unwrap();
        assert_eq!(fm.constant_value(d.field("value").unwrap()).as_ref(), Some(want), "{expr}");
    }
}

#[test]
fn constants() {
    let src = "class T { static final int A = 2; static final String S = \"a\" + A; void m() { final int b = A * 3; int x = b; String s = S + \"!\"; } }";
    let pf = ParsedFile::parse(src);
    let mut idx = Index::new(jdk());
    idx.set_file(Arc::from("T.java"), Arc::new(extract(&pf)));
    let fm = FileModel::new(&pf, &idx, Arc::from("T.java"));
    let decls: Vec<_> = pf.root().descendants().filter(|n| n.kind() == "variable_declarator").collect();
    let x = decls.iter().find(|d| d.field("name").unwrap().text(src) == "x").unwrap();
    assert_eq!(fm.constant_value(x.field("value").unwrap()), Some(Constant::Int(6)));
    let s = decls.iter().find(|d| d.field("name").unwrap().text(src) == "s").unwrap();
    assert_eq!(fm.string_value(s.field("value").unwrap()).as_deref(), Some("a2!"));
}

#[test]
fn matcher() {
    let src = "import java.util.*; class T { void m(ArrayList<String> l, String s) { l.add(\"x\"); s.equals(\"y\"); System.exit(1); } }";
    let pf = ParsedFile::parse(src);
    let mut idx = Index::new(jdk());
    idx.set_file(Arc::from("T.java"), Arc::new(extract(&pf)));
    let fm = FileModel::new(&pf, &idx, Arc::from("T.java"));
    let calls: Vec<_> = pf.root().descendants().filter(|n| n.kind() == "method_invocation").collect();
    const ADD: MethodMatcher = MethodMatcher::instance("java.util.Collection").named(&["add"]);
    const EQ: MethodMatcher = MethodMatcher::instance("java.lang.String").named(&["equals"]);
    const EXIT: MethodMatcher = MethodMatcher::statics("java.lang.System").named(&["exit"]);
    assert!(fm.call_matches(calls[0], &ADD));
    assert!(fm.call_matches(calls[1], &EQ));
    assert!(!fm.call_matches(calls[1], &ADD));
    assert!(fm.call_matches(calls[2], &EXIT));
}

#[test]
fn debug_stream() {
    let t = types_of(
        "import java.util.*; import java.util.stream.*;
         class T { void m(List<String> xs) {
           Object a = /*T*/ xs.stream();
           Object b = /*T*/ xs.stream().map(x -> x.length());
           Collector<Integer, ?, List<Integer>> c = /*T*/ Collectors.toList();
           Object d = /*T*/ xs.stream().map(x -> x.length()).collect(Collectors.toList());
         } }",
    );
    eprintln!("{:?}", t);
}

#[test]
fn debug_varargs() {
    let t = types_of(
        "import java.util.*;
         interface Builder { Builder environments(String... envs); Builder packages(String... p); static String join(Builder... bs) { return null; } }
         interface Arg<T> { static Class<?>[] toClassArray(Arg<?>... arguments) { return null; } }
         class T { void m(Builder b, Arg<?>[] arguments, byte[] bytes) {
           Object a = /*T*/ b.environments(\"x\");
           Object c = /*T*/ Arg.toClassArray(arguments);
           Object d = /*T*/ Arrays.copyOf(bytes, 3);
           Object e = /*T*/ Builder.join(b, b);
         } }",
    );
    eprintln!("{:?}", t);
}

#[test]
fn debug_stream_of() {
    let t = types_of(
        "import java.util.stream.*;
         class T { void m(String[] arr) {
           Object a = /*T*/ Stream.of(arr);
           Object b = /*T*/ Stream.of(\"a\", \"b\");
           Object c = /*T*/ Stream.of(\"a\");
         } }",
    );
    eprintln!("{:?}", t);
}

#[test]
fn debug_stream_of2() {
    let src = "import java.util.stream.*; class T { void m(String[] arr) { Stream.of(arr); } }";
    let pf = ParsedFile::parse(src);
    let mut idx = Index::new(jdk());
    idx.set_file(Arc::from("T.java"), Arc::new(extract(&pf)));
    let fm = FileModel::new(&pf, &idx, Arc::from("T.java"));
    let call = pf.root().descendants().find(|n| n.kind() == "method_invocation").unwrap();
    let ms = fm.methods_named(&Ty::class("java.util.stream.Stream"), "of");
    for m in &ms { eprintln!("cand {:?} {:?}", m.method.signature_string(), m.method.flags); }
    let s = fm.select_method(call).unwrap();
    eprintln!("selected {} amb={}", s.method.signature_string(), s.ambiguous);
}

#[test]
fn overloads_with_unknown_parameter_types_stay_ambiguous() {
    let src = "package app; import missing.Protos; class Msg {} class Event {} class P { static Msg fromProto(Protos.MsgProto p) { return null; } static Event fromProto(Protos.EventProto p) { return null; } void m(Protos.MsgProto p) { Object x = fromProto(p.getEvent()); } }";
    let pf = ParsedFile::parse(src);
    let mut idx = Index::new(jdk());
    idx.set_file(Arc::from("T.java"), Arc::new(extract(&pf)));
    let fm = FileModel::new(&pf, &idx, Arc::from("T.java"));
    let call = pf.root().descendants().filter(|n| n.kind() == "method_invocation").find(|n| n.field("name").unwrap().text(src) == "fromProto").unwrap();
    assert!(fm.select_method(call).unwrap().ambiguous);
    assert_eq!(fm.type_of(call), Ty::Unknown);
}

#[test]
fn qualified_record_patterns() {
    let t = types_of(
        "sealed interface Shape permits Shape.Circle, Shape.Group {
           record Circle(double radius) implements Shape {}
           record Group(java.util.List<Shape> members, String name) implements Shape {} }
         class T {
           Object a(Shape s) { switch (s) {
             case Shape.Circle(double r) -> { return /*T*/ r; }
             case Shape.Group(var ms, var name) when /*T*/ name.isEmpty() -> { return /*T*/ ms; }
             default -> { return null; } } }
           Object b(Object o) {
             if (o instanceof Shape.Circle(var r)) return /*T*/ r;
             if (o instanceof Shape.Group(Shape.Circle(var x), var n)) return /*T*/ x;
             switch (o) { case Shape.Circle c: return /*T*/ c; default: return null; } } }",
    );
    assert_eq!(t, vec!["double", "boolean", "java.util.List<Shape>", "double", "double", "Shape.Circle"]);
}

#[test]
fn nested_generic_overloads_stay_polynomial() {
    // Hamcrest-style matcher trees: every level is an overloaded generic call
    // whose argument is the next level. Typing used to redo each subtree per
    // overload candidate and per target-type query (helidon's CmModelTest).
    let leaf = "prop(Rec::name, is(\"a\"))";
    let mut e = leaf.to_string();
    for i in 0..14 {
        e = if i % 2 == 0 { format!("hasItem(allOf({e}, {leaf}))") } else { format!("is(allOf(java.util.List.of({e}, {leaf})))") };
    }
    let src = format!(
        "import java.util.function.Function; interface M<T> {{}} record Rec(String name) {{}}
         class T {{
           static <X> M<X> is(X x) {{ return null; }}
           static <X> M<X> is(M<X> m) {{ return null; }}
           static <X> M<X> allOf(Iterable<M<? super X>> ms) {{ return null; }}
           @SafeVarargs static <X> M<X> allOf(M<? super X>... ms) {{ return null; }}
           static <X> M<Iterable<? super X>> hasItem(X x) {{ return null; }}
           static <X> M<Iterable<? super X>> hasItem(M<? super X> m) {{ return null; }}
           static <A, B> M<A> prop(Function<A, B> f, M<B> m) {{ return null; }}
           static <X> void assertThat(X x, M<? super X> m) {{}}
           void t(java.util.List<Rec> l) {{ assertThat(l, {e}); }} }}"
    );
    let pf = ParsedFile::parse(src.as_str());
    let mut idx = Index::new(jdk());
    idx.set_file(Arc::from("T.java"), Arc::new(extract(&pf)));
    let fm = FileModel::new(&pf, &idx, Arc::from("T.java"));
    for n in pf.root().descendants().filter(|n| n.kind() == "method_invocation") {
        fm.type_of(n);
    }
    assert!(fm.work.get() < 20_000, "{} guarded computations", fm.work.get());
}

#[test]
fn module_imports_resolve_types() {
    let t = types_of(
        "import module java.base;
         class T { void m() { Object a = /*T*/ new ArrayList<String>(); Object b = /*T*/ Path.of(\"x\"); } }",
    );
    assert_eq!(t, vec!["java.util.ArrayList<java.lang.String>", "java.nio.file.Path"]);
}

#[test]
fn module_imports_include_transitive_exports() {
    let t = types_of(
        "import module java.sql;
         class T { void m() { Object a = /*T*/ java.util.logging.Logger.getLogger(\"x\"); Object b = /*T*/ Logger.getLogger(\"x\"); } }",
    );
    assert_eq!(t, vec!["java.util.logging.Logger", "java.util.logging.Logger"]);
}

#[test]
fn ambiguous_module_imported_types_stay_unresolved() {
    let t = types_of(
        "import module java.desktop;
         import java.util.*;
         class T { void m(Element e, List<String> l) { Object a = /*T*/ e; Object b = /*T*/ l; } }",
    );
    // Element is exported by javax.swing.text and javax.swing.text.html.parser
    // (and others): javac rejects it as ambiguous; the file's own java.util.*
    // shadows java.awt.List
    assert_ne!(t[0], "javax.swing.text.Element");
    assert_eq!(t[1], "java.util.List<java.lang.String>");
}

#[test]
fn assertj_style_generic_overloads_pick_the_specific_one() {
    // AssertJ's Assertions mixes exact overloads with generic ones bounded by
    // Comparable or AssertDelegateTarget; the exact match must win, and the
    // result must not be flagged as a guess
    let t = types_of(
        "import java.util.*;
         interface AssertProvider<A> { A assertThat(); } interface AssertDelegateTarget {}
         class StrA {} class ObjA<T> {} class CmpA<T> {} class FArr {} class OArr<E> {} class ListA<E> {} class CharA {}
         abstract class Chars implements CharSequence {}
         record R(int x) implements Comparable<R> { public int compareTo(R o) { return 0; } }
         class A {
           static StrA assertThat(String s) { return null; }
           static CharA assertThat(CharSequence c) { return null; }
           static <T> ObjA<T> assertThat(T t) { return null; }
           static <T extends Comparable<? super T>> CmpA<T> assertThat(T t) { return null; }
           static <T extends AssertDelegateTarget> T assertThat(T t) { return null; }
           static <T> T assertThat(AssertProvider<T> p) { return null; }
           static FArr assertThat(float[] a) { return null; }
           static <E> OArr<E> assertThat(E[] a) { return null; }
           static <E> OArr<E[]> assertThat(E[][] a) { return null; }
           static <E> ListA<E> assertThat(List<? extends E> l) { return null; }
           static <E> ObjA<E> assertThat(Iterable<? extends E> l) { return null; }
           static <E> ObjA<E> assertThat(Collection<? extends E> l) { return null; }
           void m(List<String> l, Object o, R r, Chars sb) {
             Object a = /*T*/ assertThat(\"x\");
             Object b = /*T*/ assertThat(new float[0]);
             Object c = /*T*/ assertThat(new String[0]);
             Object d = /*T*/ assertThat(l);
             Object e = /*T*/ assertThat(o);
             Object f = /*T*/ assertThat(r);
             Object g = /*T*/ assertThat(sb);
             Object h = /*T*/ assertThat(new String[0][0]);
           } }",
    );
    // `Comparable<? super T>` needs inference to confirm: unknown, not a guess
    assert_eq!(t, vec!["StrA", "FArr", "OArr<java.lang.String>", "ListA<java.lang.String>", "ObjA<java.lang.Object>", "?unknown", "CharA", "OArr<java.lang.String[]>"]);
}

#[test]
fn generic_overloads_never_claim_a_pick_javac_would_not_make() {
    // erased bounds can rule a generic candidate out, not in: either javac's
    // pick or unknown, never a confident wrong type
    let t = types_of(
        "interface I {} class D implements I {} class Base {} class A extends Base {} class B extends Base {}
         class Bad implements Comparable<String> { public int compareTo(String s) { return 0; } }
         class P1 { static <T extends Comparable<? super T>> Integer pick(T x) { return 1; } static String pick(Object x) { return null; } }
         class P2 { static <T> T pick(T a, T b) { return a; } static String pick(Object... xs) { return null; } }
         class P3 { static <U, T extends U> Integer pick(Class<U> c, T x) { return 1; } static String pick(Object c, Object x) { return null; } }
         class P4 { static <T extends Number> Integer pick(T[] x) { return 1; } static String pick(java.io.Serializable[] x) { return null; } }
         class P5 { static <T extends CharSequence> Integer pick(T x) { return 1; } static String pick(Object x) { return null; } }
         class P6 { static <T extends Object & I> Integer pick(T x) { return 1; } static String pick(I x) { return null; } }
         class T {
           void m() {
             Object a = /*T*/ P1.pick(new Bad());
             Object b = /*T*/ P2.pick(new A(), new B());
             Object c = /*T*/ P3.pick(String.class, Integer.valueOf(1));
             Object d = /*T*/ P4.pick(new Integer[0]);
             Object e = /*T*/ P5.pick(1);
             Object f = /*T*/ P6.pick(new D());
           } }",
    );
    // P4: javac picks the generic one, but proving it needs the bound: unknown
    assert_eq!(t, vec!["?unknown", "?unknown", "?unknown", "?unknown", "java.lang.String", "?unknown"]);
}

#[test]
fn generic_overloads_stay_unknown_where_inference_decides() {
    let t = types_of(
        "import java.util.*; class Base {} class A extends Base {} class B extends Base {}
         class Q1 { static <U> Integer pick(U x, List<String> y) { return 1; } static String pick(Object x, Object y) { return null; } }
         class Q2 { static <E extends Number> Integer pick(List<E>[] x) { return 1; } static String pick(Object x) { return null; } }
         class Q3 { static <E> Integer pick(E x) { return 1; } static String pick(java.io.Serializable x) { return null; } }
         class Q4 { static <T extends Base, U extends T> T pick(T a, List<U> b) { return a; } static String pick(Object a, Object b) { return null; } }
         class Q5 { static <T extends Number> Integer pick(T x, T y) { return 1; } static <T> T pick(T x, T y) { return x; } }
         class Q6 { static <T extends List<String>> Integer pick(T x) { return 1; } static String pick(ArrayList<?> x) { return null; } }
         class Q7 { static <T> T get(Class<T> c) { return null; } static Object get(String name) { return null; } }
         class T {
           void m(List<Integer> xs, List<String>[] ls, List<B> bs, ArrayList<String> al) {
             Object a = /*T*/ Q1.pick(new Object(), xs);
             Object b = /*T*/ Q2.pick(ls);
             Object c = /*T*/ Q3.<Double>pick(Integer.valueOf(1));
             Object d = /*T*/ Q4.pick(new A(), bs);
             Object e = /*T*/ Q5.pick(new A(), new B());
             Object f = /*T*/ Q6.pick(al);
             Object g = /*T*/ Q7.get(String.class);
           } }",
    );
    assert_eq!(t, vec!["?unknown", "?unknown", "?unknown", "?unknown", "?unknown", "?unknown", "java.lang.String"]);
}

#[test]
fn generic_overloads_check_fixed_type_arguments_and_wildcards() {
    let t = types_of(
        "import java.util.*;
         class R1 { static <U, T> Integer pick(U x, Map<String, T> y) { return 1; } static String pick(Object x, Object y) { return null; } }
         class R2 { static <U> Integer pick(U x, List<String>[] y) { return 1; } static String pick(Object x, Object y) { return null; } }
         class R3 { static <T> Integer pick(List<T>[] x) { return 1; } static String pick(Object x) { return null; } }
         class R4 { static <U, T> T pick(U u, List<T> x) { return null; } static String pick(Object u, Object x) { return null; } }
         class R6 { static <T> Integer pick(T x, List<String>[] y) { return 1; } static String pick(String x, List<Integer>[] y) { return null; } }
         class R7 { static <E> List<E> wrap(List<? extends E> l) { return null; } static String wrap(Object o) { return null; } }
         class T {
           void m(Map<Integer, Double> md, List<Integer>[] li, List<?>[] lw, List<? super Integer> ls, List<String> plain) {
             Object a = /*T*/ R1.pick(new Object(), md);
             Object b = /*T*/ R2.pick(new Object(), li);
             Object c = /*T*/ R3.pick(lw);
             Object d = /*T*/ R4.pick(new Object(), ls);
             Object f = /*T*/ R6.pick(\"\", null);
             Object g = /*T*/ R7.wrap(plain);
           } }",
    );
    assert_eq!(t, vec!["?unknown", "?unknown", "?unknown", "?unknown", "?unknown", "java.util.List<java.lang.String>"]);
}

#[test]
fn generic_overloads_stay_unknown_with_lambdas_and_wildcards() {
    let t = types_of(
        "import java.util.*; import java.util.function.*;
         class S1 { static <U> Integer pick(U c, Collection<Integer> xs) { return 1; } static String pick(Object c, Object xs) { return null; } }
         class S2 { static Integer pick(String k, Consumer<Integer> a) { return 1; } static <K, V> String pick(K k, Consumer<V> a) { return null; } }
         class S3 { static <T> Integer pick(T[] xs, Supplier<String> s) { return 1; } static String pick(Object xs, Supplier<Integer> s) { return null; } }
         class S4 { static <K, V> Integer pick(K k, List<V> v) { return 1; } static String pick(String k, List<?> v) { return null; } }
         class T {
           void m(List<? super Integer> ls, String[] xs, List<String> l) {
             Object a = /*T*/ S1.pick(new Object(), ls);
             Object b = /*T*/ S2.pick(\"\", (String s) -> {});
             Object c = /*T*/ S3.pick(xs, () -> 1);
             Object d = /*T*/ S4.pick(\"\", l);
           } }",
    );
    assert_eq!(t, vec!["?unknown", "?unknown", "?unknown", "?unknown"]);
}

#[test]
fn generic_overloads_through_raw_or_reshaped_supertypes() {
    let t = types_of(
        "import java.util.*;
         abstract class RawBase<A> implements Iterable<A> {} abstract class Raw<X> extends RawBase {}
         interface Pair<A, B> {} abstract class Swap<A, B> implements Pair<B, A> {}
         class U1 { static <K, V> Integer pick(K k, Iterable<V> x) { return 1; } static String pick(String k, Raw<String> x) { return null; } }
         class U2 { static <K> Integer pick(K k, Pair<String, ? extends Number> x) { return 1; } static String pick(String k, Swap<Integer, String> x) { return null; } }
         class T {
           void m() {
             Object a = /*T*/ U1.pick(\"\", null);
             Object b = /*T*/ U2.pick(\"\", null);
           } }",
    );
    assert_eq!(t, vec!["?unknown", "java.lang.String"]);
}
