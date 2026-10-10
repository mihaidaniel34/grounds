//! Shared helpers for rules: syntax predicates, expression comparisons,
//! method/class classification.

use grounds_index::*;
use grounds_semantic::FileModel;
use grounds_syntax::{Node, NodeExt};

pub fn op<'t>(n: Node<'t>) -> &'static str {
    n.field("operator").map(|o| o.kind()).unwrap_or("")
}

pub fn lr<'t>(n: Node<'t>) -> Option<(Node<'t>, Node<'t>)> {
    Some((n.field("left")?, n.field("right")?))
}

pub fn is_null_literal(n: Node) -> bool {
    n.unparen().kind() == "null_literal"
}

pub fn is_literal(n: Node) -> bool {
    grounds_flow::cfg::is_literal(n.unparen().kind())
}

/// Does the expression possibly have side effects (calls, assignments,
/// increments, object creation)?
pub fn has_side_effects(n: Node) -> bool {
    n.descendants().any(|d| {
        matches!(
            d.kind(),
            "method_invocation" | "object_creation_expression" | "assignment_expression" | "update_expression" | "array_creation_expression" | "lambda_expression"
        )
    })
}

/// Text with whitespace and comments removed, parentheses at the top level
/// stripped; used to compare expressions for structural equality.
pub fn normalized(n: Node, src: &str) -> String {
    let n = n.unparen();
    let mut out = String::new();
    norm_rec(n, src, &mut out);
    out
}

fn norm_rec(n: Node, src: &str, out: &mut String) {
    if grounds_syntax::is_comment(n.kind()) {
        return;
    }
    if n.child_count() == 0 {
        out.push_str(n.text(src));
        out.push(' ');
        return;
    }
    let mut c = n.walk();
    for k in n.children(&mut c) {
        norm_rec(k, src, out);
    }
}

/// Structurally identical and free of side effects.
pub fn same_pure_expr(a: Node, b: Node, src: &str) -> bool {
    !has_side_effects(a) && !has_side_effects(b) && normalized(a, src) == normalized(b, src)
}

/// The expression statement's value is discarded.
pub fn value_discarded(n: Node) -> bool {
    match n.parent() {
        Some(p) => p.kind() == "expression_statement" || (p.kind() == "lambda_expression" && false),
        None => false,
    }
}

/// Is `n` inside a loop body within the same callable?
pub fn in_loop(n: Node) -> bool {
    for a in n.ancestors() {
        match a.kind() {
            "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement" => return true,
            "method_declaration" | "constructor_declaration" | "lambda_expression" | "class_body" => return false,
            _ => {}
        }
    }
    false
}

/// Nearest enclosing loop statement within the same callable.
pub fn enclosing_loop<'t>(n: Node<'t>) -> Option<Node<'t>> {
    for a in n.ancestors() {
        match a.kind() {
            "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement" => return Some(a),
            "method_declaration" | "constructor_declaration" | "lambda_expression" | "class_body" => return None,
            _ => {}
        }
    }
    None
}

pub fn in_synchronized(n: Node) -> bool {
    for a in n.ancestors() {
        match a.kind() {
            "synchronized_statement" => return true,
            "method_declaration" => return a.has_modifier("synchronized"),
            "constructor_declaration" | "lambda_expression" | "class_body" => return false,
            _ => {}
        }
    }
    false
}

pub fn method_name<'t>(decl: Node<'t>, src: &'t str) -> &'t str {
    decl.field("name").map(|n| n.text(src)).unwrap_or("")
}

pub fn param_count(decl: Node) -> usize {
    decl.field("parameters").map(|p| p.kids().iter().filter(|k| matches!(k.kind(), "formal_parameter" | "spread_parameter")).count()).unwrap_or(0)
}

