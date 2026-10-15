//! Model of Lombok's generated members. We do not run Lombok; we synthesize
//! the declarations it would produce so resolution and framework models see
//! the same API the compiler sees.

use crate::model::*;
use crate::stubs::*;
use std::sync::Arc;

const LOMBOK: &str = "lombok";

/// Settings from `lombok.config` files that change generated signatures.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
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

fn has_method(c: &RawClass, name: &str, arity: usize) -> bool {
    c.methods.iter().any(|m| &*m.name == name && m.params.len() == arity)
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
        extra.extend(apply_one(c, imported, cfg));
    }
    // @UtilityClass makes nested classes static too
    let utility: Vec<Name> = classes.iter().filter(|c| c.annotations.iter().any(|a| is_lombok(a, "UtilityClass", imported))).map(|c| c.name.clone()).collect();
    for c in classes.iter_mut() {
        if c.outer.as_ref().is_some_and(|o| utility.contains(o)) {
            c.flags.set(Flags::STATIC);
        }
    }
    for b in extra {
        // merge with a hand-written builder class if present
        if let Some(existing) = classes.iter_mut().find(|c| c.name == b.name) {
            for m in b.methods {
                if !has_method(existing, &m.name, m.params.len()) {
                    existing.methods.push(m);
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
    let _ = apply_one(c, true, &LombokConfig::default());
}

fn apply_one(c: &mut RawClass, imported: bool, cfg: &LombokConfig) -> Vec<RawClass> {
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
        if has(ann).is_some() && !c.fields.iter().any(|f| &*f.name == "log") {
            c.fields.push(RawField {
                name: Arc::from("log"),
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
        if f.annotations.iter().any(|a| is_lombok(a, "With", imported) || is_lombok(a, "Wither", imported)) || has("With").is_some() {
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
    if data || value || has("EqualsAndHashCode").is_some() {
        if !has_method(c, "equals", 1) {
            c.methods.push(gen_method("equals", Flags::PUBLIC, vec![param("o", TypeSyntax::qualified("java.lang.Object"))], TypeSyntax::Prim(Prim::Boolean), range));
        }
        if !has_method(c, "hashCode", 0) {
            c.methods.push(gen_method("hashCode", Flags::PUBLIC, vec![], TypeSyntax::Prim(Prim::Int), range));
        }
        if !has_method(c, "canEqual", 1) {
            c.methods.push(gen_method("canEqual", Flags::PROTECTED, vec![param("other", TypeSyntax::qualified("java.lang.Object"))], TypeSyntax::Prim(Prim::Boolean), range));
        }
    }

    // Constructors. A @Builder.Default field's initializer moves into a
    // static method, so for constructors it counts as uninitialized.
    let builder_default = |f: &RawField| f.annotations.iter().any(|a| a.name.ends_with("Builder.Default") || (&*a.name == "Default" && imported));
    let initialized = |f: &RawField| f.has_initializer && !builder_default(f);
    let instance_fields: Vec<&RawField> = fields.iter().filter(|f| !f.flags.is_static() && !f.flags.has(Flags::ENUM_CONSTANT)).collect();
    let required: Vec<&RawField> = instance_fields
        .iter()
        .copied()
        .filter(|f| !initialized(f) && (f.flags.is_final() || f.annotations.iter().any(|a| a.simple_name() == "NonNull")))
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
    for (name, fs) in &factories {
        if !has_method(c, name, fs.len()) {
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
        // @Builder.Default fields keep their initializer but are settable
        let items: Vec<(Name, TypeSyntax, (u32, u32))> = instance_fields
            .iter()
            .filter(|f| !(f.flags.is_final() && f.has_initializer) || builder_default(f))
            .map(|f| (f.name.clone(), f.ty.clone(), f.range))
            .collect();
        if super_builder {
            generated.extend(super_builder_classes(c, &b, &items));
        } else {
            let simple = c.simple_name().to_string();
            generated.push(plain_builder(c, &b, &items, self_type(c), &format!("{simple}Builder")));
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
        let items: Vec<(Name, TypeSyntax, (u32, u32))> = m.params.iter().map(|p| (p.name.clone(), p.ty.clone(), m.range)).collect();
        let (built, default_name) = if m.flags.has(Flags::CONSTRUCTOR) {
            (self_type(c), format!("{}Builder", c.simple_name()))
        } else {
            let ret_simple = match &m.ret {
                TypeSyntax::Named(segs) => segs.last().map(|s| s.0.to_string()).unwrap_or_default(),
                _ => String::new(),
            };
            (m.ret.clone(), format!("{ret_simple}Builder"))
        };
        generated.push(plain_builder(c, &b, &items, built, &default_name));
    }
    generated
}

fn builder_names(b: &RawAnnotation, default_class: &str) -> (String, String) {
    let class = match b.arg("builderClassName") {
        Some(RawAnnValue::Const(Constant::Str(s))) if !s.is_empty() => s.to_string(),
        _ => default_class.to_string(),
    };
    let method = match b.arg("builderMethodName") {
        Some(RawAnnValue::Const(Constant::Str(s))) if !s.is_empty() => s.to_string(),
        _ => "builder".to_string(),
    };
    (class, method)
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
/// one setter per item and `build()` returning `built`.
fn plain_builder(c: &mut RawClass, b: &RawAnnotation, items: &[(Name, TypeSyntax, (u32, u32))], built: TypeSyntax, default_class: &str) -> RawClass {
    let range = c.range;
    let (bname, method_name) = builder_names(b, default_class);
    let bty = TypeSyntax::Named(vec![(Arc::from(c.simple_name()), vec![]), (Arc::from(bname.as_str()), vec![])]);
    if !has_method(c, &method_name, 0) {
        c.methods.push(gen_method(&method_name, Flags::PUBLIC | Flags::STATIC, vec![], bty.clone(), range));
    }
    if b.bool_arg("toBuilder") == Some(true) && !has_method(c, "toBuilder", 0) {
        c.methods.push(gen_method("toBuilder", Flags::PUBLIC, vec![], bty.clone(), range));
    }
    let mut methods = Vec::new();
    for (name, ty, r) in items {
        methods.push(gen_method(name, Flags::PUBLIC, vec![param(name, ty.clone())], bty.clone(), *r));
    }
    methods.push(gen_method("build", Flags::PUBLIC, vec![], built, range));
    methods.push(gen_method("toString", Flags::PUBLIC, vec![], TypeSyntax::qualified("java.lang.String"), range));
    methods.push(gen_method("<init>", Flags::CONSTRUCTOR, vec![], TypeSyntax::Void, range));
    generated_class(format!("{}${}", c.name, bname), c, Vec::new(), None, methods, 0)
}

/// `@SuperBuilder`, as Lombok generates it: an abstract
/// `XBuilder<C extends X, B extends XBuilder<C, B>>` extending the
/// superclass's builder (so inherited setters resolve through the hierarchy
/// and return `B`), and a concrete `XBuilderImpl extends XBuilder<X,
/// XBuilderImpl>` that `builder()` returns. If the superclass has no such
/// builder, the hierarchy stays incomplete and analysis stays silent.
fn super_builder_classes(c: &mut RawClass, b: &RawAnnotation, items: &[(Name, TypeSyntax, (u32, u32))]) -> Vec<RawClass> {
    let range = c.range;
    let simple = c.simple_name().to_string();
    let (bname, method_name) = builder_names(b, &format!("{simple}Builder"));
    let impl_name = format!("{bname}Impl");
    let tv = |n: &str| TypeSyntax::simple(n);
    let builder_of = |args: Vec<TypeSyntax>| TypeSyntax::Named(vec![(Arc::from(simple.as_str()), vec![]), (Arc::from(bname.as_str()), args)]);
    let impl_ty = TypeSyntax::Named(vec![(Arc::from(simple.as_str()), vec![]), (Arc::from(impl_name.as_str()), vec![])]);
    let superclass = match &c.superclass {
        Some(TypeSyntax::Named(segs)) => {
            let mut segs = segs.clone();
            let last = segs.last().map(|s| s.0.to_string()).unwrap_or_default();
            segs.push((Arc::from(format!("{last}Builder").as_str()), vec![tv("C"), tv("B")]));
            Some(TypeSyntax::Named(segs))
        }
        _ => None,
    };
    if !has_method(c, &method_name, 0) {
        c.methods.push(gen_method(&method_name, Flags::PUBLIC | Flags::STATIC, vec![], impl_ty.clone(), range));
    }
    if b.bool_arg("toBuilder") == Some(true) && !has_method(c, "toBuilder", 0) {
        c.methods.push(gen_method("toBuilder", Flags::PUBLIC, vec![], impl_ty.clone(), range));
    }
    let mut methods = Vec::new();
    for (name, ty, r) in items {
        methods.push(gen_method(name, Flags::PUBLIC, vec![param(name, ty.clone())], tv("B"), *r));
    }
    methods.push(gen_method("build", Flags::PUBLIC | Flags::ABSTRACT, vec![], tv("C"), range));
    methods.push(gen_method("self", Flags::PROTECTED | Flags::ABSTRACT, vec![], tv("B"), range));
    methods.push(gen_method("toString", Flags::PUBLIC, vec![], TypeSyntax::qualified("java.lang.String"), range));
    methods.push(gen_method("<init>", Flags::CONSTRUCTOR, vec![], TypeSyntax::Void, range));
    let params = vec![
        RawTypeParam { name: Arc::from("C"), bounds: vec![self_type(c)] },
        RawTypeParam { name: Arc::from("B"), bounds: vec![builder_of(vec![tv("C"), tv("B")])] },
    ];
    let abstract_builder = generated_class(format!("{}${}", c.name, bname), c, params, superclass, methods, Flags::ABSTRACT);
    let impl_methods = vec![
        gen_method("build", Flags::PUBLIC, vec![], self_type(c), range),
        gen_method("self", Flags::PROTECTED, vec![], impl_ty.clone(), range),
        gen_method("<init>", Flags::CONSTRUCTOR, vec![], TypeSyntax::Void, range),
    ];
    let concrete = generated_class(format!("{}${}", c.name, impl_name), c, Vec::new(), Some(builder_of(vec![self_type(c), impl_ty])), impl_methods, 0);
    vec![abstract_builder, concrete]
}

