//! Type hierarchy queries: supertypes with generic substitution, member
//! lookup, subtyping and assignability. Written against [`ClassLookup`] so
//! the semantic layer can overlay local/anonymous classes.

use crate::doubt::{self, Doubt};
use crate::model::*;
use rustc_hash::FxHashSet;
use std::collections::VecDeque;
use std::sync::Arc;

/// What keeps a class's hierarchy from being known (see
/// [`ClassLookup::hierarchy_gap`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gap {
    /// No JDK is loaded: `java.lang.Object`, above every type, is not known.
    NoJdk,
    /// A JDK class of the hierarchy may be newer than the one loaded.
    NewerJdk(Name),
    /// A class of the hierarchy is not visible.
    Missing(Name),
    /// A supertype this class names does not resolve to a class.
    UnresolvedSupertype(Name),
    /// Lombok adds members the model does not reproduce to this class of
    /// the hierarchy, or to a class enclosing it (see `members_unknown`).
    MembersUnknown(Name),
}

/// Three-valued logic: analysis results must distinguish "no" from "don't know".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tri {
    Yes,
    No,
    Unknown,
}

impl Tri {
    pub fn is_yes(self) -> bool {
        self.asked();
        self == Tri::Yes
    }
    pub fn is_no(self) -> bool {
        self.asked();
        self == Tri::No
    }
    #[inline]
    fn asked(self) {
        if self == Tri::Unknown {
            crate::doubt::note(crate::doubt::Doubt::TriCollapsed);
        }
    }
    pub fn from_bool(b: bool) -> Tri {
        if b {
            Tri::Yes
        } else {
            Tri::No
        }
    }
    pub fn or(self, o: Tri) -> Tri {
        match (self, o) {
            (Tri::Yes, _) | (_, Tri::Yes) => Tri::Yes,
            (Tri::No, Tri::No) => Tri::No,
            _ => Tri::Unknown,
        }
    }
    pub fn and(self, o: Tri) -> Tri {
        match (self, o) {
            (Tri::No, _) | (_, Tri::No) => Tri::No,
            (Tri::Yes, Tri::Yes) => Tri::Yes,
            _ => Tri::Unknown,
        }
    }
    pub fn not(self) -> Tri {
        match self {
            Tri::Yes => Tri::No,
            Tri::No => Tri::Yes,
            Tri::Unknown => Tri::Unknown,
        }
    }
}

/// A method found through member lookup, with types substituted for the
/// receiver's type arguments.
#[derive(Clone, Debug)]
pub struct MethodRef {
    pub owner: Name,
    pub method: MethodInfo,
    pub owner_kind: ClassKind,
    pub owner_flags: Flags,
}

impl MethodRef {
    pub fn is_static(&self) -> bool {
        self.method.flags.is_static()
    }
    pub fn name(&self) -> &str {
        &self.method.name
    }
}

#[derive(Clone, Debug)]
pub struct FieldRef {
    pub owner: Name,
    pub field: FieldInfo,
}

/// Whether `class` with type arguments `args` is a raw type (JLS 4.8): a
/// generic class named without any. A class that declares no type
/// parameters is never raw.
pub fn is_raw(class: &ClassInfo, args: &[Ty]) -> bool {
    args.is_empty() && !class.type_params.is_empty()
}

/// The type of an instance method or constructor `m` of a raw type of `c`:
/// the erasure of its declared type, its own type parameters erased too
/// (JLS 4.8, 4.6), so it is no longer generic. A static method of a raw
/// type keeps its type, and is not passed here.
pub fn erase_member(m: &MethodInfo, c: &ClassInfo) -> MethodInfo {
    let mut e = m.clone();
    for p in &mut e.params {
        p.ty = erasure_among(&p.ty, &m.type_params, &c.type_params, 0);
    }
    e.ret = erasure_among(&m.ret, &m.type_params, &c.type_params, 0);
    e.type_params.clear();
    e
}

/// The capture of an unbounded wildcard for a parameter with one bound
/// stands for that bound: with type arguments of its own, which this model
/// does not compute (`T extends Comparable<T>` makes it a
/// `Comparable<CAP#1>`), they are unknown, never erased into a raw type.
fn bound_of_capture(b: &Ty) -> Ty {
    match b {
        Ty::Class { name, args } => Ty::Class { name: name.clone(), args: args.iter().map(|_| Ty::Unknown).collect() },
        b => b.erasure(),
    }
}

