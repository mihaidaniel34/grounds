//! Unused declarations: locals, private members, parameters, imports.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_semantic::{LocalKind, Symbol};
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(UnusedLocal),
        Box::new(UnusedPrivateField),
        Box::new(UnusedPrivateMethod),
        Box::new(UnusedParameter),
        Box::new(UnusedImport),
        Box::new(UnusedPrivateClass),
        Box::new(UnusedPatternVariable),
    ]
}

/// Is this identifier use a pure write (target of `=`)?
fn is_write_only(u: Node) -> bool {
    match u.parent_unparen() {
        Some(p) if p.kind() == "assignment_expression" => p.field("left").map(|l| l.unparen()) == Some(u),
        Some(p) if p.kind() == "update_expression" => p.parent().is_some_and(|pp| pp.kind() == "expression_statement" || pp.kind() == "for_statement"),
        _ => false,
    }
}

declare_rule!(UnusedLocal {
    id: "unused-local-variable",
    name: "Unused local variable",
    category: BadPractice,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S1481", "sonar:S1854", "pmd:UnusedLocalVariable", "intellij:UnusedDeclaration"],
    kinds: ["<file>"],
    description: "A local variable that is never read is dead code. After an edit it often means a value was computed but the code using it was forgotten or still uses an older variable.",
});

impl Rule for UnusedLocal {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        let b = &cx.fm.binder;
        let mut out = Vec::new();
        for (i, d) in b.decls.iter().enumerate() {
            if d.kind != LocalKind::Local {
                continue;
            }
            if d.name.starts_with("unused") || d.name.starts_with("ignored") || d.name == "_" || d.name.starts_with('_') {
                continue;
            }
            // annotated locals (e.g. @SuppressWarnings) are deliberate
            if d.decl_node.parent().is_some_and(|p| p.modifiers().is_some_and(|m| m.kids().iter().any(|k| matches!(k.kind(), "annotation" | "marker_annotation")))) {
                continue;
            }
            let uses = &b.uses[i];
            let read = uses.iter().any(|u| !is_write_only(*u));
            if !read {
                let msg = if uses.is_empty() { format!("Local variable `{}` is never used", d.name) } else { format!("Local variable `{}` is assigned but never read", d.name) };
                out.push((d.name_node, msg));
            }
        }
        for (n, m) in out {
            cx.report(n, m);
        }
    }
}

declare_rule!(UnusedPatternVariable {
    id: "unused-pattern-variable",
    name: "Unused pattern binding",
    category: Style,
    severity: Info,
    sources: ["sonar:S1481", "intellij:UnusedDeclaration"],
    kinds: ["<file>"],
    description: "`x instanceof Foo f` with `f` unused can be written as `x instanceof Foo` (or use `_` on Java 22+).",
});

impl Rule for UnusedPatternVariable {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        let b = &cx.fm.binder;
        let mut out = Vec::new();
        for (i, d) in b.decls.iter().enumerate() {
            if d.kind == LocalKind::Pattern && b.uses[i].is_empty() && d.decl_node.kind() == "instanceof_expression" && !d.name.starts_with('_') {
                out.push(d.name_node);
            }
        }
        for n in out {
            let name = cx.text(n);
            cx.report(n, format!("Pattern variable `{name}` is never used"));
        }
    }
}

fn top_level_decl<'t>(n: Node<'t>) -> Option<Node<'t>> {
    let mut last = None;
    for a in std::iter::once(n).chain(n.ancestors()) {
        if grounds_syntax::is_type_declaration(a.kind()) {
            last = Some(a);
        }
    }
    last
}

const REFLECTIVE_CLASS_ANNOTATIONS: &[&str] = &[
    "Entity", "Embeddable", "MappedSuperclass", "Document", "Table", "Data", "Value", "Getter", "Setter", "JsonAutoDetect", "Introspected", "Serdeable", "ConfigurationProperties", "XmlRootElement", "XmlAccessorType", "Builder", "AllArgsConstructor", "RequiredArgsConstructor", "Jacksonized", "EachProperty", "FieldNameConstants", "EqualsAndHashCode", "ToString",
];

declare_rule!(UnusedPrivateField {
    id: "unused-private-field",
    name: "Unused private field",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S1068", "spotbugs:URF_UNREAD_FIELD", "spotbugs:UUF_UNUSED_FIELD", "pmd:UnusedPrivateField"],
    kinds: ["field_declaration"],
    description: "A private field that is never read is dead state; if it is written, the writes are wasted work and may hide a missing use.",
});

