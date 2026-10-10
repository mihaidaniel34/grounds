//! Dependency-injection (Spring, Micronaut, Jakarta) rules built on the
//! project-wide bean model.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_di::*;
use grounds_index::*;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(BeanNotFound),
        Box::new(AmbiguousBean),
        Box::new(QualifierNotFound),
        Box::new(LombokDropsQualifier),
        Box::new(FieldInjection),
        Box::new(InjectStaticField),
        Box::new(InjectFinalField),
        Box::new(MultipleInjectConstructors),
        Box::new(ProxyAnnotationOnPrivate),
        Box::new(SelfInvocationBypassesProxy),
        Box::new(AsyncReturnType),
        Box::new(ScheduledWithParameters),
        Box::new(ComponentOnAbstract),
        Box::new(BeanMethodCallInLiteMode),
        Box::new(SingletonMutableState),
        Box::new(PathVariableMismatch),
        Box::new(RequestMappingWithoutMethod),
        Box::new(RedundantResponseBody),
        Box::new(CircularDependency),
        Box::new(InvalidPlaceholder),
        Box::new(BeanNameConflict),
        Box::new(FinalConfigurationClass),
        Box::new(InjectionInNonBean),
    ]
}

/// The class declarations in this file with their binary names.
fn file_classes<'t>(cx: &Ctx<'_, 't>) -> Vec<(Node<'t>, Name)> {
    cx.fm.root().descendants().filter_map(|d| cx.fm.class_name_of_decl(d).map(|n| (d, n.clone()))).collect()
}

/// Where to report an injection point: the parameter/field node, or the
/// class name for Lombok-generated constructors.
fn point_node<'t>(cx: &Ctx<'_, 't>, decl: Node<'t>, p: &InjectionPoint) -> Option<Node<'t>> {
    match &p.kind {
        PointKind::Field => {
            let (s, e) = p.range?;
            cx.fm.root().descendant_for_byte_range(s as usize, e as usize)
        }
        PointKind::ConstructorParam(i) | PointKind::MethodParam(_, i) | PointKind::FactoryParam(_, i) => {
            let (s, e) = p.range?;
            let name_node = cx.fm.root().descendant_for_byte_range(s as usize, e as usize)?;
            let m = name_node.parent()?;
            if matches!(m.kind(), "constructor_declaration" | "method_declaration") && m.field("name") == Some(name_node) {
                let params: Vec<Node> = m.field("parameters")?.kids().into_iter().filter(|k| matches!(k.kind(), "formal_parameter" | "spread_parameter")).collect();
                return params.get(*i).copied();
            }
            // generated constructor (Lombok): report on the field with the same name
            if let Some(f) = util_find_field(decl, &p.name, cx.src) {
                return Some(f);
            }
            decl.field("name")
        }
    }
}

fn util_find_field<'t>(decl: Node<'t>, name: &str, src: &str) -> Option<Node<'t>> {
    for m in members(decl) {
        if m.kind() == "field_declaration" {
            for d in m.fields("declarator") {
                if d.field("name").is_some_and(|n| n.text(src) == name) {
                    return Some(d);
                }
            }
        }
    }
    None
}

fn bean_list(di: &DiModel, idx: &[usize]) -> String {
    let mut v: Vec<String> = idx.iter().take(5).map(|&i| di.beans[i].display()).collect();
    if idx.len() > 5 {
        v.push(format!("and {} more", idx.len() - 5));
    }
    v.join(", ")
}

/// Run `f` for every injection point declared in this file with its resolution.
fn for_each_point<'t>(cx: &mut Ctx<'_, 't>, mut f: impl FnMut(&mut Ctx<'_, 't>, Node<'t>, &InjectionPoint, Resolution, &DiModel)) {
    let Some(di) = cx.di else { return };
    let include_test = cx.is_test();
    for (decl, name) in file_classes(cx) {
        for p in di.points_of(&name) {
            let res = di.resolve(cx.fm, p, include_test);
            if let Some(node) = point_node(cx, decl, p) {
                f(cx, node, p, res, di);
            }
        }
    }
}

declare_rule!(BeanNotFound {
    id: "bean-not-found",
    name: "No bean provides an injected dependency",
    category: Framework,
    severity: Error,
    tier: Edit,
    sources: ["intellij:SpringJavaInjectionPointsAutowiringInspection", "spring:NoSuchBeanDefinitionException"],
    kinds: ["<file>"],
    description: "No component, @Bean method, repository or client in the project provides this type, so the application context fails at startup with NoSuchBeanDefinitionException (Micronaut: NoSuchBeanException). Only reported for types declared in the project, since library types are often provided by auto-configuration.",
});

impl Rule for BeanNotFound {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        for_each_point(cx, |cx, node, p, res, _| {
            if res == Resolution::NotFound {
                cx.report(node, format!("No bean of type `{}` is defined; injection of `{}` fails at startup", p.ty.display_simple(), p.name));
            }
        });
    }
}

