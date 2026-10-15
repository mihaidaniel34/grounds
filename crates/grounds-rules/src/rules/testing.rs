//! Test code: JUnit 4/5, TestNG, AssertJ, Mockito.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_semantic::Symbol;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(AssertionArgumentOrder),
        Box::new(TestWithoutAssertion),
        Box::new(PrivateTestMethod),
        Box::new(JUnit5PublicModifier),
        Box::new(MixedJUnitVersions),
        Box::new(AssertTrueEquals),
        Box::new(AssertSameArguments),
        Box::new(SleepInTest),
        Box::new(IncompleteAssertJ),
        Box::new(IncompleteMockito),
        Box::new(TestClassWithoutTests),
        Box::new(StaticNestedTest),
        Box::new(DisabledWithoutReason),
        Box::new(ConstantAssertion),
        Box::new(AssertThrowsMultipleCalls),
    ]
}

const TEST_ANNOTATIONS: &[&str] = &["Test", "ParameterizedTest", "RepeatedTest", "TestFactory", "TestTemplate"];

fn is_test_method(m: Node, src: &str) -> bool {
    annotation_names(m, src).iter().any(|a| TEST_ANNOTATIONS.contains(a))
}

/// The class declaring an assertion call: the resolved method's owner, or,
/// when overloads cannot be told apart (arguments of unknown type), the one
/// class all candidates come from: the qualifier (`Assert.assertEquals`),
/// an inherited method (JUnit 3), or a static import.
fn assertion_class<'t>(cx: &Ctx<'_, 't>, call: Node<'t>) -> Option<Name> {
    if let Some(c) = cx.fm.select_method(call) {
        return Some(c.owner.clone());
    }
    let name = cx.fm.call_name(call)?;
    let mut owners: Vec<Name> = Vec::new();
    if let Some(o) = call.field("object") {
        if let Some(Symbol::Type(t)) = cx.fm.symbol(o) {
            owners.push(t);
        }
    } else {
        for cls in call.ancestors().filter(|a| grounds_syntax::is_type_declaration(a.kind())).filter_map(|d| cx.fm.class_of_decl(d)) {
            owners.extend(cx.fm.methods_named(&cls.this_type(), name).into_iter().map(|m| m.owner));
            if !owners.is_empty() {
                break;
            }
        }
        if owners.is_empty() {
            for i in cx.fm.stub.imports.iter().filter(|i| i.is_static) {
                let class = if i.on_demand { &*i.path } else if i.last() == name { i.path.rsplit_once('.').map_or("", |x| x.0) } else { continue };
                match cx.fm.index.binary_name_for_dotted(class) {
                    Some(b) if !i.on_demand || !cx.fm.methods_named(&Ty::class(&b), name).is_empty() => owners.push(b),
                    Some(_) => {}
                    // not on the classpath: it may provide the method
                    None => owners.push(Name::from(class)),
                }
            }
        }
    }
    owners.dedup();
    match owners.as_slice() {
        [o] => Some(o.clone()),
        _ => None,
    }
}

declare_rule!(AssertionArgumentOrder {
    id: "assertion-argument-order",
    name: "Expected and actual swapped in assertEquals",
    category: Testing,
    severity: Warning,
    scope: Test,
    sources: ["sonar:S3415"],
    kinds: ["method_invocation"],
    description: "JUnit's `assertEquals(expected, actual)` takes the expected value first; with a literal in second position the failure message reads backwards (`expected:<computed> but was:<42>`). (TestNG uses the opposite order.)",
});