impl Rule for UnusedPrivateField {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !n.has_modifier("private") {
            return;
        }
        // any annotation (injection, mapping, @Getter...) means framework access
        if n.modifiers().is_some_and(|m| m.kids().iter().any(|k| matches!(k.kind(), "annotation" | "marker_annotation"))) {
            return;
        }
        let Some(decl) = owning_type_decl(n) else { return };
        if decl.kind() == "record_declaration" {
            return;
        }
        let anns = annotation_names(decl, cx.src);
        if anns.iter().any(|a| REFLECTIVE_CLASS_ANNOTATIONS.contains(a)) {
            return;
        }
        let Some(cls) = cx.fm.class_of_decl(decl) else { return };
        let Some(top) = top_level_decl(n) else { return };
        let top_text = cx.text(top);
        for d in n.fields("declarator") {
            let Some(name_node) = d.field("name") else { continue };
            let name = name_node.text(cx.src);
            if matches!(name, "serialVersionUID" | "serialPersistentFields") {
                continue;
            }
            // Lombok-generated accessors use the field
            if cls.methods.iter().any(|m| m.generated_by.is_some() && m.name.to_ascii_lowercase().ends_with(&name.to_ascii_lowercase())) {
                continue;
            }
            let top_range = top.byte_range();
            let occs: Vec<Node<'t>> = cx.fm.occurrences(name).iter().copied().filter(|o| top_range.contains(&o.start_byte()) && *o != name_node && o.kind() == "identifier").collect();
            if occs.is_empty() && !top_text.contains(&format!("\"{name}\"")) {
                cx.report(name_node, format!("Private field `{name}` is never used"));
                continue;
            }
            // resolve each occurrence
            let mut read = false;
            let mut any = false;
            for occ in occs {
                let is_field = match cx.fm.symbol(occ) {
                    Some(Symbol::Field(f)) => f.owner == cls.name && &*f.field.name == name,
                    _ => {
                        // `this.name` / `other.name`
                        occ.parent().is_some_and(|p| p.kind() == "field_access" && p.field("field") == Some(occ) && cx.fm.field_ref(p).is_some_and(|f| f.owner == cls.name && &*f.field.name == name))
                    }
                };
                if !is_field {
                    continue;
                }
                any = true;
                let target = if occ.parent().is_some_and(|p| p.kind() == "field_access" && p.field("field") == Some(occ)) { occ.parent().unwrap() } else { occ };
                if !is_write_only(target) {
                    read = true;
                    break;
                }
            }
            if !read {
                // method references / strings (reflection) keep it alive
                if top_text.contains(&format!("\"{name}\"")) {
                    continue;
                }
                let msg = if any { format!("Private field `{name}` is assigned but never read") } else { format!("Private field `{name}` is never used") };
                cx.report(name_node, msg);
            }
        }
    }
}

const SERIALIZATION_METHODS: &[&str] = &["writeObject", "readObject", "readObjectNoData", "readResolve", "writeReplace"];

declare_rule!(UnusedPrivateMethod {
    id: "unused-private-method",
    name: "Unused private method",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S1144", "spotbugs:UPM_UNCALLED_PRIVATE_METHOD", "pmd:UnusedPrivateMethod"],
    kinds: ["method_declaration"],
    description: "A private method that is never called is dead code (or the caller was removed in a refactoring that forgot it).",
});

