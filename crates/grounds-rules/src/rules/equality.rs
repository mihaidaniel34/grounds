//! Equality, identity and comparison bugs.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_semantic::MethodMatcher;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(StringReferenceEquality),
        Box::new(BoxedReferenceEquality),
        Box::new(EqualsIncompatibleTypes),
        Box::new(ArrayEquals),
        Box::new(ArrayToStringOrHashCode),
        Box::new(CollectionIncompatibleType),
        Box::new(IdenticalOperands),
        Box::new(SelfAssignment),
        Box::new(FloatEquality),
        Box::new(NanComparison),
        Box::new(EqualsNull),
        Box::new(CompareToSpecificValue),
        Box::new(NegatedCompareTo),
        Box::new(EqualsWithoutHashCode),
        Box::new(EqualsOverload),
        Box::new(ClassNameComparison),
        Box::new(LiteralFirstInComparison),
        Box::new(GetClassOnClass),
        Box::new(BigDecimalEquals),
        Box::new(UrlEqualsHashCode),
        Box::new(OptionalNullComparison),
        Box::new(ReferenceEqualityOnValueType),
        Box::new(CompareToReturnsMinValue),
        Box::new(EnumEqualsNull),
        Box::new(InstanceofIncompatible),
    ]
}

fn eq_ops(n: Node) -> bool {
    matches!(op(n), "==" | "!=")
}

declare_rule!(StringReferenceEquality {
    id: "string-reference-equality",
    name: "Strings compared with == or !=",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S4973", "spotbugs:ES_COMPARING_STRINGS_WITH_EQ", "pmd:UseEqualsToCompareStrings", "errorprone:StringEquality", "intellij:StringEquality"],
    kinds: ["binary_expression"],
    description: "`==` compares String references, not contents. Two equal strings are often different objects, so the comparison silently fails depending on how the strings were created. Use `equals()` or `Objects.equals()`.",
});

impl Rule for StringReferenceEquality {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !eq_ops(n) {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        if is_null_literal(l) || is_null_literal(r) {
            return;
        }
        if !(cx.type_of(l).is_string() && cx.type_of(r).is_string()) {
            return;
        }
        // comparing two compile-time constants is deterministic
        if cx.fm.constant_value(l).is_some() && cx.fm.constant_value(r).is_some() {
            return;
        }
        let neg = if op(n) == "!=" { "!" } else { "" };
        let fix = Fix::replace("Use Objects.equals", n, format!("{neg}java.util.Objects.equals({}, {})", cx.text(l), cx.text(r)));
        cx.report_fix(n, format!("Strings compared with `{}`; use `equals()` to compare contents", op(n)), fix);
    }
}

declare_rule!(BoxedReferenceEquality {
    id: "boxed-reference-equality",
    name: "Boxed values compared with == or !=",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S4973", "spotbugs:RC_REF_COMPARISON", "errorprone:BoxedPrimitiveEquality", "intellij:NumberEquality"],
    kinds: ["binary_expression"],
    description: "Comparing `Integer`, `Long` and other boxed types with `==` compares object identity. It happens to work for small cached values (-128..127) and fails for others. Compare with `equals()` or unbox first.",
});

impl Rule for BoxedReferenceEquality {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !eq_ops(n) {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        let (lt, rt) = (cx.type_of(l), cx.type_of(r));
        if lt.is_boxed() && rt.is_boxed() {
            cx.report(n, format!("`{}` and `{}` are boxed; `{}` compares references, not values", lt.display_simple(), rt.display_simple(), op(n)));
        }
    }
}

const OBJECT_EQUALS: MethodMatcher = MethodMatcher::any().named(&["equals"]).arity(1);
const OBJECTS_EQUALS: MethodMatcher = MethodMatcher::statics("java.util.Objects").named(&["equals"]);
const GUAVA_EQUAL: MethodMatcher = MethodMatcher::statics("com.google.common.base.Objects").named(&["equal"]);

/// (receiver/first operand, argument) of an equality call, if any.
fn equality_operands<'t>(n: Node<'t>, cx: &Ctx<'_, 't>) -> Option<(Node<'t>, Node<'t>)> {
    let args = cx.fm.args_of(n);
    if cx.fm.call_matches(n, &OBJECT_EQUALS) {
        let c = cx.fm.select_method(n)?;
        // only Object.equals(Object) signature
        if !c.method.params[0].ty.is_class("java.lang.Object") {
            return None;
        }
        let recv = n.field("object")?;
        if recv.kind() == "super" {
            return None;
        }
        return Some((recv, args[0]));
    }
    if (cx.fm.call_matches(n, &OBJECTS_EQUALS) || cx.fm.call_matches(n, &GUAVA_EQUAL)) && args.len() == 2 {
        return Some((args[0], args[1]));
    }
    None
}

declare_rule!(EqualsIncompatibleTypes {
    id: "equals-incompatible-types",
    name: "equals() between unrelated types",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S2159", "spotbugs:EC_UNRELATED_TYPES", "errorprone:EqualsIncompatibleType", "intellij:EqualsBetweenInconvertibleTypes"],
    kinds: ["method_invocation"],
    description: "`equals()` between two types that can never have a common instance always returns false, which usually means the wrong variable is compared (e.g. an id against an entity, or an Optional against its content).",
});

