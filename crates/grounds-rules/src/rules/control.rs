//! Control flow, conditions and expression simplifications.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(EmptyControlBody),
        Box::new(EmptyBlock),
        Box::new(IdenticalBranches),
        Box::new(IdenticalConditions),
        Box::new(BooleanLiteralComparison),
        Box::new(RedundantIfReturn),
        Box::new(DoubleNegation),
        Box::new(AssignmentInCondition),
        Box::new(NonShortCircuitLogic),
        Box::new(NullCheckWrongOperator),
        Box::new(LoopVariableModified),
        Box::new(WrongLoopDirection),
        Box::new(InfiniteLoop),
        Box::new(LoopExecutesOnce),
        Box::new(SwitchFallthrough),
        Box::new(SwitchMissingDefault),
        Box::new(SwitchTooFewCases),
        Box::new(NestedTernary),
        Box::new(CollapsibleIf),
        Box::new(UnusedLabel),
        Box::new(AssertSideEffect),
        Box::new(RedundantJump),
        Box::new(ImmediatelyReturnedLocal),
        Box::new(RedundantCast),
        Box::new(ConstantCondition),
        Box::new(InvertedBooleanCheck),
        Box::new(ForLoopWrongVariable),
        Box::new(StraySemicolon),
        Box::new(ElseAfterReturn),
    ]
}

fn has_comment(b: Node) -> bool {
    let mut c = b.walk();
    let r = b.children(&mut c).any(|k| grounds_syntax::is_comment(k.kind()));
    r
}

declare_rule!(EmptyControlBody {
    id: "empty-control-statement-body",
    name: "Control statement with an empty statement as body",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S1116", "errorprone:EmptyIf", "pmd:EmptyControlStatement", "intellij:ControlFlowStatementWithoutBraces"],
    kinds: ["if_statement", "while_statement", "for_statement", "enhanced_for_statement"],
    description: "`if (x);` or `for (...);` has an empty statement as its body, so the following block always runs (or the loop does nothing). Usually a stray semicolon.",
});

impl Rule for EmptyControlBody {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let body = if n.kind() == "if_statement" { n.field("consequence") } else { n.field("body") };
        let Some(b) = body else { return };
        if b.kind() == ";" || (b.kind() == "expression_statement" && cx.text(b) == ";") {
            // `while (iter.next() != null);` style busy loops are intentional when the condition has side effects
            if n.kind() == "while_statement" && n.field("condition").is_some_and(has_side_effects) {
                return;
            }
            if n.kind() == "for_statement" && n.fields("update").iter().any(|u| has_side_effects(*u)) && n.field("condition").is_some() && false {
                return;
            }
            cx.report(b, format!("`{}` has an empty statement as its body; stray semicolon?", n.kind().trim_end_matches("_statement").replace('_', " ")));
        }
    }
}

declare_rule!(EmptyBlock {
    id: "empty-block",
    name: "Empty code block",
    category: Style,
    severity: Info,
    sources: ["sonar:S108", "pmd:EmptyControlStatement", "intellij:EmptyStatementBody"],
    kinds: ["if_statement", "while_statement", "for_statement", "enhanced_for_statement", "finally_clause", "synchronized_statement", "try_statement", "do_statement"],
    description: "An empty block with no explanatory comment is either unfinished code or dead code. Remove it or add a comment explaining why nothing happens.",
});

impl Rule for EmptyBlock {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let blocks: Vec<Node<'t>> = match n.kind() {
            "if_statement" => [n.field("consequence"), n.field("alternative")].into_iter().flatten().collect(),
            "finally_clause" => n.kid_of_kind("block").into_iter().collect(),
            "try_statement" => n.field("body").into_iter().collect(),
            _ => n.field("body").into_iter().collect(),
        };
        for b in blocks {
            if b.kind() == "block" && b.kids().is_empty() && !has_comment(b) {
                // loops with side-effecting conditions are deliberate
                if matches!(n.kind(), "while_statement" | "do_statement") && n.field("condition").is_some_and(has_side_effects) {
                    continue;
                }
                cx.report(b, "Empty block; remove it or explain why it is empty");
            }
        }
    }
}

fn if_chain<'t>(n: Node<'t>) -> (Vec<Node<'t>>, Vec<Node<'t>>, bool) {
    // (conditions, branches, has_final_else)
    let mut conds = Vec::new();
    let mut branches = Vec::new();
    let mut cur = n;
    loop {
        if let Some(c) = cur.field("condition") {
            conds.push(c);
        }
        if let Some(b) = cur.field("consequence") {
            branches.push(b);
        }
        match cur.field("alternative") {
            Some(a) if a.kind() == "if_statement" => cur = a,
            Some(a) => {
                branches.push(a);
                return (conds, branches, true);
            }
            None => return (conds, branches, false),
        }
    }
}

fn is_else_if(n: Node) -> bool {
    n.parent().is_some_and(|p| p.kind() == "if_statement" && p.field("alternative") == Some(n))
}

fn branch_text(b: Node, src: &str) -> String {
    let b = if b.kind() == "block" && b.kids().len() == 1 { b.kids()[0] } else { b };
    normalized(b, src)
}

declare_rule!(IdenticalBranches {
    id: "identical-branches",
    name: "Branches with identical code",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S1871", "sonar:S3923", "spotbugs:DB_DUPLICATE_BRANCHES", "spotbugs:DB_DUPLICATE_SWITCH_CLAUSES", "intellij:IfStatementWithIdenticalBranches"],
    kinds: ["if_statement", "ternary_expression", "switch_expression"],
    description: "When all branches of an if/else, ternary or switch do the same thing, the condition is pointless; when two branches of a chain are identical, one was probably meant to differ (copy-paste error).",
});

