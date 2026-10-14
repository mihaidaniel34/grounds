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

/// The method each call marked with /*C*/ (the call starts right after the
/// marker) resolves to, with `?` when the pick is uncertain.
fn calls_of(src: &str) -> Vec<String> {
    let pf = ParsedFile::parse(src);
    let mut idx = Index::new(jdk());
    idx.set_file(Arc::from("T.java"), Arc::new(extract(&pf)));
    let fm = FileModel::new(&pf, &idx, Arc::from("T.java"));
    let marks: Vec<usize> = src.match_indices("/*C*/").map(|(i, _)| i + 5).collect();
    marks
        .into_iter()
        .map(|m| {
            let start = m + src[m..].len() - src[m..].trim_start().len();
            let call = pf.root().descendants().find(|n| n.start_byte() == start && matches!(n.kind(), "method_invocation" | "object_creation_expression")).expect("call after marker");
            match fm.resolve_call(call) {
                Some(rc) => {
                    let params: Vec<String> = rc.method.params.iter().map(|p| p.ty.erasure().to_string()).collect();
                    format!("{}.{}({}){}", simple_name_of(&rc.owner), rc.method.name, params.join(","), if rc.ambiguous { "?" } else { "" })
                }
                None => "none".to_string(),
            }
        })
        .collect()
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
    // P1: Bad is no `Comparable<? super Bad>`; P2: T is lub(A, B); P3: Integer is no String.
    // P4: javac picks the generic one, but proving it needs the bound: unknown
    assert_eq!(t, vec!["java.lang.String", "Base", "java.lang.String", "?unknown", "java.lang.String", "?unknown"]);
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
    // Q1, Q2: `List<Integer>` is no `List<String>`, String violates `E extends Number`;
    // Q5: lub(A, B) is Base, no Number
    assert_eq!(t, vec!["java.lang.String", "java.lang.String", "?unknown", "?unknown", "Base", "?unknown", "java.lang.String"]);
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
    // R4: T is the capture of `? super Integer`, whose upper bound is Object
    assert_eq!(t, vec!["java.lang.String", "java.lang.String", "java.lang.String", "java.lang.Object", "?unknown", "java.util.List<java.lang.String>"]);
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
    // S1: a `List<? super Integer>` is no `Collection<Integer>`; S3: `() -> 1` is no `Supplier<String>`
    assert_eq!(t, vec!["java.lang.String", "?unknown", "java.lang.String", "?unknown"]);
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

#[test]
fn overloads_check_type_arguments_and_inferred_bounds() {
    // a differential fuzz against javac found these picked by erasure
    let t = types_of(
        "import java.util.*;
         class P1 { static String pick(Object o) { return null; } static Integer pick(List<Integer> l) { return null; } }
         class P2 { static <T extends Number> Integer pick(Collection<T> c) { return null; } static String pick(Object o) { return null; } }
         class Poly { static Integer take(Map.Entry<String, Object> e) { return null; } static Integer take(List<String> l) { return null; } }
         class Box { static <T extends Comparable<? super T>> String f(T t) { return null; } static Integer f(double d) { return null; } }
         class T {
           void m(List<String> ls, Collection<String> cs) {
             Object a = /*T*/ P1.pick(ls);
             Object b = /*T*/ P2.pick(cs);
             Object c = /*T*/ Poly.take(Map.entry(\"a\", \"b\"));
             Object d = /*T*/ Box.f(1.5);
           } }",
    );
    // `List<String>` is no `List<Integer>`; String violates `T extends Number`;
    // a generic call as argument takes its type arguments from the parameter
    // (`MapEntry<String, String>` is not ruled out for `Entry<String, Object>`);
    // boxing for the generic one makes it phase 2
    assert_eq!(t, vec!["java.lang.String", "java.lang.String", "java.lang.Integer", "java.lang.Integer"]);
}

#[test]
fn unchecked_conversion_erases_the_result() {
    let t = types_of(
        "import java.util.*;
         class P3 { static Set<String> pick(Set<String> s) { return null; } static List<String> any(List<?> l) { return null; } }
         class T { void m(Set raw, List rawList) { Object a = /*T*/ P3.pick(raw); Object b = /*T*/ P3.any(rawList); } }",
    );
    // a raw argument for `List<?>` needs no unchecked conversion
    assert_eq!(t, vec!["java.util.Set", "java.util.List<java.lang.String>"]);
}

#[test]
fn captured_wildcards_stay_wildcards() {
    let t = types_of(
        "import java.util.*;
         class P4 { static <T> List<T> same(List<T> l) { return null; } static <T> T first(List<T> l) { return null; }
                    static <T> Iterable<T> wrap(Collection<? extends T> c) { return null; } }
         class T {
           void m(List<? super Integer> lsu, List<? extends Number> le, Collection<? extends Number> ce) {
             Object a = /*T*/ P4.same(lsu);
             Object b = /*T*/ P4.same(le);
             Object c = /*T*/ P4.first(le);
             Object d = /*T*/ P4.first(lsu);
             Object e = /*T*/ P4.wrap(ce);
           } }",
    );
    assert_eq!(
        t,
        vec![
            "java.util.List<? super java.lang.Integer>",
            "java.util.List<? extends java.lang.Number>",
            "java.lang.Number",
            "java.lang.Object",
            "java.lang.Iterable<? extends java.lang.Number>"
        ]
    );
}

#[test]
fn inference_follows_javac_resolution() {
    let t = types_of(
        "import java.util.*;
         class A implements Comparable<A> { public int compareTo(A o) { return 0; } } class B extends A {}
         class Base {} class X extends Base {} class Y extends Base {}
         class F { static <T extends Comparable<T>> T max(T a, T b) { return a; } static <T> T id(T t) { return t; }
                   static <T> T either(T a, T b) { return a; } }
         class T {
           void m(B b1, B b2) {
             Object a = /*T*/ F.max(b1, b2);
             Object b = /*T*/ F.<Number>id(1);
             Object c = /*T*/ Collections.<Number>singletonList(1);
             Object d = /*T*/ F.either(new X(), new Y());
             Object e = /*T*/ F.either(1, \"s\");
           } }",
    );
    // `T extends Comparable<T>` over a B makes T the A that B compares as;
    // explicit type arguments win; T is the lub, unknown where that is an intersection
    assert_eq!(t, vec!["A", "java.lang.Number", "java.util.List<java.lang.Number>", "Base", "?unknown"]);
}

#[test]
fn calls_javac_rejects_have_no_certain_type() {
    let t = types_of(
        "import java.util.*;
         class P5 { static Integer pick(List<Integer> l) { return null; } }
         class P6 { static Object pick(Iterable<String> a) { return null; } static Object pick(List<? extends Number> b) { return null; } }
         class P7 { static <T extends Number> Integer pick(List<T> l) { return null; } }
         class P8 { static <T> Integer pick(Iterable<T> a, Comparable<T> b) { return null; } }
         class P9 { static long pick(java.util.function.Supplier<String> s) { return 0L; } }
         class T {
           void m(List<String> ls, List<?> lw, Iterable<String> it, Comparable<?> cmp) {
             Object a = /*T*/ P5.pick(ls);
             Object b = /*T*/ P6.pick(null);
             Object c = /*T*/ P7.pick(lw);
             Object d = /*T*/ P8.pick(it, cmp);
             Object e = /*T*/ P9.pick(() -> 1);
           } }",
    );
    // the only candidate does not fit; ambiguous with null; the capture of `?`
    // is no Number; a capture equals no String; `() -> 1` supplies no String
    assert_eq!(t, vec!["?unknown"; 5]);
}

#[test]
fn poly_arguments_never_leave_a_conflicting_binding() {
    // javac infers a generic call as argument with the parameter as target;
    // where that is needed to settle a variable, the result is unknown
    let t = types_of(
        "import java.util.*;
         class R { static <T> T choose(T fallback, List<T> values) { return fallback; } static <T> T same(List<T> a, List<T> b) { return null; }
                   static <X> List<X> identity(List<X> xs) { return xs; } }
         class Q { static <X> String pick(X fallback, List<X> values) { return null; } static Integer pick(Object fallback, Collection<?> values, Object... rest) { return null; } }
         class T {
           void m(Object obj, List<String> ls) {
             Object a = /*T*/ R.choose(obj, List.of(\"x\"));
             Object b = /*T*/ R.same(ls, List.of(1));
             Object c = /*T*/ Q.pick(obj, R.identity(ls));
             Object d = /*T*/ R.choose(obj, R.identity(ls));
           } }",
    );
    // javac: Object, rejected, Integer (X = String cannot take the Object), rejected
    assert_eq!(t, vec!["?unknown"; 4]);
}

#[test]
fn lambda_bodies_are_typed_against_the_functional_result() {
    // a generic call in the body is inferred with the functional result as
    // target, its own arguments and bounds: only its erasure rules anything out
    let t = types_of(
        "import java.util.*; import java.util.function.*;
         interface General { List<?> get(); }
         interface Specific extends General { List<Object> get(); }
         class R { static String pick(Specific s) { return null; } static Integer pick(General s) { return null; }
                   static Integer one(Supplier<List<Object>> s) { return null; }
                   static <X> List<X> identity(List<X> xs) { return xs; } static <X extends Number> List<X> make() { return null; } }
         class T {
           void m(List<String> xs) {
             Object a = /*T*/ R.pick(() -> List.of(\"x\"));
             Object b = /*T*/ R.one(() -> List.of(\"x\"));
             Object c = /*T*/ R.pick(() -> R.identity(xs));
             Object d = /*T*/ R.pick(() -> R.make());
           } }",
    );
    // javac: String, Integer, Integer, Integer
    assert_eq!(t, vec!["?unknown", "java.lang.Integer", "?unknown", "?unknown"]);
}

#[test]
fn captures_with_a_common_bound_infer_that_bound() {
    let t = types_of(
        "import java.util.*; import java.util.stream.*;
         class Base {} class A extends Base {} class B extends Base {}
         class T {
           void m(Stream<? extends Number> a, Stream<? extends Number> b, Stream<?> c, Stream<?> d) {
             Object x = /*T*/ Stream.concat(a, b).toList();
             Object y = /*T*/ Stream.concat(c, d).toList();
             Object z = /*T*/ Stream.concat(a, b);
             Stream<? extends Number> u = /*T*/ Stream.concat(a, b);
             Stream<Object> v = /*T*/ Stream.concat(a, b);
             Stream<Object> w = /*T*/ Stream.of(new A(), new B());
           } }",
    );
    // the lub, unless the target forces another type (javac: Stream<Object>)
    assert_eq!(
        t,
        vec![
            "java.util.List<java.lang.Number>",
            "java.util.List<java.lang.Object>",
            "java.util.stream.Stream<java.lang.Number>",
            "java.util.stream.Stream<java.lang.Number>",
            "java.util.stream.Stream<?unknown>",
            "java.util.stream.Stream<?unknown>"
        ]
    );
}

#[test]
fn target_typed_results_rule_no_overload_in_or_out() {
    // `first(xs)` returns its own X: alone it says Object, with a target
    // anything below it; neither as lambda body nor as argument it decides
    let t = types_of(
        "import java.util.*; import java.util.function.*;
         interface Specific extends Supplier<String> {}
         class R { static <X> X first(List<? super X> xs) { return null; }
                   static String pick(Specific s) { return null; } static <Y> Integer pick(Supplier<Y> s) { return null; }
                   static Long take(String s) { return null; } static Double take(List<?> l) { return null; } }
         class T { void m(List<Object> xs) { Object a = /*T*/ R.pick(() -> R.first(xs)); Object b = /*T*/ R.take(R.first(xs)); } }",
    );
    // javac: String, ambiguous
    assert_eq!(t, vec!["?unknown", "?unknown"]);
}

#[test]
fn poly_mismatch_with_a_fixed_parameter_is_no_proof() {
    let t = types_of(
        "import java.util.*;
         class Q { static <X> List<X> identity(List<X> xs) { return xs; }
                   static String pick(List<Object> xs) { return null; } static Integer pick(Object xs, Object... rest) { return null; } }
         class T { void m(List<String> xs) { Object a = /*T*/ Q.pick(Q.identity(xs)); } }",
    );
    // javac: Integer (X = String cannot give a List<Object>)
    assert_eq!(t, vec!["?unknown"]);
}

#[test]
fn capture_projection_yields_to_an_invariant_target() {
    let t = types_of(
        "import java.util.*;
         class T { void m(List<? extends Number> xs) {
             List<Number> a = /*T*/ List.copyOf(xs);
             Object b = /*T*/ List.copyOf(xs);
             Collection<? extends Number> c = /*T*/ List.copyOf(xs);
         } }",
    );
    // javac: List<Number> for the invariant target, the projection otherwise
    assert_eq!(
        t,
        vec!["java.util.List<?unknown>", "java.util.List<? extends java.lang.Number>", "java.util.List<? extends java.lang.Number>"]
    );
}

#[test]
fn rivals_in_later_phases_keep_an_unproven_pick_uncertain() {
    let t = types_of(
        "import java.util.*;
         class P { static <T> T id(T x) { return x; } static <T> T any() { return null; }
                   static String pick(String x) { return null; } static Integer pick(Integer x, Object... rest) { return null; } }
         class T { void m() { Object a = /*T*/ P.pick(P.id(1)); Object b = /*T*/ P.pick(P.any()); } }",
    );
    // `id(1)` is an Integer whatever the target, so only the varargs one fits;
    // `any()` fits the fixed-arity one only through its target: javac Integer, String
    assert_eq!(t, vec!["java.lang.Integer", "?unknown"]);
}

#[test]
fn erasure_only_results_drop_type_arguments_inside_arrays() {
    let t = types_of(
        "import java.util.*;
         class Q { static <X> List<X> identity(List<X> xs) { return xs; }
                   static List<String>[] pick(List<String> xs) { return null; } static <Y> List<Y>[] pick(Collection<Y> xs) { return null; } }
         class T { void m(List<Integer> xs) { Object a = /*T*/ Q.pick(Q.identity(xs)); } }",
    );
    // javac: List<Integer>[]
    assert_eq!(t, vec!["java.util.List<?unknown>[]"]);
}

#[test]
fn calls_fixed_by_their_own_arguments_keep_their_type() {
    let t = types_of(
        "import java.util.*;
         class T { void m(String[] xs) { Object a = /*T*/ List.of(Arrays.copyOf(xs, 2)); Object b = /*T*/ List.of(Objects.requireNonNull(xs)); } }",
    );
    assert_eq!(t, vec!["java.util.List<java.lang.String>"; 2]);
}

#[test]
fn unchecked_conversion_erases_the_instantiated_result() {
    let t = types_of(
        "import java.util.*; import java.util.function.*;
         class Raw<X> extends ArrayList {}
         class P { static <T extends CharSequence> T pick(Raw<String> r, Supplier<T> s, List<T> l) { return null; }
                   static <T> List<T> list(Raw<String> r, List<T> l) { return null; } }
         class T { void m(Raw rw, Supplier<String> sup, List<String> ls) { Object a = /*T*/ P.pick(rw, sup, ls); Object b = /*T*/ P.list(rw, ls); } }",
    );
    // javac: String (T inferred, then erased), List
    assert_eq!(t, vec!["java.lang.String", "java.util.List"]);
}

#[test]
fn bounds_between_variables_and_unknown_arguments_leave_lubs_open() {
    let t = types_of(
        "import java.util.*;
         class Raw<X> extends ArrayList {}
         class Bad implements Comparable<String> { public int compareTo(String o) { return 0; } }
         class P2 { static <T> double pick(Iterable<T> p0) { return 0; } static <T, U extends T> List<? extends T> pick(List<? extends T> p0, long p1, List<U> p2) { return null; } }
         class P3 { static <T> List<T>[] pick(Integer a, Collection<? extends T> b, T c) { return null; } static Byte pick() { return null; } }
         class P4 { static <T, U extends T> List<T> pick(T a, U b) { return null; } }
         class T {
           void m(long l, List<?> lw, Integer i, Collection<? extends Number> ce, Bad bad, Raw rw) {
             Object a = /*T*/ P2.pick(List.of(1, 2), l, lw);
             Object b = /*T*/ P3.pick(i, ce, new ArrayList<>());
             Object c = /*T*/ P4.pick(bad, rw);
           } }",
    );
    // javac: List<?>, List<Serializable>[], List<Object>: U's type (or an
    // argument we cannot type) is one more lower bound of T
    assert_eq!(t, vec!["java.util.List<? extends ?unknown>", "java.util.List<?unknown>[]", "java.util.List<?unknown>"]);
}

#[test]
fn lambda_arguments_leave_a_lower_bound_open() {
    let t = types_of(
        "import java.util.*;
         class P1 { static String pick(String s) { return null; } static Integer pick(Object o) { return null; } }
         class T { void m() { Object a = /*T*/ P1.pick(Objects.requireNonNullElseGet(\"x\", () -> 1)); } }",
    );
    // javac: Integer (T is lub(String, Integer))
    assert_eq!(t, vec!["?unknown"]);
}

#[test]
fn equal_erasures_with_other_type_arguments_need_proof() {
    let t = types_of(
        "import java.util.*;
         class P2 { static <T> T first(List<? super T> xs) { return null; }
                    static List<String> pick(String s) { return null; } static List<Integer> pick(Integer i, Object... rest) { return null; } }
         class T { void m(List<Integer> xs) { Object a = /*T*/ P2.pick(P2.first(xs)); } }",
    );
    // javac: List<Integer>
    assert_eq!(t, vec!["java.util.List<?unknown>"]);
}

#[test]
fn untyped_poly_arguments_add_no_bound_and_targets_do_not_fill_in() {
    let t = types_of(
        "import java.util.*;
         class P3 { static <T> T any() { return null; } }
         class T { void m() { Object a = /*T*/ Objects.requireNonNullElse(\"x\", P3.any()); } }",
    );
    // javac: String
    assert_eq!(t, vec!["java.lang.String"]);
}

#[test]
fn unchecked_results_take_bounds_not_targets() {
    let t = types_of(
        "import java.util.*;
         class P4 { static <T extends Number> T first(List<T> xs) { return null; }
                    static <T extends CharSequence> T sequence(T x, List<String> xs) { return null; } }
         class T { <U extends CharSequence> void m(List raw, U u) { Object a = /*T*/ P4.first(raw); Object b = /*T*/ P4.sequence(u, raw); } }",
    );
    // javac: Number (glb of Object and the bound), CharSequence (U erased)
    assert_eq!(t, vec!["java.lang.Number", "java.lang.CharSequence"]);
}

#[test]
fn lambda_inputs_are_resolved_before_the_body() {
    // javac resolves a variable in an implicit lambda's parameter types from
    // the target or its bounds before it looks at the body
    let t = types_of(
        "import java.util.*; import java.util.function.*;
         class P { static <T extends Number> UnaryOperator<T> pick(Function<? super T, ? extends T> f) { return null; }
                   static <T> Comparable<? super T> ref(Function<? super T, ? extends T> f) { return null; } }
         class T { void m() { Object a = /*T*/ P.pick(x -> 1); Object b = /*T*/ P.ref(String::valueOf); } }",
    );
    // javac: UnaryOperator<Number>, Comparable<? super Object>
    assert_eq!(t, vec!["java.util.function.UnaryOperator<?unknown>", "java.lang.Comparable<? super ?unknown>"]);
}

#[test]
fn targets_bound_results_from_above() {
    let t = types_of(
        "import java.util.*;
         class A {}
         class P { static <T extends Number> List<? extends T> nums() { return null; } static <T extends A> T a() { return null; }
                   static <T> T any() { return null; } }
         class T { void m() { List<?> x = /*T*/ P.nums(); Object y = /*T*/ P.a(); String z = /*T*/ P.any(); } }",
    );
    // javac: List<? extends Number> (a wildcard target says nothing), A, String
    assert_eq!(t, vec!["java.util.List<? extends ?unknown>", "A", "java.lang.String"]);
}

#[test]
fn lambda_and_reference_bodies_are_lower_bounds_too() {
    let t = types_of(
        "import java.util.*; import java.util.function.*;
         class A {}
         class P { static <T> T[] pick(List<? extends T> l, Supplier<T> s) { return null; }
                   static <T> List<T> make(Supplier<T> s) { return null; }
                   static <K, V> List<V> two(BiFunction<K, K, V> f, UnaryOperator<K> g, V v) { return null; }
                   static <T> Iterable<T> cap(Supplier<T> s, List<? extends T> l, Integer i) { return null; } }
         class T { void m(List<String> ls, List<? super Integer> lsu, Integer i) {
             Object a = /*T*/ P.pick(ls, A::new);
             Object b = /*T*/ P.make(ArrayList::new);
             Object c = /*T*/ P.two((x, y) -> x, x -> null, 1L);
             Object d = /*T*/ P.cap(() -> 1, lsu, i);
         } }",
    );
    // javac: lub(String, A)[], List<ArrayList<Object>>, List<Object>, Iterable<Object>
    assert_eq!(t, vec!["?unknown[]", "java.util.List<?unknown>", "java.util.List<?unknown>", "java.lang.Iterable<?unknown>"]);
}

#[test]
fn calls_typed_through_a_lambda_are_no_bound_free_arguments() {
    let t = types_of(
        "import java.util.*; import java.util.function.*;
         class H { static <T, R> R apply(T x, Function<? super T, ? extends R> f) { return null; } }
         class P { static <T> UnaryOperator<T> pick(List<T> l) { return null; } }
         class T { void m(Function<String, Integer> fn, Integer i) { Object a = /*T*/ P.pick(Arrays.asList(fn, H.apply(i, x -> 1))); } }",
    );
    // javac: UnaryOperator<Object> (`apply` gives an Integer through its lambda)
    assert_eq!(t, vec!["java.util.function.UnaryOperator<?unknown>"]);
}

#[test]
fn a_panic_inside_a_guard_leaves_the_node() {
    // the runner catches a rule's panic and keeps using the file's model
    let pf = ParsedFile::parse("class T {}");
    let mut idx = Index::new(jdk());
    idx.set_file(Arc::from("T.java"), Arc::new(extract(&pf)));
    let fm = FileModel::new(&pf, &idx, Arc::from("T.java"));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fm.guard(7, 0, 0, || -> i32 { panic!("rule bug") })));
    assert!(r.is_err());
    assert_eq!(fm.guard(7, 0, 0, || 1), 1, "the node is not left marked in progress");
    // a real cycle still returns the default
    assert_eq!(fm.guard(8, 0, 0, || fm.guard(8, 0, -1, || 2)), -1);
}

#[test]
fn explicit_lambda_types_fix_inputs_and_references_stay_open() {
    let t = types_of(
        "import java.util.function.*;
         class P { static <T extends Number> T apply(UnaryOperator<T> f) { return null; } }
         class T { void m() { Number a = /*T*/ P.apply((Integer x) -> 1); Number b = /*T*/ P.apply(Integer::intValue); } }",
    );
    // javac: Integer, Integer (the target Number does not decide either)
    assert_eq!(t, vec!["java.lang.Integer", "?unknown"]);
}

#[test]
fn unsettled_inner_calls_do_not_adapt_through_a_wrapper() {
    let t = types_of(
        "import java.util.*;
         class Base {} class A extends Base {} class B extends Base {}
         class P { static <T> List<T> combine(T x, List<? extends T> ys) { return null; } }
         class T { void m() { Object a = /*T*/ P.combine(new A(), Collections.singletonList(Objects.requireNonNullElseGet(new B(), () -> new B()))); } }",
    );
    // javac: List<Base> (the B through the lambda is one more lower bound)
    assert_eq!(t, vec!["java.util.List<?unknown>"]);
}

#[test]
fn unsettled_poly_arguments_still_adapt_their_settled_parts() {
    let t = types_of(
        "import java.util.*;
         class P { static String pick(Map.Entry<Object, Integer> p) { return null; } static Integer pick(Object p) { return null; } }
         class T { void m() { Object a = /*T*/ P.pick(Map.entry(\"s\", Objects.requireNonNullElseGet(1, () -> 2))); } }",
    );
    // javac: String (the key adapts to Object); the Object overload is no proof
    assert_eq!(t, vec!["?unknown"]);
}

#[test]
fn explicit_lambda_types_under_a_wildcard_are_bounds() {
    let t = types_of(
        "import java.util.*; import java.util.function.*;
         class P { static <T> List<T> make(Consumer<? super T> c) { return null; } }
         class T { void m() { List<Integer> a = /*T*/ P.make((Number x) -> {}); } }",
    );
    // javac: List<Integer> (T <: Number, the target picks Integer)
    assert_eq!(t, vec!["java.util.List<?unknown>"]);
}

#[test]
fn unchecked_results_keep_lambda_uncertainty() {
    let t = types_of(
        "import java.util.*; import java.util.function.*;
         class P { static <T> T make(List<String> xs, Consumer<? super T> c) { return null; } }
         class T { void m(List raw) { Object a = /*T*/ P.make(raw, (Number x) -> {}); } }",
    );
    // javac: Number (T <: Number through the lambda, then erased)
    assert_eq!(t, vec!["?unknown"]);
}

#[test]
fn unknown_variables_cause_no_unchecked_erasure() {
    let t = types_of(
        "import java.util.*;
         class Raw<X> extends ArrayList {}
         class P { static <T, U extends T> List<String> pick(long p0, Collection<? extends T> p1, Map<String,Integer> p2) { return null; } }
         class T { void m(long l, List<? super Integer> lsu, Raw rw, Map<String,Integer> m) {
           Object a = /*T*/ List.of(lsu, rw);
           Object b = /*T*/ P.pick(l, List.of(lsu, rw), m);
         } }",
    );
    // javac: List<List> (no unchecked call), List<String>
    assert_eq!(t, vec!["java.util.List<?unknown>", "java.util.List<java.lang.String>"]);
}

#[test]
fn target_typed_arguments_keep_what_their_arguments_prove() {
    // javac: String(byte[]), doPrivileged(PrivilegedAction): the other
    // overloads fail whatever the unknown types are
    let c = calls_of(
        "import java.util.Objects; import java.security.*;
         import io.github.resilience4j.retry.Retry; import groovy.lang.GroovyClassLoader;
         class T { Object m(Retry retry) throws Exception {
           final var response = retry.executeCallable(() -> 1);
           Object a = /*C*/ new String(Objects.requireNonNullElse(response.body(), new byte[0]));
           return /*C*/ AccessController.doPrivileged((PrivilegedAction<GroovyClassLoader>) GroovyClassLoader::new);
         } }",
    );
    assert_eq!(c, vec!["String.<init>(byte[])", "AccessController.doPrivileged(java.security.PrivilegedAction)"]);
    // T is at least byte[] and, as String(byte[])'s argument, at most byte[]
    let t = types_of(
        "import java.util.Objects; import io.github.resilience4j.retry.Retry;
         class T { void m(Retry retry) throws Exception {
           final var response = retry.executeCallable(() -> 1);
           String a = new String(/*T*/ Objects.requireNonNullElse(response.body(), new byte[0]));
           Object b = /*T*/ Objects.requireNonNullElse(response.body(), new byte[0]);
         } }",
    );
    assert_eq!(t, vec!["byte[]", "?unknown"]);
}

#[test]
fn poly_arguments_bind_what_the_target_cannot_change() {
    let t = types_of(
        "import java.util.*; import java.util.stream.*;
         class T {
           record Conn(String a) { static Conn of(Number r) { return null; } }
           void m(List<Integer> rs) {
             Object a = /*T*/ rs.stream().collect(Collectors.groupingBy(Conn::of));
             Object b = /*T*/ rs.stream().collect(Collectors.groupingBy(Conn::of)).values();
             Map<Conn, List<Integer>> c = /*T*/ rs.stream().collect(Collectors.groupingBy(Conn::of));
             Map<Number, String> d = /*T*/ Stream.of(\"a\").collect(Collectors.toMap(k -> 1, v -> \"x\"));
             var e = /*T*/ Stream.of(\"a\").collect(Collectors.toMap(k -> 1, v -> \"x\"));
             Map<? extends Number, String> f = /*T*/ Stream.of(\"a\").collect(Collectors.toMap(k -> 1, v -> \"x\"));
             Map<Object, List<Integer>> g = /*T*/ rs.stream().collect(Collectors.groupingBy(Conn::of));
           }
         }",
    );
    // javac: Map<Conn, List<Integer>> (not the target's Object) and its
    // values; an invariant target of the result's class is the result; a
    // wildcard target takes the poly argument's own Map<Integer, String>
    assert_eq!(
        t,
        vec![
            "java.util.Map<T.Conn, java.util.List<?unknown>>",
            "java.util.Collection<java.util.List<?unknown>>",
            "java.util.Map<T.Conn, java.util.List<java.lang.Integer>>",
            "java.util.Map<java.lang.Number, java.lang.String>",
            "java.util.Map<java.lang.Integer, java.lang.String>",
            "java.util.Map<java.lang.Integer, java.lang.String>",
            "java.util.Map<java.lang.Object, java.util.List<java.lang.Integer>>"
        ]
    );
}

#[test]
fn unknown_variables_keep_the_concrete_parts_of_their_parameter() {
    // Astra: the Map key String rules the generic overload out whatever T
    // is; javac picks the other by unchecked conversion and erases its result
    let t = types_of(
        "import java.util.*;
         class P {
           static <T extends Number> List<String> pick(Map<String, ? super T> m, List xs) { return null; }
           static List<String> pick(Object m, Collection<String> xs) { return null; }
         }
         class T { void m(Map<Integer, ? super Integer> m, List raw) { Object r = /*T*/ P.pick(m, raw); } }",
    );
    assert_eq!(t, vec!["java.util.List"]);
}

