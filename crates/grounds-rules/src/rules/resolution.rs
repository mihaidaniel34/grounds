//! "Did I break something?" checks: references to methods, constructors and
//! fields that do not exist on project classes. Only reported when the whole
//! type hierarchy is known, so library gaps never produce noise. These are
//! the checks that catch an edit that renamed or re-shaped an API while
//! callers elsewhere still use the old shape.

use crate::framework::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_semantic::Symbol;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(UnresolvedMethod), Box::new(UnresolvedConstructor), Box::new(UnresolvedField), Box::new(UnresolvedStaticImport), Box::new(MissingJdkImport), Box::new(IncompatibleAssignment), Box::new(OverrideWithoutSuper), Box::new(UnimplementedAbstractMethod)]
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
    // library supertypes are read from guessed jar versions (the newest in the
    // Gradle/Maven caches) unless the classpath was dumped exactly: a method
    // missing there may exist in the version the project builds against
    if cx.global_option("classpath.exact") != Some("true") {
        let library = cx.fm.all_supertypes(&c.this_type()).iter().filter_map(|t| t.class_name().map(str::to_string)).any(|n| !n.starts_with("java.") && cx.fm.class(&n).is_some_and(|k| !k.is_source()));
        if library {
            return None;
        }
    }
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
    kinds: ["method_invocation", "method_reference"],
    description: "The called method does not exist on the receiver's (project) type with this number of arguments, or is private to another class, so the code does not compile. Typically a caller that was not updated after a method was renamed, removed, made private or changed its parameters. Also checks method references (`Type::name`).",
});