impl Rule for IdenticalBranches {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        match n.kind() {
            "ternary_expression" => {
                let (Some(a), Some(b)) = (n.field("consequence"), n.field("alternative")) else { return };
                if normalized(a, cx.src) == normalized(b, cx.src) {
                    cx.report(n, "Both branches of the conditional expression are identical");
                }
            }
            "if_statement" => {
                if is_else_if(n) {
                    return;
                }
                let (_, branches, has_else) = if_chain(n);
                if branches.len() < 2 {
                    return;
                }
                let texts: Vec<String> = branches.iter().map(|b| branch_text(*b, cx.src)).collect();
                if has_else && texts.iter().all(|t| *t == texts[0]) {
                    cx.report(n.child(0).unwrap_or(n), "All branches of this if/else are identical; the condition has no effect");
                    return;
                }
                for i in 1..branches.len() {
                    // single-statement trivial branches like `return null;` are often intentionally duplicated
                    let trivial = branches[i].kind() != "block" || branches[i].kids().len() <= 1 && texts[i].len() < 20;
                    if trivial {
                        continue;
                    }
                    if let Some(j) = (0..i).find(|&j| texts[j] == texts[i]) {
                        let line = line_of(branches[j]) + 1;
                        cx.report_with_severity(branches[i], Severity::Info, format!("This branch has the same code as the branch on line {line}"));
                    }
                }
            }
            "switch_expression" => {
                let Some(body) = n.field("body") else { return };
                let groups = body.kids();
                let mut bodies: Vec<(Node, String)> = Vec::new();
                for g in &groups {
                    let stmts: Vec<Node> = g.kids().into_iter().filter(|k| k.kind() != "switch_label").collect();
                    if stmts.is_empty() {
                        continue;
                    }
                    let text: String = stmts.iter().map(|s| normalized(*s, cx.src)).collect::<Vec<_>>().join(" ");
                    let stmt_count = stmts.iter().filter(|s| !matches!(s.kind(), "break_statement")).count();
                    bodies.push((*g, text.clone()));
                    let _ = stmt_count;
                }
                if bodies.len() >= 2 && bodies.iter().all(|(_, t)| *t == bodies[0].1) && cx.text(body).contains("default") {
                    cx.report(n.child(0).unwrap_or(n), "All cases of this switch execute the same code");
                    return;
                }
                for i in 1..bodies.len() {
                    if bodies[i].1.len() < 30 {
                        continue;
                    }
                    if let Some(j) = (0..i).find(|&j| bodies[j].1 == bodies[i].1) {
                        let line = line_of(bodies[j].0) + 1;
                        cx.report_with_severity(bodies[i].0, Severity::Info, format!("This case has the same code as the case on line {line}; merge the labels"));
                    }
                }
            }
            _ => {}
        }
    }
}

declare_rule!(IdenticalConditions {
    id: "identical-conditions",
    name: "Repeated condition in an if/else-if chain",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S1862", "intellij:DuplicateCondition"],
    kinds: ["if_statement"],
    description: "A condition that already appeared earlier in the same if/else-if chain can never be true at that point, so its branch is dead code. One of them was probably meant to be different.",
});

impl Rule for IdenticalConditions {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if is_else_if(n) {
            return;
        }
        let (conds, _, _) = if_chain(n);
        for i in 1..conds.len() {
            if has_side_effects(conds[i]) {
                continue;
            }
            let t = normalized(conds[i], cx.src);
            // conditions in between may change what it reads
            // (`else if ((n = read()) < 0) ... else if (n > 0)`)
            if let Some(j) = (0..i).find(|&j| normalized(conds[j], cx.src) == t && !(j + 1..i).any(|k| has_side_effects(conds[k]))) {
                cx.report(conds[i], format!("Same condition as on line {}; this branch is unreachable", line_of(conds[j]) + 1));
            }
        }
    }
}

declare_rule!(BooleanLiteralComparison {
    id: "boolean-literal-comparison",
    name: "Comparison with a boolean literal",
    category: Style,
    severity: Info,
    sources: ["sonar:S1125", "pmd:SimplifyBooleanExpressions", "intellij:PointlessBooleanExpression"],
    kinds: ["binary_expression", "ternary_expression"],
    description: "`flag == true`, `flag != false`, `x && true` and `c ? true : false` are just `flag`/`x`/`c`.",
});

impl Rule for BooleanLiteralComparison {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let is_bool_lit = |x: Node| matches!(x.unparen().kind(), "true" | "false");
        if n.kind() == "ternary_expression" {
            let (Some(c), Some(a), Some(b)) = (n.field("condition"), n.field("consequence"), n.field("alternative")) else { return };
            if is_bool_lit(a) && is_bool_lit(b) && a.unparen().kind() != b.unparen().kind() {
                let neg = a.unparen().kind() == "false";
                let repl = if neg { format!("!({})", cx.text(c)) } else { cx.text(c).to_string() };
                cx.report_fix(n, "Conditional returning boolean literals; use the condition directly", Fix::replace("Simplify", n, repl));
            }
            return;
        }
        let o = op(n);
        if !matches!(o, "==" | "!=" | "&&" | "||") {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        // Boolean (boxed) == true also guards against null; leave it
        for (lit, other) in [(l, r), (r, l)] {
            if is_bool_lit(lit) && cx.type_of(other).prim() == Some(Prim::Boolean) {
                cx.report(n, format!("Redundant comparison with `{}`", cx.text(lit)));
                return;
            }
        }
    }
}

