//! Complexity metrics and comment hygiene.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(CognitiveComplexity),
        Box::new(DeepNesting),
        Box::new(MethodTooLong),
        Box::new(TodoComment),
        Box::new(FixmeComment),
        Box::new(CommentedOutCode),
        Box::new(ComplexBooleanExpression),
    ]
}

/// Sonar's cognitive complexity of a method body.
pub fn cognitive_complexity(body: Node, src: &str, method_name: &str) -> u32 {
    fn logical_sequences(e: Node) -> u32 {
        // count changes of operator in sequences of && / ||
        fn flatten<'t>(e: Node<'t>, out: &mut Vec<&'t str>) {
            let e = e.unparen();
            if e.kind() == "binary_expression" {
                let o = e.field("operator").map(|o| o.kind()).unwrap_or("");
                if o == "&&" || o == "||" {
                    if let Some(l) = e.field("left") {
                        flatten(l, out);
                    }
                    out.push(o);
                    if let Some(r) = e.field("right") {
                        flatten(r, out);
                    }
                }
            }
        }
        let mut ops = Vec::new();
        flatten(e, &mut ops);
        let mut n = 0;
        let mut prev = "";
        for o in ops {
            if o != prev {
                n += 1;
                prev = o;
            }
        }
        n
    }
    fn walk(n: Node, nesting: u32, src: &str, name: &str, total: &mut u32) {
        for k in n.kids() {
            let kind = k.kind();
            match kind {
                "if_statement" => {
                    let is_else_if = k.parent().is_some_and(|p| p.kind() == "if_statement" && p.field("alternative") == Some(k));
                    *total += if is_else_if { 1 } else { 1 + nesting };
                    if let Some(c) = k.field("condition") {
                        *total += logical_sequences(c.unparen());
                        walk(c, nesting + 1, src, name, total);
                    }
                    if let Some(c) = k.field("consequence") {
                        walk(c, nesting + 1, src, name, total);
                    }
                    if let Some(a) = k.field("alternative") {
                        if a.kind() == "if_statement" {
                            walk_single(a, nesting, src, name, total);
                        } else {
                            *total += 1; // else
                            walk(a, nesting + 1, src, name, total);
                        }
                    }
                }
                "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement" | "catch_clause" | "switch_expression" => {
                    *total += 1 + nesting;
                    if let Some(c) = k.field("condition") {
                        *total += logical_sequences(c.unparen());
                    }
                    walk(k, nesting + 1, src, name, total);
                }
                "ternary_expression" => {
                    *total += 1 + nesting;
                    walk(k, nesting + 1, src, name, total);
                }
                "lambda_expression" | "class_body" => walk(k, nesting + 1, src, name, total),
                "break_statement" | "continue_statement" if k.kids().iter().any(|x| x.kind() == "identifier") => {
                    *total += 1;
                }
                "binary_expression" => {
                    // sequences outside conditions (e.g. assignments)
                    let o = k.field("operator").map(|o| o.kind()).unwrap_or("");
                    let parent_is_same_seq = k.parent_unparen().is_some_and(|p| p.kind() == "binary_expression" && matches!(p.field("operator").map(|o| o.kind()), Some("&&") | Some("||")));
                    let in_condition = k.ancestors().take(3).any(|a| matches!(a.kind(), "parenthesized_expression") && a.parent().is_some_and(|pp| matches!(pp.kind(), "if_statement" | "while_statement" | "do_statement")));
                    if (o == "&&" || o == "||") && !parent_is_same_seq && !in_condition {
                        *total += logical_sequences(k);
                    }
                    walk(k, nesting, src, name, total);
                }
                "method_invocation" => {
                    if k.field("object").is_none() && k.field("name").is_some_and(|nm| nm.text(src) == name) {
                        *total += 1; // recursion
                    }
                    walk(k, nesting, src, name, total);
                }
                _ => walk(k, nesting, src, name, total),
            }
        }
    }
    fn walk_single(k: Node, nesting: u32, src: &str, name: &str, total: &mut u32) {
        // an `else if` node visited through its parent
        *total += 1;
        if let Some(c) = k.field("condition") {
            *total += logical_sequences(c.unparen());
        }
        if let Some(c) = k.field("consequence") {
            walk(c, nesting + 1, src, name, total);
        }
        if let Some(a) = k.field("alternative") {
            if a.kind() == "if_statement" {
                walk_single(a, nesting, src, name, total);
            } else {
                *total += 1;
                walk(a, nesting + 1, src, name, total);
            }
        }
    }
    let mut total = 0;
    walk(body, 0, src, method_name, &mut total);
    total
}

