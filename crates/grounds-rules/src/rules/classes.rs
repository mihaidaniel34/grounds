//! Class design, inheritance, serialization and naming conventions.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_semantic::Symbol;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(UtilityClassConstructor),
        Box::new(ConstantInterface),
        Box::new(PublicStaticMutableField),
        Box::new(PublicStaticMutableContainer),
        Box::new(StaticWriteFromInstance),
        Box::new(ConstructorCallsOverridable),
        Box::new(InnerClassCouldBeStatic),
        Box::new(FieldHidesSuperField),
        Box::new(MethodNamedLikeClass),
        Box::new(ObjectMethodTypo),
        Box::new(ClassShadowsSupertypeName),
        Box::new(ClassNaming),
        Box::new(MethodNaming),
        Box::new(ConstantNaming),
        Box::new(FieldNaming),
        Box::new(LocalNaming),
        Box::new(PackageNaming),
        Box::new(TypeParamNaming),
        Box::new(MissingSerialVersionUid),
        Box::new(SerialVersionUidModifiers),
        Box::new(NonSerializableField),
        Box::new(SerializationMethodSignature),
        Box::new(CloneWithoutSuper),
        Box::new(CloneableWithoutClone),
        Box::new(CompareToWithoutEquals),
        Box::new(AccessorWrongField),
        Box::new(AbstractClassPublicConstructor),
        Box::new(MutableEnumField),
        Box::new(StaticViaInstance),
        Box::new(EmptyMethod),
        Box::new(MissingOverride),
        Box::new(UselessOverride),
        Box::new(RedundantModifier),
        Box::new(TooManyParameters),
        Box::new(MutableException),
        Box::new(ExceptionNaming),
        Box::new(RecordArrayComponent),
        Box::new(EqualsWithoutTypeCheck),
        Box::new(AbstractClassNoAbstractMethods),
        Box::new(PrivateConstructorNotFinal),
    ]
}

fn class_decls() -> &'static [&'static str] {
    &["class_declaration"]
}

declare_rule!(UtilityClassConstructor {
    id: "utility-class-instantiable",
    name: "Utility class with an implicit public constructor",
    category: Style,
    severity: Info,
    sources: ["sonar:S1118", "pmd:UseUtilityClass", "intellij:UtilityClassWithoutPrivateConstructor"],
    kinds: ["class_declaration"],
    description: "A class with only static members should not be instantiated; add a private constructor (and make it final).",
});

impl Rule for UtilityClassConstructor {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.has_modifier("abstract") || n.field("superclass").is_some() || n.field("interfaces").is_some() {
            return;
        }
        if n.parent().is_some_and(|p| p.kind() != "program") && !n.has_modifier("static") {
            return;
        }
        let ms = members(n);
        let mut static_methods = 0;
        for m in &ms {
            match m.kind() {
                "method_declaration" => {
                    if !m.has_modifier("static") {
                        return;
                    }
                    if is_main_method(*m, cx.src) {
                        return;
                    }
                    static_methods += 1;
                }
                "field_declaration" => {
                    if !m.has_modifier("static") {
                        return;
                    }
                }
                "constructor_declaration" => return,
                "block" => return,
                _ => {}
            }
        }
        if static_methods == 0 {
            return;
        }
        // Lombok @UtilityClass / Spring @SpringBootApplication etc.
        if !annotation_names(n, cx.src).is_empty() {
            return;
        }
        if let Some(name) = n.field("name") {
            cx.report(name, "Utility class has an implicit public constructor; add a private one");
        }
    }
}

declare_rule!(ConstantInterface {
    id: "constant-interface",
    name: "Interface used only to hold constants",
    category: Style,
    severity: Info,
    sources: ["sonar:S1214", "pmd:ConstantsInInterface"],
    kinds: ["interface_declaration"],
    description: "Implementing an interface just to access its constants leaks them into the implementing class's public API. Put constants in a final utility class or enum.",
});

impl Rule for ConstantInterface {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(body) = n.field("body") else { return };
        let kids = body.kids();
        if kids.is_empty() || n.kid_of_kind("extends_interfaces").is_some() {
            return;
        }
        if kids.iter().all(|k| k.kind() == "constant_declaration" || k.kind() == "field_declaration") {
            if let Some(name) = n.field("name") {
                let _ = cx.text(name);
                cx.report(name, "Interface contains only constants; use a final class or enum");
            }
        }
    }
}

fn is_mutable_type<'t>(cx: &Ctx<'_, 't>, ty: &Ty, init: Option<Node<'t>>) -> bool {
    if ty.is_array() {
        // zero-length arrays are immutable
        if let Some(i) = init {
            let t = cx.text(i).replace(' ', "");
            if t.ends_with("[0]") || t == "{}" {
                return false;
            }
        }
        return true;
    }
    if let Some(i) = init {
        let it = cx.type_of(i);
        return matches!(it.class_name(), Some("java.util.ArrayList" | "java.util.HashMap" | "java.util.HashSet" | "java.util.LinkedList" | "java.util.TreeMap" | "java.util.TreeSet" | "java.util.LinkedHashMap" | "java.util.LinkedHashSet" | "java.util.Date"));
    }
    false
}

declare_rule!(PublicStaticMutableField {
    id: "public-static-non-final-field",
    name: "public static field is not final",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S1444", "spotbugs:MS_SHOULD_BE_FINAL", "pmd:MutableStaticState"],
    kinds: ["field_declaration"],
    description: "A public static non-final field is global mutable state: any code can change it at any time, with no synchronization. Make it final (and private if it must change).",
});

impl Rule for PublicStaticMutableField {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.has_modifier("public") && n.has_modifier("static") && !n.has_modifier("final") {
            // fields in interfaces are implicitly final
            if n.parent().is_some_and(|p| p.kind() == "interface_body") {
                return;
            }
            for d in n.fields("declarator") {
                cx.report(d, format!("`{}` is public, static and mutable", d.field("name").map(|x| cx.text(x)).unwrap_or("")));
            }
        }
    }
}

declare_rule!(PublicStaticMutableContainer {
    id: "public-static-mutable-container",
    name: "public static final array or mutable collection",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S2386", "spotbugs:MS_MUTABLE_ARRAY", "spotbugs:MS_MUTABLE_COLLECTION", "spotbugs:MS_MUTABLE_COLLECTION_PKGPROTECT"],
    kinds: ["field_declaration"],
    description: "`final` only fixes the reference: the contents of a public static array or `ArrayList`/`HashMap` can still be changed by anyone. Expose an unmodifiable view or copy.",
});