declare_rule!(RedundantIfReturn {
    id: "redundant-if-return-boolean",
    name: "if/else returning boolean literals",
    category: Style,
    severity: Info,
    sources: ["sonar:S1126", "pmd:SimplifyBooleanReturns", "intellij:SimplifiableIfStatement"],
    kinds: ["if_statement"],
    description: "`if (c) return true; else return false;` is `return c;`.",
});

fn returns_bool_literal(n: Node) -> Option<bool> {
    let n = if n.kind() == "block" && n.kids().len() == 1 { n.kids()[0] } else { n };
    if n.kind() != "return_statement" {
        return None;
    }
    match n.first_kid()?.unparen().kind() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

impl Rule for RedundantIfReturn {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = n.field("condition") else { return };
        let Some(t) = n.field("consequence").and_then(returns_bool_literal) else { return };
        let other = match n.field("alternative") {
            Some(a) => returns_bool_literal(a),
            None => {
                if is_else_if(n) {
                    return;
                }
                next_sibling_stmt(n).and_then(returns_bool_literal)
            }
        };
        let Some(f) = other else { return };
        if t != f {
            let cond = c.first_kid().map(|x| cx.text(x)).unwrap_or("");
            let repl = if t { format!("return {cond};") } else { format!("return !({cond});") };
            cx.report_with_severity(n.child(0).unwrap_or(n), Severity::Info, format!("Simplify to `{}`", repl));
        }
    }
}

declare_rule!(DoubleNegation {
    id: "double-negation",
    name: "Double negation",
    category: Style,
    severity: Info,
    sources: ["sonar:S2761", "intellij:DoubleNegation"],
    kinds: ["unary_expression"],
    description: "`!!x` and `~~x` are `x`; double negation usually signals a leftover edit.",
});

impl Rule for DoubleNegation {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let o = op(n);
        if !matches!(o, "!" | "~") {
            return;
        }
        if let Some(inner) = n.field("operand").map(|x| x.unparen()) {
            if inner.kind() == "unary_expression" && op(inner) == o {
                let base = inner.field("operand").map(|x| cx.text(x)).unwrap_or("");
                cx.report_fix(n, format!("`{o}{o}` cancels out"), Fix::replace("Remove double negation", n, base));
            }
        }
    }
}

declare_rule!(AssignmentInCondition {
    id: "assignment-in-condition",
    name: "Boolean assignment used as a condition",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S1121", "spotbugs:QBA_QUESTIONABLE_BOOLEAN_ASSIGNMENT", "pmd:AssignmentInOperand", "intellij:AssignmentUsedAsCondition"],
    kinds: ["if_statement", "while_statement", "do_statement", "ternary_expression"],
    description: "`if (done = true)` assigns instead of comparing and is always true. Use `==` (or just `if (done)`).",
});

impl Rule for AssignmentInCondition {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let c = if n.kind() == "ternary_expression" { n.field("condition") } else { n.field("condition").and_then(|c| c.first_kid()) };
        let Some(c) = c.map(|c| c.unparen()) else { return };
        if c.kind() == "assignment_expression" && op(c) == "=" && cx.type_of(c).is_boolean() {
            cx.report(c, "Assignment used as a condition; did you mean `==`?");
        }
    }
}

declare_rule!(NonShortCircuitLogic {
    id: "non-short-circuit-logic",
    name: "& or | used on booleans with side effects",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2178", "spotbugs:NS_DANGEROUS_NON_SHORT_CIRCUIT", "spotbugs:NS_NON_SHORT_CIRCUIT"],
    kinds: ["binary_expression"],
    description: "`a & b` always evaluates `b`; in `x != null & x.isValid()` that means a NullPointerException. Use `&&`/`||` unless evaluating both sides is intended.",
});

impl Rule for NonShortCircuitLogic {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let o = op(n);
        if !matches!(o, "&" | "|") {
            return;
        }
        let Some((l, r)) = lr(n) else { return };
        if !(cx.type_of(l).is_boolean() && cx.type_of(r).is_boolean()) {
            return;
        }
        let null_guard = {
            let l = l.unparen();
            l.kind() == "binary_expression" && matches!(op(l), "!=" | "==") && lr(l).is_some_and(|(a, b)| is_null_literal(a) || is_null_literal(b))
        };
        if null_guard || r.descendants().any(|d| d.kind() == "method_invocation") {
            let fix = Fix { title: format!("Use {o}{o}"), edits: vec![Edit { start: n.field("operator").unwrap().start_byte() as u32, end: n.field("operator").unwrap().end_byte() as u32, replacement: format!("{o}{o}") }] };
            cx.report_fix(n, format!("`{o}` evaluates both operands; use `{o}{o}`"), fix);
        }
    }
}

declare_rule!(NullCheckWrongOperator {
    id: "null-check-wrong-operator",
    name: "Null check combined with the wrong logical operator",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S1697", "spotbugs:NP_NULL_ON_SOME_PATH", "intellij:ConstantConditions"],
    kinds: ["binary_expression"],
    description: "`x == null && x.foo()` and `x != null || x.foo()` dereference `x` exactly when it is null. The operators (or the comparison) are swapped.",
});

