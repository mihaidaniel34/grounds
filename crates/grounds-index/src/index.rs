//! The whole-program declaration index: classpath classes (lazy, from jars)
//! layered under source classes (from stubs, resolved lazily).

use crate::binary;
use crate::hierarchy::ClassLookup;
use crate::model::*;
use crate::stubs::*;
use crate::stubs::fold_binary;

thread_local! {
    /// (class, simple name) member-type lookups in progress on this thread
    static MEMBER_LOOKUPS: std::cell::RefCell<Vec<(Name, Name)>> = const { std::cell::RefCell::new(Vec::new()) };
}
use dashmap::DashMap;
use grounds_classfile::ClassContainer;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;

/// Immutable classpath: JDK symbols plus dependency jars and class dirs.
pub struct Classpath {
    containers: Vec<ClassContainer>,
    names: FxHashMap<Name, u32>,
    packages: FxHashSet<Name>,
    cache: DashMap<Name, Option<Arc<ClassInfo>>>,
}

impl std::fmt::Debug for Classpath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Classpath({} containers, {} classes)", self.containers.len(), self.names.len())
    }
}

impl Classpath {
    pub fn empty() -> Classpath {
        Classpath { containers: Vec::new(), names: FxHashMap::default(), packages: FxHashSet::default(), cache: DashMap::new() }
    }

    /// Build from containers in priority order (first definition wins).
    pub fn from_containers(containers: Vec<ClassContainer>) -> Classpath {
        let lists: Vec<Vec<String>> = {
            use rayon::prelude::*;
            containers.par_iter().map(|c| c.class_names()).collect()
        };
        let mut names = FxHashMap::default();
        let mut packages = FxHashSet::default();
        for (i, list) in lists.into_iter().enumerate() {
            for n in list {
                let b = binary::internal_to_binary(&n);
                if let Some(pos) = b.rfind('.') {
                    let pkg = &b[..pos];
                    if !packages.contains(pkg) {
                        // register all parent packages too
                        let mut p = pkg;
                        loop {
                            packages.insert(Arc::from(p));
                            match p.rfind('.') {
                                Some(j) => p = &p[..j],
                                None => break,
                            }
                        }
                    }
                }
                names.entry(Arc::from(b.as_str())).or_insert(i as u32);
            }
        }
        Classpath { containers, names, packages, cache: DashMap::new() }
    }

    /// JDK from `ct.sym` for a release, followed by the given jars/dirs.
    pub fn build(java_home: Option<&Path>, release: u32, entries: &[std::path::PathBuf]) -> Classpath {
        let mut containers = Vec::new();
        if let Some(home) = java_home {
            let ct = home.join("lib/ct.sym");
            if let Ok(c) = ClassContainer::open_ct_sym(&ct, release) {
                containers.push(c);
            }
        }
        for e in entries {
            if e.is_dir() {
                containers.push(ClassContainer::open_dir(e));
            } else if let Ok(c) = ClassContainer::open_jar(e) {
                containers.push(c);
            }
        }
        Classpath::from_containers(containers)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
    pub fn containers(&self) -> &[ClassContainer] {
        &self.containers
    }
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }
    pub fn has_package(&self, p: &str) -> bool {
        self.packages.contains(p)
    }
    pub fn class_names(&self) -> impl Iterator<Item = &Name> {
        self.names.keys()
    }

    pub fn class(&self, name: &str) -> Option<Arc<ClassInfo>> {
        if let Some(c) = self.cache.get(name) {
            return c.clone();
        }
        let r = (|| {
            let idx = *self.names.get(name)?;
            let internal = name.replace('.', "/");
            let cf = self.containers[idx as usize].read_class(&internal)?;
            Some(Arc::new(binary::lower_class(&cf, idx)))
        })();
        self.cache.insert(Arc::from(name), r.clone());
        r
    }