impl Rule for UnresolvedMethod {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.kind() == "method_reference" {
            check_method_reference(n, cx);
            return;
        }
        // inside a case label's pattern (not its guard) a call is never valid
        // Java: it is a pattern the grammar did not understand (`case R(_)`)
        if n.ancestors().take_while(|a| a.kind() != "switch_block").any(|a| a.kind() == "switch_label") && !n.ancestors().any(|a| a.kind() == "guard") {
            return;
        }
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
                    if imp.on_demand || imp.is_named(name) {
                        return;
                    }
                }
                let mut cur = cx.fm.enclosing_class_node(n);
                let mut innermost = None;
                // a superclass's private method is no member here: the search
                // goes on outward
                let mut private_hit = None;
                while let Some(c) = cur {
                    let Some(cn) = cx.fm.class_name_of_decl(c).cloned() else { return };
                    let Some(ci) = cx.fm.class(&cn) else { return };
                    if !ci.is_source() || !cx.fm.hierarchy_complete(&cn) {
                        return;
                    }
                    let ms = cx.fm.methods_named(&ci.this_type(), name);
                    if !ms.is_empty() {
                        match private_elsewhere(cx, n, &ci, name, &ms, None) {
                            Some(m) => {
                                private_hit.get_or_insert(m);
                            }
                            None => return,
                        }
                    }
                    innermost.get_or_insert(ci);
                    cur = cx.fm.enclosing_class_node(c);
                }
                let Some(ci) = innermost else { return };
                if closed_class(cx, &ci.name).is_none() {
                    return;
                }
                if let Some(m) = private_hit {
                    cx.report(n.field("name").unwrap_or(n), format!("`{}` is private in `{}`", m.method.signature_string(), simple_name_of(&m.owner)));
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
        if let Some(m) = private_elsewhere(cx, n, &cls, name, &ms, Some(nargs)) {
            cx.report(n.field("name").unwrap_or(n), format!("`{}` is private in `{}`", m.method.signature_string(), simple_name_of(&m.owner)));
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

/// The method `n` would reach in `cls` when every candidate (of `nargs`
/// arguments, if given) is private to another top-level class: nested classes
/// share their top-level class's private members. `methods_named` may hide a
/// supertype's method (an interface default) behind a superclass's private
/// one, which is not inherited.
fn private_elsewhere<'t>(cx: &Ctx<'_, 't>, n: Node<'t>, cls: &ClassInfo, name: &str, ms: &[MethodRef], nargs: Option<usize>) -> Option<MethodRef> {
    let top = |c: &str| c.split('$').next().unwrap_or(c).to_string();
    let here = cx.fm.enclosing_class_name(n).map(|e| top(&e))?;
    let fits = |m: &MethodInfo| nargs.is_none_or(|k| m.accepts_arity(k));
    let cands: Vec<&MethodRef> = ms.iter().filter(|m| fits(&m.method)).collect();
    if cands.is_empty() || !cands.iter().all(|m| m.method.flags.is_private() && top(&m.owner) != here) {
        return None;
    }
    // a class defined in several modules: the index holds one of them
    if cx.fm.index.is_duplicated(&cls.name) || cands.iter().any(|m| cx.fm.index.is_duplicated(&m.owner)) {
        return None;
    }
    let visible_elsewhere = cx.fm.all_supertypes(&cls.this_type()).iter().filter_map(|t| t.class_name().and_then(|n| cx.fm.class(n))).any(|c| c.methods_named(name).any(|m| fits(m) && !m.flags.is_private()));
    (!visible_elsewhere).then(|| cands[0].clone())
}

/// `Type::name` / `expr::name` naming no method of a closed project class.
fn check_method_reference<'t>(n: Node<'t>, cx: &mut Ctx<'_, 't>) {
    // `Type::new` has a single named child; `::` and `new` are tokens
    let kids = n.kids();
    if kids.len() < 2 || cx.text(n).rsplit("::").next().is_some_and(|r| r.trim_start().starts_with("new")) {
        return;
    }
    let (recv, name) = (kids[0], kids[kids.len() - 1]);
    if name.kind() != "identifier" || matches!(recv.kind(), "super" | "this") || recv.kids().iter().any(|k| k.kind() == "super") {
        return;
    }
    let cls = match cx.fm.symbol(recv) {
        Some(Symbol::Type(t)) => closed_class(cx, &t),
        Some(Symbol::Package(_)) => None,
        _ => cx.type_of(recv).class_name().and_then(|t| closed_class(cx, t)),
    };
    let Some(cls) = cls else { return };
    let mname = cx.text(name);
    let ms = cx.fm.methods_named(&cls.this_type(), mname);
    if ms.is_empty() {
        let hint = similar_names(&cls, cx.fm, mname).map(|s| format!("; did you mean `{s}`?")).unwrap_or_default();
        cx.report(name, format!("`{}` has no method `{mname}`{hint}", cls.simple_name()));
    } else if let Some(m) = private_elsewhere(cx, n, &cls, mname, &ms, None) {
        cx.report(name, format!("`{}` is private in `{}`", m.method.signature_string(), simple_name_of(&m.owner)));
    }
}

declare_rule!(UnresolvedStaticImport {
    id: "unresolved-static-import",
    name: "Static import of a member that does not exist",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["javac:cant.resolve.location", "intellij:UnresolvedReference"],
    kinds: ["import_declaration"],
    description: "`import static pkg.Type.member;` names no static field, method or nested type of the (project) class, so the code does not compile: typically left behind after the member was renamed or removed.",
});

impl Rule for UnresolvedStaticImport {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        // `static` and `*` are tokens, not named children
        let text: String = cx.text(n).split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.starts_with("import static ") || text.contains('*') {
            return;
        }
        let Some(path) = n.kids().into_iter().find(|k| matches!(k.kind(), "scoped_identifier" | "identifier")) else { return };
        let text: String = cx.text(path).chars().filter(|c| !c.is_whitespace()).collect();
        let segs: Vec<&str> = text.split('.').collect();
        if segs.len() < 2 {
            return;
        }
        let member = segs[segs.len() - 1];
        // `a.b.Outer.Inner`: which segments are the package
        let owner = (1..segs.len() - 1).rev().find_map(|k| {
            let name = format!("{}.{}", segs[..k].join("."), segs[k..segs.len() - 1].join("$"));
            cx.fm.class(&name).map(|_| name)
        });
        let Some(owner) = owner else { return };
        let Some(cls) = closed_class(cx, &owner) else { return };
        let mut types: Vec<String> = vec![cls.name.to_string()];
        types.extend(cx.fm.all_supertypes(&cls.this_type()).iter().filter_map(|t| t.class_name().map(str::to_string)));
        let exists = types.iter().any(|t| {
            cx.fm.class(t).is_some_and(|c| c.field(member).is_some() || c.methods_named(member).next().is_some()) || cx.fm.class(&format!("{t}${member}")).is_some()
        });
        if !exists {
            let target = path.field("name").unwrap_or(path);
            cx.report(target, format!("`{}` has no static member `{member}`", cls.simple_name()));
        }
    }
}

