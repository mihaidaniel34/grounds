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
}

enum Frame<'t> {
    Vars(Vec<(&'t str, usize)>),
    /// Class boundary: member fields with these names shadow outer locals.
    Barrier(Vec<&'t str>),
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

    fn lookup(&self, name: &str) -> Option<usize> {
        for f in self.stack.iter().rev() {
            match f {
                Frame::Vars(v) => {
                    if let Some((_, i)) = v.iter().rev().find(|(n, _)| *n == name) {
                        return Some(*i);
                    }
                }
                Frame::Barrier(fields) => {
                    if fields.contains(&name) {
                        return None;
                    }
                }
            }
        }
        None
    }

    fn reference(&mut self, ident: Node<'t>) {
        let name = ident.text(self.src);
        if let Some(i) = self.lookup(name) {
            self.b.refs.insert(ident.id(), i);
            self.b.uses[i].push(ident);
        }
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
        self.stack.push(Frame::Barrier(names));
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
                let ty = p.kids().into_iter().find(|k| k.kind() != "identifier");
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
                    if let Some(Frame::Barrier(names)) = self.stack.last_mut() {
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
            "block" | "constructor_body" | "switch_block_statement_group" | "switch_rule" => {
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