impl Rule for PublicStaticMutableContainer {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let in_iface = n.parent().is_some_and(|p| p.kind() == "interface_body");
        if !(in_iface || (n.has_modifier("public") && n.has_modifier("static"))) {
            return;
        }
        let Some(t) = n.field("type") else { return };
        let ty = cx.fm.resolve_type_node(t);
        for d in n.fields("declarator") {
            let mut dty = ty.clone();
            if d.field("dimensions").is_some() {
                dty = Ty::Array(Box::new(dty));
            }
            if is_mutable_type(cx, &dty, d.field("value")) {
                cx.report(d, format!("Public static `{}` can be modified by any caller", d.field("name").map(|x| cx.text(x)).unwrap_or("")));
            }
        }
    }
}

declare_rule!(StaticWriteFromInstance {
    id: "static-field-written-from-instance-method",
    name: "Static field written from an instance method",
    category: BadPractice,
    severity: Warning,
    scope: Main,
    sources: ["sonar:S2696", "spotbugs:ST_WRITE_TO_STATIC_FROM_INSTANCE_METHOD"],
    kinds: ["assignment_expression", "update_expression"],
    description: "Assigning a static field from an instance method makes every instance affect all others, and is a race when instances are used from multiple threads.",
});

impl Rule for StaticWriteFromInstance {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let target = if n.kind() == "assignment_expression" { n.field("left") } else { n.first_kid() };
        let Some(t) = target else { return };
        let Some(f) = cx.fm.field_ref(t.unparen()) else { return };
        if !f.field.flags.is_static() || f.field.flags.is_final() {
            return;
        }
        let Some(m) = enclosing_method_decl(n) else { return };
        if m.kind() != "method_declaration" || m.has_modifier("static") || m.has_modifier("synchronized") || in_synchronized(n) {
            return;
        }
        if n.ancestors().any(|a| a.kind() == "lambda_expression") {
            return;
        }
        cx.report(n, format!("Static field `{}` is written from an instance method", f.field.name));
    }
}

declare_rule!(ConstructorCallsOverridable {
    id: "constructor-calls-overridable-method",
    name: "Constructor calls an overridable method",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S1699", "pmd:ConstructorCallsOverridableMethod", "spotbugs:MC_OVERRIDABLE_METHOD_CALL_IN_CONSTRUCTOR"],
    kinds: ["method_invocation"],
    description: "A subclass override runs before the subclass constructor has initialized its fields, so it sees nulls and zeros. Make the called method private or final, or make the class final.",
});

impl Rule for ConstructorCallsOverridable {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.field("object").is_some_and(|o| o.kind() != "this") {
            return;
        }
        let Some(ctor) = enclosing_method_decl(n) else { return };
        if ctor.kind() != "constructor_declaration" || n.ancestors().take_while(|a| *a != ctor).any(|a| matches!(a.kind(), "lambda_expression" | "class_body")) {
            return;
        }
        let Some(cls_decl) = owning_type_decl(ctor) else { return };
        if cls_decl.has_modifier("final") || cls_decl.kind() != "class_declaration" {
            return;
        }
        let Some(cls) = cx.fm.class_of_decl(cls_decl) else { return };
        // a class with only private constructors cannot be subclassed outside
        if cls.constructors().all(|c| c.flags.is_private()) {
            return;
        }
        let Some(c) = cx.fm.select_method(n) else { return };
        if c.owner != cls.name && !cx.fm.is_subclass(&cls.name, &c.owner).is_yes() {
            return;
        }
        let f = c.method.flags;
        if f.is_private() || f.is_final() || f.is_static() {
            return;
        }
        cx.report(n, format!("Overridable method `{}()` called from a constructor", c.method.name));
    }
}

declare_rule!(InnerClassCouldBeStatic {
    id: "inner-class-could-be-static",
    name: "Inner class does not use its enclosing instance",
    category: Performance,
    severity: Info,
    sources: ["sonar:S2694", "spotbugs:SIC_INNER_SHOULD_BE_STATIC", "intellij:InnerClassMayBeStatic"],
    kinds: ["class_declaration"],
    description: "A non-static nested class keeps a hidden reference to its enclosing instance, which costs memory and can leak the outer object. Make it `static` if it does not use the outer instance.",
});

impl Rule for InnerClassCouldBeStatic {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(p) = n.parent() else { return };
        if p.kind() != "class_body" || n.has_modifier("static") {
            return;
        }
        let Some(outer_decl) = p.parent() else { return };
        if outer_decl.kind() != "class_declaration" && outer_decl.kind() != "enum_declaration" && outer_decl.kind() != "record_declaration" {
            return;
        }
        let (Some(inner), Some(outer)) = (cx.fm.class_of_decl(n), cx.fm.class_of_decl(outer_decl)) else { return };
        if !cx.fm.hierarchy_complete(&inner.name) {
            return;
        }
        // nested classes inside complicate the analysis
        if n.descendants().skip(1).any(|d| grounds_syntax::is_type_declaration(d.kind()) || d.kind() == "class_body" && d.parent().is_some_and(|x| x.kind() == "object_creation_expression")) {
            return;
        }
        // extending another inner class requires an outer instance
        if inner.superclass.as_ref().and_then(|s| s.class_name()).and_then(|s| cx.fm.class(s)).is_some_and(|s| s.is_inner()) {
            return;
        }
        let Some(body) = n.field("body") else { return };
        for d in body.descendants() {
            match d.kind() {
                "this" => {
                    if d.parent().is_some_and(|fa| fa.kind() == "field_access" && fa.field("field") == Some(d)) {
                        return; // Outer.this
                    }
                }
                "identifier" => {
                    if let Some(Symbol::Field(f)) = cx.fm.symbol(d) {
                        if !f.field.flags.is_static() && f.owner != inner.name && (f.owner == outer.name || cx.fm.is_subclass(&outer.name, &f.owner).is_yes()) && !cx.fm.is_subclass(&inner.name, &f.owner).is_yes() {
                            return;
                        }
                    }
                }
                "method_invocation" => {
                    if d.field("object").is_none() {
                        match cx.fm.select_method(d) {
                            Some(c) => {
                                if !c.method.flags.is_static() && c.owner != inner.name && !cx.fm.is_subclass(&inner.name, &c.owner).is_yes() {
                                    return;
                                }
                            }
                            None => return,
                        }
                    }
                }
                "object_creation_expression" => {
                    // creating another inner class of the outer needs the outer instance
                    let t = cx.type_of(d);
                    if t.class_name().and_then(|c| cx.fm.class(c)).is_some_and(|c| c.is_inner() && c.outer.as_deref() == Some(&*outer.name) && c.name != inner.name) {
                        return;
                    }
                    if t.is_unknown() {
                        return;
                    }
                }
                _ => {}
            }
        }
        if let Some(name) = n.field("name") {
            cx.report(name, format!("`{}` does not use its enclosing instance; make it static", cx.text(name)));
        }
    }
}

