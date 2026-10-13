//! Type hierarchy queries: supertypes with generic substitution, member
//! lookup, subtyping and assignability. Written against [`ClassLookup`] so
//! the semantic layer can overlay local/anonymous classes.

use crate::model::*;
use rustc_hash::FxHashSet;
use std::collections::VecDeque;
use std::sync::Arc;

/// Three-valued logic: analysis results must distinguish "no" from "don't know".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tri {
    Yes,
    No,
    Unknown,
}

impl Tri {
    pub fn is_yes(self) -> bool {
        self == Tri::Yes
    }
    pub fn is_no(self) -> bool {
        self == Tri::No
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

/// Map from a class's type parameters to the receiver's arguments.
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
                let a = match a {
                    Ty::Wildcard(Some(b)) => match &**b {
                        WildcardBound::Extends(t) => t.clone(),
                        WildcardBound::Super(t) => t.clone(),
                    },
                    Ty::Wildcard(None) => p.bounds.first().map(|b| b.erasure()).unwrap_or_else(Ty::object),
                    t => t.clone(),
                };
                (p.name.clone(), a)
            })
            .collect()
    } else {
        // missing type arguments: javac erases a raw type to the bounds, but
        // here they are far more often arguments we could not infer, where
        // the bound is wrong (a lambda over `ColumnFamily<DbLong, ..>` would
        // see `DbKey`): unknown, so nothing is concluded from them
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

pub trait ClassLookup {
    fn class(&self, name: &str) -> Option<Arc<ClassInfo>>;

    /// Direct supertypes of a class type with type arguments substituted.
    fn direct_supertypes(&self, ty: &Ty) -> Vec<Ty> {
        let Ty::Class { name, args } = ty else {
            if let Ty::Array(_) = ty {
                return vec![Ty::object(), Ty::class("java.lang.Cloneable"), Ty::class("java.io.Serializable")];
            }
            return Vec::new();
        };
        let Some(c) = self.class(name) else { return Vec::new() };
        let map = substitution(&c, args);
        let mut out = Vec::new();
        if let Some(s) = &c.superclass {
            out.push(apply_subst(s, &map));
        }
        for i in &c.interfaces {
            out.push(apply_subst(i, &map));
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
        self.all_supertypes(ty).into_iter().find(|t| t.class_name() == Some(target))
    }

    /// Is class `sub` a subclass/implementor of `sup` (reflexive)?
    fn is_subclass(&self, sub: &str, sup: &str) -> Tri {
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

    /// Methods named `name` visible on `ty` (declared and inherited), most
    /// derived first; overridden methods are removed.
    fn methods_named(&self, ty: &Ty, name: &str) -> Vec<MethodRef> {
        let mut out: Vec<MethodRef> = Vec::new();
        let mut sups = self.all_supertypes(ty);
        let is_iface = ty.class_name().and_then(|n| self.class(n)).is_some_and(|c| c.is_interface());
        if is_iface && !sups.iter().any(|t| t.is_class("java.lang.Object")) {
            sups.push(Ty::object());
        }
        for st in sups {
            let Ty::Class { name: cn, args } = &st else { continue };
            let Some(c) = self.class(cn) else { continue };
            let map = substitution(&c, args);
            for m in c.methods.iter().filter(|m| &*m.name == name) {
                let sm = subst_method(m, &map);
                let erased: Vec<Ty> = sm.params.iter().map(|p| p.ty.erasure()).collect();
                // methods of one class are distinct overloads; an unknown
                // parameter type cannot prove an override either
                let overridden = out.iter().any(|o| {
                    o.owner != *cn
                        && o.method.params.len() == erased.len()
                        && o.method.params.iter().zip(&erased).all(|(p, e)| !e.has_unknown() && p.ty.erasure() == *e)
                });
                if overridden {
                    continue;
                }
                out.push(MethodRef { owner: cn.clone(), method: sm, owner_kind: c.kind, owner_flags: c.flags });
            }
        }
        out
    }

    /// Every method on `ty` including inherited ones (overrides removed).
    fn all_methods(&self, ty: &Ty) -> Vec<MethodRef> {
        let mut out: Vec<MethodRef> = Vec::new();
        for st in self.all_supertypes(ty) {
            let Ty::Class { name: cn, args } = &st else { continue };
            let Some(c) = self.class(cn) else { continue };
            let map = substitution(&c, args);
            for m in &c.methods {
                if m.is_constructor() {
                    continue;
                }
                let sm = subst_method(m, &map);
                let dup = out.iter().any(|o| {
                    o.owner != *cn
                        && o.method.name == sm.name
                        && o.method.params.len() == sm.params.len()
                        && o.method.params.iter().zip(&sm.params).all(|(a, b)| !b.ty.has_unknown() && a.ty.erasure() == b.ty.erasure())
                });
                if !dup {
                    out.push(MethodRef { owner: cn.clone(), method: sm, owner_kind: c.kind, owner_flags: c.flags });
                }
            }
        }
        out
    }

    fn field(&self, ty: &Ty, name: &str) -> Option<FieldRef> {
        for st in self.all_supertypes(ty) {
            let Ty::Class { name: cn, args } = &st else { continue };
            let Some(c) = self.class(cn) else { continue };
            if let Some(f) = c.field(name) {
                let map = substitution(&c, args);
                let mut f = f.clone();
                f.ty = apply_subst(&f.ty, &map);
                return Some(FieldRef { owner: cn.clone(), field: f });
            }
        }
        None
    }

    /// Whether the full supertype closure of `name` is known. Every type has
    /// java.lang.Object above it (interfaces too: they have its methods),
    /// so without a JDK nothing is complete.
    fn hierarchy_complete(&self, name: &str) -> bool {
        if self.class("java.lang.Object").is_none() {
            return false;
        }
        let mut seen = FxHashSet::default();
        let mut q = VecDeque::new();
        q.push_back(Arc::<str>::from(name));
        while let Some(n) = q.pop_front() {
            if !seen.insert(n.clone()) {
                continue;
            }
            let Some(c) = self.class(&n) else { return false };
            for t in c.superclass.iter().chain(c.interfaces.iter()) {
                match t.class_name() {
                    Some(sn) => q.push_back(Arc::from(sn)),
                    None => return false,
                }
            }
        }
        true
    }

    /// Does the class (or a superclass other than Object) declare `name(params)`?
    fn overrides_object_method(&self, class: &str, method: &str, arity: usize) -> bool {
        let ty = Ty::class(class);
        self.methods_named(&ty, method).iter().any(|m| m.method.params.len() == arity && &*m.owner != "java.lang.Object" && !m.method.flags.is_abstract())
    }

    /// Single abstract method of a functional interface, substituted.
    fn functional_method(&self, ty: &Ty) -> Option<MethodRef> {
        let n = ty.class_name()?;
        let c = self.class(n)?;
        if !c.is_interface() {
            return None;
        }
        let mut abstract_methods: Vec<MethodRef> = self
            .all_methods(ty)
            .into_iter()
            .filter(|m| m.method.flags.is_abstract() && !m.method.flags.is_static())
            .filter(|m| {
                // methods of Object don't count
                !matches!((&*m.method.name, m.method.params.len()), ("equals", 1) | ("hashCode", 0) | ("toString", 0))
            })
            .collect();
        if abstract_methods.len() == 1 {
            abstract_methods.pop()
        } else {
            None
        }
    }
}
