//! Local name binding: maps identifier uses to local declarations
//! (variables, parameters, pattern bindings, lambda parameters, ...).
//! Anything not bound here is a field, type or package name.

use grounds_syntax::{Node, NodeExt};
use rustc_hash::FxHashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    Local,
    Param,
    LambdaParam,
    CatchParam,
    ForEach,
    Resource,
    Pattern,
}

#[derive(Clone, Debug)]
pub struct LocalDecl<'t> {
    pub name: &'t str,
    /// The identifier node naming the variable.
    pub name_node: Node<'t>,
    /// The declaring construct (variable_declarator, formal_parameter, ...).
    pub decl_node: Node<'t>,
    pub kind: LocalKind,
    /// Explicit type syntax node, if any (absent for lambda inferred params).
    pub type_node: Option<Node<'t>>,
    /// Extra array dimensions on the declarator (`int x[]`).
    pub extra_dims: usize,
    /// Method, constructor, lambda or initializer that owns this variable.
    pub owner: Option<Node<'t>>,
    /// Declared with `final`.
    pub is_final: bool,
}

#[derive(Default, Debug)]
pub struct Binder<'t> {
    pub decls: Vec<LocalDecl<'t>>,
    /// identifier node id -> decl index
    pub refs: FxHashMap<usize, usize>,
    /// decl name node id -> decl index
    pub by_name_node: FxHashMap<usize, usize>,
    /// decl index -> uses
    pub uses: Vec<Vec<Node<'t>>>,
    /// References to a local of an enclosing method from inside a class body
    /// (anonymous, local or member class): the identifier and the classes
    /// crossed, innermost first. A field the class inherits shadows the local
    /// (JLS 6.4.1), which only the class hierarchy can tell.
    pub crossed: Vec<(Node<'t>, Vec<Node<'t>>)>,
    /// References to a pattern variable that may or may not be in scope
    /// there (whether a branch completes normally is not known): the name is
    /// unknown, neither the variable nor a field.
    pub uncertain: rustc_hash::FxHashSet<usize>,
}