declare_rule!(FieldHidesSuperField {
    id: "field-hides-superclass-field",
    name: "Field hides a superclass field",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S2387", "spotbugs:MF_CLASS_MASKS_FIELD", "intellij:FieldNameHidesFieldInSuperclass"],
    kinds: ["field_declaration"],
    description: "Redeclaring a field with the same name as an accessible superclass field creates two variables; code in the superclass keeps using its own, which is rarely intended.",
});

impl Rule for FieldHidesSuperField {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.has_modifier("static") {
            return;
        }
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        let Some(sup) = cls.superclass.clone() else { return };
        if sup.is_class("java.lang.Object") {
            return;
        }
        for d in n.fields("declarator") {
            let Some(name) = d.field("name").map(|x| x.text(cx.src)) else { continue };
            if let Some(f) = cx.fm.field(&sup, name) {
                if !f.field.flags.is_private() && !f.field.flags.is_static() {
                    cx.report(d, format!("Field `{name}` hides the field inherited from `{}`", simple_name_of(&f.owner)));
                }
            }
        }
    }
}

declare_rule!(MethodNamedLikeClass {
    id: "method-named-like-class",
    name: "Method has the class name (constructor typo?)",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S1223", "spotbugs:NM_METHOD_CONSTRUCTOR_CONFUSION", "pmd:MethodWithSameNameAsEnclosingClass"],
    kinds: ["method_declaration"],
    description: "A method named like its class looks like a constructor but has a return type, so it is never called on construction. Usually `void Foo()` was meant to be `Foo()`.",
});

impl Rule for MethodNamedLikeClass {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(decl) = owning_type_decl(n) else { return };
        let (Some(cn), Some(mn)) = (decl.field("name"), n.field("name")) else { return };
        if cn.text(cx.src) == mn.text(cx.src) {
            cx.report(mn, "Method has the same name as its class; did you mean a constructor?");
        }
    }
}

declare_rule!(ObjectMethodTypo {
    id: "object-method-name-typo",
    name: "Misspelled equals/hashCode/toString",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S1221", "spotbugs:NM_LCASE_HASHCODE", "spotbugs:NM_LCASE_TOSTRING", "spotbugs:NM_BAD_EQUAL", "pmd:SuspiciousHashcodeMethodName"],
    kinds: ["method_declaration"],
    description: "`hashcode()`, `tostring()` or `equal(Object)` do not override the Object methods, so collections and string conversion silently use the defaults.",
});

impl Rule for ObjectMethodTypo {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let name = method_name(n, cx.src);
        let pc = param_count(n);
        // the return type must match too: `Set<String> equal(K key)` is no typo
        let ret = n.field("type").map(|t| cx.text(t)).unwrap_or("");
        let intended = match (name, pc, ret) {
            ("hashcode" | "hashcode_" | "hashCOde" | "HashCode", 0, "int") => "hashCode",
            ("tostring" | "toSTring" | "ToString" | "tostr", 0, "String") => "toString",
            ("equal" | "Equals", 1, "boolean") => "equals",
            _ => return,
        };
        if let Some(nm) = n.field("name") {
            cx.report(nm, format!("`{name}` does not override `{intended}`"));
        }
    }
}

declare_rule!(ClassShadowsSupertypeName {
    id: "class-shadows-supertype-name",
    name: "Class has the same simple name as its supertype",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S2176", "spotbugs:NM_SAME_SIMPLE_NAME_AS_SUPERCLASS", "spotbugs:NM_SAME_SIMPLE_NAME_AS_INTERFACE"],
    kinds: ["class_declaration", "interface_declaration"],
    description: "`class Service extends other.Service` makes every reference ambiguous to readers and forces fully qualified names.",
});

impl Rule for ClassShadowsSupertypeName {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = cx.fm.class_of_decl(n) else { return };
        let simple = c.simple_name().to_string();
        for s in c.superclass.iter().chain(c.interfaces.iter()) {
            if let Some(sn) = s.class_name() {
                if simple_name_of(sn) == simple && sn != &*c.name {
                    if let Some(name) = n.field("name") {
                        cx.report(name, format!("`{simple}` has the same simple name as its supertype `{}`", sn.replace('$', ".")));
                    }
                }
            }
        }
    }
}

// Unicode-aware: `πcase` is camelCase, `Größe` is PascalCase
fn is_pascal(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_uppercase()) && !s.contains('_') && s.chars().all(|c| c.is_alphanumeric() || c == '$')
}

fn is_camel(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_lowercase()) && s.chars().all(|c| c.is_alphanumeric())
}

fn is_upper_snake(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_uppercase()) && s.chars().all(|c| c.is_uppercase() || c.is_numeric() || c == '_' || (c.is_alphabetic() && !c.is_lowercase()))
}

declare_rule!(ClassNaming {
    id: "class-naming",
    name: "Type name does not follow PascalCase",
    category: Style,
    severity: Info,
    sources: ["sonar:S101", "pmd:ClassNamingConventions"],
    kinds: ["class_declaration", "interface_declaration", "enum_declaration", "record_declaration", "annotation_type_declaration"],
    description: "Type names should be PascalCase (`OrderService`), so they are instantly distinguishable from variables and methods.",
});

impl Rule for ClassNaming {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = n.field("name") else { return };
        let s = name.text(cx.src);
        // test classes like `Foo_Test` or JUnit nested classes are allowed underscores
        if !is_pascal(s) && !(cx.is_test() && s.contains('_')) {
            cx.report(name, format!("Type name `{s}` should be PascalCase"));
        }
    }
}

declare_rule!(MethodNaming {
    id: "method-naming",
    name: "Method name does not follow camelCase",
    category: Style,
    severity: Info,
    scope: Main,
    sources: ["sonar:S100", "pmd:MethodNamingConventions"],
    kinds: ["method_declaration"],
    description: "Method names should be camelCase (`calculateTotal`).",
});

