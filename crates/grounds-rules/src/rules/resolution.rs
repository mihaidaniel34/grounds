//! "Did I break something?" checks: references to methods, constructors and
//! fields that do not exist on project classes. Each finding claims an
//! absence, which only the kernel (`grounds_semantic::closed`) proves: it is
//! asked for a proof that the class's whole hierarchy is visible, the
//! question is asked of that proof, and the finding carries the claim it
//! answers with. Library gaps never produce noise. These are the checks
//! that catch an edit that renamed or re-shaped an API while callers
//! elsewhere still use the old shape. What a rule here decides by itself is
//! which class a reference is on and how the message reads.

use crate::framework::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_semantic::closed::InScope;
use grounds_semantic::Symbol;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(UnresolvedMethod), Box::new(UnresolvedConstructor), Box::new(UnresolvedField), Box::new(UnresolvedStaticImport), Box::new(MissingJdkImport), Box::new(IncompatibleAssignment), Box::new(OverrideWithoutSuper), Box::new(UnimplementedAbstractMethod)]
}

/// The class a reference's qualifier `o` is on: the type it names
/// (`Type.m()`, with `true`), or the class of its value. None for a package
/// or a value whose type is not a class.
fn receiver_class<'t>(cx: &Ctx<'_, 't>, o: Node<'t>) -> Option<(Name, bool)> {
    match cx.fm.symbol(o) {
        Some(Symbol::Type(t)) => Some((t, true)),
        Some(Symbol::Package(_)) => None,
        _ => cx.type_of(o).class_name().map(|t| (Name::from(t), false)),
    }
}

fn signatures(ms: &[MethodRef]) -> String {
    let mut v: Vec<String> = ms.iter().take(4).map(|m| m.method.signature_string()).collect();
    v.dedup();
    if ms.len() > 4 {
        v.push("...".into());
    }
    v.join(", ")
}

/// A method of `c` the caller may have meant by `name`, called with `nargs`
/// arguments (`None` for a method reference).
fn similar_names(c: &ClassInfo, lk: &dyn ClassLookup, name: &str, nargs: Option<usize>) -> Option<String> {
    let all = lk.all_methods(&c.this_type());
    closest_name(name, all.iter().filter(|m| &*m.method.name != "<init>").map(|m| (&*m.method.name, nargs.is_none_or(|k| m.method.accepts_arity(k))))).map(str::to_string)
}