impl Rule for EqualsIncompatibleTypes {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((a, b)) = equality_operands(n, cx) else { return };
        if is_null_literal(b) || is_null_literal(a) {
            return;
        }
        let (at, bt) = (cx.type_of(a), cx.type_of(b));
        if at.is_array() || bt.is_array() {
            return;
        }
        if equality_unrelated(cx.fm, &at, &bt) {
            cx.report(n, format!("`{}` and `{}` are unrelated types; this equality is always false", at.display_simple(), bt.boxed().display_simple()));
        }
    }
}

declare_rule!(ArrayEquals {
    id: "array-equals",
    name: "Arrays compared with equals()",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["spotbugs:EC_BAD_ARRAY_COMPARE", "errorprone:ArrayEquals", "sonar:S2159"],
    kinds: ["method_invocation"],
    description: "Arrays do not override `equals()`, so `a.equals(b)` and `Objects.equals(a, b)` compare references. Use `Arrays.equals(a, b)` (or `Arrays.deepEquals` for nested arrays).",
});

impl Rule for ArrayEquals {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((a, b)) = equality_operands(n, cx) else { return };
        if cx.type_of(a).is_array() && cx.type_of(b).is_array() {
            let fix = Fix::replace("Use Arrays.equals", n, format!("java.util.Arrays.equals({}, {})", cx.text(a), cx.text(b)));
            cx.report_fix(n, "Array compared with `equals()`, which compares references; use `Arrays.equals()`", fix);
        }
    }
}

declare_rule!(ArrayToStringOrHashCode {
    id: "array-tostring-hashcode",
    name: "toString()/hashCode() called on an array",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S2116", "spotbugs:DMI_INVOKING_TOSTRING_ON_ARRAY", "spotbugs:DMI_INVOKING_HASHCODE_ON_ARRAY", "errorprone:ArrayToString", "errorprone:ArrayHashCode"],
    kinds: ["method_invocation", "binary_expression"],
    description: "Arrays inherit `toString()` and `hashCode()` from Object: you get `[I@1b6d3586` and an identity hash instead of the contents. Use `Arrays.toString()` / `Arrays.hashCode()`.",
});

impl Rule for ArrayToStringOrHashCode {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let is_char_array = |t: &Ty| matches!(t, Ty::Array(e) if matches!(**e, Ty::Prim(Prim::Char)));
        if n.kind() == "binary_expression" {
            if op(n) != "+" || !cx.type_of(n).is_string() {
                return;
            }
            for side in [n.field("left"), n.field("right")].into_iter().flatten() {
                let t = cx.type_of(side);
                if t.is_array() {
                    cx.report(side, "Array concatenated into a String prints its identity, not its contents; use `Arrays.toString()`");
                }
            }
            return;
        }
        let name = cx.fm.call_name(n).unwrap_or("");
        let args = cx.fm.args_of(n);
        match name {
            "toString" | "hashCode" if args.is_empty() => {
                if let Some(o) = n.field("object") {
                    if cx.type_of(o).is_array() {
                        let helper = if name == "toString" { "Arrays.toString" } else { "Arrays.hashCode" };
                        let fix = Fix::replace(format!("Use {helper}"), n, format!("java.util.{helper}({})", cx.text(o)));
                        cx.report_fix(n, format!("`{name}()` on an array uses Object's implementation; use `{helper}()`"), fix);
                    }
                }
            }
            "valueOf" | "hashCode" | "toString" if args.len() == 1 => {
                let t = cx.type_of(args[0]);
                if !t.is_array() || is_char_array(&t) {
                    return;
                }
                let Some(c) = cx.fm.select_method(n) else { return };
                let owner_ok = matches!(&*c.owner, "java.lang.String" | "java.util.Objects");
                if owner_ok && c.method.params[0].ty.is_class("java.lang.Object") {
                    cx.report(args[0], format!("Array passed to `{}.{name}(Object)` uses Object's implementation; use `Arrays.{}()`", simple_name_of(&c.owner), if name == "hashCode" { "hashCode" } else { "toString" }));
                }
            }
            "println" | "print" | "append" if args.len() == 1 => {
                let t = cx.type_of(args[0]);
                if !t.is_array() || is_char_array(&t) {
                    return;
                }
                let Some(c) = cx.fm.select_method(n) else { return };
                let owner_ok = cx.fm.is_subclass(&c.owner, "java.io.PrintStream").is_yes()
                    || cx.fm.is_subclass(&c.owner, "java.io.PrintWriter").is_yes()
                    || cx.fm.is_subclass(&c.owner, "java.lang.AbstractStringBuilder").is_yes()
                    || matches!(&*c.owner, "java.lang.StringBuilder" | "java.lang.StringBuffer");
                if owner_ok && c.method.params[0].ty.is_class("java.lang.Object") {
                    cx.report(args[0], "Array printed via `Object.toString()`; use `Arrays.toString()`");
                }
            }
            _ => {}
        }
    }
}