impl Rule for NullCheckWrongOperator {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let o = op(n);
        let want = match o {
            "&&" => "==",
            "||" => "!=",
            _ => return,
        };
        let Some((l, r)) = lr(n) else { return };
        let l = l.unparen();
        if l.kind() != "binary_expression" || op(l) != want {
            return;
        }
        let Some((a, b)) = lr(l) else { return };
        let var = if is_null_literal(b) { a } else if is_null_literal(a) { b } else { return };
        let vt = normalized(var, cx.src);
        // reassigned on the right before use (`v != null || ((v = dflt) != null && v.isEmpty())`)
        let reassigned = r.descendants().any(|d| d.kind() == "assignment_expression" && d.field("left").is_some_and(|x| normalized(x, cx.src) == vt));
        if reassigned {
            return;
        }
        // a dereference behind its own guard (`... || (x != null && x.f())`) is safe
        let guarded = |d: Node<'t>| {
            let mut child = d;
            for a in d.ancestors().take_while(|a| *a != n) {
                let checks_non_null = |side: Option<Node<'t>>| {
                    side.is_some_and(|s| std::iter::once(s).chain(s.descendants()).any(|b| b.kind() == "binary_expression" && op(b) == "!=" && lr(b).is_some_and(|(x, y)| (normalized(x, cx.src) == vt && is_null_literal(y)) || (normalized(y, cx.src) == vt && is_null_literal(x)))))
                };
                if a.kind() == "binary_expression" && op(a) == "&&" && a.field("right") == Some(child) && checks_non_null(a.field("left")) {
                    return true;
                }
                if a.kind() == "ternary_expression" && a.field("consequence") == Some(child) && checks_non_null(a.field("condition")) {
                    return true;
                }
                child = a;
            }
            false
        };
        // does the right side dereference the same expression?
        let derefs = r.descendants().any(|d| {
            let target = match d.kind() {
                "method_invocation" | "field_access" => d.field("object").is_some_and(|o| normalized(o, cx.src) == vt),
                "array_access" => d.field("array").is_some_and(|o| normalized(o, cx.src) == vt),
                _ => false,
            };
            target && !guarded(d)
        });
        if derefs {
            cx.report(n, format!("`{}` is dereferenced exactly when it is null; use `{}`", cx.text(var), if o == "&&" { "!= null &&" } else { "== null ||" }));
        }
    }
}

fn for_counters<'t>(n: Node<'t>, src: &'t str) -> Vec<&'t str> {
    let mut v = Vec::new();
    for i in n.fields("init") {
        if i.kind() == "local_variable_declaration" {
            for d in i.fields("declarator") {
                if let Some(nm) = d.field("name") {
                    v.push(nm.text(src));
                }
            }
        } else if i.kind() == "assignment_expression" {
            if let Some(l) = i.field("left") {
                v.push(l.text(src));
            }
        }
    }
    v
}

declare_rule!(LoopVariableModified {
    id: "loop-counter-modified-in-body",
    name: "for-loop counter modified in the body",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S127", "pmd:AvoidReassigningLoopVariables", "intellij:AssignmentToForLoopParameter"],
    kinds: ["for_statement"],
    description: "Changing the counter inside the loop body makes the iteration logic hard to follow and easy to break. Use a while loop if the step varies.",
});

impl Rule for LoopVariableModified {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let counters = for_counters(n, cx.src);
        let Some(body) = n.field("body") else { return };
        if counters.is_empty() {
            return;
        }
        let mut assigns = Vec::new();
        find_kind(body, "assignment_expression", &mut assigns);
        find_kind(body, "update_expression", &mut assigns);
        for a in assigns {
            let target = if a.kind() == "assignment_expression" { a.field("left") } else { a.first_kid() };
            if let Some(t) = target {
                if t.kind() == "identifier" && counters.contains(&t.text(cx.src)) {
                    cx.report(a, format!("Loop counter `{}` is modified inside the loop body", t.text(cx.src)));
                }
            }
        }
    }
}

declare_rule!(WrongLoopDirection {
    id: "loop-wrong-direction",
    name: "for-loop update moves away from its condition",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S2251", "errorprone:LoopDirection"],
    kinds: ["for_statement"],
    description: "`for (int i = 0; i < n; i--)` decrements while the condition expects growth: the loop runs until overflow or never ends. The update or the comparison is reversed.",
});

impl Rule for WrongLoopDirection {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(cond) = n.field("condition").map(|c| c.unparen()) else { return };
        if cond.kind() != "binary_expression" {
            return;
        }
        let Some((l, r)) = lr(cond) else { return };
        let o = op(cond);
        let counters = for_counters(n, cx.src);
        let (var, grows) = if l.kind() == "identifier" && counters.contains(&l.text(cx.src)) {
            (l.text(cx.src), matches!(o, "<" | "<="))
        } else if r.kind() == "identifier" && counters.contains(&r.text(cx.src)) {
            (r.text(cx.src), matches!(o, ">" | ">="))
        } else {
            return;
        };
        if !matches!(o, "<" | "<=" | ">" | ">=") {
            return;
        }
        // the body may move the bound or the counter itself
        // (`for (low = 0, high = 10; low < high; low--) { high -= 2; }`)
        let bound = if l.kind() == "identifier" && l.text(cx.src) == var { r } else { l };
        let bound_vars: Vec<&str> = std::iter::once(bound).chain(bound.descendants()).filter(|d| d.kind() == "identifier").map(|d| d.text(cx.src)).collect();
        if let Some(body) = n.field("body") {
            let changed = assigned_names(body, cx.src);
            if changed.contains(&var) || changed.iter().any(|c| bound_vars.contains(c)) {
                return;
            }
        }
        if n.fields("update").iter().any(|u| assigned_names(*u, cx.src).iter().any(|c| bound_vars.contains(c))) {
            return;
        }
        for u in n.fields("update") {
            let dir = match u.kind() {
                "update_expression" if u.first_kid().is_some_and(|k| k.text(cx.src) == var) => {
                    if cx.text(u).contains("++") {
                        Some(true)
                    } else {
                        Some(false)
                    }
                }
                "assignment_expression" if u.field("left").is_some_and(|k| k.text(cx.src) == var) => match op(u) {
                    "+=" => u.field("right").and_then(|v| cx.fm.int_value(v)).map(|v| v > 0),
                    "-=" => u.field("right").and_then(|v| cx.fm.int_value(v)).map(|v| v < 0),
                    _ => None,
                },
                _ => None,
            };
            if let Some(d) = dir {
                if d != grows {
                    cx.report(u, format!("`{var}` moves away from the loop condition `{}`", cx.text(cond)));
                }
            }
        }
    }
}