/// Map from a class's type parameters to the receiver's arguments. For a
/// raw type each parameter maps to Unknown: member lookup erases the
/// members of a raw type itself (`erase_member`), and a caller that
/// substitutes with this map learns nothing definite from it.
pub fn substitution(class: &ClassInfo, args: &[Ty]) -> Vec<(Name, Ty)> {
    if class.type_params.is_empty() {
        return Vec::new();
    }
    if args.len() == class.type_params.len() {
        class
            .type_params
            .iter()
            .zip(args)
            .map(|(p, a)| {
                // the capture of a wildcard is bounded by the parameter's bounds
                // too: with several, it is their intersection (JLS 4.9, 5.1.10),
                // which one type cannot stand for, so members of the others
                // would look missing: unknown
                let a = match a {
                    Ty::Wildcard(w) if p.bounds.len() > 1 && !matches!(w.as_deref(), Some(WildcardBound::Super(_))) => Ty::Unknown,
                    Ty::Wildcard(Some(b)) => match &**b {
                        WildcardBound::Extends(t) => t.clone(),
                        WildcardBound::Super(t) => t.clone(),
                    },
                    Ty::Wildcard(None) => p.bounds.first().map(bound_of_capture).unwrap_or_else(Ty::object),
                    t => t.clone(),
                };
                (p.name.clone(), a)
            })
            .collect()
    } else {
        // a raw type, or a count of arguments that does not match (broken
        // code): no argument is known
        class.type_params.iter().map(|p| (p.name.clone(), Ty::Unknown)).collect()
    }
}

pub fn apply_subst(t: &Ty, map: &[(Name, Ty)]) -> Ty {
    if map.is_empty() {
        return t.clone();
    }
    t.subst(&|n| map.iter().find(|(k, _)| &**k == n).map(|(_, v)| v.clone()))
}

fn subst_method(m: &MethodInfo, map: &[(Name, Ty)]) -> MethodInfo {
    if map.is_empty() {
        return m.clone();
    }
    // method type params shadow class params
    let map: Vec<(Name, Ty)> = map.iter().filter(|(k, _)| !m.type_params.iter().any(|tp| tp.name == *k)).cloned().collect();
    let mut m = m.clone();
    for p in &mut m.params {
        p.ty = apply_subst(&p.ty, &map);
    }
    m.ret = apply_subst(&m.ret, &map);
    for tp in &mut m.type_params {
        for b in &mut tp.bounds {
            *b = apply_subst(b, &map);
        }
    }
    m
}

/// The type of method `m` of class `c` as a member of a type of `c` whose
/// arguments give `map`: erased for an instance method of a raw type
/// (`raw`, JLS 4.8), else substituted.
fn member_type(m: &MethodInfo, c: &ClassInfo, raw: bool, map: &[(Name, Ty)]) -> MethodInfo {
    if raw && !m.flags.is_static() {
        erase_member(m, c)
    } else {
        subst_method(m, map)
    }
}

/// Does `sub` (declared in a subtype) override `sup`, declared in `sup_class`
/// and inherited through the supertype `sup_class<sup_args>` (arguments in
/// terms of the subtype)? JLS 8.4.2: `sub`'s signature must be the same as
/// `sup`'s after substituting the supertype's arguments (and renaming method
/// type variables), or the same as the erasure of `sup`'s signature. Equal
/// erasures alone are not enough: `remove(E)` with `E extends Node<E>`
/// overloads `remove(Object)`. Unknown when our types cannot tell.
pub fn is_subsignature(sub: &MethodInfo, sup: &MethodInfo, sup_class: &ClassInfo, sup_args: &[Ty]) -> Tri {
    if sub.name != sup.name || sub.params.len() != sup.params.len() {
        return Tri::No;
    }
    let all = |f: &dyn Fn(&Ty, &Ty) -> Tri| sub.params.iter().zip(&sup.params).fold(Tri::Yes, |acc, (a, b)| acc.and(f(&a.ty, &b.ty)));
    let same = if sub.type_params.len() == sup.type_params.len() {
        let adapted = subst_method(sup, &substitution(sup_class, sup_args));
        let rename: Vec<(Name, Ty)> = sup.type_params.iter().zip(&sub.type_params).map(|(a, b)| (a.name.clone(), Ty::TypeVar(b.name.clone()))).collect();
        sub.params.iter().zip(&adapted.params).zip(&sup.params).fold(Tri::Yes, |acc, ((a, b), declared)| {
            // a type variable of an enclosing class (`Outer<T>.Inner.m(T)`)
            // is not substituted through the supertype: unknown
            let same = if mentions_foreign_var(&declared.ty, sup, sup_class) { Tri::Unknown } else { same_type(&a.ty, &apply_subst(&b.ty, &rename)) };
            acc.and(same)
        })
    } else {
        Tri::No
    };
    let erased = if sub.type_params.is_empty() { all(&|a, b| same_type(a, &erasure_in(b, sup, sup_class, 0))) } else { Tri::No };
    same.or(erased)
}

