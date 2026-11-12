//! Expression typing, name classification, expected (target) types and
//! constant folding.

use crate::binder::LocalKind;
use crate::memo::Query;
use crate::model::{FileModel, Named, Symbol};
use grounds_index::doubt::{self, Doubt};
use grounds_index::stubs::{fold_binary, fold_unary, literal_constant};
use grounds_index::*;
use grounds_syntax::{Node, NodeExt};
use std::sync::Arc;

/// What a simple name used as a case label denotes (see `simple_case_label`).
enum CaseLabel {
    /// the constant of the selector's enum
    Constant(FieldRef),
    /// an expression name, resolved as anywhere else
    Expression,
    /// the selector's type is unknown, or its enum lacks the constant
    Unknown,
}

pub fn binary_promotion(a: Prim, b: Prim) -> Prim {
    if a == Prim::Double || b == Prim::Double {
        Prim::Double
    } else if a == Prim::Float || b == Prim::Float {
        Prim::Float
    } else if a == Prim::Long || b == Prim::Long {
        Prim::Long
    } else {
        Prim::Int
    }
}

/// The type of a numeric conditional expression (JLS 15.25.2) whose
/// operands have the (unboxed) types `x` and `y`; `cx`/`cy` is the value of
/// an operand that is a constant expression of type int. The boxed and
/// unboxed forms of a type give that primitive type.
pub fn numeric_conditional(x: Prim, y: Prim, cx: Option<i64>, cy: Option<i64>) -> Prim {
    if x == y {
        return x;
    }
    // byte or Byte with short or Short
    if matches!((x, y), (Prim::Byte, Prim::Short) | (Prim::Short, Prim::Byte)) {
        return Prim::Short;
    }
    // T in {byte, short, char} (or its box) with an int constant representable in T
    let fits = |t: Prim, v: i64| match t {
        Prim::Byte => (-128..=127).contains(&v),
        Prim::Short => (-32768..=32767).contains(&v),
        Prim::Char => (0..=0xFFFF).contains(&v),
        _ => false,
    };
    if y == Prim::Int && cy.is_some_and(|v| fits(x, v)) {
        return x;
    }
    if x == Prim::Int && cx.is_some_and(|v| fits(y, v)) {
        return y;
    }
    binary_promotion(x, y)
}

pub fn unary_promotion(a: Prim) -> Prim {
    match a {
        Prim::Byte | Prim::Short | Prim::Char => Prim::Int,
        p => p,
    }
}

impl<'t> FileModel<'t> {
    /// Static type of an expression node. `Unknown` when not determinable.
    pub fn type_of(&self, n: Node<'t>) -> Ty {
        let t = self.types.get_or_compute(&self.guards, n.id(), n.id(), Query::Type, || self.compute_type(n));
        if t.is_unknown() {
            if !self.denotes_value(n) {
                return Ty::NoType;
            }
            doubt::note(Doubt::UnknownType);
        }
        t
    }

    /// Whether `n` is an expression that has a value, whose type is then
    /// unknown when `type_of` answers `Unknown`; not a node of another kind
    /// (an argument list, a type or package name, a method's name), which
    /// has no type to know.
    fn denotes_value(&self, n: Node<'t>) -> bool {
        match n.kind() {
            "identifier" | "field_access" => !self.names_no_expression(n) && !matches!(self.symbol_quietly(n), Some(Symbol::Type(_) | Symbol::Package(_))),
            "parenthesized_expression" | "class_literal" | "this" | "method_invocation" | "object_creation_expression" | "array_creation_expression" | "array_initializer" | "array_access" | "cast_expression"
            | "instanceof_expression" | "assignment_expression" | "update_expression" | "unary_expression" | "binary_expression" | "ternary_expression" | "lambda_expression" | "method_reference"
            | "switch_expression" => true,
            _ => false,
        }
    }