declare_rule!(CognitiveComplexity {
    id: "cognitive-complexity",
    name: "Method is too hard to understand",
    category: Complexity,
    severity: Info,
    sources: ["sonar:S3776"],
    kinds: ["method_declaration", "constructor_declaration"],
    description: "Cognitive complexity counts branches weighted by nesting (Sonar's metric). High values make code hard to reason about and test; split the method. Threshold: `max` option (default 15).",
});

impl Rule for CognitiveComplexity {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(body) = n.field("body") else { return };
        let max = cx.option_usize("max", 15) as u32;
        let c = cognitive_complexity(body, cx.src, method_name(n, cx.src));
        if c > max {
            if let Some(nm) = n.field("name") {
                cx.report(nm, format!("Cognitive complexity is {c} (max {max}); split this method"));
            }
        }
    }
}

declare_rule!(DeepNesting {
    id: "deep-nesting",
    name: "Control flow nested too deeply",
    category: Complexity,
    severity: Info,
    sources: ["sonar:S134", "pmd:AvoidDeeplyNestedIfStmts"],
    kinds: ["if_statement", "for_statement", "enhanced_for_statement", "while_statement", "do_statement", "switch_expression", "try_statement"],
    description: "Deeply nested control flow is hard to follow. Use early returns or extract methods. Threshold: `max` option (default 4).",
});

impl Rule for DeepNesting {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.kind() == "if_statement" && n.parent().is_some_and(|p| p.kind() == "if_statement" && p.field("alternative") == Some(n)) {
            return;
        }
        let max = cx.option_usize("max", 4);
        let depth = n
            .ancestors()
            .take_while(|a| !matches!(a.kind(), "method_declaration" | "constructor_declaration" | "lambda_expression" | "class_body"))
            .filter(|a| matches!(a.kind(), "if_statement" | "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement" | "switch_expression" | "try_statement"))
            .filter(|a| !(a.kind() == "if_statement" && a.parent().is_some_and(|p| p.kind() == "if_statement" && p.field("alternative") == Some(*a))))
            .count();
        if depth == max {
            cx.report(n.child(0).unwrap_or(n), format!("Nesting depth exceeds {max}; extract a method or return early"));
        }
    }
}

declare_rule!(MethodTooLong {
    id: "method-too-long",
    name: "Method is too long",
    category: Complexity,
    severity: Info,
    sources: ["sonar:S138", "pmd:NcssCount"],
    kinds: ["method_declaration", "constructor_declaration"],
    description: "Very long methods do too many things and are hard to test. Threshold: `max` option (lines of code, default 150).",
});

impl Rule for MethodTooLong {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(body) = n.field("body") else { return };
        let max = cx.option_usize("max", 150);
        let text = cx.text(body);
        let loc = text.lines().filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with("//") && !t.starts_with('*') && !t.starts_with("/*")
        }).count();
        if loc > max {
            if let Some(nm) = n.field("name") {
                cx.report(nm, format!("Method has {loc} lines of code (max {max})"));
            }
        }
    }
}

fn comment_task(text: &str, tag: &str) -> bool {
    let t = text.trim_start_matches(['/', '*', ' ', '\t']);
    let up = t.to_ascii_uppercase();
    if let Some(i) = up.find(tag) {
        let before_ok = i == 0 || !up.as_bytes()[i - 1].is_ascii_alphanumeric();
        let after = up.as_bytes().get(i + tag.len()).copied();
        let after_ok = after.is_none_or(|c| !c.is_ascii_alphanumeric());
        return before_ok && after_ok;
    }
    false
}

declare_rule!(TodoComment {
    id: "todo-comment",
    name: "TODO comment",
    category: Style,
    severity: Info,
    sources: ["sonar:S1135"],
    kinds: ["line_comment", "block_comment"],
    description: "TODOs mark unfinished work. New TODOs in a change are worth a second look: they often stand for code an agent or developer meant to come back to.",
});

impl Rule for TodoComment {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if comment_task(cx.text(n), "TODO") {
            cx.report(n, "TODO left in code");
        }
    }
}