fn loop_has_exit(body: Node, src: &str) -> bool {
    fn rec(n: Node, depth: usize, src: &str) -> bool {
        for k in n.kids() {
            match k.kind() {
                "return_statement" | "throw_statement" => return true,
                "break_statement" => {
                    let labeled = k.kids().iter().any(|x| x.kind() == "identifier");
                    if labeled || depth == 0 {
                        return true;
                    }
                }
                "expression_statement" => {
                    let t = k.text(src);
                    if t.starts_with("System.exit(") {
                        return true;
                    }
                }
                "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement" | "switch_expression" => {
                    if rec(k, depth + 1, src) {
                        return true;
                    }
                }
                "lambda_expression" | "class_body" | "class_declaration" => {}
                _ => {
                    if rec(k, depth, src) {
                        return true;
                    }
                }
            }
        }
        false
    }
    rec(body, 0, src)
}

declare_rule!(InfiniteLoop {
    id: "infinite-loop",
    name: "Loop without any exit",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2189", "spotbugs:IL_INFINITE_LOOP"],
    kinds: ["while_statement", "for_statement", "do_statement"],
    description: "A `while (true)`/`for (;;)` loop with no break, return, throw or exit never terminates; the code after it is unreachable and the thread hangs.",
});

impl Rule for InfiniteLoop {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let always = match n.kind() {
            "for_statement" => n.field("condition").is_none_or(|c| cx.fm.constant_value(c) == Some(Constant::Bool(true))),
            _ => n.field("condition").and_then(|c| c.first_kid()).is_some_and(|c| cx.fm.constant_value(c) == Some(Constant::Bool(true))),
        };
        if !always {
            return;
        }
        let Some(body) = n.field("body") else { return };
        // loops inside try/catch are commonly ended by an exception (server accept loops)
        if n.ancestors().take_while(|a| !matches!(a.kind(), "method_declaration" | "lambda_expression")).any(|a| matches!(a.kind(), "try_statement" | "try_with_resources_statement") && !a.kids_of_kind("catch_clause").is_empty()) {
            return;
        }
        if !loop_has_exit(body, cx.src) {
            // methods that are meant to run forever (thread loops) usually catch InterruptedException
            if cx.text(body).contains("InterruptedException") || cx.text(body).contains("isInterrupted") {
                return;
            }
            cx.report(n.child(0).unwrap_or(n), "This loop has no break, return or throw; it never terminates");
        }
    }
}

declare_rule!(LoopExecutesOnce {
    id: "loop-executes-at-most-once",
    name: "Loop body always exits on the first iteration",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S1751", "intellij:LoopStatementThatDoesntLoop"],
    kinds: ["while_statement", "for_statement", "enhanced_for_statement", "do_statement"],
    description: "A loop whose body unconditionally ends with break/return/throw runs at most once. Use an `if` (or `findFirst`) instead, or fix the misplaced jump.",
});

impl Rule for LoopExecutesOnce {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(body) = n.field("body") else { return };
        if body.kind() != "block" {
            return;
        }
        let kids = body.kids();
        let Some(last) = kids.last() else { return };
        if !matches!(last.kind(), "break_statement" | "return_statement" | "throw_statement") {
            return;
        }
        if last.kind() == "break_statement" && last.kids().iter().any(|x| x.kind() == "identifier") {
            return;
        }
        // any continue in the body means the loop can iterate
        let mut cont = Vec::new();
        find_kind(body, "continue_statement", &mut cont);
        if !cont.is_empty() {
            return;
        }
        cx.report(n.child(0).unwrap_or(n), format!("The loop body always ends with `{}`; it runs at most once", last.kind().trim_end_matches("_statement")));
    }
}

declare_rule!(SwitchFallthrough {
    id: "switch-fallthrough",
    name: "Switch case falls through",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S128", "spotbugs:SF_SWITCH_FALLTHROUGH", "errorprone:FallThrough", "pmd:ImplicitSwitchFallThrough"],
    kinds: ["switch_block_statement_group"],
    description: "A case with statements that does not end in break/return/throw continues into the next case. If intended, say so with a `// fall through` comment; otherwise add the missing `break`.",
});

impl Rule for SwitchFallthrough {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let stmts: Vec<Node> = n.kids().into_iter().filter(|k| k.kind() != "switch_label").collect();
        let Some(last) = stmts.last() else { return };
        let Some(next) = next_sibling_stmt(n) else { return };
        if next.kind() != "switch_block_statement_group" {
            return;
        }
        // all statements must be able to complete normally, i.e. we reach the end
        if !can_complete_normally(*last, cx.src) {
            return;
        }
        // comment between this group and the next case saying fall through
        let gap = &cx.src[last.end_byte()..next.start_byte()];
        let tail = cx.text(n).to_ascii_lowercase();
        let has_marker = |s: &str| {
            let s = s.to_ascii_lowercase();
            s.contains("fall") || s.contains("no break") || s.contains("nobreak") || s.contains("continue")
        };
        // last 60 characters (not bytes: the text may contain non-ASCII)
        let tail_start = tail.char_indices().rev().nth(59).map_or(0, |(i, _)| i);
        if has_marker(gap) || has_marker(&tail[tail_start..]) {
            return;
        }
        if let Some(label) = next.kid_of_kind("switch_label") {
            cx.report(label, "Previous case falls through into this one; add `break` or a `// fall through` comment");
        }
    }
}