    /// `symbol` without what it observes: asked only to classify a node.
    fn symbol_quietly(&self, n: Node<'t>) -> Option<Symbol<'t>> {
        doubt::quiet(|| self.symbol(n))
    }

    fn compute_type(&self, n: Node<'t>) -> Ty {
        let src = self.src;
        match n.kind() {
            "parenthesized_expression" => n.first_kid().map(|k| self.type_of(k)).unwrap_or(Ty::Unknown),
            "decimal_integer_literal" | "hex_integer_literal" | "octal_integer_literal" | "binary_integer_literal" => {
                if n.text(src).ends_with(['l', 'L']) {
                    Ty::Prim(Prim::Long)
                } else {
                    Ty::Prim(Prim::Int)
                }
            }
            "decimal_floating_point_literal" | "hex_floating_point_literal" => {
                if n.text(src).ends_with(['f', 'F']) {
                    Ty::Prim(Prim::Float)
                } else {
                    Ty::Prim(Prim::Double)
                }
            }
            "character_literal" => Ty::Prim(Prim::Char),
            "string_literal" | "template_expression" => Ty::string(),
            "true" | "false" => Ty::boolean(),
            "null_literal" => Ty::Null,
            "class_literal" => {
                let t = n.first_kid().map(|k| self.resolve_type_node(k)).unwrap_or(Ty::Unknown);
                let arg = match t {
                    Ty::Prim(p) => Ty::class(p.box_name()),
                    Ty::Void => Ty::class("java.lang.Void"),
                    t => t,
                };
                Ty::class_with("java.lang.Class", vec![arg])
            }
            "this" => match self.enclosing_class_name(n) {
                Some(c) => self.class(&c).map(|ci| ci.this_type()).unwrap_or_else(|| Ty::class(&c)),
                None => Ty::Unknown,
            },
            "identifier" => match self.symbol(n) {
                Some(Symbol::Local(i, _)) => self.local_type(i),
                Some(Symbol::Field(f)) => f.field.ty,
                _ => Ty::Unknown,
            },
            "field_access" => {
                let field = n.field("field");
                if field.is_some_and(|f| f.kind() == "this") {
                    // Outer.this
                    if let Some(o) = n.field("object") {
                        if let Some(Symbol::Type(t)) = self.symbol(o) {
                            return self.class(&t).map(|c| c.this_type()).unwrap_or_else(|| Ty::class(&t));
                        }
                    }
                    return Ty::Unknown;
                }
                if let (Some(o), Some(f)) = (n.field("object"), field) {
                    if f.text(src) == "length" {
                        let ot = self.type_of(o);
                        if ot.is_array() {
                            return Ty::int();
                        }
                    }
                }
                match self.symbol(n) {
                    Some(Symbol::Field(f)) => f.field.ty,
                    _ => Ty::Unknown,
                }
            }
            "method_invocation" => self.resolve_call(n).map(|c| c.ret.clone()).unwrap_or(Ty::Unknown),
            "object_creation_expression" => self.creation_type(n),
            "array_creation_expression" => {
                let base = n.field("type").map(|t| self.resolve_type_node(t)).unwrap_or(Ty::Unknown);
                if base.is_unknown() {
                    return Ty::Unknown;
                }
                let mut dims = 0;
                for d in n.fields("dimensions") {
                    dims += match d.kind() {
                        "dimensions_expr" => 1,
                        _ => d.text(src).matches('[').count(),
                    };
                }
                let mut t = base;
                for _ in 0..dims.max(1) {
                    t = Ty::Array(Box::new(t));
                }
                t
            }
            "array_initializer" => self.expected_type(n),
            "array_access" => match n.field("array").map(|a| self.type_of(a)) {
                Some(Ty::Array(e)) => *e,
                _ => Ty::Unknown,
            },
            "cast_expression" => {
                let tys = n.fields("type");
                if tys.len() == 1 {
                    self.resolve_type_node(tys[0])
                } else {
                    Ty::Unknown
                }
            }
            "instanceof_expression" => Ty::boolean(),
            "assignment_expression" => n.field("left").map(|l| self.type_of(l)).unwrap_or(Ty::Unknown),
            "update_expression" => n.first_kid().map(|k| self.type_of(k)).unwrap_or(Ty::Unknown),
            "unary_expression" => {
                let op = n.field("operator").map(|o| o.kind()).unwrap_or("");
                let t = n.field("operand").map(|o| self.type_of(o)).unwrap_or(Ty::Unknown);
                match op {
                    "!" => Ty::boolean(),
                    _ => match t.unboxed() {
                        Some(p) if p.is_numeric() => Ty::Prim(unary_promotion(p)),
                        _ => Ty::Unknown,
                    },
                }
            }
            "binary_expression" => self.binary_type(n),
            "ternary_expression" => {
                let a = n.field("consequence").map(|x| self.type_of(x)).unwrap_or(Ty::Unknown);
                let b = n.field("alternative").map(|x| self.type_of(x)).unwrap_or(Ty::Unknown);
                // the value of an operand that is a constant expression of type int
                let int_constant = |x: Option<Node<'t>>, t: &Ty| match (x, t) {
                    (Some(x), Ty::Prim(Prim::Int)) => match self.constant_value(x) {
                        Some(Constant::Int(v)) => Some(v),
                        _ => None,
                    },
                    _ => None,
                };
                let (ca, cb) = (int_constant(n.field("consequence"), &a), int_constant(n.field("alternative"), &b));
                self.conditional_type_of(&a, &b, ca, cb)
            }
            "lambda_expression" | "method_reference" => self.expected_type(n),
            "switch_expression" => self.switch_expression_type(n),
            _ => Ty::Unknown,
        }
    }

