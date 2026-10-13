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
    /// arity, whether the body fits a void / a value-returning
    /// functional method (JLS 15.27.2), whether it is explicitly typed (typed
    /// parameters, or none at all), and the expression body without
    /// parameters
    Lambda(usize, LambdaCompat, bool, Option<LambdaBody>),
    MethodRef,
}

/// An expression body of a lambda without parameters.
#[derive(Clone, Debug)]
pub struct LambdaBody {
    /// its type alone
    ty: Ty,
    /// a generic call (`() -> List.of("x")`), which javac infers with the
    /// functional result as target
    poly: bool,
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

/// An argument javac types with its parameter as target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Poly {
    No,
    /// its type arguments follow the parameter
    Yes,
    /// so do its type arguments, but where its own inference left its
    /// result open it lacks a bound
    Unsettled,
}

impl Poly {
    fn any(self) -> bool {
        self != Poly::No
    }
}

/// How sure an overload choice is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Choice {
    Sure,
    /// the candidates agree on the erased result only
    ErasureOnly,
    Guess,
}

/// Outcome of a method invocation conversion with type arguments.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Conv {
    Yes,
    /// only through unchecked conversion of a raw type
    Unchecked,
    No,
    Unknown,
}

impl Conv {
    /// Subtyping proper, as a type argument needs it.
    fn strict(self) -> Tri {
        match self {
            Conv::Yes => Tri::Yes,
            Conv::Unchecked | Conv::No => Tri::No,
            Conv::Unknown => Tri::Unknown,
        }
    }
}

/// A type seen as one of its supertypes.
enum SuperView {
    Args(Vec<Ty>),
    /// reached through a raw type
    Raw,
    Unknown,
}

/// How an argument bounds a type variable (JLS 18.2): exactly (`List<T>`),
/// from below (`T`, `? extends T`) or from above (`? super T`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rel {
    Eq,
    Lower,
    Upper,
}

/// A type variable bound to the capture of a wildcard argument.
#[derive(Clone, Debug)]
struct Capture {
    var: Name,
    /// the variable is the capture (Eq), above it (Lower) or below it (Upper)
    rel: Rel,
    wildcard: Ty,
    /// upper bound of the capture, if simple
    bound: Option<Ty>,
    /// lower bound (`? super X`)
    lower: Option<Ty>,
}

/// Type variables of a method inferred from argument types, and whether the
/// arguments surely fit.
#[derive(Clone, Debug, Default)]
struct Inference {
    /// resolved variables; a captured one is bound to its capture's bound
    bindings: Vec<(Name, Ty)>,
    captures: Vec<Capture>,
    /// variables whose binding would be a guess (conflicting bounds, a lub
    /// or glb of unrelated types): left unbound for good
    poisoned: Vec<Name>,
    /// an argument type is unknown or a bound could not be reduced
    incomplete: bool,
    /// an argument fits only by unchecked conversion
    unchecked: bool,
    /// the arguments surely do not fit
    no: bool,
    /// a poly argument bounds a variable
    soft: bool,
    /// the variables a poly argument bounds
    soft_vars: Vec<Name>,
    /// variables an argument of unknown type bounds too
    unknown_vars: Vec<Name>,
    /// variables bound from above only (`? super T`): the target picks them
    upper_only: Vec<Name>,
    /// variables bound to a computed lub: a target type can widen them
    /// (`Stream<Object> s = Stream.concat(a, b)`)
    widenable: Vec<Name>,
    /// incomplete for a reason other than an unknown type
    guess: bool,
    /// variables an invariant argument gives in part (`Map<K, List<?>>`)
    partial: Vec<(Name, Ty)>,
}

impl Inference {
    fn sure(&self) -> bool {
        !self.no && !self.incomplete && self.poisoned.is_empty()
    }

    fn unknown() -> Inference {
        Inference { incomplete: true, guess: true, ..Inference::default() }
    }

    /// Short of proof for a reason other than an unknown type.
    fn gap(&mut self) {
        self.incomplete = true;
        self.guess = true;
    }