declare_rule!(MissingJdkImport {
    id: "missing-jdk-import",
    name: "Import of a JDK class the Java release does not have",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["javac:doesnt.exist", "javac:cant.resolve.location"],
    kinds: ["import_declaration"],
    description: "The imported `java.*` class or package is not public API in the project's Java release, so the code does not compile for that release: a class added in a later release (`java.util.SequencedCollection` is Java 21), one removed since, or one the JDK does not export. Checked only when the JDK's API for the release is read exactly (see the JDK warnings).",
});

impl Rule for MissingJdkImport {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let cp = &cx.fm.index.classpath;
        let Some(release) = cp.exact_jdk_release() else { return };
        // multi-release sources (`src/main/java24`) compile for their own, newer release
        let own_release = cx.info.path.split(['/', '\\']).filter_map(|d| d.strip_prefix("java")).filter_map(|v| v.strip_prefix('-').unwrap_or(v).parse::<u32>().ok()).max();
        if own_release.is_some_and(|v| v > release) {
            return;
        }
        // `static` and `*` are tokens, not named children
        let words: String = cx.text(n).split_whitespace().collect::<Vec<_>>().join(" ");
        let is_static = words.starts_with("import static ");
        let on_demand = words.contains('*');
        let Some(path) = n.kids().into_iter().find(|k| matches!(k.kind(), "scoped_identifier" | "identifier")) else { return };
        let text: String = cx.text(path).chars().filter(|c| !c.is_whitespace()).collect();
        if !text.starts_with("java.") {
            return;
        }
        let segs: Vec<&str> = text.split('.').collect();
        // the type the import names: all of it, or all but the static member
        let ty = if is_static && !on_demand { &segs[..segs.len() - 1] } else { &segs[..] };
        let index = &cx.fm.index;
        // `a.b.Outer.Inner`: some prefix is the package
        let class_at = |k: usize| format!("{}.{}", ty[..k].join("."), ty[k..].join("$"));
        if (1..ty.len()).any(|k| index.class_exists(&class_at(k))) {
            return;
        }
        // `import java.util.*;` names a package
        if on_demand && !is_static && index.package_exists(&ty.join(".")) {
            return;
        }
        // the package, by convention the lowercase segments: a class missing
        // from a package the JDK has, or a package missing from the release
        let pkg_len = ty.iter().take_while(|s| s.starts_with(|c: char| c.is_ascii_lowercase())).count();
        let what = if on_demand && !is_static { format!("package `{}`", ty.join(".")) } else { format!("`{}`", ty.join(".")) };
        // a JDK cut down by jlink lacks whole modules, so their packages
        if !index.package_exists(&ty[..pkg_len].join(".")) && !cp.jdk_is_full() {
            return;
        }
        cx.report(path, format!("{what} is not public API in Java {release}"));
    }
}

declare_rule!(IncompatibleAssignment {
    id: "incompatible-assignment",
    name: "Call result assigned to an incompatible type",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["javac:prob.found.req", "intellij:IncompatibleTypes"],
    kinds: ["variable_declarator", "assignment_expression"],
    description: "A project method's result is assigned to a variable whose type it is not a subtype of, so the code does not compile: typically a caller left behind after the method's return type changed. Only plain class types are judged (no generics, primitives or `var`).",
});