impl Rule for AssertionArgumentOrder {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(cx.fm.call_name(n), Some("assertEquals" | "assertNotEquals" | "assertSame" | "assertNotSame" | "assertArrayEquals")) {
            return;
        }
        let args = cx.fm.args_of(n);
        if args.len() < 2 {
            return;
        }
        let owner = assertion_class(cx, n);
        // a project helper named assertEquals may take (actual, expected)
        if owner.as_ref().is_some_and(|o| !(o.starts_with("org.junit") || o.starts_with("junit.") || o.starts_with("org.testng") || o.starts_with("kotlin.test"))) {
            return;
        }
        // JUnit 4 (and 3) take an optional leading message String, JUnit 5
        // and TestNG a trailing one
        let message_first = owner.as_ref().is_some_and(|o| matches!(&**o, "org.junit.Assert" | "junit.framework.Assert" | "junit.framework.TestCase"));
        let string = |x: Node<'t>| cx.type_of(x).is_string() || x.unparen().kind() == "string_literal" || x.unparen().kind() == "binary_expression" && x.unparen().kids().iter().any(|k| k.kind() == "string_literal");
        let (first, second) = if args.len() >= 3 && message_first && string(args[0]) { (args[1], args[2]) } else { (args[0], args[1]) };
        let testng = owner.as_ref().is_some_and(|o| o.starts_with("org.testng"));
        let lit = |x: Node<'t>| cx.fm.constant_value(x).is_some() || matches!(x.unparen().kind(), "null_literal" | "class_literal");
        let (exp, act) = if testng { (second, first) } else { (first, second) };
        if lit(act) && !lit(exp) {
            cx.report(n, if testng { "TestNG expects (actual, expected); the constant should be second" } else { "The expected value should be the first argument" });
        }
    }
}

declare_rule!(TestWithoutAssertion {
    id: "test-without-assertion",
    name: "Test method without assertions",
    category: Testing,
    severity: Warning,
    scope: Test,
    sources: ["sonar:S2699", "pmd:UnitTestShouldIncludeAssert"],
    kinds: ["method_declaration"],
    description: "A test that asserts nothing only fails if an exception is thrown; it may pass while the behaviour is wrong. Assert on the result (or use assertDoesNotThrow to make the intent explicit).",
});

impl Rule for TestWithoutAssertion {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !is_test_method(n, cx.src) {
            return;
        }
        // JUnit 4 expected exceptions
        if n.descendants().any(|d| d.kind() == "element_value_pair" && d.field("key").is_some_and(|k| matches!(cx.text(k), "expected" | "expectedExceptions"))) {
            return;
        }
        let Some(body) = n.field("body") else { return };
        // empty body: a deliberate smoke test (Spring's `contextLoads()`)
        if body.named_children(&mut body.walk()).all(|c| c.kind().contains("comment")) {
            return;
        }
        let text = cx.text(body);
        if text.contains("assert ") || text.contains("fail(") {
            return;
        }
        // lambdas count too (assertions in callbacks)
        let mut seen = Vec::new();
        let has = body.descendants().any(|c| match c.kind() {
            "method_invocation" => call_asserts(c, n, cx, HELPER_DEPTH, &mut seen),
            "method_reference" => verifier_ref(c, cx),
            _ => false,
        });
        if !has {
            if let Some(nm) = n.field("name") {
                cx.report(nm, "Test method does not assert anything");
            }
        }
    }
}

/// How many levels of project helpers a test's calls are followed through.
const HELPER_DEPTH: u32 = 3;

/// Expectations a helper checks when it is given one (`perform(url, status().isOk())`).
const MATCHER_TYPES: &[&str] = &["org.hamcrest.Matcher", "org.springframework.test.web.servlet.ResultMatcher"];