declare_rule!(AmbiguousBean {
    id: "ambiguous-bean",
    name: "Several beans match an injection point",
    category: Framework,
    severity: Error,
    tier: Edit,
    sources: ["intellij:SpringJavaInjectionPointsAutowiringInspection", "spring:NoUniqueBeanDefinitionException"],
    kinds: ["<file>"],
    description: "More than one bean of the requested type exists and nothing (qualifier, @Primary, matching parameter name) selects one, so startup fails with NoUniqueBeanDefinitionException. Add a @Qualifier, mark one bean @Primary, or inject a List.",
});

impl Rule for AmbiguousBean {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        for_each_point(cx, |cx, node, p, res, di| {
            if let Resolution::Ambiguous(c) = res {
                cx.report(node, format!("{} beans of type `{}` match `{}`: {}; add a @Qualifier or @Primary", c.len(), p.ty.display_simple(), p.name, bean_list(di, &c)));
            }
        });
    }
}

declare_rule!(QualifierNotFound {
    id: "qualifier-not-found",
    name: "Qualifier matches no bean",
    category: Framework,
    severity: Error,
    tier: Edit,
    sources: ["intellij:SpringQualifierInjectionInspection", "spring:NoSuchBeanDefinitionException"],
    kinds: ["<file>"],
    description: "Beans of the requested type exist, but none has the requested qualifier or name (often a typo or a renamed bean), so startup fails.",
});

impl Rule for QualifierNotFound {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        for_each_point(cx, |cx, node, p, res, di| {
            if let Resolution::QualifierNotFound { qualifier, candidates } = res {
                cx.report(node, format!("No `{}` bean is qualified `{}`; available: {}", p.ty.display_simple(), qualifier.trim_start_matches('@'), bean_list(di, &candidates)));
            }
        });
    }
}

declare_rule!(LombokDropsQualifier {
    id: "lombok-constructor-drops-qualifier",
    name: "@Qualifier/@Value on a field of a Lombok-generated constructor",
    category: Framework,
    severity: Warning,
    tier: Edit,
    sources: ["lombok:copyableAnnotations", "intellij:SpringJavaInjectionPointsAutowiringInspection"],
    kinds: ["field_declaration"],
    description: "Lombok's @RequiredArgsConstructor/@AllArgsConstructor do not copy @Qualifier or @Value from fields to constructor parameters unless `lombok.copyableAnnotations` is configured, so the annotation is silently ignored and injection becomes ambiguous or wrong.",
});

impl Rule for LombokDropsQualifier {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.di.is_some_and(|d| d.lombok_copies_qualifiers) {
            return;
        }
        let names = annotation_names(n, cx.src);
        let Some(ann) = names.iter().find(|a| matches!(**a, "Qualifier" | "Value" | "Named")) else { return };
        if !n.has_modifier("final") && !names.contains(&"NonNull") {
            return;
        }
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        let generated_ctor = cls.constructors().any(|c| c.generated_by.as_deref() == Some("lombok") && c.params.iter().any(|p| n.fields("declarator").iter().any(|d| d.field("name").is_some_and(|x| cx.text(x) == &*p.name))));
        if generated_ctor {
            cx.report(n, format!("Lombok does not copy @{ann} to the generated constructor parameter; it is ignored unless lombok.copyableAnnotations includes it"));
        }
    }
}

declare_rule!(FieldInjection {
    id: "field-injection",
    name: "Field injection",
    category: Framework,
    severity: Info,
    scope: Main,
    sources: ["sonar:S6813", "intellij:SpringJavaAutowiredFieldsWarningInspection"],
    kinds: ["field_declaration"],
    description: "Injected fields hide dependencies, cannot be final, and make the class impossible to construct in tests without a container. Use constructor injection.",
});

impl Rule for FieldInjection {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let names = annotation_names(n, cx.src);
        if names.iter().any(|a| matches!(*a, "Autowired" | "Inject")) && !n.has_modifier("static") {
            cx.report(n, "Prefer constructor injection over field injection");
        }
    }
}

declare_rule!(InjectStaticField {
    id: "inject-static-field",
    name: "Injection annotation on a static field",
    category: Framework,
    severity: Error,
    tier: Edit,
    sources: ["intellij:SpringJavaStaticMembersAutowiringInspection", "sonar:S3749"],
    kinds: ["field_declaration", "method_declaration"],
    description: "Spring and JSR-330 containers do not inject static fields (or static methods); the field stays null.",
});

impl Rule for InjectStaticField {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let names = annotation_names(n, cx.src);
        // JSR-330 allows static injection in some containers (Guice); Spring never injects statics
        let spring = cx.fm.stub.imports.iter().any(|i| i.path.starts_with("org.springframework"));
        if n.has_modifier("static") && names.iter().any(|a| matches!(*a, "Autowired" | "Value") || (spring && matches!(*a, "Inject" | "Resource"))) {
            cx.report(n, "Static members are not injected; the value stays null");
        }
    }
}

