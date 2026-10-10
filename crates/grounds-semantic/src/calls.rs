//! Method and constructor resolution: candidate lookup, overload selection
//! (strict / loose / varargs phases, most-specific) and lightweight type
//! argument inference from arguments, lambdas and target types.

use crate::model::{FileModel, ResolvedCall, Symbol};
use grounds_index::hierarchy::{apply_subst, substitution};
use grounds_index::*;
use grounds_syntax::{Node, NodeExt};
use std::rc::Rc;
use std::sync::Arc;

const TAG_SELECT: u8 = 10;
const TAG_RESOLVE: u8 = 11;
const TAG_PARTIAL: u8 = 12;

/// Shape of an argument for overload resolution.
#[derive(Clone, Debug)]
pub enum ArgShape {
    Ty(Ty),
    /// arity, and whether the body fits a void / a value-returning
    /// functional method (JLS 15.27.2)
    Lambda(usize, LambdaCompat),
    MethodRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LambdaCompat {
    pub void_ok: bool,
    pub value_ok: bool,
}

/// Void/value compatibility of a lambda body. Expression bodies are value
/// compatible (a void method call cannot be told apart here, so it counts as
/// both) and void compatible when they are statement expressions; block
/// bodies are value compatible when they return a value or cannot complete
/// normally, void compatible when no `return` carries a value.
fn lambda_compat(body: Node) -> LambdaCompat {
    if body.kind() != "block" {
        let statement = matches!(body.unparen().kind(), "method_invocation" | "object_creation_expression" | "assignment_expression" | "update_expression");
        return LambdaCompat { void_ok: statement, value_ok: true };
    }
    let mut value_return = false;
    let mut void_return = false;
    let mut stack = vec![body];
    while let Some(n) = stack.pop() {
        for k in n.kids() {
            match k.kind() {
                // returns in nested lambdas and classes belong to them
                "lambda_expression" | "class_body" => {}
                "return_statement" => {
                    if k.first_kid().is_some() {
                        value_return = true;
                    } else {
                        void_return = true;
                    }
                }
                _ => stack.push(k),
            }
        }
    }
    let ends_abruptly = body.kids().last().is_some_and(|l| l.kind() == "throw_statement");
    LambdaCompat { void_ok: !value_return, value_ok: (value_return && !void_return) || (ends_abruptly && !void_return) }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Fit {
    Strict,
    Loose,
    Maybe,
    No,
}

impl<'t> FileModel<'t> {
    pub fn args_of(&self, call: Node<'t>) -> Vec<Node<'t>> {
        call.field("arguments").map(|a| a.kids()).unwrap_or_default()
    }

    pub fn arg_shape(&self, arg: Node<'t>) -> ArgShape {
        let a = arg.unparen();
        match a.kind() {
            "lambda_expression" => {
                let n = match a.field("parameters") {
                    Some(p) if p.kind() == "identifier" => 1,
                    Some(p) => p.kids().iter().filter(|k| matches!(k.kind(), "identifier" | "formal_parameter" | "spread_parameter")).count(),
                    None => 0,
                };
                let compat = a.field("body").map(lambda_compat).unwrap_or(LambdaCompat { void_ok: true, value_ok: true });
                ArgShape::Lambda(n, compat)
            }
            "method_reference" => ArgShape::MethodRef,
            _ => ArgShape::Ty(self.type_of(a)),
        }
    }

    /// Candidate methods for a method invocation and the receiver type.
    fn candidates(&self, call: Node<'t>) -> (Vec<MethodRef>, Option<Ty>, bool) {
        let Some(name_node) = call.field("name") else { return (Vec::new(), None, false) };
        let name = name_node.text(self.src);
        match call.field("object") {
            None => {
                // implicit this / outer / static imports
                let mut cur = self.enclosing_class_node(call);
                while let Some(c) = cur {
                    if let Some(cn) = self.class_names.get(&c.id()) {
                        if let Some(ci) = self.class(cn) {
                            let ms = self.methods_named(&ci.this_type(), name);
                            if !ms.is_empty() {
                                return (ms, Some(ci.this_type()), false);
                            }
                        }
                    }
                    cur = self.enclosing_class_node(c);
                }
                let mut out = Vec::new();
                for imp in self.stub.imports.iter().filter(|i| i.is_static) {
                    if imp.on_demand || imp.last() == name {
                        let cls = if imp.on_demand { imp.path.to_string() } else { imp.path.rsplit_once('.').map(|x| x.0.to_string()).unwrap_or_default() };
                        if let Some(b) = self.index.binary_name_for_dotted(&cls) {
                            out.extend(self.methods_named(&Ty::class(&b), name).into_iter().filter(|m| m.is_static()));
                        }
                    }
                }
                (out, None, true)
            }
            Some(obj) => {
                if obj.kind() == "super" {
                    let Some(c) = self.enclosing_class(call) else { return (Vec::new(), None, false) };
                    let mut out = Vec::new();
                    if let Some(st) = &c.superclass {
                        out = self.methods_named(st, name);
                    }
                    if out.is_empty() {
                        for i in &c.interfaces {
                            out.extend(self.methods_named(i, name));
                        }
                    }
                    return (out, c.superclass.clone(), false);
                }
                // `Iface.super.m()`: instance methods of the named supertype
                if call.kid_of_kind("super").is_some() {
                    if let Some(Symbol::Type(t)) = self.symbol(obj) {
                        let ms = self.methods_named(&Ty::class(&t), name);
                        let recv = self.enclosing_class(call).map(|c| c.this_type());
                        return (ms, recv, false);
                    }
                }
                if matches!(obj.kind(), "identifier" | "field_access" | "scoped_identifier") {
                    if let Some(Symbol::Type(t)) = self.symbol(obj) {
                        let ms = self.methods_named(&Ty::class(&t), name);
                        let statics: Vec<MethodRef> = ms.iter().filter(|m| m.is_static()).cloned().collect();
                        return (if statics.is_empty() { ms } else { statics }, None, true);
                    }
                }
                let rt = self.type_of(obj);
                if let Ty::TypeVar(ref v) = rt {
                    for b in self.type_var_bounds(v, call) {
                        let ms = self.methods_named(&b, name);
                        if !ms.is_empty() {
                            return (ms, Some(b), false);
                        }
                    }
                }
                let rt = match rt {
                    Ty::TypeVar(ref v) => self.type_var_bound(v, call),
                    Ty::Prim(_) | Ty::Unknown | Ty::Void | Ty::Null => return (Vec::new(), None, false),
                    t => t,
                };
                if let Ty::Array(_) = rt {
                    let mut ms = self.methods_named(&Ty::object(), name);
                    if name == "clone" {
                        for m in &mut ms {
                            m.method.ret = rt.clone();
                        }
                    }
                    return (ms, Some(rt), false);
                }
                (self.methods_named(&rt, name), Some(rt), false)
            }
        }
    }

    fn fit(&self, param: &Ty, arg: &ArgShape) -> Fit {
        match arg {
            ArgShape::Lambda(n, compat) => {
                if param.mentions_type_var() && param.class_name().is_none() {
                    return Fit::Maybe;
                }
                match param.class_name().and_then(|c| self.class(c)) {
                    Some(c) if c.is_interface() => match self.functional_method(param) {
                        Some(m) if m.method.params.len() == *n => {
                            let void = matches!(m.method.ret, Ty::Void);
                            if (void && !compat.void_ok) || (!void && !compat.value_ok) {
                                Fit::No
                            } else {
                                Fit::Strict
                            }
                        }
                        Some(_) => Fit::No,
                        None => Fit::No,
                    },
                    // a lambda never converts to a class type, Object included
                    Some(_) => Fit::No,
                    None => Fit::Maybe,
                }
            }
            ArgShape::MethodRef => match param.class_name().and_then(|c| self.class(c)) {
                Some(c) if c.is_interface() => {
                    if self.functional_method(param).is_some() {
                        Fit::Maybe
                    } else {
                        Fit::No
                    }
                }
                Some(_) => Fit::No,
                None => Fit::Maybe,
            },
            ArgShape::Ty(a) => {
                if param.mentions_type_var() {
                    // generic parameter: structural / erasure-level check
                    return match (param, a) {
                        (_, Ty::Unknown) => Fit::Maybe,
                        (Ty::Class { name: pn, .. }, Ty::Class { name: an, .. }) => match self.is_subclass(an, pn) {
                            Tri::Yes => Fit::Strict,
                            Tri::No => Fit::No,
                            Tri::Unknown => Fit::Maybe,
                        },
                        (Ty::Class { name: pn, .. }, Ty::Prim(p)) => match self.is_subclass(p.box_name(), pn) {
                            Tri::No => Fit::No,
                            _ => Fit::Loose,
                        },
                        (Ty::Class { name: pn, .. }, Ty::Array(_)) => {
                            if matches!(&**pn, "java.lang.Object" | "java.lang.Cloneable" | "java.io.Serializable") {
                                Fit::Strict
                            } else {
                                Fit::No
                            }
                        }
                        (Ty::Array(_), Ty::Null) => Fit::Strict,
                        (Ty::Array(_), t) if !t.is_array() => Fit::No,
                        (Ty::Array(pe), Ty::Array(ae)) => match (&**pe, &**ae) {
                            (Ty::TypeVar(_), Ty::Prim(_)) => Fit::No,
                            (Ty::Prim(x), Ty::Prim(y)) if x != y => Fit::No,
                            _ => Fit::Maybe,
                        },
                        (Ty::TypeVar(_), Ty::Prim(_)) => Fit::Loose,
                        _ => Fit::Maybe,
                    };
                }
                let strict = match (a, param) {
                    (Ty::Prim(_), Ty::Class { .. }) | (Ty::Class { .. }, Ty::Prim(_)) => false,
                    _ => true,
                };
                match self.is_assignable(a, param) {
                    Tri::Yes => {
                        if strict {
                            Fit::Strict
                        } else {
                            Fit::Loose
                        }
                    }
                    Tri::No => Fit::No,
                    Tri::Unknown => Fit::Maybe,
                }
            }
        }
    }

    /// Overall fit of a candidate for the given args. Returns (phase, worst fit).
    fn candidate_fit(&self, m: &MethodInfo, args: &[ArgShape]) -> Option<(u8, Fit)> {
        let n = args.len();
        let np = m.params.len();
        // phases 1/2: fixed arity
        if n == np {
            let mut worst = Fit::Strict;
            let mut ok = true;
            for (p, a) in m.params.iter().zip(args) {
                let f = self.fit(&p.ty, a);
                if f == Fit::No {
                    ok = false;
                    break;
                }
                worst = worst.max(f);
            }
            if ok {
                return Some((if worst == Fit::Loose { 2 } else { 1 }, worst));
            }
        }
        // phase 3: varargs
        if m.is_varargs() && n + 1 >= np {
            let mut worst = Fit::Strict;
            for (i, a) in args.iter().enumerate() {
                let pt = if i + 1 < np {
                    m.params[i].ty.clone()
                } else {
                    match &m.params[np - 1].ty {
                        Ty::Array(e) => (**e).clone(),
                        t => t.clone(),
                    }
                };
                let f = self.fit(&pt, a);
                if f == Fit::No {
                    return None;
                }
                worst = worst.max(f);
            }
            return Some((3, worst));
        }
        None
    }

    fn more_specific(&self, a: &MethodInfo, b: &MethodInfo, n: usize) -> bool {
        let pa = |m: &MethodInfo, i: usize| -> Ty {
            let np = m.params.len();
            if m.is_varargs() && i + 1 >= np && np > 0 {
                match &m.params[np - 1].ty {
                    Ty::Array(e) if n != np || i + 1 > np => (**e).clone(),
                    t => t.clone(),
                }
            } else {
                m.params.get(i).map(|p| p.ty.clone()).unwrap_or(Ty::Unknown)
            }
        };
        let count = n.max(a.params.len()).max(b.params.len());
        for i in 0..count {
            let x = pa(a, i).erasure();
            let y = pa(b, i).erasure();
            if x == y {
                continue;
            }
            match (x.prim(), y.prim()) {
                (Some(px), Some(py)) => {
                    if !px.widens_to(py) {
                        return false;
                    }
                }
                (Some(_), None) => {}
                (None, Some(_)) => return false,
                _ => {
                    if !self.is_assignable(&x, &y).is_yes() {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn select_among(&self, cands: Vec<MethodRef>, call: Node<'t>) -> Option<(MethodRef, bool)> {
        let arg_nodes = self.args_of(call);
        let n = arg_nodes.len();
        let cands: Vec<MethodRef> = cands.into_iter().filter(|m| m.method.accepts_arity(n)).collect();
        if cands.is_empty() {
            return None;
        }
        if cands.len() == 1 {
            let c = cands.into_iter().next().unwrap();
            return Some((c, false));
        }
        let args: Vec<ArgShape> = arg_nodes.into_iter().map(|a| self.arg_shape(a)).collect();
        let args = &args[..];
        let fits: Vec<(MethodRef, (u8, Fit))> = cands.into_iter().filter_map(|m| self.candidate_fit(&m.method, args).map(|f| (m, f))).collect();
        if fits.is_empty() {
            return None;
        }
        let best_phase = fits.iter().map(|(_, f)| f.0).min().unwrap();
        // If the winner only fits "maybe" (an argument type is unknown) while
        // another overload with a different result also fits, the choice is a
        // guess: flag it so callers do not trust the result type.
        let uncertain = {
            let winners: Vec<&(MethodRef, (u8, Fit))> = fits.iter().filter(|(_, f)| f.0 == best_phase).collect();
            let winner_maybe = winners.iter().any(|(_, f)| f.1 == Fit::Maybe);
            let rets: Vec<Ty> = fits.iter().map(|(m, _)| m.method.ret.erasure()).collect();
            winner_maybe && fits.len() > 1 && rets.iter().any(|r| *r != rets[0])
        };
        let mut pool: Vec<MethodRef> = fits.into_iter().filter(|(_, f)| f.0 == best_phase).map(|(m, _)| m).collect();
        if pool.len() == 1 {
            return Some((pool.pop().unwrap(), uncertain));
        }
        // most specific
        let maximal: Vec<usize> = (0..pool.len())
            .filter(|&i| (0..pool.len()).all(|j| i == j || self.more_specific(&pool[i].method, &pool[j].method, n)))
            .collect();
        if maximal.len() == 1 {
            return Some((pool.swap_remove(maximal[0]), uncertain));
        }
        // ambiguous: keep first, flag it
        let first = pool.remove(0);
        Some((first, true))
    }

    /// Overload selection only (no lambda-body inference). Receiver type
    /// arguments are substituted, method type variables are not.
    pub fn select_method(&self, call: Node<'t>) -> Option<Rc<ResolvedCall>> {
        if let Some(r) = self.selected.borrow().get(&call.id()) {
            return r.clone();
        }
        let (r, clean) = self.clean(|| self.guard(call.id(), TAG_SELECT, None, || {
            match call.kind() {
                "method_invocation" => {
                    let (cands, recv, via_type) = self.candidates(call);
                    let (m, ambiguous) = self.select_among(cands, call)?;
                    Some(Rc::new(ResolvedCall { owner: m.owner.clone(), ret: m.method.ret.clone(), method: m.method, receiver: recv, ambiguous, via_type }))
                }
                "object_creation_expression" | "explicit_constructor_invocation" => self.select_constructor(call),
                _ => None,
            }
        }));
        if clean {
            self.selected.borrow_mut().insert(call.id(), r.clone());
        }
        r
    }

    fn select_constructor(&self, call: Node<'t>) -> Option<Rc<ResolvedCall>> {
        let class_ty = if call.kind() == "object_creation_expression" {
            let t = call.field("type")?;
            let raw = self.resolve_type_node(t);
            // anonymous class: constructor of the super class
            raw
        } else {
            let c = self.enclosing_class(call)?;
            match call.field("constructor").map(|k| k.kind()) {
                Some("this") => c.this_type(),
                _ => c.superclass.clone()?,
            }
        };
        let cn = class_ty.class_name()?.to_string();
        let ci = self.class(&cn)?;
        if ci.is_interface() {
            return None;
        }
        let map = substitution(&ci, class_ty.args());
        let cands: Vec<MethodRef> = ci
            .constructors()
            .map(|m| {
                let mut m = m.clone();
                for p in &mut m.params {
                    p.ty = apply_subst(&p.ty, &map);
                }
                MethodRef { owner: ci.name.clone(), method: m, owner_kind: ci.kind, owner_flags: ci.flags }
            })
            .collect();
        let (m, ambiguous) = self.select_among(cands, call)?;
        Some(Rc::new(ResolvedCall { owner: m.owner.clone(), method: m.method, ret: class_ty, receiver: None, ambiguous, via_type: false }))
    }

    /// Full resolution with type inference.
    pub fn resolve_call(&self, call: Node<'t>) -> Option<Rc<ResolvedCall>> {
        if let Some(r) = self.calls.borrow().get(&call.id()) {
            return r.clone();
        }
        let (r, clean) = self.clean(|| self.guard(call.id(), TAG_RESOLVE, None, || {
            let sel = self.select_method(call)?;
            if sel.ambiguous && call.kind() == "method_invocation" {
                let mut rc = (*sel).clone();
                rc.ret = Ty::Unknown;
                return Some(Rc::new(rc));
            }
            if call.kind() != "method_invocation" {
                let mut rc = (*sel).clone();
                rc.ret = self.creation_type(call);
                return Some(Rc::new(rc));
            }
            let mut rc = (*sel).clone();
            // getClass() special case
            if rc.method.name.as_ref() == "getClass" && rc.method.params.is_empty() {
                if let Some(r) = &rc.receiver {
                    rc.ret = Ty::class_with("java.lang.Class", vec![Ty::Wildcard(Some(Box::new(WildcardBound::Extends(r.erasure()))))]);
                    return Some(Rc::new(rc));
                }
            }
            if rc.method.type_params.is_empty() {
                rc.ret = self.finish_type(&rc.method.ret, &[], call);
                return Some(Rc::new(rc));
            }
            let mut bindings = self.partial_bindings(call, &rc.method);
            let tvars: Vec<Name> = rc.method.type_params.iter().map(|t| t.name.clone()).collect();
            // lambdas and method references
            let args = self.args_of(call);
            for (i, a) in args.iter().enumerate() {
                let a = a.unparen();
                if !matches!(a.kind(), "lambda_expression" | "method_reference") {
                    continue;
                }
                let Some(pt) = param_type_for_arg(&rc.method, i, args.len()) else { continue };
                let pt = apply_subst(&pt, &bindings);
                let Some(sam) = self.functional_method(&pt) else { continue };
                if !sam.method.ret.mentions_type_var() {
                    continue;
                }
                let body_ty = if a.kind() == "lambda_expression" { self.lambda_result_type(a) } else { self.method_ref_result_type(a, &sam.method) };
                if body_ty.is_known() && body_ty != Ty::Void {
                    unify(self, &sam.method.ret, &body_ty, &tvars, &mut bindings);
                }
            }
            // target type
            let ret_unbound = tvars.iter().any(|v| !bindings.iter().any(|(k, _)| k == v) && mentions(&rc.method.ret, v));
            if ret_unbound {
                let exp = self.expected_type(call);
                if exp.is_known() {
                    unify(self, &rc.method.ret, &exp, &tvars, &mut bindings);
                }
            }
            for p in &mut rc.method.params {
                p.ty = apply_subst(&p.ty, &bindings);
            }
            rc.ret = self.finish_type(&rc.method.ret, &bindings, call);
            Some(Rc::new(rc))
        }));
        if clean {
            self.calls.borrow_mut().insert(call.id(), r.clone());
        }
        r
    }

    /// Substitute bindings; remaining method type variables become Unknown,
    /// class type variables of the enclosing context stay.
    fn finish_type(&self, t: &Ty, bindings: &[(Name, Ty)], at: Node<'t>) -> Ty {
        let t = apply_subst(t, bindings);
        let scope = self.scope_at(at);
        t.subst(&|v| if scope.type_vars.iter().any(|x| &**x == v) { None } else { Some(Ty::Unknown) })
    }

    /// Bindings for method type variables from non-lambda arguments.
    fn partial_bindings(&self, call: Node<'t>, m: &MethodInfo) -> Vec<(Name, Ty)> {
        self.guard(call.id(), TAG_PARTIAL, Vec::new(), || {
            let tvars: Vec<Name> = m.type_params.iter().map(|t| t.name.clone()).collect();
            let mut out = Vec::new();
            if tvars.is_empty() {
                return out;
            }
            let args = self.args_of(call);
            for (i, a) in args.iter().enumerate() {
                let a = a.unparen();
                if matches!(a.kind(), "lambda_expression" | "method_reference") {
                    continue;
                }
                let at = self.type_of(a);
                let Some(pt) = param_type_for_arg_typed(m, i, args.len(), &at) else { continue };
                if at.is_known() {
                    unify(self, &pt, &at, &tvars, &mut out);
                }
            }
            out
        })
    }

    /// Parameter type of the selected method at argument index (target type
    /// for that argument), with partial inference applied.
    pub fn param_type_at(&self, call: Node<'t>, idx: usize) -> Ty {
        let Some(sel) = self.select_method(call) else { return Ty::Unknown };
        let n = self.args_of(call).len();
        let Some(pt) = param_type_for_arg(&sel.method, idx, n) else { return Ty::Unknown };
        if sel.method.type_params.is_empty() {
            return pt;
        }
        let b = self.partial_bindings(call, &sel.method);
        apply_subst(&pt, &b)
    }

    /// Type produced by a lambda body (expression or first return value).
    pub fn lambda_result_type(&self, lambda: Node<'t>) -> Ty {
        let Some(body) = lambda.field("body") else { return Ty::Unknown };
        if body.kind() != "block" {
            return self.type_of(body);
        }
        let mut result = Ty::Void;
        let mut stack = vec![body];
        while let Some(n) = stack.pop() {
            for k in n.kids() {
                match k.kind() {
                    "return_statement" => {
                        if let Some(e) = k.first_kid() {
                            let t = self.type_of(e);
                            if t.is_known() && t != Ty::Null {
                                return t;
                            }
                            result = Ty::Unknown;
                        }
                    }
                    "lambda_expression" | "class_body" | "class_declaration" => {}
                    _ => stack.push(k),
                }
            }
        }
        result
    }

    fn method_ref_result_type(&self, mref: Node<'t>, sam: &MethodInfo) -> Ty {
        let kids = mref.kids();
        let Some(target) = kids.first().copied() else { return Ty::Unknown };
        let name = kids.last().map(|k| k.text(self.src)).unwrap_or("");
        let text = mref.text(self.src);
        if text.ends_with("::new") {
            let t = self.resolve_type_node(target);
            return t;
        }
        let (recv_ty, is_type) = match self.symbol(target) {
            Some(Symbol::Type(t)) => (self.class(&t).map(|c| c.this_type()).unwrap_or_else(|| Ty::class(&t)), true),
            _ => {
                if matches!(target.kind(), "type_identifier" | "generic_type" | "scoped_type_identifier") {
                    (self.resolve_type_node(target), true)
                } else {
                    (self.type_of(target), false)
                }
            }
        };
        if recv_ty.is_unknown() {
            return Ty::Unknown;
        }
        let ms = self.methods_named(&recv_ty, name);
        let n = sam.params.len();
        let pick = ms
            .iter()
            .find(|m| if is_type { (m.is_static() && m.method.params.len() == n) || (!m.is_static() && m.method.params.len() + 1 == n) } else { m.method.params.len() == n })
            .or_else(|| ms.first());
        match pick {
            Some(m) if !m.method.ret.mentions_type_var() => m.method.ret.clone(),
            Some(m) if is_type && !m.is_static() && n >= 1 => {
                // unbound receiver: substitute receiver args from first SAM param
                let first = sam.params[0].ty.upper_bound();
                if first.class_name().is_some() {
                    let ms2 = self.methods_named(&first, name);
                    if let Some(m2) = ms2.iter().find(|x| x.method.params.len() + 1 == n) {
                        if !m2.method.ret.mentions_type_var() {
                            return m2.method.ret.clone();
                        }
                    }
                }
                let _ = m;
                Ty::Unknown
            }
            _ => Ty::Unknown,
        }
    }

    /// Type of a `new` expression, inferring diamond type arguments from the
    /// target type.
    pub fn creation_type(&self, n: Node<'t>) -> Ty {
        if n.kind() != "object_creation_expression" {
            return Ty::Unknown;
        }
        // anonymous class -> its local class type
        if let Some(body) = n.kid_of_kind("class_body") {
            if let Some(name) = self.class_names.get(&body.id()) {
                return Ty::class(name);
            }
        }
        let Some(tn) = n.field("type") else { return Ty::Unknown };
        let t = self.resolve_type_node(tn);
        let is_diamond = tn.kind() == "generic_type" && tn.kid_of_kind("type_arguments").is_some_and(|ta| ta.kids().is_empty());
        if !is_diamond {
            return t;
        }
        let Some(cn) = t.class_name() else { return t };
        let Some(ci) = self.class(cn) else { return t };
        let exp = self.expected_type(n);
        if exp.is_known() {
            let tvars: Vec<Name> = ci.type_params.iter().map(|p| p.name.clone()).collect();
            let mut b = Vec::new();
            let this = ci.this_type();
            let sup = exp.class_name().and_then(|en| self.as_super(&this, en)).unwrap_or(this);
            unify(self, &sup, &exp, &tvars, &mut b);
            if b.len() == tvars.len() {
                return apply_subst(&ci.this_type(), &b);
            }
        }
        // infer from constructor args (e.g. new ArrayList<>(otherList))
        Ty::Class { name: ci.name.clone(), args: ci.type_params.iter().map(|_| Ty::Unknown).collect() }
    }

    /// The method declaration a call resolves to, if it is in source.
    pub fn callee_is_source(&self, call: Node<'t>) -> bool {
        self.select_method(call).is_some_and(|c| self.index.is_source_class(&c.owner))
    }
}

/// Like [`param_type_for_arg`] but passes an array argument directly to a
/// trailing varargs parameter.
pub fn param_type_for_arg_typed(m: &MethodInfo, i: usize, nargs: usize, arg: &Ty) -> Option<Ty> {
    let np = m.params.len();
    if m.is_varargs() && nargs == np && i + 1 == np && (arg.is_array() || *arg == Ty::Null) {
        return m.params.get(i).map(|p| p.ty.clone());
    }
    param_type_for_arg(m, i, nargs)
}

pub fn param_type_for_arg(m: &MethodInfo, i: usize, nargs: usize) -> Option<Ty> {
    let np = m.params.len();
    if np == 0 {
        return None;
    }
    if m.is_varargs() && i + 1 >= np {
        let last = &m.params[np - 1].ty;
        if nargs == np && i == np - 1 {
            // could be array passed directly; prefer element for lambdas
            return Some(match last {
                Ty::Array(e) => (**e).clone(),
                t => t.clone(),
            });
        }
        return Some(match last {
            Ty::Array(e) => (**e).clone(),
            t => t.clone(),
        });
    }
    m.params.get(i).map(|p| p.ty.clone())
}

fn mentions(t: &Ty, v: &str) -> bool {
    match t {
        Ty::TypeVar(n) => &**n == v,
        Ty::Class { args, .. } => args.iter().any(|a| mentions(a, v)),
        Ty::Array(e) => mentions(e, v),
        Ty::Wildcard(Some(b)) => match &**b {
            WildcardBound::Extends(t) | WildcardBound::Super(t) => mentions(t, v),
        },
        _ => false,
    }
}

/// Bind type variables in `param` against the actual type `arg`.
pub fn unify(lk: &dyn ClassLookup, param: &Ty, arg: &Ty, tvars: &[Name], out: &mut Vec<(Name, Ty)>) {
    match param {
        Ty::TypeVar(v) if tvars.contains(v) => {
            if out.iter().any(|(k, _)| k == v) {
                return;
            }
            let a = match arg {
                Ty::Prim(p) => Ty::class(p.box_name()),
                Ty::Null | Ty::Unknown | Ty::Void => return,
                Ty::Wildcard(Some(b)) => match &**b {
                    WildcardBound::Extends(t) | WildcardBound::Super(t) => t.clone(),
                },
                Ty::Wildcard(None) => return,
                t => t.clone(),
            };
            if a.has_unknown() {
                return;
            }
            out.push((v.clone(), a));
        }
        Ty::Wildcard(Some(b)) => match &**b {
            WildcardBound::Extends(t) | WildcardBound::Super(t) => unify(lk, t, &arg.upper_bound(), tvars, out),
        },
        Ty::Array(pe) => {
            if let Ty::Array(ae) = arg {
                unify(lk, pe, ae, tvars, out);
            }
        }
        Ty::Class { name, args } if !args.is_empty() => {
            let arg = match arg {
                Ty::Prim(p) => Ty::class(p.box_name()),
                t => t.clone(),
            };
            if arg.class_name().is_none() {
                return;
            }
            if let Some(sup) = lk.as_super(&arg, name) {
                for (p, a) in args.iter().zip(sup.args()) {
                    unify(lk, p, a, tvars, out);
                }
            }
        }
        _ => {}
    }
}

#[allow(dead_code)]
fn arc(s: &str) -> Name {
    Arc::from(s)
}
