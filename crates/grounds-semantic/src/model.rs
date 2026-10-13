//! Per-file semantic model. Lazily computes and memoizes types, symbols and
//! call resolutions for one compilation unit against the global index.

use crate::binder::{self, Binder, LocalDecl};
use grounds_index::stubs::{self, FileStub};
use grounds_index::*;
use grounds_syntax::{Node, NodeExt, ParsedFile};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// What an identifier or qualified name refers to.
#[derive(Clone, Debug)]
pub enum Symbol<'t> {
    Local(usize, LocalDecl<'t>),
    Field(FieldRef),
    Type(Name),
    Package(String),
}

#[derive(Clone, Debug)]
pub struct ResolvedCall {
    pub owner: Name,
    /// The selected method with receiver and inferred type arguments
    /// substituted.
    pub method: MethodInfo,
    /// Return type after inference (`Unknown` if not inferable).
    pub ret: Ty,
    /// Receiver type for instance calls, if any.
    pub receiver: Option<Ty>,
    /// Overload resolution could not pick a unique method.
    pub ambiguous: bool,
    /// The pick is a guess among overloads with the same erased result:
    /// only that erasure is known.
    pub erasure_only: bool,
    /// Static call through a type name (`Foo.bar()`).
    pub via_type: bool,
}

impl ResolvedCall {
    pub fn is(&self, owner: &str, name: &str) -> bool {
        &*self.owner == owner && &*self.method.name == name
    }
    pub fn name(&self) -> &str {
        &self.method.name
    }
    pub fn param_types(&self) -> Vec<Ty> {
        self.method.params.iter().map(|p| p.ty.clone()).collect()
    }
}

pub struct FileModel<'t> {
    pub file: &'t ParsedFile,
    pub src: &'t str,
    pub index: &'t Index,
    pub stub: Arc<FileStub>,
    pub path: Arc<str>,
    pub binder: Binder<'t>,
    /// type declaration / anonymous body node id -> binary name
    pub class_names: FxHashMap<usize, Name>,
    /// local & anonymous classes (not in the index)
    local_raw: FxHashMap<Name, (stubs::RawClass, Node<'t>)>,
    /// local types: (simple name, binary name, byte range of the declaring block)
    local_simple: Vec<(Name, Name, (usize, usize))>,
    local_lowered: RefCell<FxHashMap<Name, Option<Arc<ClassInfo>>>>,
    pub(crate) types: RefCell<FxHashMap<usize, Ty>>,
    pub(crate) calls: RefCell<FxHashMap<usize, Option<Rc<ResolvedCall>>>>,
    /// method invocation types without target typing (see `arg_type`)
    pub(crate) standalone: RefCell<FxHashMap<(usize, u8), Ty>>,
    pub(crate) selected: RefCell<FxHashMap<usize, Option<Rc<ResolvedCall>>>>,
    pub(crate) in_progress: RefCell<FxHashSet<(usize, u8)>>,
    /// Incremented whenever a cycle guard short-circuits; results computed
    /// while this changed are provisional and not memoized.
    pub(crate) cycle_hits: std::cell::Cell<u64>,
    /// Guarded computations so far; past `WORK_BUDGET` results that hit a
    /// cycle are memoized as unknown, so pathological files stay polynomial.
    pub(crate) work: std::cell::Cell<u64>,
    pub(crate) symbols: RefCell<FxHashMap<usize, Option<Symbol<'t>>>>,
    captures: std::cell::OnceCell<FxHashMap<usize, Vec<usize>>>,
    occurrences: std::cell::OnceCell<FxHashMap<&'t str, Vec<Node<'t>>>>,
}