declare_rule!(InjectFinalField {
    id: "inject-final-field",
    name: "Injection annotation on a final field",
    category: Framework,
    severity: Error,
    tier: Edit,
    sources: ["intellij:SpringJavaInjectionPointsAutowiringInspection"],
    kinds: ["field_declaration"],
    description: "JSR-330 containers (Micronaut, Guice, CDI) reject `@Inject` on a final field; use constructor injection. Not reported in Spring projects, where reflective injection sets final fields.",
});

impl Rule for InjectFinalField {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let names = annotation_names(n, cx.src);
        let spring = cx.di.is_none_or(|d| d.frameworks.contains(&grounds_di::Framework::Spring));
        if n.has_modifier("final") && names.contains(&"Inject") && !spring {
            cx.report(n, "Final fields cannot be field-injected; inject through the constructor");
        }
    }
}

declare_rule!(MultipleInjectConstructors {
    id: "multiple-inject-constructors",
    name: "Several constructors marked for injection",
    category: Framework,
    severity: Error,
    sources: ["sonar:S6818", "intellij:SpringJavaConstructorAutowiringInspection"],
    kinds: ["class_declaration"],
    description: "Only one constructor may be @Autowired(required=true)/@Inject; with several the container fails at startup.",
});

impl Rule for MultipleInjectConstructors {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let ctors: Vec<Node> = members(n).into_iter().filter(|m| m.kind() == "constructor_declaration").filter(|m| {
            let names = annotation_names(*m, cx.src);
            names.contains(&"Inject") || (names.contains(&"Autowired") && !cx.text(*m).contains("required = false") && !cx.text(*m).contains("required=false"))
        }).collect();
        if ctors.len() > 1 {
            if let Some(nm) = ctors[1].field("name") {
                cx.report(nm, "More than one constructor is marked for injection");
            }
        }
    }
}

const PROXY_ANNOTATIONS: &[&str] = &["Transactional", "Async", "Cacheable", "CacheEvict", "CachePut", "Retryable", "PreAuthorize", "PostAuthorize", "Secured", "RolesAllowed", "Timed", "Counted", "Lock", "TransactionalEventListener"];

declare_rule!(ProxyAnnotationOnPrivate {
    id: "proxy-annotation-on-non-proxyable-method",
    name: "Proxy-based annotation on a private, final or static method",
    category: Framework,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S2230", "sonar:S6814", "intellij:SpringTransactionalMethodCallsInspection"],
    kinds: ["method_declaration"],
    description: "@Transactional, @Async, @Cacheable etc. work through proxies that can only intercept overridable public methods. On private, final or static methods they are silently ignored.",
});

impl Rule for ProxyAnnotationOnPrivate {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let names = annotation_names(n, cx.src);
        let Some(a) = names.iter().find(|a| PROXY_ANNOTATIONS.contains(a)) else { return };
        // Micronaut AOP works on package-private/protected methods at compile time, but not private
        let why = if n.has_modifier("private") {
            "private"
        } else if n.has_modifier("static") {
            "static"
        } else if n.has_modifier("final") {
            // a JDK interface proxy still intercepts a final method that
            // implements an interface method, but Spring Boot proxies classes
            // by default: only when the project opts into interface proxies
            let jdk_proxies = cx.fm.index.source_class_names().filter_map(|c| cx.fm.class(c)).any(|c| {
                c.annotations.iter().any(|a| a.arg("proxyTargetClass").and_then(|v| v.as_bool()) == Some(false))
            });
            let name = method_name(n, cx.src);
            let arity = n.field("parameters").map(|p| p.kids().iter().filter(|k| matches!(k.kind(), "formal_parameter" | "spread_parameter")).count()).unwrap_or(0);
            let via_interface = owning_type_decl(n).and_then(|d| cx.fm.class_of_decl(d)).is_some_and(|c| {
                cx.fm.all_supertypes(&c.this_type()).iter().filter_map(|t| t.class_name().and_then(|cn| cx.fm.class(cn))).any(|i| i.is_interface() && i.methods.iter().any(|m| *m.name == *name && m.params.len() == arity))
            });
            if jdk_proxies && via_interface {
                return;
            }
            "final"
        } else {
            return;
        };
        if let Some(nm) = n.field("name") {
            cx.report(nm, format!("@{a} has no effect on a {why} method"));
        }
    }
}

declare_rule!(SelfInvocationBypassesProxy {
    id: "self-invocation-bypasses-proxy",
    name: "Self-invocation bypasses @Transactional/@Async/@Cacheable",
    category: Framework,
    severity: Warning,
    sources: ["sonar:S6809", "intellij:SpringTransactionalMethodCallsInspection"],
    kinds: ["method_invocation"],
    description: "Calling an annotated method on `this` skips the Spring proxy, so the transaction, async execution or caching does not happen. Call it through another bean (or inject self).",
});

