//! Standard analyses built on the dataflow framework. All of them track
//! local variables only (by binder declaration index), which keeps them
//! precise: fields can change behind our back, locals cannot.

use crate::cfg::{Cfg, EdgeKind};
use crate::dataflow::Analysis;
use grounds_index::*;
use grounds_semantic::{FileModel, LocalKind, MethodMatcher, Symbol};
use grounds_syntax::{Node, NodeExt};
use rustc_hash::{FxHashMap, FxHashSet};

/// Locals captured by a lambda or anonymous class body (declared outside it).
pub fn captured_locals<'t>(fm: &FileModel<'t>, n: Node<'t>) -> Vec<usize> {
    fm.captured_locals(n).to_vec()
}

/// Local written by an element, with whether it is a plain overwrite.
pub fn local_def<'t>(fm: &FileModel<'t>, e: Node<'t>) -> Option<(usize, bool)> {
    match e.kind() {
        "assignment_expression" => {
            let l = e.field("left")?;
            if l.kind() != "identifier" {
                return None;
            }
            let i = fm.binder.decl_index_of_ref(l)?;
            let op = e.field("operator").map(|o| o.kind()).unwrap_or("=");
            Some((i, op == "="))
        }
        "update_expression" => {
            let k = e.first_kid()?.unparen();
            if k.kind() != "identifier" {
                return None;
            }
            Some((fm.binder.decl_index_of_ref(k)?, false))
        }
        "variable_declarator" => {
            let n = e.field("name")?;
            Some((fm.binder.decl_index(n)?, true))
        }
        "enhanced_for_statement" | "catch_formal_parameter" | "resource" => {
            let n = e.field("name")?;
            Some((fm.binder.decl_index(n)?, true))
        }
        _ => None,
    }
}

/// Is this identifier element a read of a local? (assignment targets are not
/// emitted as elements by the CFG builder.)
pub fn local_read<'t>(fm: &FileModel<'t>, e: Node<'t>) -> Option<usize> {
    if e.kind() != "identifier" {
        return None;
    }
    fm.binder.decl_index_of_ref(e)
}

// ---------------------------------------------------------------- liveness

pub struct Liveness<'a, 't> {
    pub fm: &'a FileModel<'t>,
}

impl<'a, 't> Analysis<'t> for Liveness<'a, 't> {
    type Fact = FxHashSet<usize>;
    const FORWARD: bool = false;
    fn bottom(&self) -> Self::Fact {
        FxHashSet::default()
    }
    fn boundary(&self) -> Self::Fact {
        FxHashSet::default()
    }
    fn join(&self, a: &Self::Fact, b: &Self::Fact) -> Self::Fact {
        if a.len() >= b.len() {
            let mut r = a.clone();
            r.extend(b.iter().copied());
            r
        } else {
            let mut r = b.clone();
            r.extend(a.iter().copied());
            r
        }
    }
    fn transfer(&self, e: Node<'t>, live: &mut Self::Fact) {
        if let Some((i, overwrite)) = local_def(self.fm, e) {
            if overwrite {
                live.remove(&i);
            } else {
                live.insert(i);
            }
            return;
        }
        if let Some(i) = local_read(self.fm, e) {
            live.insert(i);
            return;
        }
        if matches!(e.kind(), "lambda_expression" | "class_body") {
            for i in captured_locals(self.fm, e) {
                live.insert(i);
            }
        }
    }
}

// ---------------------------------------------------------------- nullness

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nullness {
    /// Definitely null on every path.
    Null,
    NonNull,
    /// Null on at least one path (or annotated @Nullable).
    Nullable,
    /// Definitely null because a null literal was assigned.
    AssignedNull,
    /// Null on some path only because it was initialized/assigned null and
    /// conditionally reassigned. Reporting these needs path correlation, so
    /// rules treat it as unknown.
    MaybeAssignedNull,
}

impl Nullness {
    pub fn is_definitely_null(self) -> bool {
        matches!(self, Nullness::Null | Nullness::AssignedNull)
    }
}

/// Join of two nullness states (`None` = unknown).
pub fn join_nullness(a: Option<Nullness>, b: Option<Nullness>) -> Option<Nullness> {
    use Nullness::*;
    match (a, b) {
        (Some(x), Some(y)) if x == y => Some(x),
        (Some(Null), Some(AssignedNull)) | (Some(AssignedNull), Some(Null)) => Some(Null),
        (Some(Null), _) | (_, Some(Null)) => Some(Nullable),
        (Some(Nullable), _) | (_, Some(Nullable)) => Some(Nullable),
        (Some(AssignedNull), _) | (_, Some(AssignedNull)) => Some(MaybeAssignedNull),
        (Some(MaybeAssignedNull), _) | (_, Some(MaybeAssignedNull)) => Some(MaybeAssignedNull),
        _ => None,
    }
}

pub type NullFact = FxHashMap<usize, Nullness>;

pub fn is_nullable_annotation(a: &AnnotationInfo) -> bool {
    matches!(a.simple_name(), "Nullable" | "CheckForNull")
}

pub struct NullnessAnalysis<'a, 't> {
    pub fm: &'a FileModel<'t>,
    pub cfg: &'a Cfg<'t>,
}

const OBJECTS_NULL_CHECKS: &[MethodMatcher] = &[MethodMatcher::statics("java.util.Objects").named(&["isNull", "nonNull"])];

const REQUIRE_NON_NULL: &[MethodMatcher] = &[
    MethodMatcher::statics("java.util.Objects").named(&["requireNonNull", "requireNonNullElse"]),
    MethodMatcher::statics("org.junit.jupiter.api.Assertions").named(&["assertNotNull"]),
    MethodMatcher::statics("org.junit.Assert").named(&["assertNotNull"]),
    MethodMatcher::statics("org.testng.Assert").named(&["assertNotNull"]),
    MethodMatcher::statics("junit.framework.Assert").named(&["assertNotNull"]),
    MethodMatcher::statics("com.google.common.base.Preconditions").named(&["checkNotNull"]),
    MethodMatcher::statics("org.springframework.util.Assert").named(&["notNull"]),
    MethodMatcher::statics("org.apache.commons.lang3.Validate").named(&["notNull"]),
];

