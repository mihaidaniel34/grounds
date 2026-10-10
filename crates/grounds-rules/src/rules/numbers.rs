//! Arithmetic, numeric conversion and random-number bugs.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_semantic::MethodMatcher;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(BadShiftAmount),
        Box::new(IntegerDivisionInFloatingContext),
        Box::new(IntOverflowInLongContext),
        Box::new(ConstantOverflow),
        Box::new(AbsOfRandomOrHash),
        Box::new(RandomCastToInt),
        Box::new(RandomUsedOnce),
        Box::new(BigDecimalFromDouble),
        Box::new(OddCheckWithRemainder),
        Box::new(ByteComparisonOutOfRange),
        Box::new(NarrowingCompoundAssignment),
        Box::new(OverwrittenIncrement),
        Box::new(ReversedAssignmentOperator),
        Box::new(LowercaseLongLiteral),
        Box::new(OctalLiteral),
        Box::new(IntCastToFloatingForRounding),
        Box::new(CharArithmeticInConcat),
        Box::new(RemainderOfOne),
        Box::new(SignedIntToHex),
        Box::new(MathMinMaxSwapped),
        Box::new(ImpossibleBitmaskComparison),
        Box::new(LossyFloatToIntCast),
    ]
}

declare_rule!(BadShiftAmount {
    id: "bad-shift-amount",
    name: "Shift amount out of range",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S2183", "spotbugs:ICAST_BAD_SHIFT_AMOUNT", "errorprone:BadShiftAmount"],
    kinds: ["binary_expression", "assignment_expression"],
    description: "Java masks shift distances: shifting an `int` by 32 shifts by 0 and by 33 shifts by 1. Shifting by a constant outside 0..31 (0..63 for `long`) is always a mistake, usually a missing `L` suffix or cast.",
});

impl Rule for BadShiftAmount {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let o = op(n);
        let shift = matches!(o, "<<" | ">>" | ">>>" | "<<=" | ">>=" | ">>>=");
        if !shift {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        let Some(amount) = cx.fm.int_value(r) else { return };
        let lt = cx.type_of(l).unboxed();
        let bits = match lt {
            Some(Prim::Long) => 64,
            Some(p) if p.is_integral() => 32,
            _ => return,
        };
        if amount < 0 || amount >= bits {
            cx.report(n, format!("Shift by {amount} on a {bits}-bit value is masked to {}", amount.rem_euclid(bits)));
        }
    }
}

fn is_integral_ty(t: &Ty) -> bool {
    matches!(t.unboxed(), Some(p) if p.is_integral())
}

fn is_floating_ty(t: &Ty) -> bool {
    matches!(t.unboxed(), Some(Prim::Float | Prim::Double))
}

/// The type an expression is converted to by its immediate context.
fn context_type<'t>(n: Node<'t>, cx: &Ctx<'_, 't>) -> Ty {
    let mut child = n;
    let mut p = n.parent();
    while let Some(pp) = p {
        if pp.kind() == "parenthesized_expression" {
            child = pp;
            p = pp.parent();
            continue;
        }
        break;
    }
    let Some(p) = p else { return Ty::Unknown };
    match p.kind() {
        "cast_expression" => cx.type_of(p),
        "binary_expression" => {
            // arithmetic with a floating operand promotes the division result
            let other = if p.field("left") == Some(child) { p.field("right") } else { p.field("left") };
            if matches!(op(p), "+" | "-" | "*" | "/") {
                if let Some(o) = other {
                    let ot = cx.type_of(o);
                    if is_floating_ty(&ot) {
                        return ot.unboxed().map(Ty::Prim).unwrap_or(Ty::Unknown);
                    }
                }
            }
            Ty::Unknown
        }
        _ => cx.fm.expected_type(child),
    }
}

declare_rule!(IntegerDivisionInFloatingContext {
    id: "integer-division-in-floating-context",
    name: "Integer division result used as floating point",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S2184", "spotbugs:ICAST_IDIV_CAST_TO_DOUBLE", "errorprone:IntegerDivision"],
    kinds: ["binary_expression"],
    description: "`double ratio = done / total;` performs integer division first and only then converts, so 1/2 becomes 0.0. Cast an operand: `(double) done / total`.",
});