fn mentions_foreign_var(t: &Ty, m: &MethodInfo, c: &ClassInfo) -> bool {
    match t {
        Ty::TypeVar(n) => !m.type_params.iter().chain(&c.type_params).any(|p| p.name == *n),
        Ty::Class { args, .. } => args.iter().any(|a| mentions_foreign_var(a, m, c)),
        Ty::Array(e) => mentions_foreign_var(e, m, c),
        Ty::Wildcard(Some(b)) => match &**b {
            WildcardBound::Extends(t) | WildcardBound::Super(t) => mentions_foreign_var(t, m, c),
        },
        _ => false,
    }
}

/// Erasure of a type in `m`'s declaration: type variables erase to their
/// first bound (JLS 4.6).
fn erasure_in(t: &Ty, m: &MethodInfo, c: &ClassInfo, depth: u32) -> Ty {
    erasure_among(t, &m.type_params, &c.type_params, depth)
}

/// Erasure of a type declared where the type parameters `own` (a method's)
/// and `class` (its class's) are in scope. A type variable declared
/// elsewhere (an enclosing class's) is unknown.
fn erasure_among(t: &Ty, own: &[TypeParam], class: &[TypeParam], depth: u32) -> Ty {
    match t {
        Ty::TypeVar(n) => {
            let Some(tp) = own.iter().chain(class).find(|p| p.name == *n) else { return Ty::Unknown };
            match tp.bounds.first() {
                None => Ty::object(),
                Some(_) if depth > 8 => Ty::Unknown,
                Some(b) => erasure_among(b, own, class, depth + 1),
            }
        }
        Ty::Array(e) => match erasure_among(e, own, class, depth) {
            Ty::Unknown => Ty::Unknown,
            e => Ty::Array(Box::new(e)),
        },
        t => t.erasure(),
    }
}

/// Are two parameter types the same type? Type arguments are compared only
/// loosely (a difference there is Unknown): a compiling override never has
/// two parameter types with equal erasure but different arguments, so such a
/// difference is a gap in our model rather than an overload.
fn same_type(a: &Ty, b: &Ty) -> Tri {
    match (a, b) {
        (Ty::Unknown, _) | (_, Ty::Unknown) => Tri::Unknown,
        (Ty::TypeVar(x), Ty::TypeVar(y)) => {
            if x == y {
                Tri::Yes
            } else {
                Tri::Unknown
            }
        }
        (Ty::Class { name: x, args: xa }, Ty::Class { name: y, args: ya }) => {
            if x != y {
                Tri::No
            } else if xa.len() == ya.len() && xa.iter().zip(ya).all(|(p, q)| same_arg(p, q)) {
                Tri::Yes
            } else {
                Tri::Unknown
            }
        }
        (Ty::Array(x), Ty::Array(y)) => same_type(x, y),
        (Ty::Wildcard(_), _) | (_, Ty::Wildcard(_)) => Tri::Unknown,
        _ => Tri::from_bool(a == b),
    }
}

fn same_arg(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Wildcard(x), Ty::Wildcard(y)) => match (x.as_deref(), y.as_deref()) {
            (None, None) => true,
            (Some(WildcardBound::Extends(t)), None) | (None, Some(WildcardBound::Extends(t))) => t.is_class("java.lang.Object"),
            (Some(WildcardBound::Extends(p)), Some(WildcardBound::Extends(q))) | (Some(WildcardBound::Super(p)), Some(WildcardBound::Super(q))) => {
                same_type(p, q) == Tri::Yes
            }
            _ => false,
        },
        // not `is_yes`: a false here is Unknown in `same_type`, not a collapse
        _ => same_type(a, b) == Tri::Yes,
    }
}

pub trait ClassLookup {
    fn class(&self, name: &str) -> Option<Arc<ClassInfo>>;

    /// Whether a JDK class's API may be newer than the one loaded: members
    /// absent from it may exist (see `Classpath::may_lack_members`).
    fn may_lack_members(&self, _name: &str) -> bool {
        false
    }

    /// Direct supertypes of a class type with type arguments substituted.
    fn direct_supertypes(&self, ty: &Ty) -> Vec<Ty> {
        let Ty::Class { name, args } = ty else {
            if let Ty::Array(_) = ty {
                return vec![Ty::object(), Ty::class("java.lang.Cloneable"), Ty::class("java.io.Serializable")];
            }
            return Vec::new();
        };
        let Some(c) = self.class(name) else {
            doubt::note(Doubt::IncompleteHierarchy);
            return Vec::new();
        };
        // the supertypes of a raw type are the erasures of the declared ones
        // (JLS 4.8): raw too, where generic
        let raw = is_raw(&c, args);
        let map = substitution(&c, args);
        let mut out = Vec::new();
        for s in c.superclass.iter().chain(&c.interfaces) {
            out.push(if raw { s.erasure() } else { apply_subst(s, &map) });
        }
        if out.is_empty() && &*c.name != "java.lang.Object" {
            out.push(Ty::object());
        }
        out
    }