impl<'a, 't> NullnessAnalysis<'a, 't> {
    /// Nullness of an expression given current facts.
    pub fn eval(&self, v: Node<'t>, f: &NullFact) -> Option<Nullness> {
        let v = v.unparen();
        match v.kind() {
            "null_literal" => Some(Nullness::AssignedNull),
            "object_creation_expression" | "array_creation_expression" | "string_literal" | "this" | "class_literal" | "array_initializer"
            | "lambda_expression" | "method_reference" | "template_expression" => Some(Nullness::NonNull),
            "binary_expression" if self.fm.type_of(v).is_string() => Some(Nullness::NonNull),
            "identifier" => {
                let i = self.fm.binder.decl_index_of_ref(v)?;
                f.get(&i).copied()
            }
            "cast_expression" => self.eval(v.field("value")?, f),
            "ternary_expression" => {
                // each arm under its edge of the condition: `m == null ? new HashMap<>() : m`
                let effects = v.field("condition").map(|c| self.cond_effect(c)).unwrap_or_default();
                let refined = |on_true: bool| {
                    let mut g = f.clone();
                    for (i, s, t) in &effects {
                        if *t == on_true {
                            g.insert(*i, *s);
                        }
                    }
                    g
                };
                let a = self.eval(v.field("consequence")?, &refined(true));
                let b = self.eval(v.field("alternative")?, &refined(false));
                match (a, b) {
                    (Some(x), Some(y)) if x == y => Some(x),
                    // `c ? null : v` is usually correlated with later checks of `c`
                    (Some(x), _) | (_, Some(x)) if x.is_definitely_null() => Some(Nullness::MaybeAssignedNull),
                    (Some(Nullness::Nullable), _) | (_, Some(Nullness::Nullable)) => Some(Nullness::Nullable),
                    _ => None,
                }
            }
            "method_invocation" => {
                let c = self.fm.select_method(v)?;
                // an uncertain overload choice may be the wrong method
                if c.ambiguous {
                    return None;
                }
                if c.method.annotations.iter().any(is_nullable_annotation) && !c.method.ret.is_array() {
                    return Some(Nullness::Nullable);
                }
                if self.fm.call_matches_any(v, REQUIRE_NON_NULL) {
                    return Some(Nullness::NonNull);
                }
                if c.ret.is_prim() {
                    return None;
                }
                None
            }
            k if crate::cfg::is_literal(k) => Some(Nullness::NonNull),
            _ => {
                if self.fm.type_of(v).is_prim() {
                    Some(Nullness::NonNull)
                } else {
                    None
                }
            }
        }
    }

    /// A call that returns only when its first (boolean) argument is true:
    /// Spring Assert.state/isTrue, Guava checkArgument/checkState, commons
    /// Validate.isTrue/validState, JUnit assertTrue, `@Contract("false -> fail")`,
    /// or a non-overridable project method starting with `if (!ok) throw`.
    fn throws_on_false(&self, call: Node<'t>) -> bool {
        let Some(c) = self.fm.select_method(call) else { return false };
        let name = &*c.method.name;
        let known = match &*c.owner {
            "org.springframework.util.Assert" => matches!(name, "state" | "isTrue"),
            "com.google.common.base.Preconditions" => matches!(name, "checkArgument" | "checkState"),
            "org.apache.commons.lang3.Validate" => matches!(name, "isTrue" | "validState"),
            o if o.starts_with("org.junit") || o.starts_with("junit.") => name == "assertTrue",
            "org.testng.Assert" => name == "assertTrue",
            _ => false,
        };
        let contract = c.method.annotations.iter().any(|a| {
            a.simple_name() == "Contract"
                && a.arg("value").is_some_and(|v| {
                    v.strings().iter().flat_map(|s| s.split(';').map(str::to_string).collect::<Vec<_>>()).any(|clause| {
                        let (lhs, rhs) = clause.rsplit_once("->").unwrap_or(("", &clause));
                        let mut parts = lhs.split(',').map(str::trim);
                        rhs.trim() == "fail" && parts.next() == Some("false") && parts.all(|p| p == "_")
                    })
                })
        });
        let mf = c.method.flags;
        let own = mf.has(Flags::THROWS_ON_FALSE) && (mf.has(Flags::STATIC) || mf.has(Flags::PRIVATE) || mf.has(Flags::FINAL) || self.fm.class(&c.owner).is_some_and(|o| o.flags.has(Flags::FINAL)));
        known || contract || own
    }

    /// Effectively-final locals initialized as a plain copy (or cast) of local `i`.
    fn copies_of(&self, i: usize) -> Vec<usize> {
        let fm = self.fm;
        fm.binder.uses[i]
            .iter()
            .filter_map(|u| {
                // u is the initializer `x` (or `(T) x`) of a declarator
                let mut top = *u;
                while let Some(p) = top.parent().filter(|p| matches!(p.kind(), "cast_expression" | "parenthesized_expression")) {
                    top = p;
                }
                let d = top.parent().filter(|p| p.kind() == "variable_declarator" && p.field("value") == Some(top))?;
                let (j, _) = local_def(fm, d)?;
                is_effectively_final(fm, j).then_some(j)
            })
            .collect()
    }

    /// Facts implied when a boolean expression is true (`a != null && b != null`).
    fn true_effects(&self, e: Node<'t>) -> Vec<(usize, Nullness)> {
        let e = e.unparen();
        if e.kind() == "binary_expression" && e.field("operator").is_some_and(|o| o.kind() == "&&") {
            let mut out = Vec::new();
            for side in [e.field("left"), e.field("right")].into_iter().flatten() {
                out.extend(self.true_effects(side));
            }
            return out;
        }
        self.cond_effect(e).into_iter().filter(|x| x.2).map(|(i, s, _)| (i, s)).collect()
    }