declare_rule!(CollectionIncompatibleType {
    id: "collection-incompatible-type",
    name: "Collection lookup with an incompatible type",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S2175", "spotbugs:GC_UNRELATED_TYPES", "errorprone:CollectionIncompatibleType", "intellij:SuspiciousMethodCalls"],
    kinds: ["method_invocation"],
    description: "`contains`, `remove`, `get` and friends accept `Object`, so passing a value of an unrelated type compiles but can never match. This is almost always the wrong variable (e.g. a `Long` id passed to a `Map<String, ...>`).",
});

impl Rule for CollectionIncompatibleType {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = cx.fm.call_name(n) else { return };
        let (target, arg_idx) = match name {
            "contains" | "remove" | "indexOf" | "lastIndexOf" => ("java.util.Collection", 0usize),
            "get" | "containsKey" | "getOrDefault" => ("java.util.Map", 0),
            "containsValue" => ("java.util.Map", 0),
            _ => return,
        };
        let args = cx.fm.args_of(n);
        if args.is_empty() || (name != "getOrDefault" && args.len() != 1) {
            return;
        }
        let Some(recv) = n.field("object") else { return };
        let rt = cx.type_of(recv);
        let Some(c) = cx.fm.select_method(n) else { return };
        // must be the Object-accepting overload
        if !c.method.params.first().is_some_and(|p| p.ty.is_class("java.lang.Object")) {
            return;
        }
        let elem = match name {
            "remove" | "contains" | "indexOf" | "lastIndexOf" if target == "java.util.Collection" => {
                if let Some(m) = cx.fm.as_super(&rt, "java.util.Map") {
                    if name == "remove" {
                        m.arg(0).cloned()
                    } else {
                        return;
                    }
                } else {
                    cx.fm.as_super(&rt, "java.util.Collection").and_then(|t| t.arg(0).cloned())
                }
            }
            _ => cx.fm.as_super(&rt, "java.util.Map").and_then(|t| t.arg(if name == "containsValue" { 1 } else { 0 }).cloned()),
        };
        let Some(elem) = elem else { return };
        let elem = elem.upper_bound();
        if elem.mentions_type_var() || elem.has_unknown() {
            return;
        }
        let a = cx.type_of(args[arg_idx]);
        if a.has_unknown() || is_null_literal(args[arg_idx]) {
            return;
        }
        if equality_unrelated(cx.fm, &elem, &a) {
            cx.report(args[arg_idx], format!("`{name}()` on a collection of `{}` with an argument of type `{}`; this can never match", elem.display_simple(), a.boxed().display_simple()));
        }
    }
}

declare_rule!(IdenticalOperands {
    id: "identical-operands",
    name: "Identical expressions on both sides of an operator",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S1764", "spotbugs:SA_FIELD_SELF_COMPARISON", "spotbugs:SA_LOCAL_SELF_COMPARISON", "errorprone:SelfEquals", "intellij:EqualsWithItself"],
    kinds: ["binary_expression", "method_invocation"],
    description: "Using the same expression on both sides of `==`, `&&`, `-`, `equals()` etc. is either pointless or a copy-paste error where one side should be different.",
});

impl Rule for IdenticalOperands {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.kind() == "method_invocation" {
            if cx.fm.call_name(n) == Some("equals") {
                let args = cx.fm.args_of(n);
                if let (Some(o), 1) = (n.field("object"), args.len()) {
                    if o.kind() != "super" && same_pure_expr(o, args[0], cx.src) {
                        cx.report(n, "Object compared with itself via `equals()`");
                    }
                }
            }
            return;
        }
        let o = op(n);
        if !matches!(o, "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||" | "&" | "|" | "^" | "-" | "/" | "%") {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        if !same_pure_expr(l, r, cx.src) || is_literal(l) {
            return;
        }
        let t = cx.type_of(l);
        // `x != x` is the idiomatic NaN check
        if matches!(o, "!=" | "==") && matches!(t.unboxed(), Some(Prim::Float | Prim::Double)) {
            cx.report(n, format!("`{}` with identical operands: use `Double.isNaN()` / `Float.isNaN()` to test for NaN", o));
            return;
        }
        // 1 << 1 style bit masks are fine for some ops; division/subtraction of the same is a bug
        cx.report(n, format!("Identical expressions on both sides of `{o}`"));
    }
}

declare_rule!(SelfAssignment {
    id: "self-assignment",
    name: "Variable assigned to itself",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S1656", "spotbugs:SA_LOCAL_SELF_ASSIGNMENT", "spotbugs:SA_FIELD_SELF_ASSIGNMENT", "errorprone:SelfAssignment", "intellij:SillyAssignment"],
    kinds: ["assignment_expression"],
    description: "Assigning a variable to itself has no effect. In constructors and setters this usually means a parameter name does not match the field (`this.name = name` where the parameter is called `newName`).",
});