impl Rule for MethodNaming {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = n.field("name") else { return };
        let s = name.text(cx.src);
        if !is_camel(s) && !has_annotation_named(n, cx.src, "Override") {
            cx.report(name, format!("Method name `{s}` should be camelCase"));
        }
    }
}

declare_rule!(ConstantNaming {
    id: "constant-naming",
    name: "Constant name does not follow UPPER_SNAKE_CASE",
    category: Style,
    severity: Info,
    sources: ["sonar:S115", "pmd:FieldNamingConventions"],
    kinds: ["field_declaration", "constant_declaration"],
    description: "`static final` constants of primitive/String/immutable types should be UPPER_SNAKE_CASE (`MAX_RETRIES`).",
});

impl Rule for ConstantNaming {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let in_iface = n.kind() == "constant_declaration" || n.parent().is_some_and(|p| p.kind() == "interface_body");
        if !(in_iface || (n.has_modifier("static") && n.has_modifier("final"))) {
            return;
        }
        let Some(t) = n.field("type") else { return };
        let ty = cx.fm.resolve_type_node(t);
        // only clearly constant types; loggers and mutable singletons follow other conventions
        if !(ty.is_prim() || ty.is_string() || ty.is_boxed()) {
            return;
        }
        for d in n.fields("declarator") {
            if let Some(name) = d.field("name") {
                let s = name.text(cx.src);
                if !is_upper_snake(s) && s != "serialVersionUID" {
                    cx.report(name, format!("Constant `{s}` should be UPPER_SNAKE_CASE"));
                }
            }
        }
    }
}

declare_rule!(FieldNaming {
    id: "field-naming",
    name: "Field name does not follow camelCase",
    category: Style,
    severity: Info,
    sources: ["sonar:S116", "pmd:FieldNamingConventions"],
    kinds: ["field_declaration"],
    description: "Non-constant field names should be camelCase (`orderCount`).",
});

impl Rule for FieldNaming {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.has_modifier("static") {
            return;
        }
        if n.parent().is_some_and(|p| p.kind() == "interface_body") {
            return;
        }
        for d in n.fields("declarator") {
            if let Some(name) = d.field("name") {
                let s = name.text(cx.src);
                if !is_camel(s) {
                    cx.report(name, format!("Field `{s}` should be camelCase"));
                }
            }
        }
    }
}

declare_rule!(LocalNaming {
    id: "local-variable-naming",
    name: "Local variable or parameter name does not follow camelCase",
    category: Style,
    severity: Info,
    sources: ["sonar:S117", "pmd:LocalVariableNamingConventions", "pmd:FormalParameterNamingConventions"],
    kinds: ["local_variable_declaration", "formal_parameter"],
    description: "Local variables and parameters should be camelCase.",
});

impl Rule for LocalNaming {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let names: Vec<Node> = if n.kind() == "formal_parameter" { n.field("name").into_iter().collect() } else { n.fields("declarator").into_iter().filter_map(|d| d.field("name")).collect() };
        for name in names {
            let s = name.text(cx.src);
            if s == "_" || s.starts_with("__") {
                continue;
            }
            // final locals used as constants are often UPPER_SNAKE
            if n.has_modifier("final") && is_upper_snake(s) {
                continue;
            }
            if !is_camel(s) {
                cx.report(name, format!("`{s}` should be camelCase"));
            }
        }
    }
}

declare_rule!(PackageNaming {
    id: "package-naming",
    name: "Package name is not lowercase",
    category: Style,
    severity: Info,
    sources: ["sonar:S120", "pmd:PackageCase"],
    kinds: ["package_declaration"],
    description: "Package names should be all lowercase to avoid clashes with class names and case-insensitive file systems.",
});

impl Rule for PackageNaming {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(id) = n.kids().into_iter().find(|k| matches!(k.kind(), "identifier" | "scoped_identifier")) else { return };
        let s = id.text(cx.src);
        if s.chars().any(|c| c.is_ascii_uppercase()) {
            cx.report(id, format!("Package `{s}` should be lowercase"));
        }
    }
}

declare_rule!(TypeParamNaming {
    id: "type-parameter-naming",
    name: "Type parameter name is not a single capital letter",
    category: Style,
    severity: Info,
    enabled: false,
    sources: ["sonar:S119", "pmd:GenericsNaming"],
    kinds: ["type_parameter"],
    description: "Type parameters conventionally are a single uppercase letter (optionally followed by a digit) so they are not confused with class names.",
});

impl Rule for TypeParamNaming {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(id) = n.kid_of_kind("type_identifier") else { return };
        let s = id.text(cx.src);
        let ok = s.len() <= 2 && s.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && s.chars().skip(1).all(|c| c.is_ascii_digit());
        if !ok {
            cx.report(id, format!("Type parameter `{s}` should be a single capital letter"));
        }
    }
}

fn is_serializable<'t>(cx: &Ctx<'_, 't>, c: &ClassInfo) -> Tri {
    cx.fm.is_subclass(&c.name, "java.io.Serializable")
}

declare_rule!(MissingSerialVersionUid {
    id: "missing-serial-version-uid",
    name: "Serializable class without serialVersionUID",
    category: BadPractice,
    severity: Info,
    enabled: false,
    sources: ["sonar:S2057", "spotbugs:SE_NO_SERIALVERSIONID", "pmd:MissingSerialVersionUID"],
    kinds: ["class_declaration"],
    description: "Without an explicit `serialVersionUID` the JVM computes one from the class shape, so harmless changes break deserialization of stored data.",
});

impl Rule for MissingSerialVersionUid {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = cx.fm.class_of_decl(n) else { return };
        if c.is_abstract() || c.flags.has(Flags::ANON) || !is_serializable(cx, &c).is_yes() || c.field("serialVersionUID").is_some() {
            return;
        }
        // exceptions are Serializable through Throwable; most never get serialized
        if cx.fm.is_subclass(&c.name, "java.lang.Throwable").is_yes() {
            return;
        }
        if let Some(name) = n.field("name") {
            cx.report(name, "Serializable class has no serialVersionUID");
        }
    }
}

declare_rule!(SerialVersionUidModifiers {
    id: "serial-version-uid-modifiers",
    name: "serialVersionUID is not static final long",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2057", "spotbugs:SE_NONSTATIC_SERIALVERSIONID", "spotbugs:SE_NONFINAL_SERIALVERSIONID", "spotbugs:SE_NONLONG_SERIALVERSIONID"],
    kinds: ["field_declaration"],
    description: "Serialization ignores a `serialVersionUID` that is not `static final long`; the JVM-computed id is used instead.",
});