impl Rule for IncompatibleAssignment {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        // (declared type, value)
        let (target, value) = if n.kind() == "variable_declarator" {
            let Some(v) = n.field("value") else { return };
            let Some(decl) = n.parent() else { return };
            let Some(tn) = decl.field("type") else { return };
            if cx.text(tn) == "var" || n.field("dimensions").is_some() {
                return;
            }
            (cx.fm.resolve_type_node(tn), v.unparen())
        } else {
            if n.field("operator").is_some_and(|o| cx.text(o) != "=") || cx.text(n).split_once('=').is_some_and(|(l, _)| l.ends_with(['+', '-', '*', '/', '%', '&', '|', '^', '<', '>'])) {
                return;
            }
            let (Some(l), Some(r)) = (n.field("left"), n.field("right")) else { return };
            // locals only: a field name may bind to an outer class's field
            if l.kind() != "identifier" || cx.fm.local_decl(l).is_none() {
                return;
            }
            (cx.type_of(l), r.unparen())
        };
        if value.kind() != "method_invocation" {
            return;
        }
        let Ty::Class { name: want, args } = &target else { return };
        if !args.is_empty() {
            return;
        }
        let Some(c) = cx.fm.select_method(value) else { return };
        if c.ambiguous || !cx.fm.class(&c.owner).is_some_and(|k| k.is_source()) {
            return;
        }
        // a class defined in several modules: the index holds one of them
        let receiver_class = c.receiver.as_ref().and_then(|r| r.class_name().map(str::to_string));
        if cx.fm.index.is_duplicated(&c.owner) || receiver_class.as_deref().is_some_and(|r| cx.fm.index.is_duplicated(r)) {
            return;
        }
        // a private method is never inherited, nestmates or not: found through
        // another class, the lookup may have put it in front of an interface
        // default that class has
        if c.method.flags.is_private() {
            let through = receiver_class.or_else(|| cx.fm.enclosing_class_name(value).map(|e| e.to_string()));
            if through.as_deref() != Some(&*c.owner) {
                return;
            }
        }
        // overload selection is approximate (type arguments, lambdas,
        // inaccessible overloads): judge only a method without rivals
        let nargs = cx.fm.args_of(value).len();
        // rivals the lookup above does not see: the other bounds of a type
        // variable (`T extends A & B`), methods a static import brings in
        match value.field("object") {
            Some(o) if matches!(cx.type_of(o), Ty::TypeVar(_)) => return,
            None if cx.fm.stub.imports.iter().any(|i| i.is_static && (i.on_demand || i.is_named(&c.method.name))) => return,
            _ => {}
        }
        let rivals = match &c.receiver {
            Some(r) => cx.fm.methods_named(r, &c.method.name).iter().filter(|m| m.method.accepts_arity(nargs)).count(),
            None => cx.fm.class(&c.owner).map(|k| cx.fm.methods_named(&k.this_type(), &c.method.name).iter().filter(|m| m.method.accepts_arity(nargs)).count()).unwrap_or(2),
        };
        if rivals != 1 {
            return;
        }
        let Ty::Class { name: got, args: gargs } = &c.method.ret else { return };
        if !gargs.is_empty() || got == want {
            return;
        }
        // both hierarchies fully known: an unknown supertype could connect them
        let known = |n: &str| cx.fm.class(n).is_some() && cx.fm.hierarchy_complete(n) && !cx.fm.index.is_duplicated(n);
        if !known(got) || !known(want) {
            return;
        }
        if cx.fm.is_assignable(&c.method.ret, &target).is_no() {
            cx.report(value, format!("`{}.{}()` returns `{}`, which cannot be assigned to `{}`", simple_name_of(&c.owner), c.method.name, simple_name_of(got), simple_name_of(want)));
        }
    }
}