impl Rule for IntegerDivisionInFloatingContext {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if op(n) != "/" {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        if !(is_integral_ty(&cx.type_of(l)) && is_integral_ty(&cx.type_of(r))) {
            return;
        }
        if let (Some(a), Some(b)) = (cx.fm.int_value(l), cx.fm.int_value(r)) {
            if b != 0 && a % b == 0 {
                return;
            }
        }
        let ctx = context_type(n, cx);
        // the floating result is truncated again by an integral cast: precision does not matter
        let mut up = n;
        while let Some(p) = up.parent() {
            match p.kind() {
                "parenthesized_expression" | "binary_expression" => up = p,
                "cast_expression" => {
                    if matches!(cx.type_of(p).prim(), Some(pr) if pr.is_integral()) {
                        return;
                    }
                    break;
                }
                _ => break,
            }
        }
        if is_floating_ty(&ctx) {
            cx.report(n, format!("Integer division in a `{}` context truncates before converting; cast an operand to `{}`", ctx, ctx.unboxed().map(|p| p.keyword()).unwrap_or("double")));
        }
    }
}

declare_rule!(IntOverflowInLongContext {
    id: "int-overflow-in-long-context",
    name: "int arithmetic may overflow before widening to long",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2184", "spotbugs:ICAST_INTEGER_MULTIPLY_CAST_TO_LONG", "errorprone:IntLongMath"],
    kinds: ["binary_expression"],
    description: "`long millis = days * 24 * 3600 * 1000;` multiplies in `int` and overflows before the result is widened. Make the first operand long: `days * 24L * 3600 * 1000`.",
});

impl Rule for IntOverflowInLongContext {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(op(n), "*" | "<<") {
            return;
        }
        // report on the outermost arithmetic expression only
        if let Some(p) = n.parent_unparen() {
            if p.kind() == "binary_expression" && matches!(op(p), "*" | "+" | "-" | "<<") && cx.type_of(p) == Ty::Prim(Prim::Int) {
                return;
            }
        }
        if cx.type_of(n) != Ty::Prim(Prim::Int) {
            return;
        }
        let ctx = context_type(n, cx);
        if ctx.unboxed() != Some(Prim::Long) {
            return;
        }
        // constant expressions that fit are fine
        if let Some(Constant::Int(v)) = cx.fm.constant_value(n) {
            let _ = v;
            if !overflows_i32(n, cx) {
                return;
            }
        }
        let Some((l, r)) = lr(n) else { return };
        // at least one non-constant operand, or a constant that overflows
        if cx.fm.constant_value(l).is_some() && cx.fm.constant_value(r).is_some() && !overflows_i32(n, cx) {
            return;
        }
        // small multipliers (x * 2) rarely overflow in practice; require a large constant factor or two variables
        let interesting = match (cx.fm.int_value(l), cx.fm.int_value(r)) {
            (None, None) => true,
            (Some(a), None) | (None, Some(a)) => a.abs() >= 1000 || l.unparen().kind() == "binary_expression",
            (Some(_), Some(_)) => overflows_i32(n, cx),
        };
        if !interesting {
            return;
        }
        cx.report(n, "int arithmetic evaluated before widening to long may overflow; make an operand long (e.g. `1000L`)");
    }
}

/// Evaluate a constant integer expression without 32-bit wrapping and check
/// whether the exact result leaves the int range.
fn overflows_i32<'t>(n: Node<'t>, cx: &Ctx<'_, 't>) -> bool {
    fn exact<'t>(n: Node<'t>, cx: &Ctx<'_, 't>) -> Option<i128> {
        let n = n.unparen();
        if n.kind() == "binary_expression" {
            let (l, r) = lr(n)?;
            let (a, b) = (exact(l, cx)?, exact(r, cx)?);
            return match op(n) {
                "*" => a.checked_mul(b),
                "+" => a.checked_add(b),
                "-" => a.checked_sub(b),
                "<<" if (0..64).contains(&b) => a.checked_mul(1i128 << b),
                "/" if b != 0 => Some(a / b),
                _ => None,
            };
        }
        if cx.type_of(n).unboxed() == Some(Prim::Long) {
            return None;
        }
        cx.fm.int_value(n).map(|v| v as i128)
    }
    match exact(n, cx) {
        Some(v) => v > i32::MAX as i128 || v < i32::MIN as i128,
        None => false,
    }
}