    /// Read a resource from all containers (e.g. META-INF/spring.factories).
    pub fn resources(&self, name: &str) -> Vec<Vec<u8>> {
        self.containers.iter().filter_map(|c| c.read_resource(name)).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceClassRef {
    pub idx: u32,
}

/// Name lookup context for resolving type syntax.
#[derive(Clone, Debug, Default)]
pub struct Scope<'a> {
    pub package: &'a str,
    pub imports: &'a [Import],
    /// Enclosing classes, innermost first.
    pub enclosing: Vec<Name>,
    /// Type variables in scope.
    pub type_vars: Vec<Name>,
    /// Local classes visible here: simple name -> binary name.
    pub local_types: Vec<(Name, Name)>,
}

pub struct Index {
    pub classpath: Arc<Classpath>,
    files: FxHashMap<Arc<str>, Arc<FileStub>>,
    classes: FxHashMap<Name, (Arc<str>, u32)>,
    packages: FxHashMap<Name, u32>,
    cache: DashMap<Name, Option<Arc<ClassInfo>>>,
    /// Number of source files defining each class name (>1 = duplicated
    /// across modules, e.g. test fixtures).
    defs: FxHashMap<Name, u32>,
    pub generation: u64,
}

thread_local! {
    static RESOLVING: RefCell<FxHashSet<Name>> = RefCell::new(FxHashSet::default());
}

/// Marks a class as being resolved on this thread for as long as it lives,
/// also when resolution panics (rule and daemon panics are caught and the
/// thread is reused). Borrows the name: nothing to clone.
struct Resolving<'a>(&'a str);

impl<'a> Resolving<'a> {
    /// `None` if the class is already being resolved further up (a cycle).
    fn enter(name: &'a str) -> Option<Resolving<'a>> {
        RESOLVING.with(|r| {
            let mut r = r.borrow_mut();
            // allocate the key only when actually entering
            (!r.contains(name)).then(|| {
                r.insert(Arc::from(name));
                Resolving(name)
            })
        })
    }
}

impl Drop for Resolving<'_> {
    fn drop(&mut self) {
        RESOLVING.with(|r| {
            r.borrow_mut().remove(self.0);
        });
    }
}

impl Index {
    pub fn new(classpath: Arc<Classpath>) -> Index {
        Index { classpath, files: FxHashMap::default(), classes: FxHashMap::default(), packages: FxHashMap::default(), cache: DashMap::new(), defs: FxHashMap::default(), generation: 0 }
    }

    pub fn set_file(&mut self, path: Arc<str>, stub: Arc<FileStub>) {
        self.remove_file_inner(&path);
        for (i, c) in stub.classes.iter().enumerate() {
            // a class defined in several files resolves to the smallest path,
            // whatever order files were loaded, refreshed or swapped in
            match self.classes.get(&c.name) {
                Some((p, _)) if **p <= *path => {}
                _ => {
                    self.classes.insert(c.name.clone(), (path.clone(), i as u32));
                }
            }
            *self.defs.entry(c.name.clone()).or_insert(0) += 1;
        }
        let mut p: &str = &stub.package;
        loop {
            *self.packages.entry(Arc::from(p)).or_insert(0) += 1;
            match p.rfind('.') {
                Some(j) => p = &p[..j],
                None => break,
            }
        }
        self.files.insert(path, stub);
        self.cache.clear();
        self.generation += 1;
    }

    pub fn remove_file(&mut self, path: &str) {
        self.remove_file_inner(path);
        self.cache.clear();
        self.generation += 1;
    }

    fn remove_file_inner(&mut self, path: &str) {
        if let Some(old) = self.files.remove(path) {
            for c in &old.classes {
                if let Some(d) = self.defs.get_mut(&c.name) {
                    *d -= 1;
                    if *d == 0 {
                        self.defs.remove(&c.name);
                    }
                }
                if self.classes.get(&c.name).is_some_and(|(p, _)| &**p == path) {
                    self.classes.remove(&c.name);
                    // another file may define the same class name: the smallest path
                    let next = self.files.iter().filter_map(|(fp, st)| st.classes.iter().position(|x| x.name == c.name).map(|i| (fp, i))).min_by(|a, b| a.0.cmp(b.0));
                    if let Some((fp, i)) = next {
                        self.classes.insert(c.name.clone(), (fp.clone(), i as u32));
                    }
                }
            }
            let mut p: &str = &old.package;
            loop {
                if let Some(cnt) = self.packages.get_mut(p) {
                    *cnt -= 1;
                    if *cnt == 0 {
                        self.packages.remove(p);
                    }
                }
                match p.rfind('.') {
                    Some(j) => p = &p[..j],
                    None => break,
                }
            }
        }
    }