    /// For a condition, which local becomes null / non-null on the true edge.
    fn cond_effect(&self, c: Node<'t>) -> Vec<(usize, Nullness, bool)> {
        // returns (local, state, on_true)
        let c = c.unparen();
        let mut out = Vec::new();
        match c.kind() {
            "binary_expression" => {
                let op = c.field("operator").map(|o| o.kind()).unwrap_or("");
                if op != "==" && op != "!=" {
                    return out;
                }
                let (Some(l), Some(r)) = (c.field("left"), c.field("right")) else { return out };
                let (l, r) = (l.unparen(), r.unparen());
                let var = if r.kind() == "null_literal" {
                    l
                } else if l.kind() == "null_literal" {
                    r
                } else {
                    return out;
                };
                let Some(i) = (if var.kind() == "identifier" { self.fm.binder.decl_index_of_ref(var) } else { None }) else { return out };
                // `ref = (T) node; if (node == null) return;`: copies of an
                // effectively-final local share its nullness
                let mut vars = vec![i];
                if is_effectively_final(self.fm, i) {
                    vars.extend(self.copies_of(i));
                }
                for v in vars {
                    if op == "==" {
                        out.push((v, Nullness::Null, true));
                        out.push((v, Nullness::NonNull, false));
                    } else {
                        out.push((v, Nullness::NonNull, true));
                        out.push((v, Nullness::Null, false));
                    }
                }
            }
            // `boolean both = a != null && b != null; if (both) ...`
            "identifier" => {
                let Some(i) = self.fm.binder.decl_index_of_ref(c) else { return out };
                let d = &self.fm.binder.decls[i];
                if d.kind != LocalKind::Local || !is_effectively_final(self.fm, i) {
                    return out;
                }
                let Some(init) = d.decl_node.field("value").map(|v| v.unparen()) else { return out };
                if init.kind() == "identifier" {
                    return out; // no alias chains
                }
                // the true edge implies every conjunct, the false edge every
                // disjunct's negation: `boolean empty = q == null || q.isEmpty(); if (!empty) q.peek()`
                for truth in [true, false] {
                    out.extend(self.implied_direct(init, truth).into_iter().map(|(j, s)| (j, s, truth)));
                }
                // the checked locals must still hold the checked values
                out.retain(|(j, _, _)| is_effectively_final(self.fm, *j));
            }
            "instanceof_expression" => {
                if let Some(l) = c.field("left") {
                    let l = l.unparen();
                    if l.kind() == "identifier" {
                        if let Some(i) = self.fm.binder.decl_index_of_ref(l) {
                            out.push((i, Nullness::NonNull, true));
                        }
                    }
                }
            }
            "method_invocation" => {
                let name = self.fm.call_name(c).unwrap_or("");
                let args = self.fm.args_of(c);
                let first_local = args.first().map(|a| a.unparen()).filter(|a| a.kind() == "identifier").and_then(|a| self.fm.binder.decl_index_of_ref(a));
                match name {
                    "isNull" | "nonNull" => {
                        if let Some(i) = first_local {
                            let is_null = name == "isNull";
                            // non-null on the "not null" outcome is safe for any
                            // predicate; null on the other only with Objects' contract
                            out.push((i, Nullness::NonNull, !is_null));
                            if self.fm.call_matches_any(c, OBJECTS_NULL_CHECKS) {
                                out.push((i, Nullness::Null, is_null));
                            }
                        }
                    }
                    "isEmpty" | "isBlank" | "isNullOrEmpty" | "hasText" | "hasLength" | "isNotEmpty" | "isNotBlank" if !args.is_empty() => {
                        // StringUtils.isEmpty(x): false => non-null; hasText(x): true => non-null
                        if let Some(i) = first_local {
                            let nonnull_on_true = matches!(name, "hasText" | "hasLength" | "isNotEmpty" | "isNotBlank");
                            out.push((i, Nullness::NonNull, nonnull_on_true));
                        }
                    }
                    "equals" | "equalsIgnoreCase" | "contains" | "startsWith" | "endsWith" => {
                        // "lit".equals(x) true => x non-null
                        if let Some(i) = first_local {
                            out.push((i, Nullness::NonNull, true));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        out
    }

    /// Facts implied when a boolean expression has outcome `truth`, through
    /// `!`, `&&` (true) and `||` (false).
    fn implied(&self, e: Node<'t>, truth: bool) -> Vec<(usize, Nullness)> {
        let e = e.unparen();
        let op = e.field("operator").map(|o| o.kind());
        match e.kind() {
            "unary_expression" if op == Some("!") => e.field("operand").map(|o| self.implied(o, !truth)).unwrap_or_default(),
            "binary_expression" if (op == Some("&&") && truth) || (op == Some("||") && !truth) => {
                [e.field("left"), e.field("right")].into_iter().flatten().flat_map(|s| self.implied(s, truth)).collect()
            }
            _ => self.cond_effect(e).into_iter().filter(|x| x.2 == truth).map(|(i, s, _)| (i, s)).collect(),
        }
    }

    /// `implied` without following boolean locals (no alias chains).
    fn implied_direct(&self, e: Node<'t>, truth: bool) -> Vec<(usize, Nullness)> {
        let e = e.unparen();
        let op = e.field("operator").map(|o| o.kind());
        match e.kind() {
            "identifier" => Vec::new(),
            "unary_expression" if op == Some("!") => e.field("operand").map(|o| self.implied_direct(o, !truth)).unwrap_or_default(),
            "binary_expression" if (op == Some("&&") && truth) || (op == Some("||") && !truth) => {
                [e.field("left"), e.field("right")].into_iter().flatten().flat_map(|s| self.implied_direct(s, truth)).collect()
            }
            _ => self.cond_effect(e).into_iter().filter(|x| x.2 == truth).map(|(i, s, _)| (i, s)).collect(),
        }
    }

    /// Literal a reassigned local was initialized with (`String name = "";`).
    fn literal_init(&self, i: usize) -> Option<Node<'t>> {
        let d = &self.fm.binder.decls[i];
        if d.kind != LocalKind::Local {
            return None;
        }
        let v = d.decl_node.field("value")?.unparen();
        (crate::cfg::is_literal(v.kind()) && v.kind() != "template_expression").then_some(v)
    }

    /// Local proven to no longer hold its literal initializer when condition
    /// `c` has outcome `truth`: `!name.isEmpty()` after `name = ""`, `name != null`
    /// after `name = null`, `found` after `found = false`, `!"".equals(name)`.
    fn leaves_init(&self, c: Node<'t>, truth: bool, effects: &[(usize, Nullness, bool)]) -> Option<usize> {
        let fm = self.fm;
        // any non-null proof leaves a null initializer: `Objects.nonNull(name)`
        let null_init = |i: usize| self.literal_init(i).is_some_and(|v| v.kind() == "null_literal");
        if let Some(&(i, ..)) = effects.iter().find(|&&(i, s, t)| t == truth && s == Nullness::NonNull && null_init(i)) {
            return Some(i);
        }
        let local = |n: Option<Node<'t>>| n.map(|n| n.unparen()).filter(|n| n.kind() == "identifier").and_then(|n| fm.binder.decl_index_of_ref(n));
        let same = |init: Node<'t>, n: Node<'t>| init.kind() == n.kind() && fm.text(init) == fm.text(n);
        match c.kind() {
            "identifier" => {
                let i = local(Some(c))?;
                let init = self.literal_init(i)?.kind();
                ((init == "false" && truth) || (init == "true" && !truth)).then_some(i)
            }
            "binary_expression" => {
                let op = c.field("operator").map(|o| o.kind())?;
                if !matches!(op, "==" | "!=") || (op == "!=") != truth {
                    return None;
                }
                let (l, r) = (c.field("left")?.unparen(), c.field("right")?.unparen());
                let (i, lit) = match (local(Some(l)), local(Some(r))) {
                    (Some(i), None) => (i, r),
                    (None, Some(i)) => (i, l),
                    _ => return None,
                };
                same(self.literal_init(i)?, lit).then_some(i)
            }
            "method_invocation" => {
                let name = fm.call_name(c)?;
                let args = fm.args_of(c);
                if args.is_empty() {
                    // `name.isEmpty()` is false
                    let i = local(c.field("object"))?;
                    let init = self.literal_init(i)?;
                    return (matches!(name, "isEmpty" | "isBlank") && !truth && fm.text(init) == "\"\"").then_some(i);
                }
                if matches!(name, "equals" | "equalsIgnoreCase") && args.len() == 1 && !truth {
                    // `"".equals(name)` / `name.equals("")` is false
                    let (o, a) = (c.field("object")?.unparen(), args[0].unparen());
                    let (i, lit) = match (local(Some(o)), local(Some(a))) {
                        (Some(i), None) => (i, a),
                        (None, Some(i)) => (i, o),
                        _ => return None,
                    };
                    return same(self.literal_init(i)?, lit).then_some(i);
                }
                // `StringUtils.isNotEmpty(name)` after `name = ""`
                let i = local(args.first().copied())?;
                let init = self.literal_init(i)?;
                let empty = match name {
                    "isEmpty" | "isBlank" | "isNullOrEmpty" => !truth,
                    "isNotEmpty" | "isNotBlank" | "hasText" | "hasLength" => truth,
                    _ => false,
                };
                let empty_init = fm.text(init) == "\"\"" || init.kind() == "null_literal";
                (empty && empty_init).then_some(i)
            }
            _ => None,
        }
    }

    /// Locals non-null whenever local `i` no longer holds its literal
    /// initializer, because every write to `i` sits in a branch that checked
    /// them: `name = ""; if (dto != null) name = dto.getName();`. Only locals
    /// never reassigned, and not redeclared while `i` lives (a loop body
    /// declaring `dto` around an outer `name` would see the previous `dto`).
    fn nonnull_when_written(&self, i: usize) -> Vec<usize> {
        let fm = self.fm;
        let d = &fm.binder.decls[i];
        let mut guarded: Option<Vec<usize>> = None;
        for u in &fm.binder.uses[i] {
            let Some(w) = u.parent_unparen() else { continue };
            let write = match w.kind() {
                "assignment_expression" => w.field("left").map(|l| l.unparen()) == Some(*u),
                "update_expression" => true,
                _ => false,
            };
            if !write {
                continue;
            }
            let mut here = Vec::new();
            let mut child = w;
            while let Some(a) = child.parent() {
                if a.kind() == "if_statement" {
                    if let Some(c) = a.field("condition") {
                        if a.field("consequence") == Some(child) {
                            here.extend(self.implied(c, true).into_iter().filter(|x| x.1 == Nullness::NonNull).map(|x| x.0));
                        } else if a.field("alternative") == Some(child) {
                            here.extend(self.implied(c, false).into_iter().filter(|x| x.1 == Nullness::NonNull).map(|x| x.0));
                        }
                    }
                }
                if Some(a) == d.owner || grounds_syntax::is_method_like(a.kind()) || a.kind() == "lambda_expression" {
                    break;
                }
                child = a;
            }
            here.retain(|j| guarded.as_ref().is_none_or(|g| g.contains(j)));
            if here.is_empty() {
                return here;
            }
            guarded = Some(here);
        }
        let mut out = guarded.unwrap_or_default();
        out.retain(|&j| {
            let dj = &fm.binder.decls[j];
            let scope = match dj.kind {
                LocalKind::Param | LocalKind::LambdaParam => dj.owner,
                LocalKind::Local => dj.decl_node.parent().and_then(|p| p.parent()),
                LocalKind::ForEach => Some(dj.decl_node),
                _ => None,
            };
            j != i && is_effectively_final(fm, j) && scope.is_some_and(|s| d.decl_node.ancestors().any(|a| a == s))
        });
        out
    }

    /// Locals dereferenced by an element (receiver of a call, field access...).
    pub fn derefs(&self, e: Node<'t>) -> Vec<(usize, Node<'t>)> {
        let mut out = Vec::new();
        let mut push = |n: Option<Node<'t>>| {
            if let Some(n) = n {
                let n = n.unparen();
                if n.kind() == "identifier" {
                    if let Some(i) = self.fm.binder.decl_index_of_ref(n) {
                        out.push((i, n));
                    }
                }
            }
        };
        match e.kind() {
            // `s.valueOf(1)` with a null `s` is fine: static members only
            // evaluate the receiver
            "method_invocation" => {
                if !self.fm.select_method(e).is_some_and(|c| c.method.flags.has(Flags::STATIC)) {
                    push(e.field("object"))
                }
            }
            "field_access" => {
                let is_static = matches!(self.fm.symbol(e), Some(Symbol::Field(f)) if f.field.flags.has(Flags::STATIC));
                if e.field("field").is_some_and(|f| f.kind() != "this") && !is_static {
                    push(e.field("object"))
                }
            }
            "array_access" => push(e.field("array")),
            "enhanced_for_statement" => push(e.field("value")),
            "synchronized_statement" => push(e.kid_of_kind("parenthesized_expression").and_then(|p| p.first_kid())),
            "throw_statement" => push(e.first_kid()),
            _ => {}
        }
        out
    }
}

impl<'a, 't> Analysis<'t> for NullnessAnalysis<'a, 't> {
    type Fact = NullFact;
    fn bottom(&self) -> NullFact {
        FxHashMap::default()
    }
    fn boundary(&self) -> NullFact {
        let mut f = FxHashMap::default();
        // @Nullable parameters
        let owner = self.cfg.owner;
        if let Some(params) = owner.field("parameters") {
            for p in params.kids() {
                if p.kind() != "formal_parameter" {
                    continue;
                }
                let anns = self.fm.annotations_of(p);
                let is_array = p.field("type").is_some_and(|t| t.kind() == "array_type") || p.field("dimensions").is_some();
                if anns.iter().any(is_nullable_annotation) && !is_array {
                    if let Some(n) = p.field("name") {
                        if let Some(i) = self.fm.binder.decl_index(n) {
                            f.insert(i, Nullness::Nullable);
                        }
                    }
                }
            }
        }
        f
    }
    fn join(&self, a: &NullFact, b: &NullFact) -> NullFact {
        let mut out = FxHashMap::default();
        let keys: FxHashSet<usize> = a.keys().chain(b.keys()).copied().collect();
        for k in keys {
            let v = join_nullness(a.get(&k).copied(), b.get(&k).copied());
            if let Some(v) = v {
                out.insert(k, v);
            }
        }
        out
    }
    fn transfer(&self, e: Node<'t>, f: &mut NullFact) {
        // dereferences make the variable non-null afterwards
        for (i, _) in self.derefs(e) {
            f.insert(i, Nullness::NonNull);
        }
        match e.kind() {
            "variable_declarator" => {
                if let Some((i, _)) = local_def(self.fm, e) {
                    match e.field("value").and_then(|v| self.eval(v, f)) {
                        Some(s) => {
                            f.insert(i, s);
                        }
                        None => {
                            f.remove(&i);
                        }
                    }
                }
            }
            "assignment_expression" => {
                if let Some((i, plain)) = local_def(self.fm, e) {
                    let s = if plain { e.field("right").and_then(|v| self.eval(v, f)) } else { Some(Nullness::NonNull) };
                    match s {
                        Some(s) => {
                            f.insert(i, s);
                        }
                        None => {
                            f.remove(&i);
                        }
                    }
                }
            }
            "enhanced_for_statement" | "resource" => {
                if let Some((i, _)) = local_def(self.fm, e) {
                    f.remove(&i);
                }
            }
            "catch_formal_parameter" => {
                if let Some((i, _)) = local_def(self.fm, e) {
                    f.insert(i, Nullness::NonNull);
                }
            }
            "method_invocation" => {
                // throw-on-false checks used as statements: `Assert.state(x != null, ..)`,
                // `checkArgument(x != null)`, `assertTrue(x != null)`, `checkArguments(ok, ..)`
                let statement = e.parent().is_some_and(|p| p.kind() == "expression_statement");
                if statement && self.throws_on_false(e) {
                    let args = self.fm.args_of(e);
                    if let Some(a) = args.first().copied().filter(|a| self.fm.type_of(*a).unboxed() == Some(Prim::Boolean)) {
                        // `checkArgument(v != null, v = null)`: a local changed while
                        // evaluating the arguments no longer holds the checked value
                        let changed: Vec<usize> = args
                            .iter()
                            .flat_map(|x| x.descendants().chain(std::iter::once(*x)))
                            .filter_map(|d| match d.kind() {
                                "assignment_expression" => d.field("left").map(|l| l.unparen()),
                                "update_expression" => d.first_kid().map(|l| l.unparen()),
                                _ => None,
                            })
                            .filter_map(|l| if l.kind() == "identifier" { self.fm.binder.decl_index_of_ref(l) } else { None })
                            .collect();
                        for (i, s) in self.true_effects(a) {
                            if !changed.contains(&i) {
                                f.insert(i, s);
                            }
                        }
                    }
                }
                // AssertJ: `assertThat(x).isNotNull()` aborts when x is null
                if matches!(self.fm.call_name(e), Some("isNotNull" | "isNotEmpty" | "isNotBlank" | "isInstanceOf")) {
                    let subject = assert_that_root(self.fm, e.field("object"));
                    if let Some(a) = subject.and_then(|o| self.fm.args_of(o).first().map(|a| a.unparen())).filter(|a| a.kind() == "identifier") {
                        if let Some(i) = self.fm.binder.decl_index_of_ref(a) {
                            f.insert(i, Nullness::NonNull);
                        }
                    }
                }
                if self.fm.call_matches_any(e, REQUIRE_NON_NULL) {
                    // the checked value is the first argument (`requireNonNull(x, msg)`)
                    if let Some(a) = self.fm.args_of(e).first().map(|a| a.unparen()) {
                        if a.kind() == "identifier" {
                            if let Some(i) = self.fm.binder.decl_index_of_ref(a) {
                                f.insert(i, Nullness::NonNull);
                            }
                        }
                    }
                }
            }
            "instanceof_expression" => {}
            _ => {}
        }
    }
    fn edge(&self, kind: &EdgeKind<'t>, fact: &NullFact) -> Option<NullFact> {
        let (c, truth) = match kind {
            EdgeKind::True(c) => (*c, true),
            EdgeKind::False(c) => (*c, false),
            EdgeKind::Exception(_) => return Some(fact.clone()),
            EdgeKind::Normal => return Some(fact.clone()),
        };
        // constant conditions: prune impossible edges
        if let Some(Constant::Bool(b)) = self.fm.constant_value(c) {
            if b != truth {
                return None;
            }
        }
        let mut f = fact.clone();
        let effects = self.cond_effect(c);
        for &(i, s, on_true) in &effects {
            // a check of a value known non-null proves nothing: that outcome
            // is dead (`dto == null ? null : dto.getId()` after `dto.getName()`)
            if on_true == truth && !(s == Nullness::Null && f.get(&i) == Some(&Nullness::NonNull)) {
                f.insert(i, s);
            }
        }
        // `name = ""; if (dto != null) name = dto.getName(); if (!name.isEmpty()) dto...`
        if let Some(i) = self.leaves_init(c, truth, &effects) {
            for j in self.nonnull_when_written(i) {
                f.insert(j, Nullness::NonNull);
            }
        }
        Some(f)
    }
}

// ---------------------------------------------------------------- resources

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResState {
    /// Open, created at the node with this id.
    Open(usize),
    Closed,
    Escaped,
}

pub type ResFact = FxHashMap<usize, ResState>;

const EXEMPT_RESOURCES: &[&str] = &[
    "java.io.ByteArrayInputStream",
    "java.io.ByteArrayOutputStream",
    "java.io.StringReader",
    "java.io.StringWriter",
    "java.io.CharArrayReader",
    "java.io.CharArrayWriter",
    "java.util.stream.Stream",
    "java.util.stream.IntStream",
    "java.util.stream.LongStream",
    "java.util.stream.DoubleStream",
    "java.util.concurrent.ExecutorService",
    "java.net.http.HttpClient",
    "java.util.Formatter",
];

const RESOURCE_TYPES: &[&str] = &[
    "java.io.Closeable",
    "java.sql.Connection",
    "java.sql.Statement",
    "java.sql.ResultSet",
    "java.nio.channels.Channel",
    "java.util.zip.ZipFile",
    "java.net.Socket",
    "java.net.ServerSocket",
];

const RESOURCE_FACTORIES: &[MethodMatcher] = &[
    MethodMatcher::statics("java.nio.file.Files").named(&[
        "newInputStream",
        "newOutputStream",
        "newBufferedReader",
        "newBufferedWriter",
        "lines",
        "list",
        "walk",
        "find",
        "newDirectoryStream",
        "newByteChannel",
    ]),
    MethodMatcher::statics("java.sql.DriverManager").named(&["getConnection"]),
    MethodMatcher::instance("java.sql.Connection").named(&["prepareStatement", "createStatement", "prepareCall"]),
    MethodMatcher::instance("java.sql.Statement").named(&["executeQuery"]),
    MethodMatcher::instance("javax.sql.DataSource").named(&["getConnection"]),
];

pub struct ResourceAnalysis<'a, 't> {
    pub fm: &'a FileModel<'t>,
}

impl<'a, 't> ResourceAnalysis<'a, 't> {
    /// Does evaluating `v` create a resource that the caller must close?
    pub fn creates_resource(&self, v: Node<'t>) -> bool {
        let v = v.unparen();
        match v.kind() {
            "object_creation_expression" => {
                if v.kid_of_kind("class_body").is_some() {
                    return false;
                }
                let t = self.fm.type_of(v);
                let Some(n) = t.class_name() else { return false };
                if EXEMPT_RESOURCES.iter().any(|e| self.fm.is_subclass(n, e).is_yes()) {
                    return false;
                }
                // wrappers over exempt or escaped streams are not new resources
                if is_wrapper_type(self.fm, n) {
                    let args = self.fm.args_of(v);
                    if args.iter().any(|a| self.arg_is_stream_like(*a)) {
                        return false;
                    }
                }
                // Scanner(System.in), PrintWriter(System.out)
                if self.fm.args_of(v).iter().any(|a| {
                    let t = self.fm.text(*a);
                    t == "System.in" || t == "System.out" || t == "System.err"
                }) {
                    return false;
                }
                RESOURCE_TYPES.iter().any(|r| self.fm.is_subclass(n, r).is_yes())
            }
            "method_invocation" => self.fm.call_matches_any(v, RESOURCE_FACTORIES) && !self.owned_by_resource(v, 3),
            _ => false,
        }
    }

    /// A JDBC statement or result set obtained (directly or through such
    /// objects) from a try-with-resources variable is closed with it.
    fn owned_by_resource(&self, call: Node<'t>, depth: u32) -> bool {
        if depth == 0 {
            return false;
        }
        let Some(o) = call.field("object").map(|o| o.unparen()) else { return false };
        if o.kind() == "method_invocation" {
            return self.owned_by_resource(o, depth - 1);
        }
        let Some(i) = (if o.kind() == "identifier" { self.fm.binder.decl_index_of_ref(o) } else { None }) else { return false };
        let d = &self.fm.binder.decls[i];
        match d.kind {
            LocalKind::Resource => true,
            LocalKind::Local => is_effectively_final(self.fm, i) && d.decl_node.field("value").map(|v| v.unparen()).is_some_and(|v| v.kind() == "method_invocation" && self.owned_by_resource(v, depth - 1)),
            _ => false,
        }
    }

    fn arg_is_stream_like(&self, a: Node<'t>) -> bool {
        let t = self.fm.type_of(a);
        match t.class_name() {
            Some(n) => {
                ["java.io.InputStream", "java.io.OutputStream", "java.io.Reader", "java.io.Writer", "java.nio.channels.Channel"]
                    .iter()
                    .any(|s| self.fm.is_subclass(n, s).is_yes())
            }
            None => false,
        }
    }

    fn escape_all_in(&self, n: Node<'t>, f: &mut ResFact) {
        let a = n.unparen();
        if a.kind() == "identifier" {
            if let Some(i) = self.fm.binder.decl_index_of_ref(a) {
                if f.contains_key(&i) {
                    f.insert(i, ResState::Escaped);
                }
            }
        }
    }
}

fn is_wrapper_type(fm: &FileModel, n: &str) -> bool {
    ["java.io.FilterInputStream", "java.io.FilterOutputStream", "java.io.BufferedReader", "java.io.BufferedWriter", "java.io.InputStreamReader", "java.io.OutputStreamWriter", "java.io.PrintWriter", "java.io.PrintStream", "java.io.ObjectInputStream", "java.io.ObjectOutputStream", "java.util.zip.ZipInputStream", "java.util.zip.GZIPInputStream", "java.util.Scanner", "java.io.LineNumberReader", "java.io.PushbackReader"]
        .iter()
        .any(|w| fm.is_subclass(n, w).is_yes())
}

impl<'a, 't> Analysis<'t> for ResourceAnalysis<'a, 't> {
    type Fact = ResFact;
    fn bottom(&self) -> ResFact {
        FxHashMap::default()
    }
    fn boundary(&self) -> ResFact {
        FxHashMap::default()
    }
    fn join(&self, a: &ResFact, b: &ResFact) -> ResFact {
        let mut out = a.clone();
        for (k, v) in b {
            match (out.get(k), v) {
                (None, v) => {
                    out.insert(*k, *v);
                }
                (Some(ResState::Open(_)), _) => {}
                (_, ResState::Open(x)) => {
                    out.insert(*k, ResState::Open(*x));
                }
                (Some(ResState::Escaped), _) | (_, ResState::Escaped) => {
                    out.insert(*k, ResState::Escaped);
                }
                _ => {}
            }
        }
        out
    }
    /// `x.close()` that throws: nothing more can be done with `x`, so the
    /// exception path does not leak it (`try { x.close(); } catch (...) {}`).
    fn edge(&self, kind: &EdgeKind<'t>, fact: &ResFact) -> Option<ResFact> {
        if let EdgeKind::Exception(Some(n)) = kind {
            let mut f = fact.clone();
            let mut closes = Vec::new();
            for d in std::iter::once(*n).chain(n.descendants()) {
                if d.kind() == "method_invocation" && self.fm.call_name(d) == Some("close") {
                    closes.push(d);
                }
            }
            for c in closes {
                if let Some(i) = c.field("object").map(|o| o.unparen()).filter(|o| o.kind() == "identifier").and_then(|o| self.fm.binder.decl_index_of_ref(o)) {
                    if f.contains_key(&i) {
                        f.insert(i, ResState::Closed);
                    }
                }
            }
            return Some(f);
        }
        // `if (x != null) x.close();`: on the null edge there is nothing to close
        let (c, truth) = match kind {
            EdgeKind::True(c) => (*c, true),
            EdgeKind::False(c) => (*c, false),
            _ => return Some(fact.clone()),
        };
        let c = c.unparen();
        if c.kind() == "binary_expression" {
            let op = c.field("operator").map(|o| o.kind()).unwrap_or("");
            if let (Some(l), Some(r)) = (c.field("left").map(|x| x.unparen()), c.field("right").map(|x| x.unparen())) {
                let var = if r.kind() == "null_literal" { Some(l) } else if l.kind() == "null_literal" { Some(r) } else { None };
                let null_edge = (op == "==" && truth) || (op == "!=" && !truth);
                if let Some(i) = var.filter(|v| v.kind() == "identifier").and_then(|v| self.fm.binder.decl_index_of_ref(v)) {
                    if null_edge && fact.contains_key(&i) {
                        let mut f = fact.clone();
                        f.insert(i, ResState::Closed);
                        return Some(f);
                    }
                }
            }
        }
        Some(fact.clone())
    }
    fn transfer(&self, e: Node<'t>, f: &mut ResFact) {
        match e.kind() {
            "variable_declarator" | "assignment_expression" => {
                let value = if e.kind() == "variable_declarator" { e.field("value") } else { e.field("right") };
                // storing into a field / array escapes the value
                if e.kind() == "assignment_expression" {
                    let l = e.field("left");
                    if l.is_some_and(|l| l.kind() != "identifier") || l.is_some_and(|l| self.fm.binder.decl_index_of_ref(l).is_none()) {
                        if let Some(v) = value {
                            self.escape_all_in(v, f);
                        }
                        return;
                    }
                }
                if let Some((i, _)) = local_def(self.fm, e) {
                    if let Some(v) = value {
                        // aliasing: y = x escapes x
                        self.escape_all_in(v, f);
                        if self.creates_resource(v) {
                            f.insert(i, ResState::Open(v.unparen().id()));
                        } else if f.contains_key(&i) {
                            f.insert(i, ResState::Escaped);
                        }
                    }
                }
            }
            "method_invocation" => {
                // x.close()
                if self.fm.call_name(e) == Some("close") {
                    if let Some(o) = e.field("object") {
                        let o = o.unparen();
                        if o.kind() == "identifier" {
                            if let Some(i) = self.fm.binder.decl_index_of_ref(o) {
                                if f.contains_key(&i) {
                                    f.insert(i, ResState::Closed);
                                }
                            }
                        }
                    }
                }
                for a in self.fm.args_of(e) {
                    self.escape_all_in(a, f);
                }
            }
            "object_creation_expression" | "explicit_constructor_invocation" => {
                for a in self.fm.args_of(e) {
                    self.escape_all_in(a, f);
                }
            }
            "return_statement" | "yield_statement" | "throw_statement" => {
                if let Some(v) = e.first_kid() {
                    self.escape_all_in(v, f);
                }
            }
            "array_initializer" => {
                for k in e.kids() {
                    self.escape_all_in(k, f);
                }
            }
            "ternary_expression" => {
                for k in ["consequence", "alternative"] {
                    if let Some(v) = e.field(k) {
                        self.escape_all_in(v, f);
                    }
                }
            }
            "lambda_expression" | "class_body" => {
                for i in captured_locals(self.fm, e) {
                    if f.contains_key(&i) {
                        f.insert(i, ResState::Escaped);
                    }
                }
            }
            "method_reference" => {
                if let Some(t) = e.first_kid() {
                    self.escape_all_in(t, f);
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- optional

/// Tracks Optional locals that are known to be present.
pub struct OptionalPresence<'a, 't> {
    pub fm: &'a FileModel<'t>,
}

/// Optional locals known present, plus what a boolean local's value says
/// about them: after `if (expected && o.isEmpty()) throw ..;` the paths
/// where `expected` is false and those where `o` is present merge into
/// "`expected` implies `o` present".
#[derive(Clone, PartialEq, Default, Debug)]
pub struct PresentFact {
    present: FxHashSet<usize>,
    /// boolean locals with a known value on every path here
    flags: FxHashMap<usize, bool>,
    /// (flag, value, optional): the optional is present when the flag has that value
    implied: FxHashSet<(usize, bool, usize)>,
}

impl PresentFact {
    pub fn contains(&self, i: &usize) -> bool {
        self.present.contains(i)
    }

    fn holds(&self, &(flag, value, o): &(usize, bool, usize)) -> bool {
        self.present.contains(&o) || self.flags.get(&flag) == Some(&!value) || self.implied.contains(&(flag, value, o))
    }

    fn kill(&mut self, i: usize) {
        self.present.remove(&i);
        self.flags.remove(&i);
        self.implied.retain(|&(f, _, o)| f != i && o != i);
    }
}

impl<'a, 't> OptionalPresence<'a, 't> {
    pub fn optional_local(&self, n: Option<Node<'t>>) -> Option<usize> {
        let n = n?.unparen();
        if n.kind() != "identifier" {
            return None;
        }
        let i = self.fm.binder.decl_index_of_ref(n)?;
        let t = self.fm.local_type(i);
        if matches!(t.class_name(), Some("java.util.Optional" | "java.util.OptionalInt" | "java.util.OptionalLong" | "java.util.OptionalDouble")) {
            Some(i)
        } else {
            None
        }
    }
}

impl<'a, 't> OptionalPresence<'a, 't> {
    /// Optional locals known present when boolean `e` has outcome `truth`:
    /// `o.isPresent()`, `!o.isEmpty()`, through `!`, `&&` (true), `||`
    /// (false), and an effectively-final boolean local holding such a test
    /// (`boolean has = o.isPresent(); if (has) ...`).
    pub fn present_when(&self, e: Node<'t>, truth: bool) -> Vec<usize> {
        self.present_when_in(e, truth, true)
    }

    fn present_when_in(&self, e: Node<'t>, truth: bool, follow_alias: bool) -> Vec<usize> {
        let e = e.unparen();
        let op = e.field("operator").map(|o| o.kind());
        match e.kind() {
            "unary_expression" if op == Some("!") => e.field("operand").map(|o| self.present_when_in(o, !truth, follow_alias)).unwrap_or_default(),
            "binary_expression" if (op == Some("&&") && truth) || (op == Some("||") && !truth) => {
                [e.field("left"), e.field("right")].into_iter().flatten().flat_map(|s| self.present_when_in(s, truth, follow_alias)).collect()
            }
            "method_invocation" => {
                let args = self.fm.args_of(e);
                let proves = match (self.fm.call_name(e), &args[..]) {
                    (Some("isPresent"), []) => truth,
                    (Some("isEmpty"), []) => !truth,
                    // `o.map(f).orElse(false)` is true only when `o` holds a value
                    (Some("orElse"), [a]) => truth && a.unparen().kind() == "false",
                    _ => false,
                };
                if proves { self.chain_root(e.field("object")).into_iter().collect() } else { Vec::new() }
            }
            // no alias chains; the Optional must still hold the tested value
            "identifier" if follow_alias => {
                let Some(i) = self.fm.binder.decl_index_of_ref(e) else { return Vec::new() };
                let d = &self.fm.binder.decls[i];
                if d.kind != LocalKind::Local || !is_effectively_final(self.fm, i) {
                    return Vec::new();
                }
                let Some(init) = d.decl_node.field("value") else { return Vec::new() };
                let mut out = self.present_when_in(init, truth, false);
                out.retain(|j| is_effectively_final(self.fm, *j));
                out
            }
            _ => Vec::new(),
        }
    }

    /// The Optional local a `map`/`filter`/`flatMap` chain starts from: each
    /// step is empty when its receiver is, so `o.filter(p).isPresent()` proves `o`.
    fn chain_root(&self, n: Option<Node<'t>>) -> Option<usize> {
        let n = n?.unparen();
        if n.kind() == "method_invocation" && matches!(self.fm.call_name(n), Some("map" | "filter" | "flatMap")) && self.fm.args_of(n).len() == 1 {
            return self.chain_root(n.field("object"));
        }
        self.optional_local(Some(n))
    }

    /// The boolean local a condition is (`flag`), as the CFG splits `&&`,
    /// `||` and `!` into separate edges.
    fn flag_local(&self, c: Node<'t>) -> Option<usize> {
        let c = c.unparen();
        if c.kind() != "identifier" {
            return None;
        }
        let i = self.fm.binder.decl_index_of_ref(c)?;
        matches!(self.fm.binder.decls[i].kind, LocalKind::Local | LocalKind::Param).then_some(i)
    }

    /// The boolean a call requires to return, by name: Guava
    /// `checkArgument`/`checkState`, Spring `Assert.isTrue`/`state`, commons
    /// `Validate.isTrue`/`validState`, `assertTrue`/`assertFalse`, AssertJ
    /// `assertThat(b).isTrue()`, Hamcrest `assertThat(b, is(true))`.
    fn required_condition(&self, call: Node<'t>) -> Option<(Node<'t>, bool)> {
        let fm = self.fm;
        let args = fm.args_of(call);
        let name = fm.call_name(call)?;
        match name {
            "checkArgument" | "checkState" | "state" | "validState" | "assertTrue" | "assertFalse" | "isTrue" | "isFalse" if !args.is_empty() => {
                let want = !matches!(name, "assertFalse" | "isFalse");
                // JUnit 4 and TestNG: `assertTrue(message, condition)` / `assertTrue(condition, message)`
                let cond = args.iter().copied().find(|a| !matches!(a.unparen().kind(), "string_literal" | "binary_expression" if fm.type_of(*a).is_string()))?;
                Some((cond, want))
            }
            "isTrue" | "isFalse" => {
                let root = assert_that_root(fm, call.field("object"))?;
                Some((fm.args_of(root).first().copied()?, name == "isTrue"))
            }
            "assertThat" if args.len() == 2 => {
                // `is(true)`, `equalTo(false)`, `is(equalTo(true))`
                let mut m = args[1].unparen();
                while m.kind() == "method_invocation" && matches!(fm.call_name(m), Some("is" | "equalTo")) && fm.args_of(m).len() == 1 {
                    m = fm.args_of(m)[0].unparen();
                }
                match m.kind() {
                    "true" => Some((args[0], true)),
                    "false" => Some((args[0], false)),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

impl<'a, 't> Analysis<'t> for OptionalPresence<'a, 't> {
    type Fact = PresentFact;
    fn bottom(&self) -> PresentFact {
        PresentFact::default()
    }
    fn boundary(&self) -> PresentFact {
        PresentFact::default()
    }
    fn join(&self, a: &PresentFact, b: &PresentFact) -> PresentFact {
        let present: FxHashSet<usize> = a.present.intersection(&b.present).copied().collect();
        let flags = a.flags.iter().filter(|(k, v)| b.flags.get(k) == Some(v)).map(|(k, v)| (*k, *v)).collect();
        // an implication holds on each side: kept, or newly formed where one
        // side knows the flag and the other the optional (`!flag || present`)
        let formed = |x: &PresentFact, y: &PresentFact| -> Vec<(usize, bool, usize)> {
            x.flags.iter().flat_map(|(&f, &v)| y.present.iter().map(move |&o| (f, !v, o))).collect()
        };
        let implied = a.implied.iter().chain(&b.implied).copied().chain(formed(a, b)).chain(formed(b, a))
            .filter(|t| !present.contains(&t.2) && a.holds(t) && b.holds(t))
            .collect();
        PresentFact { present, flags, implied }
    }
    fn transfer(&self, e: Node<'t>, fact: &mut PresentFact) {
        let def = local_def(self.fm, e);
        if let Some((i, _)) = def {
            fact.kill(i);
        }
        let f = &mut fact.present;
        if let Some((i, _)) = def {
            // Optional.of(...) is always present
            let v = if e.kind() == "variable_declarator" { e.field("value") } else { e.field("right") };
            if let Some(v) = v {
                let v = v.unparen();
                if v.kind() == "method_invocation" && self.fm.call_name(v) == Some("of") && self.fm.text(v).starts_with("Optional.of(") {
                    f.insert(i);
                }
            }
        }
        if e.kind() == "method_invocation" {
            if let Some(name) = self.fm.call_name(e) {
                if matches!(name, "get" | "orElseThrow" | "getAsInt" | "getAsLong" | "getAsDouble") {
                    if let Some(i) = self.optional_local(e.field("object")) {
                        f.insert(i);
                    }
                }
                // assertions abort the test when empty: assertThat(opt).isPresent(),
                // assertTrue(opt.isPresent()), assertFalse(opt.isEmpty())
                if matches!(name, "isPresent" | "isNotEmpty" | "hasValue" | "contains" | "hasValueSatisfying") {
                    let subject = assert_that_root(self.fm, e.field("object"));
                    if let Some(i) = subject.and_then(|o| self.optional_local(self.fm.args_of(o).first().copied())) {
                        f.insert(i);
                    }
                }
                // calls that return only when a condition holds: `assertTrue(o.isPresent())`,
                // `checkArgument(o.isPresent(), ..)`, `assertThat(o.isPresent(), is(true))`
                if let Some((cond, want)) = self.required_condition(e) {
                    f.extend(self.present_when(cond, want));
                }
            }
        }
    }
    fn edge(&self, kind: &EdgeKind<'t>, fact: &PresentFact) -> Option<PresentFact> {
        let (c, truth) = match kind {
            EdgeKind::True(c) => (*c, true),
            EdgeKind::False(c) => (*c, false),
            _ => return Some(fact.clone()),
        };
        let flag = self.flag_local(c);
        // the flag's value is known on every path here: the other edge is never taken
        if flag.is_some_and(|g| fact.flags.get(&g) == Some(&!truth)) {
            return None;
        }
        let mut f = fact.clone();
        f.present.extend(self.present_when(c, truth));
        if let Some(flag) = flag {
            f.flags.insert(flag, truth);
            f.present.extend(fact.implied.iter().filter(|&&(g, v, _)| g == flag && v == truth).map(|t| t.2));
        }
        Some(f)
    }
}

/// `assertThat(x)` / `Assertions.assertThat(x)`, which throws on failure;
/// not `softly.assertThat(x)` (SoftAssertions collect failures and go on).
/// The `assertThat(...)` call at the root of an AssertJ chain, skipping
/// description calls: `assertThat(x).as("...").isNotNull()`.
pub fn assert_that_root<'t>(fm: &FileModel<'t>, receiver: Option<Node<'t>>) -> Option<Node<'t>> {
    let mut cur = receiver?.unparen();
    while cur.kind() == "method_invocation" && matches!(fm.call_name(cur), Some("as" | "describedAs" | "withFailMessage" | "overridingErrorMessage")) {
        cur = cur.field("object")?.unparen();
    }
    is_static_assert_that(fm, cur).then_some(cur)
}

pub fn is_static_assert_that<'t>(fm: &FileModel<'t>, call: Node<'t>) -> bool {
    // a class qualifier (`Truth.assertThat`, possibly unresolved), not a
    // `softly` instance
    let class_like = |o: Node<'t>| fm.is_type_name(o) || fm.text(o).rsplit('.').next().is_some_and(|s| s.starts_with(|c: char| c.is_ascii_uppercase()));
    call.kind() == "method_invocation" && fm.call_name(call) == Some("assertThat") && call.field("object").is_none_or(class_like)
}

/// Is this local a parameter or effectively-final variable never reassigned?
pub fn is_effectively_final(fm: &FileModel, decl: usize) -> bool {
    let d = &fm.binder.decls[decl];
    if d.is_final {
        return true;
    }
    let assigned_after_decl = fm.binder.uses[decl].iter().any(|u| {
        let p = u.parent_unparen();
        match p {
            Some(p) if p.kind() == "assignment_expression" => p.field("left").map(|l| l.unparen()) == Some(*u),
            Some(p) if p.kind() == "update_expression" => true,
            _ => false,
        }
    });
    let initialized = match d.kind {
        LocalKind::Local => d.decl_node.field("value").is_some(),
        _ => true,
    };
    !assigned_after_decl && initialized
}

/// Symbol helper for rules working with flow facts.
pub fn local_of<'t>(fm: &FileModel<'t>, n: Node<'t>) -> Option<usize> {
    match fm.symbol(n.unparen()) {
        Some(Symbol::Local(i, _)) => Some(i),
        _ => None,
    }
}