declare_rule!(ConstantOverflow {
    id: "constant-overflow",
    name: "Constant int expression overflows",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["errorprone:ConstantOverflow", "sonar:S2184"],
    kinds: ["binary_expression"],
    description: "A compile-time constant `int` expression exceeds the int range and silently wraps around (e.g. `365 * 24 * 60 * 60 * 1000`). Use long arithmetic.",
});

impl Rule for ConstantOverflow {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(op(n), "*" | "+" | "-" | "<<") || cx.type_of(n) != Ty::Prim(Prim::Int) {
            return;
        }
        if let Some(p) = n.parent_unparen() {
            if p.kind() == "binary_expression" && cx.type_of(p) == Ty::Prim(Prim::Int) && matches!(op(p), "*" | "+" | "-" | "<<") && cx.fm.constant_value(p).is_some() {
                return;
            }
        }
        if cx.fm.constant_value(n).is_some() && overflows_i32(n, cx) {
            cx.report(n, "Constant int expression overflows; use long arithmetic (e.g. a `L` suffix)");
        }
    }
}

const MATH_ABS: MethodMatcher = MethodMatcher::statics("java.lang.Math").named(&["abs"]);
const RANDOM_INT: MethodMatcher = MethodMatcher::instance("java.util.Random").named(&["nextInt", "nextLong"]).arity(0);
const HASHCODE: MethodMatcher = MethodMatcher::any().named(&["hashCode"]).arity(0);

declare_rule!(AbsOfRandomOrHash {
    id: "abs-of-hashcode-or-random",
    name: "Math.abs() of a hash code or random int",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2676", "spotbugs:RV_ABSOLUTE_VALUE_OF_HASHCODE", "spotbugs:RV_ABSOLUTE_VALUE_OF_RANDOM_INT", "errorprone:MathAbsoluteNegative"],
    kinds: ["method_invocation"],
    description: "`Math.abs(Integer.MIN_VALUE)` is still negative, so `Math.abs(key.hashCode()) % n` occasionally produces a negative index. Use `Math.floorMod(hash, n)` or `nextInt(bound)`.",
});

impl Rule for AbsOfRandomOrHash {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !cx.fm.call_matches(n, &MATH_ABS) {
            return;
        }
        let Some(a) = cx.fm.args_of(n).first().map(|a| a.unparen()) else { return };
        if a.kind() != "method_invocation" {
            return;
        }
        if cx.fm.call_matches(a, &HASHCODE) || cx.fm.call_matches(a, &RANDOM_INT) {
            cx.report(n, "`Math.abs()` can return a negative value for MIN_VALUE; use `Math.floorMod()` or a bounded random");
        }
    }
}

declare_rule!(RandomCastToInt {
    id: "random-cast-to-int",
    name: "Random double in [0,1) cast to int",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["spotbugs:RV_01_TO_INT"],
    kinds: ["cast_expression"],
    description: "`(int) Math.random()` is always 0 because the value is in [0, 1). Multiply first: `(int) (Math.random() * n)`, or use `ThreadLocalRandom.current().nextInt(n)`.",
});

impl Rule for RandomCastToInt {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(cx.type_of(n).prim(), Some(Prim::Int | Prim::Long | Prim::Short | Prim::Byte)) {
            return;
        }
        let Some(v) = n.field("value").map(|v| v.unparen()) else { return };
        if v.kind() != "method_invocation" {
            return;
        }
        let text = cx.fm.call_name(v).unwrap_or("");
        let is_unit = (text == "random" && cx.fm.select_method(v).is_some_and(|c| &*c.owner == "java.lang.Math"))
            || (matches!(text, "nextDouble" | "nextFloat") && cx.fm.args_of(v).is_empty() && cx.fm.select_method(v).is_some_and(|c| cx.fm.is_subclass(&c.owner, "java.util.Random").is_yes() || c.owner.contains("Random")));
        if is_unit {
            cx.report(n, "Casting a random value in [0,1) to an integer always yields 0");
        }
    }
}

declare_rule!(RandomUsedOnce {
    id: "random-used-once",
    name: "Random object created and used only once",
    category: Performance,
    severity: Warning,
    sources: ["sonar:S2119", "spotbugs:DMI_RANDOM_USED_ONLY_ONCE"],
    kinds: ["method_invocation"],
    description: "Creating a `new Random()` for every value is wasteful and, for some seeds/JVMs, produces poorly distributed results. Keep one instance (or use `ThreadLocalRandom.current()`).",
});