declare_rule!(FixmeComment {
    id: "fixme-comment",
    name: "FIXME comment",
    category: Style,
    severity: Warning,
    sources: ["sonar:S1134"],
    kinds: ["line_comment", "block_comment"],
    description: "FIXME marks known broken code.",
});

impl Rule for FixmeComment {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let t = cx.text(n);
        // XXX only as a leading tag: "XXX-XXXX" or "C:\Users\xxx" are placeholders
        let lead = t.trim_start_matches(['/', '*', ' ', '\t']);
        let xxx = lead.starts_with("XXX") && lead.as_bytes().get(3).is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'-' && *c != b'.');
        if comment_task(t, "FIXME") || xxx {
            cx.report(n, "FIXME left in code");
        }
    }
}

fn looks_like_code(line: &str) -> bool {
    // drop a trailing `// comment` inside the commented-out line
    let line = match line.find("//") {
        Some(i) if i > 0 => &line[..i],
        _ => line,
    };
    let t = line.trim();
    if t.is_empty() || t.starts_with('@') && t.contains(' ') && !t.contains('(') {
        return false;
    }
    let ends = t.ends_with(';') || t.ends_with('{') || t.ends_with('}') || t.ends_with(");");
    let has_code_shape = t.contains('(') || t.contains('=') || t.starts_with("return") || t.starts_with("if ") || t.starts_with("for ") || t.starts_with('}') || t.starts_with("import ");
    // prose with semicolons rarely contains these tokens together
    let words = t.split_whitespace().count();
    ends && has_code_shape && !(words > 8 && !t.contains('(')) && !t.starts_with("e.g.") && !t.contains("http")
}

declare_rule!(CommentedOutCode {
    id: "commented-out-code",
    name: "Commented-out code",
    category: Style,
    severity: Info,
    sources: ["sonar:S125"],
    kinds: ["line_comment", "block_comment"],
    description: "Commented-out code rots quickly and confuses readers; version control already keeps the history. Delete it.",
});

impl Rule for CommentedOutCode {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let t = cx.text(n);
        if t.starts_with("/**") || t.starts_with("///") {
            return;
        }
        // only report the first line of a run of line comments
        if n.kind() == "line_comment" {
            if let Some(p) = n.prev_sibling() {
                if p.kind() == "line_comment" && p.end_position().row + 1 == n.start_position().row {
                    return;
                }
            }
            // trailing comments after code on the same line are explanations
            if n.prev_sibling().is_some_and(|p| p.end_position().row == n.start_position().row) {
                return;
            }
        }
        let mut lines: Vec<String> = Vec::new();
        if n.kind() == "line_comment" {
            let mut cur = Some(n);
            while let Some(c) = cur {
                lines.push(cx.text(c).trim_start_matches('/').to_string());
                cur = c.next_sibling().filter(|x| x.kind() == "line_comment" && x.start_position().row == c.end_position().row + 1);
            }
        } else {
            lines = t.trim_start_matches("/*").trim_end_matches("*/").lines().map(|l| l.trim_start_matches([' ', '*']).to_string()).collect();
        }
        let code_lines = lines.iter().filter(|l| looks_like_code(l)).count();
        let non_empty = lines.iter().filter(|l| !l.trim().is_empty()).count();
        if code_lines >= 1 && code_lines * 2 >= non_empty {
            cx.report(n, "Commented-out code; delete it");
        }
    }
}

declare_rule!(ComplexBooleanExpression {
    id: "complex-boolean-expression",
    name: "Boolean expression with too many operators",
    category: Complexity,
    severity: Info,
    sources: ["sonar:S1067"],
    kinds: ["binary_expression"],
    description: "Conditions with many `&&`/`||` operators are hard to read and to get right. Extract parts into well-named variables or methods. Threshold: `max` option (default 3).",
});

impl Rule for ComplexBooleanExpression {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(op(n), "&&" | "||") {
            return;
        }
        if n.parent_unparen().is_some_and(|p| p.kind() == "binary_expression" && matches!(op(p), "&&" | "||")) {
            return;
        }
        let count = n.descendants().filter(|d| d.kind() == "binary_expression" && matches!(op(*d), "&&" | "||")).filter(|d| !d.ancestors().take_while(|a| *a != n).any(|a| matches!(a.kind(), "lambda_expression" | "method_invocation"))).count();
        let max = cx.option_usize("max", 3);
        if count > max {
            cx.report(n, format!("Boolean expression has {count} operators (max {max}); extract parts into named variables"));
        }
    }
}