impl Rule for SerialVersionUidModifiers {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(d) = n.fields("declarator").into_iter().find(|d| d.field("name").is_some_and(|x| cx.text(x) == "serialVersionUID")) else { return };
        let ty = n.field("type").map(|t| cx.text(t)).unwrap_or("");
        if !(n.has_modifier("static") && n.has_modifier("final") && ty == "long") {
            cx.report(d, "serialVersionUID must be declared `static final long`");
        }
    }
}

declare_rule!(NonSerializableField {
    id: "non-serializable-field",
    name: "Non-transient non-serializable field in a Serializable class",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S1948", "spotbugs:SE_BAD_FIELD"],
    kinds: ["field_declaration"],
    description: "Serializing an object whose field holds a non-Serializable value throws `NotSerializableException`. Mark the field `transient` or make its type Serializable.",
});

impl Rule for NonSerializableField {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.has_modifier("static") || n.has_modifier("transient") {
            return;
        }
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        if !is_serializable(cx, &cls).is_yes() || cx.fm.is_subclass(&cls.name, "java.lang.Throwable").is_yes() || cls.is_enum() || cls.is_record() && false {
            return;
        }
        // custom serialization handles fields itself
        if cls.methods.iter().any(|m| matches!(&*m.name, "writeObject" | "writeReplace" | "writeExternal")) {
            return;
        }
        let Some(t) = n.field("type") else { return };
        let ty = cx.fm.resolve_type_node(t);
        let Some(tn) = ty.class_name() else { return };
        let Some(tc) = cx.fm.class(tn) else { return };
        // interfaces and non-final classes may have serializable implementations
        if tc.is_interface() || (!tc.flags.is_final() && !tc.is_enum() && !tc.is_record() && tc.is_abstract()) {
            return;
        }
        if !tc.flags.is_final() && !tc.is_record() && !tc.is_source() {
            // library non-final classes: only trust a complete hierarchy
            if !cx.fm.hierarchy_complete(tn) {
                return;
            }
        }
        if cx.fm.is_subclass(tn, "java.io.Serializable").is_no() && cx.fm.hierarchy_complete(tn) {
            for d in n.fields("declarator") {
                cx.report(d, format!("Field of non-serializable type `{}` in a Serializable class; make it transient", ty.display_simple()));
            }
        }
    }
}

declare_rule!(SerializationMethodSignature {
    id: "serialization-method-signature",
    name: "Serialization method has the wrong signature",
    category: Bug,
    severity: Error,
    sources: ["sonar:S2061", "spotbugs:SE_METHOD_MUST_BE_PRIVATE", "spotbugs:SE_READ_RESOLVE_IS_STATIC", "spotbugs:SE_READ_RESOLVE_MUST_RETURN_OBJECT"],
    kinds: ["method_declaration"],
    description: "`writeObject`/`readObject`/`readObjectNoData` must be private non-static and `readResolve`/`writeReplace` must be non-static returning Object; otherwise serialization silently ignores them.",
});

impl Rule for SerializationMethodSignature {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let name = method_name(n, cx.src);
        let pc = param_count(n);
        let ptype = n.field("parameters").and_then(|p| p.kids().first().copied()).and_then(|p| p.field("type")).map(|t| type_simple_name(t, cx.src)).unwrap_or("");
        let rtype = n.field("type").map(|t| cx.text(t)).unwrap_or("");
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        if !is_serializable(cx, &cls).is_yes() {
            return;
        }
        let problem = match (name, pc) {
            ("writeObject", 1) if ptype == "ObjectOutputStream" => (!n.has_modifier("private") || n.has_modifier("static")).then_some("must be private and non-static"),
            ("readObject", 1) if ptype == "ObjectInputStream" => (!n.has_modifier("private") || n.has_modifier("static")).then_some("must be private and non-static"),
            ("readObjectNoData", 0) => (!n.has_modifier("private") || n.has_modifier("static")).then_some("must be private and non-static"),
            ("readResolve" | "writeReplace", 0) => {
                if n.has_modifier("static") {
                    Some("must not be static")
                } else if rtype != "Object" && rtype != "java.lang.Object" {
                    Some("must return Object")
                } else {
                    None
                }
            }
            _ => None,
        };
        if let (Some(p), Some(nm)) = (problem, n.field("name")) {
            cx.report(nm, format!("`{name}` {p} or serialization ignores it"));
        }
    }
}

declare_rule!(CloneWithoutSuper {
    id: "clone-without-super-clone",
    name: "clone() does not call super.clone()",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S1182", "spotbugs:CN_IDIOM_NO_SUPER_CALL", "pmd:ProperCloneImplementation"],
    kinds: ["method_declaration"],
    description: "A `clone()` built with `new` returns the wrong type for subclasses. Call `super.clone()` (or better, provide a copy constructor).",
});

impl Rule for CloneWithoutSuper {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if method_name(n, cx.src) != "clone" || param_count(n) != 0 {
            return;
        }
        let Some(decl) = owning_type_decl(n) else { return };
        if decl.has_modifier("final") || decl.kind() != "class_declaration" {
            return;
        }
        let Some(body) = n.field("body") else { return };
        if !cx.text(body).contains("super.clone()") {
            if let Some(nm) = n.field("name") {
                cx.report(nm, "`clone()` should obtain the copy via `super.clone()`");
            }
        }
    }
}

declare_rule!(CloneableWithoutClone {
    id: "cloneable-without-clone",
    name: "Cloneable class does not override clone()",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S2157", "spotbugs:CN_IDIOM"],
    kinds: ["class_declaration"],
    description: "Implementing `Cloneable` without a public `clone()` leaves only the protected `Object.clone()`, which callers cannot use.",
});

impl Rule for CloneableWithoutClone {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = cx.fm.class_of_decl(n) else { return };
        if c.is_abstract() || !c.interfaces.iter().any(|i| i.is_class("java.lang.Cloneable")) {
            return;
        }
        if !cx.fm.overrides_object_method(&c.name, "clone", 0) {
            if let Some(nm) = n.field("name") {
                cx.report(nm, "Class implements Cloneable but does not override `clone()`");
            }
        }
    }
}

declare_rule!(CompareToWithoutEquals {
    id: "compareto-without-equals",
    name: "compareTo() defined without equals()",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S1210", "spotbugs:EQ_COMPARETO_USE_OBJECT_EQUALS"],
    kinds: ["class_declaration"],
    description: "Sorted collections use `compareTo` while hash collections and `contains` use `equals`; when they disagree the same object can be both present and absent.",
});