impl Rule for RandomUsedOnce {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(o) = n.field("object").map(|o| o.unparen()) else { return };
        if o.kind() != "object_creation_expression" {
            return;
        }
        let t = cx.type_of(o);
        if matches!(t.class_name(), Some("java.util.Random" | "java.security.SecureRandom")) && cx.fm.call_name(n).is_some_and(|m| m.starts_with("next") || m == "ints" || m == "doubles") {
            cx.report(n, "New Random instance used once; reuse a single instance or `ThreadLocalRandom.current()`");
        }
    }
}

declare_rule!(BigDecimalFromDouble {
    id: "bigdecimal-from-double",
    name: "BigDecimal constructed from a double",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S2111", "spotbugs:DMI_BIGDECIMAL_CONSTRUCTED_FROM_DOUBLE", "pmd:AvoidDecimalLiteralsInBigDecimalConstructor"],
    kinds: ["object_creation_expression"],
    description: "`new BigDecimal(0.1)` captures the binary approximation 0.1000000000000000055511151231257827... Use `BigDecimal.valueOf(0.1)` or `new BigDecimal(\"0.1\")`.",
});

impl Rule for BigDecimalFromDouble {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !cx.type_of(n).is_class("java.math.BigDecimal") {
            return;
        }
        let args = cx.fm.args_of(n);
        if args.is_empty() {
            return;
        }
        let at = cx.type_of(args[0]);
        if matches!(at.prim(), Some(Prim::Double | Prim::Float)) {
            let fix = if args.len() == 1 { Some(Fix::replace("Use BigDecimal.valueOf", n, format!("java.math.BigDecimal.valueOf({})", cx.text(args[0])))) } else { None };
            match fix {
                Some(f) => cx.report_fix(n, "BigDecimal built from a double keeps its binary rounding error; use `BigDecimal.valueOf()` or a String", f),
                None => cx.report(n, "BigDecimal built from a double keeps its binary rounding error; use a String"),
            }
        }
    }
}

declare_rule!(OddCheckWithRemainder {
    id: "odd-check-remainder",
    name: "Odd check with % 2 == 1",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S2197", "spotbugs:IM_BAD_CHECK_FOR_ODD"],
    kinds: ["binary_expression"],
    description: "For negative numbers `x % 2` is -1, so `x % 2 == 1` misses negative odd values. Use `x % 2 != 0` or `(x & 1) == 1`.",
});

impl Rule for OddCheckWithRemainder {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(op(n), "==" | "!=") {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        for (rem, k) in [(l, r), (r, l)] {
            let rem = rem.unparen();
            if rem.kind() == "binary_expression" && op(rem) == "%" && cx.fm.int_value(k) == Some(1) {
                if let Some((a, b)) = lr(rem) {
                    if cx.fm.int_value(b) == Some(2) && is_integral_ty(&cx.type_of(a)) {
                        // values known to be non-negative: .size(), .length, abs()
                        let at = cx.text(a);
                        if at.ends_with(".size()") || at.ends_with(".length") || at.ends_with(".length()") || at.starts_with("Math.abs(") {
                            return;
                        }
                        let neg = if op(n) == "==" { "!=" } else { "==" };
                        let fix = Fix::replace("Compare with 0", n, format!("{} {neg} 0", cx.text(rem)));
                        cx.report_fix(n, "`% 2 == 1` fails for negative numbers; compare the remainder with 0", fix);
                        return;
                    }
                }
            }
        }
    }
}

declare_rule!(ByteComparisonOutOfRange {
    id: "comparison-out-of-range",
    name: "Comparison with a constant outside the type's range",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["spotbugs:INT_BAD_COMPARISON_WITH_SIGNED_BYTE", "spotbugs:INT_BAD_COMPARISON_WITH_INT_VALUE", "errorprone:ComparisonOutOfRange"],
    kinds: ["binary_expression"],
    description: "A `byte` is -128..127 and a `char` 0..65535: comparing with a constant outside that range is always true or false (often a forgotten `& 0xFF`).",
});