    pub fn file(&self, path: &str) -> Option<&Arc<FileStub>> {
        self.files.get(path)
    }

    pub fn files(&self) -> impl Iterator<Item = (&Arc<str>, &Arc<FileStub>)> {
        self.files.iter()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn source_class_names(&self) -> impl Iterator<Item = &Name> {
        self.classes.keys()
    }

    /// Is this class name defined in more than one source file?
    pub fn is_duplicated(&self, name: &str) -> bool {
        self.defs.get(name).is_some_and(|d| *d > 1)
    }

    pub fn is_source_class(&self, name: &str) -> bool {
        self.classes.contains_key(name)
    }

    pub fn source_file_of(&self, class: &str) -> Option<&Arc<str>> {
        self.classes.get(class).map(|(p, _)| p)
    }

    pub fn raw_class(&self, name: &str) -> Option<(&Arc<FileStub>, &RawClass)> {
        let (path, idx) = self.classes.get(name)?;
        let stub = self.files.get(path)?;
        Some((stub, &stub.classes[*idx as usize]))
    }

    pub fn class_exists(&self, name: &str) -> bool {
        self.classes.contains_key(name) || self.classpath.contains(name)
    }

    pub fn package_exists(&self, p: &str) -> bool {
        self.packages.contains_key(p) || self.classpath.has_package(p)
    }

    pub fn class(&self, name: &str) -> Option<Arc<ClassInfo>> {
        if self.classes.contains_key(name) {
            if let Some(c) = self.cache.get(name) {
                return c.clone();
            }
            let _resolving = Resolving::enter(name)?;
            let r = self.resolve_source_class(name).map(Arc::new);
            self.cache.insert(Arc::from(name), r.clone());
            return r;
        }
        self.classpath.class(name)
    }

    fn scope_for_raw<'a>(&self, stub: &'a FileStub, raw: &RawClass) -> Scope<'a> {
        let mut enclosing = vec![raw.name.clone()];
        let mut type_vars: Vec<Name> = raw.type_params.iter().map(|t| t.name.clone()).collect();
        let mut o = raw.outer.clone();
        while let Some(on) = o {
            enclosing.push(on.clone());
            o = match self.raw_class(&on) {
                Some((_, oc)) => {
                    if !raw.flags.is_static() {
                        type_vars.extend(oc.type_params.iter().map(|t| t.name.clone()));
                    }
                    oc.outer.clone()
                }
                None => None,
            };
        }
        Scope { package: &stub.package, imports: &stub.imports, enclosing, type_vars, local_types: Vec::new() }
    }

    pub fn scope_for_class<'a>(&'a self, name: &str) -> Option<Scope<'a>> {
        let (stub, raw) = self.raw_class(name)?;
        Some(self.scope_for_raw(stub, raw))
    }

    fn resolve_source_class(&self, name: &str) -> Option<ClassInfo> {
        let (path, _) = self.classes.get(name)?;
        let (stub, raw) = self.raw_class(name)?;
        let scope = self.scope_for_raw(stub, raw);
        Some(self.lower_raw_class(raw, &scope, Origin::Source { file: path.clone() }))
    }

    /// Lower a raw class with an explicit scope (also used for local classes).
    pub fn lower_raw_class(&self, raw: &RawClass, scope: &Scope, origin: Origin) -> ClassInfo {
        // Header types must not consult this class's inherited members.
        let header_scope = Scope { enclosing: scope.enclosing.iter().skip(1).cloned().collect(), ..scope.clone() };
        let mut header_scope = header_scope;
        header_scope.type_vars = scope.type_vars.clone();
        let tps = raw
            .type_params
            .iter()
            .map(|t| TypeParam { name: t.name.clone(), bounds: t.bounds.iter().map(|b| self.resolve_type(b, &header_scope)).collect() })
            .collect();
        let superclass = raw.superclass.as_ref().map(|s| self.resolve_type(s, &header_scope));
        let interfaces = raw.interfaces.iter().map(|s| self.resolve_type(s, &header_scope)).collect();
        let permits = raw.permits.iter().map(|s| self.resolve_type(s, &header_scope)).collect();
        let annotations = raw.annotations.iter().map(|a| self.resolve_annotation(a, scope)).collect();
        let mut fields: Vec<FieldInfo> = Vec::with_capacity(raw.fields.len());
        for f in &raw.fields {
            let ty = self.resolve_type(&f.ty, scope);
            let constant = f.constant.clone().or_else(|| {
                let e = f.init.as_ref()?;
                let v = self.eval_const_expr(e, scope, &fields, raw, 0)?;
                coerce_constant(v, &ty)
            });
            fields.push(FieldInfo {
                name: f.name.clone(),
                ty,
                flags: f.flags,
                annotations: f.annotations.iter().map(|a| self.resolve_annotation(a, scope)).collect(),
                constant,
                range: Some(f.range),
            });
        }
        let methods = raw
            .methods
            .iter()
            .map(|m| {
                let mut ms = scope.clone();
                ms.type_vars.extend(m.type_params.iter().map(|t| t.name.clone()));
                MethodInfo {
                    name: m.name.clone(),
                    flags: m.flags,
                    type_params: m
                        .type_params
                        .iter()
                        .map(|t| TypeParam { name: t.name.clone(), bounds: t.bounds.iter().map(|b| self.resolve_type(b, &ms)).collect() })
                        .collect(),
                    params: m
                        .params
                        .iter()
                        .map(|p| ParamInfo {
                            name: p.name.clone(),
                            ty: self.resolve_type(&p.ty, &ms),
                            annotations: p.annotations.iter().map(|a| self.resolve_annotation(a, &ms)).collect(),
                        })
                        .collect(),
                    ret: self.resolve_type(&m.ret, &ms),
                    throws: m.throws.iter().map(|t| self.resolve_type(t, &ms)).collect(),
                    annotations: m.annotations.iter().map(|a| self.resolve_annotation(a, &ms)).collect(),
                    range: Some(m.range),
                    generated_by: m.generated_by.clone(),
                }
            })
            .collect();
        ClassInfo {
            name: raw.name.clone(),
            kind: raw.kind,
            flags: raw.flags,
            type_params: tps,
            superclass: superclass.or_else(|| if raw.kind == ClassKind::Class && &*raw.name != "java.lang.Object" { Some(Ty::object()) } else { None }),
            interfaces,
            fields,
            methods,
            annotations,
            outer: raw.outer.clone(),
            inners: raw.inners.clone(),
            permits,
            origin,
            range: Some(raw.range),
        }
    }

    fn eval_const_expr(&self, e: &ConstExpr, scope: &Scope, done: &[FieldInfo], raw: &RawClass, depth: u32) -> Option<Constant> {
        if depth > 32 {
            return None;
        }
        match e {
            ConstExpr::Lit(c) => Some(c.clone()),
            ConstExpr::Name(n) => {
                if !n.contains('.') {
                    // earlier field of the same class
                    if let Some(f) = done.iter().find(|f| *f.name == **n) {
                        return f.constant.clone();
                    }
                    if let Some(f) = raw.fields.iter().find(|f| *f.name == **n) {
                        if let Some(c) = &f.constant {
                            return Some(c.clone());
                        }
                        if let Some(ie) = &f.init {
                            return self.eval_const_expr(ie, scope, done, raw, depth + 1);
                        }
                        return None;
                    }
                }
                match self.resolve_constant_name(n, scope)? {
                    AnnotationValue::Const(c) => Some(c),
                    _ => None,
                }
            }
            ConstExpr::Unary(op, v) => {
                let v = self.eval_const_expr(v, scope, done, raw, depth + 1)?;
                crate::stubs::fold_unary(op, v)
            }
            ConstExpr::Binary(op, l, r) => {
                let l = self.eval_const_expr(l, scope, done, raw, depth + 1)?;
                let r = self.eval_const_expr(r, scope, done, raw, depth + 1)?;
                fold_binary(op, &l, &r)
            }
            ConstExpr::Cast(p, v) => {
                let v = self.eval_const_expr(v, scope, done, raw, depth + 1)?;
                coerce_constant(v, &Ty::Prim(*p))
            }
            ConstExpr::CastString(v) => self.eval_const_expr(v, scope, done, raw, depth + 1),
            ConstExpr::Ternary(c, a, b) => {
                let c = self.eval_const_expr(c, scope, done, raw, depth + 1)?.as_bool()?;
                self.eval_const_expr(if c { a } else { b }, scope, done, raw, depth + 1)
            }
        }
    }

    pub fn resolve_annotation(&self, a: &RawAnnotation, scope: &Scope) -> AnnotationInfo {
        let segs: Vec<&str> = a.name.split('.').collect();
        let (name, resolved) = match self.resolve_type_name(&segs, scope) {
            Some(n) => (n, true),
            None => (a.name.clone(), false),
        };
        AnnotationInfo { name, resolved, args: a.args.iter().map(|(k, v)| (k.clone(), self.resolve_ann_value(v, scope))).collect() }
    }

    fn resolve_ann_value(&self, v: &RawAnnValue, scope: &Scope) -> AnnotationValue {
        match v {
            RawAnnValue::Const(c) => AnnotationValue::Const(c.clone()),
            RawAnnValue::ClassLit(t) => AnnotationValue::Class(self.resolve_type(t, scope)),
            RawAnnValue::Annotation(a) => AnnotationValue::Annotation(Box::new(self.resolve_annotation(a, scope))),
            RawAnnValue::Array(v) => AnnotationValue::Array(v.iter().map(|x| self.resolve_ann_value(x, scope)).collect()),
            RawAnnValue::Expr(e) => AnnotationValue::Expr(e.clone()),
            RawAnnValue::Name(n) => self.resolve_constant_name(n, scope).unwrap_or_else(|| AnnotationValue::Expr(n.clone())),
        }
    }

    /// Resolve `CONST`, `Type.CONST` or a static import to an annotation value.
    pub fn resolve_constant_name(&self, n: &str, scope: &Scope) -> Option<AnnotationValue> {
        let segs: Vec<&str> = n.split('.').collect();
        // constants of source classes are read from raw stubs, which also works
        // while the class itself is being lowered
        let raw_constant = |owner: &str, field: &str| -> Option<AnnotationValue> {
            let (_, raw) = self.raw_class(owner)?;
            let f = raw.fields.iter().find(|f| &*f.name == field)?;
            if f.flags.has(Flags::ENUM_CONSTANT) {
                return Some(AnnotationValue::Enum { ty: Arc::from(owner), constant: Arc::from(field) });
            }
            if let Some(c) = &f.constant {
                return Some(AnnotationValue::Const(c.clone()));
            }
            let init = f.init.as_ref()?;
            let owner_scope = self.scope_for_class(owner)?;
            self.eval_const_expr(init, &owner_scope, &[], raw, 0).map(AnnotationValue::Const)
        };
        if segs.len() == 1 {
            for e in &scope.enclosing {
                if let Some(v) = raw_constant(e, segs[0]) {
                    return Some(v);
                }
            }
        } else if let Some((last, rest)) = segs.split_last() {
            if let Some(owner) = self.resolve_type_name(rest, scope) {
                if let Some(v) = raw_constant(&owner, last) {
                    return Some(v);
                }
            }
        }
        let (owner, field) = if segs.len() == 1 {
            let f = segs[0];
            // enclosing classes, then static imports
            let mut owner = None;
            for e in &scope.enclosing {
                if let Some(c) = self.class(e) {
                    if self.field(&c.this_type(), f).is_some() {
                        owner = Some(e.clone());
                        break;
                    }
                }
            }
            if owner.is_none() {
                for imp in scope.imports.iter().filter(|i| i.is_static) {
                    if imp.on_demand || imp.last() == f {
                        let cls = if imp.on_demand { imp.path.to_string() } else { imp.path.rsplit_once('.').map(|x| x.0.to_string()).unwrap_or_default() };
                        if let Some(b) = self.binary_name_for_dotted(&cls) {
                            if self.class(&b).is_some_and(|c| c.field(f).is_some()) {
                                owner = Some(b);
                                break;
                            }
                        }
                    }
                }
            }
            (owner?, f)
        } else {
            let (last, rest) = segs.split_last()?;
            (self.resolve_type_name(rest, scope)?, *last)
        };
        let c = self.class(&owner)?;
        let fr = self.field(&c.this_type(), field)?;
        if fr.field.flags.has(Flags::ENUM_CONSTANT) {
            return Some(AnnotationValue::Enum { ty: fr.owner.clone(), constant: Arc::from(field) });
        }
        fr.field.constant.clone().map(AnnotationValue::Const)
    }

    pub fn resolve_type(&self, ts: &TypeSyntax, scope: &Scope) -> Ty {
        match ts {
            TypeSyntax::Prim(p) => Ty::Prim(*p),
            TypeSyntax::Void => Ty::Void,
            TypeSyntax::Var | TypeSyntax::Unknown => Ty::Unknown,
            TypeSyntax::Array(e) => match self.resolve_type(e, scope) {
                Ty::Unknown => Ty::Unknown,
                t => Ty::Array(Box::new(t)),
            },
            TypeSyntax::Wildcard(None) => Ty::Wildcard(None),
            TypeSyntax::Wildcard(Some((ext, b))) => {
                let t = self.resolve_type(b, scope);
                Ty::Wildcard(Some(Box::new(if *ext { WildcardBound::Extends(t) } else { WildcardBound::Super(t) })))
            }
            TypeSyntax::Named(segs) => {
                if segs.len() == 1 && scope.type_vars.iter().any(|v| *v == segs[0].0) {
                    return Ty::TypeVar(segs[0].0.clone());
                }
                let names: Vec<&str> = segs.iter().map(|s| &*s.0).collect();
                match self.resolve_type_name(&names, scope) {
                    Some(n) => {
                        let args = segs.last().map(|s| s.1.iter().map(|a| self.resolve_type(a, scope)).collect()).unwrap_or_default();
                        Ty::Class { name: n, args }
                    }
                    None => Ty::Unknown,
                }
            }
        }
    }

    /// Resolve a (possibly qualified) type name to a binary class name.
    pub fn resolve_type_name(&self, segs: &[&str], scope: &Scope) -> Option<Name> {
        let first = *segs.first()?;
        if let Some(n) = self.resolve_simple_type(first, scope) {
            let mut cur = n;
            for s in &segs[1..] {
                cur = self.member_type(&cur, s)?;
            }
            return Some(cur);
        }
        if segs.len() == 1 {
            return None;
        }
        let dotted = segs.join(".");
        self.binary_name_for_dotted(&dotted)
    }

    /// `java.util.Map.Entry` -> `java.util.Map$Entry`
    pub fn binary_name_for_dotted(&self, dotted: &str) -> Option<Name> {
        if self.class_exists(dotted) {
            return Some(Arc::from(dotted));
        }
        let segs: Vec<&str> = dotted.split('.').collect();
        for i in (1..segs.len()).rev() {
            let pkg = segs[..i].join(".");
            let top = format!("{}.{}", pkg, segs[i]);
            if self.class_exists(&top) {
                let mut cur: Name = Arc::from(top.as_str());
                let mut ok = true;
                for s in &segs[i + 1..] {
                    match self.member_type(&cur, s) {
                        Some(n) => cur = n,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    return Some(cur);
                }
            }
        }
        if !segs.is_empty() {
            // default package nested: Outer.Inner
            let mut cur: Name = Arc::from(segs[0]);
            if self.class_exists(&cur) {
                for s in &segs[1..] {
                    cur = self.member_type(&cur, s)?;
                }
                return Some(cur);
            }
        }
        None
    }

    /// Member type `simple` of class `owner`, including inherited member types.
    pub fn member_type(&self, owner: &str, simple: &str) -> Option<Name> {
        // Resolving a supertype resolves names, which can come back here for
        // the same lookup (classes nested in and extending each other): a
        // re-entrant lookup is "not found" instead of unbounded recursion.
        let key: (Name, Name) = (Arc::from(owner), Arc::from(simple));
        let entered = MEMBER_LOOKUPS.with(|l| {
            let mut l = l.borrow_mut();
            if l.len() > 64 || l.contains(&key) {
                false
            } else {
                l.push(key);
                true
            }
        });
        if !entered {
            return None;
        }
        struct Pop;
        impl Drop for Pop {
            fn drop(&mut self) {
                MEMBER_LOOKUPS.with(|l| {
                    l.borrow_mut().pop();
                });
            }
        }
        let _pop = Pop;
        let mut seen = FxHashSet::default();
        self.member_type_rec(owner, simple, &mut seen, 0)
    }

    fn member_type_rec(&self, owner: &str, simple: &str, seen: &mut FxHashSet<Name>, depth: u32) -> Option<Name> {
        if depth > 20 || !seen.insert(Arc::from(owner)) {
            return None;
        }
        let direct = format!("{owner}${simple}");
        if self.class_exists(&direct) {
            return Some(Arc::from(direct.as_str()));
        }
        let mut sups: Vec<Name> = Vec::new();
        match self.class(owner) {
            Some(c) => {
                if let Some(Ty::Class { name, .. }) = &c.superclass {
                    sups.push(name.clone());
                }
                for i in &c.interfaces {
                    if let Ty::Class { name, .. } = i {
                        sups.push(name.clone());
                    }
                }
            }
            None => {
                // the class is being lowered right now: resolve its header from the stub
                let (stub, raw) = self.raw_class(owner)?;
                let mut scope = self.scope_for_raw(stub, raw);
                scope.enclosing.remove(0);
                for ts in raw.superclass.iter().chain(raw.interfaces.iter()) {
                    if let Ty::Class { name, .. } = self.resolve_type(ts, &scope) {
                        sups.push(name);
                    }
                }
            }
        }
        for s in sups {
            if let Some(n) = self.member_type_rec(&s, simple, seen, depth + 1) {
                return Some(n);
            }
        }
        None
    }

    pub fn resolve_simple_type(&self, simple: &str, scope: &Scope) -> Option<Name> {
        for (s, b) in &scope.local_types {
            if &**s == simple {
                return Some(b.clone());
            }
        }
        // enclosing classes and their (inherited) member types
        for e in &scope.enclosing {
            if simple_name_of(e) == simple && !e.contains("$$") {
                return Some(e.clone());
            }
            let direct = format!("{e}${simple}");
            if self.class_exists(&direct) {
                return Some(Arc::from(direct.as_str()));
            }
            if let Some(n) = self.member_type(e, simple) {
                return Some(n);
            }
        }
        // single-type imports
        for imp in scope.imports.iter().filter(|i| !i.on_demand) {
            if imp.last() == simple {
                if imp.is_static {
                    // static import of a member type
                    if let Some((cls, _)) = imp.path.rsplit_once('.') {
                        if let Some(b) = self.binary_name_for_dotted(cls) {
                            if let Some(n) = self.member_type(&b, simple) {
                                return Some(n);
                            }
                        }
                    }
                    continue;
                }
                if let Some(b) = self.binary_name_for_dotted(&imp.path) {
                    return Some(b);
                }
                // the import shadows same-package and on-demand classes even
                // when its jar is missing: unknown, never a same-named class
                return None;
            }
        }
        // same package
        let same = if scope.package.is_empty() { simple.to_string() } else { format!("{}.{}", scope.package, simple) };
        if self.class_exists(&same) {
            // a same-package class from another module (a test fixture in a
            // module this one does not depend on) loses to an on-demand
            // import that provides the name; javac would not see it
            let module = |n: &str| self.source_file_of(n).map(|f| f.split("/src/").next().unwrap_or("").to_string());
            let here = scope.enclosing.last().and_then(|e| module(e));
            let other_module = matches!((module(&same), here), (Some(a), Some(b)) if a != b);
            if !other_module {
                return Some(Arc::from(same.as_str()));
            }
            if let Some(n) = self.resolve_on_demand(simple, scope) {
                return Some(n);
            }
            return Some(Arc::from(same.as_str()));
        }
        if let Some(n) = self.resolve_on_demand(simple, scope) {
            return Some(n);
        }
        let jl = format!("java.lang.{simple}");
        if self.class_exists(&jl) {
            return Some(Arc::from(jl.as_str()));
        }
        None
    }

    fn resolve_on_demand(&self, simple: &str, scope: &Scope) -> Option<Name> {
        let lookup = |imp: &Import| -> Option<Name> {
            if self.package_exists(&imp.path) && !imp.is_static {
                let c = format!("{}.{}", imp.path, simple);
                if self.class_exists(&c) {
                    return Some(Arc::from(c.as_str()));
                }
            }
            self.binary_name_for_dotted(&imp.path).and_then(|b| self.member_type(&b, simple))
        };
        // the file's own on-demand imports shadow module imports (JEP 511)
        if let Some(n) = scope.imports.iter().filter(|i| i.on_demand && !i.from_module).find_map(lookup) {
            return Some(n);
        }
        // module imports: a name exported by two packages is ambiguous
        let mut found: Option<Name> = None;
        for n in scope.imports.iter().filter(|i| i.on_demand && i.from_module).filter_map(lookup) {
            match &found {
                Some(f) if *f != n => return None,
                _ => found = Some(n),
            }
        }
        found
    }

    /// All source classes whose direct supertypes include `name`.
    pub fn direct_source_subclasses(&self, name: &str) -> Vec<Name> {
        let mut out = Vec::new();
        for n in self.classes.keys() {
            if let Some(c) = self.class(n) {
                let mut hit = c.superclass.as_ref().and_then(|t| t.class_name()) == Some(name);
                hit |= c.interfaces.iter().any(|t| t.class_name() == Some(name));
                if hit {
                    out.push(n.clone());
                }
            }
        }
        out
    }
}

