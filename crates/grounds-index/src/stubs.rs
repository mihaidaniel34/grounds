//! Declaration stubs extracted from source files. Stubs hold unresolved type
//! syntax; they are resolved to [`crate::model`] types lazily by the index
//! (resolution needs the whole-program name table).
//!
//! Stubs are a pure function of a file's text so they can be cached on disk
//! keyed by content hash.

use crate::model::*;
use grounds_syntax::{Node, NodeExt, ParsedFile};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TypeSyntax {
    Prim(Prim),
    Void,
    /// `var` in local declarations / lambda params.
    Var,
    /// `Map.Entry<K, V>` -> segments [("Map", []), ("Entry", [K, V])]
    Named(Vec<(Name, Vec<TypeSyntax>)>),
    Array(Box<TypeSyntax>),
    /// `?`, `? extends T` (true), `? super T` (false)
    Wildcard(Option<(bool, Box<TypeSyntax>)>),
    Unknown,
}

impl TypeSyntax {
    pub fn simple(name: &str) -> TypeSyntax {
        TypeSyntax::Named(vec![(Arc::from(name), Vec::new())])
    }
    pub fn qualified(path: &str) -> TypeSyntax {
        TypeSyntax::Named(path.split('.').map(|s| (Arc::from(s), Vec::new())).collect())
    }
    pub fn last_name(&self) -> Option<&str> {
        match self {
            TypeSyntax::Named(segs) => segs.last().map(|s| &*s.0),
            TypeSyntax::Array(e) => e.last_name(),
            _ => None,
        }
    }
    pub fn is_boolean_prim(&self) -> bool {
        matches!(self, TypeSyntax::Prim(Prim::Boolean))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RawAnnValue {
    Const(Constant),
    /// A (possibly qualified) name such as `ElementType.TYPE` or `CONST`.
    Name(Arc<str>),
    ClassLit(TypeSyntax),
    Annotation(Box<RawAnnotation>),
    Array(Vec<RawAnnValue>),
    Expr(Arc<str>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawAnnotation {
    /// As written: `Autowired` or `org.springframework.beans.factory.annotation.Autowired`
    pub name: Name,
    pub args: Vec<(Name, RawAnnValue)>,
    pub range: (u32, u32),
}

impl RawAnnotation {
    pub fn simple_name(&self) -> &str {
        self.name.rsplit('.').next().unwrap_or(&self.name)
    }
    pub fn arg(&self, k: &str) -> Option<&RawAnnValue> {
        self.args.iter().find(|(n, _)| &**n == k).map(|(_, v)| v)
    }
    pub fn bool_arg(&self, k: &str) -> Option<bool> {
        match self.arg(k)? {
            RawAnnValue::Const(Constant::Bool(b)) => Some(*b),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawTypeParam {
    pub name: Name,
    pub bounds: Vec<TypeSyntax>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawParam {
    pub name: Name,
    pub ty: TypeSyntax,
    pub annotations: Vec<RawAnnotation>,
    pub flags: Flags,
}

/// Constant expression of a final field initializer that refers to other
/// constants; evaluated during index resolution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConstExpr {
    Lit(Constant),
    Name(Arc<str>),
    Unary(Arc<str>, Box<ConstExpr>),
    Binary(Arc<str>, Box<ConstExpr>, Box<ConstExpr>),
    Cast(Prim, Box<ConstExpr>),
    CastString(Box<ConstExpr>),
    Ternary(Box<ConstExpr>, Box<ConstExpr>, Box<ConstExpr>),
}

pub fn const_expr(n: Node, src: &str) -> Option<ConstExpr> {
    let n = n.unparen();
    if let Some(c) = literal_constant(n, src) {
        return Some(ConstExpr::Lit(c));
    }
    match n.kind() {
        "identifier" => Some(ConstExpr::Name(Arc::from(n.text(src)))),
        "field_access" if is_name_chain(n) => Some(ConstExpr::Name(Arc::from(strip_ws(n.text(src)).as_str()))),
        "unary_expression" => Some(ConstExpr::Unary(Arc::from(n.field("operator")?.kind()), Box::new(const_expr(n.field("operand")?, src)?))),
        "binary_expression" => Some(ConstExpr::Binary(
            Arc::from(n.field("operator")?.kind()),
            Box::new(const_expr(n.field("left")?, src)?),
            Box::new(const_expr(n.field("right")?, src)?),
        )),
        "cast_expression" => {
            let t = n.fields("type");
            let v = Box::new(const_expr(n.field("value")?, src)?);
            match t.first().map(|t| type_syntax(*t, src)) {
                Some(TypeSyntax::Prim(p)) => Some(ConstExpr::Cast(p, v)),
                Some(ts) if ts.last_name() == Some("String") => Some(ConstExpr::CastString(v)),
                _ => None,
            }
        }
        "ternary_expression" => Some(ConstExpr::Ternary(
            Box::new(const_expr(n.field("condition")?, src)?),
            Box::new(const_expr(n.field("consequence")?, src)?),
            Box::new(const_expr(n.field("alternative")?, src)?),
        )),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawField {
    pub name: Name,
    pub ty: TypeSyntax,
    pub flags: Flags,
    pub annotations: Vec<RawAnnotation>,
    pub constant: Option<Constant>,
    pub init: Option<ConstExpr>,
    pub has_initializer: bool,
    pub range: (u32, u32),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawMethod {
    pub name: Name,
    pub flags: Flags,
    pub type_params: Vec<RawTypeParam>,
    pub params: Vec<RawParam>,
    pub ret: TypeSyntax,
    pub throws: Vec<TypeSyntax>,
    pub annotations: Vec<RawAnnotation>,
    pub range: (u32, u32),
    pub generated_by: Option<Name>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawClass {
    pub name: Name,
    pub kind: ClassKind,
    pub flags: Flags,
    pub type_params: Vec<RawTypeParam>,
    pub superclass: Option<TypeSyntax>,
    pub interfaces: Vec<TypeSyntax>,
    pub permits: Vec<TypeSyntax>,
    pub fields: Vec<RawField>,
    pub methods: Vec<RawMethod>,
    pub annotations: Vec<RawAnnotation>,
    pub outer: Option<Name>,
    pub inners: Vec<Name>,
    pub range: (u32, u32),
    /// Byte range of the whole declaration (for mapping nodes to classes).
    pub decl_range: (u32, u32),
}

impl RawClass {
    pub fn simple_name(&self) -> &str {
        simple_name_of(&self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Import {
    /// Dotted path without `.*`.
    pub path: Name,
    pub is_static: bool,
    pub on_demand: bool,
}

impl Import {
    pub fn last(&self) -> &str {
        self.path.rsplit('.').next().unwrap_or(&self.path)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FileStub {
    pub package: Name,
    pub imports: Vec<Import>,
    pub classes: Vec<RawClass>,
    /// Every simple identifier mentioned in the file (sorted, deduped). Used
    /// for cheap reverse-dependency queries.
    pub referenced_names: Vec<Name>,
    /// Type names, as written (simple or qualified), that this file
    /// instantiates (`new X(...)`, anonymous classes included) or extends
    /// (sorted, deduped).
    pub constructed_names: Vec<Name>,
    /// Hash of the declaration shape, excluding source positions. Body-only
    /// edits keep this stable.
    pub signature_hash: u64,
    pub has_syntax_errors: bool,
}

pub fn extract(file: &ParsedFile) -> FileStub {
    extract_with(file, &crate::lombok::LombokConfig::default())
}

/// Extract with the `lombok.config` settings that apply to the file.
pub fn extract_with(file: &ParsedFile, lombok: &crate::lombok::LombokConfig) -> FileStub {
    let src = &*file.text;
    let root = file.root();
    let mut stub = FileStub { package: Arc::from(""), has_syntax_errors: file.has_errors(), ..Default::default() };
    for ch in root.kids() {
        match ch.kind() {
            "package_declaration" => {
                if let Some(n) = ch.kids().into_iter().find(|k| matches!(k.kind(), "identifier" | "scoped_identifier")) {
                    stub.package = Arc::from(strip_ws(n.text(src)).as_str());
                }
            }
            "import_declaration" => {
                let mut c = ch.walk();
                let is_static = ch.children(&mut c).any(|k| !k.is_named() && k.kind() == "static");
                let on_demand = ch.kid_of_kind("asterisk").is_some();
                if let Some(n) = ch.kids().into_iter().find(|k| matches!(k.kind(), "identifier" | "scoped_identifier")) {
                    stub.imports.push(Import { path: Arc::from(strip_ws(n.text(src)).as_str()), is_static, on_demand });
                }
            }
            _ => {}
        }
    }
    let pkg = stub.package.clone();
    let mut ex = Extractor { src, classes: Vec::new() };
    for ch in root.kids() {
        if grounds_syntax::is_type_declaration(ch.kind()) {
            ex.class(ch, &pkg, None, false);
        }
    }
    stub.classes = ex.classes;
    crate::lombok::apply_all_with(&mut stub.classes, &stub.imports, lombok);
    // referenced names
    let mut names: Vec<Name> = Vec::new();
    let mut seen = rustc_hash::FxHashSet::default();
    grounds_syntax::walk(root, |n| {
        if matches!(n.kind(), "identifier" | "type_identifier") {
            let t = n.text(src);
            if seen.insert(t) {
                names.push(Arc::from(t));
            }
        }
        grounds_syntax::WalkControl::Continue
    });
    names.sort();
    stub.referenced_names = names;
    let mut constructed: Vec<Name> = Vec::new();
    grounds_syntax::walk(root, |n| {
        let ty = match n.kind() {
            "object_creation_expression" => n.child_by_field_name("type"),
            "superclass" => n.named_child(0),
            _ => None,
        };
        if let Some(mut t) = ty {
            if t.kind() == "generic_type" {
                t = t.named_child(0).unwrap_or(t);
            }
            if let Ok(text) = t.utf8_text(src.as_bytes()) {
                constructed.push(Arc::from(text.chars().filter(|c| !c.is_whitespace()).collect::<String>().as_str()));
            }
        }
        grounds_syntax::WalkControl::Continue
    });
    constructed.sort();
    constructed.dedup();
    stub.constructed_names = constructed;
    stub.signature_hash = signature_hash(&stub);
    stub
}

fn signature_hash(stub: &FileStub) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    stub.package.hash(&mut h);
    stub.imports.hash(&mut h);
    for c in &stub.classes {
        let mut c = c.clone();
        c.range = (0, 0);
        c.decl_range = (0, 0);
        for f in &mut c.fields {
            f.range = (0, 0);
            for a in &mut f.annotations {
                a.range = (0, 0);
            }
        }
        for m in &mut c.methods {
            m.range = (0, 0);
            for a in &mut m.annotations {
                a.range = (0, 0);
            }
            for p in &mut m.params {
                for a in &mut p.annotations {
                    a.range = (0, 0);
                }
            }
        }
        for a in &mut c.annotations {
            a.range = (0, 0);
        }
        format!("{c:?}").hash(&mut h);
    }
    h.finish()
}

pub fn strip_ws(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn range(n: Node) -> (u32, u32) {
    (n.start_byte() as u32, n.end_byte() as u32)
}

pub(crate) struct Extractor<'s> {
    pub src: &'s str,
    pub classes: Vec<RawClass>,
}

pub fn modifier_flags(node: Node) -> Flags {
    let mut f = Flags::default();
    if let Some(m) = node.modifiers() {
        let mut c = m.walk();
        for k in m.children(&mut c) {
            let Some(kw) = grounds_syntax::modifier_keyword(k) else { continue };
            f.set(match kw {
                "public" => Flags::PUBLIC,
                "private" => Flags::PRIVATE,
                "protected" => Flags::PROTECTED,
                "static" => Flags::STATIC,
                "final" => Flags::FINAL,
                "abstract" => Flags::ABSTRACT,
                "synchronized" => Flags::SYNCHRONIZED,
                "volatile" => Flags::VOLATILE,
                "transient" => Flags::TRANSIENT,
                "native" => Flags::NATIVE,
                "default" => Flags::DEFAULT,
                "strictfp" => Flags::STRICTFP,
                "sealed" => Flags::SEALED,
                "non-sealed" => Flags::NON_SEALED,
                _ => 0,
            });
        }
    }
    f
}

pub fn annotations_of(node: Node, src: &str) -> Vec<RawAnnotation> {
    let mut out = Vec::new();
    if let Some(m) = node.modifiers() {
        for k in m.kids() {
            if let Some(a) = annotation(k, src) {
                out.push(a);
            }
        }
    }
    // method_declaration may have annotations as direct children too
    for k in node.kids() {
        if matches!(k.kind(), "annotation" | "marker_annotation") {
            if let Some(a) = annotation(k, src) {
                out.push(a);
            }
        }
    }
    out
}

pub fn annotation(n: Node, src: &str) -> Option<RawAnnotation> {
    match n.kind() {
        "marker_annotation" => {
            let name = n.field("name")?;
            Some(RawAnnotation { name: Arc::from(strip_ws(name.text(src)).as_str()), args: Vec::new(), range: range(n) })
        }
        "annotation" => {
            let name = n.field("name")?;
            let mut args = Vec::new();
            if let Some(al) = n.field("arguments") {
                for a in al.kids() {
                    if a.kind() == "element_value_pair" {
                        if let (Some(k), Some(v)) = (a.field("key"), a.field("value")) {
                            args.push((Arc::from(k.text(src)), ann_value(v, src)));
                        }
                    } else {
                        args.push((Arc::from("value"), ann_value(a, src)));
                    }
                }
            }
            Some(RawAnnotation { name: Arc::from(strip_ws(name.text(src)).as_str()), args, range: range(n) })
        }
        _ => None,
    }
}

fn ann_value(n: Node, src: &str) -> RawAnnValue {
    match n.kind() {
        "element_value_array_initializer" | "array_initializer" => RawAnnValue::Array(n.kids().into_iter().map(|k| ann_value(k, src)).collect()),
        "annotation" | "marker_annotation" => match annotation(n, src) {
            Some(a) => RawAnnValue::Annotation(Box::new(a)),
            None => RawAnnValue::Expr(Arc::from(n.text(src))),
        },
        "identifier" | "field_access" | "scoped_identifier" => {
            if n.kind() == "field_access" && !is_name_chain(n) {
                return const_or_expr(n, src);
            }
            RawAnnValue::Name(Arc::from(strip_ws(n.text(src)).as_str()))
        }
        "class_literal" => match n.first_kid() {
            Some(t) => RawAnnValue::ClassLit(type_syntax(t, src)),
            None => RawAnnValue::Expr(Arc::from(n.text(src))),
        },
        _ => const_or_expr(n, src),
    }
}

fn is_name_chain(n: Node) -> bool {
    match n.kind() {
        "identifier" => true,
        "field_access" => n.field("object").is_some_and(is_name_chain) && n.field("field").is_some_and(|f| f.kind() == "identifier"),
        _ => false,
    }
}

fn const_or_expr(n: Node, src: &str) -> RawAnnValue {
    match literal_constant(n, src) {
        Some(c) => RawAnnValue::Const(c),
        None => RawAnnValue::Expr(Arc::from(n.text(src))),
    }
}

/// Evaluate constant expressions built only from literals.
pub fn literal_constant(n: Node, src: &str) -> Option<Constant> {
    let n = n.unparen();
    match n.kind() {
        "string_literal" => grounds_syntax::string_literal_value(n, src).map(|s| Constant::Str(Arc::from(s.as_str()))),
        "true" => Some(Constant::Bool(true)),
        "false" => Some(Constant::Bool(false)),
        "character_literal" => grounds_syntax::char_literal_value(n, src).map(Constant::Char),
        "decimal_integer_literal" | "hex_integer_literal" | "octal_integer_literal" | "binary_integer_literal" => {
            let text = n.text(src);
            let v = parse_int_literal(text)?;
            // `int` literals wrap to 32 bits (`0xFFFFFFFF` is -1)
            Some(if text.ends_with(['l', 'L']) { Constant::Long(v) } else { Constant::Int(v as i32 as i64) })
        }
        "decimal_floating_point_literal" | "hex_floating_point_literal" => {
            let t = n.text(src).replace('_', "");
            let t = t.trim_end_matches(['f', 'F', 'd', 'D']);
            t.parse::<f64>().ok().map(Constant::Float)
        }
        "unary_expression" => {
            let op = n.field("operator")?.kind();
            let v = literal_constant(n.field("operand")?, src)?;
            fold_unary(op, v)
        }
        "binary_expression" => {
            let op = n.field("operator")?.kind();
            let l = literal_constant(n.field("left")?, src)?;
            let r = literal_constant(n.field("right")?, src)?;
            fold_binary(op, &l, &r)
        }
        _ => None,
    }
}

/// Unary operator on a constant, with Java's int/long semantics.
pub fn fold_unary(op: &str, v: Constant) -> Option<Constant> {
    use Constant::*;
    let int = |i: i64| Int(i as i32 as i64);
    match (op, v) {
        ("-", Int(i)) => Some(int((i as i32).wrapping_neg() as i64)),
        ("-", Char(c)) => Some(Int(-(c as i64))),
        ("-", Long(i)) => Some(Long(i.wrapping_neg())),
        ("-", Float(f)) => Some(Float(-f)),
        ("+", Char(c)) => Some(Int(c as i64)),
        ("+", v) => Some(v),
        ("!", Bool(b)) => Some(Bool(!b)),
        ("~", Int(i)) => Some(int(!i)),
        ("~", Char(c)) => Some(int(!(c as i64))),
        ("~", Long(i)) => Some(Long(!i)),
        _ => None,
    }
}

/// Binary operator on constants, with Java's semantics: `int` arithmetic
/// wraps at 32 bits, mixed operands promote to `long`/`double`, and a shift
/// takes the type of its left operand and masks the distance to 5 or 6 bits.
pub fn fold_binary(op: &str, l: &Constant, r: &Constant) -> Option<Constant> {
    use Constant::*;
    if op == "+" {
        if let (Str(a), b) = (l, r) {
            return Some(Str(Arc::from(format!("{}{}", a, const_to_string(b)).as_str())));
        }
        if let (a, Str(b)) = (l, r) {
            return Some(Str(Arc::from(format!("{}{}", const_to_string(a), b).as_str())));
        }
    }
    let integral = |c: &Constant| matches!(c, Int(_) | Long(_) | Char(_));
    if integral(l) && integral(r) {
        let a = l.as_i64()?;
        let b = r.as_i64()?;
        if matches!(op, "<<" | ">>" | ">>>") {
            return Some(if matches!(l, Long(_)) {
                let d = (b & 63) as u32;
                Long(match op {
                    "<<" => a.wrapping_shl(d),
                    ">>" => a >> d,
                    _ => ((a as u64) >> d) as i64,
                })
            } else {
                let (a, d) = (a as i32, (b & 31) as u32);
                Int(match op {
                    "<<" => a.wrapping_shl(d),
                    ">>" => a >> d,
                    _ => ((a as u32) >> d) as i32,
                } as i64)
            });
        }
        let long = matches!(l, Long(_)) || matches!(r, Long(_));
        let wrap = |v: i64| if long { Long(v) } else { Int(v as i32 as i64) };
        return Some(match op {
            "+" => wrap(a.wrapping_add(b)),
            "-" => wrap(a.wrapping_sub(b)),
            "*" => wrap(a.wrapping_mul(b)),
            "/" if b != 0 => wrap(a.wrapping_div(b)),
            "%" if b != 0 => wrap(a.wrapping_rem(b)),
            "&" => wrap(a & b),
            "|" => wrap(a | b),
            "^" => wrap(a ^ b),
            "==" => Bool(a == b),
            "!=" => Bool(a != b),
            "<" => Bool(a < b),
            "<=" => Bool(a <= b),
            ">" => Bool(a > b),
            ">=" => Bool(a >= b),
            _ => return None,
        });
    }
    let numeric = |c: &Constant| matches!(c, Int(_) | Long(_) | Char(_) | Float(_));
    match (l, r) {
        (a, b) if numeric(a) && numeric(b) => {
            let a = a.as_f64()?;
            let b = b.as_f64()?;
            // float and double share one representation here, so comparisons
            // cannot follow Java's float rounding (`16777217L != 16777216f` is
            // false in Java): not folded
            Some(match op {
                "+" => Float(a + b),
                "-" => Float(a - b),
                "*" => Float(a * b),
                "/" => Float(a / b),
                _ => return None,
            })
        }
        (Bool(a), Bool(b)) => Some(match op {
            "&&" | "&" => Bool(*a && *b),
            "||" | "|" => Bool(*a || *b),
            "^" | "!=" => Bool(a != b),
            "==" => Bool(a == b),
            _ => return None,
        }),
        _ => None,
    }
}

pub fn const_to_string(c: &Constant) -> String {
    match c {
        Constant::Int(i) | Constant::Long(i) => i.to_string(),
        Constant::Float(f) => {
            if f.fract() == 0.0 && f.is_finite() {
                format!("{f:.1}")
            } else {
                f.to_string()
            }
        }
        Constant::Bool(b) => b.to_string(),
        Constant::Char(c) => c.to_string(),
        Constant::Str(s) => s.to_string(),
    }
}

pub fn parse_int_literal(text: &str) -> Option<i64> {
    let t = text.replace('_', "");
    let t = t.trim_end_matches(['l', 'L']);
    let (digits, radix) = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        (h, 16)
    } else if let Some(b) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        (b, 2)
    } else if t.len() > 1 && t.starts_with('0') {
        (&t[1..], 8)
    } else {
        (t, 10)
    };
    let v = u64::from_str_radix(digits, radix).ok()?;
    // int literals like 0xFFFFFFFF are negative ints
    if !text.ends_with(['l', 'L']) && radix != 10 && v <= u32::MAX as u64 {
        return Some(v as u32 as i32 as i64);
    }
    Some(v as i64)
}

pub fn type_syntax(n: Node, src: &str) -> TypeSyntax {
    match n.kind() {
        "integral_type" | "floating_point_type" | "boolean_type" => {
            Prim::from_keyword(n.text(src).trim()).map(TypeSyntax::Prim).unwrap_or(TypeSyntax::Unknown)
        }
        "void_type" => TypeSyntax::Void,
        "type_identifier" | "identifier" => {
            let t = n.text(src);
            if t == "var" {
                TypeSyntax::Var
            } else {
                TypeSyntax::Named(vec![(Arc::from(t), Vec::new())])
            }
        }
        "scoped_type_identifier" | "scoped_identifier" => {
            let mut segs = Vec::new();
            scoped_segments(n, src, &mut segs);
            TypeSyntax::Named(segs)
        }
        "generic_type" => {
            let kids = n.kids();
            let mut segs = Vec::new();
            let mut args = Vec::new();
            for k in kids {
                match k.kind() {
                    "type_identifier" => segs.push((Arc::from(k.text(src)), Vec::new())),
                    "scoped_type_identifier" => scoped_segments(k, src, &mut segs),
                    "type_arguments" => {
                        args = k.kids().into_iter().map(|a| type_syntax(a, src)).collect();
                    }
                    _ => {}
                }
            }
            if let Some(last) = segs.last_mut() {
                last.1 = args;
            }
            TypeSyntax::Named(segs)
        }
        "array_type" => {
            let el = n.field("element").map(|e| type_syntax(e, src)).unwrap_or(TypeSyntax::Unknown);
            let dims = n.field("dimensions").map(|d| d.text(src).matches('[').count()).unwrap_or(1);
            wrap_array(el, dims)
        }
        "annotated_type" => n
            .kids()
            .into_iter()
            .find(|k| !matches!(k.kind(), "annotation" | "marker_annotation"))
            .map(|k| type_syntax(k, src))
            .unwrap_or(TypeSyntax::Unknown),
        "wildcard" => {
            let mut c = n.walk();
            let mut kind = None;
            for k in n.children(&mut c) {
                if !k.is_named() && k.kind() == "extends" {
                    kind = Some(true);
                }
                if k.kind() == "super" {
                    kind = Some(false);
                }
            }
            let bound = n.kids().into_iter().find(|k| !matches!(k.kind(), "annotation" | "marker_annotation" | "super"));
            match (kind, bound) {
                (Some(e), Some(b)) => TypeSyntax::Wildcard(Some((e, Box::new(type_syntax(b, src))))),
                _ => TypeSyntax::Wildcard(None),
            }
        }
        _ => TypeSyntax::Unknown,
    }
}

fn scoped_segments(n: Node, src: &str, segs: &mut Vec<(Name, Vec<TypeSyntax>)>) {
    match n.kind() {
        "type_identifier" | "identifier" => segs.push((Arc::from(n.text(src)), Vec::new())),
        "generic_type" => {
            if let TypeSyntax::Named(s) = type_syntax(n, src) {
                segs.extend(s);
            }
        }
        "scoped_type_identifier" | "scoped_identifier" => {
            for k in n.kids() {
                if !matches!(k.kind(), "annotation" | "marker_annotation") {
                    scoped_segments(k, src, segs);
                }
            }
        }
        _ => {}
    }
}

pub fn wrap_array(mut t: TypeSyntax, dims: usize) -> TypeSyntax {
    for _ in 0..dims {
        t = TypeSyntax::Array(Box::new(t));
    }
    t
}

pub fn type_params(n: Option<Node>, src: &str) -> Vec<RawTypeParam> {
    let Some(n) = n else { return Vec::new() };
    n.kids_of_kind("type_parameter")
        .into_iter()
        .filter_map(|tp| {
            let name = tp.kid_of_kind("type_identifier")?;
            let bounds = tp
                .kid_of_kind("type_bound")
                .map(|b| b.kids().into_iter().map(|t| type_syntax(t, src)).collect())
                .unwrap_or_default();
            Some(RawTypeParam { name: Arc::from(name.text(src)), bounds })
        })
        .collect()
}

pub fn formal_params(n: Option<Node>, src: &str) -> (Vec<RawParam>, bool) {
    let mut out = Vec::new();
    let mut varargs = false;
    let Some(n) = n else { return (out, false) };
    for p in n.kids() {
        match p.kind() {
            "formal_parameter" => {
                let mut ty = p.field("type").map(|t| type_syntax(t, src)).unwrap_or(TypeSyntax::Unknown);
                if let Some(d) = p.field("dimensions") {
                    ty = wrap_array(ty, d.text(src).matches('[').count());
                }
                let name = p.field("name").map(|x| x.text(src)).unwrap_or("_");
                out.push(RawParam { name: Arc::from(name), ty, annotations: annotations_of(p, src), flags: modifier_flags(p) });
            }
            "spread_parameter" => {
                varargs = true;
                let t = p.kids().into_iter().find(|k| !matches!(k.kind(), "modifiers" | "variable_declarator" | "annotation" | "marker_annotation"));
                let ty = t.map(|t| type_syntax(t, src)).unwrap_or(TypeSyntax::Unknown);
                let name = p
                    .kid_of_kind("variable_declarator")
                    .and_then(|d| d.field("name"))
                    .map(|x| x.text(src))
                    .unwrap_or("_");
                out.push(RawParam {
                    name: Arc::from(name),
                    ty: TypeSyntax::Array(Box::new(ty)),
                    annotations: annotations_of(p, src),
                    flags: modifier_flags(p),
                });
            }
            _ => {}
        }
    }
    (out, varargs)
}

fn throws_of(n: Node, src: &str) -> Vec<TypeSyntax> {
    n.kid_of_kind("throws").map(|t| t.kids().into_iter().map(|k| type_syntax(k, src)).collect()).unwrap_or_default()
}

impl<'s> Extractor<'s> {
    /// Extract a type declaration and its nested types. Returns the binary name.
    pub fn class(&mut self, node: Node, pkg: &str, outer: Option<(&Name, ClassKind)>, _local: bool) -> Option<Name> {
        self.class_named(node, pkg, outer, None)
    }

    /// `node` is a type declaration, or a `class_body` of an anonymous class
    /// when `name_override` is given.
    pub fn class_named(&mut self, node: Node, pkg: &str, outer: Option<(&Name, ClassKind)>, name_override: Option<&str>) -> Option<Name> {
        let src = self.src;
        let anon = node.kind() == "class_body";
        let simple = match name_override {
            Some(n) => n,
            None => node.field("name")?.text(src),
        };
        let name: Name = match outer {
            Some((o, _)) if anon => Arc::from(format!("{o}${simple}").as_str()),
            Some((o, _)) => Arc::from(format!("{o}${simple}").as_str()),
            None if pkg.is_empty() => Arc::from(simple),
            None => Arc::from(format!("{pkg}.{simple}").as_str()),
        };
        let kind = match node.kind() {
            "interface_declaration" => ClassKind::Interface,
            "enum_declaration" => ClassKind::Enum,
            "record_declaration" => ClassKind::Record,
            "annotation_type_declaration" => ClassKind::Annotation,
            _ => ClassKind::Class,
        };
        let mut flags = modifier_flags(node);
        if let Some((_, okind)) = outer {
            if matches!(okind, ClassKind::Interface | ClassKind::Annotation) {
                flags.set(Flags::PUBLIC | Flags::STATIC);
            }
            if kind != ClassKind::Class {
                flags.set(Flags::STATIC);
            }
        }
        if matches!(kind, ClassKind::Interface | ClassKind::Annotation) {
            flags.set(Flags::ABSTRACT);
        }
        if matches!(kind, ClassKind::Record) {
            flags.set(Flags::FINAL);
        }
        let annotations = annotations_of(node, src);
        if annotations.iter().any(|a| a.simple_name() == "Deprecated") {
            flags.set(Flags::DEPRECATED);
        }
        let tps = type_params(node.field("type_parameters"), src);
        let superclass = match kind {
            ClassKind::Enum => Some(TypeSyntax::Named(vec![
                (Arc::from("java"), vec![]),
                (Arc::from("lang"), vec![]),
                (Arc::from("Enum"), vec![TypeSyntax::Named(vec![(Arc::from(simple), vec![])])]),
            ])),
            ClassKind::Record => Some(TypeSyntax::qualified("java.lang.Record")),
            _ => node.field("superclass").and_then(|s| s.first_kid()).map(|t| type_syntax(t, src)),
        };
        let mut interfaces: Vec<TypeSyntax> = Vec::new();
        let iface_list = node.field("interfaces").or_else(|| node.kid_of_kind("extends_interfaces"));
        if let Some(il) = iface_list {
            if let Some(tl) = il.kid_of_kind("type_list") {
                interfaces = tl.kids().into_iter().map(|t| type_syntax(t, src)).collect();
            }
        }
        if kind == ClassKind::Annotation {
            interfaces.push(TypeSyntax::qualified("java.lang.annotation.Annotation"));
        }
        let permits = node
            .field("permits")
            .and_then(|p| p.kid_of_kind("type_list"))
            .map(|tl| tl.kids().into_iter().map(|t| type_syntax(t, src)).collect())
            .unwrap_or_default();

        let idx = self.classes.len();
        self.classes.push(RawClass {
            name: name.clone(),
            kind,
            flags,
            type_params: tps,
            superclass,
            interfaces,
            permits,
            fields: Vec::new(),
            methods: Vec::new(),
            annotations,
            outer: outer.map(|o| o.0.clone()),
            inners: Vec::new(),
            range: if anon { range(node) } else { range(node.field("name")?) },
            decl_range: range(node),
        });

        let mut fields = Vec::new();
        let mut methods = Vec::new();
        let mut inners = Vec::new();
        let is_iface = matches!(kind, ClassKind::Interface | ClassKind::Annotation);

        // record components
        if kind == ClassKind::Record {
            let (comps, _) = formal_params(node.field("parameters"), src);
            for c in &comps {
                let mut ff = Flags(Flags::PRIVATE | Flags::FINAL | Flags::RECORD_COMPONENT);
                if c.annotations.iter().any(|a| a.simple_name() == "Deprecated") {
                    ff.set(Flags::DEPRECATED);
                }
                fields.push(RawField {
                    name: c.name.clone(),
                    ty: c.ty.clone(),
                    flags: ff,
                    annotations: c.annotations.clone(),
                    constant: None,
                    init: None,
                    has_initializer: false,
                    range: (0, 0),
                });
            }
            if let Some(params) = node.field("parameters") {
                for (i, p) in params.kids().into_iter().filter(|p| matches!(p.kind(), "formal_parameter" | "spread_parameter")).enumerate() {
                    if let (Some(f), Some(nm)) = (fields.get_mut(i), p.field("name").or_else(|| p.kid_of_kind("variable_declarator"))) {
                        f.range = range(nm);
                    }
                }
            }
        }

        let body = if anon { Some(node) } else { node.field("body") };
        let mut members: Vec<Node> = Vec::new();
        if let Some(b) = body {
            for m in b.kids() {
                if m.kind() == "enum_body_declarations" {
                    members.extend(m.kids());
                } else {
                    members.push(m);
                }
            }
        }
        for m in members {
            match m.kind() {
                "enum_constant" => {
                    let Some(n) = m.field("name") else { continue };
                    let mut ff = Flags(Flags::PUBLIC | Flags::STATIC | Flags::FINAL | Flags::ENUM_CONSTANT);
                    let anns = annotations_of(m, src);
                    if anns.iter().any(|a| a.simple_name() == "Deprecated") {
                        ff.set(Flags::DEPRECATED);
                    }
                    fields.push(RawField {
                        name: Arc::from(n.text(src)),
                        ty: TypeSyntax::Named(vec![(Arc::from(simple), vec![])]),
                        flags: ff,
                        annotations: anns,
                        constant: None,
                        init: None,
                        has_initializer: true,
                        range: range(n),
                    });
                }
                "field_declaration" | "constant_declaration" => {
                    let base_ty = m.field("type").map(|t| type_syntax(t, src)).unwrap_or(TypeSyntax::Unknown);
                    let mut ff = modifier_flags(m);
                    if is_iface || m.kind() == "constant_declaration" {
                        ff.set(Flags::PUBLIC | Flags::STATIC | Flags::FINAL);
                    }
                    let anns = annotations_of(m, src);
                    if anns.iter().any(|a| a.simple_name() == "Deprecated") {
                        ff.set(Flags::DEPRECATED);
                    }
                    for d in m.fields("declarator") {
                        let Some(n) = d.field("name") else { continue };
                        let mut ty = base_ty.clone();
                        if let Some(dims) = d.field("dimensions") {
                            ty = wrap_array(ty, dims.text(src).matches('[').count());
                        }
                        let value = d.field("value");
                        let constant = if ff.is_final() { value.and_then(|v| literal_constant(v, src)) } else { None };
                        let init = if ff.is_final() && constant.is_none() { value.and_then(|v| const_expr(v, src)) } else { None };
                        fields.push(RawField {
                            name: Arc::from(n.text(src)),
                            ty,
                            flags: ff,
                            annotations: anns.clone(),
                            constant,
                            init,
                            has_initializer: value.is_some(),
                            range: range(n),
                        });
                    }
                }
                "method_declaration" | "annotation_type_element_declaration" => {
                    let Some(n) = m.field("name") else { continue };
                    let mut ff = modifier_flags(m);
                    let anns = annotations_of(m, src);
                    if anns.iter().any(|a| a.simple_name() == "Deprecated") {
                        ff.set(Flags::DEPRECATED);
                    }
                    let has_body = m.field("body").is_some();
                    if is_iface {
                        if !ff.has(Flags::PRIVATE) {
                            ff.set(Flags::PUBLIC);
                        }
                        if !has_body && !ff.is_static() && !ff.has(Flags::DEFAULT) && !ff.is_private() {
                            ff.set(Flags::ABSTRACT);
                        }
                    }
                    let (params, varargs) = formal_params(m.field("parameters"), src);
                    if varargs {
                        ff.set(Flags::VARARGS);
                    }
                    if m.field("body").is_some_and(always_throws) {
                        ff.set(Flags::ALWAYS_THROWS);
                    }
                    let mut ret = m.field("type").map(|t| type_syntax(t, src)).unwrap_or(TypeSyntax::Unknown);
                    if let Some(d) = m.field("dimensions") {
                        ret = wrap_array(ret, d.text(src).matches('[').count());
                    }
                    methods.push(RawMethod {
                        name: Arc::from(n.text(src)),
                        flags: ff,
                        type_params: type_params(m.field("type_parameters"), src),
                        params,
                        ret,
                        throws: throws_of(m, src),
                        annotations: anns,
                        range: range(n),
                        generated_by: None,
                    });
                }
                "constructor_declaration" | "compact_constructor_declaration" => {
                    let Some(n) = m.field("name") else { continue };
                    let mut ff = modifier_flags(m);
                    ff.set(Flags::CONSTRUCTOR);
                    let anns = annotations_of(m, src);
                    if anns.iter().any(|a| a.simple_name() == "Deprecated") {
                        ff.set(Flags::DEPRECATED);
                    }
                    let (params, varargs) = if m.kind() == "compact_constructor_declaration" {
                        // canonical constructor: same params as record components
                        let ps = fields
                            .iter()
                            .filter(|f| f.flags.has(Flags::RECORD_COMPONENT))
                            .map(|f| RawParam { name: f.name.clone(), ty: f.ty.clone(), annotations: Vec::new(), flags: Flags::default() })
                            .collect();
                        (ps, false)
                    } else {
                        formal_params(m.field("parameters"), src)
                    };
                    if varargs {
                        ff.set(Flags::VARARGS);
                    }
                    methods.push(RawMethod {
                        name: Arc::from("<init>"),
                        flags: ff,
                        type_params: type_params(m.field("type_parameters"), src),
                        params,
                        ret: TypeSyntax::Void,
                        throws: throws_of(m, src),
                        annotations: anns,
                        range: range(n),
                        generated_by: None,
                    });
                }
                k if grounds_syntax::is_type_declaration(k) => {
                    if let Some(inner) = self.class(m, pkg, Some((&name, kind)), false) {
                        inners.push(inner);
                    }
                }
                _ => {}
            }
        }

        // implicit members
        let has_ctor = methods.iter().any(|m| &*m.name == "<init>");
        match kind {
            ClassKind::Class if !has_ctor => {
                let mut ff = Flags(Flags::CONSTRUCTOR | Flags::IMPLICIT);
                ff.set(flags.0 & (Flags::PUBLIC | Flags::PROTECTED | Flags::PRIVATE));
                methods.push(implicit_method("<init>", ff, vec![], TypeSyntax::Void));
            }
            ClassKind::Enum => {
                let ety = TypeSyntax::Named(vec![(Arc::from(simple), vec![])]);
                methods.push(implicit_method("values", Flags(Flags::PUBLIC | Flags::STATIC | Flags::IMPLICIT), vec![], TypeSyntax::Array(Box::new(ety.clone()))));
                methods.push(implicit_method(
                    "valueOf",
                    Flags(Flags::PUBLIC | Flags::STATIC | Flags::IMPLICIT),
                    vec![RawParam { name: Arc::from("name"), ty: TypeSyntax::qualified("java.lang.String"), annotations: vec![], flags: Flags::default() }],
                    ety,
                ));
                if !has_ctor {
                    methods.push(implicit_method("<init>", Flags(Flags::PRIVATE | Flags::CONSTRUCTOR | Flags::IMPLICIT), vec![], TypeSyntax::Void));
                }
            }
            ClassKind::Record => {
                let comps: Vec<RawField> = fields.iter().filter(|f| f.flags.has(Flags::RECORD_COMPONENT)).cloned().collect();
                for c in &comps {
                    if !methods.iter().any(|m| m.name == c.name && m.params.is_empty()) {
                        methods.push(implicit_method(&c.name, Flags(Flags::PUBLIC | Flags::IMPLICIT), vec![], c.ty.clone()));
                    }
                }
                let canonical_exists = methods.iter().any(|m| {
                    &*m.name == "<init>" && m.params.len() == comps.len() && m.params.iter().zip(&comps).all(|(p, c)| p.ty == c.ty)
                });
                if !canonical_exists {
                    let ps = comps
                        .iter()
                        .map(|c| RawParam { name: c.name.clone(), ty: c.ty.clone(), annotations: vec![], flags: Flags::default() })
                        .collect();
                    // `record R(String a, Object... rest)`: the canonical constructor is varargs
                    let varargs = node.field("parameters").and_then(|p| p.kids().into_iter().last()).is_some_and(|k| k.kind() == "spread_parameter");
                    let mut flags = Flags(Flags::PUBLIC | Flags::CONSTRUCTOR | Flags::IMPLICIT);
                    if varargs {
                        flags.set(Flags::VARARGS);
                    }
                    methods.push(implicit_method("<init>", flags, ps, TypeSyntax::Void));
                }
            }
            _ => {}
        }

        let c = &mut self.classes[idx];
        c.fields = fields;
        c.methods = methods;
        c.inners = inners;
        Some(name)
    }
}

/// A method body that can only end by throwing: its last statement is a
/// `throw` and nothing returns early (lambdas and nested classes aside).
fn always_throws(body: Node) -> bool {
    if !body.kids().last().is_some_and(|l| l.kind() == "throw_statement") {
        return false;
    }
    fn returns(n: Node) -> bool {
        n.kids().into_iter().any(|k| match k.kind() {
            "return_statement" => true,
            "lambda_expression" | "class_body" => false,
            _ => returns(k),
        })
    }
    !returns(body)
}

pub fn implicit_method(name: &str, flags: Flags, params: Vec<RawParam>, ret: TypeSyntax) -> RawMethod {
    RawMethod {
        name: Arc::from(name),
        flags,
        type_params: Vec::new(),
        params,
        ret,
        throws: Vec::new(),
        annotations: Vec::new(),
        range: (0, 0),
        generated_by: None,
    }
}

/// Extract a single (possibly local or anonymous-body) type declaration node.
/// Used by the semantic layer for local classes.
pub fn extract_class_node(node: Node, src: &str, pkg: &str, outer: Option<&Name>) -> Vec<RawClass> {
    let mut ex = Extractor { src, classes: Vec::new() };
    let o = outer.map(|o| (o, ClassKind::Class));
    ex.class(node, pkg, o, true);
    crate::lombok::apply_all(&mut ex.classes, &[]);
    ex.classes
}

/// Extract an anonymous class body as a class named `name` nested in `outer`.
pub fn extract_anonymous(body: Node, src: &str, pkg: &str, outer: &Name, simple: &str) -> Vec<RawClass> {
    let mut ex = Extractor { src, classes: Vec::new() };
    ex.class_named(body, pkg, Some((outer, ClassKind::Class)), Some(simple));
    ex.classes
}