impl Rule for ByteComparisonOutOfRange {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(op(n), "==" | "!=" | "<" | ">" | "<=" | ">=") {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        for (v, k) in [(l, r), (r, l)] {
            let range = match cx.type_of(v).prim() {
                Some(Prim::Byte) => (-128i64, 127i64),
                Some(Prim::Char) => (0, 65535),
                Some(Prim::Short) => (-32768, 32767),
                Some(Prim::Int) => (i32::MIN as i64, i32::MAX as i64),
                _ => continue,
            };
            if cx.fm.constant_value(v).is_some() || cx.type_of(k).prim() == Some(Prim::Long) && range.1 == i32::MAX as i64 && false {
                continue;
            }
            if let Some(c) = cx.fm.int_value(k) {
                if cx.type_of(k).prim() == Some(Prim::Char) {
                    continue;
                }
                // normalize to `v OP c`
                let o = op(n);
                let o = if v == l {
                    o
                } else {
                    match o {
                        "<" => ">",
                        ">" => "<",
                        "<=" => ">=",
                        ">=" => "<=",
                        x => x,
                    }
                };
                let (lo, hi) = range;
                let constant = match o {
                    "==" | "!=" => c < lo || c > hi,
                    "<" => c <= lo || c > hi,
                    "<=" => c < lo || c >= hi,
                    ">" => c >= hi || c < lo,
                    ">=" => c > hi || c <= lo,
                    _ => false,
                };
                // `x <= MAX`-style checks on int are usually intentional guards
                if constant && range.1 != i32::MAX as i64 {
                    cx.report(n, format!("`{}` is always in [{}, {}]; comparing it with {c} is constant", cx.type_of(v), range.0, range.1));
                    return;
                }
                if constant && (c < lo || c > hi) {
                    cx.report(n, format!("`{}` is always in [{}, {}]; comparing it with {c} is constant", cx.type_of(v), range.0, range.1));
                    return;
                }
            }
        }
    }
}

declare_rule!(NarrowingCompoundAssignment {
    id: "narrowing-compound-assignment",
    name: "Compound assignment hides a narrowing conversion",
    category: Bug,
    severity: Warning,
    sources: ["errorprone:NarrowingCompoundAssignment", "sonar:S2164"],
    kinds: ["assignment_expression"],
    description: "`int i; i += 1.5;` compiles because compound assignment casts implicitly, silently truncating the long/floating result. Make the types match or cast explicitly.",
});

impl Rule for NarrowingCompoundAssignment {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let o = op(n);
        if !matches!(o, "+=" | "-=" | "*=" | "/=" | "%=") {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        let (Some(lp), Some(rp)) = (cx.type_of(l).prim(), cx.type_of(r).unboxed()) else { return };
        let bad = (lp.is_integral() && rp.is_floating()) || (matches!(lp, Prim::Int | Prim::Short | Prim::Byte | Prim::Char) && rp == Prim::Long) || (lp == Prim::Float && rp == Prim::Double && cx.fm.constant_value(r).is_none());
        if bad {
            cx.report(n, format!("`{o}` silently narrows a `{}` result to `{}`", rp.keyword(), lp.keyword()));
        }
    }
}

declare_rule!(OverwrittenIncrement {
    id: "overwritten-increment",
    name: "Increment result overwritten (i = i++)",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["spotbugs:DLS_OVERWRITTEN_INCREMENT", "sonar:S2123"],
    kinds: ["assignment_expression"],
    description: "`i = i++;` assigns the old value back, so the variable never changes. Write `i++;`.",
});

impl Rule for OverwrittenIncrement {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if op(n) != "=" {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        let r = r.unparen();
        if r.kind() != "update_expression" {
            return;
        }
        let t = cx.text(r);
        let postfix = t.ends_with("++") || t.ends_with("--");
        if let Some(operand) = r.first_kid() {
            if postfix && normalized(operand, cx.src) == normalized(l, cx.src) {
                cx.report(n, "The increment is lost: the old value is assigned back");
            }
        }
    }
}

declare_rule!(ReversedAssignmentOperator {
    id: "reversed-assignment-operator",
    name: "Suspicious =+, =- or =! operator",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S2757"],
    kinds: ["assignment_expression"],
    description: "`x =+ 1` assigns +1 instead of adding; it is almost always a typo for `+=`. Add a space (`x = +1`) if the unary operator is intended.",
});