impl Rule for SelfAssignment {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if op(n) != "=" {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        let r = r.unparen();
        let same_text = normalized(l, cx.src) == normalized(r, cx.src) && !has_side_effects(l);
        let same_field = match (cx.fm.field_ref(l), cx.fm.field_ref(r)) {
            // a volatile write publishes earlier mutations: `state = state` can be intended
            (Some(a), _) if a.field.flags.has(Flags::VOLATILE) => false,
            (Some(a), Some(b)) => a.owner == b.owner && a.field.name == b.field.name && (l.kind() == "identifier" || l.field("object").is_some_and(|o| o.kind() == "this")) && (r.kind() == "identifier" || r.field("object").is_some_and(|o| o.kind() == "this")),
            _ => false,
        };
        let same_local = match (cx.fm.local_ref(l), cx.fm.local_ref(r)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        if same_local || same_field || (same_text && l.kind() == "identifier" && cx.fm.local_ref(l).is_some()) {
            cx.report(n, format!("`{}` is assigned to itself", cx.text(l)));
        }
    }
}

declare_rule!(FloatEquality {
    id: "float-equality",
    name: "Floating point values compared with == or !=",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S1244", "spotbugs:FE_FLOATING_POINT_EQUALITY", "intellij:FloatingPointEquality"],
    kinds: ["binary_expression"],
    description: "Floating point arithmetic is inexact: `0.1 + 0.2 == 0.3` is false. Compare with a tolerance (`Math.abs(a - b) < EPS`) or use `BigDecimal` for exact decimal values.",
});

impl Rule for FloatEquality {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !eq_ops(n) {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        let fl = |t: &Ty| matches!(t.unboxed(), Some(Prim::Float | Prim::Double));
        let (lt, rt) = (cx.type_of(l), cx.type_of(r));
        if !(fl(&lt) || fl(&rt)) || !(lt.is_numeric() && rt.is_numeric()) {
            return;
        }
        // exact comparisons against zero/integral constants and NaN checks are common and intended
        for side in [l, r] {
            if let Some(c) = cx.fm.constant_value(side) {
                if c.as_f64().is_some_and(|v| v.fract() == 0.0 || v.is_infinite()) {
                    return;
                }
            }
            let t = cx.text(side);
            if t.ends_with("MAX_VALUE") || t.ends_with("MIN_VALUE") || t.ends_with("INFINITY") || t.ends_with("NaN") {
                return;
            }
        }
        if same_pure_expr(l, r, cx.src) {
            return;
        }
        if let Some(m) = enclosing_method_decl(n) {
            if matches!(method_name(m, cx.src), "equals" | "hashCode" | "compareTo" | "compare") {
                return;
            }
        }
        cx.report(n, "Floating point values compared for exact equality; compare with a tolerance instead");
    }
}

declare_rule!(NanComparison {
    id: "nan-comparison",
    name: "Comparison with NaN",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["spotbugs:FE_TEST_IF_EQUAL_TO_NOT_A_NUMBER", "errorprone:EqualsNaN"],
    kinds: ["binary_expression"],
    description: "NaN is not equal to anything, including itself: `x == Double.NaN` is always false and `x != Double.NaN` always true. Use `Double.isNaN(x)`.",
});

impl Rule for NanComparison {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(op(n), "==" | "!=" | "<" | ">" | "<=" | ">=") {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        for side in [l, r] {
            if let Some(f) = cx.fm.field_ref(side.unparen()) {
                if &*f.field.name == "NaN" && matches!(&*f.owner, "java.lang.Double" | "java.lang.Float") {
                    let other = if side == l { r } else { l };
                    let fix = Fix::replace("Use isNaN", n, format!("{}{}.isNaN({})", if op(n) == "!=" { "!" } else { "" }, simple_name_of(&f.owner), cx.text(other)));
                    cx.report_fix(n, format!("Comparison with NaN is always {}; use `isNaN()`", if op(n) == "!=" { "true" } else { "false" }), fix);
                    return;
                }
            }
        }
    }
}

declare_rule!(EqualsNull {
    id: "equals-null",
    name: "equals(null) call",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["spotbugs:EC_NULL_ARG", "pmd:EqualsNull"],
    kinds: ["method_invocation"],
    description: "`x.equals(null)` is always false for a correct `equals` implementation (or throws if `x` is null). Use `x == null`.",
});

impl Rule for EqualsNull {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("equals") {
            return;
        }
        let args = cx.fm.args_of(n);
        if args.len() == 1 && is_null_literal(args[0]) {
            if let Some(o) = n.field("object") {
                let fix = Fix::replace("Compare with ==", n, format!("{} == null", cx.text(o)));
                cx.report_fix(n, "`equals(null)` is always false; use `== null`", fix);
            }
        }
    }
}

const COMPARE_TO: MethodMatcher = MethodMatcher::instance("java.lang.Comparable").named(&["compareTo"]).arity(1);
const COMPARATOR_COMPARE: MethodMatcher = MethodMatcher::instance("java.util.Comparator").named(&["compare"]).arity(2);

fn is_comparison_call<'t>(n: Node<'t>, cx: &Ctx<'_, 't>) -> bool {
    let n = n.unparen();
    n.kind() == "method_invocation" && (cx.fm.call_matches(n, &COMPARE_TO) || cx.fm.call_matches(n, &COMPARATOR_COMPARE) || is_call_named(n, cx.src, &["compareTo", "compare"]) && cx.type_of(n) == Ty::int())
}

