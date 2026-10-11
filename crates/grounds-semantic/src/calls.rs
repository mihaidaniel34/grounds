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
// 13..=15: TAG_STANDALONE + Target
const TAG_STANDALONE: u8 = 13;

/// Where a generic method invocation takes its target type from.
#[derive(Clone, Copy)]
enum Target {
    Full = 0,
    Param = 1,
    None = 2,
}

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
                // returns in nested lambdas and local/anonymous types belong to them
                "lambda_expression" | "class_body" | "class_declaration" | "enum_declaration" | "interface_declaration" | "record_declaration" | "annotation_type_declaration" => {}
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
            _ => ArgShape::Ty(self.arg_type(a, Target::None)),
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

    /// `fit`, but a parameter that is one of the method's own type variables
    /// takes any reference argument within its bounds: `<T> assertThat(T)`
    /// fits a `String` as surely as `assertThat(String)` does.
    fn fit_in(&self, m: &MethodInfo, param: &Ty, arg: &ArgShape) -> Fit {
        // `<E> assertThat(E[])` with a `String[]`: the element decides. A
        // primitive element only matches the same primitive (`float[]` never
        // fits `T[][]`)
        if let (Ty::Array(pe), ArgShape::Ty(Ty::Array(ae))) = (param, arg) {
            if pe.mentions_type_var() {
                return match &**ae {
                    Ty::Prim(_) => Fit::No,
                    Ty::Class { .. } | Ty::Array(_) => self.fit_in(m, pe, &ArgShape::Ty((**ae).clone())),
                    _ => self.fit(param, arg),
                };
            }
        }
        if let Ty::TypeVar(v) = param {
            if let Some(tp) = m.type_params.iter().find(|tp| tp.name == *v) {
                // a primitive is boxed first (phase 2), then checked like any reference
                let (a, mut fit) = match arg {
                    ArgShape::Ty(Ty::Prim(p)) => (Ty::class(p.box_name()), Fit::Loose),
                    ArgShape::Ty(t @ (Ty::Class { .. } | Ty::Array(_))) => (t.clone(), Fit::Strict),
                    _ => return self.fit(param, arg),
                };
                // javac's specificity for an intersection bound needs inference
                if tp.bounds.len() > 1 {
                    fit = fit.max(Fit::Maybe);
                }
                for b in &tp.bounds {
                    let bn = b.erasure();
                    let bn = bn.class_name().unwrap_or("java.lang.Object");
                    let f = match &a {
                        Ty::Class { name, .. } => match self.is_subclass(name, bn) {
                            Tri::Yes => Fit::Strict,
                            Tri::No => Fit::No,
                            Tri::Unknown => Fit::Maybe,
                        },
                        _ if matches!(bn, "java.lang.Object" | "java.lang.Cloneable" | "java.io.Serializable") => Fit::Strict,
                        _ => Fit::No,
                    };
                    // the erasure can rule a candidate out, but a bound with type
                    // arguments or variables (`Comparable<? super T>`, `Enum<T>`,
                    // `T extends U`) needs inference to rule it in
                    let generic = b.mentions_type_var() || matches!(b, Ty::Class { args, .. } if !args.is_empty());
                    fit = fit.max(if f != Fit::No && generic { Fit::Maybe } else { f });
                }
                return fit;
            }
        }
        self.fit(param, arg)
    }

    /// Overall fit of a candidate for the given args: (phase, worst fit,
    /// worst fit without the type-variable bound checks of `fit_in`). The
    /// first rules candidates out; certainty about a generic winner still
    /// comes from the second, since only inference could establish more.
    fn candidate_fit(&self, m: &MethodInfo, args: &[ArgShape]) -> Option<(u8, Fit, Fit)> {
        let n = args.len();
        let np = m.params.len();
        // phases 1/2: fixed arity
        if n == np {
            let (mut worst, mut plain) = (Fit::Strict, Fit::Strict);
            let mut ok = true;
            for (p, a) in m.params.iter().zip(args) {
                let f = self.fit_in(m, &p.ty, a);
                if f == Fit::No {
                    ok = false;
                    break;
                }
                worst = worst.max(f);
                plain = plain.max(self.fit(&p.ty, a));
            }
            if ok {
                return Some((if worst == Fit::Loose { 2 } else { 1 }, worst, plain));
            }
        }
        // phase 3: varargs
        if m.is_varargs() && n + 1 >= np {
            let (mut worst, mut plain) = (Fit::Strict, Fit::Strict);
            for (i, a) in args.iter().enumerate() {
                let pt = if i + 1 < np {
                    m.params[i].ty.clone()
                } else {
                    match &m.params[np - 1].ty {
                        Ty::Array(e) => (**e).clone(),
                        t => t.clone(),
                    }
                };
                let f = self.fit_in(m, &pt, a);
                if f == Fit::No {
                    return None;
                }
                worst = worst.max(f);
                plain = plain.max(self.fit(&pt, a));
            }
            return Some((3, worst, plain));
        }
        None
    }

    /// `a` satisfies the bound `b` of type variable `t` once `t` is `a`
    /// (`String` for `T extends Comparable<? super T>`). False when unsure.
    fn bound_holds(&self, a: &Ty, b: &Ty, t: &str) -> bool {
        let b = b.subst(&|v| (v == t).then(|| a.clone()));
        let (Some(an), Some(bn)) = (a.class_name(), b.class_name()) else { return false };
        if b.mentions_type_var() {
            return false;
        }
        let bargs = b.args();
        if bargs.is_empty() {
            return self.is_subclass(an, bn).is_yes();
        }
        let Some(sup) = self.as_super(a, bn) else { return false };
        let sargs = sup.args();
        sargs.len() == bargs.len() && bargs.iter().zip(sargs).all(|(ba, sa)| contains(self, ba, sa))
    }

    /// Parameters of `m` with type arguments and no type variable of `m`
    /// (`List<String>`) get arguments of exactly that parameterization; `fit`
    /// compares those by erasure only.
    fn args_match_exactly(&self, m: &MethodInfo, args: &[ArgShape]) -> bool {
        let np = m.params.len();
        args.iter().enumerate().all(|(i, a)| {
            let pt = match m.params.get(i.min(np.saturating_sub(1))) {
                Some(p) if m.is_varargs() && i + 1 >= np && !(args.len() == np && matches!(a, ArgShape::Ty(Ty::Array(_)))) => match &p.ty {
                    Ty::Array(e) => (**e).clone(),
                    t => t.clone(),
                },
                Some(p) => p.ty.clone(),
                None => return false,
            };
            let vars = |t: &Ty| m.type_params.iter().any(|tp| mentions(t, &tp.name));
            match a {
                ArgShape::Ty(at) => self.exact_fit(&pt, at, &vars),
                // `fit` checks a lambda's arity and void-ness, not its types
                _ => false,
            }
        })
    }

    /// Argument type `at` fits parameter `pt` beyond erasure: fixed type
    /// arguments match exactly (also inside arrays), and a type variable of
    /// the method (`vars`) is bound only by a plain shape (`T`, `T[]`, `C<T>`,
    /// `C<? extends T>`) to an argument without wildcards, so taking the
    /// argument's type as its binding is what javac infers.
    fn exact_fit(&self, pt: &Ty, at: &Ty, vars: &dyn Fn(&Ty) -> bool) -> bool {
        let plain = |x: &Ty| !has_wildcard(x) && !x.has_unknown();
        match (pt, at) {
            (_, Ty::Null) => !vars(pt),
            (Ty::TypeVar(_), _) if vars(pt) => matches!(at, Ty::Class { .. } | Ty::Array(_)) && plain(at),
            (Ty::Array(pe), Ty::Array(ae)) => match &**pe {
                Ty::Prim(_) => pe == ae,
                _ => self.exact_fit(pe, ae, vars),
            },
            (Ty::Array(_), _) => false,
            (Ty::Class { name: pn, args: pargs }, _) if !pargs.is_empty() => {
                // `as_super` substitutes wildcards away: decide only on plain arguments
                if !plain(at) {
                    return false;
                }
                let Some(sup) = self.as_super(at, pn) else { return false };
                let sargs = sup.args();
                sargs.len() == pargs.len()
                    && pargs.iter().zip(sargs).all(|(pa, sa)| {
                        let var_form = match pa {
                            Ty::TypeVar(_) => vars(pa),
                            Ty::Wildcard(Some(w)) => match &**w {
                                WildcardBound::Extends(x @ Ty::TypeVar(_)) | WildcardBound::Super(x @ Ty::TypeVar(_)) => vars(x),
                                _ => false,
                            },
                            _ => false,
                        };
                        if var_form {
                            plain(sa)
                        } else {
                            !vars(pa) && contains(self, pa, sa)
                        }
                    })
            }
            _ => !vars(pt) || matches!(pt, Ty::TypeVar(_)),
        }
    }

    /// Non-generic `w` is more specific than generic `g` the way javac would
    /// decide it: every parameter of `w` is a valid argument for `g`, with
    /// `g`'s type variables (each in one parameter, a single bound) checked
    /// against their bounds. False when unsure.
    fn surely_more_specific(&self, w: &MethodInfo, g: &MethodInfo, n: usize) -> bool {
        let at = |m: &MethodInfo, i: usize| -> Option<Ty> {
            let np = m.params.len();
            if m.is_varargs() && i + 1 >= np && np > 0 {
                match &m.params[np - 1].ty {
                    Ty::Array(e) if n != np || i + 1 > np => Some((**e).clone()),
                    t => Some(t.clone()),
                }
            } else {
                m.params.get(i).map(|p| p.ty.clone())
            }
        };
        (0..n.max(w.params.len()).max(g.params.len())).all(|i| {
            let (Some(p), Some(q)) = (at(w, i), at(g, i)) else { return false };
            self.surely_param(&p, &q, g)
        })
    }

    /// Parameter type `p` of the winner is a valid argument for parameter
    /// `q` of generic `g`.
    fn surely_param(&self, p: &Ty, q: &Ty, g: &MethodInfo) -> bool {
        let g_vars = |t: &Ty| g.type_params.iter().any(|tp| mentions(t, &tp.name));
        match q {
            Ty::TypeVar(v) if g.type_params.iter().any(|tp| tp.name == *v) => {
                let tp = g.type_params.iter().find(|tp| tp.name == *v).unwrap();
                let once = g.params.iter().map(|x| occurrences(&x.ty, v)).sum::<usize>() == 1 && !g.type_params.iter().any(|o| o.bounds.iter().any(|b| o.name != *v && mentions(b, v)));
                once && tp.bounds.len() <= 1
                    && match p {
                        Ty::Class { .. } => tp.bounds.first().is_none_or(|b| self.bound_holds(p, b, v)),
                        // an unbounded variable takes an array too
                        Ty::Array(_) => free_once(g, v),
                        _ => false,
                    }
            }
            // `E[]` against `String[][]`: element against element
            Ty::Array(qe) if g_vars(q) => match p {
                Ty::Array(pe) if !pe.is_prim() => self.surely_param(pe, qe, g),
                _ => false,
            },
            // `Iterable<? extends E>` with `E` free: any `Iterable` fits;
            // `Pair<String, ? extends Number>`: each argument contains the winner's
            Ty::Class { name: qn, args } if !args.is_empty() => {
                let free = |a: &Ty| match a {
                    Ty::TypeVar(v) => free_once(g, v),
                    Ty::Wildcard(None) => true,
                    Ty::Wildcard(Some(w)) => match &**w {
                        WildcardBound::Extends(Ty::TypeVar(v)) | WildcardBound::Super(Ty::TypeVar(v)) => free_once(g, v),
                        WildcardBound::Extends(x) | WildcardBound::Super(x) => !g_vars(x),
                    },
                    x => !g_vars(x),
                };
                let Ty::Class { name: pn, args: pargs } = p else { return false };
                if pargs.is_empty() || !args.iter().all(|a| free(a)) || !self.is_subclass(pn, qn).is_yes() {
                    return false;
                }
                // which argument of `p` lands where in `q` (`List<#0>` is
                // `Iterable<#0>`); `as_super` on `p` itself would drop wildcards
                let decl = Ty::Class { name: pn.clone(), args: (0..pargs.len()).map(|i| Ty::TypeVar(Arc::from(format!("#{i}")))).collect() };
                // a raw supertype on the way (`extends Base`) needs unchecked conversion
                let supers = self.all_supertypes(&decl);
                if supers.iter().any(|t| t.args().is_empty() && t.class_name().and_then(|c| self.class(c)).is_some_and(|c| !c.type_params.is_empty())) {
                    return false;
                }
                let Some(sup) = supers.into_iter().find(|t| t.class_name() == Some(qn)) else { return false };
                let placeholder = |t: &Ty| -> Option<Option<usize>> {
                    match t {
                        Ty::TypeVar(n) if n.starts_with('#') => n[1..].parse().ok().map(Some),
                        // also a leftover variable from a raw supertype (`extends Base`)
                        t if t.mentions_type_var() => None,
                        _ => Some(None),
                    }
                };
                sup.args().len() == args.len()
                    && args.iter().zip(sup.args()).all(|(qa, ma)| {
                        let pa = match placeholder(ma) {
                            Some(Some(i)) => pargs[i].clone(),
                            Some(None) => ma.clone(),
                            None => return false,
                        };
                        let is = |t: &Ty, f: fn(&WildcardBound) -> bool| matches!(t, Ty::Wildcard(Some(w)) if f(w));
                        match qa {
                            Ty::Wildcard(None) => true,
                            // `List<?>` is no `List<V>` in javac's specificity
                            Ty::TypeVar(_) => !matches!(pa, Ty::Wildcard(_)),
                            Ty::Wildcard(Some(w)) if matches!(&**w, WildcardBound::Extends(Ty::TypeVar(_))) => !is(&pa, |w| matches!(w, WildcardBound::Super(_))),
                            Ty::Wildcard(Some(w)) if matches!(&**w, WildcardBound::Super(Ty::TypeVar(_))) => !matches!(pa, Ty::Wildcard(None)) && !is(&pa, |w| matches!(w, WildcardBound::Extends(_))),
                            fixed if matches!(pa, Ty::Wildcard(_)) => pa == *fixed,
                            fixed => contains(self, fixed, &pa),
                        }
                    })
            }
            q if !g_vars(q) => {
                // `List<String>[]` against `List<Integer>[]`: elements must agree
                let (mut pe, mut qe) = (p, q);
                while let (Ty::Array(a), Ty::Array(b)) = (pe, qe) {
                    (pe, qe) = (&**a, &**b);
                }
                (qe.args().is_empty() || pe == qe) && self.is_assignable(&p.erasure(), &q.erasure()).is_yes()
            }
            _ => false,
        }
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
        // a method type variable erases to its bound, also as an array element;
        // with an intersection bound the order is javac inference: unknown
        fn erase(m: &MethodInfo, t: &Ty) -> Option<Ty> {
            match t {
                Ty::TypeVar(v) => match m.type_params.iter().find(|tp| tp.name == *v) {
                    Some(tp) if tp.bounds.len() > 1 => None,
                    Some(tp) => Some(tp.bounds.first().map(|b| b.erasure()).unwrap_or_else(Ty::object)),
                    None => Some(t.erasure()),
                },
                Ty::Array(e) => erase(m, e).map(|e| Ty::Array(Box::new(e))),
                _ => Some(t.erasure()),
            }
        }
        let count = n.max(a.params.len()).max(b.params.len());
        for i in 0..count {
            let (Some(x), Some(y)) = (erase(a, &pa(a, i)), erase(b, &pa(b, i))) else { return false };
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
        // a guess matters only when the overloads differ in result
        let rets_differ = {
            let rets: Vec<Ty> = cands.iter().map(|m| m.method.ret.erasure()).collect();
            rets.iter().any(|r| *r != rets[0])
        };
        let args: Vec<ArgShape> = arg_nodes.into_iter().map(|a| self.arg_shape(a)).collect();
        let args = &args[..];
        let fits: Vec<(MethodRef, (u8, Fit, Fit))> = cands.into_iter().filter_map(|m| self.candidate_fit(&m.method, args).map(|f| (m, f))).collect();
        if fits.is_empty() {
            return None;
        }
        let best_phase = fits.iter().map(|(_, f)| f.0).min().unwrap();
        let mut pool: Vec<(MethodRef, Fit, Fit)> = fits.into_iter().filter(|(_, f)| f.0 == best_phase).map(|(m, f)| (m, f.1, f.2)).collect();
        // most specific
        let maximal: Vec<usize> = (0..pool.len())
            .filter(|&i| (0..pool.len()).all(|j| i == j || self.more_specific(&pool[i].0.method, &pool[j].0.method, n)))
            .collect();
        if maximal.len() != 1 {
            // ambiguous: keep first, flag it
            return Some((pool.swap_remove(0).0, true));
        }
        let w = maximal[0];
        let generic = |m: &MethodInfo| m.params.iter().any(|p| m.type_params.iter().any(|tp| mentions(&p.ty, &tp.name)));
        // The choice is a guess when an argument type is unknown, or when only
        // inference could rule out a generic competitor: flag it so callers do
        // not trust the result type. A generic winner is judged as before the
        // bound checks; a non-generic one must beat every generic competitor
        // for sure (`assertThat(String)` over `<T> assertThat(T)`).
        let wm = &pool[w].0.method;
        // explicit `T.<Double>pick(x)` replaces inference: not modelled here
        let explicit = call.child_by_field_name("type_arguments").is_some();
        let sure = !explicit
            && pool[w].1 != Fit::Maybe
            && wm.type_params.iter().all(|tp| !wm.params.iter().any(|p| mentions(&p.ty, &tp.name)) || free_once(wm, &tp.name))
            && self.args_match_exactly(wm, args)
            && pool.iter().enumerate().all(|(i, (m, _, _))| i == w || !generic(&m.method) || self.surely_more_specific(wm, &m.method, n));
        let uncertain = rets_differ && pool.iter().any(|(_, _, plain)| *plain == Fit::Maybe) && !sure;
        Some((pool.swap_remove(w).0, uncertain))
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
        let (r, clean) = self.clean(|| self.guard(call.id(), TAG_RESOLVE, None, || self.infer_call(call, Target::Full)));
        if clean {
            self.calls.borrow_mut().insert(call.id(), r.clone());
        }
        r
    }

    /// Type of an argument as overload selection (`Target::None`) or partial
    /// inference (`Target::Param`) of the enclosing call sees it. A method
    /// invocation is not typed through `expected_type`: its target is the
    /// enclosing call being resolved, and asking for it cycles, which leaves
    /// every level of nested generic calls (`is(allOf(hasItem(...)))`)
    /// unmemoized and exponential. `Target::Param` uses the selected
    /// parameter type of the enclosing call instead, enough for
    /// `collect(Collectors.toList())`.
    fn arg_type(&self, a: Node<'t>, target: Target) -> Ty {
        if a.kind() != "method_invocation" {
            return self.type_of(a);
        }
        let key = (a.id(), target as u8);
        if let Some(t) = self.standalone.borrow().get(&key) {
            return t.clone();
        }
        let (t, clean) = self.clean(|| {
            self.guard(a.id(), TAG_STANDALONE + target as u8, Ty::Unknown, || {
                self.infer_call(a, target).map(|c| c.ret.clone()).unwrap_or(Ty::Unknown)
            })
        });
        if clean {
            self.standalone.borrow_mut().insert(key, t.clone());
        }
        t
    }

    /// Parameter type of the selected enclosing method for an argument,
    /// without inference.
    fn selected_param_type(&self, arg: Node<'t>) -> Ty {
        let Some(list) = arg.parent().filter(|p| p.kind() == "argument_list") else { return Ty::Unknown };
        let Some(call) = list.parent() else { return Ty::Unknown };
        let Some(sel) = self.select_method(call) else { return Ty::Unknown };
        let args = list.kids();
        let idx = args.iter().position(|k| *k == arg).unwrap_or(0);
        param_type_for_arg(&sel.method, idx, args.len()).unwrap_or(Ty::Unknown)
    }

    fn infer_call(&self, call: Node<'t>, target: Target) -> Option<Rc<ResolvedCall>> {
        {
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
                let exp = match target {
                    Target::Full => self.expected_type(call),
                    Target::Param => self.selected_param_type(call),
                    Target::None => Ty::Unknown,
                };
                if exp.is_known() {
                    unify(self, &rc.method.ret, &exp, &tvars, &mut bindings);
                }
            }
            for p in &mut rc.method.params {
                p.ty = apply_subst(&p.ty, &bindings);
            }
            rc.ret = self.finish_type(&rc.method.ret, &bindings, call);
            Some(Rc::new(rc))
        }
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
                let at = self.arg_type(a, Target::Param);
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
        // all returns must agree (or widen to one of them); otherwise unknown
        let mut result = Ty::Void;
        let mut known: Option<Ty> = None;
        let mut stack = vec![body];
        while let Some(n) = stack.pop() {
            for k in n.kids() {
                match k.kind() {
                    "return_statement" => {
                        if let Some(e) = k.first_kid() {
                            let t = self.type_of(e);
                            if t.is_known() && t != Ty::Null {
                                known = match known {
                                    None => Some(t),
                                    Some(prev) if prev == t => Some(prev),
                                    Some(prev) => {
                                        // generic types would need their arguments compared
                                        let sub = |a: &Ty, b: &Ty| a.args().is_empty() && b.args().is_empty() && a.class_name().zip(b.class_name()).is_some_and(|(x, y)| self.is_subclass(x, y).is_yes());
                                        let partly_unknown = |a: &Ty| a.args().iter().any(|x| !x.is_known());
                                        if sub(&t, &prev) {
                                            Some(prev)
                                        } else if sub(&prev, &t) {
                                            Some(t)
                                        } else if prev.erasure() == t.erasure() && partly_unknown(&t) && !partly_unknown(&prev) {
                                            // `Mono<Plugin>` and `Mono<?>` from an unresolved call
                                            Some(prev)
                                        } else if prev.erasure() == t.erasure() && partly_unknown(&prev) && !partly_unknown(&t) {
                                            Some(t)
                                        } else {
                                            return Ty::Unknown;
                                        }
                                    }
                                };
                            } else {
                                result = Ty::Unknown;
                            }
                        }
                    }
                    "lambda_expression" | "class_body" | "class_declaration" => {}
                    _ => stack.push(k),
                }
            }
        }
        known.unwrap_or(result)
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

/// Type argument `ba` contains the actual argument `sa` (`? super String`
/// contains `String`); false when unsure.
fn contains(lk: &dyn ClassLookup, ba: &Ty, sa: &Ty) -> bool {
    let sub = |x: &Ty, y: &Ty| {
        x == y
            || y.class_name() == Some("java.lang.Object")
            || matches!((x, y), (Ty::Class { name: xn, args: xa }, Ty::Class { name: yn, args: ya }) if xa.is_empty() && ya.is_empty() && lk.is_subclass(xn, yn).is_yes())
    };
    match ba {
        Ty::Wildcard(None) => true,
        Ty::Wildcard(Some(w)) => match &**w {
            WildcardBound::Extends(x) => sub(sa, x),
            WildcardBound::Super(x) => sub(x, sa),
        },
        t => t == sa,
    }
}

fn has_wildcard(t: &Ty) -> bool {
    match t {
        Ty::Wildcard(_) => true,
        Ty::Class { args, .. } => args.iter().any(has_wildcard),
        Ty::Array(e) => has_wildcard(e),
        _ => false,
    }
}

fn occurrences(t: &Ty, v: &str) -> usize {
    match t {
        Ty::TypeVar(n) => usize::from(&**n == v),
        Ty::Class { args, .. } => args.iter().map(|a| occurrences(a, v)).sum(),
        Ty::Array(e) => occurrences(e, v),
        Ty::Wildcard(Some(b)) => match &**b {
            WildcardBound::Extends(t) | WildcardBound::Super(t) => occurrences(t, v),
        },
        _ => 0,
    }
}

/// Type variables of `m` that inference cannot get wrong: no bound but
/// Object, used once in all of `m`'s parameters (not a varargs element), and
/// not by another variable's bound. Any argument fixes such a variable.
fn free_once(m: &MethodInfo, v: &str) -> bool {
    let Some(tp) = m.type_params.iter().find(|tp| &*tp.name == v) else { return false };
    let np = m.params.len();
    tp.bounds.iter().all(|b| b.class_name() == Some("java.lang.Object"))
        && m.params.iter().map(|p| occurrences(&p.ty, v)).sum::<usize>() == 1
        && !(m.is_varargs() && np > 0 && mentions(&m.params[np - 1].ty, v))
        && !m.type_params.iter().any(|o| o.name != tp.name && o.bounds.iter().any(|b| mentions(b, v)))
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
