//! Model of Lombok's generated members. We do not run Lombok; we synthesize
//! the declarations it would produce so resolution and framework models see
//! the same API the compiler sees.

use crate::model::*;
use crate::stubs::*;
use std::sync::Arc;

const LOMBOK: &str = "lombok";

/// Settings from `lombok.config` files that change generated signatures.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LombokConfig {
    /// `lombok.accessors.fluent`: getters/setters named like the field
    pub fluent: bool,
    /// `lombok.accessors.chain`: setters return `this`
    pub chain: bool,
    /// `lombok.accessors.prefix`: field prefixes stripped for accessor names
    pub prefixes: Vec<String>,
    /// `lombok.copyableAnnotations`: field annotations copied onto generated
    /// constructor parameters (and setters)
    pub copyable: Vec<String>,
    /// `lombok.getter.noIsPrefix`: getters of `boolean` fields are named
    /// `getX`, not `isX`
    pub no_is_prefix: bool,
    /// `lombok.builder.className`: the builder class name, `*` standing for
    /// the built type's name (default `*Builder`)
    pub builder_class_name: Option<String>,
    /// `lombok.log.fieldName`: the logger field's name (default `log`)
    pub log_field_name: Option<String>,
    /// `lombok.fieldNameConstants.innerTypeName` (default `Fields`)
    pub field_name_constants_type: Option<String>,
    /// `lombok.fieldNameConstants.uppercase`: constants named `CREATE_TIME`
    pub field_name_constants_uppercase: bool,
    /// A key is set that changes the members Lombok generates in a way this
    /// model does not reproduce (see [`UNMODELLED_KEYS`]): the members of
    /// every class Lombok processes under it are unknown.
    pub unmodelled: bool,
}

/// `lombok.config` keys whose non-default values change the names, types or
/// number of generated members in ways the model does not reproduce.
/// (Keys that only change annotations, null checks, `toString` output or
/// warnings are not listed: they leave the members as modelled.)
pub const UNMODELLED_KEYS: &[&str] = &[
    // getter/setter names under the JavaBeans capitalization (`getuName`)
    "lombok.accessors.capitalization",
    "lombok.accessors.javabeansspeccapitalization",
    // Guava builders instead of ArrayList fields and other method shapes
    "lombok.singular.useguava",
    // a private no-args constructor next to @Data/@Value's
    "lombok.noargsconstructor.extraprivate",
    // a logger field that is not static
    "lombok.log.fieldisstatic",
    // a custom logger's type and factory
    "lombok.log.custom.declaration",
];

/// The annotations of Lombok's packages (1.18.36), by package. A simple name
/// is taken for Lombok's through an on-demand import only when listed here;
/// a single-type import or a qualified name is recognised whatever it names.
const LOMBOK_NAMES: &[(&str, &[&str])] = &[
    ("lombok", &["AllArgsConstructor", "Builder", "Cleanup", "CustomLog", "Data", "EqualsAndHashCode", "Generated", "Getter", "Locked", "NoArgsConstructor", "NonNull", "RequiredArgsConstructor", "Setter", "Singular", "SneakyThrows", "Synchronized", "ToString", "Value", "With"]),
    ("lombok.experimental", &["Accessors", "Delegate", "ExtensionMethod", "FieldDefaults", "FieldNameConstants", "Helper", "Jacksonized", "NonFinal", "PackagePrivate", "StandardException", "SuperBuilder", "Tolerate", "UtilityClass", "WithBy", "Wither"]),
    ("lombok.extern.apachecommons", &["CommonsLog"]),
    ("lombok.extern.flogger", &["Flogger"]),
    ("lombok.extern.jackson", &["Jacksonized"]),
    ("lombok.extern.java", &["Log"]),
    ("lombok.extern.jbosslog", &["JBossLog"]),
    ("lombok.extern.log4j", &["Log4j", "Log4j2"]),
    ("lombok.extern.slf4j", &["Slf4j", "XSlf4j"]),
];

/// Packages of annotation processors that add members to the class they
/// annotate (rather than generating new classes next to it). Only Lombok's
/// effects are modelled, and only those of [`IMPLEMENTED`].
const MEMBER_PROCESSORS: &[&str] = &["lombok"];

/// The Lombok annotations whose effect on members this model reproduces
/// fully: every member they add (fields, methods, constructors, nested
/// classes) is in the stub, with the options that change names or
/// signatures. Exceptions inside them make the class open too (see
/// [`unmodelled_use`]). Any other annotation of a [`MEMBER_PROCESSORS`]
/// package (`@Delegate`, `@ExtensionMethod`, `@Accessors`, `@Helper`,
/// `@WithBy`, `@CustomLog`, ones added by later versions...) leaves the
/// members of its class unknown.
pub const IMPLEMENTED: &[&str] = &[
    "lombok.AllArgsConstructor",
    "lombok.Builder",
    "lombok.Builder.Default",
    "lombok.Builder.ObtainVia",
    "lombok.Cleanup",
    "lombok.Data",
    "lombok.EqualsAndHashCode",
    "lombok.EqualsAndHashCode.Exclude",
    "lombok.EqualsAndHashCode.Include",
    "lombok.Generated",
    "lombok.Getter",
    "lombok.Locked",
    "lombok.Locked.Read",
    "lombok.Locked.Write",
    "lombok.NoArgsConstructor",
    "lombok.NonNull",
    "lombok.RequiredArgsConstructor",
    "lombok.Setter",
    "lombok.Singular",
    "lombok.SneakyThrows",
    "lombok.Synchronized",
    "lombok.ToString",
    "lombok.ToString.Exclude",
    "lombok.ToString.Include",
    "lombok.Value",
    "lombok.With",
    "lombok.experimental.FieldDefaults",
    "lombok.experimental.FieldNameConstants",
    "lombok.experimental.FieldNameConstants.Exclude",
    "lombok.experimental.FieldNameConstants.Include",
    "lombok.experimental.Jacksonized",
    "lombok.experimental.NonFinal",
    "lombok.experimental.PackagePrivate",
    "lombok.experimental.StandardException",
    "lombok.experimental.SuperBuilder",
    "lombok.experimental.Tolerate",
    "lombok.experimental.UtilityClass",
    "lombok.experimental.Wither",
    "lombok.extern.apachecommons.CommonsLog",
    "lombok.extern.flogger.Flogger",
    "lombok.extern.jackson.Jacksonized",
    "lombok.extern.java.Log",
    "lombok.extern.jbosslog.JBossLog",
    "lombok.extern.log4j.Log4j",
    "lombok.extern.log4j.Log4j2",
    "lombok.extern.slf4j.Slf4j",
    "lombok.extern.slf4j.XSlf4j",
];

/// The qualified name of a processor annotation (see [`MEMBER_PROCESSORS`])
/// as written and imported: `Builder.Default` under `import lombok.Builder;`
/// is `lombok.Builder.Default`. None for any other annotation.
pub fn processor_annotation(a: &RawAnnotation, imports: &[Import]) -> Option<String> {
    let in_processor = |q: &str| MEMBER_PROCESSORS.iter().any(|p| q.starts_with(p) && q[p.len()..].starts_with('.'));
    if in_processor(&a.name) {
        return Some(a.name.to_string());
    }
    let (first, rest) = match a.name.split_once('.') {
        Some((f, r)) => (f, Some(r)),
        None => (&*a.name, None),
    };
    let join = |q: &str| match rest {
        Some(r) => format!("{q}.{r}"),
        None => q.to_string(),
    };
    // a single-type import decides the name
    if let Some(i) = imports.iter().find(|i| !i.is_static && !i.on_demand && i.path.rsplit('.').next() == Some(first)) {
        return in_processor(&i.path).then(|| join(&i.path));
    }
    for i in imports.iter().filter(|i| !i.is_static && i.on_demand) {
        if let Some((pkg, names)) = LOMBOK_NAMES.iter().find(|(p, _)| *p == &*i.path) {
            if names.contains(&first) {
                return Some(join(&format!("{pkg}.{first}")));
            }
        }
    }
    None
}