    fn binary_type(&self, n: Node<'t>) -> Ty {
        let op = n.field("operator").map(|o| o.kind()).unwrap_or("");
        match op {
            "==" | "!=" | "<" | "<=" | ">" | ">=" | "&&" | "||" => return Ty::boolean(),
            _ => {}
        }
        let l = n.field("left").map(|x| self.type_of(x)).unwrap_or(Ty::Unknown);
        let r = n.field("right").map(|x| self.type_of(x)).unwrap_or(Ty::Unknown);
        if op == "+" && (l.is_string() || r.is_string()) {
            return Ty::string();
        }
        match op {
            "<<" | ">>" | ">>>" => match l.unboxed() {
                Some(p) if p.is_integral() => Ty::Prim(unary_promotion(p)),
                _ => Ty::Unknown,
            },
            "&" | "|" | "^" if l.is_boolean() && r.is_boolean() => Ty::boolean(),
            _ => match (l.unboxed(), r.unboxed()) {
                (Some(a), Some(b)) if a.is_numeric() && b.is_numeric() => Ty::Prim(binary_promotion(a, b)),
                _ => Ty::Unknown,
            },
        }
    }

    /// JLS 15.25, simplified.
    pub fn conditional_type(&self, a: &Ty, b: &Ty) -> Ty {
        self.conditional_type_of(a, b, None, None)
    }