impl Rule for CompareToWithoutEquals {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = cx.fm.class_of_decl(n) else { return };
        if c.is_abstract() {
            return;
        }
        let has_compare = c.methods.iter().any(|m| &*m.name == "compareTo" && m.params.len() == 1);
        if has_compare && !cx.fm.overrides_object_method(&c.name, "equals", 1) {
            if let Some(m) = methods_of(n).into_iter().find(|m| method_name(*m, cx.src) == "compareTo") {
                if let Some(nm) = m.field("name") {
                    cx.report(nm, "Class defines `compareTo` but not `equals`; they may disagree");
                }
            }
        }
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

declare_rule!(AccessorWrongField {
    id: "accessor-uses-wrong-field",
    name: "Getter or setter accesses a different field",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S4275"],
    kinds: ["method_declaration"],
    description: "`getName()` returning `this.title`, or `setName(x)` assigning `this.title`, is a copy-paste error when a field `name` exists.",
});

impl Rule for AccessorWrongField {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let name = method_name(n, cx.src);
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        let Some(body) = n.field("body") else { return };
        let stmts = body.kids();
        if stmts.len() != 1 {
            return;
        }
        let prop = if let Some(p) = name.strip_prefix("get").or_else(|| name.strip_prefix("is")) {
            p
        } else if let Some(p) = name.strip_prefix("set") {
            p
        } else {
            return;
        };
        if prop.is_empty() || !prop.chars().next().unwrap().is_uppercase() {
            return;
        }
        // the expected field name exists
        let expected = cls.fields.iter().find(|f| cap(&f.name) == prop || cap(&f.name) == format!("Is{prop}")).map(|f| f.name.clone());
        let Some(expected) = expected else { return };
        let accessed = if name.starts_with("set") {
            if param_count(n) != 1 {
                return;
            }
            let e = stmts[0].first_kid().filter(|e| e.kind() == "assignment_expression");
            e.and_then(|e| e.field("left")).and_then(|l| cx.fm.field_ref(l).map(|f| (f, l)))
        } else {
            if param_count(n) != 0 || stmts[0].kind() != "return_statement" {
                return;
            }
            stmts[0].first_kid().and_then(|v| cx.fm.field_ref(v.unparen()).map(|f| (f, v)))
        };
        let Some((f, node)) = accessed else { return };
        if f.owner == cls.name && f.field.name != expected {
            cx.report(node, format!("`{name}` accesses `{}` but field `{expected}` exists", f.field.name));
        }
    }
}

declare_rule!(AbstractClassPublicConstructor {
    id: "abstract-class-public-constructor",
    name: "Abstract class with a public constructor",
    category: Style,
    severity: Info,
    sources: ["sonar:S5993"],
    kinds: ["constructor_declaration"],
    description: "Abstract classes cannot be instantiated directly; a public constructor is misleading. Make it protected.",
});

impl Rule for AbstractClassPublicConstructor {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !n.has_modifier("public") {
            return;
        }
        if owning_type_decl(n).is_some_and(|d| d.has_modifier("abstract")) {
            if let Some(nm) = n.field("name") {
                let _ = cx.text(nm);
                cx.report(nm, "Constructor of an abstract class should be protected");
            }
        }
    }
}

declare_rule!(MutableEnumField {
    id: "mutable-enum-field",
    name: "Enum with mutable state",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S3066", "spotbugs:ME_MUTABLE_ENUM_FIELD", "spotbugs:ME_ENUM_FIELD_SETTER"],
    kinds: ["field_declaration"],
    description: "Enum constants are global singletons; mutable instance fields make them shared mutable state across the whole application and all threads.",
});

impl Rule for MutableEnumField {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.has_modifier("static") || n.has_modifier("final") {
            return;
        }
        let in_enum = n.parent().is_some_and(|p| p.kind() == "enum_body_declarations");
        if in_enum {
            for d in n.fields("declarator") {
                cx.report(d, format!("Enum field `{}` is mutable; make it final", d.field("name").map(|x| cx.text(x)).unwrap_or("")));
            }
        }
    }
}

declare_rule!(StaticViaInstance {
    id: "static-member-via-instance",
    name: "Static member accessed through an instance",
    category: Style,
    severity: Info,
    sources: ["sonar:S2209", "pmd:StaticMemberAccessedViaInstance", "intellij:StaticMethodReferencedViaSubclass"],
    kinds: ["method_invocation", "field_access"],
    description: "`obj.staticMethod()` reads as if it depended on `obj`; it does not (and the instance expression may not even be evaluated as expected). Use the class name.",
});

impl Rule for StaticViaInstance {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(o) = n.field("object") else { return };
        if matches!(o.kind(), "this" | "super") || cx.fm.is_type_name(o) || matches!(cx.fm.symbol(o), Some(Symbol::Package(_))) {
            return;
        }
        if cx.type_of(o).is_unknown() {
            return;
        }
        let (is_static, owner, member) = if n.kind() == "method_invocation" {
            match cx.fm.select_method(n) {
                Some(c) => (c.method.flags.is_static(), c.owner.clone(), c.method.name.clone()),
                None => return,
            }
        } else {
            match cx.fm.field_ref(n) {
                Some(f) => (f.field.flags.is_static(), f.owner.clone(), f.field.name.clone()),
                None => return,
            }
        };
        if is_static {
            cx.report(n, format!("Static member `{member}` accessed via an instance; use `{}`", simple_name_of(&owner)));
        }
    }
}

declare_rule!(EmptyMethod {
    id: "empty-method",
    name: "Empty method without explanation",
    category: Style,
    severity: Info,
    scope: Main,
    sources: ["sonar:S1186", "pmd:UncommentedEmptyMethodBody"],
    kinds: ["method_declaration"],
    description: "An empty method body without a comment leaves readers wondering whether it is unfinished. Add a comment explaining why it does nothing, or throw UnsupportedOperationException.",
});

impl Rule for EmptyMethod {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(body) = n.field("body") else { return };
        if !body.kids().is_empty() || block_has_own_line_comment(body) || cx.text(body).contains("//") || cx.text(body).contains("/*") {
            return;
        }
        // default methods / abstract-class hooks
        if owning_type_decl(n).is_some_and(|d| d.kind() == "interface_declaration" || d.has_modifier("abstract")) {
            return;
        }
        if let Some(nm) = n.field("name") {
            cx.report(nm, "Empty method body; add a comment explaining why or implement it");
        }
    }
}