declare_rule!(CompareToSpecificValue {
    id: "compareto-specific-value",
    name: "compareTo() result compared to a specific value",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S2200", "spotbugs:RV_CHECK_COMPARETO_FOR_SPECIFIC_RETURN_VALUE"],
    kinds: ["binary_expression"],
    description: "`compareTo` and `compare` only guarantee the sign of the result. Checking `== 1` or `== -1` breaks with implementations returning other magnitudes (e.g. String). Compare against 0 instead.",
});

impl Rule for CompareToSpecificValue {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !eq_ops(n) {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        for (call, k) in [(l, r), (r, l)] {
            if let Some(v) = cx.fm.int_value(k) {
                if v != 0 && is_comparison_call(call, cx) {
                    cx.report(n, format!("`{}` only guarantees the sign; comparing with {v} is unreliable", cx.fm.call_name(call.unparen()).unwrap_or("compareTo")));
                    return;
                }
            }
        }
    }
}

declare_rule!(NegatedCompareTo {
    id: "negated-compareto",
    name: "Result of compareTo() negated",
    category: Bug,
    severity: Warning,
    sources: ["spotbugs:RV_NEGATING_RESULT_OF_COMPARETO"],
    kinds: ["unary_expression"],
    description: "`compareTo` may return `Integer.MIN_VALUE`, whose negation is itself, so `-a.compareTo(b)` does not reliably reverse the order. Swap the operands instead: `b.compareTo(a)`.",
});

impl Rule for NegatedCompareTo {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if op(n) != "-" {
            return;
        }
        if let Some(o) = n.field("operand") {
            if is_comparison_call(o, cx) {
                cx.report(n, "Negating a comparison result fails for Integer.MIN_VALUE; swap the operands instead");
            }
        }
    }
}

fn class_methods<'t>(decl: Node<'t>, src: &str) -> Vec<(Node<'t>, String, usize)> {
    methods_of(decl).into_iter().map(|m| (m, method_name(m, src).to_string(), param_count(m))).collect()
}

declare_rule!(EqualsWithoutHashCode {
    id: "equals-without-hashcode",
    name: "equals() and hashCode() not overridden together",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S1206", "spotbugs:HE_EQUALS_NO_HASHCODE", "spotbugs:HE_HASHCODE_NO_EQUALS", "pmd:OverrideBothEqualsAndHashcode", "intellij:EqualsAndHashcode"],
    kinds: ["class_declaration", "record_declaration", "enum_declaration"],
    description: "Equal objects must have equal hash codes. Overriding only one of `equals()`/`hashCode()` breaks `HashMap`, `HashSet` and every other hash-based collection.",
});

impl Rule for EqualsWithoutHashCode {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let ms = class_methods(n, cx.src);
        let eq = ms.iter().find(|(m, name, pc)| name == "equals" && *pc == 1 && m.field("parameters").is_some_and(|p| {
            p.kids().first().and_then(|fp| fp.field("type")).is_some_and(|t| matches!(type_simple_name(t, cx.src), "Object"))
        }));
        let hc = ms.iter().find(|(_, name, pc)| name == "hashCode" && *pc == 0);
        // Lombok @EqualsAndHashCode/@Data/@Value generate both
        if cx.fm.class_of_decl(n).is_some_and(|c| c.methods.iter().any(|m| m.generated_by.is_some() && matches!(&*m.name, "equals" | "hashCode"))) {
            return;
        }
        match (eq, hc) {
            // records generate hashCode(); a class may inherit a matching one
            // (an AbstractList subclass delegating equals to super)
            (Some(_), None) if n.kind() == "record_declaration" => {}
            (Some((m, _, _)), None) => {
                let inherits_hashcode = cx.fm.class_of_decl(n).is_some_and(|c| {
                    c.superclass.as_ref().and_then(|s| s.class_name()).is_some_and(|sn| cx.fm.overrides_object_method(sn, "hashCode", 0))
                        || c.superclass.as_ref().is_some_and(|s| s.is_unknown())
                });
                if inherits_hashcode {
                    return;
                }
                if let Some(name) = m.field("name") {
                    cx.report(name, "Class overrides `equals()` but not `hashCode()`");
                }
            }
            (None, Some(_)) if n.kind() == "record_declaration" => {}
            (None, Some((m, _, _))) => {
                let inherits_equals = cx.fm.class_of_decl(n).is_some_and(|c| {
                    c.superclass.as_ref().and_then(|s| s.class_name()).is_some_and(|sn| cx.fm.overrides_object_method(sn, "equals", 1))
                        || c.superclass.as_ref().is_some_and(|s| s.is_unknown())
                });
                if inherits_equals {
                    return;
                }
                if let Some(name) = m.field("name") {
                    cx.report(name, "Class overrides `hashCode()` but not `equals()`");
                }
            }
            _ => {}
        }
    }
}

declare_rule!(EqualsOverload {
    id: "equals-overload",
    name: "equals() overloaded instead of overridden",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S1201", "spotbugs:EQ_SELF_NO_OBJECT", "errorprone:NonOverridingEquals", "intellij:CovariantEquals"],
    kinds: ["method_declaration"],
    description: "`boolean equals(MyType other)` does not override `Object.equals(Object)`; collections and most libraries call the Object version, which still uses identity. Declare `equals(Object)`.",
});