/// Annotation simple names on a declaration.
pub fn annotation_names<'t>(decl: Node<'t>, src: &'t str) -> Vec<&'t str> {
    let mut v = Vec::new();
    if let Some(m) = decl.modifiers() {
        for a in m.kids() {
            if matches!(a.kind(), "annotation" | "marker_annotation") {
                if let Some(n) = a.field("name") {
                    let t = n.text(src);
                    v.push(t.rsplit('.').next().unwrap_or(t));
                }
            }
        }
    }
    for a in decl.kids() {
        if matches!(a.kind(), "annotation" | "marker_annotation") {
            if let Some(n) = a.field("name") {
                let t = n.text(src);
                v.push(t.rsplit('.').next().unwrap_or(t));
            }
        }
    }
    v
}

pub fn has_annotation_named(decl: Node, src: &str, simple: &str) -> bool {
    annotation_names(decl, src).contains(&simple)
}

/// The type declaration node directly containing a member declaration.
pub fn owning_type_decl<'t>(member: Node<'t>) -> Option<Node<'t>> {
    let body = member.parent()?;
    let body = if body.kind() == "enum_body_declarations" { body.parent()? } else { body };
    let d = body.parent()?;
    if grounds_syntax::is_type_declaration(d.kind()) || d.kind() == "object_creation_expression" || d.kind() == "enum_constant" {
        Some(d)
    } else {
        None
    }
}

/// Members of a type declaration body (flattening enum body declarations).
pub fn members<'t>(type_decl: Node<'t>) -> Vec<Node<'t>> {
    let Some(body) = type_decl.field("body").or_else(|| type_decl.kid_of_kind("class_body")) else { return Vec::new() };
    let mut out = Vec::new();
    for m in body.kids() {
        if m.kind() == "enum_body_declarations" {
            out.extend(m.kids());
        } else {
            out.push(m);
        }
    }
    out
}

pub fn methods_of<'t>(type_decl: Node<'t>) -> Vec<Node<'t>> {
    members(type_decl).into_iter().filter(|m| m.kind() == "method_declaration").collect()
}

/// Does the method declaration override/implement a supertype method?
pub fn overrides_super<'t>(fm: &FileModel<'t>, decl: Node<'t>) -> Tri {
    if has_annotation_named(decl, fm.src, "Override") {
        return Tri::Yes;
    }
    let Some(m) = fm.method_of_decl(decl) else { return Tri::Unknown };
    if m.flags.is_static() || m.flags.is_private() || m.is_constructor() {
        return Tri::No;
    }
    let Some(cls) = fm.enclosing_class(decl) else { return Tri::Unknown };
    let mut complete = true;
    for st in fm.direct_supertypes(&cls.this_type()) {
        if st.class_name().is_none() {
            complete = false;
            continue;
        }
        if !fm.hierarchy_complete(st.class_name().unwrap()) {
            complete = false;
        }
        for cand in fm.methods_named(&st, &m.name) {
            if cand.method.flags.is_private() || cand.method.flags.is_static() {
                continue;
            }
            if cand.method.params.len() == m.params.len()
                && cand.method.params.iter().zip(&m.params).all(|(a, b)| a.ty.erasure() == b.ty.erasure() || a.ty.mentions_type_var() || b.ty.has_unknown())
            {
                return Tri::Yes;
            }
        }
    }
    // also interfaces of anonymous classes, Object methods
    if matches!((&*m.name, m.params.len()), ("equals", 1) | ("hashCode", 0) | ("toString", 0) | ("clone", 0) | ("finalize", 0)) {
        return Tri::Yes;
    }
    if complete {
        Tri::No
    } else {
        Tri::Unknown
    }
}

pub fn is_main_method(decl: Node, src: &str) -> bool {
    method_name(decl, src) == "main" && decl.has_modifier("static") && param_count(decl) <= 1
}

/// Simple name of a type syntax node (`java.util.List<String>` -> `List`).
pub fn type_simple_name<'t>(t: Node<'t>, src: &'t str) -> &'t str {
    match t.kind() {
        "generic_type" => t.kids().first().map(|k| type_simple_name(*k, src)).unwrap_or(""),
        "scoped_type_identifier" => t.kids().last().map(|k| type_simple_name(*k, src)).unwrap_or(""),
        "array_type" => t.field("element").map(|e| type_simple_name(e, src)).unwrap_or(""),
        "annotated_type" => t.kids().last().map(|k| type_simple_name(*k, src)).unwrap_or(""),
        _ => t.text(src),
    }
}