declare_rule!(MissingOverride {
    id: "missing-override",
    name: "Missing @Override annotation",
    category: Style,
    severity: Info,
    sources: ["sonar:S1161", "pmd:MissingOverride", "intellij:MissingOverrideAnnotation"],
    kinds: ["method_declaration"],
    description: "`@Override` makes the compiler check that the method really overrides something, catching signature typos and broken overrides after refactorings.",
});

impl Rule for MissingOverride {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if has_annotation_named(n, cx.src, "Override") || n.has_modifier("static") || n.has_modifier("private") {
            return;
        }
        let Some(m) = cx.fm.method_of_decl(n) else { return };
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        let mut found: Option<Name> = None;
        for st in cx.fm.all_supertypes(&cls.this_type()).into_iter().skip(1) {
            let Some(sn) = st.class_name() else { continue };
            let Some(sc) = cx.fm.class(sn) else { continue };
            let map = grounds_index::hierarchy::substitution(&sc, st.args());
            for sm in sc.methods.iter().filter(|x| x.name == m.name && x.params.len() == m.params.len() && !x.flags.is_static() && !x.flags.is_private()) {
                let same = sm.params.iter().zip(&m.params).all(|(a, b)| grounds_index::hierarchy::apply_subst(&a.ty, &map).erasure() == b.ty.erasure());
                if same {
                    found = Some(sc.name.clone());
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        if let (Some(owner), Some(nm)) = (found, n.field("name")) {
            let ann = n.modifiers().map(|m| m.start_byte()).unwrap_or(n.start_byte()) as u32;
            let fix = Fix { title: "Add @Override".into(), edits: vec![Edit { start: ann, end: ann, replacement: "@Override\n    ".into() }] };
            cx.report_fix(nm, format!("`{}` overrides `{}` but lacks @Override", m.name, simple_name_of(&owner)), fix);
        }
    }
}

declare_rule!(UselessOverride {
    id: "useless-override",
    name: "Override only calls super",
    category: Style,
    severity: Info,
    sources: ["sonar:S1185", "pmd:UselessOverridingMethod"],
    kinds: ["method_declaration"],
    description: "A method that overrides another only to call `super` with the same arguments adds nothing.",
});

impl Rule for UselessOverride {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(body) = n.field("body") else { return };
        let stmts = body.kids();
        if stmts.len() != 1 {
            return;
        }
        let names = annotation_names(n, cx.src);
        if names.iter().any(|a| *a != "Override") {
            return;
        }
        let name = method_name(n, cx.src);
        let params: Vec<&str> = n.field("parameters").map(|p| p.kids().iter().filter_map(|k| k.field("name").map(|x| x.text(cx.src))).collect()).unwrap_or_default();
        let call = match stmts[0].kind() {
            "return_statement" | "expression_statement" => stmts[0].first_kid(),
            _ => None,
        };
        let Some(call) = call else { return };
        if call.kind() != "method_invocation" || !call.field("object").is_some_and(|o| o.kind() == "super") || cx.fm.call_name(call) != Some(name) {
            return;
        }
        let args: Vec<&str> = cx.fm.args_of(call).iter().map(|a| cx.text(*a)).collect();
        if args == params {
            // widening visibility (protected -> public) is a legitimate reason
            let Some(sel) = cx.fm.select_method(call) else { return };
            if sel.method.flags.has(Flags::PROTECTED) && n.has_modifier("public") {
                return;
            }
            if sel.method.flags.has(Flags::SYNCHRONIZED) != n.has_modifier("synchronized") || n.has_modifier("final") {
                return;
            }
            if let Some(nm) = n.field("name") {
                cx.report(nm, "This override only calls super; remove it");
            }
        }
    }
}

declare_rule!(RedundantModifier {
    id: "redundant-modifier",
    name: "Redundant modifier",
    category: Style,
    severity: Info,
    sources: ["sonar:S2333", "pmd:UnnecessaryModifier", "intellij:UnnecessaryInterfaceModifier"],
    kinds: ["method_declaration", "constant_declaration", "field_declaration", "interface_declaration", "enum_declaration", "record_declaration"],
    description: "Interface methods are implicitly `public abstract`, interface fields `public static final`, nested interfaces/enums/records `static`. Repeating it is noise.",
});

impl Rule for RedundantModifier {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(mods) = n.modifiers() else { return };
        let in_iface = n.parent().is_some_and(|p| matches!(p.kind(), "interface_body" | "annotation_type_body"));
        let redundant: &[&str] = match n.kind() {
            "method_declaration" if in_iface => &["public", "abstract"],
            "constant_declaration" | "field_declaration" if in_iface => &["public", "static", "final"],
            "interface_declaration" | "enum_declaration" | "record_declaration" if n.parent().is_some_and(|p| p.kind() != "program") => {
                if in_iface {
                    &["public", "static"]
                } else {
                    &["static"]
                }
            }
            "method_declaration" => {
                // final on private methods or on methods of a final class
                let cls_final = owning_type_decl(n).is_some_and(|d| d.has_modifier("final") || d.kind() == "record_declaration" || d.kind() == "enum_declaration");
                if n.has_modifier("final") && (n.has_modifier("private") || cls_final) && !has_annotation_named(n, cx.src, "SafeVarargs") {
                    &["final"]
                } else {
                    return;
                }
            }
            _ => return,
        };
        let mut c = mods.walk();
        for k in mods.children(&mut c) {
            if !k.is_named() && redundant.contains(&k.kind()) {
                // abstract on an interface method with a body is an error; public on default is fine to drop
                cx.report_fix(k, format!("Redundant `{}` modifier", k.kind()), Fix::delete("Remove modifier", k.start_byte() as u32, (k.end_byte() + 1).min(cx.src.len()) as u32));
            }
        }
    }
}

declare_rule!(TooManyParameters {
    id: "too-many-parameters",
    name: "Method has too many parameters",
    category: Complexity,
    severity: Info,
    sources: ["sonar:S107", "pmd:ExcessiveParameterList"],
    kinds: ["method_declaration", "constructor_declaration"],
    description: "Long parameter lists are hard to call correctly (argument order mistakes). Group related parameters into an object. Threshold: `max` option (default 7).",
});