/// Whether a call made by `test` asserts: an assertion-style call, a helper
/// given a matcher, a project helper that asserts, or an inherited
/// test-harness method.
fn call_asserts<'t>(c: Node<'t>, test: Node<'t>, cx: &Ctx<'_, 't>, depth: u32, seen: &mut Vec<usize>) -> bool {
    let name = cx.fm.call_name(c).unwrap_or("");
    if is_assertion_name(name) {
        return true;
    }
    // Awaitility's `await().atMost(t).until(condition)` fails when the
    // condition never holds
    if name.starts_with("until") && chain_root(c).is_some_and(|r| cx.fm.call_name(r) == Some("await")) {
        return true;
    }
    let resolved = cx.fm.select_method(c);
    let matcher = |t: &Ty| is_subtype_any(cx.fm, t, MATCHER_TYPES);
    if resolved.as_ref().is_some_and(|m| m.method.params.iter().any(|p| matcher(&p.ty))) || cx.fm.args_of(c).iter().any(|a| matcher(&cx.type_of(*a))) {
        return true;
    }
    let unqualified = c.field("object").is_none_or(|o| matches!(o.kind(), "this" | "super"));
    let Some(m) = resolved else {
        return unqualified && unresolved_helper_asserts(c, name, test, cx, depth, seen);
    };
    match cx.fm.index.source_file_of(&m.owner) {
        // declared here: follow the body, which may call helpers of other files
        Some(f) if *f == cx.fm.path => m.method.flags.has(Flags::ASSERTS) || depth > 0 && method_decl(cx, &m.method).is_some_and(|d| decl_asserts(d, test, cx, depth - 1, seen)),
        // a test helper of another file, checked when it was indexed; one
        // that delegates to a method inherited from yet another file may
        // assert there
        Some(f) if grounds_index::sources::is_test_path(f) => m.method.flags.has(Flags::ASSERTS) || m.method.flags.has(Flags::DELEGATES),
        // library (or main) code: an inherited void harness method asserts
        // (`rewriteRun(...)` from OpenRewrite's RewriteTest); `int compute()`
        // merely computes
        _ => {
            unqualified
                && m.method.ret == Ty::Void
                && test.ancestors().filter(|a| grounds_syntax::is_type_declaration(a.kind())).filter_map(|d| cx.fm.class_of_decl(d)).any(|oc| m.owner != oc.name && cx.fm.is_subclass(&oc.name, &m.owner).is_yes())
        }
    }
}

/// A verifying method passed to a helper (`StepVerifier.LastStep::verifyComplete`).
fn verifier_ref<'t>(r: Node<'t>, cx: &Ctx<'_, 't>) -> bool {
    r.kids().last().is_some_and(|k| k.kind() == "identifier" && is_assertion_name(cx.text(*k)))
}

/// The first call of a receiver chain (`await()` in `await().atMost(t).until(c)`).
fn chain_root(c: Node<'_>) -> Option<Node<'_>> {
    let mut root = c.field("object").filter(|o| o.kind() == "method_invocation")?;
    while let Some(o) = root.field("object").filter(|o| o.kind() == "method_invocation") {
        root = o;
    }
    Some(root)
}

/// An unqualified call that did not resolve: a same-named helper declared
/// around it, else a method inherited from a supertype we cannot see (unless
/// a static import may provide it).
fn unresolved_helper_asserts<'t>(c: Node<'t>, name: &str, test: Node<'t>, cx: &Ctx<'_, 't>, depth: u32, seen: &mut Vec<usize>) -> bool {
    let arity = cx.fm.args_of(c).len();
    for cls in c.ancestors().filter(|a| grounds_syntax::is_type_declaration(a.kind())) {
        let own: Vec<Node<'t>> = methods_of(cls).into_iter().filter(|m| method_name(*m, cx.src) == name && takes_args(*m, arity)).collect();
        if !own.is_empty() {
            return depth > 0 && own.iter().any(|m| decl_asserts(*m, test, cx, depth - 1, seen));
        }
    }
    !cx.fm.stub.imports.iter().any(|i| {
        i.is_static
            && if i.on_demand {
                cx.fm.index.binary_name_for_dotted(&i.path).is_none_or(|b| !cx.fm.methods_named(&Ty::class(&b), name).is_empty())
            } else {
                i.last() == name
            }
    })
}

/// The declaration of a method of this file.
fn method_decl<'t>(cx: &Ctx<'_, 't>, m: &MethodInfo) -> Option<Node<'t>> {
    let (s, e) = m.range?;
    let name = cx.fm.root().descendant_for_byte_range(s as usize, e as usize)?;
    name.parent().filter(|p| p.kind() == "method_declaration")
}

/// Whether a helper declared in this file asserts: `assert`, a thrown
/// `AssertionError`, or a call that asserts. `seen` guards against cycles.
fn decl_asserts<'t>(d: Node<'t>, test: Node<'t>, cx: &Ctx<'_, 't>, depth: u32, seen: &mut Vec<usize>) -> bool {
    if seen.contains(&d.id()) {
        return false;
    }
    seen.push(d.id());
    let Some(body) = d.field("body") else { return false };
    body.descendants().any(|x| match x.kind() {
        "method_invocation" => call_asserts(x, test, cx, depth, seen),
        "method_reference" => verifier_ref(x, cx),
        "assert_statement" => true,
        "throw_statement" => cx.text(x).contains("AssertionError"),
        _ => false,
    })
}