/// The best candidate `(name, takes the call's arguments)` for `name`: one
/// that starts with it (`fetch` -> `fetchByName`), then one containing it or
/// that it starts with, then one within a small edit distance; within each,
/// one taking the call's arguments, then the closest.
fn closest_name<'a>(name: &str, cands: impl Iterator<Item = (&'a str, bool)>) -> Option<&'a str> {
    let lname = name.to_ascii_lowercase();
    cands
        .filter(|(n, _)| *n != name)
        .filter_map(|(n, fits)| {
            let ln = n.to_ascii_lowercase();
            let d = edit_distance(&lname, &ln);
            // `get` or `is` is a prefix of half the class
            let long = lname.len().min(ln.len()) >= 4;
            let tier = if long && ln.starts_with(&lname) {
                0
            } else if long && (ln.contains(&lname) || lname.starts_with(&ln)) {
                1
            } else if d <= 2.max(name.len() / 4) {
                2
            } else {
                return None;
            };
            Some(((tier, !fits, d, n), n))
        })
        .min_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, n)| n)
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
    maturity: Preview,
    tier: Edit,
    absence: true,
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
        let at = n.field("name").unwrap_or(n);
        let Some(o) = n.field("object") else {
            // implicit receiver: the enclosing classes, and the static imports
            let Ok(scope) = cx.fm.closed_scope(n, cx.jars()) else { return };
            let ci = scope.innermost().class().clone();
            match scope.method_missing(n, name, nargs) {
                Some(InScope::Private(p)) => cx.report_claim(at, format!("`{}` is private in `{}`", p.method.method.signature_string(), simple_name_of(&p.method.owner)), p.claim),
                Some(InScope::NotDefined(claim)) => {
                    let hint = similar_names(&ci, cx.fm, name, Some(nargs)).map(|s| format!("; did you mean `{s}`?")).unwrap_or_default();
                    cx.report_claim(at, format!("Method `{name}` is not defined in `{}` or its enclosing classes{hint}", ci.simple_name()), claim);
                }
                None => {}
            }
            return;
        };
        // `super.m()` and `Iface.super.m()`
        if o.kind() == "super" || (o.kind() == "field_access" && o.kids().iter().any(|k| k.kind() == "super")) || cx.text(o).ends_with(".super") {
            return;
        }
        let Some((t, _)) = receiver_class(cx, o) else { return };
        let Ok(closed) = cx.fm.closed(&t, cx.jars()) else { return };
        let cls = closed.class().clone();
        let ms = closed.methods(name);
        if let Some(claim) = ms.none() {
            let hint = similar_names(&cls, cx.fm, name, Some(nargs)).map(|s| format!("; did you mean `{s}`?")).unwrap_or_default();
            cx.report_claim(at, format!("`{}` has no method `{name}`{hint}", cls.simple_name()), claim);
        } else if let Some(claim) = ms.none_taking(nargs) {
            cx.report_claim(at, format!("No `{name}` in `{}` takes {nargs} argument(s); available: {}", cls.simple_name(), signatures(ms.all())), claim);
        } else if let Some(m) = ms.none_accepting(n) {
            let shown = m.args.iter().map(|t| t.display_simple()).collect::<Vec<_>>().join(", ");
            cx.report_claim(at, format!("No `{name}` in `{}` accepts ({shown}); available: {}", cls.simple_name(), signatures(ms.all())), m.claim);
        } else if let Some(p) = ms.private_elsewhere(n, Some(nargs)) {
            cx.report_claim(at, format!("`{}` is private in `{}`", p.method.method.signature_string(), simple_name_of(&p.method.owner)), p.claim);
        } else if let Some(claim) = ms.instance_on_type(o, nargs) {
            // calling an instance method through the type name
            cx.report_claim(at, format!("`{}.{name}()` is an instance method and cannot be called on the type", cls.simple_name()), claim);
        }
    }
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
    let Some((t, _)) = receiver_class(cx, recv) else { return };
    let Ok(closed) = cx.fm.closed(&t, cx.jars()) else { return };
    let cls = closed.class().clone();
    let mname = cx.text(name);
    let ms = closed.methods(mname);
    if let Some(claim) = ms.none() {
        let hint = similar_names(&cls, cx.fm, mname, None).map(|s| format!("; did you mean `{s}`?")).unwrap_or_default();
        cx.report_claim(name, format!("`{}` has no method `{mname}`{hint}", cls.simple_name()), claim);
    } else if let Some(p) = ms.private_elsewhere(n, None) {
        cx.report_claim(name, format!("`{}` is private in `{}`", p.method.method.signature_string(), simple_name_of(&p.method.owner)), p.claim);
    }
}

declare_rule!(UnresolvedStaticImport {
    id: "unresolved-static-import",
    name: "Static import of a member that does not exist",
    category: Bug,
    severity: Error,
    maturity: Preview,
    tier: Edit,
    absence: true,
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
        let Ok(closed) = cx.fm.closed_import_owner(&segs, cx.jars()) else { return };
        let Some(claim) = closed.no_static_member(segs[segs.len() - 1]) else { return };
        let target = path.field("name").unwrap_or(path);
        // the claim is the message
        cx.report_claim(target, claim.as_str().to_string(), claim);
    }
}

declare_rule!(MissingJdkImport {
    id: "missing-jdk-import",
    name: "Import of a JDK class the Java release does not have",
    category: Bug,
    severity: Warning,
    maturity: Preview,
    tier: Edit,
    absence: true,
    sources: ["javac:doesnt.exist", "javac:cant.resolve.location"],
    kinds: ["import_declaration"],
    description: "The imported `java.*` class or package is not public API in the project's Java release, so the code does not compile for that release: a class added in a later release (`java.util.SequencedCollection` is Java 21), one removed since, or one the JDK does not export. Checked only in modules whose build states the release exactly (`--release`, `maven.compiler.release`, `options.release`, a toolchain) and whose code does not contradict it, against that release's API from the JDK.",
});

impl Rule for MissingJdkImport {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Ok(release) = cx.fm.java_release() else { return };
        let Some(claim) = release.lacks_import(n) else { return };
        let Some(path) = n.kids().into_iter().find(|k| matches!(k.kind(), "scoped_identifier" | "identifier")) else { return };
        // the claim is the message, qualified
        cx.report_claim(path, claim.as_str().to_string(), claim);
    }
}

declare_rule!(IncompatibleAssignment {
    id: "incompatible-assignment",
    name: "Call result assigned to an incompatible type",
    category: Bug,
    severity: Error,
    maturity: Preview,
    tier: Edit,
    absence: true,
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
        let Some(claim) = cx.fm.unassignable(value, &target) else { return };
        // the claim is the message
        cx.report_claim(value, claim.as_str().to_string(), claim);
    }
}