impl Rule for EqualsOverload {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if method_name(n, cx.src) != "equals" || param_count(n) != 1 || n.has_modifier("static") {
            return;
        }
        let Some(t) = n.field("type") else { return };
        if t.text(cx.src) != "boolean" {
            return;
        }
        let pt = n.field("parameters").and_then(|p| p.kids().first().copied()).and_then(|p| p.field("type"));
        let Some(pt) = pt else { return };
        if type_simple_name(pt, cx.src) == "Object" {
            return;
        }
        // does the class also define equals(Object)?
        if let Some(decl) = owning_type_decl(n) {
            let has_object = methods_of(decl).iter().any(|m| {
                method_name(*m, cx.src) == "equals"
                    && m.field("parameters").and_then(|p| p.kids().first().copied()).and_then(|p| p.field("type")).is_some_and(|t| type_simple_name(t, cx.src) == "Object")
            });
            if has_object {
                return;
            }
        }
        if let Some(name) = n.field("name") {
            cx.report(name, format!("`equals({})` overloads rather than overrides `Object.equals(Object)`", pt.text(cx.src)));
        }
    }
}

declare_rule!(ClassNameComparison {
    id: "class-name-comparison",
    name: "Classes compared by name",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S1872", "spotbugs:EQ_COMPARING_CLASS_NAMES"],
    kinds: ["method_invocation"],
    description: "Comparing `getClass().getName()` or `getSimpleName()` with a string is fragile: different class loaders and same-named classes in other packages match too. Use `instanceof` or compare `Class` objects.",
});

impl Rule for ClassNameComparison {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("equals") {
            return;
        }
        let args = cx.fm.args_of(n);
        if args.len() != 1 {
            return;
        }
        let (Some(recv), arg) = (n.field("object"), args[0]) else { return };
        for (a, b) in [(recv, arg), (arg, recv)] {
            let a = a.unparen();
            if is_call_named(a, cx.src, &["getName", "getSimpleName"]) && cx.fm.string_value(b).is_some() {
                if let Some(o) = a.field("object") {
                    if cx.type_of(o).class_name() == Some("java.lang.Class") {
                        cx.report(n, "Class compared by name; use `instanceof` or compare `Class` objects");
                        return;
                    }
                }
            }
        }
    }
}

declare_rule!(LiteralFirstInComparison {
    id: "literal-first-in-comparison",
    name: "String literal should be the receiver of equals()",
    category: Style,
    severity: Info,
    sources: ["sonar:S1132", "pmd:LiteralsFirstInComparisons", "intellij:LiteralAsArgToStringEquals"],
    kinds: ["method_invocation"],
    description: "`variable.equals(\"literal\")` throws when the variable is null; `\"literal\".equals(variable)` returns false. Put the literal first.",
});

impl Rule for LiteralFirstInComparison {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = cx.fm.call_name(n) else { return };
        if !matches!(name, "equals" | "equalsIgnoreCase" | "contentEquals" | "compareTo" | "compareToIgnoreCase") {
            return;
        }
        let args = cx.fm.args_of(n);
        let Some(o) = n.field("object") else { return };
        if args.len() != 1 || args[0].unparen().kind() != "string_literal" || o.unparen().kind() == "string_literal" {
            return;
        }
        if !cx.type_of(o).is_string() {
            return;
        }
        // receiver that cannot be null: method chains on literals, `this`
        if matches!(o.kind(), "this" | "string_literal") {
            return;
        }
        if matches!(name, "equals" | "equalsIgnoreCase") {
            let fix = Fix::replace("Put the literal first", n, format!("{}.{name}({})", cx.text(args[0]), cx.text(o)));
            cx.report_fix(n, "Put the string literal first to avoid a NullPointerException", fix);
        } else {
            cx.report(n, "Put the string literal first to avoid a NullPointerException");
        }
    }
}

declare_rule!(GetClassOnClass {
    id: "getclass-on-class",
    name: "getClass() called on a Class object",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["errorprone:GetClassOnClass"],
    kinds: ["method_invocation"],
    description: "Calling `getClass()` on a `Class` returns `Class<Class>`, not the class it represents. Use the `Class` object directly.",
});

impl Rule for GetClassOnClass {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("getClass") || !cx.fm.args_of(n).is_empty() {
            return;
        }
        if let Some(o) = n.field("object") {
            if cx.type_of(o).class_name() == Some("java.lang.Class") {
                cx.report(n, "`getClass()` on a Class object returns `java.lang.Class`; use the Class directly");
            }
        }
    }
}

declare_rule!(BigDecimalEquals {
    id: "bigdecimal-equals",
    name: "BigDecimal compared with equals()",
    category: Bug,
    severity: Warning,
    sources: ["errorprone:BigDecimalEquals", "sonar:S2111"],
    kinds: ["method_invocation"],
    description: "`BigDecimal.equals` also compares scale: `new BigDecimal(\"2.0\").equals(new BigDecimal(\"2.00\"))` is false. Use `compareTo(...) == 0` for numeric equality.",
});