impl Rule for SelfInvocationBypassesProxy {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.field("object").is_some_and(|o| o.kind() != "this") {
            return;
        }
        let Some(c) = cx.fm.select_method(n) else { return };
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        if c.owner != cls.name {
            return;
        }
        let Some(a) = c.method.annotations.iter().find(|a| PROXY_ANNOTATIONS.contains(&a.simple_name())) else { return };
        // the caller itself having the same annotation means the call already runs in that context
        if let Some(m) = enclosing_method_decl(n) {
            if annotation_names(m, cx.src).contains(&a.simple_name()) {
                return;
            }
        }
        // class-level annotation applies to all methods: self-calls are fine
        if cls.annotations.iter().any(|x| x.simple_name() == a.simple_name()) {
            return;
        }
        if cx.di.is_some_and(|d| d.bean_of_class(&cls.name).is_none()) {
            return;
        }
        cx.report(n, format!("Self-invocation of @{} method `{}()` bypasses the proxy", a.simple_name(), c.method.name));
    }
}

declare_rule!(AsyncReturnType {
    id: "async-method-return-type",
    name: "@Async method returns a value that is lost",
    category: Framework,
    severity: Warning,
    sources: ["sonar:S5772", "intellij:SpringAsyncMethodInspection"],
    kinds: ["method_declaration"],
    description: "Spring @Async methods run on another thread and can only return void or a Future/CompletableFuture; any other return value is replaced by null.",
});

impl Rule for AsyncReturnType {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !annotation_names(n, cx.src).contains(&"Async") {
            return;
        }
        let Some(t) = n.field("type") else { return };
        let ty = cx.fm.resolve_type_node(t);
        if ty == Ty::Void || ty.is_unknown() {
            return;
        }
        let ok = is_subtype_any(cx.fm, &ty, &["java.util.concurrent.Future", "java.util.concurrent.CompletionStage", "org.reactivestreams.Publisher"]) || ty.class_name().is_some_and(|c| c.contains("Mono") || c.contains("Flux") || c.contains("Future"));
        if !ok {
            cx.report(t, format!("@Async methods cannot return `{}`; the caller receives null", ty.display_simple()));
        }
    }
}

declare_rule!(ScheduledWithParameters {
    id: "scheduled-method-with-parameters",
    name: "@Scheduled method with parameters",
    category: Framework,
    severity: Error,
    tier: Edit,
    sources: ["intellij:SpringScheduledMethodInspection"],
    kinds: ["method_declaration"],
    description: "Scheduled methods are invoked without arguments; Spring refuses to start if a @Scheduled method declares parameters.",
});

impl Rule for ScheduledWithParameters {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let names = annotation_names(n, cx.src);
        if names.contains(&"Scheduled") && param_count(n) > 0 && !cx.fm.stub.imports.iter().any(|i| i.path.starts_with("io.micronaut")) {
            if let Some(nm) = n.field("name") {
                cx.report(nm, "@Scheduled methods must not take parameters");
            }
        }
    }
}

declare_rule!(ComponentOnAbstract {
    id: "component-on-abstract-type",
    name: "Stereotype annotation on an abstract class or interface",
    category: Framework,
    severity: Warning,
    sources: ["intellij:SpringJavaComponentInspection"],
    kinds: ["class_declaration", "interface_declaration"],
    description: "Spring's @Component/@Service/@Controller on an abstract class or plain interface creates no bean: component scanning skips abstract types. Annotate the concrete implementation. (Not reported for jakarta @Singleton: Micronaut generates implementations for introduction advice and inherits scopes from interfaces.)",
});

impl Rule for ComponentOnAbstract {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.kind() == "class_declaration" && !n.has_modifier("abstract") {
            return;
        }
        let names = annotation_names(n, cx.src);
        if let Some(a) = names.iter().find(|a| matches!(**a, "Component" | "Service" | "Controller" | "RestController")) {
            // Spring Data / Micronaut Data interfaces annotated @Repository are fine
            if n.kind() == "interface_declaration" && names.iter().any(|x| matches!(*x, "Repository" | "Client" | "FeignClient" | "Mapper" | "JdbcRepository")) {
                return;
            }
            // abstract classes with @Lookup methods are subclassed by Spring
            if methods_of(n).iter().any(|m| annotation_names(*m, cx.src).contains(&"Lookup")) {
                return;
            }
            if let Some(nm) = n.field("name") {
                cx.report(nm, format!("@{a} on an abstract type creates no bean"));
            }
        }
    }
}

declare_rule!(BeanMethodCallInLiteMode {
    id: "bean-method-call-in-lite-mode",
    name: "@Bean method called directly outside a proxied @Configuration",
    category: Framework,
    severity: Warning,
    sources: ["intellij:SpringConfigurationProxyMethods", "sonar:S6838"],
    kinds: ["method_invocation"],
    description: "Calling another @Bean method only returns the container's singleton inside a proxied `@Configuration` class. In `@Component` classes or `@Configuration(proxyBeanMethods = false)` each call creates a new instance. Inject the bean as a method parameter instead.",
});

