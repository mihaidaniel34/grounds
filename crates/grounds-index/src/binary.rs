//! Lowering of parsed class files into the shared model.

use crate::model::*;
use grounds_classfile::classfile::access;
use grounds_classfile::signature::{self, JType, JTypeArg, JTypeParam};
use grounds_classfile::{Annotation, ClassFile, ConstValue, ElementValue};
use std::sync::Arc;

pub fn internal_to_binary(s: &str) -> String {
    s.replace('/', ".")
}

pub fn jtype_to_ty(t: &JType) -> Ty {
    match t {
        JType::Base('V') => Ty::Void,
        JType::Base(c) => Prim::from_descriptor(*c).map(Ty::Prim).unwrap_or(Ty::Unknown),
        JType::Class { name, args } => Ty::Class { name: Arc::from(internal_to_binary(name).as_str()), args: args.iter().map(jarg_to_ty).collect() },
        JType::Array(e) => Ty::Array(Box::new(jtype_to_ty(e))),
        JType::TypeVar(n) => Ty::TypeVar(Arc::from(n.as_str())),
    }
}

fn jarg_to_ty(a: &JTypeArg) -> Ty {
    match a {
        JTypeArg::Any => Ty::Wildcard(None),
        JTypeArg::Exact(t) => jtype_to_ty(t),
        JTypeArg::Extends(t) => Ty::Wildcard(Some(Box::new(WildcardBound::Extends(jtype_to_ty(t))))),
        JTypeArg::Super(t) => Ty::Wildcard(Some(Box::new(WildcardBound::Super(jtype_to_ty(t))))),
    }
}

fn type_params(tps: &[JTypeParam]) -> Vec<TypeParam> {
    tps.iter().map(|p| TypeParam { name: Arc::from(p.name.as_str()), bounds: p.bounds.iter().map(jtype_to_ty).collect() }).collect()
}

fn desc_to_name(desc: &str) -> String {
    let d = desc.strip_prefix('L').and_then(|d| d.strip_suffix(';')).unwrap_or(desc);
    internal_to_binary(d)
}

fn element_value(v: &ElementValue) -> AnnotationValue {
    match v {
        ElementValue::Const(tag, c) => AnnotationValue::Const(match (tag, c) {
            ('Z', ConstValue::Int(i)) => Constant::Bool(*i != 0),
            ('C', ConstValue::Int(i)) => Constant::Char(char::from_u32(*i as u32).unwrap_or('\0')),
            (_, c) => const_value(c),
        }),
        ElementValue::Enum { type_desc, name } => {
            AnnotationValue::Enum { ty: Arc::from(desc_to_name(type_desc).as_str()), constant: Arc::from(name.as_str()) }
        }
        ElementValue::Class(d) => AnnotationValue::Class(signature::parse_field_type(d).map(|t| jtype_to_ty(&t)).unwrap_or(Ty::Unknown)),
        ElementValue::Annotation(a) => AnnotationValue::Annotation(Box::new(annotation(a))),
        ElementValue::Array(v) => AnnotationValue::Array(v.iter().map(element_value).collect()),
    }
}

fn const_value(c: &ConstValue) -> Constant {
    match c {
        ConstValue::Int(i) => Constant::Int(*i as i64),
        ConstValue::Long(i) => Constant::Long(*i),
        ConstValue::Float(f) => Constant::Float(*f as f64),
        ConstValue::Double(f) => Constant::Float(*f),
        ConstValue::String(s) => Constant::Str(Arc::from(s.as_str())),
    }
}

pub fn annotation(a: &Annotation) -> AnnotationInfo {
    AnnotationInfo {
        name: Arc::from(desc_to_name(&a.type_desc).as_str()),
        resolved: true,
        args: a.elements.iter().map(|(k, v)| (Arc::from(k.as_str()), element_value(v))).collect(),
    }
}

fn flags_of(acc: u16) -> Flags {
    let mut f = Flags::default();
    let map = [
        (access::PUBLIC, Flags::PUBLIC),
        (access::PRIVATE, Flags::PRIVATE),
        (access::PROTECTED, Flags::PROTECTED),
        (access::STATIC, Flags::STATIC),
        (access::FINAL, Flags::FINAL),
        (access::ABSTRACT, Flags::ABSTRACT),
        (access::SYNTHETIC, Flags::SYNTHETIC),
    ];
    for (a, b) in map {
        if acc & a != 0 {
            f.set(b);
        }
    }
    f
}