    /// All supertypes including `ty` itself, breadth first, deduplicated by
    /// class name.
    fn all_supertypes(&self, ty: &Ty) -> Vec<Ty> {
        let mut out = Vec::new();
        let mut seen = FxHashSet::default();
        let mut q = VecDeque::new();
        q.push_back(ty.clone());
        while let Some(t) = q.pop_front() {
            let Some(n) = t.class_name() else { continue };
            if !seen.insert(Arc::<str>::from(n)) {
                continue;
            }
            for s in self.direct_supertypes(&t) {
                q.push_back(s);
            }
            out.push(t);
            if out.len() > 200 {
                doubt::note(Doubt::IncompleteHierarchy);
                break;
            }
        }
        out
    }

    /// View `ty` as its supertype `target` with type arguments (e.g.
    /// `ArrayList<String>` as `java.lang.Iterable` -> `Iterable<String>`).
    fn as_super(&self, ty: &Ty, target: &str) -> Option<Ty> {
        if ty.class_name() == Some(target) {
            return Some(ty.clone());
        }
        self.find_supertype(ty, &mut |t| t.class_name() == Some(target))
    }

    /// The first supertype of `ty` (itself included, in the order of
    /// `all_supertypes`) that `f` accepts. It walks no further: what it
    /// cannot see past the answer cannot change it, and is not noted.
    fn find_supertype(&self, ty: &Ty, f: &mut dyn FnMut(&Ty) -> bool) -> Option<Ty> {
        let mut seen = FxHashSet::default();
        let mut q = VecDeque::new();
        q.push_back(ty.clone());
        let mut walked = 0;
        while let Some(t) = q.pop_front() {
            let Some(n) = t.class_name() else { continue };
            if !seen.insert(Arc::<str>::from(n)) {
                continue;
            }
            if f(&t) {
                return Some(t);
            }
            walked += 1;
            if walked > 200 {
                doubt::note(Doubt::IncompleteHierarchy);
                break;
            }
            q.extend(self.direct_supertypes(&t));
        }
        None
    }

    /// Is class `sub` a subclass/implementor of `sup` (reflexive)?
    fn is_subclass(&self, sub: &str, sup: &str) -> Tri {
        // what it cannot find makes the answer Unknown: not a collapse
        doubt::quiet(|| self.is_subclass_walk(sub, sup))
    }

    fn is_subclass_walk(&self, sub: &str, sup: &str) -> Tri {
        if sub == sup || sup == "java.lang.Object" {
            return Tri::Yes;
        }
        // nothing but itself can be a subtype of a final class
        if let Some(sc) = self.class(sup) {
            if sc.flags.is_final() && !sc.is_interface() {
                return Tri::No;
            }
        }
        let mut seen = FxHashSet::default();
        let mut q = VecDeque::new();
        q.push_back(Arc::<str>::from(sub));
        let mut incomplete = false;
        while let Some(n) = q.pop_front() {
            if !seen.insert(n.clone()) {
                continue;
            }
            if &*n == sup {
                return Tri::Yes;
            }
            match self.class(&n) {
                Some(c) => {
                    if let Some(s) = &c.superclass {
                        match s.class_name() {
                            Some(sn) => q.push_back(Arc::from(sn)),
                            None => incomplete = true,
                        }
                    }
                    for i in &c.interfaces {
                        match i.class_name() {
                            Some(sn) => q.push_back(Arc::from(sn)),
                            None => incomplete = true,
                        }
                    }
                }
                None => incomplete = true,
            }
        }
        if incomplete {
            Tri::Unknown
        } else {
            Tri::No
        }
    }

    fn is_subtype_ty(&self, sub: &Ty, sup_class: &str) -> Tri {
        match sub {
            Ty::Class { name, .. } => self.is_subclass(name, sup_class),
            Ty::Array(_) => Tri::from_bool(matches!(sup_class, "java.lang.Object" | "java.lang.Cloneable" | "java.io.Serializable")),
            Ty::Null => Tri::Yes,
            Ty::Prim(p) => self.is_subclass(p.box_name(), sup_class),
            _ => Tri::Unknown,
        }
    }