/// Whether a method declaration accepts `n` arguments (varargs included).
fn takes_args(m: Node<'_>, n: usize) -> bool {
    let pc = param_count(m);
    let varargs = m.field("parameters").is_some_and(|p| p.kid_of_kind("spread_parameter").is_some());
    if varargs {
        n + 1 >= pc
    } else {
        n == pc
    }
}

fn is_junit5_class<'t>(decl: Node<'t>, cx: &Ctx<'_, 't>) -> bool {
    let text = cx.fm.stub.imports.iter().any(|i| i.path.starts_with("org.junit.jupiter"));
    text || cx.text(decl).contains("org.junit.jupiter")
}

declare_rule!(PrivateTestMethod {
    id: "junit5-private-test-method",
    name: "JUnit 5 test method is private or static",
    category: Testing,
    severity: Error,
    scope: Test,
    sources: ["sonar:S5810"],
    kinds: ["method_declaration"],
    description: "JUnit 5 silently skips `private` (and `static`) test methods and lifecycle methods, so the test never runs.",
});

impl Rule for PrivateTestMethod {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let anns = annotation_names(n, cx.src);
        let is_test = anns.iter().any(|a| TEST_ANNOTATIONS.contains(a));
        // @BeforeEach/@AfterEach must not be private (or static) either, but
        // they are not tests: say so
        let lifecycle = anns.iter().find(|a| matches!(**a, "BeforeEach" | "AfterEach")).copied();
        if !(is_test || lifecycle.is_some()) || !owning_type_decl(n).is_some_and(|d| is_junit5_class(d, cx)) {
            return;
        }
        if n.has_modifier("private") || n.has_modifier("static") {
            let what = if n.has_modifier("private") { "private" } else { "static" };
            if let Some(nm) = n.field("name") {
                match lifecycle {
                    Some(l) if !is_test => cx.report(nm, format!("JUnit 5 requires @{l} methods to be non-{what}; depending on the version it is ignored or rejected")),
                    _ => cx.report(nm, format!("JUnit 5 ignores {what} test methods; this test never runs")),
                }
            }
        }
    }
}

declare_rule!(JUnit5PublicModifier {
    id: "junit5-unnecessary-public",
    name: "Unnecessary public modifier on JUnit 5 test",
    category: Style,
    severity: Info,
    scope: Test,
    enabled: false,
    sources: ["sonar:S5786"],
    kinds: ["method_declaration"],
    description: "JUnit 5 test classes and methods can be package-private; `public` is noise.",
});

impl Rule for JUnit5PublicModifier {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if is_test_method(n, cx.src) && n.has_modifier("public") && owning_type_decl(n).is_some_and(|d| is_junit5_class(d, cx)) {
            if let Some(nm) = n.field("name") {
                cx.report(nm, "JUnit 5 tests do not need to be public");
            }
        }
    }
}

declare_rule!(MixedJUnitVersions {
    id: "mixed-junit-versions",
    name: "JUnit 4 and JUnit 5 annotations mixed",
    category: Testing,
    severity: Error,
    scope: Test,
    sources: ["sonar:S5967", "sonar:S6073"],
    kinds: ["class_declaration"],
    description: "A class mixing `org.junit.Test` with JUnit 5 annotations runs under only one engine, so some tests or lifecycle methods are silently ignored.",
});

impl Rule for MixedJUnitVersions {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let mut v4 = None;
        let mut v5 = false;
        for m in methods_of(n) {
            for a in cx.fm.annotations_of(m) {
                if a.name.starts_with("org.junit.jupiter") {
                    v5 = true;
                } else if a.name.starts_with("org.junit.") && !a.name.starts_with("org.junit.jupiter") && v4.is_none() {
                    v4 = Some(m);
                }
            }
        }
        if let (Some(m), true) = (v4, v5) {
            if let Some(nm) = m.field("name") {
                cx.report(nm, "JUnit 4 annotation in a class that also uses JUnit 5; one of them is ignored");
            }
        }
    }
}