impl Rule for BeanMethodCallInLiteMode {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.field("object").is_some_and(|o| o.kind() != "this") {
            return;
        }
        let Some(m) = enclosing_method_decl(n) else { return };
        if !annotation_names(m, cx.src).contains(&"Bean") {
            return;
        }
        let Some(c) = cx.fm.select_method(n) else { return };
        if !c.method.annotations.iter().any(|a| a.simple_name() == "Bean" && (a.name.starts_with("org.springframework") || !a.resolved)) {
            return;
        }
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        if c.owner != cls.name {
            return;
        }
        let config = cls.annotations.iter().find(|a| a.simple_name() == "Configuration");
        let proxied = config.is_some_and(|a| a.arg("proxyBeanMethods").and_then(|v| v.as_bool()) != Some(false));
        if !proxied {
            cx.report(n, format!("`{}()` creates a new instance here because the class is not a proxied @Configuration", c.method.name));
        }
    }
}

declare_rule!(SingletonMutableState {
    id: "singleton-mutable-state",
    name: "Mutable state in a singleton bean",
    category: Concurrency,
    severity: Warning,
    scope: Main,
    sources: ["sonar:S3749", "intellij:SpringBeanMutableState"],
    kinds: ["field_declaration"],
    description: "Singleton beans (controllers, services) are shared by all requests and threads. A non-final, non-injected field that methods write to leaks data between requests and races. Keep request state in local variables.",
});

impl Rule for SingletonMutableState {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.has_modifier("static") || n.has_modifier("final") || n.has_modifier("volatile") {
            return;
        }
        if !annotation_names(n, cx.src).is_empty() {
            return;
        }
        let Some(decl) = owning_type_decl(n) else { return };
        let cls_anns = annotation_names(decl, cx.src);
        if !cls_anns.iter().any(|a| matches!(*a, "Service" | "Controller" | "RestController" | "Component" | "Repository" | "Singleton")) {
            return;
        }
        if cls_anns.iter().any(|a| matches!(*a, "Scope" | "RequestScope" | "SessionScope" | "Prototype" | "ConfigurationProperties" | "Data" | "Setter")) {
            return;
        }
        let Some(cls) = cx.fm.class_of_decl(decl) else { return };
        for d in n.fields("declarator") {
            let Some(name) = d.field("name").map(|x| x.text(cx.src)) else { continue };
            // written outside constructors / init methods?
            let written_in_method = methods_of(decl).iter().filter(|m| !annotation_names(**m, cx.src).iter().any(|a| matches!(*a, "PostConstruct" | "Autowired" | "Inject"))).any(|m| {
                let mut assigns = Vec::new();
                if let Some(b) = m.field("body") {
                    find_kind(b, "assignment_expression", &mut assigns);
                    find_kind(b, "update_expression", &mut assigns);
                }
                assigns.iter().any(|a| {
                    let t = if a.kind() == "assignment_expression" { a.field("left") } else { a.first_kid() };
                    t.and_then(|t| cx.fm.field_ref(t)).is_some_and(|f| f.owner == cls.name && &*f.field.name == name)
                })
            });
            if written_in_method {
                cx.report(d, format!("Field `{name}` of a singleton bean is modified by its methods; this state is shared across requests"));
            }
        }
    }
}

/// Mapping templates of a declaration; `None` if one is not a constant we
/// could evaluate (`IMAGE_URL + "/info"` with an unresolved constant).
fn mapping_paths<'t>(cx: &Ctx<'_, 't>, decl: Node<'t>) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for a in cx.fm.annotations_of(decl) {
        if matches!(a.simple_name(), "RequestMapping" | "GetMapping" | "PostMapping" | "PutMapping" | "DeleteMapping" | "PatchMapping" | "Get" | "Post" | "Put" | "Delete" | "Patch" | "Controller") {
            for k in ["value", "path", "uri", "uris"] {
                if let Some(v) = a.arg(k) {
                    if v.values().iter().any(|x| !matches!(x, AnnotationValue::Const(Constant::Str(_)))) {
                        return None;
                    }
                    out.extend(v.strings());
                }
            }
        }
    }
    Some(out)
}

fn template_vars(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = path;
    while let Some(s) = rest.find('{') {
        let after = &rest[s + 1..];
        let Some(e) = after.find('}') else { break };
        let var = &after[..e];
        let name = var.split(':').next().unwrap_or("").trim_start_matches(['?', '+', '.', '/', '#', '*']);
        if !name.is_empty() {
            out.push(name.to_string());
        }
        rest = &after[e + 1..];
    }
    out
}

declare_rule!(PathVariableMismatch {
    id: "path-variable-mismatch",
    name: "@PathVariable not present in the mapping template",
    category: Framework,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S6856", "intellij:MVCPathVariableInspection"],
    kinds: ["method_declaration"],
    description: "A `@PathVariable` whose name does not appear as `{name}` in the request mapping fails every request with MissingPathVariableException.",
});