declare_rule!(UnresolvedConstructor {
    id: "unresolved-constructor",
    name: "No constructor matches the arguments",
    category: Bug,
    severity: Error,
    maturity: Preview,
    tier: Edit,
    absence: true,
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
        // `outer.new Inner()` names a member type of the qualifier's type
        let ty = cx.fm.created_class(n);
        let Some(tn) = ty.class_name() else { return };
        let Ok(closed) = cx.fm.closed(tn, cx.jars()) else { return };
        let Some(none) = closed.no_constructor(n) else { return };
        let cls = closed.class();
        let avail: Vec<String> = none.constructors.iter().take(4).map(|c| format!("{}({})", cls.simple_name(), c.params.iter().map(|p| p.ty.display_simple()).collect::<Vec<_>>().join(", "))).collect();
        cx.report_claim(t, format!("No constructor of `{}` takes {} argument(s); available: {}", cls.simple_name(), none.nargs, avail.join(", ")), none.claim);
    }
}

declare_rule!(UnresolvedField {
    id: "unresolved-field",
    name: "Access to a field that does not exist",
    category: Bug,
    severity: Error,
    maturity: Preview,
    tier: Edit,
    absence: true,
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
        let Some((t, _)) = receiver_class(cx, o) else { return };
        let Ok(closed) = cx.fm.closed(&t, cx.jars()) else { return };
        let Some(claim) = closed.no_field(n) else { return };
        cx.report_claim(f, format!("`{}` has no field `{}`", closed.class().simple_name(), cx.text(f)), claim);
    }
}

declare_rule!(OverrideWithoutSuper {
    id: "override-matches-nothing",
    name: "@Override method does not override anything",
    category: Bug,
    severity: Error,
    maturity: Preview,
    tier: Edit,
    absence: true,
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
        let Ok(closed) = cx.fm.closed(&cls.name, cx.jars()) else { return };
        let Some(claim) = closed.overrides_nothing(n) else { return };
        if let Some(nm) = n.field("name") {
            cx.report_claim(nm, format!("`{}` is marked @Override but overrides no supertype method", m.signature_string()), claim);
        }
    }
}

declare_rule!(UnimplementedAbstractMethod {
    id: "unimplemented-abstract-method",
    name: "Concrete class does not implement an abstract method",
    category: Bug,
    severity: Error,
    maturity: Preview,
    tier: Edit,
    absence: true,
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
        if cls.is_abstract() {
            return;
        }
        let Ok(closed) = cx.fm.closed(&cls.name, cx.jars()) else { return };
        let Some(u) = closed.unimplemented(n) else { return };
        let Some(nm) = n.field("name") else { return };
        let missing: Vec<String> = u.missing.iter().map(|m| format!("`{}.{}`", simple_name_of(&m.owner), m.method.signature_string())).collect();
        let more = if missing.len() > 3 { format!(" and {} more", missing.len() - 3) } else { String::new() };
        // the claim is every missing method, whatever the message shows
        cx.report_claim(nm, format!("`{}` does not implement {}{more}", cls.simple_name(), missing.iter().take(3).cloned().collect::<Vec<_>>().join(", ")), u.claim);
    }
}


#[cfg(test)]
mod tests {
    use super::closest_name;

    fn pick<'a>(name: &str, cands: &[(&'a str, bool)]) -> Option<&'a str> {
        closest_name(name, cands.iter().copied())
    }

    #[test]
    fn renamed_method_suggestions() {
        // a rename that extends the name beats a short edit distance
        assert_eq!(pick("fetch", &[("watch", true), ("fetchByName", true), ("close", true)]), Some("fetchByName"));
        assert_eq!(pick("fetchByName", &[("watch", true), ("fetch", true)]), Some("fetch"));
        assert_eq!(pick("name", &[("getName", true), ("same", true)]), Some("getName"));
        // the same number of arguments breaks a tie
        assert_eq!(pick("fetch", &[("fetchAll", false), ("fetchOne", true)]), Some("fetchOne"));
        assert_eq!(pick("fetch", &[("fetchOne", false), ("fetchAll", true)]), Some("fetchAll"));
        assert_eq!(pick("lod", &[("lad", false), ("log", true)]), Some("log"));
        // the edit-distance fallback, case-insensitive
        assert_eq!(pick("findByID", &[("findById", true), ("save", true)]), Some("findById"));
        assert_eq!(pick("sav", &[("save", true)]), Some("save"));
        // short names are no prefix match
        assert_eq!(pick("is", &[("isEmptyCollection", true)]), None);
        assert_eq!(pick("getValueOfField", &[("get", true)]), None);
        assert_eq!(pick("fetch", &[("unrelated", true), ("fetch", true)]), None);
    }
}