declare_rule!(AssertTrueEquals {
    id: "assert-true-with-comparison",
    name: "assertTrue/assertFalse with an equality check",
    category: Testing,
    severity: Info,
    scope: Test,
    sources: ["sonar:S5785", "pmd:SimplifiableTestAssertion"],
    kinds: ["method_invocation"],
    description: "`assertTrue(a.equals(b))`, `assertTrue(x == null)` and `assertEquals(null, x)` give useless failure messages. Use `assertEquals`, `assertNull`, `assertSame`.",
});

impl Rule for AssertTrueEquals {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = cx.fm.call_name(n) else { return };
        let args = cx.fm.args_of(n);
        if matches!(name, "assertEquals" | "assertNotEquals") && args.len() >= 2 && (is_null_literal(args[0]) || is_null_literal(args[1])) {
            cx.report(n, format!("Use `{}` instead of comparing with null", if name == "assertEquals" { "assertNull" } else { "assertNotNull" }));
            return;
        }
        if !matches!(name, "assertTrue" | "assertFalse") || args.is_empty() {
            return;
        }
        let c = args[0].unparen();
        let suggestion = match c.kind() {
            "method_invocation" if cx.fm.call_name(c) == Some("equals") && cx.fm.args_of(c).len() == 1 => Some(if name == "assertTrue" { "assertEquals" } else { "assertNotEquals" }),
            "binary_expression" if matches!(op(c), "==" | "!=") => {
                let (l, r) = lr(c).unwrap();
                if is_null_literal(l) || is_null_literal(r) {
                    Some(if (name == "assertTrue") == (op(c) == "==") { "assertNull" } else { "assertNotNull" })
                } else if cx.type_of(l).is_reference() {
                    Some(if (name == "assertTrue") == (op(c) == "==") { "assertSame" } else { "assertNotSame" })
                } else {
                    Some(if (name == "assertTrue") == (op(c) == "==") { "assertEquals" } else { "assertNotEquals" })
                }
            }
            "instanceof_expression" => Some("assertInstanceOf"),
            _ => None,
        };
        if let Some(s) = suggestion {
            cx.report(n, format!("Use `{s}` for a meaningful failure message"));
        }
    }
}

declare_rule!(AssertSameArguments {
    id: "assertion-compares-same-value",
    name: "Assertion compares a value with itself",
    category: Testing,
    severity: Error,
    scope: Test,
    sources: ["sonar:S5863"],
    kinds: ["method_invocation"],
    description: "`assertSame(x, x)` always passes; one argument should be the expected value. Equality assertions (`assertEquals(x, x)`, `isEqualTo`) are only reported for types with a known `equals` (primitives, boxes, String, enums): on other types they are a legitimate reflexivity test of `equals`.",
});

impl Rule for AssertSameArguments {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = cx.fm.call_name(n) else { return };
        if !matches!(name, "assertEquals" | "assertSame" | "assertNotEquals" | "assertNotSame" | "assertArrayEquals" | "isEqualTo" | "isSameAs") {
            return;
        }
        let identity = matches!(name, "assertSame" | "assertNotSame" | "isSameAs");
        // equality on a type with its own equals may be testing that equals
        let trivial = |x: Node<'t>| {
            if identity {
                return true;
            }
            let t = cx.type_of(x);
            t.is_prim() || t.is_string() || t.unboxed().is_some() || t.class_name().and_then(|c| cx.fm.class(c)).is_some_and(|c| c.is_enum())
        };
        let args = cx.fm.args_of(n);
        if args.len() >= 2 && same_pure_expr(args[0], args[1], cx.src) && !is_literal(args[0]) && trivial(args[0]) {
            cx.report(n, "Both sides of the assertion are the same expression");
            return;
        }
        // AssertJ: assertThat(x).isEqualTo(x)
        if args.len() == 1 {
            if let Some(o) = n.field("object").map(|o| o.unparen()) {
                if is_call_named(o, cx.src, &["assertThat"]) {
                    if let Some(a) = cx.fm.args_of(o).first() {
                        if same_pure_expr(*a, args[0], cx.src) && !is_literal(args[0]) && trivial(args[0]) {
                            cx.report(n, "The actual and expected values are the same expression");
                        }
                    }
                }
            }
        }
    }
}