/// Every class the type mentions (type arguments included) is known.
fn fully_known(cx: &Ctx<'_, '_>, t: &Ty) -> bool {
    match t {
        Ty::Class { name, args } => cx.fm.class(name).is_some() && args.iter().all(|a| fully_known(cx, a)),
        Ty::Array(e) => fully_known(cx, e),
        Ty::Wildcard(Some(b)) => match &**b {
            WildcardBound::Extends(t) | WildcardBound::Super(t) => fully_known(cx, t),
        },
        _ => true,
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
            if m.method.is_varargs() && i + 1 >= np {
                let last = &m.method.params[np - 1].ty;
                let Ty::Array(e) = last else { return accepts(cx, at, last) };
                // one argument in the varargs position: the array itself
                // (`of(byte[][])`) or one element of it (`of(byte[])` for
                // `of(byte[]...)`, `int[]` for `Object...`)
                return (args.len() == np && accepts(cx, at, last)) || accepts(cx, at, e);
            }
            accepts(cx, at, &m.method.params[i].ty)
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

/// The argument type may be passed for the parameter type: not provably wrong.
fn accepts(cx: &Ctx<'_, '_>, at: &Ty, pt: &Ty) -> bool {
    if pt.has_unknown() || pt.mentions_type_var() {
        return true;
    }
    // a type whose class is not on the classpath, or the same simple
    // name from another package (javax vs jakarta when cached jars do
    // not match the project's versions), proves nothing
    if !fully_known(cx, pt) || !fully_known(cx, at) {
        return true;
    }
    // skew is only possible for jar classes, not between project classes
    // (arrays of them too, with the same number of dimensions)
    let (mut ab, mut pb) = (at, pt);
    while let (Ty::Array(a), Ty::Array(p)) = (ab, pb) {
        (ab, pb) = (&**a, &**p);
    }
    if let (Some(a), Some(p)) = (ab.class_name(), pb.class_name()) {
        let from_jar = |n: &str| cx.fm.class(n).is_some_and(|c| !c.is_source());
        if a != p && simple_name_of(a) == simple_name_of(p) && (from_jar(a) || from_jar(p)) {
            return true;
        }
    }
    // compile-time narrowing of int constants (byte b = 1 style) is not applied to calls,
    // but keep constants permissive anyway
    !cx.fm.is_assignable(at, pt).is_no()
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

declare_rule!(OverrideWithoutSuper {
    id: "override-matches-nothing",
    name: "@Override method does not override anything",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["javac:method.does.not.override.superclass", "intellij:MethodDoesntOverride"],
    kinds: ["method_declaration"],
    description: "The method is annotated @Override but no supertype declares a method with this name and parameter types, so the code does not compile. Usually the supertype method was renamed or its parameters changed.",
});

impl Rule for OverrideWithoutSuper {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !crate::util::has_annotation_named(n, cx.src, "Override") {
            return;
        }
        let Some(m) = cx.fm.method_of_decl(n) else { return };
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        if closed_class(cx, &cls.name).is_none() || m.params.iter().any(|p| p.ty.has_unknown()) {
            return;
        }
        // explicit record accessors may carry @Override
        if cls.is_record() && m.params.is_empty() && cls.fields.iter().any(|f| f.flags.has(Flags::RECORD_COMPONENT) && f.name == m.name) {
            return;
        }
        // Object methods are always overridable
        let mut found = matches!((&*m.name, m.params.len()), ("equals", 1) | ("hashCode", 0) | ("toString", 0) | ("clone", 0) | ("finalize", 0));
        for st in cx.fm.all_supertypes(&cls.this_type()).into_iter().skip(1) {
            if found {
                break;
            }
            let Some(sn) = st.class_name() else { return };
            let Some(sc) = cx.fm.class(sn) else { return };
            let map = grounds_index::hierarchy::substitution(&sc, st.args());
            found = sc.methods.iter().any(|x| {
                x.name == m.name
                    && !x.flags.is_static()
                    && !x.flags.is_private()
                    && x.params.len() == m.params.len()
                    && (!grounds_index::hierarchy::is_subsignature(&m, x, &sc, st.args()).is_no()
                        || x.params.iter().zip(&m.params).all(|(a, b)| {
                        let at = grounds_index::hierarchy::apply_subst(&a.ty, &map);
                        // unknown classes and same-named types from another
                        // package (javax vs jakarta when the cached jars do not
                        // match the project) may still be the same parameter
                        let skew = match (at.erasure().class_name(), b.ty.erasure().class_name()) {
                            (Some(x), Some(y)) => {
                                let from_jar = |n: &str| cx.fm.class(n).is_some_and(|c| !c.is_source());
                                x != y && simple_name_of(x) == simple_name_of(y) && (from_jar(x) || from_jar(y))
                            }
                            _ => false,
                        };
                        skew || !fully_known(cx, &at) || !fully_known(cx, &b.ty)
                    }))
            });
        }
        if !found && cls.is_interface() {
            found = false;
        }
        if !found {
            if let Some(nm) = n.field("name") {
                cx.report(nm, format!("`{}` is marked @Override but overrides no supertype method", m.signature_string()));
            }
        }
    }
}

declare_rule!(UnimplementedAbstractMethod {
    id: "unimplemented-abstract-method",
    name: "Concrete class does not implement an abstract method",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["javac:does.not.override.abstract", "intellij:ClassNotImplementingAbstractMethods"],
    kinds: ["class_declaration", "enum_declaration", "record_declaration"],
    description: "A non-abstract class must implement every abstract method it inherits; otherwise it does not compile. Typically an interface method was added or changed while an implementation was not updated.",
});

impl Rule for UnimplementedAbstractMethod {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.has_modifier("abstract") {
            return;
        }
        let Some(cls) = cx.fm.class_of_decl(n) else { return };
        if cls.is_abstract() || closed_class(cx, &cls.name).is_none() {
            return;
        }
        // enum constants with bodies implement abstract (or interface)
        // methods themselves
        let constant_bodies = n.field("body").is_some_and(|b| b.kids().iter().any(|k| k.kind() == "enum_constant" && k.kid_of_kind("class_body").is_some()));
        if cls.is_enum() && (constant_bodies || cx.text(n).contains("abstract ")) {
            return;
        }
        let mut missing: Vec<String> = Vec::new();
        for m in cx.fm.all_methods(&cls.this_type()) {
            if !m.method.flags.is_abstract() || m.method.flags.is_static() {
                continue;
            }
            // Object's methods (re-declared abstract by Record, Comparator...) are always implemented
            if matches!((&*m.method.name, m.method.params.len()), ("equals", 1) | ("hashCode", 0) | ("toString", 0)) {
                continue;
            }
            if m.method.params.iter().any(|p| p.ty.has_unknown()) {
                return;
            }
            // implemented by this class, a superclass, or an interface default method?
            let implemented = implements(cx, &cls, &m.method.name, m.method.params.len());
            if !implemented {
                missing.push(format!("`{}.{}`", simple_name_of(&m.owner), m.method.signature_string()));
            }
        }
        if !missing.is_empty() {
            if let Some(nm) = n.field("name") {
                let more = if missing.len() > 3 { format!(" and {} more", missing.len() - 3) } else { String::new() };
                cx.report(nm, format!("`{}` does not implement {}{more}", cls.simple_name(), missing.iter().take(3).cloned().collect::<Vec<_>>().join(", ")));
            }
        }
    }
}

fn implements<'t>(cx: &Ctx<'_, 't>, cls: &ClassInfo, name: &str, arity: usize) -> bool {
    let concrete = |c: &ClassInfo| c.methods.iter().any(|x| &*x.name == name && x.params.len() == arity && !x.flags.is_abstract() && !x.flags.is_static());
    // superclass chain
    let mut cur = Some(std::sync::Arc::new(cls.clone()));
    let mut depth = 0;
    while let Some(c) = cur {
        if concrete(&c) {
            return true;
        }
        depth += 1;
        if depth > 30 {
            return true;
        }
        cur = c.superclass.as_ref().and_then(|s| s.class_name()).and_then(|s| cx.fm.class(s));
    }
    // default methods anywhere in the interface closure
    cx.fm.all_supertypes(&cls.this_type()).iter().filter_map(|t| t.class_name()).filter_map(|n| cx.fm.class(n)).any(|c| c.is_interface() && concrete(&c))
}