/// Whether Lombok adds members to `c` that the model does not reproduce: an
/// annotation of a member-adding processor that is not in [`IMPLEMENTED`]
/// on the class, a field or a method, or an implemented one used in a way
/// the model does not cover.
fn members_unknown(c: &RawClass, imports: &[Import], cfg: &LombokConfig) -> bool {
    let anns = c.annotations.iter().chain(c.fields.iter().flat_map(|f| f.annotations.iter())).chain(c.methods.iter().flat_map(|m| m.annotations.iter().chain(m.params.iter().flat_map(|p| p.annotations.iter()))));
    let mut any = false;
    for a in anns {
        let Some(q) = processor_annotation(a, imports) else { continue };
        any = true;
        if !IMPLEMENTED.contains(&q.as_str()) || unmodelled_use(&q, a) {
            return true;
        }
    }
    // a lombok.config key the model does not reproduce
    if any && cfg.unmodelled {
        return true;
    }
    // @Singular on a type whose builder methods are not modelled (Guava's,
    // or a `? super` element), or without a name Lombok can derive (an error)
    let unmodelled_singular = |anns: &[RawAnnotation], ty: &TypeSyntax, name: &str| match singular_of(anns) {
        Some(explicit) => singular_kind(ty).is_none() || (explicit.is_none() && auto_singularize(name).is_none()),
        None => false,
    };
    c.fields.iter().any(|f| unmodelled_singular(&f.annotations, &f.ty, &strip_prefix_name(&f.name, &cfg.prefixes)))
        || c.methods.iter().flat_map(|m| m.params.iter()).any(|p| unmodelled_singular(&p.annotations, &p.ty, &p.name))
}

/// The annotations Lombok takes for "non-null" (`NONNULL_ANNOTATIONS` of
/// Lombok 1.18.36, read from its `HandlerUtil`): a field carrying one is required by
/// `@RequiredArgsConstructor`.
const NON_NULL: &[&str] = &[
    "android.annotation.NonNull",
    "android.support.annotation.NonNull",
    "android.support.annotation.RecentlyNonNull",
    "androidx.annotation.NonNull",
    "androidx.annotation.RecentlyNonNull",
    "com.android.annotations.NonNull",
    "com.google.firebase.database.annotations.NotNull",
    "com.mongodb.lang.NonNull",
    "com.sun.istack.NotNull",
    "com.unboundid.util.NotNull",
    "edu.umd.cs.findbugs.annotations.NonNull",
    "io.micrometer.core.lang.NonNull",
    "io.reactivex.annotations.NonNull",
    "io.reactivex.rxjava3.annotations.NonNull",
    "jakarta.annotation.Nonnull",
    "javax.annotation.Nonnull",
    "libcore.util.NonNull",
    "lombok.NonNull",
    "org.checkerframework.checker.nullness.compatqual.NonNullDecl",
    "org.checkerframework.checker.nullness.compatqual.NonNullType",
    "org.checkerframework.checker.nullness.qual.NonNull",
    "org.codehaus.commons.nullanalysis.NotNull",
    "org.eclipse.jdt.annotation.NonNull",
    "org.jetbrains.annotations.NotNull",
    "org.jmlspecs.annotation.NonNull",
    "org.jspecify.annotations.NonNull",
    "org.netbeans.api.annotations.common.NonNull",
    "org.springframework.lang.NonNull",
    "reactor.util.annotation.NonNull",
];

/// Whether `a` is one of [`NON_NULL`], by its qualified name or through the
/// file's imports. A bare `@NonNull` stays Lombok's when nothing else is
/// imported under that name, as before.
fn non_null(a: &RawAnnotation, imports: &[Import]) -> bool {
    let simple = a.simple_name();
    if !matches!(simple, "NonNull" | "NotNull" | "Nonnull" | "RecentlyNonNull" | "NonNullDecl" | "NonNullType") {
        return false;
    }
    if a.name.contains('.') {
        return NON_NULL.contains(&&*a.name);
    }
    let named: Vec<&Import> = imports.iter().filter(|i| !i.is_static && !i.on_demand && i.path.rsplit('.').next() == Some(simple)).collect();
    if let Some(i) = named.first() {
        return NON_NULL.contains(&&*i.path);
    }
    simple == "NonNull" || imports.iter().any(|i| i.on_demand && !i.is_static && NON_NULL.contains(&format!("{}.{simple}", i.path).as_str()))
}

/// An implemented annotation used with an option the model does not
/// reproduce.
fn unmodelled_use(q: &str, a: &RawAnnotation) -> bool {
    match q {
        // the field becomes an AtomicReference behind the getter
        "lombok.Getter" => a.bool_arg("lazy") == Some(true),
        // the pre-1.18.4 form (on a field, with prefix/suffix) is an error now
        "lombok.experimental.FieldNameConstants" => a.arg("prefix").is_some() || a.arg("suffix").is_some(),
        _ => false,
    }
}

/// Field name with a configured prefix removed (`mName` -> `name` with
/// prefix `m`); Lombok only strips a prefix followed by a non-lowercase char.
fn strip_prefix_name(field: &str, prefixes: &[String]) -> String {
    for p in prefixes {
        if let Some(rest) = field.strip_prefix(p.as_str()) {
            if rest.chars().next().is_some_and(|c| !c.is_lowercase()) || (p.ends_with('_') && !rest.is_empty()) {
                let mut cs = rest.chars();
                return match cs.next() {
                    Some(f) => f.to_lowercase().collect::<String>() + cs.as_str(),
                    None => rest.to_string(),
                };
            }
        }
    }
    field.to_string()
}

fn is_lombok(a: &RawAnnotation, simple: &str, lombok_imported: bool) -> bool {
    if &*a.name == simple {
        return lombok_imported;
    }
    a.name.starts_with("lombok.") && a.simple_name() == simple
}