/// Is the expression a call to one of `names` (by syntax only)?
pub fn is_call_named(n: Node, src: &str, names: &[&str]) -> bool {
    n.kind() == "method_invocation" && n.field("name").is_some_and(|x| names.contains(&x.text(src)))
}

pub fn call_object<'t>(n: Node<'t>) -> Option<Node<'t>> {
    n.field("object")
}

/// The variable/field symbol name an expression refers to, if simple.
pub fn simple_ref_name<'t>(n: Node<'t>, src: &'t str) -> Option<&'t str> {
    let n = n.unparen();
    match n.kind() {
        "identifier" => Some(n.text(src)),
        "field_access" if n.field("object").is_some_and(|o| o.kind() == "this") => n.field("field").map(|f| f.text(src)),
        _ => None,
    }
}

/// Is the class (by name) a subtype of any of `supers` (definitely)?
pub fn is_subtype_any(fm: &FileModel, ty: &Ty, supers: &[&str]) -> bool {
    match ty.class_name() {
        Some(n) => supers.iter().any(|s| fm.is_subclass(n, s).is_yes()),
        None => false,
    }
}

/// Two types definitely cannot have a common instance (both concrete classes
/// unrelated, or a final class not implementing an interface).
pub fn definitely_unrelated(fm: &FileModel, a: &Ty, b: &Ty) -> bool {
    let a = a.boxed();
    let b = b.boxed();
    let (Some(an), Some(bn)) = (a.class_name(), b.class_name()) else { return false };
    if an == bn {
        return false;
    }
    if !fm.is_subclass(an, bn).is_no() || !fm.is_subclass(bn, an).is_no() {
        return false;
    }
    let (Some(ac), Some(bc)) = (fm.class(an), fm.class(bn)) else { return false };
    match (ac.is_interface(), bc.is_interface()) {
        (false, false) => true,
        (true, false) => bc.flags.is_final() || bc.is_enum() || bc.is_record(),
        (false, true) => ac.flags.is_final() || ac.is_enum() || ac.is_record(),
        (true, true) => false,
    }
}

pub fn is_string_type(t: &Ty) -> bool {
    t.is_string()
}

/// Line of a node (0-based).
pub fn line_of(n: Node) -> usize {
    n.start_position().row
}

/// Enclosing method/constructor declaration node (not lambda).
pub fn enclosing_method_decl<'t>(n: Node<'t>) -> Option<Node<'t>> {
    for a in n.ancestors() {
        match a.kind() {
            "method_declaration" | "constructor_declaration" | "compact_constructor_declaration" => return Some(a),
            "class_body" | "interface_body" | "enum_body" => return None,
            _ => {}
        }
    }
    None
}

/// Statement-level: can this statement complete normally (JLS 14.22, approx)?
pub fn can_complete_normally(n: Node, src: &str) -> bool {
    match n.kind() {
        "return_statement" | "throw_statement" | "break_statement" | "continue_statement" | "yield_statement" => false,
        "block" => {
            let kids = n.kids();
            match kids.last() {
                Some(last) => kids.iter().all(|k| k == last || can_complete_normally(*k, src)) && can_complete_normally(*last, src),
                None => true,
            }
        }
        "if_statement" => match (n.field("consequence"), n.field("alternative")) {
            (Some(c), Some(a)) => can_complete_normally(c, src) || can_complete_normally(a, src),
            _ => true,
        },
        "while_statement" => {
            let infinite = n.field("condition").and_then(|c| c.first_kid()).is_some_and(|c| c.kind() == "true");
            !infinite || contains_break_for(n)
        }
        "do_statement" => n.field("body").is_some_and(|b| can_complete_normally(b, src)) || contains_break_for(n),
        "for_statement" => n.field("condition").is_some() || contains_break_for(n),
        "try_statement" | "try_with_resources_statement" => {
            if let Some(f) = n.kid_of_kind("finally_clause").and_then(|f| f.kid_of_kind("block")) {
                if !can_complete_normally(f, src) {
                    return false;
                }
            }
            let body_ok = n.field("body").is_some_and(|b| can_complete_normally(b, src));
            body_ok || n.kids_of_kind("catch_clause").iter().any(|c| c.field("body").is_some_and(|b| can_complete_normally(b, src)))
        }
        "labeled_statement" => true,
        "synchronized_statement" => n.field("body").is_none_or(|b| can_complete_normally(b, src)),
        "expression_statement" => {
            // System.exit(...) still "completes" syntactically; treat as abrupt
            let t = n.text(src);
            !(t.starts_with("System.exit(") || t.starts_with("Runtime.getRuntime().halt("))
        }
        "switch_expression" => true,
        _ => true,
    }
}