declare_rule!(SleepInTest {
    id: "thread-sleep-in-test",
    name: "Thread.sleep() in a test",
    category: Testing,
    severity: Info,
    scope: Test,
    sources: ["sonar:S2925"],
    kinds: ["method_invocation"],
    description: "Sleeping makes tests slow and flaky (too short on loaded CI, too long locally). Wait for the condition with a polling helper (Awaitility) or a latch.",
});

impl Rule for SleepInTest {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) == Some("sleep") && cx.fm.select_method(n).is_some_and(|c| &*c.owner == "java.lang.Thread" || c.owner.ends_with("TimeUnit"))
            && enclosing_method_decl(n).is_some_and(|m| is_test_method(m, cx.src)) {
                cx.report(n, "Avoid Thread.sleep in tests; wait for the condition instead");
            }
    }
}

declare_rule!(IncompleteAssertJ {
    id: "assertj-assertion-without-check",
    name: "AssertJ assertThat() without an assertion",
    category: Testing,
    severity: Error,
    scope: Test,
    sources: ["sonar:S2970", "errorprone:AssertJIsNull"],
    kinds: ["expression_statement"],
    description: "`assertThat(x);` (or `.as(\"...\")` without a check) only builds an assertion object and never verifies anything.",
});

impl Rule for IncompleteAssertJ {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = n.first_kid() else { return };
        if e.kind() != "method_invocation" {
            return;
        }
        // assertThatThrownBy(...) alone already asserts that something is thrown
        let is_entry = |m: Node<'t>| m.kind() == "method_invocation" && matches!(cx.fm.call_name(m), Some("assertThat" | "assertThatCode" | "assertThatObject"));
        let is_description = |m: Node<'t>| m.kind() == "method_invocation" && matches!(cx.fm.call_name(m), Some("as" | "describedAs" | "withFailMessage" | "overridingErrorMessage"));
        // descriptions directly on assertThat(...), with no check in between;
        // `assertThat(x).isEqualTo(1).describedAs("..")` is complete
        let mut entry = e;
        while is_description(entry) {
            match entry.field("object") {
                Some(o) => entry = o.unparen(),
                None => break,
            }
        }
        let incomplete = is_entry(entry);
        // AssertJ's assertThat takes one argument; Hamcrest's
        // MatcherAssert.assertThat(reason, actual, matcher) is complete
        let single_arg = cx.fm.args_of(entry).len() == 1;
        let other_owner = cx.fm.select_method(entry).is_some_and(|c| !(c.owner.contains("assertj") || c.owner.contains("Truth")));
        if incomplete && single_arg && !other_owner && (cx.fm.select_method(e).is_some_and(|c| c.owner.contains("assertj") || c.owner.contains("Truth")) || cx.fm.stub.imports.iter().any(|i| i.path.contains("assertj"))) {
            let boolean = cx.fm.call_name(entry) == Some("assertThat") && cx.fm.args_of(entry).first().is_some_and(|a| matches!(cx.type_of(*a).prim(), Some(Prim::Boolean)) || cx.type_of(*a).is_class("java.lang.Boolean"));
            if boolean {
                cx.report(e, "`assertThat(condition)` has no check; it verifies nothing (add `.isTrue()`)");
            } else {
                cx.report(e, "AssertJ assertion has no check (e.g. `.isEqualTo(...)`); it verifies nothing");
            }
        }
    }
}

declare_rule!(IncompleteMockito {
    id: "mockito-incomplete-stubbing",
    name: "Incomplete Mockito stubbing or verification",
    category: Testing,
    severity: Error,
    scope: Test,
    sources: ["errorprone:MockitoUsage", "sonar:S6068"],
    kinds: ["expression_statement"],
    description: "`when(mock.call());` without `thenReturn`/`thenThrow` stubs nothing, and `verify(mock);` without a method call verifies nothing; Mockito reports it later as UnfinishedStubbingException or silently passes.",
});

