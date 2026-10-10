//! Model of Lombok's generated members. We do not run Lombok; we synthesize
//! the declarations it would produce so resolution and framework models see
//! the same API the compiler sees.

use crate::model::*;
use crate::stubs::*;
use std::sync::Arc;

const LOMBOK: &str = "lombok";

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
    let imported = imports.iter().any(|i| i.path.starts_with("lombok"));
    let mut extra = Vec::new();
    for c in classes.iter_mut() {
        if let Some(b) = apply_one(c, imported) {
            extra.push(b);
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
    let _ = apply_one(c, true);
}

fn apply_one(c: &mut RawClass, imported: bool) -> Option<RawClass> {
    let anns = c.annotations.clone();
    let has = |s: &str| anns.iter().find(|a| is_lombok(a, s, imported));
    let any_lombok = anns.iter().any(|a| a.name.starts_with("lombok.") || imported)
        || c.fields.iter().any(|f| f.annotations.iter().any(|a| a.name.starts_with("lombok.") || imported));
    if !any_lombok {
        return None;
    }
    let range = c.range;
    let data = has("Data").is_some();
    let value = has("Value").is_some();
    let fluent = has("Accessors").is_some_and(|a| a.bool_arg("fluent") == Some(true));

    if value {
        c.flags.set(Flags::FINAL);
        for f in &mut c.fields {
            if !f.flags.is_static() && !f.annotations.iter().any(|a| a.simple_name() == "NonFinal") {
                f.flags.set(Flags::FINAL);
                if !f.flags.has(Flags::PUBLIC | Flags::PROTECTED) && !f.annotations.iter().any(|a| a.simple_name() == "PackagePrivate") {
                    f.flags.set(Flags::PRIVATE);
                }
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
        if f.flags.is_static() || f.flags.has(Flags::ENUM_CONSTANT) {
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
        if want_get {
            let n = if fluent { f.name.to_string() } else { getter_name(&f.name, &f.ty) };
            if !has_method(c, &n, 0) {
                c.methods.push(gen_method(&n, Flags::PUBLIC, vec![], f.ty.clone(), f.range));
            }
        }
        if want_set {
            let n = if fluent { f.name.to_string() } else { setter_name(&f.name, &f.ty) };
            if !has_method(c, &n, 1) {
                let ret = if fluent { self_type(c) } else { TypeSyntax::Void };
                c.methods.push(gen_method(&n, Flags::PUBLIC, vec![param(&f.name, f.ty.clone())], ret, f.range));
            }
        }
        if f.annotations.iter().any(|a| is_lombok(a, "With", imported) || is_lombok(a, "Wither", imported)) || has("With").is_some() {
            let n = format!("with{}", cap(&f.name));
            if !has_method(c, &n, 1) {
                let st = self_type(c);
                c.methods.push(gen_method(&n, Flags::PUBLIC, vec![param(&f.name, f.ty.clone())], st, f.range));
            }
        }
    }

    if data || value || has("ToString").is_some() {
        if !has_method(c, "toString", 0) {
            c.methods.push(gen_method("toString", Flags::PUBLIC, vec![], TypeSyntax::qualified("java.lang.String"), range));
        }
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

    // Constructors
    let instance_fields: Vec<&RawField> = fields.iter().filter(|f| !f.flags.is_static() && !f.flags.has(Flags::ENUM_CONSTANT)).collect();
    let required: Vec<&RawField> = instance_fields
        .iter()
        .copied()
        .filter(|f| !f.has_initializer && (f.flags.is_final() || f.annotations.iter().any(|a| a.simple_name() == "NonNull")))
        .collect();
    let all: Vec<&RawField> = instance_fields.iter().copied().filter(|f| !(f.flags.is_final() && f.has_initializer)).collect();
    let explicit_ctor = c.methods.iter().any(|m| m.flags.has(Flags::CONSTRUCTOR) && !m.flags.has(Flags::IMPLICIT));
    let mut new_ctors: Vec<Vec<&RawField>> = Vec::new();
    if has("NoArgsConstructor").is_some() {
        new_ctors.push(Vec::new());
    }
    if has("AllArgsConstructor").is_some() {
        new_ctors.push(all.clone());
    }
    if has("RequiredArgsConstructor").is_some() {
        new_ctors.push(required.clone());
    }
    let has_ctor_ann = !new_ctors.is_empty();
    if data && !explicit_ctor && !has_ctor_ann {
        new_ctors.push(required.clone());
    }
    if value && !explicit_ctor && !has_ctor_ann {
        new_ctors.push(all.clone());
    }
    let builder = has("Builder").or_else(|| has("SuperBuilder")).cloned();
    if builder.is_some() && !explicit_ctor && !has_ctor_ann && !data && !value {
        new_ctors.push(all.clone());
    }
    if !new_ctors.is_empty() {
        c.methods.retain(|m| !(m.flags.has(Flags::CONSTRUCTOR) && m.flags.has(Flags::IMPLICIT)));
        for fs in new_ctors {
            let ps: Vec<RawParam> = fs.iter().map(|f| param(&f.name, f.ty.clone())).collect();
            let dup = c.methods.iter().any(|m| m.flags.has(Flags::CONSTRUCTOR) && m.params.len() == ps.len() && m.params.iter().zip(&ps).all(|(a, b)| a.ty == b.ty));
            if !dup {
                let mut m = gen_method("<init>", Flags::PUBLIC | Flags::CONSTRUCTOR, ps, TypeSyntax::Void, range);
                // mark which fields were injected via this ctor, useful for DI
                m.annotations = Vec::new();
                c.methods.push(m);
            }
        }
    }

    // Builder
    if let Some(b) = builder {
        let simple = c.simple_name().to_string();
        let bname = match b.arg("builderClassName") {
            Some(RawAnnValue::Const(Constant::Str(s))) => s.to_string(),
            _ => format!("{simple}Builder"),
        };
        let bty = TypeSyntax::Named(vec![(Arc::from(simple.as_str()), vec![]), (Arc::from(bname.as_str()), vec![])]);
        let method_name = match b.arg("builderMethodName") {
            Some(RawAnnValue::Const(Constant::Str(s))) => s.to_string(),
            _ => "builder".to_string(),
        };
        if !has_method(c, &method_name, 0) {
            c.methods.push(gen_method(&method_name, Flags::PUBLIC | Flags::STATIC, vec![], bty.clone(), range));
        }
        if b.bool_arg("toBuilder") == Some(true) && !has_method(c, "toBuilder", 0) {
            c.methods.push(gen_method("toBuilder", Flags::PUBLIC, vec![], bty.clone(), range));
        }
        let full = format!("{}${}", c.name, bname);
        let mut methods = Vec::new();
        for f in &all {
            methods.push(gen_method(&f.name, Flags::PUBLIC, vec![param(&f.name, f.ty.clone())], bty.clone(), f.range));
        }
        methods.push(gen_method("build", Flags::PUBLIC, vec![], self_type(c), range));
        methods.push(gen_method("toString", Flags::PUBLIC, vec![], TypeSyntax::qualified("java.lang.String"), range));
        methods.push(gen_method("<init>", Flags::CONSTRUCTOR, vec![], TypeSyntax::Void, range));
        return Some(RawClass {
            name: Arc::from(full.as_str()),
            kind: ClassKind::Class,
            flags: Flags(Flags::PUBLIC | Flags::STATIC | Flags::GENERATED),
            type_params: Vec::new(),
            superclass: None,
            interfaces: Vec::new(),
            permits: Vec::new(),
            fields: Vec::new(),
            methods,
            annotations: Vec::new(),
            outer: Some(c.name.clone()),
            inners: Vec::new(),
            range,
            decl_range: c.decl_range,
        });
    }
    None
}