    /// Assignment/invocation compatibility (JLS 5.2/5.3), generics erased.
    fn is_assignable(&self, from: &Ty, to: &Ty) -> Tri {
        match (from, to) {
            (Ty::Unknown, _) | (_, Ty::Unknown) => Tri::Unknown,
            (_, Ty::TypeVar(_)) | (Ty::TypeVar(_), _) => Tri::Unknown,
            (_, Ty::Wildcard(_)) | (Ty::Wildcard(_), _) => Tri::Unknown,
            (Ty::Null, t) => Tri::from_bool(t.is_reference()),
            (Ty::Prim(a), Ty::Prim(b)) => Tri::from_bool(a.widens_to(*b)),
            (Ty::Prim(a), Ty::Class { name, .. }) => self.is_subclass(a.box_name(), name),
            (Ty::Class { name, .. }, Ty::Prim(b)) => match Prim::from_box_name(name) {
                Some(a) => Tri::from_bool(a.widens_to(*b)),
                None => Tri::No,
            },
            (Ty::Class { name: a, .. }, Ty::Class { name: b, .. }) => self.is_subclass(a, b),
            (Ty::Array(_), Ty::Class { name, .. }) => {
                Tri::from_bool(matches!(&**name, "java.lang.Object" | "java.lang.Cloneable" | "java.io.Serializable"))
            }
            (Ty::Array(a), Ty::Array(b)) => match (&**a, &**b) {
                (Ty::Prim(x), Ty::Prim(y)) => Tri::from_bool(x == y),
                (Ty::Prim(_), _) | (_, Ty::Prim(_)) => Tri::No,
                (x, y) => self.is_assignable(x, y),
            },
            (Ty::Class { .. }, Ty::Array(_)) => Tri::No,
            (Ty::Prim(_), Ty::Array(_)) | (Ty::Array(_), Ty::Prim(_)) => Tri::No,
            (Ty::Void, _) | (_, Ty::Void) => Tri::No,
            _ => Tri::Unknown,
        }
    }

    /// All supertypes including `ty` itself, each before its own supertypes
    /// (breadth first otherwise), with the names of the types in the list
    /// that are its proper subtypes.
    fn supertypes_ordered(&self, ty: &Ty) -> Vec<(Ty, FxHashSet<Name>)> {
        // the closure, breadth first, with its edges
        let mut types: Vec<Ty> = Vec::new();
        let mut index: rustc_hash::FxHashMap<Name, usize> = rustc_hash::FxHashMap::default();
        let mut edges: Vec<Vec<Name>> = Vec::new();
        let mut q = VecDeque::new();
        q.push_back(ty.clone());
        while let Some(t) = q.pop_front() {
            let Some(n) = t.class_name() else { continue };
            if index.contains_key(n) {
                continue;
            }
            if types.len() > 200 {
                doubt::note(Doubt::IncompleteHierarchy);
                continue;
            }
            index.insert(Arc::from(n), types.len());
            let direct = self.direct_supertypes(&t);
            edges.push(direct.iter().filter_map(|d| d.class_name().map(Arc::<str>::from)).collect());
            q.extend(direct);
            types.push(t);
        }
        // Kahn's algorithm: a type once all its subtypes in the list are placed
        let mut pending = vec![0usize; types.len()];
        for es in &edges {
            for e in es {
                if let Some(&k) = index.get(e) {
                    pending[k] += 1;
                }
            }
        }
        let mut subs: Vec<FxHashSet<Name>> = vec![FxHashSet::default(); types.len()];
        let mut ready: std::collections::BTreeSet<usize> = (0..types.len()).filter(|&k| pending[k] == 0).collect();
        let mut order = Vec::with_capacity(types.len());
        while let Some(k) = ready.pop_first() {
            order.push(k);
            let mut below = subs[k].clone();
            below.insert(Arc::from(types[k].class_name().unwrap_or("")));
            for e in &edges[k] {
                if let Some(&s) = index.get(e) {
                    subs[s].extend(below.iter().cloned());
                    pending[s] -= 1;
                    if pending[s] == 0 {
                        ready.insert(s);
                    }
                }
            }
        }
        // a cycle (broken code) leaves types unplaced: breadth first
        order.extend((0..types.len()).filter(|k| !order.contains(k)).collect::<Vec<_>>());
        let mut types: Vec<Option<Ty>> = types.into_iter().map(Some).collect();
        order.into_iter().map(|k| (types[k].take().unwrap(), std::mem::take(&mut subs[k]))).collect()
    }