impl Rule for IncompleteMockito {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = n.first_kid() else { return };
        if e.kind() != "method_invocation" {
            return;
        }
        let name = cx.fm.call_name(e).unwrap_or("");
        if !matches!(name, "when" | "verify" | "given" | "doReturn" | "doThrow") {
            return;
        }
        // Mockito's versions take the mock; Reactor's StepVerifier.verify() and
        // other libraries' methods of the same name do not
        if cx.fm.args_of(e).is_empty() || cx.fm.select_method(e).is_some_and(|c| !c.owner.starts_with("org.mockito")) {
            return;
        }
        // static-imported, or called on Mockito/BDDMockito (not `jwt.verify(token)`)
        if e.field("object").is_some_and(|o| !matches!(cx.text(o).rsplit('.').next(), Some("Mockito" | "BDDMockito"))) {
            return;
        }
        let is_mockito = cx.fm.select_method(e).is_some_and(|c| c.owner.starts_with("org.mockito")) || cx.fm.stub.imports.iter().any(|i| i.path.starts_with("org.mockito"));
        if !is_mockito {
            return;
        }
        match name {
            "when" | "given" => cx.report(e, format!("`{name}(...)` without `thenReturn`/`thenThrow` stubs nothing")),
            // MockedStatic.verify(() -> X.call()) is a complete verification
            "verify" if cx.fm.args_of(e).iter().any(|a| matches!(a.unparen().kind(), "lambda_expression" | "method_reference")) => {}
            "verify" => cx.report(e, "`verify(mock)` without a method call verifies nothing"),
            _ => cx.report(e, format!("`{name}(...)` without `.when(mock)` stubs nothing")),
        }
    }
}

declare_rule!(TestClassWithoutTests {
    id: "test-class-without-tests",
    name: "Test class contains no tests",
    category: Testing,
    severity: Info,
    scope: Test,
    sources: ["sonar:S2187"],
    kinds: ["class_declaration"],
    description: "A class named `*Test` with no test methods is usually a leftover or has tests that never run (e.g. missing @Test).",
});

impl Rule for TestClassWithoutTests {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(nm) = n.field("name") else { return };
        let name = cx.text(nm);
        if !(name.ends_with("Test") || name.ends_with("Tests")) || n.has_modifier("abstract") || n.field("superclass").is_some() {
            return;
        }
        let has_test = n.descendants().any(|d| d.kind() == "method_declaration" && is_test_method(d, cx.src))
            || n.descendants().any(|d| matches!(d.kind(), "class_declaration") && d != n && annotation_names(d, cx.src).contains(&"Nested"));
        if !has_test && !annotation_names(n, cx.src).iter().any(|a| matches!(*a, "Suite" | "RunWith" | "SelectClasses" | "SelectPackages" | "Cucumber")) {
            cx.report(nm, "Test class has no test methods");
        }
    }
}

declare_rule!(StaticNestedTest {
    id: "junit5-static-nested-class",
    name: "@Nested test class is static",
    category: Testing,
    severity: Warning,
    scope: Test,
    sources: ["sonar:S5790"],
    kinds: ["class_declaration"],
    description: "`@Nested` only applies to non-static inner classes. A static one is not run as part of the enclosing class: Gradle runs it as a separate top-level test class, Maven Surefire excludes nested classes by default and skips it.",
});

impl Rule for StaticNestedTest {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.has_modifier("static") && annotation_names(n, cx.src).contains(&"Nested") {
            if let Some(nm) = n.field("name") {
                cx.report(nm, "@Nested has no effect on a static class; make it non-static (Maven Surefire skips static nested classes)");
            }
        }
    }
}

declare_rule!(DisabledWithoutReason {
    id: "disabled-test-without-reason",
    name: "Disabled test without a reason",
    category: Testing,
    severity: Info,
    scope: Test,
    sources: ["sonar:S1607"],
    kinds: ["marker_annotation", "annotation"],
    description: "A disabled test without an explanation tends to stay disabled forever. Give `@Disabled`/`@Ignore` a reason (ideally with a ticket).",
});