impl Rule for UnusedPrivateMethod {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !n.has_modifier("private") {
            return;
        }
        let name = method_name(n, cx.src);
        if SERIALIZATION_METHODS.contains(&name) {
            return;
        }
        if n.modifiers().is_some_and(|m| m.kids().iter().any(|k| matches!(k.kind(), "annotation" | "marker_annotation") && !cx.text(*k).contains("SuppressWarnings"))) {
            return;
        }
        let Some(top) = top_level_decl(n) else { return };
        let top_text = cx.text(top);
        // referenced by name in strings (@MethodSource, reflection) or method references
        if top_text.contains(&format!("\"{name}\"")) || top_text.contains(&format!("::{name}")) || top_text.contains(&format!("#{name}")) {
            return;
        }
        // a bare @MethodSource uses the provider named like the test method
        if let Some(decl) = owning_type_decl(n) {
            if methods_of(decl).iter().any(|m| method_name(*m, cx.src) == name && *m != n && m.modifiers().is_some_and(|ms| cx.text(ms).contains("@MethodSource") && !cx.text(ms).contains("@MethodSource("))) {
                return;
            }
        }
        let Some(name_node) = n.field("name") else { return };
        let decl_range = (name_node.start_byte() as u32, name_node.end_byte() as u32);
        let mut called = false;
        let top_range = top.byte_range();
        let calls: Vec<Node<'t>> = cx
            .fm
            .occurrences(name)
            .iter()
            .filter(|o| top_range.contains(&o.start_byte()))
            .filter_map(|o| o.parent().filter(|p| p.kind() == "method_invocation" && p.field("name") == Some(*o)))
            .collect();
        {
            for occ in calls {
                match cx.fm.select_method(occ) {
                    Some(c) => {
                        if c.method.range == Some(decl_range) || c.ambiguous {
                            called = true;
                            break;
                        }
                    }
                    // unresolved calls with the same name and arity might be ours
                    None => {
                        if cx.fm.args_of(occ).len() == param_count(n) || n.field("parameters").is_some_and(|p| p.kid_of_kind("spread_parameter").is_some()) {
                            called = true;
                            break;
                        }
                    }
                }
            }
        }
        if !called {
            cx.report(name_node, format!("Private method `{name}` is never called"));
        }
    }
}

declare_rule!(UnusedParameter {
    id: "unused-parameter",
    name: "Unused method parameter",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S1172", "pmd:UnusedFormalParameter", "intellij:UnusedParameters"],
    kinds: ["method_declaration", "constructor_declaration"],
    description: "A parameter that a private or static method never uses misleads callers into computing a value for nothing. Remove it.",
});

impl Rule for UnusedParameter {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let private_like = n.has_modifier("private") || (n.kind() == "method_declaration" && n.has_modifier("static") && !n.has_modifier("public") && !n.has_modifier("protected"));
        if !private_like || is_main_method(n, cx.src) || SERIALIZATION_METHODS.contains(&method_name(n, cx.src)) {
            return;
        }
        if !annotation_names(n, cx.src).is_empty() {
            return;
        }
        let Some(body) = n.field("body") else { return };
        let stmts = body.kids();
        // stubs that only throw / return a constant
        if stmts.is_empty() || (stmts.len() == 1 && matches!(stmts[0].kind(), "throw_statement")) {
            return;
        }
        // used as a method reference: signature fixed by a functional interface
        let name = method_name(n, cx.src);
        if top_level_decl(n).is_some_and(|t| cx.text(t).contains(&format!("::{name}"))) {
            return;
        }
        let Some(params) = n.field("parameters") else { return };
        for p in params.kids() {
            let Some(pn) = p.field("name").or_else(|| p.kid_of_kind("variable_declarator").and_then(|d| d.field("name"))) else { continue };
            let pname = pn.text(cx.src);
            if pname == "_" || pname.starts_with("unused") || pname.starts_with("ignored") {
                continue;
            }
            if p.modifiers().is_some_and(|m| m.kids().iter().any(|k| matches!(k.kind(), "annotation" | "marker_annotation"))) {
                continue;
            }
            let Some(i) = cx.fm.binder.decl_index(pn) else { continue };
            if cx.fm.binder.uses[i].is_empty() {
                cx.report(pn, format!("Parameter `{pname}` is never used"));
            }
        }
    }
}

declare_rule!(UnusedImport {
    id: "unused-import",
    name: "Unused or redundant import",
    category: Style,
    severity: Info,
    tier: Edit,
    sources: ["sonar:S1128", "pmd:UnnecessaryImport", "intellij:UnusedImport"],
    kinds: ["<file>"],
    description: "Imports that are never used, duplicated, from `java.lang` or from the file's own package clutter the file and can cause ambiguity.",
});