pub fn lower_class(cf: &ClassFile, container: u32) -> ClassInfo {
    let name: Name = Arc::from(internal_to_binary(&cf.this_class).as_str());
    let acc = cf.effective_access();
    let mut flags = flags_of(acc);
    let kind = if cf.access & access::ANNOTATION != 0 {
        ClassKind::Annotation
    } else if cf.access & access::INTERFACE != 0 {
        ClassKind::Interface
    } else if cf.access & access::ENUM != 0 {
        ClassKind::Enum
    } else if cf.super_class.as_deref() == Some("java/lang/Record") {
        ClassKind::Record
    } else {
        ClassKind::Class
    };
    let annotations: Vec<AnnotationInfo> = cf.annotations.iter().map(annotation).collect();
    if cf.deprecated || annotations.iter().any(|a| &*a.name == "java.lang.Deprecated") {
        flags.set(Flags::DEPRECATED);
    }
    let sig = cf.signature.as_deref().and_then(signature::parse_class_signature);
    let (tps, superclass, interfaces) = match &sig {
        Some(s) => (type_params(&s.type_params), s.superclass.as_ref().map(jtype_to_ty), s.interfaces.iter().map(jtype_to_ty).collect()),
        None => (
            Vec::new(),
            cf.super_class.as_ref().map(|s| Ty::class(&internal_to_binary(s))),
            cf.interfaces.iter().map(|i| Ty::class(&internal_to_binary(i))).collect(),
        ),
    };
    let outer = cf
        .inner_classes
        .iter()
        .find(|ic| ic.inner == cf.this_class)
        .and_then(|ic| ic.outer.as_ref())
        .map(|o| Arc::from(internal_to_binary(o).as_str()));
    let inners = cf
        .inner_classes
        .iter()
        .filter(|ic| ic.outer.as_deref() == Some(cf.this_class.as_str()) && ic.simple_name.is_some())
        .map(|ic| Arc::from(internal_to_binary(&ic.inner).as_str()))
        .collect();
    let is_inner_nonstatic = outer.is_some() && acc & access::STATIC == 0 && kind == ClassKind::Class;

    let mut fields = Vec::new();
    for f in &cf.fields {
        if f.access & access::SYNTHETIC != 0 {
            continue;
        }
        let ty = f
            .signature
            .as_deref()
            .and_then(signature::parse_field_type)
            .or_else(|| signature::parse_field_type(&f.descriptor))
            .map(|t| jtype_to_ty(&t))
            .unwrap_or(Ty::Unknown);
        let mut ff = flags_of(f.access);
        if f.access & access::VOLATILE != 0 {
            ff.set(Flags::VOLATILE);
        }
        if f.access & access::TRANSIENT != 0 {
            ff.set(Flags::TRANSIENT);
        }
        if f.access & access::ENUM != 0 {
            ff.set(Flags::ENUM_CONSTANT);
        }
        let anns: Vec<AnnotationInfo> = f.annotations.iter().map(annotation).collect();
        if f.deprecated || anns.iter().any(|a| &*a.name == "java.lang.Deprecated") {
            ff.set(Flags::DEPRECATED);
        }
        let constant = f.constant.as_ref().map(|c| match (f.descriptor.as_str(), c) {
            ("Z", ConstValue::Int(i)) => Constant::Bool(*i != 0),
            ("C", ConstValue::Int(i)) => Constant::Char(char::from_u32(*i as u32).unwrap_or('\0')),
            (_, c) => const_value(c),
        });
        fields.push(FieldInfo { name: Arc::from(f.name.as_str()), ty, flags: ff, annotations: anns, constant, range: None });
    }

    let mut methods = Vec::new();
    for m in &cf.methods {
        if m.access & (access::SYNTHETIC | access::BRIDGE) != 0 || m.name == "<clinit>" {
            continue;
        }
        if kind == ClassKind::Enum && m.name == "<init>" {
            continue;
        }
        let from_sig = m.signature.as_deref().and_then(signature::parse_method_signature);
        let used_sig = from_sig.is_some();
        let Some(ms) = from_sig.or_else(|| signature::parse_method_signature(&m.descriptor)) else { continue };
        let mut ptys: Vec<Ty> = ms.params.iter().map(jtype_to_ty).collect();
        if m.name == "<init>" && is_inner_nonstatic && !used_sig && !ptys.is_empty() {
            ptys.remove(0);
        }
        let mut ff = flags_of(m.access);
        if m.access & access::VARARGS != 0 {
            ff.set(Flags::VARARGS);
        }
        if m.access & access::SYNCHRONIZED != 0 {
            ff.set(Flags::SYNCHRONIZED);
        }
        if m.access & access::NATIVE != 0 {
            ff.set(Flags::NATIVE);
        }
        if m.name == "<init>" {
            ff.set(Flags::CONSTRUCTOR);
        }
        if kind == ClassKind::Interface && m.access & (access::ABSTRACT | access::STATIC | access::PRIVATE) == 0 {
            ff.set(Flags::DEFAULT);
        }
        let anns: Vec<AnnotationInfo> = m.annotations.iter().map(annotation).collect();
        if m.deprecated || anns.iter().any(|a| &*a.name == "java.lang.Deprecated") {
            ff.set(Flags::DEPRECATED);
        }
        let offset = m.param_annotations.len().saturating_sub(ptys.len());
        let params = ptys
            .into_iter()
            .enumerate()
            .map(|(i, ty)| ParamInfo {
                name: Arc::from(m.param_names.get(i).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| format!("arg{i}")).as_str()),
                ty,
                annotations: m.param_annotations.get(i + offset).map(|v| v.iter().map(annotation).collect()).unwrap_or_default(),
            })
            .collect();
        let throws = if ms.throws.is_empty() {
            m.exceptions.iter().map(|e| Ty::class(&internal_to_binary(e))).collect()
        } else {
            ms.throws.iter().map(jtype_to_ty).collect()
        };
        methods.push(MethodInfo {
            name: Arc::from(m.name.as_str()),
            flags: ff,
            type_params: type_params(&ms.type_params),
            params,
            ret: jtype_to_ty(&ms.ret),
            throws,
            annotations: anns,
            range: None,
            generated_by: None,
        });
    }

    ClassInfo {
        name,
        kind,
        flags,
        type_params: tps,
        superclass: if kind == ClassKind::Interface || kind == ClassKind::Annotation { None } else { superclass },
        interfaces,
        fields,
        methods,
        annotations,
        outer,
        inners,
        permits: cf.permitted_subclasses.iter().map(|p| Ty::class(&internal_to_binary(p))).collect(),
        origin: Origin::Binary { container },
        range: None,
    }
}