fn access_none(a: &RawAnnotation) -> bool {
    match a.arg("value") {
        Some(RawAnnValue::Name(n)) => n.ends_with("NONE"),
        _ => false,
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn getter_name(field: &str, ty: &TypeSyntax) -> String {
    if ty.is_boolean_prim() {
        if let Some(rest) = field.strip_prefix("is") {
            if rest.chars().next().is_some_and(|c| c.is_uppercase()) {
                return field.to_string();
            }
        }
        return format!("is{}", cap(field));
    }
    format!("get{}", cap(field))
}

pub fn setter_name(field: &str, ty: &TypeSyntax) -> String {
    if ty.is_boolean_prim() {
        if let Some(rest) = field.strip_prefix("is") {
            if rest.chars().next().is_some_and(|c| c.is_uppercase()) {
                return format!("set{rest}");
            }
        }
    }
    format!("set{}", cap(field))
}

fn gen_method(name: &str, flags: u32, params: Vec<RawParam>, ret: TypeSyntax, range: (u32, u32)) -> RawMethod {
    RawMethod {
        name: Arc::from(name),
        flags: Flags(flags | Flags::GENERATED),
        type_params: Vec::new(),
        params,
        ret,
        throws: Vec::new(),
        annotations: Vec::new(),
        range,
        generated_by: Some(Arc::from(LOMBOK)),
    }
}

fn param(name: &str, ty: TypeSyntax) -> RawParam {
    RawParam { name: Arc::from(name), ty, annotations: Vec::new(), flags: Flags::default() }
}

/// Whether `c` declares `name` with `arity` parameters, so that Lombok
/// does not generate it. A method marked `@Tolerate` does not count: Lombok
/// generates its own next to it.
fn has_method(c: &RawClass, name: &str, arity: usize) -> bool {
    c.methods.iter().any(|m| &*m.name == name && m.params.len() == arity && !m.annotations.iter().any(|a| a.simple_name() == "Tolerate"))
}

// a parameter type as far as signatures clash: its name as written, without
// type arguments, and array dimensions. Two spellings of one class (`Widget`,
// `a.Widget`) count as different: the extra overload this models can only
// make a call ambiguous, never unresolved.
fn erased(t: &TypeSyntax) -> (Option<String>, usize) {
    match t {
        TypeSyntax::Array(e) => {
            let (n, d) = erased(e);
            (n, d + 1)
        }
        TypeSyntax::Prim(p) => (Some(p.keyword().to_string()), 0),
        TypeSyntax::Named(segs) => (Some(segs.iter().map(|s| &*s.0).collect::<Vec<_>>().join(".")), 0),
        _ => (None, 0),
    }
}

fn self_type(c: &RawClass) -> TypeSyntax {
    let args = c.type_params.iter().map(|t| TypeSyntax::simple(&t.name)).collect();
    TypeSyntax::Named(vec![(Arc::from(c.simple_name()), args)])
}

/// Apply Lombok models to every class of a file; may append builder classes.
pub fn apply_all(classes: &mut Vec<RawClass>, imports: &[Import]) {
    apply_all_with(classes, imports, &LombokConfig::default())
}

pub fn apply_all_with(classes: &mut Vec<RawClass>, imports: &[Import], cfg: &LombokConfig) {
    let imported = imports.iter().any(|i| i.path.starts_with("lombok"));
    let mut extra = Vec::new();
    for c in classes.iter_mut() {
        extra.extend(apply_one(c, imported, imports, cfg));
        if members_unknown(c, imports, cfg) {
            c.flags.set(Flags::MEMBERS_UNKNOWN);
        }
    }
    // @UtilityClass makes nested classes static too
    let utility: Vec<Name> = classes.iter().filter(|c| c.annotations.iter().any(|a| is_lombok(a, "UtilityClass", imported))).map(|c| c.name.clone()).collect();
    for c in classes.iter_mut() {
        if c.outer.as_ref().is_some_and(|o| utility.contains(o)) {
            c.flags.set(Flags::STATIC);
        }
    }
    for b in extra {
        // Lombok completes a hand-written builder (or `Fields`) class: it adds
        // the fields and methods not declared there by name (and arity)
        if let Some(existing) = classes.iter_mut().find(|c| c.name == b.name) {
            for m in b.methods {
                // the hand-written class keeps its own constructors
                if m.flags.has(Flags::CONSTRUCTOR) && existing.methods.iter().any(|e| e.flags.has(Flags::CONSTRUCTOR)) {
                    continue;
                }
                if !has_method(existing, &m.name, m.params.len()) {
                    existing.methods.push(m);
                }
            }
            for f in b.fields {
                if !existing.fields.iter().any(|e| e.name == f.name) {
                    existing.fields.push(f);
                }
            }
        } else {
            if let Some(outer) = b.outer.clone() {
                if let Some(o) = classes.iter_mut().find(|c| c.name == outer) {
                    o.inners.push(b.name.clone());
                }
            }
            classes.push(b);
        }
    }
}

pub fn apply(c: &mut RawClass) {
    let _ = apply_one(c, true, &[], &LombokConfig::default());
}

fn apply_one(c: &mut RawClass, imported: bool, imports: &[Import], cfg: &LombokConfig) -> Vec<RawClass> {
    let anns = c.annotations.clone();
    let has = |s: &str| anns.iter().find(|a| is_lombok(a, s, imported));
    // Lombok annotations may sit on the class, a field or only a
    // constructor/method (`@Builder private Foo(...)`)
    let any_lombok = imported
        || anns.iter().any(|a| a.name.starts_with("lombok."))
        || c.fields.iter().any(|f| f.annotations.iter().any(|a| a.name.starts_with("lombok.")))
        || c.methods.iter().any(|m| m.annotations.iter().any(|a| a.name.starts_with("lombok.")));
    if !any_lombok {
        return Vec::new();
    }
    let mut generated: Vec<RawClass> = Vec::new();
    let range = c.range;
    let data = has("Data").is_some();
    let value = has("Value").is_some();
    // @Accessors on the class overrides lombok.config
    let accessors = has("Accessors");
    let fluent = accessors.and_then(|a| a.bool_arg("fluent")).unwrap_or(cfg.fluent);
    let chain = accessors.and_then(|a| a.bool_arg("chain")).unwrap_or(cfg.chain) || fluent;

    // @UtilityClass: final class, every member static
    if has("UtilityClass").is_some() {
        c.flags.set(Flags::FINAL);
        for m in &mut c.methods {
            if !m.flags.has(Flags::CONSTRUCTOR) {
                m.flags.set(Flags::STATIC);
            }
        }
        for f in &mut c.fields {
            f.flags.set(Flags::STATIC);
        }
    }

    // @Value: final class (unless @NonFinal), private final instance fields
    // (@NonFinal keeps a field assignable, @PackagePrivate keeps its access)
    if value {
        if has("NonFinal").is_none() {
            c.flags.set(Flags::FINAL);
        }
        for f in &mut c.fields {
            if f.flags.is_static() {
                continue;
            }
            if !f.annotations.iter().any(|a| a.simple_name() == "NonFinal") {
                f.flags.set(Flags::FINAL);
            }
            if !f.flags.has(Flags::PUBLIC | Flags::PROTECTED | Flags::PRIVATE) && !f.annotations.iter().any(|a| a.simple_name() == "PackagePrivate") {
                f.flags.set(Flags::PRIVATE);
            }
        }
    }

    // @FieldDefaults(makeFinal = true): instance fields become final (so
    // @Data / @RequiredArgsConstructor take them, setters skip them)
    if has("FieldDefaults").is_some_and(|a| a.bool_arg("makeFinal") == Some(true)) {
        for f in &mut c.fields {
            if !f.flags.is_static() && !f.flags.has(Flags::ENUM_CONSTANT) && !f.annotations.iter().any(|a| a.simple_name() == "NonFinal") {
                f.flags.set(Flags::FINAL);
            }
        }
    }

    // Loggers
    for (ann, ty) in [
        ("Slf4j", "org.slf4j.Logger"),
        ("XSlf4j", "org.slf4j.ext.XLogger"),
        ("Log4j2", "org.apache.logging.log4j.Logger"),
        ("Log4j", "org.apache.log4j.Logger"),
        ("Log", "java.util.logging.Logger"),
        ("CommonsLog", "org.apache.commons.logging.Log"),
        ("JBossLog", "org.jboss.logging.Logger"),
        ("Flogger", "com.google.common.flogger.FluentLogger"),
    ] {
        let log = cfg.log_field_name.as_deref().unwrap_or("log");
        if has(ann).is_some() && !c.fields.iter().any(|f| &*f.name == log) {
            c.fields.push(RawField {
                name: Arc::from(log),
                ty: TypeSyntax::qualified(ty),
                flags: Flags(Flags::PRIVATE | Flags::STATIC | Flags::FINAL | Flags::GENERATED),
                annotations: Vec::new(),
                constant: None,
                init: None,
                has_initializer: true,
                range: (0, 0),
            });
        }
    }

    let getter_name = |field: &str, ty: &TypeSyntax| if cfg.no_is_prefix && ty.is_boolean_prim() { format!("get{}", cap(field)) } else { getter_name(field, ty) };
    let class_getter = has("Getter").filter(|a| !access_none(a)).is_some() || data || value;
    let class_setter = has("Setter").filter(|a| !access_none(a)).is_some() || data;
    let fields = c.fields.clone();
    for f in &fields {
        if f.flags.has(Flags::ENUM_CONSTANT) {
            continue;
        }
        // static fields only get accessors from field-level annotations
        if f.flags.is_static() {
            let stat = Flags::PUBLIC | Flags::STATIC;
            if f.annotations.iter().any(|a| is_lombok(a, "Getter", imported) && !access_none(a)) {
                let n = getter_name(&strip_prefix_name(&f.name, &cfg.prefixes), &f.ty);
                if !has_method(c, &n, 0) {
                    c.methods.push(gen_method(&n, stat, vec![], f.ty.clone(), f.range));
                }
            }
            if !f.flags.is_final() && f.annotations.iter().any(|a| is_lombok(a, "Setter", imported) && !access_none(a)) {
                let n = setter_name(&strip_prefix_name(&f.name, &cfg.prefixes), &f.ty);
                if !has_method(c, &n, 1) {
                    c.methods.push(gen_method(&n, stat, vec![param(&f.name, f.ty.clone())], TypeSyntax::Void, f.range));
                }
            }
            continue;
        }
        let fg = f.annotations.iter().find(|a| is_lombok(a, "Getter", imported));
        let fs = f.annotations.iter().find(|a| is_lombok(a, "Setter", imported));
        let want_get = match fg {
            Some(a) => !access_none(a),
            None => class_getter,
        };
        let want_set = match fs {
            Some(a) => !access_none(a),
            None => class_setter && !f.flags.is_final(),
        } && !f.flags.is_final();
        let base = strip_prefix_name(&f.name, &cfg.prefixes);
        if want_get {
            let n = if fluent { base.clone() } else { getter_name(&base, &f.ty) };
            if !has_method(c, &n, 0) {
                c.methods.push(gen_method(&n, Flags::PUBLIC, vec![], f.ty.clone(), f.range));
            }
        }
        if want_set {
            let n = if fluent { base.clone() } else { setter_name(&base, &f.ty) };
            if !has_method(c, &n, 1) {
                let ret = if chain { self_type(c) } else { TypeSyntax::Void };
                c.methods.push(gen_method(&n, Flags::PUBLIC, vec![param(&f.name, f.ty.clone())], ret, f.range));
            }
        }
        if f.annotations.iter().any(|a| is_lombok(a, "With", imported) || is_lombok(a, "Wither", imported)) || has("With").is_some() || has("Wither").is_some() {
            // `boolean isReadable` gets `withReadable`, like its getter/setter
            let with_base = match base.strip_prefix("is") {
                Some(rest) if f.ty.is_boolean_prim() && rest.chars().next().is_some_and(|c| c.is_uppercase()) => rest.to_string(),
                _ => cap(&base),
            };
            let n = format!("with{with_base}");
            if !has_method(c, &n, 1) {
                let st = self_type(c);
                c.methods.push(gen_method(&n, Flags::PUBLIC, vec![param(&f.name, f.ty.clone())], st, f.range));
            }
        }
    }

    if (data || value || has("ToString").is_some())
        && !has_method(c, "toString", 0) {
            c.methods.push(gen_method("toString", Flags::PUBLIC, vec![], TypeSyntax::qualified("java.lang.String"), range));
        }
    // equals and hashCode come as a pair: a class that declares either one
    // itself (by name and parameter count) gets neither, and no canEqual.
    // canEqual is left out where no subclass could need it, in a final class
    // that extends Object (no `extends` clause, or `extends Object`); a final
    // class with a superclass gets it. A declared canEqual only stops
    // canEqual. (Lombok's HandleEqualsAndHashCode; the class files of
    // testcontainers and easyexcel show each case but the last: @Value
    // classes without a superclass have equals and hashCode but no canEqual,
    // a @Value class extending another class has all three, and a @Data or
    // @EqualsAndHashCode class declaring equals and hashCode has only its
    // own.)
    if (data || value || has("EqualsAndHashCode").is_some()) && !has_method(c, "equals", 1) && !has_method(c, "hashCode", 0) {
        c.methods.push(gen_method("equals", Flags::PUBLIC, vec![param("o", TypeSyntax::qualified("java.lang.Object"))], TypeSyntax::Prim(Prim::Boolean), range));
        c.methods.push(gen_method("hashCode", Flags::PUBLIC, vec![], TypeSyntax::Prim(Prim::Int), range));
        let extends_object = c.superclass.as_ref().is_none_or(|s| matches!(erased(s).0.as_deref(), Some("Object" | "java.lang.Object")));
        if !(c.flags.is_final() && extends_object) && !has_method(c, "canEqual", 1) {
            c.methods.push(gen_method("canEqual", Flags::PROTECTED, vec![param("other", TypeSyntax::qualified("java.lang.Object"))], TypeSyntax::Prim(Prim::Boolean), range));
        }
    }

    // Constructors. A @Builder.Default field's initializer moves into a
    // static method, so for constructors it counts as uninitialized.
    let builder_default = |f: &RawField| f.annotations.iter().any(|a| a.name.ends_with("Builder.Default") || (&*a.name == "Default" && imported));
    let initialized = |f: &RawField| f.has_initializer && !builder_default(f);
    // Lombok leaves fields whose name starts with `$` out of constructors
    let instance_fields: Vec<&RawField> = fields.iter().filter(|f| !f.flags.is_static() && !f.flags.has(Flags::ENUM_CONSTANT) && !f.name.starts_with('$')).collect();
    let required: Vec<&RawField> = instance_fields
        .iter()
        .copied()
        .filter(|f| !initialized(f) && (f.flags.is_final() || f.annotations.iter().any(|a| non_null(a, imports))))
        .collect();
    let all: Vec<&RawField> = instance_fields.iter().copied().filter(|f| !(f.flags.is_final() && initialized(f))).collect();
    let explicit_ctor = c.methods.iter().any(|m| m.flags.has(Flags::CONSTRUCTOR) && !m.flags.has(Flags::IMPLICIT));
    let mut new_ctors: Vec<Vec<&RawField>> = Vec::new();
    // `staticName = "of"` (constructor annotations) and
    // `@Data(staticConstructor = "of")` add a static factory
    let mut factories: Vec<(String, Vec<&RawField>)> = Vec::new();
    let static_name = |a: &RawAnnotation, key: &str| match a.arg(key) {
        Some(RawAnnValue::Const(Constant::Str(n))) if !n.is_empty() => Some(n.to_string()),
        _ => None,
    };
    if let Some(a) = has("NoArgsConstructor") {
        new_ctors.push(Vec::new());
        factories.extend(static_name(a, "staticName").map(|n| (n, Vec::new())));
    }
    if let Some(a) = has("AllArgsConstructor") {
        new_ctors.push(all.clone());
        factories.extend(static_name(a, "staticName").map(|n| (n, all.clone())));
    }
    if let Some(a) = has("RequiredArgsConstructor") {
        new_ctors.push(required.clone());
        factories.extend(static_name(a, "staticName").map(|n| (n, required.clone())));
    }
    if let Some(a) = has("Data") {
        factories.extend(static_name(a, "staticConstructor").map(|n| (n, required.clone())));
    }
    // @Value(staticConstructor = "of"): a factory taking every field (its
    // all-args constructor, made private)
    if let Some(a) = has("Value").filter(|_| !explicit_ctor && has("NoArgsConstructor").is_none() && has("RequiredArgsConstructor").is_none() && has("AllArgsConstructor").is_none() && has("Builder").is_none() && has("SuperBuilder").is_none()) {
        factories.extend(static_name(a, "staticConstructor").map(|n| (n, all.clone())));
    }
    // the factory is generated whatever methods of that name exist: a
    // hand-written `of(...)` is an overload unless it takes the same
    // parameter types (which would not compile)
    for (name, fs) in &factories {
        let same = |m: &RawMethod| &*m.name == name.as_str() && m.params.len() == fs.len() && m.params.iter().zip(fs.iter()).all(|(p, f)| erased(&p.ty) == erased(&f.ty));
        if !c.methods.iter().any(same) {
            let ps: Vec<RawParam> = fs.iter().map(|f| param(&f.name, f.ty.clone())).collect();
            c.methods.push(gen_method(name, Flags::PUBLIC | Flags::STATIC, ps, self_type(c), range));
        }
    }
    let has_ctor_ann = !new_ctors.is_empty();
    let builder = has("Builder").or_else(|| has("SuperBuilder")).cloned();
    // @Builder without constructors generates an all-args constructor, which
    // also keeps @Data from adding its required-args one
    let builder_ctor = builder.is_some() && !explicit_ctor && !has_ctor_ann;
    if builder_ctor {
        new_ctors.push(all.clone());
    }
    if data && !explicit_ctor && !has_ctor_ann && !builder_ctor {
        new_ctors.push(required.clone());
    }
    if value && !explicit_ctor && !has_ctor_ann && !builder_ctor {
        new_ctors.push(all.clone());
    }
    if !new_ctors.is_empty() {
        c.methods.retain(|m| !(m.flags.has(Flags::CONSTRUCTOR) && m.flags.has(Flags::IMPLICIT)));
        for fs in new_ctors {
            let ps: Vec<RawParam> = fs
                .iter()
                .map(|f| {
                    let mut p = param(&f.name, f.ty.clone());
                    // lombok.copyableAnnotations (e.g. Spring's @Lazy, @Qualifier)
                    p.annotations = f.annotations.iter().filter(|a| cfg.copyable.iter().any(|c| *c == *a.name || c.rsplit('.').next() == Some(a.simple_name()))).cloned().collect();
                    p
                })
                .collect();
            let dup = c.methods.iter().any(|m| m.flags.has(Flags::CONSTRUCTOR) && m.params.len() == ps.len() && m.params.iter().zip(&ps).all(|(a, b)| a.ty == b.ty));
            if !dup {
                let mut m = gen_method("<init>", Flags::PUBLIC | Flags::CONSTRUCTOR, ps, TypeSyntax::Void, range);
                // mark which fields were injected via this ctor, useful for DI
                m.annotations = Vec::new();
                c.methods.push(m);
            }
        }
    }

    // Builder on the class (all fields) ...
    let super_builder = has("SuperBuilder").is_some();
    if let Some(b) = builder {
        // fields named `$x` are not Lombok's to build; @Builder.Default
        // fields keep their initializer but are settable
        let items: Vec<Item> = instance_fields
            .iter()
            .filter(|f| !f.name.starts_with('$'))
            .filter(|f| !(f.flags.is_final() && f.has_initializer) || builder_default(f))
            .map(|f| Item::new(&strip_prefix_name(&f.name, &cfg.prefixes), f.ty.clone(), f.range, builder_default(f) && f.has_initializer, &f.annotations))
            .collect();
        if super_builder {
            generated.extend(super_builder_classes(c, &b, &items, cfg));
        } else {
            let tps = c.type_params.clone();
            let name = default_builder_class(c.simple_name(), cfg);
            generated.push(plain_builder(c, &b, &items, self_type(c), &name, tps));
        }
    }
    // ... and on constructors and static methods (their parameters)
    let annotated: Vec<(RawMethod, RawAnnotation)> = c
        .methods
        .iter()
        .filter(|m| !m.flags.has(Flags::GENERATED))
        .filter_map(|m| m.annotations.iter().find(|a| is_lombok(a, "Builder", imported)).map(|a| (m.clone(), a.clone())))
        .collect();
    for (m, b) in annotated {
        let items: Vec<Item> = m.params.iter().map(|p| Item::new(&p.name, p.ty.clone(), m.range, false, &p.annotations)).collect();
        let (built, default_name, tps) = if m.flags.has(Flags::CONSTRUCTOR) {
            (self_type(c), default_builder_class(c.simple_name(), cfg), c.type_params.clone())
        } else {
            // named after the simple name of the type the method returns
            let ret_simple = match &m.ret {
                TypeSyntax::Named(segs) => segs.last().map(|s| s.0.to_string()).unwrap_or_default(),
                TypeSyntax::Void => "Void".to_string(),
                TypeSyntax::Prim(p) => cap(p.keyword()),
                _ => String::new(),
            };
            (m.ret.clone(), default_builder_class(&ret_simple, cfg), m.type_params.clone())
        };
        generated.push(plain_builder(c, &b, &items, built, &default_name, tps));
    }

    // @StandardException: the four constructors of an exception, those not
    // written by hand
    if has("StandardException").is_some() {
        let string = || TypeSyntax::qualified("java.lang.String");
        let throwable = || TypeSyntax::qualified("java.lang.Throwable");
        for ps in [vec![], vec![param("message", string())], vec![param("cause", throwable())], vec![param("message", string()), param("cause", throwable())]] {
            let exists = c.methods.iter().any(|m| m.flags.has(Flags::CONSTRUCTOR) && !m.flags.has(Flags::IMPLICIT) && m.params.len() == ps.len() && m.params.iter().zip(&ps).all(|(a, b)| erased(&a.ty).0.as_deref().map(simple_name_of) == erased(&b.ty).0.as_deref().map(simple_name_of)));
            if !exists {
                c.methods.retain(|m| !(m.flags.has(Flags::CONSTRUCTOR) && m.flags.has(Flags::IMPLICIT)));
                c.methods.push(gen_method("<init>", Flags::PUBLIC | Flags::CONSTRUCTOR, ps, TypeSyntax::Void, range));
            }
        }
    }

    // @Synchronized and @Locked without a lock name: the lock field (`$lock`,
    // or `$LOCK` for a static method)
    let mut locks: Vec<(&str, &str, bool)> = Vec::new();
    for m in &c.methods {
        let named = |a: &RawAnnotation| matches!(a.arg("value"), Some(RawAnnValue::Const(Constant::Str(s))) if !s.is_empty());
        let stat = m.flags.is_static();
        for a in &m.annotations {
            let ty = match &*a.name {
                n if is_lombok(a, "Synchronized", imported) && (n == "Synchronized" || n == "lombok.Synchronized") => "java.lang.Object",
                "Locked" | "lombok.Locked" if imported || a.name.starts_with("lombok.") => "java.util.concurrent.locks.Lock",
                "Locked.Read" | "Locked.Write" | "lombok.Locked.Read" | "lombok.Locked.Write" if imported || a.name.starts_with("lombok.") => "java.util.concurrent.locks.ReadWriteLock",
                _ => continue,
            };
            if !named(a) {
                locks.push((if stat { "$LOCK" } else { "$lock" }, ty, stat));
            }
        }
    }
    for (name, ty, stat) in locks {
        if !c.fields.iter().any(|f| &*f.name == name) {
            c.fields.push(gen_field(name, TypeSyntax::qualified(ty), Flags::PRIVATE | Flags::FINAL | if stat { Flags::STATIC } else { 0 }, None));
        }
    }

    // @FieldNameConstants: a nested `Fields` class (or enum) with a constant
    // per field
    if let Some(a) = has("FieldNameConstants") {
        generated.extend(field_name_constants(c, a, &fields, imports, cfg));
    }
    generated
}

fn gen_field(name: &str, ty: TypeSyntax, flags: u32, constant: Option<Constant>) -> RawField {
    RawField { name: Arc::from(name), ty, flags: Flags(flags | Flags::GENERATED), annotations: Vec::new(), constant, init: None, has_initializer: true, range: (0, 0) }
}

/// The builder class's name for a built type: `lombok.builder.className`
/// with `*` standing for the type's name, by default `*Builder`.
fn default_builder_class(type_name: &str, cfg: &LombokConfig) -> String {
    match cfg.builder_class_name.as_deref() {
        Some(p) if !p.is_empty() => p.replace('*', type_name),
        _ => format!("{type_name}Builder"),
    }
}

/// One property of a builder: a field of the class, or a parameter of the
/// annotated constructor or method.
struct Item {
    /// the property's name (a field's without its configured prefix)
    name: String,
    ty: TypeSyntax,
    range: (u32, u32),
    /// a `@Builder.Default` field: stored as `name$value`, with `name$set`
    default: bool,
    /// `@Singular`: the singular name and what the collection holds
    singular: Option<(String, SingularKind)>,
}

impl Item {
    fn new(name: &str, ty: TypeSyntax, range: (u32, u32), default: bool, anns: &[RawAnnotation]) -> Item {
        let singular = singular_of(anns).and_then(|explicit| {
            let kind = singular_kind(&ty)?;
            Some((explicit.or_else(|| auto_singularize(name))?, kind))
        });
        Item { name: name.to_string(), ty, range, default: default && singular.is_none(), singular }
    }
}

/// The fields and setter methods a builder has for its items, with setters
/// returning `ret`. Checked against Lombok 1.18.36 (javap of compiled
/// builders): a field per property (private, of the property's type), named
/// `x$value` with a `boolean x$set` for `@Builder.Default`; for `@Singular`
/// a `java.util.ArrayList` of the elements (two, `x$key` and `x$value`, for
/// a map) and the methods `item(e)`, `items(Collection<? extends E>)` (or
/// `Map<? extends K, ? extends V>`) and `clearItems()`; `setterPrefix`
/// prefixes every setter but `clearX`.
fn builder_members(items: &[Item], prefix: &str, ret: &TypeSyntax) -> (Vec<RawField>, Vec<RawMethod>) {
    let setter = |n: &str| if prefix.is_empty() { n.to_string() } else { format!("{prefix}{}", cap(n)) };
    let array_list = |t: &TypeSyntax| TypeSyntax::Named(vec![(Arc::from("java"), vec![]), (Arc::from("util"), vec![]), (Arc::from("ArrayList"), vec![t.clone()])]);
    let extends = |t: &TypeSyntax| TypeSyntax::Wildcard(Some((true, Box::new(t.clone()))));
    let util = |n: &str, args: Vec<TypeSyntax>| TypeSyntax::Named(vec![(Arc::from("java"), vec![]), (Arc::from("util"), vec![]), (Arc::from(n), args)]);
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    for it in items {
        let r = it.range;
        match &it.singular {
            None => {
                if it.default {
                    fields.push(gen_field(&format!("{}$value", it.name), it.ty.clone(), Flags::PRIVATE, None));
                    fields.push(gen_field(&format!("{}$set", it.name), TypeSyntax::Prim(Prim::Boolean), Flags::PRIVATE, None));
                } else {
                    fields.push(gen_field(&it.name, it.ty.clone(), Flags::PRIVATE, None));
                }
                methods.push(gen_method(&setter(&it.name), Flags::PUBLIC, vec![param(&it.name, it.ty.clone())], ret.clone(), r));
            }
            Some((one, SingularKind::Collection(e))) => {
                fields.push(gen_field(&it.name, array_list(e), Flags::PRIVATE, None));
                methods.push(gen_method(&setter(one), Flags::PUBLIC, vec![param(one, e.clone())], ret.clone(), r));
                methods.push(gen_method(&setter(&it.name), Flags::PUBLIC, vec![param(&it.name, util("Collection", vec![extends(e)]))], ret.clone(), r));
                methods.push(gen_method(&format!("clear{}", cap(&it.name)), Flags::PUBLIC, vec![], ret.clone(), r));
            }
            Some((one, SingularKind::Map(k, v))) => {
                fields.push(gen_field(&format!("{}$key", it.name), array_list(k), Flags::PRIVATE, None));
                fields.push(gen_field(&format!("{}$value", it.name), array_list(v), Flags::PRIVATE, None));
                methods.push(gen_method(&setter(one), Flags::PUBLIC, vec![param(&format!("{one}Key"), k.clone()), param(&format!("{one}Value"), v.clone())], ret.clone(), r));
                methods.push(gen_method(&setter(&it.name), Flags::PUBLIC, vec![param(&it.name, util("Map", vec![extends(k), extends(v)]))], ret.clone(), r));
                methods.push(gen_method(&format!("clear{}", cap(&it.name)), Flags::PUBLIC, vec![], ret.clone(), r));
            }
        }
    }
    (fields, methods)
}

fn str_arg(b: &RawAnnotation, key: &str) -> Option<String> {
    match b.arg(key) {
        Some(RawAnnValue::Const(Constant::Str(s))) => Some(s.to_string()),
        _ => None,
    }
}

/// (builder class name, builder method name or None when `""` turns it
/// off, build method name, setter prefix)
fn builder_names(b: &RawAnnotation, default_class: &str) -> (String, Option<String>, String, String) {
    let class = str_arg(b, "builderClassName").filter(|s| !s.is_empty()).unwrap_or_else(|| default_class.to_string());
    let method = match str_arg(b, "builderMethodName") {
        Some(s) if s.is_empty() => None,
        Some(s) => Some(s),
        None => Some("builder".to_string()),
    };
    let build = str_arg(b, "buildMethodName").filter(|s| !s.is_empty()).unwrap_or_else(|| "build".to_string());
    let prefix = str_arg(b, "setterPrefix").unwrap_or_default();
    (class, method, build, prefix)
}

fn generated_class(name: String, outer: &RawClass, type_params: Vec<RawTypeParam>, superclass: Option<TypeSyntax>, methods: Vec<RawMethod>, extra_flags: u32) -> RawClass {
    RawClass {
        name: Arc::from(name.as_str()),
        kind: ClassKind::Class,
        flags: Flags(Flags::PUBLIC | Flags::STATIC | Flags::GENERATED | extra_flags),
        type_params,
        superclass,
        interfaces: Vec::new(),
        permits: Vec::new(),
        fields: Vec::new(),
        methods,
        annotations: Vec::new(),
        outer: Some(outer.name.clone()),
        inners: Vec::new(),
        range: outer.range,
        decl_range: outer.decl_range,
    }
}

/// `@Builder`: `static XBuilder builder()` on `c` and the builder class with
/// a field and setter per item and `build()` returning `built`. The builder
/// class takes the type parameters of the class (for a class or
/// constructor) or of the method, and so does `builder()`.
fn plain_builder(c: &mut RawClass, b: &RawAnnotation, items: &[Item], built: TypeSyntax, default_class: &str, tps: Vec<RawTypeParam>) -> RawClass {
    let range = c.range;
    let (bname, method_name, build, prefix) = builder_names(b, default_class);
    let targs: Vec<TypeSyntax> = tps.iter().map(|t| TypeSyntax::simple(&t.name)).collect();
    let bty = TypeSyntax::Named(vec![(Arc::from(c.simple_name()), vec![]), (Arc::from(bname.as_str()), targs)]);
    if let Some(method_name) = method_name {
        if !has_method(c, &method_name, 0) {
            let mut m = gen_method(&method_name, Flags::PUBLIC | Flags::STATIC, vec![], bty.clone(), range);
            m.type_params = tps.clone();
            c.methods.push(m);
        }
    }
    if b.bool_arg("toBuilder") == Some(true) && !has_method(c, "toBuilder", 0) {
        c.methods.push(gen_method("toBuilder", Flags::PUBLIC, vec![], bty.clone(), range));
    }
    let (fields, mut methods) = builder_members(items, &prefix, &bty);
    methods.push(gen_method(&build, Flags::PUBLIC, vec![], built, range));
    methods.push(gen_method("toString", Flags::PUBLIC, vec![], TypeSyntax::qualified("java.lang.String"), range));
    methods.push(gen_method("<init>", Flags::CONSTRUCTOR, vec![], TypeSyntax::Void, range));
    let mut class = generated_class(format!("{}${}", c.name, bname), c, tps, None, methods, 0);
    class.fields = fields;
    class
}

/// `@SuperBuilder`, as Lombok generates it: an abstract
/// `XBuilder<C extends X, B extends XBuilder<C, B>>` extending the
/// superclass's builder (so inherited setters resolve through the hierarchy
/// and return `B`), holding the fields, and a concrete `XBuilderImpl extends
/// XBuilder<X, XBuilderImpl>` that `builder()` returns; `X` gets a
/// protected constructor taking the builder. A generic class's type
/// parameters come first. If the superclass has no such builder, the
/// hierarchy stays incomplete and analysis stays silent.
fn super_builder_classes(c: &mut RawClass, b: &RawAnnotation, items: &[Item], cfg: &LombokConfig) -> Vec<RawClass> {
    let range = c.range;
    let simple = c.simple_name().to_string();
    let (bname, method_name, build, prefix) = builder_names(b, &default_builder_class(&simple, cfg));
    let impl_name = format!("{bname}Impl");
    // the builder's own type variables, named apart from the names they
    // are written next to (Lombok picks `C2`, `B2`... the same way)
    let mut taken: Vec<String> = c.type_params.iter().map(|t| t.name.to_string()).collect();
    taken.push(simple.clone());
    if let Some(TypeSyntax::Named(segs)) = &c.superclass {
        taken.extend(segs.iter().map(|s| s.0.to_string()));
    }
    let fresh = |base: &str, taken: &[String]| (1..).map(|i| if i == 1 { base.to_string() } else { format!("{base}{i}") }).find(|n| !taken.contains(n)).unwrap();
    let cv = fresh("C", &taken);
    taken.push(cv.clone());
    let bv = fresh("B", &taken);
    let tv = |n: &str| TypeSyntax::simple(n);
    let class_args: Vec<TypeSyntax> = c.type_params.iter().map(|t| tv(&t.name)).collect();
    let with = |extra: Vec<TypeSyntax>| class_args.iter().cloned().chain(extra).collect::<Vec<_>>();
    let builder_of = |args: Vec<TypeSyntax>| TypeSyntax::Named(vec![(Arc::from(simple.as_str()), vec![]), (Arc::from(bname.as_str()), args)]);
    let impl_ty = TypeSyntax::Named(vec![(Arc::from(simple.as_str()), vec![]), (Arc::from(impl_name.as_str()), class_args.clone())]);
    let superclass = match &c.superclass {
        Some(TypeSyntax::Named(segs)) => {
            let mut segs = segs.clone();
            let (last, sargs) = segs.last().map(|s| (s.0.to_string(), s.1.clone())).unwrap_or_default();
            if let Some(l) = segs.last_mut() {
                l.1 = Vec::new();
            }
            segs.push((Arc::from(default_builder_class(&last, cfg).as_str()), sargs.into_iter().chain([tv(&cv), tv(&bv)]).collect()));
            Some(TypeSyntax::Named(segs))
        }
        _ => None,
    };
    if let Some(method_name) = &method_name {
        if !has_method(c, method_name, 0) {
            let mut m = gen_method(method_name, Flags::PUBLIC | Flags::STATIC, vec![], impl_ty.clone(), range);
            m.type_params = c.type_params.clone();
            c.methods.push(m);
        }
    }
    if b.bool_arg("toBuilder") == Some(true) && !has_method(c, "toBuilder", 0) {
        c.methods.push(gen_method("toBuilder", Flags::PUBLIC, vec![], impl_ty.clone(), range));
    }
    // protected X(XBuilder<?, ?> b)
    let wild = TypeSyntax::Wildcard(None);
    if !c.methods.iter().any(|m| m.flags.has(Flags::CONSTRUCTOR) && !m.flags.has(Flags::IMPLICIT) && m.params.len() == 1 && m.params[0].ty.last_name() == Some(bname.as_str())) {
        let mut args = vec![wild.clone(); class_args.len()];
        args.extend([wild.clone(), wild]);
        // a declared constructor: the implicit default one is gone
        c.methods.retain(|m| !(m.flags.has(Flags::CONSTRUCTOR) && m.flags.has(Flags::IMPLICIT)));
        c.methods.push(gen_method("<init>", Flags::PROTECTED | Flags::CONSTRUCTOR, vec![param("b", builder_of(args))], TypeSyntax::Void, range));
    }
    let (fields, mut methods) = builder_members(items, &prefix, &tv(&bv));
    methods.push(gen_method(&build, Flags::PUBLIC | Flags::ABSTRACT, vec![], tv(&cv), range));
    methods.push(gen_method("self", Flags::PROTECTED | Flags::ABSTRACT, vec![], tv(&bv), range));
    // toBuilder copies an instance through `$fillValuesFrom`, which a
    // hand-written builder may override
    if b.bool_arg("toBuilder") == Some(true) {
        methods.push(gen_method("$fillValuesFrom", Flags::PROTECTED, vec![param("instance", tv(&cv))], tv(&bv), range));
    }
    methods.push(gen_method("toString", Flags::PUBLIC, vec![], TypeSyntax::qualified("java.lang.String"), range));
    methods.push(gen_method("<init>", Flags::CONSTRUCTOR | Flags::PUBLIC, vec![], TypeSyntax::Void, range));
    let mut params = c.type_params.clone();
    params.push(RawTypeParam { name: Arc::from(cv.as_str()), bounds: vec![self_type(c)] });
    params.push(RawTypeParam { name: Arc::from(bv.as_str()), bounds: vec![builder_of(with(vec![tv(&cv), tv(&bv)]))] });
    let mut abstract_builder = generated_class(format!("{}${}", c.name, bname), c, params, superclass, methods, Flags::ABSTRACT);
    abstract_builder.fields = fields;
    let impl_methods = vec![
        gen_method(&build, Flags::PUBLIC, vec![], self_type(c), range),
        gen_method("self", Flags::PROTECTED, vec![], impl_ty.clone(), range),
        gen_method("<init>", Flags::CONSTRUCTOR | Flags::PRIVATE, vec![], TypeSyntax::Void, range),
    ];
    let concrete = generated_class(format!("{}${}", c.name, impl_name), c, c.type_params.clone(), Some(builder_of(with(vec![self_type(c), impl_ty]))), impl_methods, Flags::FINAL);
    vec![abstract_builder, concrete]
}

/// `@FieldNameConstants` (checked against Lombok 1.18.36's
/// HandleFieldNameConstants and javap): a nested static final class
/// `Fields` (`innerTypeName`, or `lombok.fieldNameConstants.innerTypeName`)
/// with a `public static final String` per field holding its name, or with
/// `asEnum = true` an enum with a constant per field. Fields count unless
/// static or named `$...`; with `onlyExplicitlyIncluded` only those marked
/// `@FieldNameConstants.Include`; `@FieldNameConstants.Exclude` drops one.
/// `lombok.fieldNameConstants.uppercase` names the constants `CREATE_TIME`.
/// `level` sets the type's access (`NONE`: nothing is generated). A
/// hand-written type of that name is completed (see `apply_all_with`).
fn field_name_constants(c: &RawClass, a: &RawAnnotation, fields: &[RawField], imports: &[Import], cfg: &LombokConfig) -> Option<RawClass> {
    let level = match a.arg("level") {
        Some(RawAnnValue::Name(n)) => n.rsplit('.').next().unwrap_or("").to_string(),
        _ => "PUBLIC".to_string(),
    };
    let access = match level.as_str() {
        "NONE" => return None,
        "PUBLIC" => Flags::PUBLIC,
        "PROTECTED" => Flags::PROTECTED,
        "PRIVATE" => Flags::PRIVATE,
        _ => 0,
    };
    let as_enum = a.bool_arg("asEnum") == Some(true);
    let only_explicit = a.bool_arg("onlyExplicitlyIncluded") == Some(true);
    let inner = str_arg(a, "innerTypeName").filter(|s| !s.is_empty()).or_else(|| cfg.field_name_constants_type.clone().filter(|s| !s.is_empty())).unwrap_or_else(|| "Fields".to_string());
    // `@FieldNameConstants.Include`, or `@Include` imported from it
    let marked = |f: &RawField, which: &str| {
        f.annotations.iter().any(|x| x.name.ends_with(&format!("FieldNameConstants.{which}")) || processor_annotation(x, imports).is_some_and(|q| q == format!("lombok.experimental.FieldNameConstants.{which}")))
    };
    let ty = TypeSyntax::Named(vec![(Arc::from(c.simple_name()), vec![]), (Arc::from(inner.as_str()), vec![])]);
    let mut out_fields = Vec::new();
    for f in fields {
        if f.flags.has(Flags::GENERATED) || f.flags.has(Flags::ENUM_CONSTANT) {
            continue;
        }
        let include = if marked(f, "Exclude") {
            false
        } else if marked(f, "Include") {
            true
        } else {
            !only_explicit && !f.name.starts_with('$') && !f.flags.is_static()
        };
        if !include {
            continue;
        }
        let name = if cfg.field_name_constants_uppercase { camel_case_to_constant(&f.name) } else { f.name.to_string() };
        if as_enum {
            out_fields.push(gen_field(&name, ty.clone(), Flags::PUBLIC | Flags::STATIC | Flags::FINAL | Flags::ENUM_CONSTANT, None));
        } else {
            out_fields.push(gen_field(&name, TypeSyntax::qualified("java.lang.String"), Flags::PUBLIC | Flags::STATIC | Flags::FINAL, Some(Constant::Str(Arc::from(&*f.name)))));
        }
    }
    // no field qualifies: Lombok warns and generates nothing
    if out_fields.is_empty() {
        return None;
    }
    let range = c.range;
    let (kind, flags, superclass, methods) = if as_enum {
        let methods = vec![
            gen_method("values", Flags::PUBLIC | Flags::STATIC, vec![], TypeSyntax::Array(Box::new(ty.clone())), range),
            gen_method("valueOf", Flags::PUBLIC | Flags::STATIC, vec![param("name", TypeSyntax::qualified("java.lang.String"))], ty.clone(), range),
            gen_method("<init>", Flags::PRIVATE | Flags::CONSTRUCTOR, vec![], TypeSyntax::Void, range),
        ];
        let sup = TypeSyntax::Named(vec![(Arc::from("java"), vec![]), (Arc::from("lang"), vec![]), (Arc::from("Enum"), vec![ty.clone()])]);
        (ClassKind::Enum, Flags::STATIC | Flags::FINAL | access, Some(sup), methods)
    } else {
        (ClassKind::Class, Flags::STATIC | Flags::FINAL | access, None, vec![gen_method("<init>", Flags::PRIVATE | Flags::CONSTRUCTOR, vec![], TypeSyntax::Void, range)])
    };
    let mut class = generated_class(format!("{}${}", c.name, inner), c, Vec::new(), superclass, methods, 0);
    class.kind = kind;
    class.flags = Flags(flags | Flags::GENERATED);
    class.fields = out_fields;
    Some(class)
}

/// Lombok's `HandlerUtil.camelCaseToConstant`: `createTime` -> `CREATE_TIME`.
fn camel_case_to_constant(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if i > 0 && ch.is_uppercase() {
            out.push('_');
        }
        out.extend(ch.to_uppercase());
    }
    out
}

/// `@Singular` among annotations: Some(the singular name given, if any).
fn singular_of(anns: &[RawAnnotation]) -> Option<Option<String>> {
    let a = anns.iter().find(|a| a.simple_name() == "Singular" && (&*a.name == "Singular" || &*a.name == "lombok.Singular"))?;
    Some(match a.arg("value") {
        Some(RawAnnValue::Const(Constant::Str(s))) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    })
}

/// What `@Singular` makes of a `java.util` collection type: the element
/// types of a collection, or the key and value types of a map. None for a
/// type whose singular methods are not modelled (Guava's immutable
/// collections) or an element type written `? super T`.
#[derive(Debug, PartialEq)]
enum SingularKind {
    Collection(TypeSyntax),
    Map(TypeSyntax, TypeSyntax),
}

fn singular_kind(ty: &TypeSyntax) -> Option<SingularKind> {
    let TypeSyntax::Named(segs) = ty else { return None };
    let path: Vec<&str> = segs.iter().map(|s| &*s.0).collect();
    let simple = *path.last()?;
    let qualified = path.join(".");
    let known = |pkg: &str| path.len() == 1 || qualified == format!("{pkg}.{simple}");
    let args = &segs.last()?.1;
    // `? extends T` stands for T; a raw type for Object
    let elem = |i: usize| -> Option<TypeSyntax> {
        match args.get(i) {
            None => Some(TypeSyntax::qualified("java.lang.Object")),
            Some(TypeSyntax::Wildcard(None)) => Some(TypeSyntax::qualified("java.lang.Object")),
            Some(TypeSyntax::Wildcard(Some((true, t)))) => Some((**t).clone()),
            Some(TypeSyntax::Wildcard(Some((false, _)))) => None,
            Some(t) => Some(t.clone()),
        }
    };
    match simple {
        "List" | "Collection" | "Set" | "SortedSet" | "NavigableSet" if known("java.util") => Some(SingularKind::Collection(elem(0)?)),
        "Iterable" if known("java.lang") => Some(SingularKind::Collection(elem(0)?)),
        "Map" | "SortedMap" | "NavigableMap" if known("java.util") => Some(SingularKind::Map(elem(0)?, elem(1)?)),
        _ => None,
    }
}

/// Lombok's automatic singular of a plural name (`lombok.core.handlers.Singulars`,
/// with its rule table `singulars.txt`, 1.18.36): the first rule whose
/// ending matches decides; `!` means Lombok cannot tell (an error).
pub fn auto_singularize(name: &str) -> Option<String> {
    // (ending, replacement): an ending in capitals matches only a whole word
    // (or one starting at a capital); a leading `-` matches any word end
    // but keeps the character the ending starts with
    const RULES: &[(&str, &str)] = &[
        ("quizzes", "quiz"), ("matrices", "matrix"), ("indices", "index"), ("vertices", "vertex"), ("statuses", "status"), ("aliases", "alias"),
        ("alias", "!"), ("species", "!"), ("Axes", "!"), ("-axes", "axe"), ("sexes", "sex"), ("Testes", "testis"), ("movies", "movie"),
        ("octopodes", "octopus"), ("buses", "bus"), ("Mice", "mouse"), ("Lice", "louse"), ("News", "!"), ("Men", "man"), ("Women", "woman"),
        ("minutiae", "minutia"), ("shoes", "shoe"), ("synopses", "synopsis"), ("prognoses", "prognosis"), ("theses", "thesis"),
        ("diagnoses", "diagnosis"), ("bases", "base"), ("analyses", "analysis"), ("Crises", "crisis"), ("children", "child"), ("moves", "move"),
        ("zombies", "zombie"), ("-quies", "quy"), ("-us", "!"), ("-is", "!"), ("series", "!"), ("-ies", "y"), ("-oes", "o"), ("hives", "hive"),
        ("-tives", "tive"), ("-sses", "ss"), ("-ches", "ch"), ("-xes", "x"), ("-shes", "sh"), ("-lves", "lf"), ("-rves", "rf"), ("saves", "save"),
        ("Leaves", "leaf"), ("-ves", "!"), ("-ss", "!"), ("-us", "!"), ("-s", ""),
    ];
    let input: Vec<char> = name.chars().collect();
    let in_len = input.len();
    for (last_part, replacement) in RULES {
        let lp: Vec<char> = last_part.chars().collect();
        let whole_word = lp[0].is_uppercase();
        let ending_only = usize::from(lp[0] == '-');
        let len = lp.len();
        if in_len < len {
            continue;
        }
        let start = in_len - len + ending_only;
        let matches = input[start..].iter().zip(&lp[ending_only..]).all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()));
        if !matches {
            continue;
        }
        if whole_word && in_len != len && !input[in_len - len].is_uppercase() {
            continue;
        }
        if *replacement == "!" {
            return None;
        }
        let capitalize = !replacement.is_empty() && input[start].is_uppercase();
        let pre: String = input[..start].iter().collect();
        let post = if capitalize { cap(replacement) } else { replacement.to_string() };
        return Some(pre + &post);
    }
    None
}