impl Rule for PathVariableMismatch {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(method_paths) = mapping_paths(cx, n) else { return };
        if method_paths.is_empty() && !annotation_names(n, cx.src).iter().any(|a| a.ends_with("Mapping")) {
            return;
        }
        let Some(class_paths) = owning_type_decl(n).map(|d| mapping_paths(cx, d)).unwrap_or(Some(Vec::new())) else { return };
        let mut vars: Vec<String> = Vec::new();
        for p in method_paths.iter().chain(class_paths.iter()) {
            vars.extend(template_vars(p));
        }
        let Some(params) = n.field("parameters") else { return };
        for p in params.kids() {
            let anns = cx.fm.annotations_of(p);
            let Some(pv) = anns.iter().find(|a| a.simple_name() == "PathVariable" || (a.simple_name() == "PathVariable" && a.name.starts_with("io.micronaut"))) else { continue };
            let pname = p.field("name").map(|x| cx.text(x)).unwrap_or("");
            let name = first_string_ann(pv, &["value", "name"]).unwrap_or_else(|| pname.to_string());
            // a Map parameter receives all variables; an optional variable
            // (`required = false`, Optional<..>) may be absent from the template
            let simple_type = p.field("type").map(|t| type_simple_name(t, cx.src)).unwrap_or("");
            if matches!(simple_type, "Map" | "MultiValueMap" | "Optional") {
                continue;
            }
            if pv.arg("required").and_then(|v| v.as_bool()) == Some(false) {
                continue;
            }
            if !vars.contains(&name) {
                cx.report(p, format!("Path variable `{name}` does not appear in the mapping{}", if vars.is_empty() { String::new() } else { format!(" (available: {})", vars.join(", ")) }));
            }
        }
    }
}

fn first_string_ann(a: &AnnotationInfo, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| a.arg(k).and_then(|v| v.strings().into_iter().find(|s| !s.is_empty())))
}

declare_rule!(RequestMappingWithoutMethod {
    id: "request-mapping-without-method",
    name: "@RequestMapping without an HTTP method",
    category: Security,
    severity: Info,
    sources: ["sonar:S3752"],
    kinds: ["method_declaration"],
    description: "A method-level @RequestMapping without `method` accepts every HTTP verb, including state-changing ones from cross-site requests. Use @GetMapping/@PostMapping or set `method`.",
});

impl Rule for RequestMappingWithoutMethod {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        for a in cx.fm.annotations_of(n) {
            if a.simple_name() == "RequestMapping" && a.arg("method").is_none() {
                if let Some(nm) = n.field("name") {
                    cx.report(nm, "@RequestMapping without `method` accepts every HTTP method");
                }
            }
        }
    }
}

declare_rule!(RedundantResponseBody {
    id: "redundant-response-body",
    name: "@ResponseBody in a @RestController",
    category: Style,
    severity: Info,
    sources: ["sonar:S6837"],
    kinds: ["method_declaration"],
    description: "@RestController already implies @ResponseBody on every handler method.",
});

impl Rule for RedundantResponseBody {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !annotation_names(n, cx.src).contains(&"ResponseBody") {
            return;
        }
        if owning_type_decl(n).is_some_and(|d| annotation_names(d, cx.src).contains(&"RestController")) {
            if let Some(nm) = n.field("name") {
                cx.report(nm, "@ResponseBody is redundant in a @RestController");
            }
        }
    }
}

declare_rule!(CircularDependency {
    id: "circular-constructor-dependency",
    name: "Circular constructor dependency",
    category: Framework,
    severity: Error,
    sources: ["spring:BeanCurrentlyInCreationException", "intellij:SpringJavaInjectionPointsAutowiringInspection"],
    kinds: ["<file>"],
    description: "Beans that depend on each other through constructors cannot be created (BeanCurrentlyInCreationException / CircularDependencyException). Break the cycle by extracting the shared logic or injecting lazily.",
});