    /// JLS 15.25, with the value of each operand that is a constant
    /// expression of type int (`ca`, `cb`), for the numeric table of 15.25.2.
    pub fn conditional_type_of(&self, a: &Ty, b: &Ty, ca: Option<i64>, cb: Option<i64>) -> Ty {
        if a == b {
            return a.clone();
        }
        match (a, b) {
            (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
            (Ty::Null, t) | (t, Ty::Null) => t.boxed(),
            _ => {
                if a.is_boolean() && b.is_boolean() {
                    return Ty::boolean();
                }
                if let (Some(x), Some(y)) = (a.unboxed(), b.unboxed()) {
                    if x.is_numeric() && y.is_numeric() && (a.is_prim() || b.is_prim() || x != y) {
                        return Ty::Prim(numeric_conditional(x, y, ca, cb));
                    }
                }
                if self.is_assignable(a, b).is_yes() {
                    return b.clone();
                }
                if self.is_assignable(b, a).is_yes() {
                    return a.clone();
                }
                Ty::Unknown
            }
        }
    }

    fn switch_expression_type(&self, n: Node<'t>) -> Ty {
        let Some(body) = n.field("body") else { return Ty::Unknown };
        let mut result: Option<Ty> = None;
        for rule in body.kids() {
            let exprs: Vec<Node> = match rule.kind() {
                "switch_rule" => rule.kids().into_iter().filter(|k| k.kind() == "expression_statement").filter_map(|e| e.first_kid()).collect(),
                _ => Vec::new(),
            };
            for e in exprs {
                let t = self.type_of(e);
                result = Some(match result {
                    None => t,
                    Some(r) => self.conditional_type(&r, &t),
                });
            }
        }
        result.unwrap_or_else(|| self.expected_type(n))
    }

    /// Type of a local variable declaration by decl index.
    pub fn local_type(&self, idx: usize) -> Ty {
        let decl = self.binder.decls[idx].clone();
        let key = decl.name_node.id();
        let t = self.locals.get_or_compute(&self.guards, key, key, Query::Local, || {
            let base = match decl.type_node {
                Some(tn) if tn.text(self.src) != "var" => {
                    if tn.kind() == "catch_type" {
                        let alts = tn.kids();
                        if alts.len() == 1 {
                            self.resolve_type_node(alts[0])
                        } else {
                            Ty::Unknown
                        }
                    } else {
                        self.resolve_type_node(tn)
                    }
                }
                _ => match decl.kind {
                    LocalKind::Local | LocalKind::Resource => decl.decl_node.field("value").map(|v| self.type_of(v)).unwrap_or(Ty::Unknown),
                    LocalKind::ForEach => decl.decl_node.field("value").map(|v| self.element_type_of_iterable(&self.type_of(v))).unwrap_or(Ty::Unknown),
                    LocalKind::LambdaParam => self.lambda_param_type(decl.decl_node, decl.name_node),
                    LocalKind::Pattern if decl.decl_node.kind() == "record_pattern_component" => self.record_pattern_component_type(decl.decl_node),
                    _ => Ty::Unknown,
                },
            };
            let mut t = base;
            if t.is_known() {
                for _ in 0..decl.extra_dims {
                    t = Ty::Array(Box::new(t));
                }
            }
            t
        });
        if t.is_unknown() {
            doubt::note(Doubt::UnknownType);
        }
        t
    }

    pub fn element_type_of_iterable(&self, t: &Ty) -> Ty {
        match t {
            Ty::Array(e) => (**e).clone(),
            Ty::Class { .. } => match self.as_super(t, "java.lang.Iterable") {
                Some(it) => match it.arg(0) {
                    Some(a) => a.upper_bound(),
                    None => Ty::Unknown,
                },
                None => Ty::Unknown,
            },
            _ => Ty::Unknown,
        }
    }

    fn lambda_param_type(&self, lambda: Node<'t>, name_node: Node<'t>) -> Ty {
        if lambda.kind() != "lambda_expression" {
            // explicitly typed lambda param: formal_parameter
            return Ty::Unknown;
        }
        let Some(params) = lambda.field("parameters") else { return Ty::Unknown };
        let idx = match params.kind() {
            "identifier" => 0,
            _ => params.kids().iter().position(|k| *k == name_node || k.field("name") == Some(name_node)).unwrap_or(0),
        };
        let target = self.expected_type(lambda);
        // a target that is not (known to be) functional gives no type
        match self.functional_method(&target) {
            Functional::Method(m) => match m.method.params.get(idx) {
                Some(p) if !p.ty.mentions_type_var() => p.ty.upper_bound(),
                _ => Ty::Unknown,
            },
            Functional::Not | Functional::Unknown => Ty::Unknown,
        }
    }

    /// The result type of the function type of `target`; Unknown where it
    /// is not (known to be) a functional interface, or mentions a type
    /// variable.
    fn functional_result(&self, target: &Ty) -> Ty {
        match self.functional_method(target) {
            Functional::Method(m) if !m.method.ret.mentions_type_var() => m.method.ret,
            _ => Ty::Unknown,
        }
    }

    /// All bounds of a type variable visible at `at` (`T extends A & B`).
    /// A variable that no declaration in scope there names (one of another
    /// declaration, which a type the model did not substitute carried here)
    /// has a bound that is unknown, not Object.
    pub fn type_var_bounds(&self, name: &str, at: Node<'t>) -> Vec<Ty> {
        self.declared_type_var_bounds(name, at).unwrap_or_else(|| vec![Ty::Unknown])
    }

    /// The bounds of the type variable `name` declared in scope at `at`
    /// (Object when it has none); `None` when no declaration there names it.
    pub fn declared_type_var_bounds(&self, name: &str, at: Node<'t>) -> Option<Vec<Ty>> {
        for a in std::iter::once(at).chain(self.ancestors_of(at)) {
            if let Some(tp) = a.field("type_parameters") {
                for p in tp.kids_of_kind("type_parameter") {
                    if p.kid_of_kind("type_identifier").map(|i| i.text(self.src)) == Some(name) {
                        return Some(match p.kid_of_kind("type_bound") {
                            Some(b) => b.kids().into_iter().map(|t| self.resolve_type_node(t)).collect(),
                            None => vec![Ty::object()],
                        });
                    }
                }
            }
        }
        None
    }

    /// First bound of a type variable visible at `at` (Object when it is
    /// declared without one); unknown for a variable no declaration in scope
    /// names (see `type_var_bounds`).
    pub fn type_var_bound(&self, name: &str, at: Node<'t>) -> Ty {
        for a in std::iter::once(at).chain(self.ancestors_of(at)) {
            if let Some(tp) = a.field("type_parameters") {
                for p in tp.kids_of_kind("type_parameter") {
                    if p.kid_of_kind("type_identifier").map(|i| i.text(self.src)) == Some(name) {
                        if let Some(b) = p.kid_of_kind("type_bound").and_then(|b| b.first_kid()) {
                            return self.resolve_type_node(b);
                        }
                        return Ty::object();
                    }
                }
            }
        }
        Ty::Unknown
    }

    // ----- names -----

    /// What an identifier / field access / scoped identifier refers to.
    pub fn symbol(&self, n: Node<'t>) -> Option<Symbol<'t>> {
        let s = self.symbols.get_or_compute(&self.guards, n.id(), n.id(), Query::Symbol, || self.compute_symbol(n));
        match &s {
            // a name that resolves to nothing; not one that names no
            // expression (a declaration's name, a method's name, a label)
            None if matches!(n.kind(), "identifier" | "field_access" | "scoped_identifier" | "scoped_type_identifier" | "type_identifier") && !self.names_no_expression(n) => {
                doubt::note(Doubt::UnresolvedName)
            }
            // left a local by `inherited_fields_shadow_locals`: may be a field
            Some(Symbol::Local(..)) if self.shadow_unknown.contains(&n.id()) => doubt::note(Doubt::IncompleteHierarchy),
            _ => {}
        }
        s
    }

    /// An identifier that is not an expression name: a declaration's,
    /// method's or lambda parameter's name, a label, an annotation key.
    fn names_no_expression(&self, n: Node<'t>) -> bool {
        if n.kind() != "identifier" {
            return false;
        }
        let Some(p) = self.parent_of(n) else { return false };
        match p.kind() {
            "method_invocation" => p.field("name") == Some(n),
            "method_reference" => p.kids().last() == Some(&n) && p.kids().len() > 1,
            "variable_declarator" | "formal_parameter" | "catch_formal_parameter" | "enhanced_for_statement" | "resource" => p.field("name") == Some(n),
            "method_declaration" | "constructor_declaration" | "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration" | "annotation_type_declaration" | "enum_constant" => p.field("name") == Some(n),
            "labeled_statement" | "break_statement" | "continue_statement" | "inferred_parameters" => true,
            // `@A(key = VALUE)`: the key names an element, the value is an expression
            "element_value_pair" => p.field("key") == Some(n),
            "lambda_expression" => p.field("parameters") == Some(n),
            _ => false,
        }
    }

    /// What a simple name that is a case label of a switch means, by the
    /// type of the switch's selector (JLS 14.11.1). On an enum, the label is
    /// the name of one of its constants, whatever else the name means in
    /// scope (a field, a local, a static import): an identifier is never
    /// resolved as an expression there. On another known type it is an
    /// ordinary constant expression. With the selector's type unknown it
    /// may be either. `None` for a node that is not such a label. A
    /// qualified label (`case Kind.ON`, which Java 21 allows on a selector
    /// of any type the constant's enum is assignable to) names its enum
    /// itself and is resolved as the qualified name it is.
    fn simple_case_label(&self, n: Node<'t>) -> Option<CaseLabel> {
        if n.kind() != "identifier" {
            return None;
        }
        let label = self.parent_of(n).filter(|p| p.kind() == "switch_label")?;
        let switch = self.parent_of(label).and_then(|g| self.parent_of(g)).and_then(|b| self.parent_of(b)).filter(|s| s.kind() == "switch_expression")?;
        let Some(selector) = switch.field("condition") else { return Some(CaseLabel::Unknown) };
        let name = n.text(self.src);
        Some(match self.type_of(selector) {
            Ty::Prim(_) => CaseLabel::Expression,
            Ty::Class { name: cn, .. } => match self.class(&cn) {
                // an enum's constants are declared in it, never inherited
                Some(c) if c.is_enum() => match c.fields.iter().find(|f| *f.name == *name && f.flags.has(Flags::ENUM_CONSTANT)) {
                    Some(f) => CaseLabel::Constant(FieldRef { owner: c.name.clone(), field: f.clone() }),
                    None => CaseLabel::Unknown,
                },
                Some(_) => CaseLabel::Expression,
                None => CaseLabel::Unknown,
            },
            _ => CaseLabel::Unknown,
        })
    }

    fn compute_symbol(&self, n: Node<'t>) -> Option<Symbol<'t>> {
        match n.kind() {
            "identifier" => {
                match self.simple_case_label(n) {
                    Some(CaseLabel::Constant(f)) => return Some(Symbol::Field(f)),
                    Some(CaseLabel::Unknown) => return None,
                    Some(CaseLabel::Expression) | None => {}
                }
                if let Some(i) = self.binder.decl_index_of_ref(n) {
                    return Some(Symbol::Local(i, self.binder.decls[i].clone()));
                }
                // a pattern variable that may be in scope here, or not
                if self.binder.uncertain.contains(&n.id()) {
                    return None;
                }
                if let Some(p) = self.parent_of(n) {
                    if self.names_no_expression(n) {
                        return None;
                    }
                    // the member of `obj.name` is what the qualified expression
                    // denotes, not whatever `name` means as a simple name
                    let member = match p.kind() {
                        "field_access" => p.field("field") == Some(n),
                        "scoped_identifier" => p.field("name") == Some(n),
                        _ => false,
                    };
                    if member {
                        return self.symbol(p);
                    }
                }
                let name = n.text(self.src);
                let (field, d) = doubt::observe(|| self.field_named(n, name));
                if field.is_some() {
                    doubt::replay(d);
                    return field;
                }
                let scope = self.scope_at(n);
                let named = if let Some(t) = self.index.resolve_simple_type(name, &scope) {
                    Some(Symbol::Type(t))
                } else if self.index.package_exists(name) {
                    Some(Symbol::Package(name.to_string()))
                } else {
                    None
                };
                // a qualifier that names a visible type or package, and no visible
                // variable: an unseen supertype is not supposed to declare a
                // variable that obscures it (an assumption, see `grounds_index::doubt`)
                if !(named.is_some() && self.is_qualifier(n)) {
                    doubt::replay(d);
                }
                named
            }
            "field_access" | "scoped_identifier" | "scoped_type_identifier" => {
                let (obj, field) = if n.kind() == "field_access" {
                    (n.field("object")?, n.field("field")?)
                } else {
                    (n.field("scope").or_else(|| n.first_kid())?, n.field("name").or_else(|| n.kids().last().copied())?)
                };
                let fname = field.text(self.src);
                if obj.kind() == "super" {
                    let c = self.enclosing_class(n)?;
                    let st = c.superclass.clone()?;
                    return self.field(&st, fname).map(Symbol::Field);
                }
                match self.symbol(obj) {
                    Some(Symbol::Package(p)) => {
                        let q = format!("{p}.{fname}");
                        if self.index.class_exists(&q) {
                            Some(Symbol::Type(Arc::from(q.as_str())))
                        } else if self.index.package_exists(&q) {
                            Some(Symbol::Package(q))
                        } else {
                            None
                        }
                    }
                    Some(Symbol::Type(t)) => {
                        if let Some(f) = self.field(&Ty::class(&t), fname) {
                            return Some(Symbol::Field(f));
                        }
                        self.index.member_type(&t, fname).or_else(|| self.local_member_type(&t, fname)).map(Symbol::Type)
                    }
                    _ => {
                        let ot = self.type_of(obj);
                        let ot = match ot {
                            Ty::TypeVar(v) => self.type_var_bound(&v, n),
                            t => t,
                        };
                        if ot.is_unknown() {
                            return None;
                        }
                        self.field(&ot, fname).map(Symbol::Field)
                    }
                }
            }
            "type_identifier" => {
                let scope = self.scope_at(n);
                self.index.resolve_simple_type(n.text(self.src), &scope).map(Symbol::Type)
            }
            _ => None,
        }
    }

    /// The field a simple name `name` at `n` denotes: of an enclosing class
    /// (declared or inherited), else statically imported.
    fn field_named(&self, n: Node<'t>, name: &str) -> Option<Symbol<'t>> {
        // fields of enclosing classes; a superclass's private field is
        // not inherited (and hides the ones above it), so the name
        // belongs to an outer class if one has it
        let mut cur = self.enclosing_class_node(n);
        let mut not_inherited = None;
        while let Some(c) = cur {
            if let Some(cn) = self.class_names.get(&c.id()) {
                if let Some(ci) = self.class(cn) {
                    if let Some(f) = self.field(&ci.this_type(), name) {
                        if f.field.flags.is_private() && f.owner != *cn {
                            not_inherited.get_or_insert(f);
                        } else {
                            return Some(Symbol::Field(f));
                        }
                    }
                }
            }
            cur = self.enclosing_class_node(c);
        }
        if let Some(f) = not_inherited {
            return Some(Symbol::Field(f));
        }
        // static imports
        for imp in self.stub.imports.iter().filter(|i| i.is_static) {
            if imp.on_demand || imp.is_named(name) {
                let cls = if imp.on_demand { imp.path.to_string() } else { imp.path.rsplit_once('.').map(|x| x.0.to_string()).unwrap_or_default() };
                if let Some(b) = self.index.binary_name_for_dotted(&cls) {
                    if let Some(f) = self.field(&Ty::class(&b), name) {
                        return Some(Symbol::Field(f));
                    }
                }
            }
        }
        None
    }

    /// Is `n` the qualifier of a member access (`Objects` in
    /// `Objects.requireNonNull(x)`, `org` in `org.junit.Assert`)?
    fn is_qualifier(&self, n: Node<'t>) -> bool {
        self.parent_of(n).is_some_and(|p| match p.kind() {
            "method_invocation" | "field_access" => p.field("object") == Some(n),
            "scoped_identifier" | "scoped_type_identifier" => p.field("scope").or_else(|| p.kids().first().copied()) == Some(n),
            _ => false,
        })
    }

    pub(crate) fn local_member_type(&self, owner: &str, simple: &str) -> Option<Name> {
        let n = format!("{owner}${simple}");
        // a JDK class that is not public is there for hierarchy walks, not
        // for names (`Classpath::contains`)
        self.class(&n).filter(|c| c.is_source() || self.index.class_exists(&c.name)).map(|c| c.name.clone())
    }

    /// True if the node denotes a type name rather than a value.
    pub fn is_type_name(&self, n: Node<'t>) -> bool {
        matches!(self.symbol(n), Some(Symbol::Type(_)))
    }

    /// The field referenced by an identifier/field access, if any.
    pub fn field_ref(&self, n: Node<'t>) -> Option<FieldRef> {
        match self.symbol(n) {
            Some(Symbol::Field(f)) => Some(f),
            _ => None,
        }
    }

    /// The type a call's qualifier names (`Assert` in `Assert.assertEquals(..)`):
    /// resolved, or, when no visible class has the name, the class the code
    /// names by a single-type import (`import org.junit.Assert`) or writes
    /// qualified (`org.junit.Assert`). None when the qualifier is not a type
    /// name the code writes. An unresolved class named by a known library's
    /// qualified name is taken to be that library's class: the caller decides
    /// what to judge by it, and nothing is noted for the unresolved case.
    pub fn qualifier_type(&self, o: Node<'t>) -> Option<Named> {
        let (s, d) = doubt::observe(|| self.symbol(o));
        if s.is_some() {
            doubt::replay(d);
        }
        match s {
            Some(Symbol::Type(t)) => Some(Named::Resolved(t)),
            Some(_) => None,
            None => {
                let text = self.text(o);
                let named = if o.kind() == "identifier" {
                    self.stub.imports.iter().find(|i| !i.is_static && !i.on_demand && i.is_named(text)).map(|i| Name::from(&*i.path))
                } else if o.kind() == "field_access" && text.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_') && text.starts_with(|c: char| c.is_lowercase()) {
                    Some(Name::from(text))
                } else {
                    None
                };
                if named.is_none() {
                    doubt::replay(d);
                }
                named.map(Named::Unresolved)
            }
        }
    }

    /// The local declaration index referenced by an identifier, if any.
    pub fn local_ref(&self, n: Node<'t>) -> Option<usize> {
        if n.kind() == "identifier" {
            if self.shadow_unknown.contains(&n.id()) {
                doubt::note(Doubt::IncompleteHierarchy);
            }
            // a case label names a local only where it is an expression
            match self.simple_case_label(n) {
                Some(CaseLabel::Constant(_)) => return None,
                Some(CaseLabel::Unknown) => {
                    doubt::note(Doubt::UnresolvedName);
                    return None;
                }
                Some(CaseLabel::Expression) | None => {}
            }
            self.binder.decl_index_of_ref(n)
        } else {
            None
        }
    }

    // ----- target typing -----

    /// The type the context expects for `n` (assignment, return, argument,
    /// cast...). `Unknown` when there is no target type.
    pub fn expected_type(&self, n: Node<'t>) -> Ty {
        self.guards.guarded(n.id(), Query::Expected, Ty::Unknown, || self.compute_expected(n))
    }

    fn compute_expected(&self, n: Node<'t>) -> Ty {
        let mut child = n;
        let mut p = match self.parent_of(n) {
            Some(p) => p,
            None => return Ty::Unknown,
        };
        while p.kind() == "parenthesized_expression" {
            child = p;
            p = match self.parent_of(p) {
                Some(x) => x,
                None => return Ty::Unknown,
            };
        }
        match p.kind() {
            "variable_declarator" if p.field("value") == Some(child) => {
                if let Some(name) = p.field("name") {
                    if let Some(i) = self.binder.decl_index(name) {
                        let d = &self.binder.decls[i];
                        if d.type_node.is_some_and(|t| t.text(self.src) == "var") {
                            return Ty::Unknown;
                        }
                        return self.local_type(i);
                    }
                    // field
                    if let Some(decl) = self.parent_of(p) {
                        if let Some(t) = decl.field("type") {
                            let mut ty = self.resolve_type_node(t);
                            if let Some(d) = p.field("dimensions") {
                                for _ in 0..d.text(self.src).matches('[').count() {
                                    ty = Ty::Array(Box::new(ty));
                                }
                            }
                            return ty;
                        }
                    }
                }
                Ty::Unknown
            }
            "assignment_expression" if p.field("right") == Some(child) => p.field("left").map(|l| self.type_of(l)).unwrap_or(Ty::Unknown),
            "return_statement" => self.return_type_at(p),
            "lambda_expression" if p.field("body") == Some(child) => {
                let target = self.expected_type(p);
                self.functional_result(&target)
            }
            "argument_list" => {
                let Some(call) = self.parent_of(p) else { return Ty::Unknown };
                let idx = p.kids().iter().position(|k| *k == child).unwrap_or(0);
                self.param_type_at(call, idx)
            }
            "cast_expression" => {
                let tys = p.fields("type");
                if tys.len() == 1 {
                    self.resolve_type_node(tys[0])
                } else {
                    Ty::Unknown
                }
            }
            "ternary_expression" if p.field("condition") != Some(child) => self.expected_type(p),
            "array_initializer" => match self.expected_type(p) {
                Ty::Array(e) => *e,
                _ => Ty::Unknown,
            },
            "array_creation_expression" => self.type_of(p),
            "switch_rule" | "expression_statement" => {
                // arrow switch expression arm
                let sw = self.ancestors_of(p).find(|a| a.kind() == "switch_expression");
                match sw {
                    Some(sw) if p.kind() == "switch_rule" || self.parent_of(p).is_some_and(|pp| pp.kind() == "switch_rule") => self.expected_type(sw),
                    _ => Ty::Unknown,
                }
            }
            "yield_statement" => match self.ancestors_of(p).find(|a| a.kind() == "switch_expression") {
                Some(sw) => self.expected_type(sw),
                None => Ty::Unknown,
            },
            _ => Ty::Unknown,
        }
    }

    /// Declared return type of the callable enclosing a return statement.
    pub fn return_type_at(&self, at: Node<'t>) -> Ty {
        match self.enclosing_callable(at) {
            Some(c) if c.kind() == "lambda_expression" => {
                let target = self.expected_type(c);
                self.functional_result(&target)
            }
            Some(m) if m.kind() == "method_declaration" => {
                let mut t = m.field("type").map(|t| self.resolve_type_node(t)).unwrap_or(Ty::Unknown);
                if let Some(d) = m.field("dimensions") {
                    for _ in 0..d.text(self.src).matches('[').count() {
                        t = Ty::Array(Box::new(t));
                    }
                }
                t
            }
            Some(_) => Ty::Void,
            None => Ty::Unknown,
        }
    }

    // ----- constants -----

    /// Compile-time constant value of an expression, if any, converted to the
    /// expression's static type (`final long n = 1` is a long constant).
    pub fn constant_value(&self, n: Node<'t>) -> Option<Constant> {
        self.constants.get_or_compute(&self.guards, n.id(), n.id(), Query::Constant, || self.constant_value_uncached(n))
    }

    /// How often `constant_value` gave up at the nesting bound so far. Its
    /// answer was then "not a constant", which is not known to be right, and
    /// neither is anything computed from it since.
    pub fn constant_cut_offs(&self) -> u64 {
        self.guards.nesting_cut_offs()
    }

    fn constant_value_uncached(&self, n: Node<'t>) -> Option<Constant> {
        let v = self.constant_value_raw(n)?;
        Some(match self.type_of(n.unparen()) {
            t @ Ty::Prim(Prim::Int | Prim::Long | Prim::Short | Prim::Byte | Prim::Double | Prim::Float) if !matches!(v, Constant::Bool(_) | Constant::Str(_)) => {
                grounds_index::index::coerce_constant(v.clone(), &t).unwrap_or(v)
            }
            _ => v,
        })
    }

    fn constant_value_raw(&self, n: Node<'t>) -> Option<Constant> {
        let n = n.unparen();
        if let Some(c) = literal_constant(n, self.src) {
            return Some(c);
        }
        match n.kind() {
            "identifier" | "field_access" => match self.symbol(n)? {
                Symbol::Field(f) => {
                    if f.field.flags.is_final() {
                        f.field.constant.clone()
                    } else {
                        None
                    }
                }
                Symbol::Local(_, d) => {
                    // final locals with constant initializers
                    let writes = 0;
                    let _ = writes;
                    if d.kind == LocalKind::Local && d.is_final {
                        d.decl_node.field("value").and_then(|v| self.constant_value(v))
                    } else {
                        None
                    }
                }
                _ => None,
            },
            "binary_expression" => {
                let op = n.field("operator")?.kind();
                let l = self.constant_value(n.field("left")?)?;
                let r = self.constant_value(n.field("right")?)?;
                fold_binary(op, &l, &r)
            }
            "unary_expression" => {
                let op = n.field("operator")?.kind();
                let v = self.constant_value(n.field("operand")?)?;
                fold_unary(op, v)
            }
            "ternary_expression" => {
                let c = self.constant_value(n.field("condition")?)?.as_bool()?;
                self.constant_value(n.field(if c { "consequence" } else { "alternative" })?)
            }
            "cast_expression" => {
                let v = self.constant_value(n.field("value")?)?;
                match self.type_of(n) {
                    Ty::Prim(Prim::Int) => Some(Constant::Int(match v {
                        Constant::Float(f) => f as i32 as i64,
                        c => c.as_i64()? as i32 as i64,
                    })),
                    Ty::Prim(Prim::Long) => Some(Constant::Long(match v {
                        Constant::Float(f) => f as i64,
                        c => c.as_i64()?,
                    })),
                    Ty::Prim(Prim::Double) | Ty::Prim(Prim::Float) => Some(Constant::Float(v.as_f64()?)),
                    Ty::Prim(Prim::Char) => Some(Constant::Char(char::from_u32(v.as_i64()? as u32)?)),
                    Ty::Prim(Prim::Byte) => Some(Constant::Int(v.as_i64()? as i8 as i64)),
                    Ty::Prim(Prim::Short) => Some(Constant::Int(v.as_i64()? as i16 as i64)),
                    Ty::Class { ref name, .. } if &**name == "java.lang.String" => Some(v),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// String value of a constant string expression.
    pub fn string_value(&self, n: Node<'t>) -> Option<String> {
        match self.constant_value(n)? {
            Constant::Str(s) => Some(s.to_string()),
            _ => None,
        }
    }

    pub fn int_value(&self, n: Node<'t>) -> Option<i64> {
        match self.constant_value(n)? {
            Constant::Int(i) | Constant::Long(i) => Some(i),
            Constant::Char(c) => Some(c as i64),
            _ => None,
        }
    }

}