impl Rule for ReversedAssignmentOperator {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if op(n) != "=" {
            return;
        }
        let Some(opn) = n.field("operator") else { return };
        let Some(r) = n.field("right") else { return };
        if r.kind() != "unary_expression" {
            return;
        }
        let Some(u) = r.field("operator") else { return };
        let uo = u.kind();
        if !matches!(uo, "+" | "-" | "!") {
            return;
        }
        // `=` immediately followed by the unary operator, then a space or operand
        if u.start_byte() == opn.end_byte() {
            let before = &cx.src[..opn.start_byte()];
            if before.ends_with(' ') || before.ends_with('\t') {
                cx.report(n, format!("`={uo}` looks like a reversed `{uo}=`"));
            }
        }
    }
}

declare_rule!(LowercaseLongLiteral {
    id: "lowercase-long-literal",
    name: "Long literal with lowercase l suffix",
    category: Style,
    severity: Info,
    sources: ["sonar:S818", "pmd:LongVariable", "errorprone:LongLiteralLowerCaseSuffix"],
    kinds: ["decimal_integer_literal", "hex_integer_literal", "octal_integer_literal", "binary_integer_literal"],
    description: "A lowercase `l` suffix is easily mistaken for the digit 1 (`101l`). Use an uppercase `L`.",
});

impl Rule for LowercaseLongLiteral {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let t = cx.text(n);
        if t.ends_with('l') {
            let fix = Fix::replace("Use uppercase L", n, format!("{}L", &t[..t.len() - 1]));
            cx.report_fix(n, "Use an uppercase `L` suffix for long literals", fix);
        }
    }
}

declare_rule!(OctalLiteral {
    id: "octal-literal",
    name: "Octal literal",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S1314", "pmd:AvoidUsingOctalValues"],
    kinds: ["octal_integer_literal", "decimal_integer_literal"],
    description: "A leading zero makes an integer literal octal: `010` is 8. If a decimal number was intended (e.g. zero-padded), remove the leading zero.",
});

impl Rule for OctalLiteral {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let t = cx.text(n).trim_end_matches(['l', 'L']).replace('_', "");
        if !t.starts_with('0') || t.starts_with("0x") || t.starts_with("0X") || t.starts_with("0b") || t.starts_with("0B") {
            return;
        }
        // 00 and single digits 01..07 are harmless
        if t.len() <= 2 {
            return;
        }
        // permission masks like 0644/0755 are conventional
        if t.len() == 4 && t[1..].chars().all(|c| matches!(c, '0' | '4' | '5' | '6' | '7')) {
            return;
        }
        let v = i64::from_str_radix(&t[1..], 8).unwrap_or(0);
        cx.report(n, format!("`{}` is an octal literal (value {v})", cx.text(n)));
    }
}

const MATH_CEIL_ROUND: MethodMatcher = MethodMatcher::statics("java.lang.Math").named(&["ceil", "floor", "round", "rint"]);

declare_rule!(IntCastToFloatingForRounding {
    id: "rounding-of-integer-division",
    name: "Math.ceil/round of an integer division",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["spotbugs:ICAST_INT_CAST_TO_DOUBLE_PASSED_TO_CEIL", "spotbugs:ICAST_INT_CAST_TO_FLOAT_PASSED_TO_ROUND", "sonar:S2184"],
    kinds: ["method_invocation"],
    description: "`Math.ceil(a / b)` with integer operands truncates in the division, so the rounding does nothing. Divide in floating point: `Math.ceil((double) a / b)` or use `Math.ceilDiv`.",
});

impl Rule for IntCastToFloatingForRounding {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !cx.fm.call_matches(n, &MATH_CEIL_ROUND) {
            return;
        }
        let Some(a) = cx.fm.args_of(n).first().map(|a| a.unparen()) else { return };
        let inner = if a.kind() == "cast_expression" { a.field("value").map(|v| v.unparen()) } else { Some(a) };
        let Some(inner) = inner else { return };
        if is_integral_ty(&cx.type_of(inner)) {
            cx.report(n, format!("`Math.{}()` of an integer value does nothing; the integer arithmetic already truncated", cx.fm.call_name(n).unwrap_or("")));
        }
    }
}

