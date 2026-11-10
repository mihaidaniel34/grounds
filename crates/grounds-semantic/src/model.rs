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
    /// The overloads that may be called agree on the result: `ret` stands
    /// even where `ambiguous` says `method` may be another one of them.
    pub same_result: bool,
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
    /// see `local_class_scope`; None while it is being computed
    local_scopes: RefCell<FxHashMap<Name, Option<LocalClassScope>>>,
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
    /// node id -> parent, built in one pass on first use: tree-sitter's
    /// `parent()` walks down from the root on every call
    parents: std::cell::OnceCell<FxHashMap<usize, Node<'t>>>,
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
            local_scopes: RefCell::new(FxHashMap::default()),
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
            parents: std::cell::OnceCell::new(),
        };
        m.collect_classes();
        m.inherited_fields_shadow_locals();
        m
    }

    /// Inside a class body, a field the class inherits shadows a local of
    /// the enclosing method (JLS 6.4.1): such references are fields, not
    /// locals.
    fn inherited_fields_shadow_locals(&mut self) {
        let crossed = std::mem::take(&mut self.binder.crossed);
        for (ident, classes) in &crossed {
            let name = ident.text(self.src);
            let mut is_field = false;
            for c in classes {
                let Some(cn) = self.class_names.get(&c.id()).cloned() else { continue };
                let Some(ci) = self.class(&cn) else { break };
                // a private field of a supertype is not inherited, nor a
                // package-private one of another package (JLS 8.2)
                let package = |n: &str| n.split('$').next().unwrap_or(n).rsplit_once('.').map(|(p, _)| p.to_string()).unwrap_or_default();
                let inherited = |f: &FieldRef| {
                    f.owner == cn || (!f.field.flags.is_private() && (f.field.flags.is_public() || f.field.flags.has(Flags::PROTECTED) || package(&f.owner) == package(&cn)))
                };
                if self.field(&ci.this_type(), name).is_some_and(|f| inherited(&f)) {
                    is_field = true;
                    break;
                }
                // a supertype that is not resolved may declare the field: that
                // stays unknown, and the name a local, as other rules (unused
                // fields and locals) would count an unknown name as no use
                if !self.hierarchy_complete(&cn) {
                    break;
                }
            }
            if is_field {
                if let Some(i) = self.binder.refs.remove(&ident.id()) {
                    self.binder.uses[i].retain(|u| u.id() != ident.id());
                }
            }
        }
        self.binder.crossed = crossed;
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
                    let lombok = m.stub.lombok.clone().unwrap_or_default();
                    let raws = stubs::extract_class_node(n, src, pkg, Some(&o), &m.stub.imports, &lombok);
                    // extract_class_node names it Outer$Simple: rename it, and
                    // the classes nested in it, under its binary name
                    let old = format!("{o}${simple}");
                    let rename = |x: &Name| -> Name {
                        match x.strip_prefix(old.as_str()) {
                            Some(rest) if rest.is_empty() || rest.starts_with('$') => Arc::from(format!("{nm}{rest}").as_str()),
                            _ => x.clone(),
                        }
                    };
                    for mut r in raws {
                        r.name = rename(&r.name);
                        r.outer = r.outer.as_ref().map(rename);
                        r.inners = r.inners.iter().map(rename).collect();
                        let node = if r.name == nm { Some(n) } else { declaration_at(n, r.decl_range) };
                        // a class Lombok generates has no node: it sits in the local class
                        m.local_raw.insert(r.name.clone(), (r, node.unwrap_or(n)));
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
                let raws = stubs::extract_anonymous(n, src, pkg, &o, &simple);
                for mut r in raws {
                    if r.name == nm {
                        r.flags.set(Flags::ANON);
                        m.local_raw.insert(nm.clone(), (r, n));
                    } else if let Some(d) = declaration_at(n, r.decl_range) {
                        // a member class of the anonymous class
                        m.local_raw.insert(r.name.clone(), (r, d));
                    }
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

    /// What the local or anonymous class `name` brings into scope in its body:
    /// its member types and the supertypes it inherits member types from.
    /// None for a class the index holds, or while the class's own header is
    /// being resolved.
    fn local_class_scope(&self, name: &Name) -> Option<LocalClassScope> {
        let (raw, node) = self.local_raw.get(name)?;
        if let Some(s) = self.local_scopes.borrow().get(name) {
            return s.clone();
        }
        self.local_scopes.borrow_mut().insert(name.clone(), None);
        let mut lc = LocalClassScope { name: name.clone(), ..Default::default() };
        let prefix = format!("{name}$");
        for k in self.local_raw.keys() {
            if let Some(simple) = k.strip_prefix(prefix.as_str()).filter(|s| !s.contains('$')) {
                lc.members.push((Arc::from(simple), k.clone()));
            }
        }
        lc.members.sort();
        // the supertypes, resolved where the class header is (outside its body)
        let supers: Vec<Ty> = if raw.flags.has(Flags::ANON) {
            match self.parent_of(*node) {
                Some(oce) if oce.kind() == "object_creation_expression" => vec![self.created_class(oce)],
                Some(ec) if ec.kind() == "enum_constant" => self.enclosing_class_name(ec).map(|e| Ty::class(&e)).into_iter().collect(),
                _ => vec![Ty::Unknown],
            }
        } else {
            let mut scope = self.parent_of(*node).map(|p| self.scope_at(p)).unwrap_or_default();
            scope.type_vars.extend(raw.type_params.iter().map(|t| t.name.clone()));
            raw.superclass.iter().chain(raw.interfaces.iter()).map(|t| self.index.resolve_type(t, &scope)).collect()
        };
        for t in supers {
            match t {
                Ty::Class { name: s, .. } => match self.local_class_scope(&s) {
                    // a local supertype: what it declares and inherits
                    Some(sl) => {
                        // a private member type is not inherited (JLS 8.2, 8.5)
                        let private = |b: &Name| self.local_raw.get(b).is_some_and(|(r, _)| r.flags.is_private());
                        lc.members.extend(sl.members.iter().filter(|(m, b)| !private(b) && !lc.members.iter().any(|(x, _)| x == m)).cloned().collect::<Vec<_>>());
                        lc.supers.extend(sl.supers);
                        lc.unknown_super |= sl.unknown_super;
                    }
                    None if self.local_raw.contains_key(&s) => lc.unknown_super = true,
                    None => lc.supers.push(s),
                },
                _ => lc.unknown_super = true,
            }
        }
        self.local_scopes.borrow_mut().insert(name.clone(), Some(lc.clone()));
        Some(lc)
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

    /// The parent of a node of this file (`Node::parent` without its walk
    /// from the root).
    pub fn parent_of(&self, n: Node<'t>) -> Option<Node<'t>> {
        let parents = self.parents.get_or_init(|| {
            let mut map = FxHashMap::default();
            let mut cursor = self.file.root().walk();
            let mut stack: Vec<Node<'t>> = Vec::new();
            loop {
                let node = cursor.node();
                if let Some(&p) = stack.last() {
                    map.insert(node.id(), p);
                }
                if cursor.goto_first_child() {
                    stack.push(node);
                    continue;
                }
                loop {
                    if cursor.goto_next_sibling() {
                        break;
                    }
                    if !cursor.goto_parent() {
                        return map;
                    }
                    stack.pop();
                }
            }
        });
        parents.get(&n.id()).copied()
    }

    /// Ancestors of a node of this file, innermost first (`NodeExt::ancestors`
    /// through the parent table).
    pub fn ancestors_of(&self, n: Node<'t>) -> impl Iterator<Item = Node<'t>> + '_ {
        std::iter::successors(self.parent_of(n), move |&a| self.parent_of(a))
    }

    /// The innermost type declaration or anonymous class body enclosing `n`.
    pub fn enclosing_class_node(&self, n: Node<'t>) -> Option<Node<'t>> {
        self.ancestors_of(n).find(|a| self.class_names.contains_key(&a.id()))
    }

    pub fn enclosing_class_name(&self, n: Node<'t>) -> Option<Name> {
        self.enclosing_class_node(n).and_then(|c| self.class_names.get(&c.id()).cloned())
    }

    pub fn enclosing_class(&self, n: Node<'t>) -> Option<Arc<ClassInfo>> {
        self.enclosing_class_name(n).and_then(|c| self.class(&c))
    }

    /// Enclosing method, constructor or lambda.
    pub fn enclosing_callable(&self, n: Node<'t>) -> Option<Node<'t>> {
        self.ancestors_of(n).take_while(|a| !self.class_names.contains_key(&a.id())).find(|a| grounds_syntax::is_method_like(a.kind()))
    }

    /// Enclosing method or constructor declaration (skipping lambdas).
    pub fn enclosing_method(&self, n: Node<'t>) -> Option<Node<'t>> {
        self.ancestors_of(n)
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
        // the byte ranges of the enclosing classes, for the local classes' levels
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        let mut type_vars = Vec::new();
        let mut prev: Option<Node<'t>> = None;
        for a in std::iter::once(n).chain(self.ancestors_of(n)) {
            // a class's members are in scope in its body (JLS 6.3), not in
            // its header: the annotations, the extends and implements clauses
            let header = prev.is_some_and(|p| grounds_syntax::is_type_declaration(a.kind()) && a.field("body") != Some(p));
            prev = Some(a);
            if let Some(name) = self.class_names.get(&a.id()).filter(|_| !header) {
                enclosing.push(name.clone());
                ranges.push((a.start_byte(), a.end_byte()));
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
        // a local class is shadowed by the member types of the classes declared
        // inside the block that declares it (JLS 6.4.1)
        let local_types = locals.into_iter().map(|(s, b, (a, e))| (s.clone(), b.clone(), ranges.iter().filter(|(x, y)| a <= x && y <= e).count())).collect();
        let local_classes = enclosing.iter().filter_map(|e| self.local_class_scope(e)).collect();
        Scope { package: &self.stub.package, imports: &self.stub.imports, enclosing, type_vars, local_types, local_classes, unknown_if_inherited: true }
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
        let Some(body) = self.parent_of(component).filter(|b| b.kind() == "record_pattern_body") else { return Ty::Unknown };
        let parts: Vec<Node> = body.kids().into_iter().filter(|k| matches!(k.kind(), "record_pattern_component" | "record_pattern")).collect();
        let Some(pos) = parts.iter().position(|k| *k == component) else { return Ty::Unknown };
        let Some(type_node) = self.parent_of(body).and_then(|p| p.kids().into_iter().find(|k| k.kind() != "record_pattern_body")) else { return Ty::Unknown };
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
        // a type declaration's annotations are outside its body
        let at = if grounds_syntax::is_type_declaration(decl.kind()) { self.parent_of(decl).unwrap_or(decl) } else { decl };
        let scope = self.scope_at(at);
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
                    for a in self.ancestors_of(*u) {
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

/// The type declaration in `root` (or `root` itself) at `range`.
fn declaration_at(root: Node<'_>, range: (u32, u32)) -> Option<Node<'_>> {
    root.descendants().find(|d| grounds_syntax::is_type_declaration(d.kind()) && (d.start_byte() as u32, d.end_byte() as u32) == range)
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

impl<'t> FileModel<'t> {
    /// Whether the members of `name` as this file sees them are certain: a
    /// class declared once, or in several files with one copy this file
    /// certainly compiles against (its own, or the one of its module).
    pub fn copy_is_certain(&self, name: &str) -> bool {
        !self.index.is_duplicated(name) || self.stub.classes.iter().any(|c| &*c.name == name) || self.index.certain_copy(name, &self.path).is_some()
    }
}

impl<'t> ClassLookup for FileModel<'t> {
    fn may_lack_members(&self, name: &str) -> bool {
        self.index.may_lack_members(name)
    }

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
                if let Some(oce) = self.parent_of(*node) {
                    if oce.kind() == "object_creation_expression" {
                        if let Some(t) = oce.field("type") {
                            let mut st = self.created_class(oce);
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
        // a class declared in several files: this file's own copy, else the
        // copy this file certainly compiles against (see `copy_is_certain`)
        if self.index.is_duplicated(name) {
            let own = self.stub.classes.iter().find(|c| &*c.name == name).map(|raw| (&self.stub, raw, &self.path));
            if let Some((stub, raw, file)) = own.or_else(|| self.index.certain_copy(name, &self.path)) {
                if let Some(c) = self.local_lowered.borrow().get(name) {
                    return c.clone();
                }
                self.local_lowered.borrow_mut().insert(Arc::from(name), None);
                let scope = self.index.scope_for_raw(stub, raw);
                let info = self.index.lower_raw_class(raw, &scope, Origin::Source { file: file.clone() });
                let arc = Arc::new(info);
                self.local_lowered.borrow_mut().insert(Arc::from(name), Some(arc.clone()));
                return Some(arc);
            }
        }
        self.index.class(name)
    }
}