impl Rule for DisabledWithoutReason {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let name = n.field("name").map(|x| cx.text(x)).unwrap_or("");
        let simple = name.rsplit('.').next().unwrap_or(name);
        if !matches!(simple, "Disabled" | "Ignore") {
            return;
        }
        let empty = n.kind() == "marker_annotation" || n.field("arguments").is_some_and(|a| cx.text(a).replace(char::is_whitespace, "") == "(\"\")" || cx.text(a) == "()");
        if empty {
            cx.report(n, format!("@{simple} without a reason"));
        }
    }
}

declare_rule!(ConstantAssertion {
    id: "constant-assertion",
    name: "Assertion that always passes or fails",
    category: Testing,
    severity: Warning,
    scope: Test,
    sources: ["sonar:S2701", "sonar:S5845"],
    kinds: ["method_invocation"],
    description: "`assertTrue(true)`, `assertNotNull(new X())` or `assertEquals(1, 1)` can never fail; they hide missing assertions. Use `fail()` for unreachable code.",
});

impl Rule for ConstantAssertion {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = cx.fm.call_name(n) else { return };
        let all = cx.fm.args_of(n);
        // JUnit 4 overloads take a leading message: assertEquals("msg", 0, failures)
        let base = if name == "assertEquals" { 2 } else { 1 };
        let args: &[Node<'t>] = if all.len() > base && all[0].unparen().kind() == "string_literal" { &all[1..] } else { &all[..] };
        let Some(first) = args.first().map(|a| a.unparen()) else { return };
        let constant = match name {
            "assertTrue" | "assertFalse" => matches!(first.kind(), "true" | "false"),
            "assertNotNull" => matches!(first.kind(), "object_creation_expression" | "string_literal" | "array_creation_expression"),
            "assertNull" => first.kind() == "null_literal",
            "assertEquals" if args.len() >= 2 => is_literal(args[0]) && is_literal(args[1]),
            _ => false,
        };
        if constant {
            let fails = match name {
                "assertTrue" => first.kind() == "false",
                "assertFalse" => first.kind() == "true",
                "assertEquals" => cx.fm.constant_value(args[0]).zip(cx.fm.constant_value(args[1])).is_some_and(|(a, b)| a != b),
                _ => false,
            };
            if fails {
                cx.report(n, "Use `fail()` instead of an assertion that always fails");
            } else {
                cx.report(n, "This assertion always passes");
            }
        }
    }
}

declare_rule!(AssertThrowsMultipleCalls {
    id: "assertthrows-multiple-calls",
    name: "assertThrows lambda with more than one call",
    category: Testing,
    severity: Info,
    scope: Test,
    sources: ["sonar:S5778"],
    kinds: ["method_invocation"],
    description: "When the lambda passed to `assertThrows` contains several calls that could throw, the test may pass because of the wrong one. Keep a single invocation inside the lambda.",
});

impl Rule for AssertThrowsMultipleCalls {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(cx.fm.call_name(n), Some("assertThrows" | "assertThatThrownBy" | "assertThatExceptionOfType")) {
            return;
        }
        let Some(l) = cx.fm.args_of(n).into_iter().find(|a| a.kind() == "lambda_expression") else { return };
        let Some(body) = l.field("body") else { return };
        if invocations(body, cx) > 1 {
            cx.report(l, "Keep a single throwing invocation inside the assertThrows lambda");
        }
    }
}

/// Invocations that run when a lambda body runs. A receiver chain
/// (`builder().a(x).build()`) is one invocation, and calls made only as its
/// arguments are part of it; nested lambdas, anonymous class bodies and
/// `catch` clauses do not run with it, and `fail()` is not a call that could
/// throw the expected exception (its arguments are).
fn invocations<'t>(n: Node<'t>, cx: &Ctx<'_, 't>) -> usize {
    match n.kind() {
        "lambda_expression" | "class_body" | "catch_clause" => 0,
        // its message is still evaluated
        "method_invocation" if cx.fm.call_name(n) == Some("fail") => n.kids().into_iter().map(|k| invocations(k, cx)).sum(),
        "method_invocation" | "object_creation_expression" => 1,
        _ => n.kids().into_iter().map(|k| invocations(k, cx)).sum(),
    }
}