impl Rule for UnusedImport {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        let root = cx.fm.root();
        let pkg = cx.fm.package().to_string();
        // names used in code (not in imports / package)
        let mut used: rustc_hash::FxHashSet<&str> = rustc_hash::FxHashSet::default();
        let mut comments = String::new();
        for k in root.kids() {
            if matches!(k.kind(), "import_declaration" | "package_declaration") {
                continue;
            }
            grounds_syntax::walk(k, |n| {
                if matches!(n.kind(), "identifier" | "type_identifier") {
                    used.insert(n.text(cx.src));
                }
                grounds_syntax::WalkControl::Continue
            });
        }
        // javadoc references
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            let mut c = n.walk();
            for k in n.children(&mut c) {
                if k.kind() == "block_comment" || k.kind() == "line_comment" {
                    comments.push_str(cx.text(k));
                    comments.push('\n');
                } else if k.child_count() > 0 {
                    stack.push(k);
                }
            }
        }
        let doc_refs = javadoc_references(&comments);
        let mut seen: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
        let mut out = Vec::new();
        for imp in root.kids().into_iter().filter(|k| k.kind() == "import_declaration") {
            let mut c = imp.walk();
            let is_static = imp.children(&mut c).any(|k| !k.is_named() && k.kind() == "static");
            let on_demand = imp.kid_of_kind("asterisk").is_some();
            // `import module java.base;`: not a type import, its use cannot be seen by name
            let mut c2 = imp.walk();
            if imp.children(&mut c2).any(|k| !k.is_named() && k.kind() == "module") {
                continue;
            }
            let Some(path_node) = imp.kids().into_iter().find(|k| matches!(k.kind(), "identifier" | "scoped_identifier")) else { continue };
            let path = path_node.text(cx.src).chars().filter(|c| !c.is_whitespace()).collect::<String>();
            let key = format!("{is_static}{on_demand}{path}");
            if !seen.insert(key) {
                out.push((imp, format!("Duplicate import `{path}`")));
                continue;
            }
            if on_demand {
                continue;
            }
            let (prefix, last) = match path.rsplit_once('.') {
                Some(x) => x,
                None => continue,
            };
            if !is_static {
                if prefix == "java.lang" {
                    out.push((imp, format!("`{path}` is imported automatically")));
                    continue;
                }
                if prefix == pkg && cx.fm.index.class_exists(&path) {
                    out.push((imp, format!("`{path}` is in the same package")));
                    continue;
                }
            }
            if used.contains(last) {
                continue;
            }
            if doc_refs.contains(last) {
                continue;
            }
            out.push((imp, format!("Unused import `{path}`")));
        }
        for (imp, msg) in out {
            let end = cx.src[imp.end_byte()..].find('\n').map(|i| imp.end_byte() + i + 1).unwrap_or(imp.end_byte());
            cx.report_fix(imp, msg, Fix::delete("Remove import", imp.start_byte() as u32, end as u32));
        }
    }
}

/// Identifiers named by Javadoc references (`{@link Foo#bar(Baz, int)}`,
/// `@see a.b.Foo`, `@throws FooException`...), also when the reference is
/// wrapped onto the next comment line.
fn javadoc_references(comments: &str) -> rustc_hash::FxHashSet<&str> {
    // longer tags first: `{@link` is a prefix of `{@linkplain`
    const TAGS: &[&str] = &["{@linkplain", "{@link", "{@value", "@see", "@throws", "@exception"];
    let b = comments.as_bytes();
    let mut out = rustc_hash::FxHashSet::default();
    let mut i = 0;
    while let Some(at) = comments[i..].find('@').map(|k| i + k) {
        i = at + 1;
        let start = if at > 0 && b[at - 1] == b'{' { at - 1 } else { at };
        let Some(tag) = TAGS.iter().find(|t| comments[start..].starts_with(**t)) else { continue };
        let mut j = start + tag.len();
        if j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
            continue; // `@seeAlso`, `{@linkx`
        }
        // skip whitespace and the `*` that starts a continued comment line
        while j < b.len() && (b[j].is_ascii_whitespace() || b[j] == b'*') {
            j += 1;
        }
        let ref_start = j;
        let mut depth = 0;
        while j < b.len() {
            match b[j] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b'}' => break,
                c if c.is_ascii_whitespace() && depth <= 0 => break,
                _ => {}
            }
            j += 1;
        }
        for id in comments[ref_start..j].split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')) {
            if !id.is_empty() {
                out.insert(id);
            }
        }
        i = j.max(i);
    }
    out
}

declare_rule!(UnusedPrivateClass {
    id: "unused-private-class",
    name: "Unused private nested class",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S1144"],
    kinds: ["class_declaration", "interface_declaration", "enum_declaration", "record_declaration"],
    description: "A private nested type that is never referenced is dead code.",
});

impl Rule for UnusedPrivateClass {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !n.has_modifier("private") || !annotation_names(n, cx.src).is_empty() {
            return;
        }
        let Some(name) = n.field("name") else { return };
        let simple = name.text(cx.src);
        let Some(top) = top_level_decl(n) else { return };
        let top_range = top.byte_range();
        let refs = cx.fm.occurrences(simple).iter().filter(|d| top_range.contains(&d.start_byte()) && **d != name).count();
        if refs == 0 {
            cx.report(name, format!("Private type `{simple}` is never used"));
        }
    }
}