    /// Methods named `name` visible on `ty` (declared and inherited), most
    /// derived first; overridden methods are removed. A method overrides
    /// another only from a subtype of the other's class (JLS 8.4.8.1); of
    /// override-equivalent methods inherited from unrelated supertypes, the
    /// one a call selects is kept (JLS 8.4.8.4, 15.12.2.5).
    fn methods_named(&self, ty: &Ty, name: &str) -> Vec<MethodRef> {
        let mut out: Vec<MethodRef> = Vec::new();
        let mut sups = self.supertypes_ordered(ty);
        let is_iface = ty.class_name().and_then(|n| self.class(n)).is_some_and(|c| c.is_interface());
        if is_iface && !sups.iter().any(|(t, _)| t.is_class("java.lang.Object")) {
            let all: FxHashSet<Name> = sups.iter().filter_map(|(t, _)| t.class_name().map(Arc::<str>::from)).collect();
            sups.push((Ty::object(), all));
        }
        for (st, below) in sups {
            let Ty::Class { name: cn, args } = &st else { continue };
            let Some(c) = self.class(cn) else {
                doubt::note(Doubt::IncompleteHierarchy);
                continue;
            };
            let map = substitution(&c, args);
            let raw = is_raw(&c, args);
            for m in c.methods.iter().filter(|m| &*m.name == name) {
                let sm = member_type(m, &c, raw, &map);
                let erased: Vec<Ty> = sm.params.iter().map(|p| p.ty.erasure()).collect();
                // methods of one class are distinct overloads; an unknown
                // parameter type cannot prove an override either
                let overridden = out.iter().any(|o| {
                    below.contains(&o.owner)
                        && match is_subsignature(&o.method, m, &c, args) {
                            Tri::Yes => true,
                            Tri::No => false,
                            Tri::Unknown => {
                                o.method.params.len() == erased.len()
                                    && o.method.params.iter().zip(&erased).all(|(p, e)| p.ty.erasure().same(e) == Tri::Yes)
                            }
                        }
                });
                if overridden {
                    continue;
                }
                out.push(MethodRef { owner: cn.clone(), method: sm, owner_kind: c.kind, owner_flags: c.flags });
            }
        }
        self.merge_inherited(out)
    }

    /// Of instance methods with the same erased parameters inherited from
    /// supertypes that do not override each other, keep the one a call
    /// selects: the one that is not abstract (a class's over interface
    /// defaults), else the one whose return type is a subtype of the others'
    /// (JLS 8.4.8, 8.4.8.4, 15.12.2.5); when none is, its return type is
    /// unknown.
    fn merge_inherited(&self, mut out: Vec<MethodRef>) -> Vec<MethodRef> {
        let key = |m: &MethodRef| -> Option<Vec<Ty>> {
            let ps: Vec<Ty> = m.method.params.iter().map(|p| p.ty.erasure()).collect();
            // a private or static method is not inherited: never in a group
            (!m.is_static() && !m.method.flags.is_private() && !m.method.is_constructor() && m.method.type_params.is_empty() && !ps.iter().any(|p| p.has_unknown())).then_some(ps)
        };
        let mut i = 0;
        while i < out.len() {
            let Some(k) = key(&out[i]) else {
                i += 1;
                continue;
            };
            let group: Vec<usize> = (i..out.len()).filter(|&j| j == i || (out[j].owner != out[i].owner && key(&out[j]).as_ref() == Some(&k) && out[j].method.is_varargs() == out[i].method.is_varargs())).collect();
            if group.len() < 2 {
                i += 1;
                continue;
            }
            let abstract_ = |m: &MethodRef| m.method.flags.is_abstract();
            let concrete: Vec<usize> = group.iter().copied().filter(|&j| !abstract_(&out[j])).collect();
            // a concrete method of a class wins over interface defaults: they
            // are not inherited beside it (JLS 8.4.8)
            let from_class: Vec<usize> = concrete.iter().copied().filter(|&j| out[j].owner_kind != ClassKind::Interface).collect();
            let keep = match (concrete.as_slice(), from_class.as_slice()) {
                ([one], _) => Some(*one),
                (_, [one]) => Some(*one),
                ([], _) => group.iter().copied().find(|&j| group.iter().all(|&o| o == j || self.return_substitutable(&out[j].method.ret, &out[o].method.ret))),
                // several defaults: javac rejects the class unless it
                // overrides them, which it then does
                _ => None,
            };
            let mut kept = out[keep.unwrap_or(i)].clone();
            if keep.is_none() {
                kept.method.ret = Ty::Unknown;
            }
            for &j in group.iter().rev() {
                out.remove(j);
            }
            out.insert(i, kept);
            i += 1;
        }
        out
    }