declare_rule!(SwitchMissingDefault {
    id: "switch-missing-default",
    name: "switch statement without default",
    category: Style,
    severity: Info,
    enabled: false,
    sources: ["sonar:S131", "pmd:SwitchStmtsShouldHaveDefault", "intellij:SwitchStatementWithoutDefaultBranch"],
    kinds: ["switch_expression"],
    description: "A switch statement without a `default` silently ignores unexpected values. Add a default that handles or rejects them.",
});

impl Rule for SwitchMissingDefault {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        // statements only (switch expressions must be exhaustive)
        if n.parent().is_some_and(|p| p.kind() != "block" && p.kind() != "switch_block_statement_group" && p.kind() != "switch_rule") && !n.parent().is_some_and(|p| matches!(p.kind(), "constructor_body")) {
            return;
        }
        let Some(body) = n.field("body") else { return };
        let has_default = body.descendants().any(|d| d.kind() == "switch_label" && (cx.text(d).trim_start().starts_with("default") || cx.text(d).contains("default")));
        // enum switches are often exhaustive by design
        let enum_switch = n.field("condition").and_then(|c| c.first_kid()).is_some_and(|c| cx.type_of(c).class_name().and_then(|x| cx.fm.class(x)).is_some_and(|ci| ci.is_enum()));
        if !has_default && !enum_switch {
            cx.report(n.child(0).unwrap_or(n), "switch without a default case");
        }
    }
}

declare_rule!(SwitchTooFewCases {
    id: "switch-with-few-cases",
    name: "switch with fewer than three cases",
    category: Style,
    severity: Info,
    enabled: false,
    sources: ["sonar:S1301", "pmd:TooFewBranchesForSwitch"],
    kinds: ["switch_expression"],
    description: "A switch with one or two cases reads better as an if/else.",
});

impl Rule for SwitchTooFewCases {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(body) = n.field("body") else { return };
        let labels = body.descendants().filter(|d| d.kind() == "switch_label").count();
        if labels > 0 && labels < 3 {
            let _ = cx.text(n);
            cx.report(n.child(0).unwrap_or(n), "switch with fewer than three cases; use if/else");
        }
    }
}

declare_rule!(NestedTernary {
    id: "nested-ternary",
    name: "Nested conditional expression",
    category: Style,
    severity: Info,
    sources: ["sonar:S3358", "intellij:NestedConditionalExpression"],
    kinds: ["ternary_expression"],
    description: "Nested `?:` expressions are hard to read and easy to get wrong. Use if/else or a switch expression.",
});

impl Rule for NestedTernary {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.parent_unparen().is_some_and(|p| p.kind() == "ternary_expression") {
            return;
        }
        for f in ["consequence", "alternative", "condition"] {
            if n.field(f).is_some_and(|x| x.unparen().kind() == "ternary_expression") {
                let _ = cx.text(n);
                cx.report(n, "Nested conditional expression; use if/else");
                return;
            }
        }
    }
}

declare_rule!(CollapsibleIf {
    id: "collapsible-if",
    name: "Nested if statements can be merged",
    category: Style,
    severity: Info,
    sources: ["sonar:S1066", "pmd:CollapsibleIfStatements", "intellij:IfStatementMissingBreakInLoop"],
    kinds: ["if_statement"],
    description: "`if (a) { if (b) { ... } }` without else branches is `if (a && b) { ... }`.",
});

impl Rule for CollapsibleIf {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.field("alternative").is_some() {
            return;
        }
        let Some(cons) = n.field("consequence") else { return };
        let inner = if cons.kind() == "block" {
            let k = cons.kids();
            if k.len() != 1 || block_has_own_line_comment(cons) {
                return;
            }
            k[0]
        } else {
            cons
        };
        if inner.kind() == "if_statement" && inner.field("alternative").is_none() {
            // pattern-binding conditions often read better nested
            let _ = cx.text(n);
            cx.report(n.child(0).unwrap_or(n), "Merge this if with the nested if using `&&`");
        }
    }
}

declare_rule!(UnusedLabel {
    id: "unused-label",
    name: "Unused label",
    category: Style,
    severity: Info,
    sources: ["sonar:S1065", "intellij:UnusedLabel"],
    kinds: ["labeled_statement"],
    description: "A label that no break/continue refers to is dead code.",
});

impl Rule for UnusedLabel {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(label) = n.kids().into_iter().find(|k| k.kind() == "identifier") else { return };
        let name = label.text(cx.src);
        let used = n.descendants().any(|d| matches!(d.kind(), "break_statement" | "continue_statement") && d.kids().iter().any(|k| k.kind() == "identifier" && k.text(cx.src) == name));
        if !used {
            cx.report(label, format!("Label `{name}` is never used"));
        }
    }
}

declare_rule!(AssertSideEffect {
    id: "assert-with-side-effect",
    name: "assert with side effects",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S3346", "spotbugs:SA_LOCAL_SELF_COMPARISON", "intellij:AssertWithSideEffects"],
    kinds: ["assert_statement"],
    description: "Assertions are disabled by default in production, so a side effect inside `assert` (like `assert list.remove(x)`) silently does not happen there.",
});