impl Rule for TooManyParameters {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let max = cx.option_usize("max", 7);
        let pc = param_count(n);
        if pc <= max {
            return;
        }
        // DI constructors and overrides are dictated by others
        let anns = annotation_names(n, cx.src);
        if anns.iter().any(|a| matches!(*a, "Override" | "Inject" | "Autowired" | "JsonCreator")) {
            return;
        }
        if n.kind() == "constructor_declaration" && owning_type_decl(n).is_some_and(|d| annotation_names(d, cx.src).iter().any(|a| matches!(*a, "Component" | "Service" | "Repository" | "Controller" | "RestController" | "Configuration" | "Singleton"))) {
            return;
        }
        if let Some(nm) = n.field("name") {
            cx.report(nm, format!("{pc} parameters (max {max}); consider a parameter object"));
        }
    }
}

declare_rule!(MutableException {
    id: "mutable-exception",
    name: "Exception class with mutable fields",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S1165"],
    kinds: ["field_declaration"],
    description: "Exceptions are values describing a failure; mutable fields allow handlers to change that description after the fact. Make exception fields final.",
});

impl Rule for MutableException {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.has_modifier("static") || n.has_modifier("final") {
            return;
        }
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        if cx.fm.is_subclass(&cls.name, "java.lang.Throwable").is_yes() {
            for d in n.fields("declarator") {
                cx.report(d, "Exception fields should be final");
            }
        }
    }
}

declare_rule!(ExceptionNaming {
    id: "exception-class-naming",
    name: "Exception class name does not end with Exception",
    category: Style,
    severity: Info,
    sources: ["sonar:S2166", "pmd:ClassNamingConventions"],
    kinds: ["class_declaration"],
    description: "Classes extending Exception should end in `Exception` (and non-exceptions should not), so readers know what they are.",
});

impl Rule for ExceptionNaming {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = cx.fm.class_of_decl(n) else { return };
        let name = c.simple_name().to_string();
        let is_exc = cx.fm.is_subclass(&c.name, "java.lang.Exception").is_yes();
        if is_exc && !name.ends_with("Exception") && !name.ends_with("Failure") && !name.ends_with("Error") {
            if let Some(nm) = n.field("name") {
                cx.report(nm, format!("Exception class `{name}` should end with `Exception`"));
            }
        }
    }
}

declare_rule!(RecordArrayComponent {
    id: "record-array-component",
    name: "Record with an array component uses default equals/hashCode",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S6218"],
    kinds: ["record_declaration"],
    description: "Records compare array components by reference, so two records with equal array contents are not equal. Override `equals`, `hashCode` and `toString` using `Arrays` methods.",
});

impl Rule for RecordArrayComponent {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = cx.fm.class_of_decl(n) else { return };
        let has_array = c.fields.iter().any(|f| f.flags.has(Flags::RECORD_COMPONENT) && f.ty.is_array());
        if !has_array {
            return;
        }
        let overrides = c.methods.iter().any(|m| &*m.name == "equals" && m.params.len() == 1 && !m.flags.has(Flags::IMPLICIT));
        if !overrides {
            if let Some(nm) = n.field("name") {
                cx.report(nm, "Record has an array component but does not override equals/hashCode");
            }
        }
    }
}

declare_rule!(EqualsWithoutTypeCheck {
    id: "equals-without-type-check",
    name: "equals() casts its argument without checking the type",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2097", "spotbugs:BC_EQUALS_METHOD_SHOULD_WORK_FOR_ALL_OBJECTS"],
    kinds: ["method_declaration"],
    description: "`equals(Object)` must return false for objects of other types; casting without an `instanceof`/`getClass()` check throws ClassCastException instead.",
});

impl Rule for EqualsWithoutTypeCheck {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if method_name(n, cx.src) != "equals" || param_count(n) != 1 {
            return;
        }
        let Some(p) = n.field("parameters").and_then(|p| p.kids().first().copied()) else { return };
        let Some(pname) = p.field("name").map(|x| x.text(cx.src)) else { return };
        if p.field("type").is_none_or(|t| type_simple_name(t, cx.src) != "Object") {
            return;
        }
        let Some(body) = n.field("body") else { return };
        let text = cx.text(body);
        if text.contains("instanceof") || text.contains("getClass()") || text.contains("isInstance(") || text.contains("isAssignableFrom") || text.contains("super.equals") {
            return;
        }
        let mut casts = Vec::new();
        find_kind(body, "cast_expression", &mut casts);
        if let Some(c) = casts.iter().find(|c| c.field("value").is_some_and(|v| cx.text(v) == pname)) {
            cx.report(*c, format!("`{pname}` is cast without a type check; equals must return false for other types"));
        }
    }
}

declare_rule!(AbstractClassNoAbstractMethods {
    id: "abstract-class-without-abstract-methods",
    name: "Abstract class without abstract methods",
    category: Style,
    severity: Info,
    enabled: false,
    sources: ["sonar:S1694", "pmd:AbstractClassWithoutAbstractMethod"],
    kinds: ["class_declaration"],
    description: "An abstract class with no abstract methods may just need a protected constructor, or should be a concrete class.",
});

impl Rule for AbstractClassNoAbstractMethods {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !n.has_modifier("abstract") || n.field("superclass").is_some() || n.field("interfaces").is_some() {
            return;
        }
        if !methods_of(n).iter().any(|m| m.has_modifier("abstract")) {
            if let Some(nm) = n.field("name") {
                let _ = cx.text(nm);
                cx.report(nm, "Abstract class declares no abstract methods");
            }
        }
    }
}

declare_rule!(PrivateConstructorNotFinal {
    id: "private-constructor-class-not-final",
    name: "Class with only private constructors is not final",
    category: Style,
    severity: Info,
    sources: ["pmd:ClassWithOnlyPrivateConstructorsShouldBeFinal", "sonar:S2974"],
    kinds: ["class_declaration"],
    description: "A class whose constructors are all private cannot be subclassed outside its nest; declaring it final states that explicitly.",
});

impl Rule for PrivateConstructorNotFinal {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.has_modifier("final") || n.has_modifier("abstract") || n.has_modifier("sealed") || n.has_modifier("non-sealed") {
            return;
        }
        let ctors: Vec<Node> = members(n).into_iter().filter(|m| m.kind() == "constructor_declaration").collect();
        if ctors.is_empty() || !ctors.iter().all(|c| c.has_modifier("private")) {
            return;
        }
        // nested subclasses inside the same file can extend it
        let name = n.field("name").map(|x| x.text(cx.src)).unwrap_or("");
        let root = cx.fm.root();
        let extended = root.descendants().any(|d| d.kind() == "superclass" && cx.text(d).contains(name));
        if extended {
            return;
        }
        let _ = class_decls();
        if let Some(nm) = n.field("name") {
            cx.report(nm, "Class has only private constructors; make it final");
        }
    }
}
