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
                let a = self.eval(v.field("consequence")?, f);
                let b = self.eval(v.field("alternative")?, f);
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
                if op == "==" {
                    out.push((i, Nullness::Null, true));
                    out.push((i, Nullness::NonNull, false));
                } else {
                    out.push((i, Nullness::NonNull, true));
                    out.push((i, Nullness::Null, false));
                }
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
                            out.push((i, if is_null { Nullness::Null } else { Nullness::NonNull }, true));
                            out.push((i, if is_null { Nullness::NonNull } else { Nullness::Null }, false));
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
            "method_invocation" => push(e.field("object")),
            "field_access" => {
                if e.field("field").is_some_and(|f| f.kind() != "this") {
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
            EdgeKind::Exception => return Some(fact.clone()),
            EdgeKind::Normal => return Some(fact.clone()),
        };
        // constant conditions: prune impossible edges
        if let Some(Constant::Bool(b)) = self.fm.constant_value(c) {
            if b != truth {
                return None;
            }
        }
        let mut f = fact.clone();
        for (i, s, on_true) in self.cond_effect(c) {
            if on_true == truth {
                // contradiction: known non-null but branch requires null
                f.insert(i, s);
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
            "method_invocation" => self.fm.call_matches_any(v, RESOURCE_FACTORIES),
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

pub type PresentFact = FxHashSet<usize>;

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

impl<'a, 't> Analysis<'t> for OptionalPresence<'a, 't> {
    type Fact = PresentFact;
    fn bottom(&self) -> PresentFact {
        FxHashSet::default()
    }
    fn boundary(&self) -> PresentFact {
        FxHashSet::default()
    }
    fn join(&self, a: &PresentFact, b: &PresentFact) -> PresentFact {
        a.intersection(b).copied().collect()
    }
    fn transfer(&self, e: Node<'t>, f: &mut PresentFact) {
        if let Some((i, _)) = local_def(self.fm, e) {
            f.remove(&i);
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
            }
        }
    }
    fn edge(&self, kind: &EdgeKind<'t>, fact: &PresentFact) -> Option<PresentFact> {
        let (c, truth) = match kind {
            EdgeKind::True(c) => (*c, true),
            EdgeKind::False(c) => (*c, false),
            _ => return Some(fact.clone()),
        };
        let c = c.unparen();
        if c.kind() == "method_invocation" {
            if let Some(i) = self.optional_local(c.field("object")) {
                match (self.fm.call_name(c), truth) {
                    (Some("isPresent"), true) | (Some("isEmpty"), false) => {
                        let mut f = fact.clone();
                        f.insert(i);
                        return Some(f);
                    }
                    _ => {}
                }
            }
        }
        Some(fact.clone())
    }
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
