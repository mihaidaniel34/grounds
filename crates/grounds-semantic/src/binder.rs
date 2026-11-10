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
}

enum Frame<'t> {
    Vars(Vec<(&'t str, usize)>),
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
            v.push((name, idx));
        }
    }

    /// The local `name` refers to, and the classes crossed to reach it.
    fn lookup(&self, name: &str) -> Option<(usize, Vec<Node<'t>>)> {
        let mut crossed = Vec::new();
        for f in self.stack.iter().rev() {
            match f {
                Frame::Vars(v) => {
                    if let Some((_, i)) = v.iter().rev().find(|(n, _)| *n == name) {
                        return Some((*i, crossed));
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
        if let Some((i, crossed)) = self.lookup(name) {
            self.b.refs.insert(ident.id(), i);
            self.b.uses[i].push(ident);
            if !crossed.is_empty() {
                self.b.crossed.push((ident, crossed));
            }
        }
    }

    /// Put declared locals back in scope, in the innermost frame.
    fn bring_into_scope(&mut self, decls: &[usize]) {
        for &i in decls {
            let name = self.b.decls[i].name;
            if let Some(Frame::Vars(v)) = self.stack.iter_mut().rev().find(|f| matches!(f, Frame::Vars(_))) {
                v.push((name, i));
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
                self.push();
                self.bring_into_scope(&when_true);
                if let Some(t) = then {
                    self.visit(t);
                }
                self.pop();
                if let Some(e) = alt {
                    self.push();
                    self.bring_into_scope(&when_false);
                    self.visit(e);
                    self.pop();
                }
                if then.is_some_and(|t| !completes_normally(t)) {
                    self.bring_into_scope(&when_false);
                }
                if alt.is_some_and(|e| !completes_normally(e)) {
                    self.bring_into_scope(&when_true);
                }
            }
            "while_statement" => {
                let (when_true, when_false) = self.condition(n.field("condition"));
                self.push();
                self.bring_into_scope(&when_true);
                if let Some(b) = n.field("body") {
                    self.visit(b);
                }
                self.pop();
                let breaks = n.field("body").is_some_and(|b| b.descendants().any(|d| d.kind() == "break_statement"));
                if !breaks {
                    self.bring_into_scope(&when_false);
                }
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

/// Whether a statement can complete normally, roughly (JLS 14.22): not when it
/// is a `return`, `throw`, `break`, `continue` or `yield`, a block ending with
/// one, or an `if` whose branches both cannot.
fn completes_normally(s: Node) -> bool {
    match s.kind() {
        "return_statement" | "throw_statement" | "break_statement" | "continue_statement" | "yield_statement" => false,
        "block" => s.kids().into_iter().rfind(|k| !matches!(k.kind(), "line_comment" | "block_comment")).is_none_or(completes_normally),
        "if_statement" => match (s.field("consequence"), s.field("alternative")) {
            (Some(t), Some(e)) => completes_normally(t) || completes_normally(e),
            _ => true,
        },
        _ => true,
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