impl Rule for BigDecimalEquals {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("equals") {
            return;
        }
        let args = cx.fm.args_of(n);
        let Some(o) = n.field("object") else { return };
        if args.len() == 1 && cx.type_of(o).is_class("java.math.BigDecimal") && cx.type_of(args[0]).is_class("java.math.BigDecimal") {
            if enclosing_method_decl(n).is_some_and(|m| matches!(method_name(m, cx.src), "equals" | "hashCode")) {
                return;
            }
            let fix = Fix::replace("Use compareTo", n, format!("{}.compareTo({}) == 0", cx.text(o), cx.text(args[0])));
            cx.report_fix(n, "`BigDecimal.equals()` also compares scale; use `compareTo() == 0`", fix);
        }
    }
}

declare_rule!(UrlEqualsHashCode {
    id: "url-equals-hashcode",
    name: "URL equals()/hashCode() or URL in hash collections",
    category: Performance,
    severity: Warning,
    sources: ["sonar:S2112", "spotbugs:DMI_BLOCKING_METHODS_ON_URL", "spotbugs:DMI_COLLECTION_OF_URLS", "errorprone:URLEqualsHashCode"],
    kinds: ["method_invocation", "object_creation_expression"],
    description: "`java.net.URL.equals()` and `hashCode()` resolve host names over the network, are slow, and give different answers depending on DNS. Use `java.net.URI` instead, also as Map/Set keys.",
});

impl Rule for UrlEqualsHashCode {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.kind() == "method_invocation" {
            let Some(name) = cx.fm.call_name(n) else { return };
            if !matches!(name, "equals" | "hashCode") {
                return;
            }
            if let Some(o) = n.field("object") {
                if cx.type_of(o).is_class("java.net.URL") {
                    cx.report(n, format!("`URL.{name}()` performs DNS lookups; use `java.net.URI`"));
                }
            }
            return;
        }
        let t = cx.type_of(n);
        let Some(cn) = t.class_name() else { return };
        let is_hash = ["java.util.HashSet", "java.util.HashMap", "java.util.LinkedHashMap", "java.util.LinkedHashSet", "java.util.Hashtable"].contains(&cn);
        if is_hash && t.arg(0).is_some_and(|a| a.is_class("java.net.URL")) {
            cx.report(n, "Hash collection keyed by `java.net.URL` calls `URL.hashCode()`/`equals()`, which resolve host names; use `java.net.URI`");
        }
    }
}

declare_rule!(OptionalNullComparison {
    id: "optional-null-comparison",
    name: "Optional compared with or assigned null",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S2789", "spotbugs:NP_OPTIONAL_RETURN_NULL", "errorprone:OptionalEquality"],
    kinds: ["binary_expression", "return_statement", "variable_declarator", "assignment_expression"],
    description: "An `Optional` variable should never itself be null; that defeats its purpose and forces callers to check twice. Use `Optional.empty()` and `isPresent()`.",
});

fn is_optional(t: &Ty) -> bool {
    matches!(t.class_name(), Some("java.util.Optional" | "java.util.OptionalInt" | "java.util.OptionalLong" | "java.util.OptionalDouble" | "com.google.common.base.Optional"))
}

impl Rule for OptionalNullComparison {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        match n.kind() {
            "binary_expression" => {
                if !eq_ops(n) {
                    return;
                }
                let Some((l, r)) = lr(n) else { return };
                let other = if is_null_literal(r) {
                    l
                } else if is_null_literal(l) {
                    r
                } else {
                    return;
                };
                // fields used as lazily computed caches (`null` = not computed) are a common, deliberate pattern
                if cx.fm.field_ref(other.unparen()).is_some() {
                    return;
                }
                if let Some(i) = cx.fm.local_ref(other.unparen()) {
                    let d = &cx.fm.binder.decls[i];
                    if d.decl_node.field("value").is_some_and(|v| cx.fm.field_ref(v.unparen()).is_some()) {
                        return;
                    }
                }
                if is_optional(&cx.type_of(other)) {
                    cx.report(n, "Optional compared with null; an Optional should never be null, use `isPresent()`/`isEmpty()`");
                }
            }
            "return_statement" => {
                if let Some(v) = n.first_kid() {
                    if is_null_literal(v) && is_optional(&cx.fm.return_type_at(n)) {
                        let fix = Fix::replace("Return Optional.empty()", v, "java.util.Optional.empty()");
                        cx.report_fix(n, "Method returning Optional returns null; return `Optional.empty()`", fix);
                    }
                }
            }
            "variable_declarator" | "assignment_expression" => {
                let v = if n.kind() == "variable_declarator" { n.field("value") } else { n.field("right") };
                let Some(v) = v else { return };
                if !is_null_literal(v) {
                    return;
                }
                let target_is_field = if n.kind() == "assignment_expression" {
                    n.field("left").is_some_and(|l| cx.fm.field_ref(l).is_some())
                } else {
                    n.parent().is_some_and(|p| p.kind() == "field_declaration")
                };
                if target_is_field {
                    return;
                }
                if is_optional(&cx.fm.expected_type(v)) {
                    cx.report(n, "Optional variable assigned null; use `Optional.empty()`");
                }
            }
            _ => {}
        }
    }
}