enum Frame<'t> {
    /// Locals in scope: name, declaration, and whether only maybe in scope.
    Vars(Vec<(&'t str, usize, bool)>),
    /// Class boundary: member fields with these names shadow outer locals;
    /// the class's declaration (or anonymous body) node.
    Barrier(Vec<&'t str>, Option<Node<'t>>),
}

struct State<'t, 's> {
    src: &'s str,
    b: Binder<'t>,
    stack: Vec<Frame<'t>>,
    owners: Vec<Node<'t>>,
}

pub fn bind<'t>(root: Node<'t>, src: &'t str) -> Binder<'t> {
    let mut st = State { src, b: Binder::default(), stack: vec![Frame::Vars(Vec::new())], owners: Vec::new() };
    st.visit(root);
    st.b
}

fn dims_of(n: Option<Node>, src: &str) -> usize {
    n.map(|d| d.text(src).matches('[').count()).unwrap_or(0)
}

impl<'t, 's> State<'t, 's>
where
    's: 't,
{
    fn push(&mut self) {
        self.stack.push(Frame::Vars(Vec::new()));
    }
    fn pop(&mut self) {
        self.stack.pop();
    }

    fn declare(&mut self, name_node: Node<'t>, decl_node: Node<'t>, kind: LocalKind, type_node: Option<Node<'t>>, extra_dims: usize, is_final: bool) {
        if name_node.kind() != "identifier" {
            return;
        }
        let name = name_node.text(self.src);
        if name == "_" {
            return;
        }
        let idx = self.b.decls.len();
        self.b.decls.push(LocalDecl {
            name,
            name_node,
            decl_node,
            kind,
            type_node,
            extra_dims,
            owner: self.owners.last().copied(),
            is_final,
        });
        self.b.uses.push(Vec::new());
        self.b.by_name_node.insert(name_node.id(), idx);
        if let Some(Frame::Vars(v)) = self.stack.iter_mut().rev().find(|f| matches!(f, Frame::Vars(_))) {
            v.push((name, idx, false));
        }
    }

    /// The local `name` refers to, whether it is only maybe in scope, and the
    /// classes crossed to reach it.
    fn lookup(&self, name: &str) -> Option<(usize, bool, Vec<Node<'t>>)> {
        let mut crossed = Vec::new();
        for f in self.stack.iter().rev() {
            match f {
                Frame::Vars(v) => {
                    if let Some((_, i, maybe)) = v.iter().rev().find(|(n, _, _)| *n == name) {
                        return Some((*i, *maybe, crossed));
                    }
                }
                Frame::Barrier(fields, class) => {
                    if fields.contains(&name) {
                        return None;
                    }
                    crossed.extend(*class);
                }
            }
        }
        None
    }

    fn reference(&mut self, ident: Node<'t>) {
        let name = ident.text(self.src);
        if let Some((i, maybe, crossed)) = self.lookup(name) {
            if maybe {
                self.b.uncertain.insert(ident.id());
                return;
            }
            self.b.refs.insert(ident.id(), i);
            self.b.uses[i].push(ident);
            if !crossed.is_empty() {
                self.b.crossed.push((ident, crossed));
            }
        }
    }

    /// Visit the children of `n` that are not in the named fields (error
    /// nodes in code that does not parse).
    fn visit_rest(&mut self, n: Node<'t>, fields: &[&str]) {
        let skip: Vec<Node<'t>> = fields.iter().filter_map(|f| n.field(f)).collect();
        for k in n.kids() {
            if !skip.contains(&k) {
                self.visit(k);
            }
        }
    }

    /// Visit a branch with pattern variables in scope, in a frame of its own
    /// when there are any (without, as before: code that does not parse
    /// may put a declaration there, `#if (x) Foo a = ...;` in a template).
    fn visit_with(&mut self, branch: Node<'t>, bindings: &[usize]) {
        if bindings.is_empty() {
            self.visit(branch);
            return;
        }
        self.push();
        self.bring_into_scope(bindings);
        self.visit(branch);
        self.pop();
    }

    /// Put declared locals back in scope, in the innermost frame.
    fn bring_into_scope(&mut self, decls: &[usize]) {
        self.bring_into_scope_if(decls, Some(true));
    }

    /// Put declared locals in scope when `certain` is true, as maybe in scope
    /// when it is not known (None), not at all when false.
    fn bring_into_scope_if(&mut self, decls: &[usize], certain: Option<bool>) {
        if certain == Some(false) {
            return;
        }
        for &i in decls {
            let name = self.b.decls[i].name;
            if let Some(Frame::Vars(v)) = self.stack.iter_mut().rev().find(|f| matches!(f, Frame::Vars(_))) {
                v.push((name, i, certain.is_none()));
            }
        }
    }

    /// Visit the condition of an `if` or `while` in a scope of its own, and
    /// return the pattern variables it introduces when true and when false
    /// (JLS 6.3.1).
    fn condition(&mut self, cond: Option<Node<'t>>) -> (Vec<usize>, Vec<usize>) {
        let Some(cond) = cond else { return (Vec::new(), Vec::new()) };
        let first = self.b.decls.len();
        self.push();
        self.visit(cond);
        self.pop();
        let (mut when_true, mut when_false) = (Vec::new(), Vec::new());
        for i in first..self.b.decls.len() {
            let d = &self.b.decls[i];
            if d.kind != LocalKind::Pattern {
                continue;
            }
            match introduced_when(d.decl_node, cond) {
                Some(true) => when_true.push(i),
                Some(false) => when_false.push(i),
                None => {}
            }
        }
        (when_true, when_false)
    }

    fn visit_kids(&mut self, n: Node<'t>) {
        for k in n.kids() {
            self.visit(k);
        }
    }

    fn params(&mut self, params: Option<Node<'t>>, kind: LocalKind) {
        let Some(ps) = params else { return };
        for p in ps.kids() {
            match p.kind() {
                "formal_parameter" if grounds_index::stubs::is_receiver_parameter(p, self.src) => {}
                "formal_parameter" => {
                    let is_final = p.has_modifier("final");
                    if let Some(n) = p.field("name") {
                        self.declare(n, p, kind, p.field("type"), dims_of(p.field("dimensions"), self.src), is_final);
                    }
                }
                "spread_parameter" => {
                    let is_final = p.has_modifier("final");
                    let ty = p.kids().into_iter().find(|k| !matches!(k.kind(), "modifiers" | "variable_declarator" | "annotation" | "marker_annotation"));
                    if let Some(d) = p.kid_of_kind("variable_declarator") {
                        if let Some(n) = d.field("name") {
                            // varargs: one extra dimension
                            self.declare(n, p, kind, ty, 1, is_final);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn class_barrier(&mut self, body: Option<Node<'t>>) {
        let mut names = Vec::new();
        if let Some(b) = body {
            let mut members = b.kids();
            if let Some(ebd) = members.iter().find(|m| m.kind() == "enum_body_declarations").copied() {
                members.extend(ebd.kids());
            }
            for m in members {
                if matches!(m.kind(), "field_declaration" | "constant_declaration") {
                    for d in m.fields("declarator") {
                        if let Some(n) = d.field("name") {
                            names.push(n.text(self.src));
                        }
                    }
                }
                if m.kind() == "enum_constant" {
                    if let Some(n) = m.field("name") {
                        names.push(n.text(self.src));
                    }
                }
            }
        }
        let class = body.and_then(|b| if b.kind() == "class_body" && b.parent().is_some_and(|p| matches!(p.kind(), "object_creation_expression" | "enum_constant")) { Some(b) } else { b.parent() });
        self.stack.push(Frame::Barrier(names, class));
    }

    fn bind_pattern(&mut self, p: Node<'t>) {
        match p.kind() {
            "type_pattern" => {
                let ty = p.kids().into_iter().find(|k| k.kind() != "identifier" && k.kind() != "modifiers");
                if let Some(id) = p.kids().into_iter().rev().find(|k| k.kind() == "identifier") {
                    self.declare(id, p, LocalKind::Pattern, ty, 0, false);
                }
            }
            "record_pattern" | "record_pattern_body" | "pattern" => {
                for k in p.kids() {
                    self.bind_pattern(k);
                }
            }
            "record_pattern_component" => {
                // `final var cause`: the modifiers are not the type
                let ty = p.kids().into_iter().find(|k| !matches!(k.kind(), "identifier" | "modifiers"));
                if let Some(id) = p.kids().into_iter().rev().find(|k| k.kind() == "identifier") {
                    self.declare(id, p, LocalKind::Pattern, ty, 0, false);
                } else {
                    for k in p.kids() {
                        self.bind_pattern(k);
                    }
                }
            }
            _ => {}
        }
    }

    fn visit(&mut self, n: Node<'t>) {
        match n.kind() {
            "identifier" => self.reference(n),
            "package_declaration" | "import_declaration" | "line_comment" | "block_comment" | "annotation" | "marker_annotation"
            | "type_identifier" | "scoped_type_identifier" | "generic_type" | "scoped_identifier" => {}
            "modifiers" => {}
            "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration" | "annotation_type_declaration" => {
                self.class_barrier(n.field("body"));
                // record components are fields of the record
                if n.kind() == "record_declaration" {
                    if let Some(Frame::Barrier(names, _)) = self.stack.last_mut() {
                        if let Some(ps) = n.field("parameters") {
                            for p in ps.kids() {
                                if let Some(nm) = p.field("name") {
                                    names.push(nm.text(self.src));
                                }
                            }
                        }
                    }
                }
                if let Some(b) = n.field("body") {
                    self.visit_body_members(b, Some(n));
                }
                self.pop();
            }
            "object_creation_expression" => {
                for k in n.kids() {
                    if k.kind() == "class_body" {
                        self.class_barrier(Some(k));
                        self.visit_body_members(k, None);
                        self.pop();
                    } else {
                        self.visit(k);
                    }
                }
            }
            "method_declaration" | "constructor_declaration" => {
                self.owners.push(n);
                self.push();
                self.params(n.field("parameters"), LocalKind::Param);
                if let Some(b) = n.field("body") {
                    self.visit(b);
                }
                self.pop();
                self.owners.pop();
            }
            "compact_constructor_declaration" => {
                self.owners.push(n);
                self.push();
                if let Some(rec) = n.ancestor_of_kind(&["record_declaration"]) {
                    self.params(rec.field("parameters"), LocalKind::Param);
                }
                if let Some(b) = n.field("body") {
                    self.visit(b);
                }
                self.pop();
                self.owners.pop();
            }
            "lambda_expression" => {
                self.owners.push(n);
                self.push();
                if let Some(p) = n.field("parameters") {
                    match p.kind() {
                        "identifier" => self.declare(p, n, LocalKind::LambdaParam, None, 0, false),
                        "inferred_parameters" => {
                            for id in p.kids() {
                                self.declare(id, n, LocalKind::LambdaParam, None, 0, false);
                            }
                        }
                        "formal_parameters" => self.params(Some(p), LocalKind::LambdaParam),
                        _ => {}
                    }
                }
                if let Some(b) = n.field("body") {
                    self.visit(b);
                }
                self.pop();
                self.owners.pop();
            }
            "static_initializer" => {
                self.owners.push(n);
                self.visit_kids(n);
                self.owners.pop();
            }
            // locals declared in a colon-style case group stay in scope for
            // the following groups: the scope is the whole switch block
            // (`case 0: int v; case 1: v = 2;` is valid Java)
            "block" | "constructor_body" | "switch_block" | "switch_rule" => {
                let is_init_block = n.kind() == "block" && n.parent().is_some_and(|p| matches!(p.kind(), "class_body" | "enum_body_declarations"));
                if is_init_block {
                    self.owners.push(n);
                }
                self.push();
                self.visit_kids(n);
                self.pop();
                if is_init_block {
                    self.owners.pop();
                }
            }
            "local_variable_declaration" => {
                let ty = n.field("type");
                let is_final = n.has_modifier("final");
                for d in n.fields("declarator") {
                    if let Some(v) = d.field("value") {
                        self.visit(v);
                    }
                    if let Some(nm) = d.field("name") {
                        self.declare(nm, d, LocalKind::Local, ty, dims_of(d.field("dimensions"), self.src), is_final);
                    }
                }
                // local class declarations inside handled by "class_declaration"
            }
            // pattern variables of the condition are in scope where it is
            // true or false, and after the statement when the other branch
            // cannot complete normally (JLS 6.3.1, 6.3.2)
            "if_statement" => {
                let (when_true, when_false) = self.condition(n.field("condition"));
                let then = n.field("consequence");
                let alt = n.field("alternative");
                if let Some(t) = then {
                    self.visit_with(t, &when_true);
                }
                if let Some(e) = alt {
                    self.visit_with(e, &when_false);
                }
                // in scope after the `if` when the other branch cannot
                // complete normally; maybe in scope when that is not known
                if let Some(t) = then {
                    self.bring_into_scope_if(&when_false, completes_normally(t).map(|c| !c));
                }
                if let Some(e) = alt {
                    self.bring_into_scope_if(&when_true, completes_normally(e).map(|c| !c));
                }
                self.visit_rest(n, &["condition", "consequence", "alternative"]);
            }
            "while_statement" => {
                let (when_true, when_false) = self.condition(n.field("condition"));
                if let Some(b) = n.field("body") {
                    self.visit_with(b, &when_true);
                }
                // in scope after the loop unless a break may leave it
                // JLS 6.3.2.3: no reachable break that targets the loop, even
                // one a finally keeps from leaving it
                self.bring_into_scope_if(&when_false, breaks_targeting(n, false).map(|b| !b));
                self.visit_rest(n, &["condition", "body"]);
            }
            "for_statement" => {
                self.push();
                for k in n.fields("init") {
                    self.visit(k);
                }
                if let Some(c) = n.field("condition") {
                    self.visit(c);
                }
                for k in n.fields("update") {
                    self.visit(k);
                }
                if let Some(b) = n.field("body") {
                    self.visit(b);
                }
                self.pop();
            }
            "enhanced_for_statement" => {
                if let Some(v) = n.field("value") {
                    self.visit(v);
                }
                self.push();
                if let Some(nm) = n.field("name") {
                    self.declare(nm, n, LocalKind::ForEach, n.field("type"), dims_of(n.field("dimensions"), self.src), n.has_modifier("final"));
                }
                if let Some(b) = n.field("body") {
                    self.visit(b);
                }
                self.pop();
            }
            "catch_clause" => {
                self.push();
                if let Some(p) = n.kid_of_kind("catch_formal_parameter") {
                    let ty = p.kid_of_kind("catch_type");
                    if let Some(nm) = p.field("name") {
                        self.declare(nm, p, LocalKind::CatchParam, ty, 0, p.has_modifier("final"));
                    }
                }
                if let Some(b) = n.field("body") {
                    self.visit(b);
                }
                self.pop();
            }
            "try_with_resources_statement" => {
                self.push();
                if let Some(rs) = n.field("resources") {
                    for r in rs.kids() {
                        if let Some(v) = r.field("value") {
                            self.visit(v);
                        }
                        match r.field("name") {
                            Some(nm) => self.declare(nm, r, LocalKind::Resource, r.field("type"), 0, true),
                            None => self.visit_kids(r),
                        }
                    }
                }
                if let Some(b) = n.field("body") {
                    self.visit(b);
                }
                self.pop();
                for k in n.kids() {
                    if matches!(k.kind(), "catch_clause" | "finally_clause") {
                        self.visit(k);
                    }
                }
            }
            "instanceof_expression" => {
                if let Some(l) = n.field("left") {
                    self.visit(l);
                }
                if let Some(nm) = n.field("name") {
                    self.declare(nm, n, LocalKind::Pattern, n.field("right"), 0, false);
                }
                if let Some(p) = n.field("pattern") {
                    self.bind_pattern(p);
                }
            }
            "switch_label" => {
                for k in n.kids() {
                    match k.kind() {
                        "pattern" | "type_pattern" | "record_pattern" => self.bind_pattern(k),
                        _ => self.visit(k),
                    }
                }
            }
            "method_invocation" => {
                for k in n.kids() {
                    if Some(k) == n.field("name") {
                        continue;
                    }
                    self.visit(k);
                }
            }
            "field_access" => {
                if let Some(o) = n.field("object") {
                    self.visit(o);
                }
            }
            "method_reference" => {
                if let Some(f) = n.first_kid() {
                    self.visit(f);
                }
            }
            "labeled_statement" => {
                for k in n.kids() {
                    if k.kind() != "identifier" {
                        self.visit(k);
                    }
                }
            }
            "break_statement" | "continue_statement" => {}
            "enum_constant" => {
                if let Some(a) = n.field("arguments") {
                    self.visit(a);
                }
                if let Some(b) = n.field("body") {
                    self.class_barrier(Some(b));
                    self.visit_body_members(b, None);
                    self.pop();
                }
            }
            "variable_declarator" => {
                if let Some(v) = n.field("value") {
                    self.visit(v);
                }
            }
            "explicit_constructor_invocation" => self.visit_kids(n),
            "element_value_pair" => {
                if let Some(v) = n.field("value") {
                    self.visit(v);
                }
            }
            "formal_parameters" | "type_parameters" | "type_arguments" | "dimensions" => {}
            _ => self.visit_kids(n),
        }
    }

    fn visit_body_members(&mut self, body: Node<'t>, _decl: Option<Node<'t>>) {
        for m in body.kids() {
            match m.kind() {
                "field_declaration" | "constant_declaration" => {
                    self.owners.push(m);
                    for d in m.fields("declarator") {
                        if let Some(v) = d.field("value") {
                            self.visit(v);
                        }
                    }
                    self.owners.pop();
                }
                "enum_body_declarations" => self.visit_body_members(m, None),
                _ => self.visit(m),
            }
        }
    }
}

/// Whether the pattern variable declared at `decl` is introduced by `cond`
/// when it is true, when it is false, or neither (JLS 6.3.1): through
/// parentheses, `!`, the operands of `&&` (when true) and of `||` (when false).
fn introduced_when(decl: Node, cond: Node) -> Option<bool> {
    let mut when = true;
    let mut cur = decl;
    // from a pattern up to its `instanceof`
    while !matches!(cur.kind(), "instanceof_expression") {
        cur = cur.parent()?;
    }
    while cur != cond {
        let p = cur.parent()?;
        match p.kind() {
            "parenthesized_expression" => {}
            "unary_expression" if p.field("operator").is_some_and(|o| o.kind() == "!") => when = !when,
            "binary_expression" => match p.field("operator").map(|o| o.kind()) {
                Some("&&") if when => {}
                Some("||") if !when => {}
                _ => return None,
            },
            _ => return None,
        }
        cur = p;
    }
    Some(when)
}

/// Whether a statement can complete normally (JLS 14.22): Some(false) for
/// `return`, `throw`, `break`, `continue`, `yield`, a block ending with one,
/// an `if` whose branches both cannot, a `try` whose blocks cannot, a loop
/// that runs forever with no break out; Some(true) where the rules say it
/// completes; None for everything they are not implemented for (a labeled
/// statement, a switch, a condition that is not a literal...), so that a gap
/// leaves a name unknown instead of taking another declaration.
fn completes_normally(s: Node) -> Option<bool> {
    match s.kind() {
        "return_statement" | "throw_statement" | "break_statement" | "continue_statement" | "yield_statement" => Some(false),
        "block" | "constructor_body" => match s.kids().into_iter().rfind(|k| !matches!(k.kind(), "line_comment" | "block_comment" | "{" | "}")) {
            Some(last) => completes_normally(last),
            None => Some(true),
        },
        "if_statement" => match (s.field("consequence"), s.field("alternative")) {
            (Some(t), Some(e)) => or3(completes_normally(t), completes_normally(e)),
            _ => Some(true),
        },
        "synchronized_statement" => s.kid_of_kind("block").map_or(Some(true), completes_normally),
        // the try block or a catch completes, and the finally block does
        "try_statement" | "try_with_resources_statement" => {
            let mut body = s.field("body").map_or(Some(true), completes_normally);
            for c in s.kids().into_iter().filter(|k| k.kind() == "catch_clause") {
                body = or3(body, c.field("body").map_or(Some(true), completes_normally));
            }
            match s.kid_of_kind("finally_clause").and_then(|f| f.kid_of_kind("block")) {
                Some(f) => match completes_normally(f) {
                    Some(false) => Some(false),
                    Some(true) => body,
                    None => body.filter(|b| !b),
                },
                None => body,
            }
        }
        // JLS 14.22: a `while` or `for` completes normally iff its condition
        // is not constant true or a break leaves it; a `do` also needs its body
        // (or a continue) to reach the condition. A condition other than a
        // literal may still be constant: unknown.
        "while_statement" | "for_statement" => match loop_condition(s) {
            Some(true) => breaks_targeting(s, true),
            Some(false) => Some(true),
            None => None,
        },
        "do_statement" => {
            let breaks = breaks_targeting(s, true);
            if breaks == Some(true) {
                return Some(true);
            }
            let reaches_condition = match s.field("body").map_or(Some(true), completes_normally) {
                Some(true) => Some(true),
                // a continue reaches the condition too: not modelled
                Some(false) if !s.field("body").is_some_and(|b| b.descendants().any(|d| d.kind() == "continue_statement")) => Some(false),
                _ => None,
            };
            match (reaches_condition, loop_condition(s), breaks) {
                (Some(false), _, Some(false)) => Some(false),
                (Some(true), Some(false), _) => Some(true),
                (_, Some(true), Some(false)) => Some(false),
                _ => None,
            }
        }
        // these complete normally when reachable (JLS 14.22)
        "expression_statement" | "local_variable_declaration" | "assert_statement" | ";" | "empty_statement" | "class_declaration" | "record_declaration"
        | "enum_declaration" | "interface_declaration" | "explicit_constructor_invocation" | "enhanced_for_statement" => Some(true),
        // a labeled statement, a switch, and anything not listed: unknown
        _ => None,
    }
}

/// Three-valued "either completes".
fn or3(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

/// The value of a loop's condition when it is the literal `true` or
/// `false`; a `for` without one is true. None for anything else, which may
/// still be a constant expression (`1 == 1`, a constant variable).
fn loop_condition(s: Node) -> Option<bool> {
    match s.field("condition") {
        None if s.kind() == "for_statement" => Some(true),
        None => None,
        Some(c) => match c.unparen().kind() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
    }
}

/// Whether a `break` in the body of `loop_` targets it (JLS 14.15), and with
/// `leaving`, also gets out through every `finally` in between (14.20.2):
/// Some(true) for an unlabeled one that does, None when that cannot be told
/// (a labeled break, a `finally` that may not complete), Some(false) when
/// none does.
fn breaks_targeting(loop_: Node, leaving: bool) -> Option<bool> {
    let Some(body) = loop_.field("body") else { return Some(false) };
    let mut unknown = false;
    for b in body.descendants().filter(|d| d.kind() == "break_statement") {
        if b.kid_of_kind("identifier").is_some() {
            unknown = true;
            continue;
        }
        // the target, and the finally blocks on the way out
        let mut leaves = Some(true);
        let mut prev = b;
        let mut cur = b.parent();
        let mut targets = false;
        while let Some(a) = cur {
            if a == loop_ {
                targets = true;
                break;
            }
            if matches!(a.kind(), "while_statement" | "do_statement" | "for_statement" | "enhanced_for_statement" | "switch_block" | "switch_expression") {
                break;
            }
            if leaving && matches!(a.kind(), "try_statement" | "try_with_resources_statement") {
                let finally = a.kid_of_kind("finally_clause");
                let inside_finally = finally.is_some_and(|f| f == prev);
                if let (Some(f), false) = (finally.and_then(|f| f.kid_of_kind("block")), inside_finally) {
                    leaves = match completes_normally(f) {
                        Some(true) => leaves,
                        Some(false) => Some(false),
                        None => None,
                    };
                }
            }
            prev = a;
            cur = a.parent();
        }
        if targets {
            match leaves {
                Some(true) => return Some(true),
                None => unknown = true,
                Some(false) => {}
            }
        }
    }
    if unknown {
        None
    } else {
        Some(false)
    }
}

impl<'t> Binder<'t> {
    pub fn decl_of(&self, ident: Node<'t>) -> Option<&LocalDecl<'t>> {
        self.refs.get(&ident.id()).map(|&i| &self.decls[i])
    }
    pub fn decl_index_of_ref(&self, ident: Node<'t>) -> Option<usize> {
        self.refs.get(&ident.id()).copied()
    }
    pub fn decl_by_name_node(&self, name_node: Node<'t>) -> Option<&LocalDecl<'t>> {
        self.by_name_node.get(&name_node.id()).map(|&i| &self.decls[i])
    }
    pub fn decl_index(&self, name_node: Node<'t>) -> Option<usize> {
        self.by_name_node.get(&name_node.id()).copied()
    }
    pub fn uses(&self, decl_idx: usize) -> &[Node<'t>] {
        &self.uses[decl_idx]
    }
}