    /// Nothing against the arguments but types the model does not know: what
    /// the old erasure check accepted.
    fn only_blind(&self) -> bool {
        !self.no && self.poisoned.iter().all(|v| self.unknown_vars.contains(v)) && !self.guess
    }
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
                let typed = a.field("parameters").is_none_or(|p| p.kind() == "formal_parameters");
                let body = a.field("body").filter(|b| n == 0 && b.kind() != "block").and_then(|b| {
                    let b = b.unparen();
                    let ty = self.arg_type(b, Target::None);
                    let poly = self.is_poly(b);
                    (ty.is_known() && !self.poly_untyped(b)).then_some(LambdaBody { ty, poly })
                });
                ArgShape::Lambda(n, compat, typed, body)
            }
            "method_reference" => ArgShape::MethodRef,
            // its type alone is a provisional guess, erasure included
            _ if self.poly_untyped(a) => ArgShape::Ty(Ty::Unknown),
            _ => ArgShape::Ty(self.arg_type(a, Target::None)),
        }
    }

    /// An argument whose type javac infers with the parameter as target: a
    /// generic method call (`List.of(x)`), a diamond, a conditional of those.
    fn is_poly(&self, a: Node<'t>) -> bool {
        let a = a.unparen();
        match a.kind() {
            "method_invocation" => {
                a.child_by_field_name("type_arguments").is_none()
                    && self.select_method(a).is_some_and(|c| c.method.type_params.iter().any(|tp| mentions(&c.method.ret, &tp.name)))
            }
            "object_creation_expression" => a.field("type").is_some_and(|t| t.kind() == "generic_type" && t.kid_of_kind("type_arguments").is_some_and(|ta| ta.kids().is_empty())),
            "ternary_expression" => a.kids().iter().skip(1).any(|k| self.is_poly(*k)),
            "switch_expression" => true,
            _ => false,
        }
    }

    /// A poly expression whose very class comes from the target: a generic
    /// call returning a bare type variable of its own that its arguments do
    /// not fix (`<X> X first(List<? super X>)`, `<T> T empty()`). One
    /// bounded from below or exactly by plain arguments (`Arrays.copyOf(xs,
    /// 2)`, `requireNonNull(x)`) has that type, the least any target allows.
    fn poly_untyped(&self, a: Node<'t>) -> bool {
        self.poly_target(a).is_some()
    }

    /// Types a target-typed call's result is surely a supertype of: its
    /// plain arguments for a parameter that is the bare result variable
    /// (`requireNonNullElse(x, new byte[0])` is no `char[]`, whatever `x` is).
    fn poly_floor(&self, a: Node<'t>) -> Vec<Ty> {
        let a = a.unparen();
        if a.kind() != "method_invocation" || !self.poly_untyped(a) {
            return Vec::new();
        }
        let Some(c) = self.select_method(a) else { return Vec::new() };
        let Ty::TypeVar(v) = &c.method.ret else { return Vec::new() };
        if c.ambiguous || c.erasure_only || c.method.is_varargs() {
            return Vec::new();
        }
        let args = self.args_of(a);
        if args.len() != c.method.params.len() {
            return Vec::new();
        }
        args.iter()
            .zip(&c.method.params)
            .filter(|(x, p)| matches!(&p.ty, Ty::TypeVar(w) if w == v) && !self.is_poly(**x))
            .filter_map(|(x, _)| match self.arg_type(*x, Target::None) {
                Ty::Prim(pr) => Some(Ty::class(pr.box_name())),
                t @ (Ty::Class { .. } | Ty::Array(_)) if !t.has_unknown() && !t.mentions_type_var() => Some(t),
                _ => None,
            })
            .collect()
    }

    /// For a target-typed call ([`Self::poly_untyped`]): whether it only
    /// adapts to its target and bounds nothing itself. One whose variable a
    /// lambda also bounds (`apply(i, x -> 1)`) adds a lower bound.
    fn poly_adapts(&self, a: Node<'t>) -> bool {
        self.poly_target(a) == Some(true)
    }

    /// Whether the argument is a poly argument as the enclosing inference
    /// sees it: one only typed by a lambda counts as an unknown type.
    fn poly_arg(&self, a: Node<'t>) -> Poly {
        if !self.is_poly(a) || self.poly_untyped(a) && !self.poly_adapts(a) {
            Poly::No
        } else if self.poly_unsettled(a) {
            Poly::Unsettled
        } else {
            Poly::Yes
        }
    }

    /// A generic call whose own inference left a variable of its result
    /// open for a reason (`singletonList(requireNonNullElseGet(b, () -> c))`):
    /// what it lacks there is a bound, not a free choice. Its other type
    /// arguments still adapt to the parameter.
    fn poly_unsettled(&self, a: Node<'t>) -> bool {
        let a = a.unparen();
        if a.kind() != "method_invocation" {
            return false;
        }
        let Some(c) = self.select_method(a) else { return false };
        let inf = self.partial_inference(a, &c.method);
        inf.poisoned.iter().chain(&inf.unknown_vars).any(|v| mentions(&c.method.ret, v))
            || self.args_of(a).iter().any(|x| self.poly_untyped(*x) && !self.poly_adapts(*x))
    }

    fn poly_target(&self, a: Node<'t>) -> Option<bool> {
        let a = a.unparen();
        match a.kind() {
            "method_invocation" => {
                if a.child_by_field_name("type_arguments").is_some() {
                    return None;
                }
                let c = self.select_method(a)?;
                let mut r = &c.method.ret;
                while let Ty::Array(e) = r {
                    r = e;
                }
                let Ty::TypeVar(v) = r else { return None };
                if !c.method.type_params.iter().any(|tp| tp.name == *v) {
                    return None;
                }
                // a lambda or method reference may add a lower bound once checked
                // (`requireNonNullElseGet("x", () -> 1)` is no String)
                let args = self.args_of(a);
                let functional = args.iter().enumerate().any(|(i, x)| {
                    matches!(x.unparen().kind(), "lambda_expression" | "method_reference") && param_type_for_arg(&c.method, i, args.len()).is_some_and(|p| mentions(&p, v))
                });
                if functional {
                    return Some(false);
                }
                let inf = self.partial_inference(a, &c.method);
                let bound = inf.bindings.iter().any(|(k, _)| k == v);
                let fixed = bound && !inf.upper_only.contains(v) && !inf.soft_vars.contains(v) && !inf.poisoned.contains(v) && !inf.captures.iter().any(|c| c.var == *v);
                if fixed {
                    return None;
                }
                // unbound, or bounded from above only: it adapts
                Some(!bound || inf.upper_only.contains(v) && !inf.soft_vars.contains(v))
            }
            "ternary_expression" => a.kids().iter().skip(1).any(|k| self.poly_untyped(*k)).then_some(false),
            "switch_expression" => Some(false),
            _ => None,
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
            ArgShape::Lambda(n, compat, _, body) => {
                if matches!(param, Ty::Array(_) | Ty::Prim(_)) {
                    return Fit::No;
                }
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
                                match body {
                                    // without parameters the body counts for applicability
                                    // (JLS 15.12.2.2); an int constant also narrows to byte, short, char
                                    // `() -> 1` is no `Iterator<T>` whatever T is
                                    Some(bt) if !void && m.method.ret.mentions_type_var() => {
                                        let bc = bt.ty.boxed();
                                        match (bc.class_name(), m.method.ret.class_name()) {
                                            (Some(x), Some(y)) if self.is_subclass(x, y).is_no() => Fit::No,
                                            _ => Fit::Strict,
                                        }
                                    }
                                    Some(bt) if !void => {
                                        let narrows = matches!(bt.ty, Ty::Prim(Prim::Int))
                                            && matches!(m.method.ret.unboxed(), Some(Prim::Byte | Prim::Short | Prim::Char));
                                        match self.body_fits(bt, &m.method.ret) {
                                            Conv::No if !narrows => Fit::No,
                                            Conv::Yes | Conv::Unchecked => Fit::Strict,
                                            _ => Fit::Maybe,
                                        }
                                    }
                                    _ => Fit::Strict,
                                }
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
            ArgShape::MethodRef if matches!(param, Ty::Array(_) | Ty::Prim(_)) => Fit::No,
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
                // type arguments count too: a `List<String>` is no `List<Integer>`
                match self.conv(a, param) {
                    Conv::Yes | Conv::Unchecked => {
                        if strict {
                            Fit::Strict
                        } else {
                            Fit::Loose
                        }
                    }
                    Conv::No => Fit::No,
                    Conv::Unknown => Fit::Maybe,
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

    /// Phase in which `m` fits the args (1 strict, 2 loose, 3 varargs),
    /// with the inference over the arguments' full types. `None` when `m`
    /// surely does not fit; the inference says whether it surely does.
    fn candidate_fit(&self, m: &MethodInfo, args: &[ArgShape], poly: &[Poly], floors: &[Vec<Ty>]) -> Option<(u8, Inference)> {
        let n = args.len();
        let np = m.params.len();
        let own = |t: &Ty| m.type_params.iter().any(|tp| mentions(t, &tp.name));
        let in_phase = |params: Vec<Ty>| -> Option<(bool, Inference)> {
            let mut loose = false;
            let mut fits = Vec::new();
            for (((p, a), poly), floor) in params.iter().zip(args).zip(poly).zip(floors) {
                // a target-typed call is a supertype of its floor
                if !p.is_prim() && !own(p) && floor.iter().any(|f| self.conv(f, &p.erasure()) == Conv::No) {
                    return None;
                }
                let mut f = self.fit_in(m, p, a);
                // a poly argument's type arguments follow the parameter in javac
                let mut elem = p;
                while let Ty::Array(e) = elem {
                    elem = e;
                }
                if f == Fit::No && poly.any() && !elem.args().is_empty() {
                    if let ArgShape::Ty(t) = a {
                        if self.fit(&erase_in(m, p), &ArgShape::Ty(t.erasure())) != Fit::No {
                            f = Fit::Maybe;
                        }
                    }
                }
                if f == Fit::No {
                    return None;
                }
                // boxing or unboxing makes it phase 2 even where the rest is unsure
                let boxing = match a {
                    ArgShape::Ty(Ty::Prim(_)) => !p.is_prim(),
                    ArgShape::Ty(Ty::Class { .. }) => p.is_prim(),
                    _ => false,
                };
                loose |= f == Fit::Loose || boxing;
                fits.push(f);
            }
            let tys: Vec<Option<Ty>> = args.iter().map(|a| if let ArgShape::Ty(t) = a { Some(t.clone()) } else { None }).collect();
            let mut inf = self.infer_args(m, &params, &tys, poly);
            if inf.no {
                return None;
            }
            // a lambda is checked for arity and void-ness only, which settles
            // applicability unless it is explicitly typed; without parameters
            // its body type settles it against the instantiated result
            let functions_fit = params.iter().zip(args).zip(&fits).all(|((p, a), f)| match a {
                ArgShape::Lambda(n, _, typed, body) if *f == Fit::Strict => {
                    !typed
                        || *n == 0 && {
                            let sam = self.functional_method(&apply_subst(p, &inf.bindings));
                            // a free variable of `m` without bounds takes the body's type
                            let free = |v: &Name| {
                                !inf.bindings.iter().any(|(k, _)| k == v)
                                    && !inf.poisoned.contains(v)
                                    && m.type_params.iter().find(|tp| tp.name == *v).is_some_and(|tp| tp.bounds.iter().all(|b| b.is_class("java.lang.Object")))
                            };
                            sam.is_some_and(|sam| match (&sam.method.ret, body) {
                                (Ty::Void, _) => true,
                                (Ty::TypeVar(v), Some(_)) => free(v),
                                (r, Some(bt)) => !r.mentions_type_var() && self.body_fits(bt, r) == Conv::Yes,
                                _ => false,
                            })
                        }
                }
                ArgShape::Ty(_) => true,
                _ => false,
            });
            if !functions_fit {
                inf.gap();
            }
            Some((loose, inf))
        };
        // phases 1/2: fixed arity
        if n == np {
            if let Some((loose, inf)) = in_phase(m.params.iter().map(|p| p.ty.clone()).collect()) {
                return Some((if loose { 2 } else { 1 }, inf));
            }
        }
        // phase 3: varargs
        if m.is_varargs() && n + 1 >= np {
            let elem = match &m.params[np - 1].ty {
                Ty::Array(e) => (**e).clone(),
                t => t.clone(),
            };
            let params = (0..n).map(|i| if i + 1 < np { m.params[i].ty.clone() } else { elem.clone() }).collect();
            return in_phase(params).map(|(_, inf)| (3, inf));
        }
        None
    }

    /// A lambda body against the functional result `r`. A generic call
    /// there is inferred with `r` as target (JLS 18.5.2), with its own
    /// arguments' constraints and its declared bounds: beyond this model,
    /// so only its erasure can rule it out.
    fn body_fits(&self, b: &LambdaBody, r: &Ty) -> Conv {
        if !b.poly {
            return self.conv(&b.ty, r);
        }
        match self.conv(&b.ty.erasure(), &r.erasure()) {
            Conv::No => Conv::No,
            _ => Conv::Unknown,
        }
    }

    /// `t` viewed as its supertype `target` with `t`'s own type arguments,
    /// wildcards included (`as_super` replaces them by their bounds).
    fn super_view(&self, t: &Ty, target: &str) -> SuperView {
        let Ty::Class { name, args } = t else { return SuperView::Unknown };
        let Some(c) = self.class(name) else { return SuperView::Unknown };
        if args.is_empty() && !c.type_params.is_empty() {
            return SuperView::Raw;
        }
        if args.len() != c.type_params.len() {
            return SuperView::Unknown;
        }
        if &**name == target {
            return SuperView::Args(args.clone());
        }
        let generic = |n: &str| self.class(n).is_some_and(|c| !c.type_params.is_empty());
        // which argument of `t` lands where in `target` (`List<#0>` is `Iterable<#0>`)
        let decl = Ty::Class { name: name.clone(), args: (0..args.len()).map(|i| Ty::TypeVar(Arc::from(format!("#{i}")))).collect() };
        let supers = self.all_supertypes(&decl);
        let Some(sup) = supers.iter().find(|s| s.class_name() == Some(target)) else { return SuperView::Unknown };
        if sup.args().is_empty() {
            return if generic(target) { SuperView::Raw } else { SuperView::Args(Vec::new()) };
        }
        // a raw supertype on the way (`class Raw<X> extends ArrayList`) leaves
        // the arguments above it unknown
        let raw_on_way = supers.iter().any(|s| s.args().is_empty() && s.class_name().is_some_and(generic));
        if raw_on_way || sup.has_unknown() {
            return if raw_on_way && sup.has_unknown() { SuperView::Raw } else { SuperView::Unknown };
        }
        let slot = |v: &str| v.strip_prefix('#').and_then(|i| i.parse::<usize>().ok()).filter(|i| *i < args.len());
        let mut out = Vec::new();
        for a in sup.args() {
            match a {
                Ty::TypeVar(v) if slot(v).is_some() => out.push(args[slot(v).unwrap()].clone()),
                a => {
                    // a wildcard nested deeper would need capture conversion
                    if (0..args.len()).any(|i| matches!(args[i], Ty::Wildcard(_)) && mentions(a, &format!("#{i}"))) {
                        return SuperView::Unknown;
                    }
                    out.push(a.subst(&|v| slot(v).map(|i| args[i].clone())));
                }
            }
        }
        SuperView::Args(out)
    }

    /// Method invocation conversion of `s` to `t` with type arguments (JLS
    /// 5.3, 4.10.2, 4.5.1). Type variables are left undecided.
    fn conv(&self, s: &Ty, t: &Ty) -> Conv {
        if s == t && !s.has_unknown() {
            return Conv::Yes;
        }
        let yes = |b: bool| if b { Conv::Yes } else { Conv::No };
        match (s, t) {
            (Ty::Unknown, _) | (_, Ty::Unknown) => Conv::Unknown,
            (Ty::Void, _) | (_, Ty::Void) => Conv::No,
            (Ty::Null, t) => yes(t.is_reference()),
            (Ty::Prim(a), Ty::Prim(b)) => yes(a.widens_to(*b)),
            (Ty::Prim(a), Ty::Class { .. }) => self.conv(&Ty::class(a.box_name()), t),
            (Ty::Class { name, .. }, Ty::Prim(b)) => yes(Prim::from_box_name(name).is_some_and(|a| a.widens_to(*b))),
            (Ty::TypeVar(_) | Ty::Wildcard(_), _) | (_, Ty::TypeVar(_) | Ty::Wildcard(_)) => Conv::Unknown,
            (Ty::Array(_), Ty::Class { name, .. }) => yes(matches!(&**name, "java.lang.Object" | "java.lang.Cloneable" | "java.io.Serializable")),
            (Ty::Array(a), Ty::Array(b)) => match (&**a, &**b) {
                (Ty::Prim(x), Ty::Prim(y)) => yes(x == y),
                (Ty::Prim(_), _) | (_, Ty::Prim(_)) => Conv::No,
                (x, y) => self.conv(x, y),
            },
            (Ty::Class { name: sn, .. }, Ty::Class { name: tn, args: targs }) => match self.is_subclass(sn, tn) {
                Tri::No => Conv::No,
                Tri::Unknown => Conv::Unknown,
                Tri::Yes if targs.is_empty() => Conv::Yes,
                Tri::Yes => match self.super_view(s, tn) {
                    // a raw type converts to `List<?>` without a warning
                    SuperView::Raw if targs.iter().all(|t| matches!(t, Ty::Wildcard(None))) => Conv::Yes,
                    SuperView::Raw => Conv::Unchecked,
                    SuperView::Args(sargs) if sargs.len() == targs.len() => {
                        let decl = self.class(tn);
                        let free = |i: usize| decl.as_ref().and_then(|c| c.type_params.get(i)).is_some_and(|tp| tp.bounds.iter().all(|b| b.is_class("java.lang.Object")));
                        match targs.iter().zip(&sargs).enumerate().fold(Tri::Yes, |acc, (i, (ta, sa))| acc.and(self.contains_arg(ta, sa, free(i)))) {
                            Tri::Yes => Conv::Yes,
                            Tri::No => Conv::No,
                            Tri::Unknown => Conv::Unknown,
                        }
                    }
                    _ => Conv::Unknown,
                },
            },
            (Ty::Class { .. } | Ty::Prim(_), Ty::Array(_)) | (Ty::Array(_), Ty::Prim(_)) => Conv::No,
            _ => Conv::Unknown,
        }
    }

    /// Type argument `ta` contains the type argument `sa` (JLS 4.5.1); a
    /// wildcard in `sa` stands for its capture, whose upper bound is Object
    /// when the type parameter is `free` of bounds.
    fn contains_arg(&self, ta: &Ty, sa: &Ty, free: bool) -> Tri {
        if ta == sa && !ta.has_unknown() {
            return Tri::Yes;
        }
        let object = |x: &Ty| x.is_class("java.lang.Object");
        match ta {
            Ty::Wildcard(None) => Tri::Yes,
            Ty::Wildcard(Some(w)) => match (&**w, sa) {
                (WildcardBound::Extends(x), _) if object(x) => Tri::Yes,
                (_, Ty::Unknown) => Tri::Unknown,
                (WildcardBound::Extends(x), Ty::Wildcard(Some(sw))) => match &**sw {
                    WildcardBound::Extends(y) => self.conv(y, x).strict(),
                    // the capture's upper bound is the declared one
                    WildcardBound::Super(_) if free => Tri::No,
                    WildcardBound::Super(_) => Tri::Unknown,
                },
                (WildcardBound::Extends(_), Ty::Wildcard(None)) if free => Tri::No,
                (WildcardBound::Extends(_), Ty::Wildcard(None)) => Tri::Unknown,
                (WildcardBound::Extends(x), s) => self.conv(s, x).strict(),
                (WildcardBound::Super(x), Ty::Wildcard(Some(sw))) => match &**sw {
                    WildcardBound::Super(y) => self.conv(x, y).strict(),
                    WildcardBound::Extends(_) => Tri::No,
                },
                (WildcardBound::Super(_), Ty::Wildcard(None)) => Tri::No,
                (WildcardBound::Super(x), s) => self.conv(x, s).strict(),
            },
            _ if ta.has_unknown() || sa.has_unknown() => Tri::Unknown,
            _ if matches!(sa, Ty::Wildcard(_)) => Tri::No,
            _ if ta.mentions_type_var() || sa.mentions_type_var() => Tri::Unknown,
            _ => Tri::No,
        }
    }

    /// Least upper bound (JLS 4.10.4) of class types when it is one class:
    /// the only minimal common supertype, with type arguments the types
    /// agree on. `None` for an intersection or differing type arguments.
    fn lub(&self, tys: &[Ty]) -> Option<Ty> {
        if tys.iter().any(|t| !matches!(t, Ty::Class { .. }) || t.has_unknown() || t.mentions_type_var()) {
            return None;
        }
        let first = tys.first()?;
        let mut common = Vec::new();
        for st in self.all_supertypes(&first.erasure()) {
            let n = st.class_name()?.to_string();
            let mut all = true;
            for t in tys {
                match self.is_subclass(t.class_name()?, &n) {
                    Tri::Yes => {}
                    Tri::No => all = false,
                    // a library class missing from the classpath does not
                    // extend a class of the project's own sources
                    Tri::Unknown if self.index.is_source_class(&n) => all = false,
                    Tri::Unknown => return None,
                }
            }
            if all {
                common.push(n);
            }
        }
        let minimal: Vec<&String> = common.iter().filter(|c| !common.iter().any(|d| d != *c && self.is_subclass(d, c).is_yes())).collect();
        let [name] = minimal[..] else { return None };
        let mut args: Option<Vec<Ty>> = None;
        for t in tys {
            let SuperView::Args(xs) = self.super_view(t, name) else { return None };
            if xs.iter().any(|x| matches!(x, Ty::Wildcard(_)) || x.has_unknown()) || args.as_ref().is_some_and(|a| *a != xs) {
                return None;
            }
            args = Some(xs);
        }
        Some(Ty::Class { name: Arc::from(name.as_str()), args: args.unwrap_or_default() })
    }

    /// The type an F-bound `G<.., T, ..>` of `t` fixes for `t` given `b <:
    /// T`: `b` seen as a `G` has a plain type argument where `T` stands.
    fn f_bound_type(&self, b: &Ty, bound: &Ty, t: &str) -> Option<Ty> {
        let Ty::Class { name, args } = bound else { return None };
        let i = args.iter().position(|a| matches!(a, Ty::TypeVar(v) if &**v == t))?;
        let SuperView::Args(xs) = self.super_view(b, name) else { return None };
        xs.get(i).filter(|x| matches!(x, Ty::Class { .. } | Ty::Array(_)) && !x.mentions_type_var() && !x.has_unknown()).cloned()
    }

    /// Like [`Self::f_bound_type`] for `G<.., ? super T, ..>`: the type `t`
    /// must be below.
    fn f_bound_type_super(&self, b: &Ty, bound: &Ty, t: &str) -> Option<Ty> {
        let Ty::Class { name, args } = bound else { return None };
        let i = args.iter().position(|a| matches!(a, Ty::Wildcard(Some(w)) if matches!(&**w, WildcardBound::Super(Ty::TypeVar(v)) if &**v == t)))?;
        let SuperView::Args(xs) = self.super_view(b, name) else { return None };
        xs.get(i).filter(|x| matches!(x, Ty::Class { .. } | Ty::Array(_)) && !x.mentions_type_var() && !x.has_unknown()).cloned()
    }

    /// Infer `m`'s type variables from the argument types (`None` for
    /// lambdas and method references) against `params`, one per argument,
    /// the way javac resolves them (JLS 18.4: an exact bound, else the lub
    /// of the lower bounds, else the glb of the upper ones, here only when
    /// one of them is that), and check the arguments against the parameters
    /// so instantiated.
    fn infer_args(&self, m: &MethodInfo, params: &[Ty], args: &[Option<Ty>], poly: &[Poly]) -> Inference {
        let mut inf = self.infer_args_exact(m, params, args, poly);
        // a poly argument (`List.of(x)`, `new ArrayList<>()`) takes its type
        // arguments from the parameter in javac: what it says about the
        // variables, or its own mismatch, proves nothing
        if inf.no && inf.soft {
            inf.no = false;
            // javac infers it with the parameter as target, which can resolve
            // the conflict or not: the variables it bounds stay unknown
            inf.gap();
            let soft = std::mem::take(&mut inf.soft_vars);
            inf.bindings.retain(|(k, _)| !soft.contains(k));
            inf.captures.retain(|c| !soft.contains(&c.var));
            for v in soft {
                if !inf.poisoned.contains(&v) {
                    inf.poisoned.push(v);
                }
            }
        }
        inf
    }

    fn infer_args_exact(&self, m: &MethodInfo, params: &[Ty], args: &[Option<Ty>], poly: &[Poly]) -> Inference {
        let vars: Vec<Name> = m.type_params.iter().map(|t| t.name.clone()).collect();
        let mut inf = Inference::default();
        let mut cs: Vec<(Name, Rel, Ty)> = Vec::new();
        for (i, (p, a)) in params.iter().zip(args).enumerate() {
            if let Some(a) = a {
                let before = (cs.len(), inf.captures.len());
                let unknown_before = inf.unknown_vars.len();
                self.gather(p, a, Rel::Lower, true, &vars, &mut cs, &mut inf);
                // a poly argument adapts its type arguments to the parameter
                // (`Collections.emptyList()` for `Collection<? extends K>`
                // bounds nothing); for a bare `T` its class is one more bound
                // and one without any type (`any()`) adapts to a bare `T` as well
                // (an unsettled one lacks a bound where its type is unknown)
                if poly.get(i) == Some(&Poly::Yes) && (!matches!(p, Ty::TypeVar(_)) || a.is_unknown()) {
                    inf.unknown_vars.truncate(unknown_before);
                }
                if poly.get(i).is_some_and(|p| p.any()) && (cs.len(), inf.captures.len()) != before {
                    inf.soft = true;
                    for c in &cs[before.0..] {
                        inf.soft_vars.push(c.0.clone());
                    }
                    inf.soft_vars.extend(inf.captures[before.1..].iter().map(|c| c.var.clone()));
                }
            }
        }
        // below a capture without lower bound only `null` fits: another
        // argument for the variable is no blind spot even when its type is unknown
        for c in inf.captures.clone() {
            if c.rel == Rel::Upper && c.lower.is_none() && params.iter().zip(args).filter(|(p, a)| mentions(p, &c.var) && a.as_ref().is_some_and(|a| *a != Ty::Null)).count() > 1 {
                inf.gap();
            }
        }
        // variables bound exactly: by equality, or as a capture
        let exact: Vec<Name> = vars.iter().filter(|v| cs.iter().any(|(k, r, _)| k == *v && *r == Rel::Eq) || inf.captures.iter().any(|c| c.var == **v && c.rel == Rel::Eq)).cloned().collect();
        for v in &vars {
            let of = |r: Rel| -> Vec<Ty> { cs.iter().filter(|(k, kr, _)| k == v && *kr == r).map(|(_, _, t)| t.clone()).collect() };
            let (eqs, lowers, uppers) = (of(Rel::Eq), of(Rel::Lower), of(Rel::Upper));
            let plain = |t: &Ty| !t.mentions_type_var() && !t.has_unknown();
            let fail = |inf: &mut Inference, sure: bool| {
                if sure {
                    inf.no = true
                } else if !inf.poisoned.contains(v) {
                    inf.poisoned.push(v.clone())
                }
            };
            // the bounds without captures first: a conflict there is final
            let b = if let Some(e) = eqs.first() {
                if eqs.iter().any(|x| x != e) {
                    fail(&mut inf, eqs.iter().all(plain));
                    continue;
                }
                Some(e.clone())
            } else if !lowers.is_empty() {
                let lub = lowers.iter().find(|l| lowers.iter().all(|o| self.conv(o, l) == Conv::Yes)).cloned().or_else(|| {
                    inf.widenable.push(v.clone());
                    self.lub(&lowers)
                });
                if lub.is_none() {
                    fail(&mut inf, false);
                }
                lub
            } else if !uppers.is_empty() {
                inf.upper_only.push(v.clone());
                // a poly argument adapts its type arguments to the parameter
                if inf.soft_vars.contains(v) {
                    fail(&mut inf, false);
                    continue;
                }
                let glb = uppers.iter().find(|u| uppers.iter().all(|o| self.conv(u, o) == Conv::Yes)).cloned();
                if glb.is_none() {
                    fail(&mut inf, false);
                }
                glb
            } else {
                None
            };
            if let Some(b) = &b {
                let checks: Vec<Conv> = lowers.iter().map(|l| self.conv(l, b)).chain(uppers.iter().map(|u| self.conv(b, u))).collect();
                for c in checks {
                    match c {
                        Conv::Yes => {}
                        Conv::No | Conv::Unchecked => inf.no = true,
                        Conv::Unknown => inf.incomplete = true,
                    }
                }
            }
            let caps: Vec<Capture> = inf.captures.iter().filter(|c| c.var == *v).cloned().collect();
            if caps.is_empty() {
                if let Some(b) = b {
                    if !inf.poisoned.contains(v) {
                        inf.bindings.push((v.clone(), b));
                    }
                }
                continue;
            }
            // above several captures, or a capture and other types
            // (`concat(Stream<? extends T>, Stream<? extends T>)`), T is their lub:
            // a proper type when the upper bounds have one
            if caps.len() + lowers.len() > 1 && eqs.is_empty() && uppers.is_empty() && caps.iter().all(|c| c.rel == Rel::Lower) {
                let bounds: Option<Vec<Ty>> = caps.iter().map(|c| c.bound.clone()).collect();
                let lub = bounds.and_then(|bs| {
                    let tys: Vec<Ty> = lowers.iter().cloned().chain(bs).collect();
                    tys.iter().find(|t| tys.iter().all(|o| self.conv(o, t) == Conv::Yes)).cloned().or_else(|| self.lub(&tys))
                });
                match lub {
                    Some(l) => {
                        inf.captures.retain(|c| c.var != *v);
                        inf.bindings.push((v.clone(), l));
                        inf.widenable.push(v.clone());
                    }
                    None => fail(&mut inf, false),
                }
                continue;
            }
            // A capture is a fresh type: a proper type is below it only below
            // its lower bound, it is below a proper type only through its upper
            // bound. Every lower-side bound must be below every upper-side one.
            enum Side<'c> {
                Proper(&'c Ty),
                Cap(&'c Capture),
            }
            let below: Vec<Side> = eqs.iter().chain(&lowers).map(Side::Proper).chain(caps.iter().filter(|c| c.rel != Rel::Upper).map(Side::Cap)).collect();
            let above: Vec<Side> = eqs.iter().chain(&uppers).map(Side::Proper).chain(caps.iter().filter(|c| c.rel != Rel::Lower).map(Side::Cap)).collect();
            for x in &below {
                for y in &above {
                    let verdict = match (x, y) {
                        (Side::Proper(_), Side::Proper(_)) => continue,
                        (Side::Cap(a), Side::Cap(b)) if std::ptr::eq(*a, *b) => continue,
                        (Side::Proper(t), Side::Cap(c)) => match &c.lower {
                            Some(l) => self.conv(t, l).strict(),
                            None if plain(t) => Tri::No,
                            None => Tri::Unknown,
                        },
                        (Side::Cap(c), Side::Proper(t)) => match &c.bound {
                            Some(b) => self.conv(b, t).strict(),
                            None => Tri::Unknown,
                        },
                        (Side::Cap(a), Side::Cap(b)) => match (&b.lower, &a.bound) {
                            (None, _) => Tri::No,
                            (Some(l), Some(ub)) if self.conv(ub, l) == Conv::Yes => Tri::Yes,
                            _ => Tri::Unknown,
                        },
                    };
                    match verdict {
                        Tri::No => inf.no = true,
                        Tri::Unknown => fail(&mut inf, false),
                        Tri::Yes => {}
                    }
                }
            }
            // the variable is the capture only when bound to it exactly; a
            // lub or glb with a capture is beyond this
            let exact_cap = caps.iter().filter(|c| c.rel == Rel::Eq).count() == 1 && eqs.is_empty();
            if !exact_cap && (caps.len() > 1 || !eqs.is_empty() || !lowers.is_empty() || !uppers.is_empty()) {
                fail(&mut inf, false);
            }
            if !inf.poisoned.contains(v) {
                // lambdas see the wildcard's bound
                let c = caps.iter().find(|c| c.rel == Rel::Eq).unwrap_or(&caps[0]);
                match &c.wildcard {
                    Ty::Wildcard(Some(w)) => {
                        let (WildcardBound::Extends(b) | WildcardBound::Super(b)) = &**w;
                        inf.bindings.push((v.clone(), b.clone()));
                    }
                    Ty::Wildcard(None) => {}
                    t => inf.bindings.push((v.clone(), t.clone())),
                }
            }
        }
        if inf.no {
            return inf;
        }
        // an argument of unknown type may add a bound that changes the
        // lub: only an exact bound fixes such a variable
        let loose: Vec<Name> = inf.unknown_vars.iter().filter(|v| !exact.contains(v)).cloned().collect();
        inf.bindings.retain(|(k, _)| !loose.contains(k));
        inf.captures.retain(|c| !loose.contains(&c.var));
        // nor may a target or lambda bind it later
        for v in loose {
            if !inf.poisoned.contains(&v) {
                inf.poisoned.push(v);
            }
        }
        let own = |t: &Ty| vars.iter().any(|v| mentions(t, v));
        // a variable of `m` in `t` with no binding (a binding may be the
        // caller's variable of the same name, so look before substituting)
        let open_in = |t: &Ty, inf: &Inference| vars.iter().any(|v| mentions(t, v) && !inf.bindings.iter().any(|(k, _)| k == v));
        // `U extends T` makes U's type a lower bound of T: a bound check that
        // settles nothing leaves T's lub open too
        let others = |bound: &Ty, tp: &Name, inf: &mut Inference| {
            for v in vars.iter().filter(|v| *v != tp && mentions(bound, v) && !exact.contains(v)) {
                if !inf.poisoned.contains(v) {
                    inf.poisoned.push(v.clone());
                }
            }
        };
        // declared bounds
        for tp in &m.type_params {
            let real: Vec<&Ty> = tp.bounds.iter().filter(|b| !b.is_class("java.lang.Object")).collect();
            if real.is_empty() {
                continue;
            }
            if let Some(c) = inf.captures.iter().find(|c| c.var == tp.name).cloned() {
                for bound in real {
                    let bb = apply_subst(bound, &inf.bindings);
                    let sure = !own(bound) || !open_in(bound, &inf) && vars.iter().filter(|v| mentions(bound, v)).all(|v| exact.contains(v) && !inf.captures.iter().any(|c| c.var == *v));
                    let verdict = match &c.bound {
                        None => Conv::Unknown,
                        Some(ub) if sure => self.conv(ub, &bb),
                        // `T extends Comparable<? super T>`: the classes must relate at least
                        Some(ub) => match (ub.erasure().class_name(), bound.erasure().class_name()) {
                            (Some(x), Some(y)) if self.is_subclass(x, y).is_no() => Conv::No,
                            _ => Conv::Unknown,
                        },
                    };
                    match verdict {
                        Conv::Yes => {}
                        // above the capture (`? super T`), javac takes an intersection
                        _ if c.rel == Rel::Upper => {
                            inf.poisoned.push(tp.name.clone());
                            others(bound, &tp.name, &mut inf);
                        }
                        // the capture, or a supertype of it: within the bound or not at all
                        Conv::No => inf.no = true,
                        _ => {
                            inf.poisoned.push(tp.name.clone());
                            others(bound, &tp.name, &mut inf);
                        }
                    }
                }
                continue;
            }
            let Some((_, mut b)) = inf.bindings.iter().find(|(k, _)| *k == tp.name).cloned() else { continue };
            for bound in real {
                // `U extends T` with T a capture: U's type must be below the capture's lower bound
                if let Ty::TypeVar(t) = bound {
                    if let Some(c) = inf.captures.iter().find(|c| c.var == *t && c.rel == Rel::Eq) {
                        let exact_b = exact.contains(&tp.name) || !b.mentions_type_var();
                        match c.lower.as_ref().map(|x| self.conv(&b, x)) {
                            Some(Conv::Yes) => {}
                            Some(Conv::No) | None if exact_b && !b.mentions_type_var() && !b.has_unknown() => inf.no = true,
                            _ => inf.gap(),
                        }
                        continue;
                    }
                }
                let bb = apply_subst(bound, &inf.bindings);
                if open_in(bound, &inf) || inf.captures.iter().any(|c| mentions(bound, &c.var)) {
                    inf.gap();
                    others(bound, &tp.name, &mut inf);
                    continue;
                }
                // `T extends Comparable<T>` over a lower bound `B` that is a
                // `Comparable<A>`: javac's incorporation makes T exactly A
                // `T extends Comparable<? super T>` over `B`, a `Comparable<A>`: T is below A
                if let Some(x) = self.f_bound_type_super(&b, bound, &tp.name) {
                    let lowers: Vec<&Ty> = cs.iter().filter(|(k, r, _)| *k == tp.name && *r == Rel::Lower).map(|(_, _, t)| t).collect();
                    if lowers.iter().any(|l| self.conv(l, &x) == Conv::No) {
                        inf.no = true;
                    }
                }
                if let Some(x) = self.f_bound_type(&b, bound, &tp.name).filter(|x| *x != b) {
                    let lowers: Vec<&Ty> = cs.iter().filter(|(k, r, _)| *k == tp.name && *r == Rel::Lower).map(|(_, _, t)| t).collect();
                    let eq = cs.iter().any(|(k, r, _)| *k == tp.name && *r == Rel::Eq);
                    if !eq && !lowers.is_empty() {
                        let fits: Vec<Conv> = lowers.iter().map(|l| self.conv(l, &x)).collect();
                        if fits.contains(&Conv::No) {
                            inf.no = true;
                        } else if fits.iter().all(|f| *f == Conv::Yes) && self.conv(&x, &apply_subst(bound, &[(tp.name.clone(), x.clone())])) == Conv::Yes {
                            b = x.clone();
                            if let Some(slot) = inf.bindings.iter_mut().find(|(k, _)| *k == tp.name) {
                                slot.1 = x;
                            }
                            continue;
                        }
                    }
                }
                let unrelated = || matches!((b.erasure().class_name(), bound.erasure().class_name()), (Some(x), Some(y)) if self.is_subclass(x, y).is_no());
                match self.conv(&b, &bb) {
                    Conv::Yes => {}
                    // the other variables bound exactly: a plain check
                    Conv::No if !own(bound) || unrelated() || vars.iter().filter(|v| **v != tp.name && mentions(bound, v)).all(|v| exact.contains(v)) && !mentions(bound, &tp.name) => {
                        inf.no = true
                    }
                    // a bound over variables adds bounds to them in javac
                    // (`U extends T` makes U's type a lower bound of T;
                    // `T extends Comparable<T>` can make T a superclass of
                    // its lower bound): not a proof
                    Conv::No => {
                        inf.poisoned.push(tp.name.clone());
                        others(bound, &tp.name, &mut inf);
                    }
                    Conv::Unchecked | Conv::Unknown => {
                        inf.gap();
                        others(bound, &tp.name, &mut inf);
                    }
                }
            }
        }
        if inf.no {
            return inf;
        }
        // the arguments against the instantiated parameters
        let subst: Vec<(Name, Ty)> = inf.bindings.iter().filter(|(k, _)| !inf.captures.iter().any(|c| c.var == *k)).cloned().collect();
        for (i, (p, a)) in params.iter().zip(args).enumerate() {
            let Some(a) = a else { continue };
            // a variable left unknown proves nothing either way, not even
            // through a capture it would otherwise have
            if inf.poisoned.iter().any(|v| mentions(p, v)) {
                inf.incomplete = true;
                continue;
            }
            let q = apply_subst(&with_captures(p, &inf.captures), &subst);
            let poly = poly.get(i).is_some_and(|p| p.any());
            // variables left open (checked before substituting: a binding may be
            // the caller's own variable of the same name)
            let open = vars.iter().any(|v| mentions(p, v) && !subst.iter().any(|(k, _)| k == v) && !inf.captures.iter().any(|c| c.var == *v));
            if open {
                // a free variable takes `null`; a raw argument fits by unchecked
                // conversion whatever the variables are
                let free = |t: &Ty| !own(t) || matches!(t, Ty::TypeVar(v) if !inf.poisoned.contains(v));
                let raw = matches!(&q, Ty::Class { name, .. } if matches!(a, Ty::Class { .. }) && self.is_subclass(a.class_name().unwrap_or(""), name).is_yes() && matches!(self.super_view(a, name), SuperView::Raw));
                if raw {
                    inf.unchecked = true;
                } else if !(*a == Ty::Null && free(&q)) {
                    inf.incomplete = true;
                }
                continue;
            }
            match self.conv(a, &q) {
                Conv::Yes => {}
                Conv::Unchecked => inf.unchecked = true,
                // javac may or may not infer it to fit: no proof either way
                Conv::No if poly && self.conv(&a.erasure(), &q.erasure()) != Conv::No => inf.gap(),
                Conv::No => inf.no = true,
                Conv::Unknown => inf.incomplete = true,
            }
        }
        let poisoned = std::mem::take(&mut inf.poisoned);
        inf.captures.retain(|c| !poisoned.contains(&c.var));
        inf.bindings.retain(|(k, _)| !poisoned.contains(k));
        inf.poisoned = poisoned;
        inf
    }

    /// Bounds on `vars` from argument type `a` meeting parameter type `p`
    /// under `rel`: `a` is `p` (Eq), a subtype of it (Lower) or a supertype
    /// (Upper). Only a `top` argument type is captured.
    #[allow(clippy::too_many_arguments)]
    fn gather(&self, p: &Ty, a: &Ty, rel: Rel, top: bool, vars: &[Name], cs: &mut Vec<(Name, Rel, Ty)>, inf: &mut Inference) {
        if !vars.iter().any(|v| mentions(p, v)) {
            return;
        }
        match (p, a) {
            (_, Ty::Unknown) => {
                inf.incomplete = true;
                inf.unknown_vars.extend(vars.iter().filter(|v| mentions(p, v)).cloned());
            }
            // `null` bounds nothing
            (_, Ty::Null) if rel == Rel::Lower => {}
            (Ty::Class { .. }, Ty::Prim(pr)) if rel == Rel::Lower => self.gather(p, &Ty::class(pr.box_name()), rel, top, vars, cs, inf),
            (Ty::TypeVar(v), _) => match a {
                Ty::Prim(pr) if rel == Rel::Lower => cs.push((v.clone(), rel, Ty::class(pr.box_name()))),
                // `T` for a `List<?>`: T is `List<CAP>`, shown as the argument's type
                Ty::Class { args, .. } if top && args.iter().any(|x| matches!(x, Ty::Wildcard(_))) && !a.has_unknown() => {
                    inf.captures.push(Capture { var: v.clone(), rel: Rel::Lower, wildcard: a.clone(), bound: Some(a.clone()), lower: None })
                }
                Ty::Class { .. } | Ty::Array(_) | Ty::TypeVar(_) if !a.has_unknown() => cs.push((v.clone(), rel, a.clone())),
                _ if a.has_unknown() => {
                    inf.incomplete = true;
                    inf.unknown_vars.push(v.clone());
                }
                _ => inf.gap(),
            },
            (Ty::Array(pe), Ty::Array(ae)) => match (&**pe, &**ae) {
                (Ty::TypeVar(_), Ty::Prim(_)) => inf.no = true,
                (_, Ty::Prim(_)) => inf.gap(),
                _ => self.gather(pe, ae, rel, false, vars, cs, inf),
            },
            (Ty::Class { name, args: pargs }, Ty::Class { name: an, args: aargs }) => match rel {
                Rel::Eq if an == name && aargs.len() == pargs.len() => {
                    for (pa, aa) in pargs.iter().zip(aargs) {
                        match (pa, aa) {
                            (Ty::Wildcard(Some(pw)), Ty::Wildcard(Some(aw))) => match (&**pw, &**aw) {
                                (WildcardBound::Extends(x), WildcardBound::Extends(y)) | (WildcardBound::Super(x), WildcardBound::Super(y)) => {
                                    self.gather(x, y, Rel::Eq, false, vars, cs, inf)
                                }
                                _ => inf.gap(),
                            },
                            (Ty::Wildcard(_), _) | (_, Ty::Wildcard(_)) => if vars.iter().any(|v| mentions(pa, v)) { inf.gap() },
                            _ => self.gather(pa, aa, Rel::Eq, false, vars, cs, inf),
                        }
                    }
                }
                Rel::Lower => match self.super_view(a, name) {
                    // unchecked conversion: the bounds from this argument vanish
                    SuperView::Raw => inf.unchecked = true,
                    SuperView::Args(sargs) if sargs.len() == pargs.len() => {
                        for (i, (pa, sa)) in pargs.iter().zip(&sargs).enumerate() {
                            self.gather_type_arg(name, i, pa, sa, top, vars, cs, inf);
                        }
                    }
                    _ => inf.incomplete = true,
                },
                _ => inf.gap(),
            },
            _ => inf.gap(),
        }
    }

    /// Bounds from type argument `pa` (number `i` of class `owner`) of a
    /// parameter that must contain the argument's type argument `sa`.
    #[allow(clippy::too_many_arguments)]
    fn gather_type_arg(&self, owner: &str, i: usize, pa: &Ty, sa: &Ty, top: bool, vars: &[Name], cs: &mut Vec<(Name, Rel, Ty)>, inf: &mut Inference) {
        if !vars.iter().any(|v| mentions(pa, v)) {
            return;
        }
        if sa.has_unknown() {
            inf.incomplete = true;
            inf.unknown_vars.extend(vars.iter().filter(|v| mentions(pa, v)).cloned());
            // an invariant argument known in part is the variable's exact
            // type in that part
            if let (Ty::TypeVar(v), Ty::Class { .. }) = (pa, sa) {
                if vars.contains(v) {
                    inf.partial.push((v.clone(), sa.clone()));
                }
            }
            return;
        }
        // the argument's own wildcards are captured (JLS 5.1.10): a variable
        // bounded by the capture alone is the capture
        let captured = |v: &Name, rel: Rel, inf: &mut Inference| {
            let declared_free = self.class(owner).and_then(|c| c.type_params.get(i).cloned()).is_none_or(|tp| tp.bounds.iter().all(|b| b.is_class("java.lang.Object")));
            let (bound, lower) = match sa {
                Ty::Wildcard(Some(b)) => match &**b {
                    WildcardBound::Extends(x) => (declared_free.then(|| x.clone()), None),
                    WildcardBound::Super(x) => (declared_free.then(Ty::object), Some(x.clone())),
                },
                _ => (declared_free.then(Ty::object), None),
            };
            inf.captures.push(Capture { var: v.clone(), rel, wildcard: sa.clone(), bound, lower });
        };
        match (pa, sa) {
            (Ty::Wildcard(None), _) => {}
            // `List<T>`, `List<? extends T>`, `List<? super T>` for a `List<? super Integer>`
            (Ty::TypeVar(v), Ty::Wildcard(_)) if top => captured(v, Rel::Eq, inf),
            (Ty::Wildcard(Some(w)), Ty::Wildcard(_)) if top => match &**w {
                WildcardBound::Extends(Ty::TypeVar(v)) if vars.contains(v) => captured(v, Rel::Lower, inf),
                WildcardBound::Super(Ty::TypeVar(v)) if vars.contains(v) => captured(v, Rel::Upper, inf),
                _ => inf.gap(),
            },
            (Ty::Wildcard(Some(w)), _) => match (&**w, sa) {
                (WildcardBound::Extends(x), Ty::Wildcard(Some(sw))) => match &**sw {
                    WildcardBound::Extends(y) => self.gather(x, y, Rel::Lower, false, vars, cs, inf),
                    WildcardBound::Super(_) => inf.gap(),
                },
                (WildcardBound::Super(x), Ty::Wildcard(Some(sw))) => match &**sw {
                    WildcardBound::Super(y) => self.gather(x, y, Rel::Upper, false, vars, cs, inf),
                    WildcardBound::Extends(_) => inf.gap(),
                },
                (_, Ty::Wildcard(None)) => inf.gap(),
                (WildcardBound::Extends(x), s) => self.gather(x, s, Rel::Lower, false, vars, cs, inf),
                (WildcardBound::Super(x), s) => self.gather(x, s, Rel::Upper, false, vars, cs, inf),
            },
            // a type argument is no wildcard: only capture makes it one
            (Ty::TypeVar(_), Ty::Wildcard(_)) => inf.no = true,
            (_, Ty::Wildcard(_)) => inf.gap(),
            _ => self.gather(pa, sa, Rel::Eq, false, vars, cs, inf),
        }
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

    /// `a` is more specific than `b` (JLS 15.12.2.5). Parameters without
    /// method type variables are compared with their type arguments; `sure`
    /// asks for proof instead of the absence of a counterexample.
    fn more_specific(&self, a: &MethodInfo, b: &MethodInfo, n: usize, sure: bool) -> bool {
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
        let own = |m: &MethodInfo, t: &Ty| m.type_params.iter().any(|tp| mentions(t, &tp.name));
        let count = n.max(a.params.len()).max(b.params.len());
        for i in 0..count {
            let (p, q) = (pa(a, i), pa(b, i));
            let (Some(x), Some(y)) = (erase(a, &p), erase(b, &q)) else { return false };
            let plain = !own(a, &p) && !own(b, &q);
            if x == y && !(sure && plain && p != q) {
                if plain && p != q && self.conv(&p, &q) == Conv::No {
                    return false;
                }
                continue;
            }
            match (x.prim(), y.prim()) {
                (Some(px), Some(py)) => {
                    if !px.widens_to(py) {
                        return false;
                    }
                }
                // no subtyping between primitive and reference types
                (Some(_), None) if sure => return false,
                (Some(_), None) => {}
                (None, Some(_)) => return false,
                // a generic `a` against a non-generic `b`: its parameters as
                // they stand, type variables included (JLS 15.12.2.5)
                _ if sure && !own(b, &q) && own(a, &p) => {
                    let ok = match &p {
                        Ty::TypeVar(v) => {
                            let bounds = a.type_params.iter().find(|tp| tp.name == *v).map(|tp| tp.bounds.clone()).unwrap_or_default();
                            q.is_class("java.lang.Object") || bounds.iter().any(|bd| !own(a, bd) && self.conv(bd, &q) == Conv::Yes)
                        }
                        _ => q.is_class("java.lang.Object") || self.conv(&p, &q) == Conv::Yes,
                    };
                    if !ok {
                        return false;
                    }
                }
                _ if sure => {
                    if !plain || self.conv(&p, &q) != Conv::Yes {
                        return false;
                    }
                }
                _ => {
                    if !self.is_assignable(&x, &y).is_yes() || (plain && self.conv(&p, &q) == Conv::No) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// The applicable method for `call` among `cands`, and how far the choice
    /// is a guess.
    fn select_among(&self, cands: Vec<MethodRef>, call: Node<'t>) -> Option<(MethodRef, Choice)> {
        let arg_nodes = self.args_of(call);
        let n = arg_nodes.len();
        let cands: Vec<MethodRef> = cands.into_iter().filter(|m| m.method.accepts_arity(n)).collect();
        let single = cands.len() == 1;
        if cands.is_empty() {
            return None;
        }
        // a guess matters only when the overloads differ in result; a bare
        // type variable of the method stands for any result
        let ret_key = |m: &MethodInfo| {
            let mut r = &m.ret;
            while let Ty::Array(e) = r {
                r = e;
            }
            (!matches!(r, Ty::TypeVar(v) if m.type_params.iter().any(|tp| tp.name == *v))).then(|| m.ret.erasure())
        };
        let rets_differ = cands.len() > 1 && {
            let first = ret_key(&cands[0].method);
            first.is_none() || cands.iter().any(|m| ret_key(&m.method) != first)
        };
        let same_rets = cands.iter().all(|m| m.method.ret == cands[0].method.ret && !m.method.type_params.iter().any(|tp| mentions(&m.method.ret, &tp.name)));
        let poly: Vec<Poly> = arg_nodes.iter().map(|a| self.poly_arg(*a)).collect();
        let args: Vec<ArgShape> = arg_nodes.iter().map(|a| self.arg_shape(*a)).collect();
        let args = &args[..];
        let floors: Vec<Vec<Ty>> = arg_nodes.iter().map(|a| self.poly_floor(*a)).collect();
        let fits: Vec<(MethodRef, (u8, Inference))> = cands.into_iter().filter_map(|m| self.candidate_fit(&m.method, args, &poly, &floors).map(|f| (m, f))).collect();
        if fits.is_empty() {
            return None;
        }
        let best_phase = fits.iter().map(|(_, f)| f.0).min().unwrap();
        // candidates not proven inapplicable, in any phase
        let rivals = fits.len() - 1;
        let mut pool: Vec<(MethodRef, Inference)> = fits.into_iter().filter(|(_, f)| f.0 == best_phase).map(|(m, f)| (m, f.1)).collect();
        // most specific
        let maximal: Vec<usize> = (0..pool.len())
            .filter(|&i| (0..pool.len()).all(|j| i == j || self.more_specific(&pool[i].0.method, &pool[j].0.method, n, false)))
            .collect();
        if maximal.len() != 1 {
            // ambiguous: keep first, flag it
            return Some((pool.swap_remove(0).0, Choice::Guess));
        }
        let w = maximal[0];
        let generic = |m: &MethodInfo| m.params.iter().any(|p| m.type_params.iter().any(|tp| mentions(&p.ty, &tp.name)));
        // The choice is sure when the winner surely fits, type arguments and
        // inferred bounds included, and surely beats every other candidate
        // that may fit as well; a non-generic one must beat a generic
        // competitor the way javac's inference would (`assertThat(String)`
        // over `<T> assertThat(T)`).
        let wm = &pool[w].0.method;
        // explicit `T.<Double>pick(x)` replaces inference: not modelled here
        let explicit = call.child_by_field_name("type_arguments").is_some();
        let sure = !explicit
            && pool[w].1.sure()
            && pool.iter().enumerate().all(|(i, (m, _))| {
                i == w || if generic(&m.method) { self.surely_more_specific(wm, &m.method, n) } else { self.more_specific(wm, &m.method, n, true) }
            });
        // the only candidate left after proofs in every phase, short of proof
        // only where a type is unknown: javac's pick in code that compiles.
        // A rival in a later phase wins where the unproven fit fails.
        let only = rivals == 0 && !explicit && pool[w].1.only_blind();
        // overloads alike in erasure but not in type arguments (`List<String>`,
        // `List<Integer>`) need the proof as well, or keep the erasure only
        let choice = if single || sure || only {
            Choice::Sure
        } else if rets_differ {
            Choice::Guess
        } else if same_rets {
            Choice::Sure
        } else {
            Choice::ErasureOnly
        };
        Some((pool.swap_remove(w).0, choice))
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
                    let (m, choice) = self.select_among(cands, call)?;
                    let (ambiguous, erasure_only) = (choice == Choice::Guess, choice == Choice::ErasureOnly);
                    Some(Rc::new(ResolvedCall { owner: m.owner.clone(), ret: m.method.ret.clone(), method: m.method, receiver: recv, ambiguous, erasure_only, via_type }))
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
        let (m, choice) = self.select_among(cands, call)?;
        Some(Rc::new(ResolvedCall { owner: m.owner.clone(), method: m.method, ret: class_ty, receiver: None, ambiguous: choice == Choice::Guess, erasure_only: false, via_type: false }))
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
            let tvars: Vec<Name> = rc.method.type_params.iter().map(|t| t.name.clone()).collect();
            let inf = if rc.method.params.is_empty() { Inference::default() } else { self.partial_inference(call, &rc.method) };
            // bindings the result was computed with
            let mut final_bindings: Vec<(Name, Ty)> = Vec::new();
            // the part of them the target type gave
            let mut target_bound: Vec<(Name, Ty)> = Vec::new();
            // variables left unknown on purpose (lambda inputs, clashes)
            let mut final_poisoned: Vec<Name> = Vec::new();
            if let Some(ta) = call.child_by_field_name("type_arguments") {
                // `Collections.<String>emptyList()`: no inference
                let given: Vec<Ty> = ta.kids().into_iter().map(|t| self.resolve_type_node(t)).collect();
                let bindings: Vec<(Name, Ty)> = if given.len() == tvars.len() { tvars.iter().cloned().zip(given).collect() } else { Vec::new() };
                for p in &mut rc.method.params {
                    p.ty = apply_subst(&p.ty, &bindings);
                }
                rc.ret = self.finish_type(&rc.method.ret, &bindings, call);
            } else if tvars.is_empty() {
                rc.ret = self.finish_type(&rc.method.ret, &[], call);
            } else {
                let mut bindings = inf.bindings.clone();
                let mut poisoned = inf.poisoned.clone();
                // a lub stands only where the target agrees (javac widens it to an
                // invariant target); without a target it is what `var` gets
                // so does a binding from a poly argument, whose type arguments
                // javac adapts to the target (`Map<Number, String> m =
                // s.collect(toMap(k -> 1, v -> ""))` makes K Number)
                let wide: Vec<Name> = inf.widenable.iter().chain(&inf.soft_vars).filter(|v| mentions(&rc.method.ret, v) && bindings.iter().any(|(k, _)| k == *v)).cloned().collect();
                let target_type = || match target {
                    Target::Full => self.expected_type(call),
                    Target::Param => self.selected_param_type(call),
                    Target::None => Ty::Unknown,
                };
                if !wide.is_empty() {
                    let exp = target_type();
                    // the target bounds the lub from above, or forces another
                    // type (an invariant `Stream<Object>`): then it is no answer
                    let lub_ret = apply_subst(&rc.method.ret, &bindings);
                    if exp.is_known() && (lub_ret.mentions_type_var() || self.conv(&lub_ret, &exp) != Conv::Yes) {
                        bindings.retain(|(k, _)| !wide.contains(k));
                        poisoned.extend(wide);
                    }
                }
                let exp = target_type();
                let captured = |v: &Name| inf.captures.iter().any(|c| c.var == *v);
                // lambdas and method references
                let args = self.args_of(call);
                for (i, a) in args.iter().enumerate() {
                    let a = a.unparen();
                    if !matches!(a.kind(), "lambda_expression" | "method_reference") {
                        continue;
                    }
                    let Some(pt0) = param_type_for_arg(&rc.method, i, args.len()) else { continue };
                    let pt = apply_subst(&pt0, &bindings);
                    let Some(sam) = self.functional_method(&pt) else { continue };
                    // its parameter types are its inputs: javac resolves their
                    // variables before the body, from the target or else from
                    // the bounds (`pick(x -> 1)` for `Function<? super T, ? extends T>`
                    // with `T extends Number` is a T of Number); the body binds none
                    let open: Vec<Name> = tvars.iter().filter(|v| !poisoned.contains(v)).cloned().collect();
                    let inputs: Vec<Name> = open.iter().filter(|v| !bindings.iter().any(|(k, _)| k == *v) && sam.method.params.iter().any(|p| mentions(&p.ty, v))).cloned().collect();
                    if !inputs.is_empty() {
                        // an explicitly typed lambda states them (`(Integer x) -> 1`
                        // makes T Integer); an exact method reference can too, and
                        // that is beyond this model: neither yields to the target
                        let declared: Option<Vec<Ty>> = (a.kind() == "lambda_expression")
                            .then(|| a.field("parameters").filter(|p| p.kind() == "formal_parameters"))
                            .flatten()
                            .map(|ps| ps.kids().into_iter().filter(|k| k.kind() == "formal_parameter").map(|k| k.field("type").map(|t| self.resolve_type_node(t)).unwrap_or(Ty::Unknown)).collect());
                        // equal only where the functional type holds the variable
                        // invariantly: `Consumer<? super T>` with `(Number x)` is T <: Number
                        let invariant = !wild_on(&pt, &inputs);
                        if let Some(declared) = declared.filter(|d| invariant && !d.is_empty() && d.len() == sam.method.params.len() && d.iter().all(|t| t.is_known() && !t.has_unknown())) {
                            for (sp, d) in sam.method.params.iter().zip(&declared) {
                                unify(self, &sp.ty, d, &inputs, &mut bindings);
                            }
                        } else if a.kind() == "lambda_expression" && a.field("parameters").is_none_or(|p| p.kind() != "formal_parameters" || p.kids().is_empty()) && exp.is_known() {
                            let before = bindings.len();
                            self.target_bind(&rc.method, &rc.method.ret, &exp, &inputs, &mut bindings);
                            target_bound.extend(bindings[before..].iter().cloned());
                        }
                        for v in &inputs {
                            if !bindings.iter().any(|(k, _)| k == v) {
                                poisoned.push(v.clone());
                            }
                        }
                    }
                    // the result as declared, before what is bound so far
                    let Some(sam0) = self.functional_method(&pt0) else { continue };
                    if !sam0.method.ret.mentions_type_var() {
                        continue;
                    }
                    let body_ty = if a.kind() == "lambda_expression" { self.lambda_result_type(a) } else { self.method_ref_result_type(a, &sam.method) };
                    // a body we cannot type may still be one more lower bound
                    if body_ty.has_unknown() && !matches!(sam0.method.ret, Ty::Void) {
                        for v in tvars.iter().filter(|v| mentions(&sam0.method.ret, v)) {
                            if !poisoned.contains(v) {
                                poisoned.push(v.clone());
                                bindings.retain(|(k, _)| k != v);
                            }
                        }
                        continue;
                    }
                    if body_ty.is_known() && body_ty != Ty::Void {
                        // the body is one more lower bound: a binding it does not
                        // fit, or a capture, would need the lub
                        let mut said = Vec::new();
                        unify(self, &sam0.method.ret, &body_ty, &tvars, &mut said);
                        for (v, x) in &said {
                            let clash = captured(v) || bindings.iter().any(|(k, b)| k == v && self.conv(x, b) != Conv::Yes);
                            if clash && !poisoned.contains(v) {
                                poisoned.push(v.clone());
                                bindings.retain(|(k, _)| k != v);
                            }
                        }
                        let open: Vec<Name> = tvars.iter().filter(|v| !poisoned.contains(v)).cloned().collect();
                        unify(self, &sam0.method.ret, &body_ty, &open, &mut bindings);
                    }
                }
                // a variable an invariant argument gives in part only (`R` of
                // `collect(groupingBy(Connection::of))` is `Map<Connection, List<?>>`):
                // that part is exact unless another parameter bounds it too or
                // the target reshapes its type arguments; a target never binds it
                for (v, t) in &inf.partial {
                    if bindings.iter().any(|(k, _)| k == v) || poisoned.contains(v) {
                        continue;
                    }
                    let once = rc.method.params.iter().filter(|p| mentions(&p.ty, v)).count() == 1;
                    let reshapes = exp.is_known() && !(matches!(&exp, Ty::Class { args, .. } if args.is_empty()) && self.conv(&t.erasure(), &exp) == Conv::Yes);
                    if once && !reshapes {
                        bindings.push((v.clone(), t.clone()));
                    } else {
                        poisoned.push(v.clone());
                    }
                }
                let open: Vec<Name> = tvars.iter().filter(|v| !poisoned.contains(v)).cloned().collect();
                // target type
                let ret_unbound = open.iter().any(|v| !bindings.iter().any(|(k, _)| k == v) && mentions(&rc.method.ret, v));
                if ret_unbound && exp.is_known() {
                    let before = bindings.len();
                    self.target_bind(&rc.method, &rc.method.ret, &exp, &open, &mut bindings);
                    target_bound.extend(bindings[before..].iter().cloned());
                }
                for p in &mut rc.method.params {
                    p.ty = apply_subst(&p.ty, &bindings);
                }
                // a captured variable shows as its projection: the wildcard
                // again, where that fits the target; an invariant target
                // (`List<Number> l = List.copyOf(xs)`) widens it in javac
                bindings.retain(|(k, _)| !inf.captures.iter().any(|c| c.var == *k));
                let mut captures = inf.captures.clone();
                captures.retain(|c| !poisoned.contains(&c.var));
                if captures.iter().any(|c| mentions(&rc.method.ret, &c.var)) {
                    let exp = target_type();
                    let projected = apply_subst(&self.project(&rc.method.ret, &captures), &bindings);
                    if exp.is_known() && (projected.mentions_type_var() || self.conv(&projected, &exp) != Conv::Yes) {
                        captures.retain(|c| !mentions(&rc.method.ret, &c.var));
                    }
                }
                let ret = self.project(&rc.method.ret, &captures);
                rc.ret = self.finish_type(&ret, &bindings, call);
                final_bindings = bindings;
                final_poisoned = poisoned;
            }
            // unchecked conversion of an argument erases the result as
            // instantiated (JLS 18.5.2): `<T> T` inferred String stays String
            if inf.unchecked {
                // a variable no argument constrains is its bound in javac
                let from_args: Vec<(Name, Ty)> = final_bindings.iter().filter(|(k, _)| !target_bound.iter().any(|(t, _)| t == k)).cloned().collect();
                let free = |v: &Name| {
                    !from_args.iter().any(|(k, _)| k == v)
                        && !inf.poisoned.contains(v)
                        && !final_poisoned.contains(v)
                        && !inf.unknown_vars.contains(v)
                        && !inf.captures.iter().any(|c| c.var == *v)
                        && !inf.upper_only.contains(v)
                };
                fn bound_of(m: &MethodInfo, v: &str, free: &dyn Fn(&Name) -> bool, depth: usize) -> Ty {
                    let Some(tp) = m.type_params.iter().find(|tp| &*tp.name == v) else { return Ty::Unknown };
                    let real: Vec<&Ty> = tp.bounds.iter().filter(|b| !b.is_class("java.lang.Object")).collect();
                    match real[..] {
                        [] => Ty::object(),
                        [Ty::TypeVar(w)] if depth < 8 && free(w) => bound_of(m, w, free, depth + 1),
                        [b] if !b.mentions_type_var() || matches!(b, Ty::Class { .. }) => b.erasure(),
                        _ => Ty::Unknown,
                    }
                }
                if let Some(tvars_free) = (!rc.method.type_params.is_empty()).then(|| rc.method.type_params.iter().filter(|tp| free(&tp.name)).map(|tp| tp.name.clone()).collect::<Vec<_>>()) {
                    if tvars_free.iter().any(|v| mentions(&sel.method.ret, v)) {
                        // with a target, javac takes the glb of target and bound
                        let bounds: Vec<(Name, Ty)> = tvars_free
                            .iter()
                            .map(|v| {
                                let b = bound_of(&rc.method, v, &free, 0);
                                let t = match target_bound.iter().find(|(k, _)| k == v) {
                                    None => b,
                                    Some(_) if b.is_unknown() => Ty::Unknown,
                                    Some((_, x)) if self.conv(x, &b) == Conv::Yes => x.clone(),
                                    Some((_, x)) if self.conv(&b, x) == Conv::Yes => b,
                                    Some(_) => Ty::Unknown,
                                };
                                (v.clone(), t)
                            })
                            .collect();
                        let t = apply_subst(&apply_subst(&sel.method.ret, &from_args), &bounds);
                        rc.ret = self.finish_type(&t, &[], call);
                    }
                }
                // a caller's type variable erases to its bound
                fn erased(t: &Ty, var_bound: &dyn Fn(&str) -> Ty) -> Ty {
                    match t {
                        Ty::Array(e) => match erased(e, var_bound) {
                            Ty::Unknown => Ty::Unknown,
                            e => Ty::Array(Box::new(e)),
                        },
                        Ty::Class { .. } => t.erasure(),
                        Ty::TypeVar(v) => match var_bound(v) {
                            b @ Ty::Class { .. } => b.erasure(),
                            _ => Ty::Unknown,
                        },
                        Ty::Prim(_) | Ty::Void => t.clone(),
                        _ => Ty::Unknown,
                    }
                }
                rc.ret = erased(&rc.ret, &|v| self.type_var_bound(v, call));
            }
            if rc.erasure_only {
                fn erasure_only(t: &Ty) -> Ty {
                    match t {
                        Ty::Class { name, args } => Ty::Class { name: name.clone(), args: args.iter().map(|_| Ty::Unknown).collect() },
                        Ty::Array(e) => match erasure_only(e) {
                            Ty::Unknown => Ty::Unknown,
                            e => Ty::Array(Box::new(e)),
                        },
                        t if t.mentions_type_var() || t.has_unknown() => Ty::Unknown,
                        t => t.clone(),
                    }
                }
                rc.ret = erasure_only(&rc.ret);
            }
            Some(Rc::new(rc))
        }
    }

    /// Bindings for `vars` from the target type `exp` of a call returning
    /// `ret`. The target bounds the result from above (JLS 18.5.2): a bare
    /// `T` gets the glb of target and declared bound, an invariant type
    /// argument fixes T, `? super X` is a lower bound X, a wildcard target
    /// argument (`List<?>`) says nothing.
    fn target_bind(&self, m: &MethodInfo, ret: &Ty, exp: &Ty, vars: &[Name], out: &mut Vec<(Name, Ty)>) {
        let free = |v: &Name, out: &Vec<(Name, Ty)>| vars.contains(v) && !out.iter().any(|(k, _)| k == v);
        match (ret, exp) {
            (Ty::TypeVar(v), _) if free(v, out) => {
                let x = exp.boxed();
                if !matches!(x, Ty::Class { .. } | Ty::Array(_)) || x.has_unknown() {
                    return;
                }
                let Some(tp) = m.type_params.iter().find(|tp| tp.name == *v) else { return };
                let real: Vec<&Ty> = tp.bounds.iter().filter(|b| !b.is_class("java.lang.Object")).collect();
                let t = match real[..] {
                    [] => Some(x),
                    [b] if !b.mentions_type_var() => match (self.conv(&x, b), self.conv(b, &x)) {
                        (Conv::Yes, _) => Some(x),
                        (_, Conv::Yes) => Some((*b).clone()),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(t) = t {
                    out.push((v.clone(), t));
                }
            }
            (Ty::Class { name, args }, Ty::Class { name: en, args: eargs }) if name == en && args.len() == eargs.len() => {
                for (ra, ea) in args.iter().zip(eargs) {
                    match (ra, ea) {
                        (_, Ty::Wildcard(None)) => {}
                        (Ty::TypeVar(v), Ty::Wildcard(Some(w))) if free(v, out) => {
                            if let WildcardBound::Super(x) = &**w {
                                if !x.has_unknown() && !x.mentions_type_var() {
                                    out.push((v.clone(), x.clone()));
                                }
                            }
                        }
                        (Ty::TypeVar(v), x) if free(v, out) && !matches!(x, Ty::Wildcard(_)) && !x.has_unknown() => out.push((v.clone(), x.clone())),
                        (Ty::Class { .. }, Ty::Class { .. }) => self.target_bind(m, ra, ea, vars, out),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// Upward projection (JLS 4.10.5) of captured type variables in a result
    /// type, the type javac gives a `var`; what it cannot express here is
    /// unknown.
    fn project(&self, t: &Ty, caps: &[Capture]) -> Ty {
        let cap = |v: &str| caps.iter().find(|c| &*c.var == v);
        let upper = |c: &Capture| match &c.bound {
            Some(b) if b.is_class("java.lang.Object") && c.lower.as_ref().is_some_and(|l| l.is_class("java.lang.Object")) => b.clone(),
            Some(b) if b.is_class("java.lang.Object") => match &c.lower {
                Some(l) => Ty::Wildcard(Some(Box::new(WildcardBound::Super(l.clone())))),
                None => Ty::Wildcard(None),
            },
            Some(b) => Ty::Wildcard(Some(Box::new(WildcardBound::Extends(b.clone())))),
            None => Ty::Unknown,
        };
        match t {
            Ty::TypeVar(v) => match cap(v) {
                Some(c) => c.bound.clone().unwrap_or(Ty::Unknown),
                None => t.clone(),
            },
            Ty::Array(e) => Ty::Array(Box::new(self.project(e, caps))),
            Ty::Class { name, args } if !args.is_empty() => {
                let decl = self.class(name);
                let args = args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        if !caps.iter().any(|c| mentions(a, &c.var)) {
                            return a.clone();
                        }
                        let free = decl.as_ref().and_then(|d| d.type_params.get(i)).is_some_and(|tp| tp.bounds.iter().all(|b| b.is_class("java.lang.Object")));
                        if !free {
                            return Ty::Unknown;
                        }
                        match a {
                            Ty::TypeVar(v) => upper(cap(v).unwrap()),
                            Ty::Wildcard(Some(w)) => match &**w {
                                WildcardBound::Extends(Ty::TypeVar(v)) if cap(v).is_some() => match upper(cap(v).unwrap()) {
                                    Ty::Wildcard(Some(b)) if matches!(&*b, WildcardBound::Super(_)) => Ty::Wildcard(None),
                                    u => u,
                                },
                                WildcardBound::Super(Ty::TypeVar(v)) if cap(v).is_some() => match &cap(v).unwrap().lower {
                                    Some(l) => Ty::Wildcard(Some(Box::new(WildcardBound::Super(l.clone())))),
                                    None => Ty::Wildcard(None),
                                },
                                _ => Ty::Unknown,
                            },
                            _ => Ty::Unknown,
                        }
                    })
                    .collect();
                Ty::Class { name: name.clone(), args }
            }
            _ => t.clone(),
        }
    }

    /// Substitute bindings; remaining method type variables become Unknown,
    /// class type variables of the enclosing context stay.
    fn finish_type(&self, t: &Ty, bindings: &[(Name, Ty)], at: Node<'t>) -> Ty {
        let t = apply_subst(t, bindings);
        let scope = self.scope_at(at);
        let t = t.subst(&|v| if scope.type_vars.iter().any(|x| &**x == v) { None } else { Some(Ty::Unknown) });
        // the result is captured (JLS 15.12.3): `? extends Object` is `?`,
        // `? super Object` is Object
        match t {
            Ty::Class { name, args } => Ty::Class {
                name,
                args: args
                    .into_iter()
                    .map(|a| match &a {
                        Ty::Wildcard(Some(w)) => match &**w {
                            WildcardBound::Extends(o) if o.is_class("java.lang.Object") => Ty::Wildcard(None),
                            WildcardBound::Super(o) if o.is_class("java.lang.Object") => o.clone(),
                            _ => a,
                        },
                        _ => a,
                    })
                    .collect(),
            },
            t => t,
        }
    }

    /// Inference of `m`'s type variables from the non-lambda arguments,
    /// each typed with the selected parameter as target.
    fn partial_inference(&self, call: Node<'t>, m: &MethodInfo) -> Inference {
        self.guard(call.id(), TAG_PARTIAL, Inference::unknown(), || {
            let args = self.args_of(call);
            let mut params = Vec::new();
            let mut tys = Vec::new();
            let poly: Vec<Poly> = args.iter().map(|a| self.poly_arg(*a)).collect();
            for (i, a) in args.iter().enumerate() {
                let a = a.unparen();
                let lambda = matches!(a.kind(), "lambda_expression" | "method_reference");
                // a call typed by its target alone adapts to the parameter
                let at = if lambda || self.poly_untyped(a) { Ty::Unknown } else { self.arg_type(a, Target::Param) };
                let Some(pt) = param_type_for_arg_typed(m, i, args.len(), &at) else { return Inference::unknown() };
                params.push(pt);
                tys.push((!lambda).then_some(at));
            }
            self.infer_args(m, &params, &tys, &poly)
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
        let b = self.partial_inference(call, &sel.method).bindings;
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
            // `ArrayList::new` infers its type arguments, it is no raw type
            if let Ty::Class { name, args } = &t {
                if args.is_empty() && self.class(name).is_some_and(|c| !c.type_params.is_empty()) {
                    return Ty::Unknown;
                }
            }
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

/// A wildcard in `t` whose bound mentions one of `vars`.
fn wild_on(t: &Ty, vars: &[Name]) -> bool {
    match t {
        Ty::Wildcard(Some(b)) => {
            let (WildcardBound::Extends(x) | WildcardBound::Super(x)) = &**b;
            vars.iter().any(|v| mentions(x, v))
        }
        Ty::Class { args, .. } => args.iter().any(|a| wild_on(a, vars)),
        Ty::Array(e) => wild_on(e, vars),
        _ => false,
    }
}

/// `t` with captured variables replaced the way the argument holds them:
/// `C<T>`, `C<? extends T>` and `C<? super T>` all contain the capture of
/// the argument's wildcard, which shows as that wildcard; a capture typed as
/// a whole (`T` for a `List<?>`) is the argument's type. Deeper uses stay.
fn with_captures(t: &Ty, caps: &[Capture]) -> Ty {
    let cap = |v: &str| caps.iter().find(|c| &*c.var == v);
    match t {
        Ty::TypeVar(v) => match cap(v) {
            Some(c) if !matches!(c.wildcard, Ty::Wildcard(_)) => c.wildcard.clone(),
            _ => t.clone(),
        },
        Ty::Class { name, args } if !args.is_empty() => Ty::Class {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| match a {
                    Ty::TypeVar(v) => cap(v).map(|c| c.wildcard.clone()).unwrap_or_else(|| a.clone()),
                    Ty::Wildcard(Some(w)) => match &**w {
                        WildcardBound::Extends(Ty::TypeVar(v)) | WildcardBound::Super(Ty::TypeVar(v)) => cap(v).map(|c| c.wildcard.clone()).unwrap_or_else(|| a.clone()),
                        _ => a.clone(),
                    },
                    a => a.clone(),
                })
                .collect(),
        },
        Ty::Array(e) => Ty::Array(Box::new(with_captures(e, caps))),
        t => t.clone(),
    }
}

/// Erasure of `t` with `m`'s type variables erased to their bound; javac's
/// result for an intersection bound is not the leftmost one: unknown.
fn erase_in(m: &MethodInfo, t: &Ty) -> Ty {
    match t {
        Ty::TypeVar(v) => match m.type_params.iter().find(|tp| tp.name == *v) {
            Some(tp) if tp.bounds.len() > 1 => Ty::Unknown,
            Some(tp) => tp.bounds.first().map(|b| b.erasure()).unwrap_or_else(Ty::object),
            None => t.clone(),
        },
        Ty::Array(e) => Ty::Array(Box::new(erase_in(m, e))),
        t => t.erasure(),
    }
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