impl<'t> FileModel<'t> {
    pub fn new(file: &'t ParsedFile, index: &'t Index, path: Arc<str>) -> FileModel<'t> {
        let stub = match index.file(&path) {
            Some(s) => s.clone(),
            None => Arc::new(stubs::extract(file)),
        };
        Self::with_stub(file, index, path, stub)
    }

    pub fn with_stub(file: &'t ParsedFile, index: &'t Index, path: Arc<str>, stub: Arc<FileStub>) -> FileModel<'t> {
        let src: &'t str = &file.text;
        let binder = binder::bind(file.root(), src);
        let mut m = FileModel {
            file,
            src,
            index,
            stub,
            path,
            binder,
            class_names: FxHashMap::default(),
            local_raw: FxHashMap::default(),
            local_simple: Vec::new(),
            local_lowered: RefCell::new(FxHashMap::default()),
            types: RefCell::new(FxHashMap::default()),
            calls: RefCell::new(FxHashMap::default()),
            standalone: RefCell::new(FxHashMap::default()),
            selected: RefCell::new(FxHashMap::default()),
            in_progress: RefCell::new(FxHashSet::default()),
            cycle_hits: std::cell::Cell::new(0),
            work: std::cell::Cell::new(0),
            symbols: RefCell::new(FxHashMap::default()),
            captures: std::cell::OnceCell::new(),
            occurrences: std::cell::OnceCell::new(),
        };
        m.collect_classes();
        m
    }

    fn collect_classes(&mut self) {
        let pkg = self.stub.package.to_string();
        let src = self.src;
        let mut anon_counter: FxHashMap<Name, u32> = FxHashMap::default();
        // DFS with stack of (node, enclosing class name, in_method)
        fn visit<'t>(
            m: &mut FileModel<'t>,
            n: Node<'t>,
            outer: Option<Name>,
            in_code: bool,
            pkg: &str,
            src: &'t str,
            anon: &mut FxHashMap<Name, u32>,
        ) {
            let kind = n.kind();
            if grounds_syntax::is_type_declaration(kind) {
                let Some(simple) = n.field("name").map(|x| x.text(src)) else { return };
                let name: Name = if in_code {
                    let o = outer.clone().unwrap_or_else(|| Arc::from(pkg));
                    let c = anon.entry(o.clone()).or_insert(0);
                    *c += 1;
                    let nm: Name = Arc::from(format!("{}${}{}", o, c, simple).as_str());
                    let raws = stubs::extract_class_node(n, src, pkg, Some(&o));
                    // extract_class_node names it Outer$Simple; rename the root
                    for mut r in raws {
                        if r.decl_range == (n.start_byte() as u32, n.end_byte() as u32) {
                            r.name = nm.clone();
                            m.local_raw.insert(nm.clone(), (r, n));
                        }
                    }
                    // visible in the block that declares it
                    let scope = n.parent().map(|p| (p.start_byte(), p.end_byte())).unwrap_or((0, usize::MAX));
                    m.local_simple.push((Arc::from(simple), nm.clone(), scope));
                    nm
                } else {
                    match &outer {
                        Some(o) => Arc::from(format!("{o}${simple}").as_str()),
                        None if pkg.is_empty() => Arc::from(simple),
                        None => Arc::from(format!("{pkg}.{simple}").as_str()),
                    }
                };
                m.class_names.insert(n.id(), name.clone());
                if let Some(b) = n.field("body") {
                    for k in b.kids() {
                        visit(m, k, Some(name.clone()), false, pkg, src, anon);
                    }
                }
                return;
            }
            if kind == "class_body" && n.parent().is_some_and(|p| matches!(p.kind(), "object_creation_expression" | "enum_constant")) {
                let o = outer.clone().unwrap_or_else(|| Arc::from(pkg));
                let c = anon.entry(o.clone()).or_insert(0);
                *c += 1;
                let simple = format!("{c}");
                let nm: Name = Arc::from(format!("{o}${simple}").as_str());
                let mut raws = stubs::extract_anonymous(n, src, pkg, &o, &simple);
                if let Some(mut r) = raws.drain(..).next() {
                    r.flags.set(Flags::ANON);
                    m.local_raw.insert(nm.clone(), (r, n));
                }
                m.class_names.insert(n.id(), nm.clone());
                for k in n.kids() {
                    visit(m, k, Some(nm.clone()), false, pkg, src, anon);
                }
                return;
            }
            let code = in_code || matches!(kind, "method_declaration" | "constructor_declaration" | "compact_constructor_declaration" | "block" | "lambda_expression" | "field_declaration" | "static_initializer");
            for k in n.kids() {
                visit(m, k, outer.clone(), code, pkg, src, anon);
            }
        }
        let root = self.file.root();
        for k in root.kids() {
            visit(self, k, None, false, &pkg, src, &mut anon_counter);
        }
    }

    pub fn text(&self, n: Node) -> &'t str {
        &self.src[n.start_byte()..n.end_byte()]
    }

    pub fn root(&self) -> Node<'t> {
        self.file.root()
    }

    pub fn package(&self) -> &str {
        &self.stub.package
    }

    /// Binary name of the class declared by a type declaration node or
    /// anonymous class body.
    pub fn class_name_of_decl(&self, decl: Node<'t>) -> Option<&Name> {
        self.class_names.get(&decl.id())
    }

    pub fn class_of_decl(&self, decl: Node<'t>) -> Option<Arc<ClassInfo>> {
        let n = self.class_names.get(&decl.id())?.clone();
        self.class(&n)
    }

    /// The innermost type declaration or anonymous class body enclosing `n`.
    pub fn enclosing_class_node(&self, n: Node<'t>) -> Option<Node<'t>> {
        n.ancestors().find(|a| self.class_names.contains_key(&a.id()))
    }

    pub fn enclosing_class_name(&self, n: Node<'t>) -> Option<Name> {
        self.enclosing_class_node(n).and_then(|c| self.class_names.get(&c.id()).cloned())
    }

    pub fn enclosing_class(&self, n: Node<'t>) -> Option<Arc<ClassInfo>> {
        self.enclosing_class_name(n).and_then(|c| self.class(&c))
    }

    /// Enclosing method, constructor or lambda.
    pub fn enclosing_callable(&self, n: Node<'t>) -> Option<Node<'t>> {
        n.ancestors().take_while(|a| !self.class_names.contains_key(&a.id())).find(|a| grounds_syntax::is_method_like(a.kind()))
    }

    /// Enclosing method or constructor declaration (skipping lambdas).
    pub fn enclosing_method(&self, n: Node<'t>) -> Option<Node<'t>> {
        n.ancestors()
            .take_while(|a| !self.class_names.contains_key(&a.id()))
            .find(|a| matches!(a.kind(), "method_declaration" | "constructor_declaration" | "compact_constructor_declaration"))
    }

    /// Method info for a method/constructor declaration node.
    pub fn method_of_decl(&self, decl: Node<'t>) -> Option<MethodInfo> {
        let cls = self.enclosing_class(decl)?;
        let name_node = decl.field("name")?;
        let r = (name_node.start_byte() as u32, name_node.end_byte() as u32);
        cls.methods.iter().find(|m| m.range == Some(r)).cloned()
    }

    pub fn field_of_declarator(&self, declarator: Node<'t>) -> Option<FieldInfo> {
        let cls = self.enclosing_class(declarator)?;
        let name_node = declarator.field("name")?;
        let r = (name_node.start_byte() as u32, name_node.end_byte() as u32);
        cls.fields.iter().find(|f| f.range == Some(r)).cloned()
    }

    /// Name-resolution scope at a node.
    pub fn scope_at(&self, n: Node<'t>) -> Scope<'_> {
        let mut enclosing = Vec::new();
        let mut type_vars = Vec::new();
        for a in std::iter::once(n).chain(n.ancestors()) {
            if let Some(name) = self.class_names.get(&a.id()) {
                enclosing.push(name.clone());
            }
            if let Some(tp) = a.field("type_parameters") {
                for p in tp.kids_of_kind("type_parameter") {
                    if let Some(id) = p.kid_of_kind("type_identifier") {
                        type_vars.push(Arc::from(id.text(self.src)));
                    }
                }
            }
        }
        // local types whose declaring block contains `n`, innermost first
        let mut locals: Vec<&(Name, Name, (usize, usize))> = self.local_simple.iter().filter(|(_, _, (a, b))| *a <= n.start_byte() && n.end_byte() <= *b).collect();
        locals.sort_by_key(|(_, _, (a, b))| b - a);
        let local_types = locals.into_iter().map(|(s, b, _)| (s.clone(), b.clone())).collect();
        Scope { package: &self.stub.package, imports: &self.stub.imports, enclosing, type_vars, local_types }
    }

    /// Resolve a type syntax node in the scope where it appears.
    pub fn resolve_type_node(&self, type_node: Node<'t>) -> Ty {
        let ts = stubs::type_syntax(type_node, self.src);
        let scope = self.scope_at(type_node);
        self.index.resolve_type(&ts, &scope)
    }

    /// Type of a record pattern: a type node, or (after the parser fixup for
    /// qualified record patterns) an identifier whose text is `Shape.Circle`.
    pub fn resolve_pattern_type(&self, n: Node<'t>) -> Ty {
        let text = n.text(self.src);
        if n.kind() == "identifier" {
            let segs = text.split('.').map(|s| (Arc::<str>::from(s.trim()), Vec::new())).collect();
            return self.resolve_type_syntax_at(&TypeSyntax::Named(segs), n);
        }
        self.resolve_type_node(n)
    }

    /// Type of a `var` component of a record pattern: the record's component
    /// at the same position (unknown for generic components).
    pub fn record_pattern_component_type(&self, component: Node<'t>) -> Ty {
        let Some(body) = component.parent().filter(|b| b.kind() == "record_pattern_body") else { return Ty::Unknown };
        let parts: Vec<Node> = body.kids().into_iter().filter(|k| matches!(k.kind(), "record_pattern_component" | "record_pattern")).collect();
        let Some(pos) = parts.iter().position(|k| *k == component) else { return Ty::Unknown };
        let Some(type_node) = body.parent().and_then(|p| p.kids().into_iter().find(|k| k.kind() != "record_pattern_body")) else { return Ty::Unknown };
        let record = self.resolve_pattern_type(type_node);
        let Some(cls) = record.class_name().and_then(|c| self.class(c)) else { return Ty::Unknown };
        let comps: Vec<&FieldInfo> = cls.fields.iter().filter(|f| f.flags.has(Flags::RECORD_COMPONENT)).collect();
        match comps.get(pos) {
            Some(f) if comps.len() == parts.len() && !f.ty.mentions_type_var() => f.ty.clone(),
            _ => Ty::Unknown,
        }
    }

    pub fn resolve_type_syntax_at(&self, ts: &TypeSyntax, at: Node<'t>) -> Ty {
        let scope = self.scope_at(at);
        self.index.resolve_type(ts, &scope)
    }

    /// Resolve an annotation node.
    pub fn annotation_info(&self, ann: Node<'t>) -> Option<AnnotationInfo> {
        let raw = stubs::annotation(ann, self.src)?;
        Some(self.index.resolve_annotation(&raw, &self.scope_at(ann)))
    }

    pub fn annotations_of(&self, decl: Node<'t>) -> Vec<AnnotationInfo> {
        let scope = self.scope_at(decl);
        stubs::annotations_of(decl, self.src).iter().map(|a| self.index.resolve_annotation(a, &scope)).collect()
    }

    pub fn has_annotation(&self, decl: Node<'t>, fqn: &str) -> bool {
        self.annotations_of(decl).iter().any(|a| a.is(fqn))
    }

    pub fn local_decl(&self, ident: Node<'t>) -> Option<&LocalDecl<'t>> {
        self.binder.decl_of(ident)
    }

    /// Classes declared in this file (member classes from the index and
    /// local/anonymous ones).
    pub fn declared_classes(&self) -> Vec<(Node<'t>, Arc<ClassInfo>)> {
        let mut v: Vec<(Node<'t>, Arc<ClassInfo>)> = Vec::new();
        for d in self.root().descendants() {
            if let Some(n) = self.class_names.get(&d.id()) {
                if let Some(c) = self.class(n) {
                    v.push((d, c));
                }
            }
        }
        v
    }

    /// All `identifier`/`type_identifier` nodes in the file with this text.
    pub fn occurrences(&self, name: &str) -> &[Node<'t>] {
        let map = self.occurrences.get_or_init(|| {
            let mut m: FxHashMap<&'t str, Vec<Node<'t>>> = FxHashMap::default();
            let src = self.src;
            grounds_syntax::walk(self.file.root(), |n| {
                if matches!(n.kind(), "identifier" | "type_identifier") {
                    m.entry(&src[n.start_byte()..n.end_byte()]).or_default().push(n);
                }
                grounds_syntax::WalkControl::Continue
            });
            m
        });
        map.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Locals declared outside a lambda / anonymous class body but used
    /// inside it (by lambda or class_body node id).
    pub fn captured_locals(&self, n: Node<'t>) -> &[usize] {
        let map = self.captures.get_or_init(|| {
            let mut m: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
            for (i, d) in self.binder.decls.iter().enumerate() {
                let decl_pos = d.name_node.start_byte();
                for u in &self.binder.uses[i] {
                    for a in u.ancestors() {
                        if a.start_byte() <= decl_pos && decl_pos < a.end_byte() {
                            break;
                        }
                        if matches!(a.kind(), "lambda_expression" | "class_body") {
                            let e = m.entry(a.id()).or_default();
                            if !e.contains(&i) {
                                e.push(i);
                            }
                        }
                    }
                }
            }
            m
        });
        map.get(&n.id()).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub(crate) fn guard<R>(&self, id: usize, tag: u8, default: R, f: impl FnOnce() -> R) -> R {
        self.work.set(self.work.get() + 1);
        if !self.in_progress.borrow_mut().insert((id, tag)) {
            self.cycle_hits.set(self.cycle_hits.get() + 1);
            return default;
        }
        // leave the node even when `f` panics: the runner catches a rule's
        // panic and runs the file's other rules on this same model
        struct Leave<'a>(&'a RefCell<FxHashSet<(usize, u8)>>, (usize, u8));
        impl Drop for Leave<'_> {
            fn drop(&mut self) {
                self.0.borrow_mut().remove(&self.1);
            }
        }
        let _leave = Leave(&self.in_progress, (id, tag));
        f()
    }

    /// Run `f` and report whether its result may be memoized: it completed
    /// without hitting a cycle. Once the file has used up its work budget, a
    /// result that did hit a cycle is replaced by "unknown" and memoized, so
    /// provisional answers never pass for definitive ones.
    pub(crate) fn clean<R: Unknowable>(&self, f: impl FnOnce() -> R) -> (R, bool) {
        let before = self.cycle_hits.get();
        let r = f();
        if self.cycle_hits.get() == before {
            (r, true)
        } else if self.work.get() > WORK_BUDGET {
            (R::unknown(), true)
        } else {
            (r, false)
        }
    }
}

/// The "don't know" value of a memoized query.
pub(crate) trait Unknowable {
    fn unknown() -> Self;
}

impl<T> Unknowable for Option<T> {
    fn unknown() -> Self {
        None
    }
}

impl Unknowable for Ty {
    fn unknown() -> Self {
        Ty::Unknown
    }
}

// Ordinary files stay below ~50k (largest seen across the corpus); nested
// Hamcrest matcher trees reach millions.
const WORK_BUDGET: u64 = 300_000;

impl<'t> ClassLookup for FileModel<'t> {
    fn class(&self, name: &str) -> Option<Arc<ClassInfo>> {
        if let Some((raw, node)) = self.local_raw.get(name) {
            if let Some(c) = self.local_lowered.borrow().get(name) {
                return c.clone();
            }
            // guard against recursion while lowering
            self.local_lowered.borrow_mut().insert(Arc::from(name), None);
            let scope = self.scope_at(*node);
            let mut info = self.index.lower_raw_class(raw, &scope, Origin::Source { file: self.path.clone() });
            if raw.flags.has(Flags::ANON) {
                // super type from the creation expression
                if let Some(oce) = node.parent() {
                    if oce.kind() == "object_creation_expression" {
                        if let Some(t) = oce.field("type") {
                            let mut st = self.resolve_type_node(t);
                            // `new TypeReference<>() {}`: inferred arguments, not a raw type
                            let diamond = t.kind() == "generic_type" && t.kid_of_kind("type_arguments").is_some_and(|ta| ta.kids().is_empty());
                            if let (true, Ty::Class { name, args }) = (diamond, &mut st) {
                                if let Some(c) = self.index.class(name) {
                                    *args = c.type_params.iter().map(|_| Ty::Unknown).collect();
                                }
                            }
                            let is_iface = st.class_name().and_then(|n| self.index.class(n)).is_some_and(|c| c.is_interface());
                            if is_iface {
                                info.interfaces = vec![st];
                                info.superclass = Some(Ty::object());
                            } else if st.is_known() {
                                info.superclass = Some(st);
                            } else {
                                info.superclass = Some(Ty::Unknown);
                            }
                        }
                    } else if oce.kind() == "enum_constant" {
                        if let Some(e) = self.enclosing_class_name(oce) {
                            info.superclass = Some(Ty::class(&e));
                        }
                    }
                }
            }
            let arc = Arc::new(info);
            self.local_lowered.borrow_mut().insert(Arc::from(name), Some(arc.clone()));
            return Some(arc);
        }
        // a class declared in this file (possibly duplicated elsewhere) comes from this file
        if self.index.is_duplicated(name) {
            if let Some(raw) = self.stub.classes.iter().find(|c| &*c.name == name) {
                if let Some(c) = self.local_lowered.borrow().get(name) {
                    return c.clone();
                }
                self.local_lowered.borrow_mut().insert(Arc::from(name), None);
                let scope = self.index.scope_for_class(name).unwrap_or_default();
                let info = self.index.lower_raw_class(raw, &Scope { package: &self.stub.package, imports: &self.stub.imports, ..scope }, Origin::Source { file: self.path.clone() });
                let arc = Arc::new(info);
                self.local_lowered.borrow_mut().insert(Arc::from(name), Some(arc.clone()));
                return Some(arc);
            }
        }
        self.index.class(name)
    }
}