impl Rule for CircularDependency {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        let Some(di) = cx.di else { return };
        let classes = file_classes(cx);
        if classes.is_empty() || di.points.is_empty() {
            return;
        }
        let edges = &di.ctor_edges;
        let mut adj: rustc_hash::FxHashMap<&str, Vec<(&str, usize)>> = Default::default();
        for (a, b, pi) in edges.iter() {
            adj.entry(a).or_default().push((b, *pi));
        }
        for (decl, name) in &classes {
            // DFS from this class looking for a path back to it
            let start: &str = name;
            let mut stack: Vec<(&str, Vec<&str>)> = vec![(start, vec![start])];
            let mut seen = rustc_hash::FxHashSet::default();
            let mut found: Option<Vec<&str>> = None;
            while let Some((cur, path)) = stack.pop() {
                for (next, _) in adj.get(cur).map(|v| v.as_slice()).unwrap_or(&[]) {
                    if *next == start {
                        let mut p = path.clone();
                        p.push(start);
                        found = Some(p);
                        break;
                    }
                    if seen.insert(*next) && path.len() < 8 {
                        let mut p = path.clone();
                        p.push(next);
                        stack.push((next, p));
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            if let Some(cycle) = found {
                if let Some(nm) = decl.field("name") {
                    let chain: Vec<&str> = cycle.iter().map(|c| simple_name_of(c)).collect();
                    cx.report(nm, format!("Circular constructor dependency: {}", chain.join(" -> ")));
                }
            }
        }
    }
}

fn check_placeholder(v: &str) -> Option<&'static str> {
    let b = v.as_bytes();
    let mut i = 0;
    let mut depth = 0i32;
    while i < b.len() {
        if (b[i] == b'$' || b[i] == b'#') && i + 1 < b.len() && b[i + 1] == b'{' {
            depth += 1;
            i += 2;
            continue;
        }
        if b[i] == b'}' && depth > 0 {
            depth -= 1;
        }
        i += 1;
    }
    if depth > 0 {
        return Some("unclosed `${`/`#{` placeholder");
    }
    if v.contains("${}") {
        return Some("empty placeholder `${}`");
    }
    // "$key" without braces is a literal string, often a mistake
    if v.starts_with('$') && !v.starts_with("${") && v.len() > 1 && v.as_bytes()[1].is_ascii_alphabetic() {
        return Some("`$name` is a literal; use `${name}` for a property placeholder");
    }
    None
}

declare_rule!(InvalidPlaceholder {
    id: "invalid-value-placeholder",
    name: "Malformed @Value placeholder",
    category: Framework,
    severity: Error,
    tier: Edit,
    sources: ["intellij:SpringPlaceholdersInspection", "sonar:S6857"],
    kinds: ["annotation"],
    description: "`@Value(\"${server.port\")` (unclosed) or `@Value(\"$server.port\")` (missing braces) injects the literal text or fails to start.",
});

impl Rule for InvalidPlaceholder {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let name = n.field("name").map(|x| cx.text(x)).unwrap_or("");
        if !matches!(name.rsplit('.').next(), Some("Value" | "Scheduled" | "Property")) {
            return;
        }
        // judge whole values: a literal inside a concatenation
        // (`"${" + KEY + "}"`) is only a fragment; the concatenation is
        // checked when it folds to a constant and skipped otherwise
        let in_concat = |d: Node<'t>| d.parent().is_some_and(|p| p.kind() == "binary_expression");
        for v in n.descendants().filter(|d| matches!(d.kind(), "string_literal" | "binary_expression") && !in_concat(*d)) {
            let text = match cx.fm.constant_value(v) {
                Some(grounds_index::Constant::Str(s)) => s.to_string(),
                _ => continue,
            };
            if let Some(problem) = check_placeholder(&text) {
                cx.report(v, format!("Malformed placeholder: {problem}"));
            }
        }
    }
}

declare_rule!(BeanNameConflict {
    id: "bean-name-conflict",
    name: "Two beans with the same name",
    category: Framework,
    severity: Error,
    sources: ["spring:ConflictingBeanDefinitionException", "intellij:SpringJavaBeanNameConflict"],
    kinds: ["<file>"],
    description: "Two components or @Bean methods resolve to the same bean name (e.g. `com.a.UserService` and `com.b.UserService` both become `userService`). Spring Boot fails at startup unless bean overriding is enabled. Give one an explicit name.",
});

impl Rule for BeanNameConflict {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        let Some(di) = cx.di else { return };
        let path: &str = &cx.fm.path;
        for (a, b) in di.name_conflicts() {
            let (ba, bb) = (&di.beans[a], &di.beans[b]);
            if ba.declaring_class == bb.declaring_class && matches!((&ba.source, &bb.source), (BeanSource::FactoryMethod(_), BeanSource::FactoryMethod(_))) {
                continue; // overloaded @Bean methods in one class are merged by Spring
            }
            // with overriding enabled only two scanned components still clash
            if di.bean_overriding_allowed && !matches!((&ba.source, &bb.source), (BeanSource::Component, BeanSource::Component)) {
                continue;
            }
            // an @Bean method may replace a scanned component of the same class
            // (Spring 6.1+, even with overriding disabled)
            let same_class_override = |x: &grounds_di::Bean, y: &grounds_di::Bean| {
                x.source == BeanSource::Component && matches!(y.source, BeanSource::FactoryMethod(_)) && y.ty.class_name() == Some(&*x.declaring_class)
            };
            if same_class_override(ba, bb) || same_class_override(bb, ba) {
                continue;
            }
            for (this, other) in [(ba, bb), (bb, ba)] {
                if this.file.as_deref() != Some(path) {
                    continue;
                }
                for (decl, name) in file_classes(cx) {
                    if name == this.declaring_class {
                        let node = match &this.source {
                            BeanSource::FactoryMethod(m) => methods_of(decl).into_iter().find(|x| method_name(*x, cx.src) == &**m).and_then(|x| x.field("name")),
                            _ => decl.field("name"),
                        };
                        if let Some(nd) = node {
                            // the classes often share a simple name (`a.Exporter`, `b.Exporter`)
                            let by = match &other.source {
                                BeanSource::FactoryMethod(m) => format!("@Bean {}.{}()", other.declaring_class, m),
                                _ => other.declaring_class.to_string(),
                            };
                            let fails = if matches!((&this.source, &other.source), (BeanSource::Component, BeanSource::Component)) {
                                "Spring fails at startup (ConflictingBeanDefinitionException)"
                            } else {
                                "Spring Boot fails at startup (BeanDefinitionOverrideException) unless bean overriding is enabled"
                            };
                            cx.report(nd, format!("Bean name '{}' is also used by {by}; {fails}. Give one an explicit name", this.name));
                        }
                    }
                }
            }
        }
    }
}

