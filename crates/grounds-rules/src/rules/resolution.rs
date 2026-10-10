//! "Did I break something?" checks: references to methods, constructors and
//! fields that do not exist on project classes. Only reported when the whole
//! type hierarchy is known, so library gaps never produce noise. These are
//! the checks that catch an edit that renamed or re-shaped an API while
//! callers elsewhere still use the old shape.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_semantic::Symbol;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(UnresolvedMethod), Box::new(UnresolvedConstructor), Box::new(UnresolvedField)]
}

/// A project class whose full hierarchy is known and that no processor other
/// than our Lombok model adds members to.
fn closed_class<'t>(cx: &Ctx<'_, 't>, name: &str) -> Option<std::sync::Arc<ClassInfo>> {
    let c = cx.fm.class(name)?;
    if !c.is_source() || !cx.fm.hierarchy_complete(name) || cx.fm.index.is_duplicated(name) {
        return None;
    }
    // Lombok @Delegate / @ExtensionMethod add members we do not model
    let file = c.source_file()?;
    let stub = cx.fm.index.file(file)?;
    if stub.referenced_names.iter().any(|n| matches!(&**n, "Delegate" | "ExtensionMethod" | "Accessors")) {
        return None;
    }
    // every supertype must be closed as well (a library supertype is complete
    // only if we could read it)
    Some(c)
}

fn signatures(ms: &[MethodRef]) -> String {
    let mut v: Vec<String> = ms.iter().take(4).map(|m| m.method.signature_string()).collect();
    v.dedup();
    if ms.len() > 4 {
        v.push("...".into());
    }
    v.join(", ")
}

fn similar_names(c: &ClassInfo, lk: &dyn ClassLookup, name: &str) -> Option<String> {
    let all = lk.all_methods(&c.this_type());
    let lname = name.to_ascii_lowercase();
    let best = all
        .iter()
        .map(|m| &*m.method.name)
        .filter(|n| *n != "<init>")
        .map(|n| (edit_distance(&lname, &n.to_ascii_lowercase()), n))
        .filter(|(d, n)| *d <= 2.max(name.len() / 4) && *n != name)
        .min_by_key(|(d, _)| *d)?;
    Some(best.1.to_string())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

declare_rule!(UnresolvedMethod {
    id: "unresolved-method",
    name: "Call to a method that does not exist",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["javac:cant.resolve.location.args", "intellij:UnresolvedReference"],
    kinds: ["method_invocation"],
    description: "The called method does not exist on the receiver's (project) type with this number of arguments, so the code does not compile. Typically a caller that was not updated after a method was renamed, removed or changed its parameters.",
});

impl Rule for UnresolvedMethod {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = cx.fm.call_name(n) else { return };
        // `Iface.super.m()` parses as object `Iface` plus a `super` child
        if n.kid_of_kind("super").is_some() {
            return;
        }
        let nargs = cx.fm.args_of(n).len();
        // the receiver class whose members we inspect
        let (cls, is_static_ctx) = match n.field("object") {
            Some(o) => {
                // `super.m()` and `Iface.super.m()`
                if o.kind() == "super" || (o.kind() == "field_access" && o.kids().iter().any(|k| k.kind() == "super")) || cx.text(o).ends_with(".super") {
                    return;
                }
                match cx.fm.symbol(o) {
                    Some(Symbol::Type(t)) => match closed_class(cx, &t) {
                        Some(c) => (c, true),
                        None => return,
                    },
                    Some(Symbol::Package(_)) => return,
                    _ => {
                        let t = cx.type_of(o);
                        let Some(tn) = t.class_name() else { return };
                        match closed_class(cx, tn) {
                            Some(c) => (c, false),
                            None => return,
                        }
                    }
                }
            }
            None => {
                // implicit receiver: every enclosing class must be closed and
                // static imports must not be able to provide the name
                for imp in cx.fm.stub.imports.iter().filter(|i| i.is_static) {
                    if imp.on_demand || imp.last() == name {
                        return;
                    }
                }
                let mut cur = cx.fm.enclosing_class_node(n);
                let mut innermost = None;
                while let Some(c) = cur {
                    let Some(cn) = cx.fm.class_name_of_decl(c).cloned() else { return };
                    let Some(ci) = cx.fm.class(&cn) else { return };
                    if !ci.is_source() || !cx.fm.hierarchy_complete(&cn) {
                        return;
                    }
                    if !cx.fm.methods_named(&ci.this_type(), name).is_empty() {
                        return;
                    }
                    innermost.get_or_insert(ci);
                    cur = cx.fm.enclosing_class_node(c);
                }
                let Some(ci) = innermost else { return };
                if closed_class(cx, &ci.name).is_none() {
                    return;
                }
                let hint = similar_names(&ci, cx.fm, name).map(|s| format!("; did you mean `{s}`?")).unwrap_or_default();
                cx.report(n.field("name").unwrap_or(n), format!("Method `{name}` is not defined in `{}` or its enclosing classes{hint}", ci.simple_name()));
                return;
            }
        };
        let ms = cx.fm.methods_named(&cls.this_type(), name);
        if ms.is_empty() {
            let hint = similar_names(&cls, cx.fm, name).map(|s| format!("; did you mean `{s}`?")).unwrap_or_default();
            cx.report(n.field("name").unwrap_or(n), format!("`{}` has no method `{name}`{hint}", cls.simple_name()));
            return;
        }
        if !ms.iter().any(|m| m.method.accepts_arity(nargs)) {
            cx.report(n.field("name").unwrap_or(n), format!("No `{name}` in `{}` takes {nargs} argument(s); available: {}", cls.simple_name(), signatures(&ms)));
            return;
        }
        if let Some(arg_list) = definitely_incompatible(cx, n, &ms) {
            cx.report(n.field("name").unwrap_or(n), format!("No `{name}` in `{}` accepts ({arg_list}); available: {}", cls.simple_name(), signatures(&ms)));
            return;
        }
        // calling an instance method through the type name
        if is_static_ctx && ms.iter().filter(|m| m.method.accepts_arity(nargs)).all(|m| !m.method.flags.is_static()) {
            // `Outer.method()` inside an inner class reaches the outer instance legally
            let inside = cx.fm.enclosing_class_name(n).is_some_and(|e| e.starts_with(&*cls.name));
            if !inside {
                cx.report(n.field("name").unwrap_or(n), format!("`{}.{name}()` is an instance method and cannot be called on the type", cls.simple_name()));
            }
        }
    }
}

