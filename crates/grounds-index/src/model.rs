//! Language-level model shared by every layer: types, classes, members and
//! annotations. Both source stubs and class files are lowered into these.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

/// Interned-ish name. Class names are binary names using `.` for packages and
/// `$` for nesting: `java.util.Map$Entry`.
pub type Name = Arc<str>;

pub fn name(s: &str) -> Name {
    Arc::from(s)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Prim {
    Boolean,
    Byte,
    Short,
    Char,
    Int,
    Long,
    Float,
    Double,
}

impl Prim {
    pub fn from_keyword(s: &str) -> Option<Prim> {
        Some(match s {
            "boolean" => Prim::Boolean,
            "byte" => Prim::Byte,
            "short" => Prim::Short,
            "char" => Prim::Char,
            "int" => Prim::Int,
            "long" => Prim::Long,
            "float" => Prim::Float,
            "double" => Prim::Double,
            _ => return None,
        })
    }
    pub fn from_descriptor(c: char) -> Option<Prim> {
        Some(match c {
            'Z' => Prim::Boolean,
            'B' => Prim::Byte,
            'S' => Prim::Short,
            'C' => Prim::Char,
            'I' => Prim::Int,
            'J' => Prim::Long,
            'F' => Prim::Float,
            'D' => Prim::Double,
            _ => return None,
        })
    }
    pub fn keyword(self) -> &'static str {
        match self {
            Prim::Boolean => "boolean",
            Prim::Byte => "byte",
            Prim::Short => "short",
            Prim::Char => "char",
            Prim::Int => "int",
            Prim::Long => "long",
            Prim::Float => "float",
            Prim::Double => "double",
        }
    }
    pub fn box_name(self) -> &'static str {
        match self {
            Prim::Boolean => "java.lang.Boolean",
            Prim::Byte => "java.lang.Byte",
            Prim::Short => "java.lang.Short",
            Prim::Char => "java.lang.Character",
            Prim::Int => "java.lang.Integer",
            Prim::Long => "java.lang.Long",
            Prim::Float => "java.lang.Float",
            Prim::Double => "java.lang.Double",
        }
    }
    pub fn from_box_name(s: &str) -> Option<Prim> {
        Some(match s {
            "java.lang.Boolean" => Prim::Boolean,
            "java.lang.Byte" => Prim::Byte,
            "java.lang.Short" => Prim::Short,
            "java.lang.Character" => Prim::Char,
            "java.lang.Integer" => Prim::Int,
            "java.lang.Long" => Prim::Long,
            "java.lang.Float" => Prim::Float,
            "java.lang.Double" => Prim::Double,
            _ => return None,
        })
    }
    pub fn is_numeric(self) -> bool {
        !matches!(self, Prim::Boolean)
    }
    pub fn is_integral(self) -> bool {
        matches!(self, Prim::Byte | Prim::Short | Prim::Char | Prim::Int | Prim::Long)
    }
    pub fn is_floating(self) -> bool {
        matches!(self, Prim::Float | Prim::Double)
    }
    /// Numeric rank for widening/promotion.
    pub fn rank(self) -> u8 {
        match self {
            Prim::Boolean => 0,
            Prim::Byte => 1,
            Prim::Short => 2,
            Prim::Char => 2,
            Prim::Int => 3,
            Prim::Long => 4,
            Prim::Float => 5,
            Prim::Double => 6,
        }
    }
    /// Can `self` be widened to `to` (JLS 5.1.2), including identity.
    pub fn widens_to(self, to: Prim) -> bool {
        if self == to {
            return true;
        }
        use Prim::*;
        match (self, to) {
            (Byte, Short | Int | Long | Float | Double) => true,
            (Short, Int | Long | Float | Double) => true,
            (Char, Int | Long | Float | Double) => true,
            (Int, Long | Float | Double) => true,
            (Long, Float | Double) => true,
            (Float, Double) => true,
            _ => false,
        }
    }
    pub fn bits(self) -> u32 {
        match self {
            Prim::Boolean => 1,
            Prim::Byte => 8,
            Prim::Short | Prim::Char => 16,
            Prim::Int | Prim::Float => 32,
            Prim::Long | Prim::Double => 64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ty {
    /// Could not be determined. Rules must stay silent on unknown types.
    Unknown,
    Void,
    Null,
    Prim(Prim),
    Class { name: Name, args: Vec<Ty> },
    Array(Box<Ty>),
    TypeVar(Name),
    /// Only valid as a type argument.
    Wildcard(Option<Box<WildcardBound>>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WildcardBound {
    Extends(Ty),
    Super(Ty),
}

impl Ty {
    pub fn class(name: &str) -> Ty {
        Ty::Class { name: Arc::from(name), args: Vec::new() }
    }
    pub fn class_with(name: &str, args: Vec<Ty>) -> Ty {
        Ty::Class { name: Arc::from(name), args }
    }
    pub fn object() -> Ty {
        Ty::class("java.lang.Object")
    }
    pub fn string() -> Ty {
        Ty::class("java.lang.String")
    }
    pub fn int() -> Ty {
        Ty::Prim(Prim::Int)
    }
    pub fn boolean() -> Ty {
        Ty::Prim(Prim::Boolean)
    }
    pub fn is_unknown(&self) -> bool {
        matches!(self, Ty::Unknown)
    }
    pub fn is_known(&self) -> bool {
        !self.is_unknown()
    }
    /// Whether any component of this type is unknown.
    pub fn has_unknown(&self) -> bool {
        match self {
            Ty::Unknown => true,
            Ty::Class { args, .. } => args.iter().any(|a| a.has_unknown()),
            Ty::Array(e) => e.has_unknown(),
            Ty::Wildcard(Some(b)) => match &**b {
                WildcardBound::Extends(t) | WildcardBound::Super(t) => t.has_unknown(),
            },
            _ => false,
        }
    }
    pub fn is_prim(&self) -> bool {
        matches!(self, Ty::Prim(_))
    }
    pub fn prim(&self) -> Option<Prim> {
        match self {
            Ty::Prim(p) => Some(*p),
            _ => None,
        }
    }
    pub fn is_reference(&self) -> bool {
        matches!(self, Ty::Class { .. } | Ty::Array(_) | Ty::TypeVar(_) | Ty::Null)
    }
    pub fn class_name(&self) -> Option<&str> {
        match self {
            Ty::Class { name, .. } => Some(name),
            _ => None,
        }
    }
    pub fn is_class(&self, n: &str) -> bool {
        self.class_name() == Some(n)
    }
    pub fn is_string(&self) -> bool {
        self.is_class("java.lang.String")
    }
    pub fn is_boolean(&self) -> bool {
        matches!(self, Ty::Prim(Prim::Boolean)) || self.is_class("java.lang.Boolean")
    }
    pub fn args(&self) -> &[Ty] {
        match self {
            Ty::Class { args, .. } => args,
            _ => &[],
        }
    }
    pub fn arg(&self, i: usize) -> Option<&Ty> {
        self.args().get(i)
    }
    pub fn element(&self) -> Option<&Ty> {
        match self {
            Ty::Array(e) => Some(e),
            _ => None,
        }
    }
    pub fn is_array(&self) -> bool {
        matches!(self, Ty::Array(_))
    }
    /// The boxed primitive for a boxed type name, if any.
    pub fn unboxed(&self) -> Option<Prim> {
        match self {
            Ty::Prim(p) => Some(*p),
            Ty::Class { name, .. } => Prim::from_box_name(name),
            _ => None,
        }
    }
    pub fn is_boxed(&self) -> bool {
        matches!(self, Ty::Class { name, .. } if Prim::from_box_name(name).is_some())
    }
    pub fn boxed(&self) -> Ty {
        match self {
            Ty::Prim(p) => Ty::class(p.box_name()),
            t => t.clone(),
        }
    }
    pub fn erasure(&self) -> Ty {
        match self {
            Ty::Class { name, .. } => Ty::Class { name: name.clone(), args: Vec::new() },
            Ty::Array(e) => Ty::Array(Box::new(e.erasure())),
            Ty::TypeVar(_) => Ty::object(),
            Ty::Wildcard(Some(b)) => match &**b {
                WildcardBound::Extends(t) => t.erasure(),
                WildcardBound::Super(_) => Ty::object(),
            },
            Ty::Wildcard(None) => Ty::object(),
            t => t.clone(),
        }
    }
    /// For a wildcard, its upper bound; other types unchanged.
    pub fn upper_bound(&self) -> Ty {
        match self {
            Ty::Wildcard(Some(b)) => match &**b {
                WildcardBound::Extends(t) => t.clone(),
                WildcardBound::Super(_) => Ty::object(),
            },
            Ty::Wildcard(None) => Ty::object(),
            t => t.clone(),
        }
    }
    pub fn is_numeric(&self) -> bool {
        self.unboxed().is_some_and(|p| p.is_numeric())
    }
    /// Substitute type variables using `f`.
    pub fn subst(&self, f: &dyn Fn(&str) -> Option<Ty>) -> Ty {
        match self {
            Ty::TypeVar(n) => f(n).unwrap_or_else(|| self.clone()),
            Ty::Class { name, args } if !args.is_empty() => {
                Ty::Class { name: name.clone(), args: args.iter().map(|a| a.subst(f)).collect() }
            }
            Ty::Array(e) => Ty::Array(Box::new(e.subst(f))),
            Ty::Wildcard(Some(b)) => Ty::Wildcard(Some(Box::new(match &**b {
                WildcardBound::Extends(t) => WildcardBound::Extends(t.subst(f)),
                WildcardBound::Super(t) => WildcardBound::Super(t.subst(f)),
            }))),
            t => t.clone(),
        }
    }
    pub fn mentions_type_var(&self) -> bool {
        match self {
            Ty::TypeVar(_) => true,
            Ty::Class { args, .. } => args.iter().any(|a| a.mentions_type_var()),
            Ty::Array(e) => e.mentions_type_var(),
            Ty::Wildcard(Some(b)) => match &**b {
                WildcardBound::Extends(t) | WildcardBound::Super(t) => t.mentions_type_var(),
            },
            _ => false,
        }
    }
    /// Short human readable form (simple names).
    pub fn display_simple(&self) -> String {
        match self {
            Ty::Class { name, args } => {
                let simple = simple_name_of(name).replace('$', ".");
                if args.is_empty() {
                    simple
                } else {
                    format!("{}<{}>", simple, args.iter().map(|a| a.display_simple()).collect::<Vec<_>>().join(", "))
                }
            }
            Ty::Array(e) => format!("{}[]", e.display_simple()),
            t => t.to_string(),
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Unknown => write!(f, "?unknown"),
            Ty::Void => write!(f, "void"),
            Ty::Null => write!(f, "null"),
            Ty::Prim(p) => write!(f, "{}", p.keyword()),
            Ty::Class { name, args } => {
                write!(f, "{}", name.replace('$', "."))?;
                if !args.is_empty() {
                    write!(f, "<")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{a}")?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }
            Ty::Array(e) => write!(f, "{e}[]"),
            Ty::TypeVar(n) => write!(f, "{n}"),
            Ty::Wildcard(None) => write!(f, "?"),
            Ty::Wildcard(Some(b)) => match &**b {
                WildcardBound::Extends(t) => write!(f, "? extends {t}"),
                WildcardBound::Super(t) => write!(f, "? super {t}"),
            },
        }
    }
}

/// Last segment of a binary class name, with `$` nesting stripped:
/// `java.util.Map$Entry` -> `Entry`. An anonymous class (`C$1`), which no
/// source can name, is `anonymous class`.
pub fn simple_name_of(binary: &str) -> &str {
    let after_dot = binary.rsplit('.').next().unwrap_or(binary);
    let last = after_dot.rsplit('$').next().unwrap_or(after_dot);
    if !last.is_empty() && last.len() < after_dot.len() && last.bytes().all(|b| b.is_ascii_digit()) {
        return "anonymous class";
    }
    last
}

pub fn package_of(binary: &str) -> &str {
    match binary.rfind('.') {
        Some(i) => &binary[..i],
        None => "",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClassKind {
    Class,
    Interface,
    Enum,
    Record,
    Annotation,
}

/// Modifier and property bit flags.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Flags(pub u32);

impl Flags {
    pub const PUBLIC: u32 = 1 << 0;
    pub const PRIVATE: u32 = 1 << 1;
    pub const PROTECTED: u32 = 1 << 2;
    pub const STATIC: u32 = 1 << 3;
    pub const FINAL: u32 = 1 << 4;
    pub const ABSTRACT: u32 = 1 << 5;
    pub const SYNCHRONIZED: u32 = 1 << 6;
    pub const VOLATILE: u32 = 1 << 7;
    pub const TRANSIENT: u32 = 1 << 8;
    pub const NATIVE: u32 = 1 << 9;
    pub const DEFAULT: u32 = 1 << 10;
    pub const VARARGS: u32 = 1 << 11;
    pub const SYNTHETIC: u32 = 1 << 12;
    pub const DEPRECATED: u32 = 1 << 13;
    pub const SEALED: u32 = 1 << 14;
    pub const NON_SEALED: u32 = 1 << 15;
    pub const STRICTFP: u32 = 1 << 16;
    /// Member generated by an annotation processor model (Lombok etc).
    pub const GENERATED: u32 = 1 << 17;
    /// Implicitly declared (default constructor, enum values(), record accessors).
    pub const IMPLICIT: u32 = 1 << 18;
    pub const CONSTRUCTOR: u32 = 1 << 19;
    pub const ENUM_CONSTANT: u32 = 1 << 20;
    pub const RECORD_COMPONENT: u32 = 1 << 21;
    pub const ANON: u32 = 1 << 22;
    pub const LOCAL: u32 = 1 << 23;
    /// A source method whose body always ends by throwing (`void fail(String m) { throw ...; }`).
    pub const ALWAYS_THROWS: u32 = 1 << 24;
    /// A source method that starts with `if (!flag) throw ...;` on its first
    /// (boolean) parameter: a check that returns only when the flag is true.
    pub const THROWS_ON_FALSE: u32 = 1 << 25;
    /// A source method whose body asserts (an assertion-style call, `assert`,
    /// a thrown `AssertionError`), directly or through a helper of its class.
    /// Only meaningful in test code: see [`crate::stubs::is_assertion_name`].
    pub const ASSERTS: u32 = 1 << 26;
    /// A source method not known to assert that calls a method its class does
    /// not declare (inherited from another file), directly or through a helper
    /// of its class: it may assert there.
    pub const DELEGATES: u32 = 1 << 27;
    /// A source `close()` that only sets its object's own fields to
    /// constants or calls `super.close()` (`closed = true;`): it releases
    /// nothing itself.
    pub const RELEASES_NOTHING: u32 = 1 << 28;
    /// A source class whose constructors or field initializers open a file,
    /// socket or JDBC connection (`new FileOutputStream(...)`,
    /// `Files.newBufferedWriter(...)`): it owns a resource of its own.
    pub const OPENS_RESOURCE: u32 = 1 << 29;

    pub fn has(self, f: u32) -> bool {
        self.0 & f != 0
    }
    pub fn set(&mut self, f: u32) {
        self.0 |= f;
    }
    pub fn clear(&mut self, f: u32) {
        self.0 &= !f;
    }
    pub fn is_static(self) -> bool {
        self.has(Self::STATIC)
    }
    pub fn is_private(self) -> bool {
        self.has(Self::PRIVATE)
    }
    pub fn is_public(self) -> bool {
        self.has(Self::PUBLIC)
    }
    pub fn is_final(self) -> bool {
        self.has(Self::FINAL)
    }
    pub fn is_abstract(self) -> bool {
        self.has(Self::ABSTRACT)
    }
}

impl fmt::Debug for Flags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = [
            "public", "private", "protected", "static", "final", "abstract", "synchronized", "volatile", "transient", "native",
            "default", "varargs", "synthetic", "deprecated", "sealed", "non-sealed", "strictfp", "generated", "implicit", "ctor",
            "enum-const", "record-comp", "anon", "local",
        ];
        let v: Vec<&str> = names.iter().enumerate().filter(|(i, _)| self.0 & (1 << i) != 0).map(|(_, n)| *n).collect();
        write!(f, "Flags({})", v.join(" "))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Constant {
    /// `int` (and byte/short) values, always within i32 range
    Int(i64),
    /// `long` values
    Long(i64),
    Float(f64),
    Bool(bool),
    Char(char),
    Str(Arc<str>),
}

impl Constant {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Constant::Int(v) | Constant::Long(v) => Some(*v),
            Constant::Char(c) => Some(*c as i64),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Constant::Int(v) | Constant::Long(v) => Some(*v as f64),
            Constant::Float(v) => Some(*v),
            Constant::Char(c) => Some(*c as u32 as f64),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Constant::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Constant::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AnnotationValue {
    Const(Constant),
    Enum { ty: Name, constant: Name },
    Class(Ty),
    Annotation(Box<AnnotationInfo>),
    Array(Vec<AnnotationValue>),
    /// Something we did not evaluate; holds source text.
    Expr(Arc<str>),
}

impl AnnotationValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            AnnotationValue::Const(Constant::Str(s)) => Some(s),
            _ => None,
        }
    }
    /// Flatten single values and arrays into a list of strings.
    pub fn strings(&self) -> Vec<String> {
        match self {
            AnnotationValue::Const(Constant::Str(s)) => vec![s.to_string()],
            AnnotationValue::Array(v) => v.iter().flat_map(|x| x.strings()).collect(),
            _ => Vec::new(),
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            AnnotationValue::Const(Constant::Bool(b)) => Some(*b),
            _ => None,
        }
    }
    pub fn values(&self) -> Vec<&AnnotationValue> {
        match self {
            AnnotationValue::Array(v) => v.iter().collect(),
            v => vec![v],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnotationInfo {
    /// Resolved binary name when known; otherwise the name as written.
    pub name: Name,
    pub resolved: bool,
    pub args: Vec<(Name, AnnotationValue)>,
}

impl AnnotationInfo {
    pub fn simple_name(&self) -> &str {
        simple_name_of(&self.name)
    }
    pub fn arg(&self, key: &str) -> Option<&AnnotationValue> {
        self.args.iter().find(|(k, _)| &**k == key).map(|(_, v)| v)
    }
    pub fn value(&self) -> Option<&AnnotationValue> {
        self.arg("value")
    }
    /// Match against a fully qualified name; unresolved annotations match by
    /// simple name.
    pub fn is(&self, fqn: &str) -> bool {
        if self.resolved {
            &*self.name == fqn
        } else {
            &*self.name == fqn || simple_name_of(&self.name) == simple_name_of(fqn)
        }
    }
}

pub fn find_annotation<'a>(anns: &'a [AnnotationInfo], fqn: &str) -> Option<&'a AnnotationInfo> {
    anns.iter().find(|a| a.is(fqn))
}

pub fn has_annotation(anns: &[AnnotationInfo], fqn: &str) -> bool {
    find_annotation(anns, fqn).is_some()
}

pub fn has_annotation_simple(anns: &[AnnotationInfo], simple: &str) -> bool {
    anns.iter().any(|a| a.simple_name() == simple)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypeParam {
    pub name: Name,
    pub bounds: Vec<Ty>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamInfo {
    pub name: Name,
    pub ty: Ty,
    pub annotations: Vec<AnnotationInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldInfo {
    pub name: Name,
    pub ty: Ty,
    pub flags: Flags,
    pub annotations: Vec<AnnotationInfo>,
    pub constant: Option<Constant>,
    /// Range of the declarator name in source.
    pub range: Option<(u32, u32)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MethodInfo {
    /// `<init>` for constructors.
    pub name: Name,
    pub flags: Flags,
    pub type_params: Vec<TypeParam>,
    pub params: Vec<ParamInfo>,
    pub ret: Ty,
    pub throws: Vec<Ty>,
    pub annotations: Vec<AnnotationInfo>,
    pub range: Option<(u32, u32)>,
    /// Which processor model synthesised this member, if any (e.g. "lombok").
    pub generated_by: Option<Name>,
}

impl MethodInfo {
    pub fn is_constructor(&self) -> bool {
        &*self.name == "<init>"
    }
    pub fn is_varargs(&self) -> bool {
        self.flags.has(Flags::VARARGS)
    }
    pub fn arity(&self) -> usize {
        self.params.len()
    }
    pub fn accepts_arity(&self, n: usize) -> bool {
        if self.is_varargs() {
            n + 1 >= self.params.len()
        } else {
            n == self.params.len()
        }
    }
    /// `name(T1,T2)` with erased simple types, for messages and fingerprints.
    pub fn signature_string(&self) -> String {
        let ps: Vec<String> = self.params.iter().map(|p| p.ty.erasure().display_simple()).collect();
        format!("{}({})", self.name, ps.join(","))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Origin {
    Source { file: Arc<str> },
    Binary { container: u32 },
    Synthetic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClassInfo {
    pub name: Name,
    pub kind: ClassKind,
    pub flags: Flags,
    pub type_params: Vec<TypeParam>,
    pub superclass: Option<Ty>,
    pub interfaces: Vec<Ty>,
    pub fields: Vec<FieldInfo>,
    pub methods: Vec<MethodInfo>,
    pub annotations: Vec<AnnotationInfo>,
    pub outer: Option<Name>,
    pub inners: Vec<Name>,
    pub permits: Vec<Ty>,
    pub origin: Origin,
    /// Name range in source.
    pub range: Option<(u32, u32)>,
}

impl ClassInfo {
    pub fn simple_name(&self) -> &str {
        simple_name_of(&self.name)
    }
    pub fn package(&self) -> &str {
        package_of(&self.name)
    }
    pub fn is_interface(&self) -> bool {
        matches!(self.kind, ClassKind::Interface | ClassKind::Annotation)
    }
    pub fn is_enum(&self) -> bool {
        self.kind == ClassKind::Enum
    }
    pub fn is_record(&self) -> bool {
        self.kind == ClassKind::Record
    }
    pub fn is_abstract(&self) -> bool {
        self.flags.is_abstract() || self.is_interface()
    }
    pub fn is_source(&self) -> bool {
        matches!(self.origin, Origin::Source { .. })
    }
    pub fn source_file(&self) -> Option<&str> {
        match &self.origin {
            Origin::Source { file } => Some(file),
            _ => None,
        }
    }
    /// Is this an inner (non-static nested) class?
    pub fn is_inner(&self) -> bool {
        self.outer.is_some() && !self.flags.is_static() && self.kind == ClassKind::Class
    }
    pub fn methods_named<'a>(&'a self, n: &'a str) -> impl Iterator<Item = &'a MethodInfo> + 'a {
        self.methods.iter().filter(move |m| &*m.name == n)
    }
    pub fn constructors(&self) -> impl Iterator<Item = &MethodInfo> {
        self.methods.iter().filter(|m| m.is_constructor())
    }
    pub fn field(&self, n: &str) -> Option<&FieldInfo> {
        self.fields.iter().find(|f| &*f.name == n)
    }
    pub fn has_annotation(&self, fqn: &str) -> bool {
        has_annotation(&self.annotations, fqn)
    }
    /// The declared type with its own type parameters as arguments.
    pub fn this_type(&self) -> Ty {
        Ty::Class { name: self.name.clone(), args: self.type_params.iter().map(|p| Ty::TypeVar(p.name.clone())).collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::simple_name_of;

    #[test]
    fn simple_names() {
        assert_eq!(simple_name_of("java.util.Map$Entry"), "Entry");
        assert_eq!(simple_name_of("p.C$1"), "anonymous class");
        assert_eq!(simple_name_of("C"), "C");
    }
}