declare_rule!(CharArithmeticInConcat {
    id: "char-addition-in-concat",
    name: "char + char in string concatenation",
    category: Bug,
    severity: Warning,
    sources: ["errorprone:CharacterGetNumericValue", "sonar:S2185"],
    kinds: ["binary_expression"],
    description: "`'a' + 'b' + \"\"` adds the character codes first (\"195\"). Start the concatenation with a String or use `String.valueOf`.",
});

impl Rule for CharArithmeticInConcat {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if op(n) != "+" {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        if cx.type_of(l).prim() != Some(Prim::Char) || cx.type_of(r).prim() != Some(Prim::Char) {
            return;
        }
        // part of a string concatenation?
        if let Some(p) = n.parent_unparen() {
            if p.kind() == "binary_expression" && op(p) == "+" && cx.type_of(p).is_string() && p.field("left").map(|x| x.unparen()) == Some(n) {
                cx.report(n, "`char + char` is integer addition, not concatenation");
                return;
            }
        }
        if cx.fm.expected_type(n).is_string() {
            cx.report(n, "`char + char` is integer addition, not concatenation");
        }
    }
}

declare_rule!(RemainderOfOne {
    id: "remainder-by-one",
    name: "Remainder by 1",
    category: Bug,
    severity: Error,
    sources: ["spotbugs:INT_BAD_REM_BY_1"],
    kinds: ["binary_expression"],
    description: "`x % 1` on an integer is always 0; this is usually a typo for `x % 2` or a misplaced operator.",
});

impl Rule for RemainderOfOne {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if op(n) != "%" {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        if cx.fm.int_value(r).is_some_and(|v| v == 1 || v == -1) && is_integral_ty(&cx.type_of(l)) {
            cx.report(n, "Integer remainder by 1 is always 0");
        }
    }
}

declare_rule!(SignedIntToHex {
    id: "masked-value-comparison",
    name: "Masked value compared with an impossible constant",
    category: Bug,
    severity: Error,
    sources: ["spotbugs:BIT_AND", "spotbugs:BIT_AND_ZZ", "spotbugs:BIT_IOR"],
    kinds: ["binary_expression"],
    description: "`(x & 0xF0) == 0x0F` can never be true because the mask clears bits the constant requires; `(x & 0) == 0` is always true. The mask or constant is wrong.",
});

impl Rule for SignedIntToHex {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(op(n), "==" | "!=") {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        for (m, k) in [(l, r), (r, l)] {
            let m = m.unparen();
            if m.kind() != "binary_expression" {
                continue;
            }
            let Some(c) = cx.fm.int_value(k) else { continue };
            let Some((a, b)) = lr(m) else { continue };
            let mask = cx.fm.int_value(b).or_else(|| cx.fm.int_value(a));
            let Some(mask) = mask else { continue };
            match op(m) {
                "&" if (c & !mask) != 0 => {
                    cx.report(n, format!("`x & {mask:#x}` can never equal {c:#x}"));
                    return;
                }
                "&" if mask == 0 => {
                    cx.report(n, "`x & 0` is always 0");
                    return;
                }
                "|" if (mask & !c) != 0 => {
                    cx.report(n, format!("`x | {mask:#x}` can never equal {c:#x}"));
                    return;
                }
                _ => {}
            }
        }
    }
}

const MATH_MIN: MethodMatcher = MethodMatcher::statics("java.lang.Math").named(&["min"]);
const MATH_MAX: MethodMatcher = MethodMatcher::statics("java.lang.Math").named(&["max"]);

declare_rule!(MathMinMaxSwapped {
    id: "min-max-clamp-swapped",
    name: "Math.min/max clamp with swapped bounds",
    category: Bug,
    severity: Error,
    sources: ["errorprone:BadMinMax", "sonar:S3518"],
    kinds: ["method_invocation"],
    description: "`Math.min(LOW, Math.max(HIGH, x))` with LOW < HIGH always returns LOW. The bounds of the clamp are swapped.",
});