    /// Whether return type `a` may stand for `b`: the same type, or a
    /// subtype for reference types (JLS 8.4.8.3).
    fn return_substitutable(&self, a: &Ty, b: &Ty) -> bool {
        if a == b {
            return true;
        }
        match (a, b) {
            (Ty::Class { name: x, args: xa }, Ty::Class { name: y, .. }) => {
                if x == y {
                    return false;
                }
                // generic: the arguments must agree through the supertype
                match self.as_super(a, y) {
                    Some(v) => xa.iter().all(|t| !t.has_unknown()) && v == *b || (b.args().is_empty() && self.is_subclass(x, y).is_yes()),
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// Every method on `ty` including inherited ones (overrides removed).
    fn all_methods(&self, ty: &Ty) -> Vec<MethodRef> {
        let mut out: Vec<MethodRef> = Vec::new();
        for (st, _) in self.supertypes_ordered(ty) {
            let Ty::Class { name: cn, args } = &st else { continue };
            let Some(c) = self.class(cn) else {
                doubt::note(Doubt::IncompleteHierarchy);
                continue;
            };
            let map = substitution(&c, args);
            let raw = is_raw(&c, args);
            for m in &c.methods {
                if m.is_constructor() {
                    continue;
                }
                let sm = member_type(m, &c, raw, &map);
                // also a method of an unrelated supertype with the same
                // signature: one entry per signature, the most derived first
                let dup = out.iter().any(|o| {
                    o.owner != *cn
                        && match is_subsignature(&o.method, m, &c, args) {
                            Tri::Yes => true,
                            Tri::No => false,
                            Tri::Unknown => {
                                o.method.params.len() == sm.params.len()
                                    && o.method.params.iter().zip(&sm.params).all(|(a, b)| a.ty.erasure().same(&b.ty.erasure()) == Tri::Yes)
                            }
                        }
                });
                if !dup {
                    out.push(MethodRef { owner: cn.clone(), method: sm, owner_kind: c.kind, owner_flags: c.flags });
                }
            }
        }
        out
    }

    fn field(&self, ty: &Ty, name: &str) -> Option<FieldRef> {
        let mut found = None;
        self.find_supertype(ty, &mut |st| {
            let Ty::Class { name: cn, args } = st else { return false };
            let Some(c) = self.class(cn) else {
                doubt::note(Doubt::IncompleteHierarchy);
                return false;
            };
            let Some(f) = c.field(name) else { return false };
            let mut f = f.clone();
            // a non-static field of a raw type has the erasure of its type
            // (JLS 4.8); a static one keeps it
            f.ty = if is_raw(&c, args) && !f.flags.is_static() { erasure_among(&f.ty, &[], &c.type_params, 0) } else { apply_subst(&f.ty, &substitution(&c, args)) };
            found = Some(FieldRef { owner: cn.clone(), field: f });
            true
        });
        found
    }

    /// Whether the model may lack members of class `name`: an annotation
    /// processor adds members to it that the Lombok model does not reproduce
    /// (`Flags::MEMBERS_UNKNOWN`: `@Delegate`, `@Accessors`...), or to a
    /// class enclosing it. An enclosing class counts because Lombok reads
    /// `@Accessors` from the enclosing classes too, which renames the
    /// accessors it generates in the nested class.
    fn members_unknown(&self, name: &str) -> bool {
        let mut k = self.class(name);
        while let Some(c) = k {
            if c.flags.has(Flags::MEMBERS_UNKNOWN) {
                return true;
            }
            k = c.outer.as_ref().and_then(|o| self.class(o));
        }
        false
    }

    /// Whether every member of `name` and of its supertypes is known: every
    /// supertype is (see `supertypes_known`), and no class of the closure has
    /// members the model does not know (see `members_unknown`).
    fn hierarchy_complete(&self, name: &str) -> bool {
        // asking whether it is complete is handling the unknown
        doubt::quiet(|| self.hierarchy_gap(name, true)).is_none()
    }

    /// Whether every supertype of `name` is known, whatever members its
    /// classes have that the model does not: enough for a question of
    /// subtyping (is it `Serializable`, may a bean be injected as `T`), which
    /// the members Lombok adds do not change. Every type has java.lang.Object
    /// above it (interfaces too: they have its methods), so without a JDK
    /// nothing is known. A JDK class newer than the one loaded (the project's
    /// release is newer than the JDK) may have members and supertypes we
    /// cannot see (`List` is a `SequencedCollection` from Java 21 on), so it
    /// is not known either.
    fn supertypes_known(&self, name: &str) -> bool {
        doubt::quiet(|| self.hierarchy_gap(name, false)).is_none()
    }

    /// What keeps the hierarchy of `name` from being known: a supertype,
    /// and with `members` also a member of a class in it. None: nothing.
    fn hierarchy_gap(&self, name: &str, members: bool) -> Option<Gap> {
        if self.class("java.lang.Object").is_none() {
            return Some(Gap::NoJdk);
        }
        let mut seen = FxHashSet::default();
        let mut q = VecDeque::new();
        q.push_back(Arc::<str>::from(name));
        while let Some(n) = q.pop_front() {
            if !seen.insert(n.clone()) {
                continue;
            }
            if &*n != "java.lang.Object" && self.may_lack_members(&n) {
                return Some(Gap::NewerJdk(n));
            }
            let Some(c) = self.class(&n) else { return Some(Gap::Missing(n)) };
            if members && self.members_unknown(&n) {
                return Some(Gap::MembersUnknown(n));
            }
            for t in c.superclass.iter().chain(c.interfaces.iter()) {
                match t.class_name() {
                    Some(sn) => q.push_back(Arc::from(sn)),
                    None => return Some(Gap::UnresolvedSupertype(n)),
                }
            }
        }
        None
    }

    /// Does the class (or a superclass other than Object) declare `name(params)`?
    fn overrides_object_method(&self, class: &str, method: &str, arity: usize) -> bool {
        let ty = Ty::class(class);
        self.methods_named(&ty, method).iter().any(|m| m.method.params.len() == arity && &*m.owner != "java.lang.Object" && !m.method.flags.is_abstract())
    }

    /// Whether `ty` is a functional interface (JLS 9.8), with its single
    /// abstract method substituted. An interface whose supertypes are not
    /// all known may have another abstract method, or fewer: unknown. A
    /// type variable or wildcard stands here for a type the model has not
    /// inferred: unknown too.
    fn functional_method(&self, ty: &Ty) -> Functional {
        let n = match ty {
            Ty::Class { name, .. } => name,
            Ty::Unknown | Ty::NoType | Ty::TypeVar(_) | Ty::Wildcard(_) => return Functional::Unknown,
            _ => return Functional::Not,
        };
        // a gap the answer rests on makes it unknown: handled here, not noted
        let (answer, d) = doubt::observe(|| {
            let Some(c) = self.class(n) else { return Functional::Unknown };
            if !c.is_interface() {
                return Functional::Not;
            }
            // whether it is one does not depend on its type arguments: decided
            // on the generic declaration, where an unknown argument cannot hide
            // that one method overrides another (`Set<?>.add` and
            // `Collection<?>.add`), and the method then taken from `ty`
            let ((declared, substituted), gaps) = doubt::observe(|| (self.all_methods(&c.this_type()), self.all_methods(ty)));
            if gaps.contains(Doubt::IncompleteHierarchy) || gaps.contains(Doubt::MissingClass) || gaps.contains(Doubt::UnknownType) {
                return Functional::Unknown;
            }
            doubt::replay(gaps);
            let abstract_of = |ms: Vec<MethodRef>| -> Vec<MethodRef> {
                ms.into_iter()
                    .filter(|m| m.method.flags.is_abstract() && !m.method.flags.is_static())
                    // methods of Object don't count
                    .filter(|m| !matches!((&*m.method.name, m.method.params.len()), ("equals", 1) | ("hashCode", 0) | ("toString", 0)))
                    .collect()
            };
            let declared = abstract_of(declared);
            // two of the same name and arity may be one, overridden, where a
            // declared parameter type is unknown and `all_methods` could not tell
            let maybe_one = |a: &MethodRef, b: &MethodRef| a.method.name == b.method.name && a.method.params.len() == b.method.params.len() && a.method.params.iter().chain(&b.method.params).any(|p| p.ty.has_unknown());
            let undecided = declared.iter().enumerate().any(|(i, a)| declared[i + 1..].iter().any(|b| maybe_one(a, b)));
            match declared.as_slice() {
                [one] => abstract_of(substituted)
                    .into_iter()
                    .find(|m| m.owner == one.owner && m.method.name == one.method.name && m.method.params.len() == one.method.params.len())
                    .map_or(Functional::Unknown, Functional::Method),
                _ if undecided => Functional::Unknown,
                _ => Functional::Not,
            }
        });
        if !matches!(answer, Functional::Unknown) {
            doubt::replay(d);
        }
        answer
    }
}

/// Whether a type is a functional interface: `ClassLookup::functional_method`.
#[derive(Clone, Debug)]
pub enum Functional {
    /// It is one: its single abstract method, substituted.
    Method(MethodRef),
    /// It certainly is not one: not an interface, or an interface whose
    /// supertypes are all known that has not exactly one abstract method.
    Not,
    /// The type, its class or one of its supertypes is not known.
    Unknown,
}