impl Rule for AssertSideEffect {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = n.first_kid() else { return };
        for d in e.descendants() {
            let side = match d.kind() {
                "assignment_expression" | "update_expression" => true,
                "method_invocation" => matches!(cx.fm.call_name(d), Some("add" | "remove" | "put" | "poll" | "offer" | "push" | "pop" | "next" | "delete" | "set" | "clear" | "addAll" | "removeAll" | "incrementAndGet" | "getAndIncrement" | "compareAndSet" | "mkdirs" | "createNewFile")),
                _ => false,
            };
            if side {
                cx.report(d, "Side effect inside assert does not run when assertions are disabled");
                return;
            }
        }
    }
}

declare_rule!(RedundantJump {
    id: "redundant-jump",
    name: "Redundant return or continue",
    category: Style,
    severity: Info,
    sources: ["sonar:S3626", "pmd:UnnecessaryReturn", "intellij:UnnecessaryReturnStatement"],
    kinds: ["return_statement", "continue_statement"],
    description: "A `return;` at the end of a void method or a `continue;` at the end of a loop body does nothing.",
});

impl Rule for RedundantJump {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.kind() == "return_statement" && n.first_kid().is_some() {
            return;
        }
        if n.kind() == "continue_statement" && n.kids().iter().any(|k| k.kind() == "identifier") {
            return;
        }
        let Some(block) = n.parent() else { return };
        if block.kind() != "block" || block.kids().last() != Some(&n) {
            return;
        }
        let Some(owner) = block.parent() else { return };
        let redundant = match n.kind() {
            "return_statement" => matches!(owner.kind(), "method_declaration" | "constructor_declaration" | "lambda_expression") && owner.field("body") == Some(block),
            _ => matches!(owner.kind(), "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement") && owner.field("body") == Some(block),
        };
        if redundant {
            let kw = if n.kind() == "return_statement" { "return" } else { "continue" };
            cx.report_fix(n, format!("Redundant `{kw}`"), Fix::delete(format!("Remove {kw}"), n.start_byte() as u32, n.end_byte() as u32));
        }
    }
}

declare_rule!(ImmediatelyReturnedLocal {
    id: "local-immediately-returned",
    name: "Local variable declared and immediately returned",
    category: Style,
    severity: Info,
    sources: ["sonar:S1488", "pmd:UnnecessaryLocalBeforeReturn", "intellij:UnnecessaryLocalVariable"],
    kinds: ["return_statement"],
    description: "`T x = expr; return x;` adds a name without adding information; return the expression directly.",
});

impl Rule for ImmediatelyReturnedLocal {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(v) = n.first_kid() else { return };
        if v.kind() != "identifier" {
            return;
        }
        let Some(prev) = prev_sibling_stmt(n) else { return };
        if prev.kind() != "local_variable_declaration" {
            return;
        }
        let decls = prev.fields("declarator");
        if decls.len() != 1 {
            return;
        }
        let d = decls[0];
        if d.field("name").map(|x| x.text(cx.src)) != Some(v.text(cx.src)) || d.field("value").is_none() {
            return;
        }
        // annotated locals (@SuppressWarnings) or explicitly typed for a cast are intentional
        if prev.modifiers().is_some() {
            return;
        }
        cx.report(v, format!("`{}` is returned immediately after being assigned; return the expression directly", v.text(cx.src)));
    }
}

declare_rule!(RedundantCast {
    id: "redundant-cast",
    name: "Cast to the expression's own type",
    category: Style,
    severity: Info,
    sources: ["sonar:S1905", "pmd:UnnecessaryCast", "intellij:RedundantCast"],
    kinds: ["cast_expression"],
    description: "Casting an expression to the type it already has does nothing.",
});

impl Rule for RedundantCast {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(v) = n.field("value") else { return };
        let tys = n.fields("type");
        if tys.len() != 1 {
            return;
        }
        let target = cx.fm.resolve_type_node(tys[0]);
        let vt = cx.type_of(v);
        if target.has_unknown() || vt.has_unknown() || target.mentions_type_var() || vt.mentions_type_var() {
            return;
        }
        // lambdas/method refs need target types; null casts select overloads
        if matches!(v.unparen().kind(), "lambda_expression" | "method_reference" | "null_literal" | "switch_expression" | "ternary_expression") {
            return;
        }
        // casts in arguments may pick an overload; casts in lambda bodies/returns drive inference
        if n.parent_unparen().is_some_and(|p| matches!(p.kind(), "argument_list" | "lambda_expression" | "return_statement")) {
            return;
        }
        // only trust the value's type when it does not come from generic inference
        let v0 = v.unparen();
        let trustworthy = match v0.kind() {
            "identifier" | "field_access" | "this" => true,
            k if grounds_flow::cfg::is_literal(k) => true,
            "method_invocation" | "object_creation_expression" => cx.fm.select_method(v0).is_some_and(|c| {
                !c.ambiguous && c.method.type_params.is_empty() && !c.method.ret.mentions_type_var() && target.args().is_empty() && cx.fm.args_of(v0).iter().all(|a| cx.type_of(*a).is_known())
            }) && !target.is_prim(),
            _ => false,
        };
        if !trustworthy {
            return;
        }
        if target == vt && !target.args().iter().any(|a| matches!(a, Ty::Wildcard(_))) {
            let fix = Fix::replace("Remove cast", n, cx.text(v));
            cx.report_fix(n, format!("Redundant cast to `{}`", target.display_simple()), fix);
        }
    }
}

declare_rule!(ConstantCondition {
    id: "constant-boolean-condition",
    name: "Condition is a boolean literal",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2583", "sonar:S2589", "intellij:ConstantConditions"],
    kinds: ["if_statement", "ternary_expression", "binary_expression"],
    description: "`if (true)`, `if (false)` or `x || true` make one branch dead code, usually a debugging leftover.",
});