impl ClassLookup for Index {
    fn class(&self, name: &str) -> Option<Arc<ClassInfo>> {
        Index::class(self, name)
    }
}

/// Convert a folded constant to the representation of the declared type.
pub fn coerce_constant(v: Constant, ty: &Ty) -> Option<Constant> {
    Some(match ty {
        Ty::Prim(Prim::Int) => Constant::Int(match v {
            Constant::Float(f) => f as i32 as i64,
            c => c.as_i64()? as i32 as i64,
        }),
        Ty::Prim(Prim::Long) => Constant::Long(match v {
            Constant::Float(f) => f as i64,
            c => c.as_i64()?,
        }),
        Ty::Prim(Prim::Short) => Constant::Int(v.as_i64()? as i16 as i64),
        Ty::Prim(Prim::Byte) => Constant::Int(v.as_i64()? as i8 as i64),
        Ty::Prim(Prim::Char) => Constant::Char(char::from_u32(v.as_i64()? as u32)?),
        Ty::Prim(Prim::Float) | Ty::Prim(Prim::Double) => Constant::Float(v.as_f64()?),
        Ty::Prim(Prim::Boolean) => Constant::Bool(v.as_bool()?),
        _ => v,
    })
}

#[cfg(test)]
mod resolving_tests {
    use super::*;

    fn resolving(name: &str) -> bool {
        RESOLVING.with(|r| r.borrow().contains(name))
    }

    #[test]
    fn the_guard_marks_a_cycle_and_leaves_on_panic() {
        {
            let outer = Resolving::enter("p.A").expect("enters");
            assert!(Resolving::enter("p.A").is_none(), "a cycle");
            assert!(resolving("p.A"), "the failed entry did not remove the outer one");
            drop(outer);
        }
        assert!(!resolving("p.A"));
        let r = std::panic::catch_unwind(|| {
            let _g = Resolving::enter("p.B").expect("enters");
            panic!("resolution bug");
        });
        assert!(r.is_err());
        assert!(!resolving("p.B"), "left on panic");
    }
}