impl Rule for MathMinMaxSwapped {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let outer_min = cx.fm.call_matches(n, &MATH_MIN);
        let outer_max = !outer_min && cx.fm.call_matches(n, &MATH_MAX);
        if !outer_min && !outer_max {
            return;
        }
        let args = cx.fm.args_of(n);
        if args.len() != 2 {
            return;
        }
        for (c, inner) in [(args[0], args[1]), (args[1], args[0])] {
            let inner = inner.unparen();
            let Some(outer_c) = cx.fm.constant_value(c).and_then(|v| v.as_f64()) else { continue };
            if inner.kind() != "method_invocation" {
                continue;
            }
            let inner_ok = if outer_min { cx.fm.call_matches(inner, &MATH_MAX) } else { cx.fm.call_matches(inner, &MATH_MIN) };
            if !inner_ok {
                continue;
            }
            for ia in cx.fm.args_of(inner) {
                if let Some(inner_c) = cx.fm.constant_value(ia).and_then(|v| v.as_f64()) {
                    let swapped = if outer_min { outer_c < inner_c } else { outer_c > inner_c };
                    if swapped {
                        cx.report(n, format!("Clamp bounds are swapped; the result is always {}", cx.text(c)));
                        return;
                    }
                }
            }
        }
    }
}

declare_rule!(ImpossibleBitmaskComparison {
    id: "useless-sign-check",
    name: "Comparison of an unsigned value with zero",
    category: Bug,
    severity: Error,
    sources: ["sonar:S3981", "spotbugs:INT_VACUOUS_COMPARISON", "errorprone:ComparisonOutOfRange"],
    kinds: ["binary_expression"],
    description: "Sizes, lengths and `char` values are never negative: `list.size() < 0` is always false and `str.length() >= 0` always true. The check is dead code or the wrong comparison.",
});

impl Rule for ImpossibleBitmaskComparison {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let o = op(n);
        if !matches!(o, "<" | ">=" | ">" | "<=") {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        let non_negative = |x: Node<'t>| -> bool {
            let x = x.unparen();
            match x.kind() {
                "method_invocation" => {
                    let name = cx.fm.call_name(x).unwrap_or("");
                    if !cx.fm.args_of(x).is_empty() {
                        return false;
                    }
                    match name {
                        "size" => x.field("object").is_some_and(|o| is_subtype_any(cx.fm, &cx.type_of(o), &["java.util.Collection", "java.util.Map"])),
                        "length" => x.field("object").is_some_and(|o| is_subtype_any(cx.fm, &cx.type_of(o), &["java.lang.CharSequence"])),
                        _ => false,
                    }
                }
                "field_access" => x.field("field").is_some_and(|f| cx.text(f) == "length") && x.field("object").is_some_and(|o| cx.type_of(o).is_array()),
                _ => false,
            }
        };
        let (v, k, flipped) = if non_negative(l) {
            (l, r, false)
        } else if non_negative(r) {
            (r, l, true)
        } else {
            return;
        };
        if cx.fm.int_value(k) != Some(0) {
            return;
        }
        let o = if flipped {
            match o {
                "<" => ">",
                ">" => "<",
                "<=" => ">=",
                ">=" => "<=",
                x => x,
            }
        } else {
            o
        };
        match o {
            "<" => cx.report(n, format!("`{}` is never negative; this comparison is always false", cx.text(v))),
            ">=" => cx.report(n, format!("`{}` is never negative; this comparison is always true", cx.text(v))),
            _ => {}
        }
    }
}

declare_rule!(LossyFloatToIntCast {
    id: "floating-cast-in-integer-division",
    name: "Floating value cast to int before division",
    category: Bug,
    severity: Warning,
    sources: ["errorprone:FloatCast", "spotbugs:ICAST_INT_2_LONG_AS_INSTANT"],
    kinds: ["cast_expression"],
    description: "Casts bind tighter than arithmetic: `(int) x * 1.5` casts `x` first, and `(int) Math.random() * 10` is always 0. Parenthesize the intended expression.",
});

impl Rule for LossyFloatToIntCast {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(p) = n.parent() else { return };
        if p.kind() != "binary_expression" || !matches!(op(p), "*" | "/") || p.field("left") != Some(n) {
            return;
        }
        if !matches!(cx.type_of(n).prim(), Some(Prim::Int | Prim::Long)) {
            return;
        }
        let Some(v) = n.field("value") else { return };
        if v.kind() == "parenthesized_expression" {
            return;
        }
        if v.kind() == "method_invocation" && is_floating_ty(&cx.type_of(v)) {
            let Some(r) = p.field("right") else { return };
            if cx.fm.constant_value(r).is_some() {
                cx.report(n, "The cast applies only to the method result, before the multiplication; add parentheses");
            }
        }
    }
}