impl Rule for ConstantCondition {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        match n.kind() {
            "if_statement" | "ternary_expression" => {
                let c = if n.kind() == "if_statement" { n.field("condition").and_then(|c| c.first_kid()) } else { n.field("condition") };
                let Some(c) = c.map(|c| c.unparen()) else { return };
                if matches!(c.kind(), "true" | "false") {
                    cx.report(c, format!("Condition is always `{}`", c.kind()));
                }
            }
            _ => {
                let o = op(n);
                if !matches!(o, "&&" | "||") {
                    return;
                }
                let Some((l, r)) = lr(n) else { return };
                for side in [l, r] {
                    let s = side.unparen();
                    let absorbing = (o == "||" && s.kind() == "true") || (o == "&&" && s.kind() == "false");
                    if absorbing {
                        cx.report(n, format!("`{o} {}` makes the whole expression constant", s.kind()));
                        return;
                    }
                }
            }
        }
    }
}

declare_rule!(InvertedBooleanCheck {
    id: "inverted-comparison",
    name: "Negated comparison",
    category: Style,
    severity: Info,
    sources: ["sonar:S1940", "pmd:LogicInversion", "intellij:NegatedEqualityExpression"],
    kinds: ["unary_expression"],
    description: "`!(a == b)` is `a != b`, and `!(a < b)` is `a >= b`; the direct form is easier to read.",
});

impl Rule for InvertedBooleanCheck {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if op(n) != "!" {
            return;
        }
        let Some(inner) = n.field("operand") else { return };
        if inner.kind() != "parenthesized_expression" {
            return;
        }
        let e = inner.unparen();
        if e.kind() != "binary_expression" {
            return;
        }
        let inv = match op(e) {
            "==" => "!=",
            "!=" => "==",
            "<" => ">=",
            ">" => "<=",
            "<=" => ">",
            ">=" => "<",
            _ => return,
        };
        // floating point comparisons are not invertible because of NaN
        if let Some((l, _)) = lr(e) {
            if matches!(cx.type_of(l).unboxed(), Some(Prim::Float | Prim::Double)) && op(e) != "==" && op(e) != "!=" {
                return;
            }
        }
        let (l, r) = lr(e).unwrap();
        cx.report_fix(n, format!("Use `{inv}` instead of negating the comparison"), Fix::replace("Invert comparison", n, format!("{} {inv} {}", cx.text(l), cx.text(r))));
    }
}

declare_rule!(ForLoopWrongVariable {
    id: "for-loop-updates-wrong-variable",
    name: "for-loop update does not change the tested variable",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S1994", "intellij:ForLoopThatDoesntUseLoopVariable"],
    kinds: ["for_statement"],
    description: "In `for (int j = 0; j < n; i++)` the update changes a different variable than the condition tests, typically a copy-paste error in nested loops.",
});

impl Rule for ForLoopWrongVariable {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(cond) = n.field("condition") else { return };
        let updates = n.fields("update");
        if updates.is_empty() {
            return;
        }
        let cond_vars: Vec<&str> = cond.descendants().filter(|d| d.kind() == "identifier").map(|d| d.text(cx.src)).collect();
        let counters = for_counters(n, cx.src);
        let mut updated = Vec::new();
        for u in &updates {
            let t = match u.kind() {
                "update_expression" => u.first_kid(),
                "assignment_expression" => u.field("left"),
                _ => None,
            };
            match t {
                Some(t) if t.kind() == "identifier" => updated.push(t.text(cx.src)),
                _ => return,
            }
        }
        if updated.iter().any(|u| cond_vars.contains(u)) {
            return;
        }
        // the body advances the tested variable (`for (r = 3; r > 0; attempts++) { r--; }`)
        if n.field("body").is_some_and(|b| assigned_names(b, cx.src).iter().any(|c| cond_vars.contains(c))) {
            return;
        }
        // the condition must test the declared counter for this to be clearly wrong
        if counters.iter().any(|c| cond_vars.contains(c)) && !counters.iter().any(|c| updated.contains(c)) {
            cx.report(updates[0], format!("The loop tests `{}` but updates `{}`", counters.iter().find(|c| cond_vars.contains(c)).unwrap(), updated.join(", ")));
        }
    }
}

declare_rule!(StraySemicolon {
    id: "stray-semicolon",
    name: "Stray semicolon",
    category: Style,
    severity: Info,
    sources: ["sonar:S1116", "pmd:UnnecessarySemicolon", "intellij:UnnecessarySemicolon"],
    kinds: ["block", "class_body"],
    description: "An empty statement `;` inside a block or class body does nothing.",
});

impl Rule for StraySemicolon {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let mut c = n.walk();
        let kids: Vec<Node> = n.children(&mut c).collect();
        for (i, k) in kids.iter().enumerate() {
            if k.kind() == ";" && !k.is_named() {
                // in a class body, a `;` right after an enum constant list is required
                if n.kind() == "class_body" && i == 0 {
                    continue;
                }
                let _ = cx.text(*k);
                cx.report(*k, "Stray semicolon");
            }
        }
    }
}

declare_rule!(ElseAfterReturn {
    id: "else-after-return",
    name: "else after a branch that always returns",
    category: Style,
    severity: Info,
    enabled: false,
    sources: ["intellij:RedundantElse", "pmd:UnnecessaryElse"],
    kinds: ["if_statement"],
    description: "When the if-branch always returns or throws, the else block can be un-nested.",
});

impl Rule for ElseAfterReturn {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let (Some(c), Some(a)) = (n.field("consequence"), n.field("alternative")) else { return };
        if a.kind() == "if_statement" {
            return;
        }
        if !can_complete_normally(c, cx.src) {
            cx.report(a, "Unnecessary else after a branch that always exits");
        }
    }
}