fn contains_break_for(loop_node: Node) -> bool {
    // any unlabeled break not nested in an inner loop/switch, or labeled break
    fn rec(n: Node, depth: usize) -> bool {
        for k in n.kids() {
            match k.kind() {
                "break_statement" => {
                    let labeled = k.kids().iter().any(|x| x.kind() == "identifier");
                    if labeled || depth == 0 {
                        return true;
                    }
                }
                "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement" | "switch_expression" => {
                    if rec(k, depth + 1) {
                        return true;
                    }
                }
                "lambda_expression" | "class_body" => {}
                _ => {
                    if rec(k, depth) {
                        return true;
                    }
                }
            }
        }
        false
    }
    n_body(loop_node).is_some_and(|b| rec(b, 0))
}

fn n_body(n: Node) -> Option<Node> {
    n.field("body")
}

/// Does the subtree (not descending into lambdas/classes) contain a node of kind?
pub fn contains_kind(n: Node, kinds: &[&str]) -> bool {
    for k in n.kids() {
        if kinds.contains(&k.kind()) {
            return true;
        }
        if matches!(k.kind(), "lambda_expression" | "class_body") {
            continue;
        }
        if contains_kind(k, kinds) {
            return true;
        }
    }
    false
}

/// Collect descendants of kind without entering lambdas / class bodies.
pub fn find_kind<'t>(n: Node<'t>, kind: &str, out: &mut Vec<Node<'t>>) {
    for k in n.kids() {
        if k.kind() == kind {
            out.push(k);
        }
        if matches!(k.kind(), "lambda_expression" | "class_body" | "class_declaration") {
            continue;
        }
        find_kind(k, kind, out);
    }
}

pub fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

/// Truncate long code snippets for messages.
pub fn snippet(s: &str) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 50 {
        format!("{}…", one.chars().take(47).collect::<String>())
    } else {
        one
    }
}

/// Is the expression a Throwable (by type, or because it is a catch parameter
/// whose type could not be resolved)?
pub fn is_throwable_expr<'t>(fm: &FileModel<'t>, n: Node<'t>) -> bool {
    let n = n.unparen();
    if is_subtype_any(fm, &fm.type_of(n), &["java.lang.Throwable"]) {
        return true;
    }
    if let Some(i) = fm.local_ref(n) {
        return fm.binder.decls[i].kind == grounds_semantic::LocalKind::CatchParam;
    }
    false
}

/// Next named sibling that is not a comment.
pub fn next_sibling_stmt(n: Node) -> Option<Node> {
    let mut s = n.next_named_sibling();
    while let Some(x) = s {
        if !grounds_syntax::is_comment(x.kind()) {
            return Some(x);
        }
        s = x.next_named_sibling();
    }
    None
}

/// Previous named sibling that is not a comment.
pub fn prev_sibling_stmt(n: Node) -> Option<Node> {
    let mut s = n.prev_named_sibling();
    while let Some(x) = s {
        if !grounds_syntax::is_comment(x.kind()) {
            return Some(x);
        }
        s = x.prev_named_sibling();
    }
    None
}

/// Does a block contain a comment on its own line (not trailing the `{`)?
pub fn block_has_own_line_comment(b: Node) -> bool {
    let start_row = b.start_position().row;
    let mut c = b.walk();
    let r = b.children(&mut c).any(|k| grounds_syntax::is_comment(k.kind()) && k.start_position().row > start_row);
    r
}