/// If every overload with a matching arity has a parameter that definitely
/// cannot accept the corresponding argument, return the argument types.
fn definitely_incompatible<'t>(cx: &Ctx<'_, 't>, call: Node<'t>, ms: &[MethodRef]) -> Option<String> {
    let args = cx.fm.args_of(call);
    let arg_tys: Vec<Ty> = args.iter().map(|a| cx.type_of(*a)).collect();
    // lambdas, method refs, unknown or generic argument types: not decidable
    if args.iter().any(|a| matches!(a.unparen().kind(), "lambda_expression" | "method_reference")) || arg_tys.iter().any(|t| t.has_unknown() || t.mentions_type_var() || *t == Ty::Null) {
        return None;
    }
    let mut any = false;
    for m in ms.iter().filter(|m| m.method.accepts_arity(args.len())) {
        any = true;
        let np = m.method.params.len();
        let ok = arg_tys.iter().enumerate().all(|(i, at)| {
            let pt = if m.method.is_varargs() && i + 1 >= np {
                match &m.method.params[np - 1].ty {
                    Ty::Array(e) if !(args.len() == np && at.is_array()) => (**e).clone(),
                    t => t.clone(),
                }
            } else {
                m.method.params[i].ty.clone()
            };
            if pt.has_unknown() || pt.mentions_type_var() {
                return true;
            }
            // compile-time narrowing of int constants (byte b = 1 style) is not applied to calls,
            // but keep constants permissive anyway
            !cx.fm.is_assignable(at, &pt).is_no()
        });
        if ok {
            return None;
        }
    }
    if !any {
        return None;
    }
    Some(arg_tys.iter().map(|t| t.display_simple()).collect::<Vec<_>>().join(", "))
}

declare_rule!(UnresolvedConstructor {
    id: "unresolved-constructor",
    name: "No constructor matches the arguments",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["javac:cant.apply.symbol", "intellij:UnresolvedReference"],
    kinds: ["object_creation_expression"],
    description: "No constructor of the (project) class takes this number of arguments, so the code does not compile: usually a call site not updated after a constructor changed (including Lombok-generated constructors after adding a field).",
});

impl Rule for UnresolvedConstructor {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.kid_of_kind("class_body").is_some() {
            return;
        }
        let Some(t) = n.field("type") else { return };
        let ty = cx.fm.resolve_type_node(t);
        let Some(tn) = ty.class_name() else { return };
        let Some(cls) = closed_class(cx, tn) else { return };
        if cls.is_interface() || cls.is_enum() {
            return;
        }
        let nargs = cx.fm.args_of(n).len();
        let ctors: Vec<&MethodInfo> = cls.constructors().collect();
        if ctors.is_empty() || ctors.iter().any(|c| c.accepts_arity(nargs)) {
            return;
        }
        let avail: Vec<String> = ctors.iter().take(4).map(|c| format!("{}({})", cls.simple_name(), c.params.iter().map(|p| p.ty.display_simple()).collect::<Vec<_>>().join(", "))).collect();
        cx.report(t, format!("No constructor of `{}` takes {nargs} argument(s); available: {}", cls.simple_name(), avail.join(", ")));
    }
}

declare_rule!(UnresolvedField {
    id: "unresolved-field",
    name: "Access to a field that does not exist",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["javac:cant.resolve.location", "intellij:UnresolvedReference"],
    kinds: ["field_access"],
    description: "The accessed field (or constant) does not exist on the (project) type, so the code does not compile: typically a reference left behind after a rename.",
});

impl Rule for UnresolvedField {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let (Some(o), Some(f)) = (n.field("object"), n.field("field")) else { return };
        if f.kind() != "identifier" || o.kind() == "super" {
            return;
        }
        // part of a qualified name `a.b.C.method()` or a method call receiver are fine to check,
        // but the field position of a method reference / annotation value is not an access
        if n.ancestors().any(|a| matches!(a.kind(), "annotation" | "import_declaration" | "package_declaration")) {
            return;
        }
        let fname = f.text(cx.src);
        if cx.fm.field_ref(n).is_some() {
            return;
        }
        let cls = match cx.fm.symbol(o) {
            Some(Symbol::Type(t)) => {
                // could be a nested type reference
                if cx.fm.index.member_type(&t, fname).is_some() || cx.fm.class(&format!("{t}${fname}")).is_some() {
                    return;
                }
                closed_class(cx, &t)
            }
            Some(Symbol::Package(_)) => return,
            _ => {
                let t = cx.type_of(o);
                if t.is_array() {
                    return;
                }
                t.class_name().and_then(|tn| closed_class(cx, tn))
            }
        };
        let Some(cls) = cls else { return };
        cx.report(f, format!("`{}` has no field `{fname}`", cls.simple_name()));
    }
}
