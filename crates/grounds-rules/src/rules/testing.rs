//! Test code: JUnit 4/5, TestNG, AssertJ, Mockito.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
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

fn assertion_owner<'t>(cx: &Ctx<'_, 't>, call: Node<'t>) -> Option<Name> {
    cx.fm.select_method(call).map(|c| c.owner.clone())
}

fn is_testng<'t>(cx: &Ctx<'_, 't>, call: Node<'t>) -> bool {
    assertion_owner(cx, call).is_some_and(|o| o.starts_with("org.testng"))
}

declare_rule!(AssertionArgumentOrder {
    id: "assertion-argument-order",
    name: "Expected and actual swapped in assertEquals",
    category: Testing,
    severity: Warning,
    scope: Test,
    sources: ["sonar:S3415", "pmd:JUnitAssertionsShouldIncludeMessage"],
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
        // JUnit 4 has an optional leading message String
        let (first, second) = if args.len() >= 3 && cx.type_of(args[0]).is_string() && assertion_owner(cx, n).is_some_and(|o| &*o == "org.junit.Assert") { (args[1], args[2]) } else { (args[0], args[1]) };
        let testng = is_testng(cx, n);
        let lit = |x: Node<'t>| cx.fm.constant_value(x).is_some() || matches!(x.unparen().kind(), "null_literal" | "class_literal");
        let (exp, act) = if testng { (second, first) } else { (first, second) };
        if lit(act) && !lit(exp) {
            cx.report(n, if testng { "TestNG expects (actual, expected); the constant should be second" } else { "The expected value should be the first argument" });
        }
    }
}

fn is_assertion_call(name: &str) -> bool {
    // MockMvc `andExpect`, Awaitility `untilAsserted`
    name.starts_with("andExpect") || name == "untilAsserted" || name.starts_with("assert") || name.starts_with("verify") || name.starts_with("expect") || name.starts_with("check") || name.starts_with("should") || name == "fail" || name.starts_with("then") || name == "isEqualTo" || name.starts_with("validate") || name.starts_with("ensure") || name.starts_with("must")
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
        let mut calls = Vec::new();
        find_kind(body, "method_invocation", &mut calls);
        // lambdas count too (assertions in callbacks)
        for c in body.descendants().filter(|d| d.kind() == "method_invocation") {
            calls.push(c);
        }
        let has = calls.iter().any(|c| {
            let name = cx.fm.call_name(*c).unwrap_or("");
            if is_assertion_call(name) {
                return true;
            }
            // helper in the same class that asserts
            if c.field("object").is_none() {
                if let Some(cls_decl) = enclosing_method_decl(n).and_then(owning_type_decl) {
                    return methods_of(cls_decl).iter().any(|m| method_name(*m, cx.src) == name && m.field("body").is_some_and(|b| cx.text(b).contains("assert") || cx.text(b).contains("verify") || cx.text(b).contains("fail(")));
                }
            }
            false
        });
        if !has {
            if let Some(nm) = n.field("name") {
                cx.report(nm, "Test method does not assert anything");
            }
        }
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
        let is_test = anns.iter().any(|a| TEST_ANNOTATIONS.contains(a) || matches!(*a, "BeforeEach" | "AfterEach"));
        if !is_test || !owning_type_decl(n).is_some_and(|d| is_junit5_class(d, cx)) {
            return;
        }
        if n.has_modifier("private") || n.has_modifier("static") {
            if let Some(nm) = n.field("name") {
                cx.report(nm, format!("JUnit 5 ignores {} test methods; this test never runs", if n.has_modifier("private") { "private" } else { "static" }));
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
        let name = cx.fm.call_name(e).unwrap_or("");
        let incomplete = match name {
            "assertThat" | "assertThatThrownBy" | "assertThatCode" | "assertThatObject" => true,
            "as" | "describedAs" | "withFailMessage" | "overridingErrorMessage" => e.field("object").is_some_and(|o| cx.text(o).contains("assertThat")),
            _ => false,
        };
        if incomplete && (cx.fm.select_method(e).is_some_and(|c| c.owner.contains("assertj") || c.owner.contains("Truth")) || cx.fm.stub.imports.iter().any(|i| i.path.contains("assertj"))) {
            cx.report(e, "AssertJ assertion has no check (e.g. `.isEqualTo(...)`); it verifies nothing");
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
        let is_mockito = cx.fm.select_method(e).is_some_and(|c| c.owner.starts_with("org.mockito")) || cx.fm.stub.imports.iter().any(|i| i.path.starts_with("org.mockito"));
        if !is_mockito {
            return;
        }
        match name {
            "when" | "given" => cx.report(e, format!("`{name}(...)` without `thenReturn`/`thenThrow` stubs nothing")),
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
        let args = cx.fm.args_of(n);
        let Some(first) = args.first().map(|a| a.unparen()) else { return };
        let constant = match name {
            "assertTrue" | "assertFalse" => matches!(first.kind(), "true" | "false"),
            "assertNotNull" => matches!(first.kind(), "object_creation_expression" | "string_literal" | "array_creation_expression"),
            "assertNull" => first.kind() == "null_literal",
            "assertEquals" if args.len() >= 2 => is_literal(args[0]) && is_literal(args[1]),
            _ => false,
        };
        if constant {
            // assertTrue(false) as a "should not reach" marker
            if name == "assertTrue" && first.kind() == "false" || name == "assertFalse" && first.kind() == "true" {
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
        let calls: Vec<Node> = body.descendants().filter(|d| matches!(d.kind(), "method_invocation" | "object_creation_expression")).collect();
        // nested calls used only as arguments of the single call are fine
        let top_level = calls.iter().filter(|c| !c.ancestors().take_while(|a| *a != body).any(|a| a.kind() == "argument_list" || a.kind() == "method_invocation" && a != **c && c.parent() != Some(a))).count();
        if body.kind() == "block" && body.kids().len() > 1 || top_level > 1 {
            cx.report(l, "Keep a single throwing invocation inside the assertThrows lambda");
        }
    }
}