declare_rule!(FinalConfigurationClass {
    id: "final-configuration-class",
    name: "@Configuration class or @Bean method cannot be proxied",
    category: Framework,
    severity: Error,
    tier: Edit,
    sources: ["intellij:SpringFacetCodeInspection", "spring:BeanDefinitionParsingException"],
    kinds: ["class_declaration", "method_declaration"],
    description: "Spring subclasses proxied @Configuration classes with CGLIB: a final configuration class, or a final/private @Bean method in it, fails at startup.",
});

impl Rule for FinalConfigurationClass {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let cfg_proxied = |decl: Node<'t>| {
            cx.fm.annotations_of(decl).iter().any(|a| a.simple_name() == "Configuration" && (a.name.starts_with("org.springframework") || !a.resolved) && a.arg("proxyBeanMethods").and_then(|v| v.as_bool()) != Some(false))
        };
        if n.kind() == "class_declaration" {
            if n.has_modifier("final") && cfg_proxied(n) {
                if let Some(nm) = n.field("name") {
                    cx.report(nm, "@Configuration classes must not be final (use proxyBeanMethods = false to opt out of proxying)");
                }
            }
            return;
        }
        if !annotation_names(n, cx.src).contains(&"Bean") || n.has_modifier("static") {
            return;
        }
        if (n.has_modifier("final") || n.has_modifier("private")) && owning_type_decl(n).is_some_and(cfg_proxied) {
            if let Some(nm) = n.field("name") {
                cx.report(nm, "@Bean methods in a proxied @Configuration must not be final or private");
            }
        }
    }
}

declare_rule!(InjectionInNonBean {
    id: "injection-in-non-bean",
    name: "Injection annotations in a class that is not a bean",
    category: Framework,
    severity: Warning,
    scope: Main,
    sources: ["intellij:SpringJavaAutowiredMembersInspection", "sonar:S3749"],
    kinds: ["class_declaration"],
    description: "@Autowired/@Inject/@Value only work on objects created by the container. In a class that is not a component (and not returned by a @Bean method) these fields stay null.",
});

impl Rule for InjectionInNonBean {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(di) = cx.di else { return };
        if di.frameworks.is_empty() || n.has_modifier("abstract") {
            return;
        }
        let Some(name) = cx.fm.class_name_of_decl(n).cloned() else { return };
        if di.bean_of_class(&name).is_some() {
            return;
        }
        // provided by a @Bean method or used as a configuration/test class
        if di.beans.iter().any(|b| b.ty.class_name() == Some(&*name)) {
            return;
        }
        let cls_anns = annotation_names(n, cx.src);
        if !cls_anns.is_empty() {
            // any class-level annotation might be a stereotype we cannot resolve
            return;
        }
        let injected: Vec<Node> = members(n)
            .into_iter()
            .filter(|m| matches!(m.kind(), "field_declaration" | "method_declaration"))
            .filter(|m| annotation_names(*m, cx.src).iter().any(|a| matches!(*a, "Autowired" | "Inject" | "Value")))
            .collect();
        if let Some(first) = injected.first() {
            // registered programmatically (`ctx.registerBean(Sample.class)`,
            // `new AnnotationConfigApplicationContext(Sample.class)`)
            let simple = simple_name_of(&name);
            let registered = cx.fm.root().descendants().any(|d| {
                if d.kind() != "class_literal" || d.first_kid().is_none_or(|t| type_simple_name(t, cx.src) != simple) {
                    return false;
                }
                let Some(call) = d.parent().filter(|p| p.kind() == "argument_list").and_then(|p| p.parent()) else { return false };
                match call.kind() {
                    "method_invocation" => matches!(cx.fm.call_name(call), Some("register" | "registerBean" | "run" | "sources" | "child" | "parent")),
                    "object_creation_expression" => call.field("type").is_some_and(|t| {
                        let t = type_simple_name(t, cx.src);
                        t.ends_with("ApplicationContext") || t.starts_with("SpringApplication")
                    }),
                    _ => false,
                }
            });
            if registered {
                return;
            }
            // subclasses may be beans
            if cx.fm.index.direct_source_subclasses(&name).is_empty() {
                cx.report(*first, format!("`{}` is not a bean, so this injection never happens", simple_name_of(&name)));
            }
        }
    }
}