declare_rule!(ReferenceEqualityOnValueType {
    id: "reference-equality-value-type",
    name: "Objects with value semantics compared with ==",
    category: Bug,
    severity: Info,
    sources: ["errorprone:ReferenceEquality", "pmd:CompareObjectsWithEquals", "intellij:ObjectEquality"],
    kinds: ["binary_expression"],
    description: "The compared type overrides `equals()`, signalling value semantics, but `==` compares identity. Two equal values are often distinct objects. Use `equals()` unless identity is really intended.",
});

impl Rule for ReferenceEqualityOnValueType {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !eq_ops(n) {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        if is_null_literal(l) || is_null_literal(r) || l.unparen().kind() == "this" || r.unparen().kind() == "this" {
            return;
        }
        let (lt, rt) = (cx.type_of(l), cx.type_of(r));
        let (Some(ln), Some(rn)) = (lt.class_name(), rt.class_name()) else { return };
        if lt.is_string() || lt.is_boxed() || rt.is_boxed() {
            return; // covered by dedicated rules
        }
        if ln != rn && !cx.fm.is_subclass(ln, rn).is_yes() && !cx.fm.is_subclass(rn, ln).is_yes() {
            return;
        }
        let Some(c) = cx.fm.class(ln) else { return };
        if c.is_enum() || c.is_interface() || ln == "java.lang.Class" || ln == "java.lang.Object" {
            return;
        }
        if !cx.fm.overrides_object_method(ln, "equals", 1) {
            return;
        }
        // comparing against a shared constant instance is a deliberate fast path
        for side in [l, r] {
            if cx.fm.field_ref(side.unparen()).is_some_and(|f| f.field.flags.is_static() && f.field.flags.is_final()) {
                return;
            }
        }
        if let Some(m) = enclosing_method_decl(n) {
            if matches!(method_name(m, cx.src), "equals" | "hashCode" | "compareTo" | "compare") {
                return;
            }
        }
        cx.report(n, format!("`{}` overrides `equals()`; `{}` compares identity", simple_name_of(ln), op(n)));
    }
}

declare_rule!(CompareToReturnsMinValue {
    id: "compareto-min-value",
    name: "compareTo() returns Integer.MIN_VALUE",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2167", "spotbugs:CO_COMPARETO_RESULTS_MIN_VALUE"],
    kinds: ["return_statement"],
    description: "Callers commonly negate comparison results; `-Integer.MIN_VALUE` is still `Integer.MIN_VALUE`, inverting the meaning. Return -1 instead.",
});

impl Rule for CompareToReturnsMinValue {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(v) = n.first_kid() else { return };
        let Some(m) = enclosing_method_decl(n) else { return };
        if !matches!(method_name(m, cx.src), "compareTo" | "compare") {
            return;
        }
        if cx.fm.int_value(v) == Some(i32::MIN as i64) {
            cx.report(n, "Returning Integer.MIN_VALUE from a comparison; negating it overflows. Return -1");
        }
    }
}

declare_rule!(EnumEqualsNull {
    id: "enum-compared-with-equals",
    name: "Enum compared with equals()",
    category: Style,
    severity: Info,
    sources: ["sonar:S4551", "pmd:CompareObjectsWithEquals"],
    kinds: ["method_invocation"],
    description: "Enum constants are singletons: `==` is null-safe, faster and checked at compile time for type compatibility, while `equals()` throws on a null receiver.",
});

impl Rule for EnumEqualsNull {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("equals") {
            return;
        }
        let args = cx.fm.args_of(n);
        let Some(o) = n.field("object") else { return };
        if args.len() != 1 || o.kind() == "super" {
            return;
        }
        let (ot, at) = (cx.type_of(o), cx.type_of(args[0]));
        let is_enum = |t: &Ty| t.class_name().and_then(|c| cx.fm.class(c)).is_some_and(|c| c.is_enum());
        if is_enum(&ot) && is_enum(&at) {
            let fix = Fix::replace("Use ==", n, format!("{} == {}", cx.text(o), cx.text(args[0])));
            cx.report_fix(n, "Compare enum values with `==`", fix);
        }
    }
}

declare_rule!(InstanceofIncompatible {
    id: "instanceof-always-false",
    name: "instanceof check that can never succeed",
    category: Bug,
    severity: Error,
    sources: ["sonar:S2589", "spotbugs:BC_IMPOSSIBLE_INSTANCEOF"],
    kinds: ["instanceof_expression"],
    description: "The expression's static type and the checked type can never share an instance (unrelated classes, or a final class not implementing the interface), so the check is always false.",
});

impl Rule for InstanceofIncompatible {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let (Some(l), Some(r)) = (n.field("left"), n.field("right")) else { return };
        let lt = cx.type_of(l);
        let rt = cx.fm.resolve_type_node(r);
        if lt.is_array() || rt.is_array() {
            return;
        }
        if definitely_unrelated(cx.fm, &lt, &rt) {
            cx.report(n, format!("`{}` can never be an instance of `{}`", lt.display_simple(), rt.display_simple()));
        }
    }
}
