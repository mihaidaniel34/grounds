//! Dependency-injection model: which beans exist, what they provide, and how
//! each injection point resolves - without running the application.
//!
//! The model is framework-neutral (beans, qualifiers, primaries, conditions,
//! injection points) with framework-specific discovery for Spring, Micronaut
//! and Jakarta/JSR-330. It is built from the declaration index only, so it
//! sees exactly what the compiler sees, including Lombok-generated
//! constructors.

use grounds_index::*;
use rustc_hash::FxHashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framework {
    Spring,
    Micronaut,
    Jakarta,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeanSource {
    /// A component class found by scanning.
    Component,
    /// A `@Bean` (or factory) method; holds the method name.
    FactoryMethod(Name),
    /// A Spring Data / Micronaut Data repository interface.
    Repository,
    /// A declarative client interface (@FeignClient, Micronaut @Client).
    Client,
    /// MapStruct mapper with a DI component model.
    Mapper,
}

#[derive(Clone, Debug)]
pub struct Bean {
    pub name: String,
    pub aliases: Vec<String>,
    /// Provided type.
    pub ty: Ty,
    /// Class declaring the bean (component class or factory/configuration).
    pub declaring_class: Name,
    pub source: BeanSource,
    /// Qualifier values (`@Qualifier("x")`, `@Named("x")`) and custom
    /// qualifier annotation names.
    pub qualifiers: Vec<String>,
    pub primary: bool,
    /// Has @Profile/@Conditional*/@Requires: may be absent at runtime.
    pub conditional: bool,
    pub file: Option<Arc<str>>,
    pub is_test: bool,
    pub autowire_candidate: bool,
    pub framework: Framework,
    /// Names come from configuration (@EachBean/@EachProperty factories):
    /// any qualifier may match at runtime.
    pub dynamic_names: bool,
    /// Declares @Replaces (Micronaut): replacement semantics we do not model fully.
    pub replaces: bool,
    /// Made by a static factory method, which has no factory bean (Spring
    /// does not count it as a self-reference of its configuration).
    pub static_factory: bool,
}

/// Module directory of a source path (the part before `/src/`).
pub fn module_of(path: &str) -> &str {
    match path.find("/src/") {
        Some(i) => &path[..i],
        None => "",
    }
}

impl Bean {
    pub fn display(&self) -> String {
        match &self.source {
            BeanSource::FactoryMethod(m) => format!("'{}' ({}.{}())", self.name, simple_name_of(&self.declaring_class), m),
            _ => format!("'{}' ({})", self.name, simple_name_of(&self.declaring_class)),
        }
    }
    pub fn matches_qualifier(&self, q: &str) -> bool {
        self.name == q || self.aliases.iter().any(|a| a == q) || self.qualifiers.iter().any(|x| x == q)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PointKind {
    ConstructorParam(usize),
    Field,
    MethodParam(Name, usize),
    FactoryParam(Name, usize),
}

#[derive(Clone, Debug)]
pub struct InjectionPoint {
    /// Class that receives the injection.
    pub owner: Name,
    pub kind: PointKind,
    /// The element type requested (unwrapped from Optional/Provider/List...).
    pub ty: Ty,
    pub declared_ty: Ty,
    pub qualifier: Option<String>,
    /// Parameter or field name (Spring falls back to matching bean names).
    pub name: String,
    pub optional: bool,
    pub multiple: bool,
    pub lazy: bool,
    /// Name range of the field / factory or constructor method.
    pub range: Option<(u32, u32)>,
    pub framework: Framework,
    pub file: Option<Arc<str>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Unique(usize),
    /// Collection injection: all matching beans.
    Multiple(Vec<usize>),
    /// Optional dependency without a candidate.
    Absent,
    /// Nothing provides this type.
    NotFound,
    Ambiguous(Vec<usize>),
    QualifierNotFound { qualifier: String, candidates: Vec<usize> },
    /// The model cannot decide (unknown types, external providers, ...).
    Unknown,
}

/// One Spring application context, as far as the model can be certain of
/// it. Spring builds a context from the classes handed to it
/// (`SpringApplication.run(App.class)`, `new
/// AnnotationConfigApplicationContext(Config.class)`, a servlet
/// initializer's sources), plus what they import and scan, not from a
/// module or a package. A class is a member when it is the root, is
/// named by an `@Import` of a member, or is a component that a scan
/// declared by a member covers, in the root's module or a module the
/// root's module certainly depends on (it names one of that module's
/// classes in an import). Scans with exclude filters, scans of
/// conditional classes and scans of modules the root may not see make
/// nobody a member: what they hold is unknown.
#[derive(Clone, Debug, Default)]
pub struct AppContext {
    /// The class the context is started from (registered before scanning).
    pub root: Name,
    /// Main-source classes certainly registered in it.
    pub members: rustc_hash::FxHashSet<Name>,
    /// Test-source components of the root's module its scans pick up when
    /// a test starts it (`@SpringBootTest`); test configurations excluded.
    pub test_members: rustc_hash::FxHashSet<Name>,
}

impl AppContext {
    /// Is the class registered in this context (in a test run with `test`)?
    pub fn holds(&self, class: &str, test: bool) -> bool {
        self.members.contains(class) || (test && self.test_members.contains(class))
    }
}

#[derive(Debug, Default)]
pub struct DiModel {
    pub beans: Vec<Bean>,
    pub points: Vec<InjectionPoint>,
    by_class: FxHashMap<Name, Vec<usize>>,
    pub frameworks: Vec<Framework>,
    pub scan_roots: Vec<String>,
    /// The main sources define a Spring application: some application
    /// context is started from a class the model can name (see
    /// `AppContext`). Wiring is judged only in the classes those contexts
    /// certainly hold; elsewhere the code is a library, or belongs to a
    /// context assembled by tests or by code the model cannot read, which
    /// may provide any bean.
    pub spring_application: bool,
    /// The application contexts started in main sources, each from its root.
    pub contexts: Vec<AppContext>,
    /// Auto-configurations: annotated `@AutoConfiguration` or listed in
    /// META-INF registration files. `@SpringBootApplication`'s scan skips
    /// them (AutoConfigurationExcludeFilter), so its packages covering one
    /// (a nested plugin project sharing the application's base package) do not make it
    /// application code.
    pub auto_configurations: rustc_hash::FxHashSet<Name>,
    /// simple names referenced by classes that register beans
    /// programmatically (BeanDefinitionRegistryPostProcessor,
    /// ImportBeanDefinitionRegistrar): such types may have beans we cannot see
    pub dynamic_types: rustc_hash::FxHashSet<String>,
    /// lombok.config copies @Qualifier/@Value onto generated constructors.
    pub lombok_copies_qualifiers: bool,
    /// `spring.main.allow-bean-definition-overriding=true` in some
    /// application config: a later @Bean definition replaces an earlier one.
    pub bean_overriding_allowed: bool,
    /// An imported ImportSelector/ImportBeanDefinitionRegistrar may register
    /// beans we cannot see: "no bean" is never certain.
    pub opaque_imports: bool,
    /// Classes that are components (by binary name), for quick lookup.
    pub component_classes: FxHashMap<Name, usize>,
    /// Class names defined in more than one source file (independent modules
    /// of one repository, each its own application): the index keeps one of
    /// them, so wiring that involves such a class is not judged.
    pub duplicated: rustc_hash::FxHashSet<Name>,
    /// Types a copy the index left out may provide: the supertypes of the
    /// classes in files defining duplicated classes, and what their factory
    /// methods return (resolved by simple name, with their supertypes).
    pub duplicated_provides: rustc_hash::FxHashSet<Name>,
    /// Constructor-injection edges (owner, dependency class, point index),
    /// computed once at build time.
    pub ctor_edges: Vec<(Name, Name, usize)>,
    /// type name -> beans assignable to it (via the full supertype closure)
    providers: FxHashMap<Name, Vec<usize>>,
    /// beans whose supertype closure is incomplete (unknown library types)
    incomplete: Vec<usize>,
    /// classes with an @Inject constructor: just-in-time bindable in JSR-330 containers
    jit: Vec<Name>,
    /// A JSR-330 container or binding mechanism whose rules the model does
    /// not implement (Guice/Sisu modules, HK2 binders, Dagger, Maven 4 DI,
    /// Quarkus build-time beans), found in main sources: any type may be
    /// bound there, so no JSR-330 injection point is judged.
    pub unmodeled_container: Option<&'static str>,
    /// The same, found only in test sources: test injection points are not judged.
    pub unmodeled_container_in_tests: Option<&'static str>,
    /// Spring component beans and the `@Bean` methods of the same name in
    /// the same module. Spring keeps one definition per name: since 6.1 the
    /// `@Bean` method replaces a scanned class of its return type even with
    /// overriding disabled; otherwise the registration fails or, with
    /// overriding enabled, the later definition wins. Never two beans.
    same_name_factories: FxHashMap<usize, Vec<usize>>,
    /// Registrations the model saw but could not turn into beans, as the
    /// type each may provide and whether it is in test sources: classes
    /// and interfaces carrying an annotation the model cannot classify
    /// (`@Endpoint`, `@MessagingGateway`, a library's scanning annotation),
    /// classes named in an @Import the model did not follow, return types
    /// of factory methods outside any bean and of methods with such an
    /// annotation, and every interface when a @MapperScan pattern cannot be
    /// expanded. "No bean of this type" is only concluded when none of them
    /// may provide the type.
    unseen: Vec<(Name, bool)>,
    /// Spring XML bean definitions (@ImportResource, an XML application
    /// context) in main sources: any class may be a bean.
    pub xml_config: bool,
    /// The same, found only in test sources: test points only.
    pub xml_config_in_tests: bool,
    /// simple names referenced by test files that register beans in code
    /// (registerSingleton, registerBean): for test points only
    pub dynamic_types_in_tests: rustc_hash::FxHashSet<String>,
    /// Producers the model saw but could not turn into beans, as their
    /// declared return type and whether they are in test sources: the
    /// instance may be of any subtype (`Store store()` may return a
    /// `RedisStore`), so they may provide subtypes of that type too.
    unseen_producers: Vec<(Name, bool)>,
}

/// Imports that show wiring through a container or mechanism the model does
/// not implement, by package prefix. A Guice module, an HK2 binder or a
/// Dagger component binds types in code; Sisu and Maven 4's own DI add
/// index-based and bridged bindings; Quarkus extensions add synthetic beans
/// at build time (config mappings, REST and AI clients, recorder beans).
const UNMODELED_CONTAINERS: &[(&str, &str)] = &[
    ("com.google.inject.", "Guice"),
    ("org.eclipse.sisu.", "Sisu"),
    ("org.apache.maven.api.di.", "Maven DI"),
    ("org.glassfish.hk2.", "HK2"),
    ("org.glassfish.jersey.internal.inject.", "HK2"),
    ("dagger.", "Dagger"),
    ("io.quarkus.", "Quarkus"),
];

const SPRING_STEREOTYPES: &[&str] = &[
    "org.springframework.stereotype.Component",
    "org.springframework.stereotype.Service",
    "org.springframework.stereotype.Repository",
    "org.springframework.stereotype.Controller",
    "org.springframework.web.bind.annotation.RestController",
    "org.springframework.web.bind.annotation.ControllerAdvice",
    "org.springframework.web.bind.annotation.RestControllerAdvice",
    "org.springframework.context.annotation.Configuration",
    "org.springframework.boot.test.context.TestConfiguration",
    "org.springframework.boot.autoconfigure.SpringBootApplication",
    "org.springframework.boot.SpringBootConfiguration",
    "org.springframework.boot.context.properties.ConfigurationProperties",
];

const MICRONAUT_BEAN_ANNOTATIONS: &[&str] = &[
    "jakarta.inject.Singleton",
    "javax.inject.Singleton",
    "io.micronaut.context.annotation.Bean",
    "io.micronaut.context.annotation.Prototype",
    "io.micronaut.context.annotation.Context",
    "io.micronaut.context.annotation.Infrastructure",
    "io.micronaut.context.annotation.Factory",
    "io.micronaut.http.annotation.Controller",
    "io.micronaut.runtime.context.scope.ThreadLocal",
    "io.micronaut.runtime.context.scope.Refreshable",
    "io.micronaut.runtime.http.scope.RequestScope",
    "io.micronaut.context.annotation.ConfigurationProperties",
    "io.micronaut.context.annotation.EachProperty",
];

/// CDI bean-defining annotations: the built-in scopes, the Faces and
/// transaction scopes (normal scopes), and the stereotypes @Model and
/// OmniFaces's own. Project annotations meta-annotated @NormalScope or
/// @Stereotype are found through their meta-annotations.
const JAKARTA_SCOPES: &[&str] = &[
    "jakarta.enterprise.context.ApplicationScoped",
    "jakarta.enterprise.context.RequestScoped",
    "jakarta.enterprise.context.SessionScoped",
    "jakarta.enterprise.context.ConversationScoped",
    "jakarta.enterprise.context.Dependent",
    "jakarta.enterprise.inject.Model",
    "javax.enterprise.context.ApplicationScoped",
    "javax.enterprise.context.RequestScoped",
    "javax.enterprise.context.SessionScoped",
    "javax.enterprise.context.ConversationScoped",
    "javax.enterprise.context.Dependent",
    "javax.enterprise.inject.Model",
    "jakarta.faces.view.ViewScoped",
    "javax.faces.view.ViewScoped",
    "jakarta.faces.flow.FlowScoped",
    "javax.faces.flow.FlowScoped",
    "jakarta.transaction.TransactionScoped",
    "javax.transaction.TransactionScoped",
    "org.omnifaces.cdi.ViewScoped",
    "org.omnifaces.cdi.Startup",
    "org.omnifaces.cdi.GraphicImageBean",
];

/// Meta-annotations that make an annotation bean-defining in CDI.
const CDI_BEAN_DEFINING_META: &[&str] = &["jakarta.enterprise.context.NormalScope", "javax.enterprise.context.NormalScope", "jakarta.enterprise.inject.Stereotype", "javax.enterprise.inject.Stereotype"];

pub const QUALIFIER_ANNOTATIONS: &[&str] = &["org.springframework.beans.factory.annotation.Qualifier", "jakarta.inject.Named", "javax.inject.Named"];
pub const INJECT_ANNOTATIONS: &[&str] = &["org.springframework.beans.factory.annotation.Autowired", "jakarta.inject.Inject", "javax.inject.Inject", "jakarta.annotation.Resource", "javax.annotation.Resource"];
const BEAN_METHOD_ANNOTATIONS: &[&str] = &["org.springframework.context.annotation.Bean", "io.micronaut.context.annotation.Bean", "jakarta.inject.Singleton", "javax.inject.Singleton", "io.micronaut.context.annotation.Prototype", "io.micronaut.context.annotation.Context", "jakarta.enterprise.inject.Produces", "javax.enterprise.inject.Produces"];
const CONDITIONAL_PREFIXES: &[&str] = &["Profile", "Conditional", "Requires", "Requirements", "IfBuildProperty", "LookupIfProperty"];

/// Is this class part of a DI framework itself (not user code that happens to
/// live under a framework's namespace, like Spring's sample applications)?
pub fn is_framework_class(n: &str) -> bool {
    const FRAMEWORK_PACKAGES: &[&str] = &["io.micronaut.", "org.springframework.", "jakarta.", "javax.", "com.google.inject.", "io.quarkus."];
    FRAMEWORK_PACKAGES.iter().any(|p| n.starts_with(p)) && !n.starts_with("org.springframework.samples.") && !n.starts_with("io.micronaut.docs.")
}

/// Introspector.decapitalize semantics used by Spring for bean names.
pub fn decapitalize(s: &str) -> String {
    let mut c = s.chars();
    let (Some(a), b) = (c.next(), c.next()) else { return s.to_string() };
    if a.is_uppercase() && b.is_some_and(|b| b.is_uppercase()) {
        return s.to_string();
    }
    a.to_lowercase().collect::<String>() + &s[a.len_utf8()..]
}

use grounds_index::sources::{is_test_code, is_test_path};

/// `public static void main(String[])`, the entry point of an application.
fn has_main(c: &ClassInfo) -> bool {
    c.methods_named("main").any(|m| m.flags.is_static() && m.flags.is_public() && m.ret == Ty::Void && matches!(m.params.as_slice(), [p] if matches!(&p.ty, Ty::Array(e) if e.is_string())))
}

/// MapStruct's `@Mapper`, or an unresolved `@Mapper` (the library is not on
/// the classpath; a project-defined annotation would resolve).
pub fn is_mapstruct_mapper(a: &AnnotationInfo) -> bool {
    &*a.name == "org.mapstruct.Mapper" || (!a.resolved && &*a.name == "Mapper")
}

/// Whether a `@Mapper` annotation yields a bean: a MapStruct mapper whose
/// generated `*Impl` is registered by the DI container, or a MyBatis mapper.
/// Only an explicit `componentModel = "default"` (or
/// `MappingConstants.ComponentModel.DEFAULT`) rules it out; a missing model
/// may come from the `mapstruct.defaultComponentModel` compiler option.
pub fn mapper_is_bean(a: &AnnotationInfo) -> bool {
    let model = first_string(a, &["componentModel"]).unwrap_or_default();
    let explicit_default = model == "default" || a.arg("componentModel").is_some_and(|v| matches!(v, AnnotationValue::Expr(e) if names_default_constant(e)));
    !explicit_default
}

/// An unevaluated expression that is a plain reference to a constant named
/// `DEFAULT` (`ComponentModel.DEFAULT`, a static import, parenthesized or
/// not). Anything else, such as `DEFAULT_SPRING` or a conditional, is not
/// known to select the default model.
fn names_default_constant(e: &str) -> bool {
    let e: String = e.chars().filter(|c| !c.is_whitespace()).collect();
    let e = e.trim_matches(|c| c == '(' || c == ')');
    e.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$' || c == '.') && e.rsplit('.').next() == Some("DEFAULT")
}

/// Does a scan of base package `pattern` cover package `pkg`? Spring (and
/// MyBatis's @MapperScan, which uses Spring's scanner) looks for
/// `<pattern as a path>/**/*.class` with Ant-style matching: `*` and `?`
/// match within one segment, `**` matches any number of segments
/// (`com.shop.**.dao` covers `com.shop.order.dao.impl`). A pattern with a
/// placeholder left in it covers nothing we can name.
pub fn package_covered(pattern: &str, pkg: &str) -> bool {
    fn segment(p: &[u8], s: &[u8]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((b'*', rest)) => (0..=s.len()).any(|k| segment(rest, &s[k..])),
            Some((b'?', rest)) => !s.is_empty() && segment(rest, &s[1..]),
            Some((c, rest)) => s.first() == Some(c) && segment(rest, &s[1..]),
        }
    }
    fn segments(p: &[&str], s: &[&str]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((&"**", rest)) => (0..=s.len()).any(|k| segments(rest, &s[k..])),
            Some((x, rest)) => s.first().is_some_and(|y| segment(x.as_bytes(), y.as_bytes())) && segments(rest, &s[1..]),
        }
    }
    if pattern.contains("${") {
        return false;
    }
    if !pattern.contains(['*', '?']) {
        return pattern.is_empty() || pkg == pattern || pkg.strip_prefix(pattern).is_some_and(|rest| rest.starts_with('.'));
    }
    let mut p: Vec<&str> = pattern.split('.').filter(|x| !x.is_empty()).collect();
    p.push("**");
    let s: Vec<&str> = pkg.split('.').filter(|x| !x.is_empty()).collect();
    segments(&p, &s)
}

/// Base package values as Spring reads them: each may hold several
/// packages separated by commas, semicolons or whitespace.
fn split_packages(values: Vec<String>) -> Vec<String> {
    values.iter().flat_map(|v| v.split([',', ';', ' ', '\t', '\n'])).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect()
}

/// The packages a `@ComponentScan` or `@SpringBootApplication` on `c`
/// scans: the ones it names, the packages of the classes it names, or
/// else `c`'s own package.
fn scan_packages(c: &ClassInfo, a: &AnnotationInfo) -> Vec<String> {
    let explicit: Vec<String> = split_packages(["scanBasePackages", "basePackages", "value"].iter().flat_map(|k| a.arg(k).map(|v| v.strings()).unwrap_or_default()).collect());
    let classes: Vec<String> = ["scanBasePackageClasses", "basePackageClasses"]
        .iter()
        .flat_map(|k| a.arg(k).map(|v| v.values().into_iter().filter_map(|x| if let AnnotationValue::Class(t) = x { t.class_name().map(|c| package_of(c).to_string()) } else { None }).collect::<Vec<_>>()).unwrap_or_default())
        .collect();
    if explicit.is_empty() && classes.is_empty() { vec![c.package().to_string()] } else { explicit.into_iter().chain(classes).collect() }
}

/// Annotations known not to register what they annotate as a bean, by
/// package (every annotation in it) or by name. Stereotypes and bean
/// annotations are checked before this list, so a package listed here
/// may also hold some (Spring's web annotations hold @RestController).
fn inert_annotation(q: &str) -> bool {
    const PACKAGES: &[&str] = &[
        "java.", "kotlin.", "lombok.", "com.fasterxml.jackson.", "com.google.gson.annotations.", "jakarta.json.bind.annotation.", "javax.json.bind.annotation.",
        "jakarta.xml.bind.", "javax.xml.bind.", "jakarta.persistence.", "javax.persistence.", "jakarta.validation.", "javax.validation.", "org.hibernate.annotations.",
        "org.hibernate.validator.", "io.swagger.", "springfox.", "org.springdoc.", "org.jetbrains.annotations.", "org.checkerframework.", "edu.umd.cs.findbugs.",
        "com.google.errorprone.annotations.", "org.jspecify.", "javax.annotation.processing.", "org.immutables.", "com.google.auto.value.", "org.junit.", "org.mockito.",
        "org.assertj.", "org.testcontainers.", "com.baomidou.mybatisplus.annotation.", "com.alibaba.fastjson.", "com.alibaba.fastjson2.", "com.alibaba.excel.annotation.",
        "org.springframework.lang.", "org.springframework.format.annotation.", "org.springframework.data.annotation.", "org.springframework.web.bind.annotation.",
        "org.springframework.transaction.annotation.", "org.springframework.cache.annotation.", "org.springframework.scheduling.annotation.",
        "org.springframework.validation.annotation.", "org.springframework.security.access.", "org.springframework.jmx.export.annotation.",
        "org.springframework.retry.annotation.", "org.springframework.messaging.handler.annotation.", "org.springframework.test.",
        "org.springframework.boot.autoconfigure.condition.", "org.springframework.boot.context.properties.bind.", "io.micronaut.core.annotation.",
        "io.micronaut.serde.annotation.", "io.micronaut.validation.", "io.micronaut.data.annotation.", "io.micronaut.http.annotation.",
        "jakarta.interceptor.", "javax.interceptor.", "jakarta.decorator.", "javax.decorator.", "jakarta.annotation.", "javax.annotation.",
    ];
    const EXCEPT: &[&str] = &[
        "javax.annotation.ManagedBean", "jakarta.annotation.ManagedBean", "io.micronaut.data.annotation.Repository", "io.micronaut.http.annotation.Filter",
        "io.micronaut.http.annotation.ServerFilter", "io.micronaut.http.annotation.ClientFilter",
    ];
    const NAMES: &[&str] = &[
        "org.springframework.context.annotation.Primary", "org.springframework.context.annotation.Lazy", "org.springframework.context.annotation.Scope",
        "org.springframework.context.annotation.DependsOn", "org.springframework.context.annotation.Description", "org.springframework.context.annotation.Role",
        "org.springframework.context.annotation.Import", "org.springframework.context.annotation.PropertySource", "org.springframework.context.annotation.PropertySources",
        "org.springframework.context.annotation.ComponentScan", "org.springframework.context.annotation.ComponentScans", "org.springframework.context.annotation.Fallback",
        "org.springframework.context.annotation.EnableAspectJAutoProxy", "org.springframework.context.annotation.ImportResource",
        "org.springframework.core.annotation.Order", "org.springframework.core.annotation.AliasFor", "org.springframework.stereotype.Indexed",
        "org.springframework.beans.factory.annotation.Autowired", "org.springframework.beans.factory.annotation.Value", "org.springframework.beans.factory.annotation.Lookup",
        "org.springframework.beans.factory.annotation.Configurable", "org.springframework.beans.factory.annotation.Required",
        "org.springframework.boot.context.properties.ConstructorBinding", "org.springframework.boot.context.properties.NestedConfigurationProperty",
        "org.springframework.boot.context.properties.EnableConfigurationProperties", "org.springframework.boot.context.properties.ConfigurationPropertiesScan",
        "org.springframework.boot.context.properties.DeprecatedConfigurationProperty", "org.springframework.boot.autoconfigure.EnableAutoConfiguration",
        "org.springframework.boot.autoconfigure.ImportAutoConfiguration", "org.springframework.boot.autoconfigure.AutoConfigureAfter",
        "org.springframework.boot.autoconfigure.AutoConfigureBefore", "org.springframework.boot.autoconfigure.AutoConfigureOrder",
        "org.springframework.boot.test.context.SpringBootTest", "org.springframework.boot.test.mock.mockito.MockBean", "org.springframework.boot.test.mock.mockito.SpyBean",
        "jakarta.inject.Inject", "javax.inject.Inject", "jakarta.enterprise.inject.Alternative", "javax.enterprise.inject.Alternative",
        "jakarta.enterprise.inject.Vetoed", "javax.enterprise.inject.Vetoed", "jakarta.enterprise.inject.Typed", "javax.enterprise.inject.Typed",
        "jakarta.enterprise.inject.Specializes", "javax.enterprise.inject.Specializes", "jakarta.enterprise.inject.Default", "javax.enterprise.inject.Default",
        "io.micronaut.context.annotation.Requires", "io.micronaut.context.annotation.Requirements", "io.micronaut.context.annotation.Secondary",
        "io.micronaut.context.annotation.Primary", "io.micronaut.context.annotation.Value", "io.micronaut.context.annotation.Property",
        "io.micronaut.context.annotation.Parameter", "io.micronaut.context.annotation.Executable", "io.micronaut.context.annotation.Replaces",
        "io.micronaut.context.annotation.DefaultImplementation", "io.micronaut.context.annotation.ConfigurationInject", "io.micronaut.context.annotation.Type",
    ];
    (PACKAGES.iter().any(|p| q.starts_with(p)) && !EXCEPT.contains(&q)) || NAMES.contains(&q) || (q.starts_with("org.springframework.") && simple_name_of(q).starts_with("Enable"))
}

fn first_string(a: &AnnotationInfo, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = a.arg(k) {
            if let Some(s) = v.strings().into_iter().find(|s| !s.is_empty()) {
                return Some(s);
            }
        }
    }
    None
}

struct Builder<'a> {
    index: &'a Index,
    model: DiModel,
    /// memo: annotation name -> is stereotype / qualifier
    stereo: FxHashMap<Name, Option<Framework>>,
    qualifier_ann: FxHashMap<Name, bool>,
    /// memo: annotation name -> makes a method a factory method
    factory_ann: FxHashMap<Name, bool>,
    /// (class, method) pairs that produce a bean
    produced: rustc_hash::FxHashSet<(Name, Name)>,
    /// project annotations named by scan filters (`includeFilters`,
    /// @MapperScan's annotationClass): they register what they annotate
    filter_annotations: rustc_hash::FxHashSet<String>,
    /// a @MapperScan the model could not expand (placeholders, @MapperScans,
    /// a MapperScannerConfigurer set up in code), in main sources and only
    /// in test sources
    unexpanded_mapper_scan: bool,
    unexpanded_mapper_scan_in_tests: bool,
    /// types named by ASSIGNABLE_TYPE include filters (registering their
    /// subtypes), and whether the filter is in test sources
    filter_types: Vec<(Name, bool)>,
    /// an include filter the model cannot evaluate (REGEX, ASPECTJ, CUSTOM)
    /// in main sources / only in test sources: any class may be registered
    opaque_filter: bool,
    opaque_filter_in_tests: bool,
    /// classes registered through META-INF files
    registered: rustc_hash::FxHashSet<String>,
    /// the auto-configurations among them
    auto_registered: rustc_hash::FxHashSet<String>,
}

impl<'a> Builder<'a> {
    /// Is the annotation (or one of its meta-annotations) a bean-defining one?
    fn stereotype(&mut self, ann: &AnnotationInfo, depth: u32) -> Option<Framework> {
        if let Some(r) = self.stereo.get(&ann.name) {
            return *r;
        }
        let n = &*ann.name;
        let direct = if SPRING_STEREOTYPES.iter().any(|s| ann.is(s)) {
            Some(Framework::Spring)
        } else if MICRONAUT_BEAN_ANNOTATIONS.iter().any(|s| ann.is(s)) {
            Some(if n.starts_with("io.micronaut") { Framework::Micronaut } else { Framework::Jakarta })
        } else if JAKARTA_SCOPES.iter().any(|s| ann.is(s)) {
            Some(Framework::Jakarta)
        } else {
            None
        };
        // unresolved annotations only match the table by simple name
        let r = direct.or_else(|| {
            if depth > 4 || !ann.resolved || n.starts_with("java.") {
                return None;
            }
            let c = self.index.class(n)?;
            if !c.is_interface() {
                return None;
            }
            let metas = c.annotations.clone();
            // a JSR-330 scope (Micronaut's own scopes are meta-annotated @Scope)
            // (an unresolved `@Scope` by the file's imports: it may be Spring's)
            let jsr330_scope = |m: &AnnotationInfo| match (m.resolved, &*m.name) {
                (true, name) => matches!(name, "jakarta.inject.Scope" | "javax.inject.Scope"),
                (false, "jakarta.inject.Scope" | "javax.inject.Scope") => true,
                (false, "Scope") => self.index.raw_class(n).is_some_and(|(stub, _)| {
                    let single: Vec<_> = stub.imports.iter().filter(|i| !i.is_static && !i.on_demand && i.is_named("Scope")).collect();
                    match single.as_slice() {
                        [] => stub.imports.iter().any(|i| !i.is_static && i.on_demand && matches!(&*i.path, "jakarta.inject" | "javax.inject")),
                        [i] => matches!(&*i.path, "jakarta.inject.Scope" | "javax.inject.Scope"),
                        _ => false,
                    }
                }),
                _ => false,
            };
            if metas.iter().any(jsr330_scope) {
                return Some(if n.starts_with("io.micronaut") { Framework::Micronaut } else { Framework::Jakarta });
            }
            if metas.iter().any(|m| CDI_BEAN_DEFINING_META.iter().any(|s| m.is(s))) {
                return Some(Framework::Jakarta);
            }
            metas.iter().find_map(|m| if &*m.name == n { None } else { self.stereotype(m, depth + 1) })
        });
        self.stereo.insert(ann.name.clone(), r);
        r
    }

    /// Does the annotation make a method a factory method: `@Bean`,
    /// `@Produces`, a JSR-330 scope, or a project annotation carrying one of
    /// them (`@QueueBean` meta-annotated `@Singleton @Bean`), as the
    /// containers read them through meta-annotations?
    fn factory_annotation(&mut self, ann: &AnnotationInfo, depth: u32) -> bool {
        if BEAN_METHOD_ANNOTATIONS.iter().any(|b| ann.is(b)) {
            return true;
        }
        if let Some(r) = self.factory_ann.get(&ann.name) {
            return *r;
        }
        let n = &*ann.name;
        let r = depth <= 4
            && ann.resolved
            && !n.starts_with("java.")
            && self.index.class(n).filter(|c| c.kind == ClassKind::Annotation).is_some_and(|c| {
                c.annotations.iter().any(|m| &*m.name != n && (m.is("jakarta.inject.Scope") || m.is("javax.inject.Scope") || self.factory_annotation(m, depth + 1)))
            });
        self.factory_ann.insert(ann.name.clone(), r);
        r
    }

    fn is_factory_method(&mut self, m: &MethodInfo) -> bool {
        !m.is_constructor() && m.annotations.iter().any(|a| self.factory_annotation(a, 0))
    }

    fn is_custom_qualifier(&mut self, ann: &AnnotationInfo) -> bool {
        if let Some(r) = self.qualifier_ann.get(&ann.name) {
            return *r;
        }
        let r = ann.resolved
            && self.index.class(&ann.name).is_some_and(|c| c.annotations.iter().any(|m| m.is("jakarta.inject.Qualifier") || m.is("javax.inject.Qualifier") || m.is("org.springframework.beans.factory.annotation.Qualifier")));
        self.qualifier_ann.insert(ann.name.clone(), r);
        r
    }

    fn qualifiers_of(&mut self, anns: &[AnnotationInfo]) -> Vec<String> {
        let mut out = Vec::new();
        for a in anns {
            if QUALIFIER_ANNOTATIONS.iter().any(|q| a.is(q)) {
                if let Some(v) = first_string(a, &["value"]) {
                    out.push(v);
                }
            } else if self.is_custom_qualifier(a) {
                out.push(format!("@{}", a.name));
            }
        }
        out
    }

    fn injection_qualifier(&mut self, anns: &[AnnotationInfo]) -> Option<String> {
        for a in anns {
            if QUALIFIER_ANNOTATIONS.iter().any(|q| a.is(q)) {
                return first_string(a, &["value"]).or(Some(String::new()));
            }
            if a.is("jakarta.annotation.Resource") || a.is("javax.annotation.Resource") {
                if let Some(n) = first_string(a, &["name"]) {
                    return Some(n);
                }
            }
            if a.simple_name() == "Any" {
                return Some(String::new());
            }
            if self.is_custom_qualifier(a) {
                // qualifier annotations with members select by member values, which we do not model
                if !a.args.is_empty() {
                    return Some(String::new());
                }
                return Some(format!("@{}", a.name));
            }
        }
        None
    }

    fn conditional(anns: &[AnnotationInfo]) -> bool {
        anns.iter().any(|a| {
            let s = a.simple_name();
            CONDITIONAL_PREFIXES.iter().any(|p| s.starts_with(p))
        })
    }

    /// Conditional directly or through a composed annotation carrying a
    /// condition (`@TbCoreComponent` meta-annotated `@ConditionalOnExpression`).
    fn conditional_meta(&self, anns: &[AnnotationInfo]) -> bool {
        fn rec(idx: &Index, anns: &[AnnotationInfo], depth: u32) -> bool {
            Builder::conditional(anns)
                || (depth < 3
                    && anns.iter().any(|a| !a.name.starts_with("java.") && idx.class(&a.name).is_some_and(|c| c.kind == ClassKind::Annotation && rec(idx, &c.annotations, depth + 1))))
        }
        rec(self.index, anns, 0)
    }

    /// May this annotation register what it annotates in a way the model
    /// does not implement? Bean-defining annotations the model knows (they
    /// made a bean, or the model knows why not) and annotations known not
    /// to register anything are classified; everything else is not: a
    /// library annotation from a jar we do not have may be a stereotype
    /// through its meta-annotations, or a library may scan for it.
    fn unclassified(&mut self, a: &AnnotationInfo, stub: Option<&FileStub>, on_method: bool, depth: u32) -> bool {
        let defining = if on_method { self.factory_annotation(a, 0) } else { self.stereotype(a, 0).is_some() };
        if defining || Self::conditional(std::slice::from_ref(a)) {
            return false;
        }
        if QUALIFIER_ANNOTATIONS.iter().any(|q| a.is(q)) || self.is_custom_qualifier(a) {
            return false;
        }
        let qualified: Option<String> = if a.resolved || a.name.contains('.') {
            Some(a.name.to_string())
        } else {
            stub.and_then(|st| st.imports.iter().find(|i| !i.is_static && !i.on_demand && i.is_named(&a.name)).map(|i| i.path.to_string()))
        };
        let Some(q) = qualified else { return true };
        if self.filter_annotations.contains(&q) {
            return true;
        }
        // a project annotation: what its own annotations are
        if let Some(c) = self.index.class(&q).filter(|c| c.is_source() && c.kind == ClassKind::Annotation) {
            if depth > 4 {
                return true;
            }
            let meta_stub = self.index.raw_class(&q).map(|(st, _)| st.clone());
            let metas: Vec<AnnotationInfo> = c.annotations.iter().filter(|m| *m.name != *q && !m.name.starts_with("java.lang.annotation.")).cloned().collect();
            // bean-defining annotations it carries that did not make it
            // bean-defining where it is used (a class stereotype on a method
            // annotation): the container may read them all the same
            if metas.iter().any(|m| self.stereotype(m, depth + 1).is_some() || self.factory_annotation(m, depth + 1)) {
                return true;
            }
            return metas.iter().any(|m| self.unclassified(m, meta_stub.as_deref(), on_method, depth + 1));
        }
        !inert_annotation(&q)
    }

    /// Registrations the model could not turn into beans (see `DiModel::unseen`).
    fn unseen_registrations(&mut self, names: &[Name], imported_anywhere: &[Name]) {
        let mut unseen: Vec<(Name, bool)> = Vec::new();
        for n in imported_anywhere {
            if !self.model.component_classes.contains_key(n) {
                let is_test = self.index.source_file_of(n).is_some_and(|f| is_test_path(f));
                unseen.push((n.clone(), is_test));
            }
        }
        for n in names {
            let Some(c) = self.index.class(n) else { continue };
            let is_test = c.source_file().is_some_and(is_test_path);
            let stub = self.index.raw_class(n).map(|(st, _)| st.clone());
            let skip = c.flags.has(Flags::ANON) || c.kind == ClassKind::Annotation || c.kind == ClassKind::Enum || c.is_inner() || is_framework_class(n);
            if skip || self.model.component_classes.contains_key(n) {
                continue;
            }
            let anns = c.annotations.clone();
            let unknown = anns.iter().any(|a| self.unclassified(a, stub.as_deref(), false, 0));
            // a stereotype on a concrete class that is still no bean, for a
            // reason the model does not know
            let stereotyped = !c.is_abstract() && anns.iter().any(|a| self.stereotype(a, 0).is_some());
            // (a scan the model cannot read, declared in test sources,
            // registers for test points only)
            let main_scan = !is_test && (if c.is_interface() { self.unexpanded_mapper_scan } else { self.opaque_filter });
            let test_scan = if c.is_interface() { self.unexpanded_mapper_scan_in_tests } else { self.opaque_filter_in_tests };
            let concrete = !c.is_interface() && !c.is_abstract();
            let assignable: Vec<bool> = if concrete { self.filter_types.iter().filter(|(t, _)| self.index.is_subclass(n, t).is_yes()).map(|(_, t)| *t).collect() } else { Vec::new() };
            if unknown || stereotyped || main_scan || (!is_test && assignable.iter().any(|t| !t)) {
                unseen.push((n.clone(), is_test));
            } else if (test_scan && (c.is_interface() || concrete)) || !assignable.is_empty() {
                unseen.push((n.clone(), true));
            }
        }
        // producers: in a class that is or may be a bean, factory methods
        // that made no bean and methods with an annotation the model cannot
        // classify (a plain class's leftover @Bean method registers nothing)
        let mut producers: Vec<(Name, bool)> = Vec::new();
        for n in names {
            let Some(c) = self.index.class(n) else { continue };
            // in the scope the class is (or may be) registered in: a main
            // class registered only by a test's scan produces for tests only
            let is_test = if self.model.component_classes.contains_key(n) {
                c.source_file().is_some_and(is_test_path)
            } else {
                match unseen.iter().filter(|(u, _)| u == n).map(|(_, t)| *t).min() {
                    Some(t) => t,
                    None => continue,
                }
            };
            let stub = self.index.raw_class(n).map(|(st, _)| st.clone());
            for m in &c.methods {
                if m.is_constructor() || m.ret == Ty::Void || self.produced.contains(&(n.clone(), m.name.clone())) {
                    continue;
                }
                let Some(ret) = m.ret.class_name() else { continue };
                let anns = m.annotations.clone();
                if self.is_factory_method(m) || anns.iter().any(|a| self.unclassified(a, stub.as_deref(), true, 0)) {
                    producers.push((Arc::from(ret), is_test));
                }
            }
        }
        unseen.sort();
        unseen.dedup();
        producers.sort();
        producers.dedup();
        self.model.unseen = unseen;
        self.model.unseen_producers = producers;
    }

    fn add_bean(&mut self, b: Bean) -> usize {
        let i = self.model.beans.len();
        self.model.beans.push(b);
        i
    }

    fn scan(&mut self) {
        let names: Vec<Name> = self.index.source_class_names().cloned().collect();
        // frameworks in use and scan roots
        let mut roots = Vec::new();
        // (root package, declared on a conditional class such as @Profile,
        // module declaring it)
        let mut root_conditions: Vec<(String, bool, String)> = Vec::new();
        let mut spring = false;
        let mut micronaut = false;
        for (path, stub) in self.index.files() {
            // the frameworks' own sources (vendored, or test stubs) say nothing
            // about what the project uses
            if is_framework_class(&format!("{}.X", stub.package)) {
                continue;
            }
            for imp in &stub.imports {
                // (`import com.google.inject.*` keeps only the package)
                let imported = format!("{}.", imp.path);
                if let Some((_, name)) = UNMODELED_CONTAINERS.iter().find(|(prefix, _)| imported.starts_with(prefix)) {
                    // (the smallest name, whatever order the files come in)
                    let slot = if is_test_code(path) { &mut self.model.unmodeled_container_in_tests } else { &mut self.model.unmodeled_container };
                    if slot.is_none_or(|s| *name < s) {
                        *slot = Some(name);
                    }
                }
                if imp.path.starts_with("org.springframework") {
                    spring = true;
                }
                if imp.path.starts_with("io.micronaut") {
                    micronaut = true;
                }
            }
        }
        for n in &names {
            let Some(c) = self.index.class(n) else { continue };
            for a in &c.annotations {
                if a.is("org.springframework.boot.autoconfigure.SpringBootApplication") || a.is("org.springframework.context.annotation.ComponentScan") {
                    let declared = scan_packages(&c, a);
                    let cond = self.conditional_meta(&c.annotations);
                    let module = c.source_file().map(module_of).unwrap_or("").to_string();
                    root_conditions.extend(declared.iter().map(|r| (r.clone(), cond, module.clone())));
                    roots.extend(declared);
                }
            }
        }
        for n in &names {
            let Some(c) = self.index.class(n) else { continue };
            if c.annotations.iter().any(|a| a.is("org.springframework.boot.autoconfigure.AutoConfiguration") || (!a.resolved && a.simple_name() == "AutoConfiguration"))
                || self.auto_registered.contains(&**n)
                || self.auto_registered.contains(&n.replace('$', "."))
            {
                self.model.auto_configurations.insert(n.clone());
            }
        }
        if spring {
            self.model.frameworks.push(Framework::Spring);
        }
        if micronaut {
            self.model.frameworks.push(Framework::Micronaut);
        }
        self.model.scan_roots = roots.clone();

        let in_scan = |pkg: &str| roots.is_empty() || roots.iter().any(|r| package_covered(r, pkg));
        // MyBatis @MapperScan: every interface in these packages is a bean
        // (package pattern, declared in test sources: registers test beans)
        let mut mapper_pkgs: Vec<(String, bool)> = Vec::new();
        for n in &names {
            let Some(c) = self.index.class(n) else { continue };
            let in_tests = c.source_file().is_some_and(is_test_path);
            let mut unexpanded = false;
            for a in c.annotations.iter().filter(|a| a.simple_name() == "MapperScan") {
                let mut pkgs: Vec<String> = split_packages(["value", "basePackages"].iter().flat_map(|k| a.arg(k).map(|v| v.strings()).unwrap_or_default()).collect());
                if let Some(v) = a.arg("basePackageClasses") {
                    pkgs.extend(v.values().into_iter().filter_map(|x| if let AnnotationValue::Class(t) = x { t.class_name().map(|c| package_of(c).to_string()) } else { None }));
                }
                // without packages MyBatis scans the annotated class's package
                if pkgs.is_empty() {
                    if ["value", "basePackages", "basePackageClasses"].iter().any(|k| a.arg(k).is_some()) {
                        unexpanded = true;
                    } else {
                        pkgs.push(c.package().to_string());
                    }
                }
                mapper_pkgs.extend(pkgs.into_iter().map(|p| (p, in_tests)));
                if let Some(v) = a.arg("annotationClass") {
                    self.filter_annotations.extend(v.values().into_iter().filter_map(|x| if let AnnotationValue::Class(t) = x { t.class_name().map(str::to_string) } else { None }));
                }
            }
            if c.annotations.iter().any(|a| a.simple_name() == "MapperScans") {
                unexpanded = true;
            }
            if unexpanded {
                if in_tests { self.unexpanded_mapper_scan_in_tests = true } else { self.unexpanded_mapper_scan = true }
            }
            // include filters: ANNOTATION filters register what the
            // annotations they name annotate, ASSIGNABLE_TYPE filters the
            // subtypes of the types they name; others (REGEX, ASPECTJ,
            // CUSTOM) may register any class
            for a in c.annotations.iter().filter(|a| matches!(a.simple_name(), "ComponentScan" | "SpringBootApplication" | "ComponentScans")) {
                let mut filters: Vec<AnnotationInfo> = Vec::new();
                let mut stack: Vec<AnnotationValue> = a.args.iter().filter(|(k, _)| matches!(&**k, "includeFilters" | "value")).map(|(_, v)| v.clone()).collect();
                if a.simple_name() != "ComponentScans" {
                    stack = a.arg("includeFilters").cloned().into_iter().collect();
                }
                while let Some(v) = stack.pop() {
                    match v {
                        AnnotationValue::Annotation(n) if n.simple_name() == "Filter" => filters.push(*n),
                        AnnotationValue::Annotation(n) => stack.extend(n.arg("includeFilters").cloned()),
                        AnnotationValue::Array(xs) => stack.extend(xs),
                        AnnotationValue::Expr(_) => {
                            if in_tests { self.opaque_filter_in_tests = true } else { self.opaque_filter = true }
                        }
                        _ => {}
                    }
                }
                for f in filters {
                    let kind = match f.arg("type") {
                        None => Some("ANNOTATION".to_string()),
                        Some(AnnotationValue::Enum { constant, .. }) => Some(constant.to_string()),
                        Some(AnnotationValue::Expr(e)) => Some(e.rsplit('.').next().unwrap_or("").trim().to_string()),
                        Some(_) => None,
                    };
                    let classes: Vec<String> = ["classes", "value"].iter().flat_map(|k| f.arg(k).map(|v| v.values()).unwrap_or_default()).filter_map(|x| if let AnnotationValue::Class(t) = x { t.class_name().map(str::to_string) } else { None }).collect();
                    match kind.as_deref() {
                        Some("ANNOTATION") => self.filter_annotations.extend(classes),
                        Some("ASSIGNABLE_TYPE") => self.filter_types.extend(classes.into_iter().map(|t| (Arc::from(t.as_str()), in_tests))),
                        _ => {
                            if in_tests { self.opaque_filter_in_tests = true } else { self.opaque_filter = true }
                        }
                    }
                }
            }
        }
        // a pattern with a placeholder, or one that covers no package of the
        // project (a syntax we misread, or mappers in another repository)
        let packages: rustc_hash::FxHashSet<&str> = names.iter().map(|n| package_of(n)).collect();
        for (r, in_tests) in &mapper_pkgs {
            if r.contains("${") || !packages.iter().any(|p| package_covered(r, p)) {
                if *in_tests { self.unexpanded_mapper_scan_in_tests = true } else { self.unexpanded_mapper_scan = true }
            }
        }
        // None: not covered; Some(true): only by scans in test sources
        let in_mapper_scan = |pkg: &str| {
            let covering: Vec<bool> = mapper_pkgs.iter().filter(|(r, _)| package_covered(r, pkg)).map(|(_, t)| *t).collect();
            if covering.is_empty() { None } else { Some(covering.iter().all(|t| *t)) }
        };
        for n in &names {
            let registers = ["org.springframework.beans.factory.support.BeanDefinitionRegistryPostProcessor", "org.springframework.context.annotation.ImportBeanDefinitionRegistrar"].iter().any(|i| self.index.is_subclass(n, i).is_yes())
                || self.index.raw_class(n).is_some_and(|(_, r)| r.interfaces.iter().any(|t| t.last_name().is_some_and(|l| matches!(l, "BeanDefinitionRegistryPostProcessor" | "ImportBeanDefinitionRegistrar"))));
            if registers {
                if let Some(stub) = self.index.source_file_of(n).and_then(|f| self.index.file(f)) {
                    self.model.dynamic_types.extend(stub.referenced_names.iter().map(|r| r.to_string()));
                }
            }
        }
        // classes registered explicitly by scanned classes (@Import chains,
        // @EnableConfigurationProperties) are beans wherever they live
        let mut imported: rustc_hash::FxHashSet<Name> = rustc_hash::FxHashSet::default();
        // start from classes component scanning registers: an @Import on a
        // class that is never registered has no effect
        let mut work: Vec<Name> = Vec::new();
        for n in &names {
            let Some(c) = self.index.class(n) else { continue };
            if !in_scan(c.package()) {
                continue;
            }
            let anns = c.annotations.clone();
            if anns.iter().any(|a| a.simple_name() == "SpringBootApplication" || self.stereotype(a, 0) == Some(Framework::Spring)) {
                work.push(n.clone());
            }
        }
        while let Some(n) = work.pop() {
            let Some(c) = self.index.class(&n) else { continue };
            for a in &c.annotations {
                if !matches!(a.simple_name(), "Import" | "EnableConfigurationProperties") {
                    continue;
                }
                for v in a.arg("value").map(|v| v.values()).unwrap_or_default() {
                    if let AnnotationValue::Class(t) = v {
                        if let Some(t) = t.class_name() {
                            let t: Name = Arc::from(t);
                            if imported.insert(t.clone()) {
                                work.push(t);
                            }
                        }
                    }
                }
            }
        }
        // classes named by an @Import anywhere: those the walk from scanned
        // classes did not reach may still be registered (by a configuration
        // registered some other way)
        let mut imported_anywhere: Vec<Name> = Vec::new();
        for n in &names {
            let Some(c) = self.index.class(n) else { continue };
            for a in c.annotations.iter().filter(|a| matches!(a.simple_name(), "Import" | "EnableConfigurationProperties")) {
                for v in a.arg("value").map(|v| v.values()).unwrap_or_default() {
                    if let AnnotationValue::Class(t) = v {
                        imported_anywhere.extend(t.class_name().map(Arc::from));
                    }
                }
            }
            if c.annotations.iter().any(|a| a.simple_name() == "ImportResource") {
                if c.source_file().is_some_and(is_test_path) { self.model.xml_config_in_tests = true } else { self.model.xml_config = true }
            }
        }
        for (path, stub) in self.index.files() {
            if is_framework_class(&format!("{}.X", stub.package)) {
                continue;
            }
            let mentions = |n: &str| stub.referenced_names.binary_search_by(|r| (**r).cmp(n)).is_ok();
            // (evidence in test sources counts for test points only)
            let in_tests = is_test_path(path);
            if ["ClassPathXmlApplicationContext", "GenericXmlApplicationContext", "FileSystemXmlApplicationContext", "XmlBeanDefinitionReader", "XmlWebApplicationContext"].iter().any(|n| mentions(n)) {
                if in_tests { self.model.xml_config_in_tests = true } else { self.model.xml_config = true }
            }
            if mentions("MapperScannerConfigurer") {
                if in_tests { self.unexpanded_mapper_scan_in_tests = true } else { self.unexpanded_mapper_scan = true }
            }
            // registered in code: registerSingleton(Foo.class, ...),
            // registerBean(...), registerBeanDefinition(...)
            if ["registerSingleton", "registerBean", "registerBeanDefinition"].iter().any(|n| mentions(n)) {
                let target = if in_tests { &mut self.model.dynamic_types_in_tests } else { &mut self.model.dynamic_types };
                target.extend(stub.referenced_names.iter().map(|r| r.to_string()));
            }
        }
        // ImportSelectors and ImportBeanDefinitionRegistrars are not beans; the
        // beans they register are invisible to us
        let registers_dynamically = |idx: &Index, n: &str| {
            ["org.springframework.context.annotation.ImportSelector", "org.springframework.context.annotation.ImportBeanDefinitionRegistrar"].iter().any(|i| idx.is_subclass(n, i).is_yes())
                || idx.raw_class(n).is_some_and(|(_, r)| r.interfaces.iter().any(|t| t.last_name().is_some_and(|l| matches!(l, "ImportSelector" | "DeferredImportSelector" | "ImportBeanDefinitionRegistrar"))))
        };
        let dynamic: Vec<Name> = imported.iter().filter(|n| registers_dynamically(self.index, n)).cloned().collect();
        self.model.opaque_imports = !dynamic.is_empty();
        for n in dynamic {
            imported.remove(&n);
        }
        // anything else outside the scanned packages may still be registered
        // (AutoConfiguration.imports, ImportSelectors, manual registration):
        // modeled as conditional, i.e. it satisfies injections but never makes
        // them ambiguous or clashes by name
        let outside_scan = |c: &ClassInfo| !in_scan(c.package()) && !imported.contains(&c.name);

        for n in &names {
            let Some(c) = self.index.class(n) else { continue };
            let file = c.source_file().map(Arc::from);
            let is_test = file.as_deref().is_some_and(is_test_path);
            // repositories and clients are interfaces
            if c.is_interface() && c.kind != ClassKind::Annotation {
                let mut source = None;
                let no_repo = c.annotations.iter().any(|a| a.simple_name() == "NoRepositoryBean");
                if !no_repo && !is_framework_class(&c.name) {
                    if self.index.is_subclass(&c.name, "org.springframework.data.repository.Repository").is_yes() {
                        source = Some((BeanSource::Repository, Framework::Spring));
                    } else if let Some((stub, raw)) = self.index.raw_class(&c.name) {
                        // without the spring-data jars: recognize the well-known base interfaces
                        const DATA_BASES: &[&str] = &[
                            "Repository", "CrudRepository", "ListCrudRepository", "JpaRepository", "PagingAndSortingRepository", "ListPagingAndSortingRepository",
                            "MongoRepository", "ReactiveCrudRepository", "ReactiveMongoRepository", "R2dbcRepository", "ElasticsearchRepository", "Neo4jRepository",
                            "CassandraRepository", "KeyValueRepository", "RevisionRepository", "ReactiveSortingRepository", "CoroutineCrudRepository",
                        ];
                        let uses_data = stub.imports.iter().any(|i| i.path.starts_with("org.springframework.data"));
                        if uses_data && raw.interfaces.iter().any(|t| t.last_name().is_some_and(|n| DATA_BASES.contains(&n))) {
                            source = Some((BeanSource::Repository, Framework::Spring));
                        }
                    }
                }
                // an unresolved annotation keeps its simple name: judge by the imports
                // (a single-type import of another `Repository` shadows the on-demand one)
                let imports_jakarta_data = self.index.raw_class(&c.name).is_some_and(|(stub, _)| {
                    let single = stub.imports.iter().filter(|i| !i.is_static && !i.on_demand && i.is_named("Repository")).collect::<Vec<_>>();
                    match single.as_slice() {
                        [] => stub.imports.iter().any(|i| !i.is_static && i.on_demand && &*i.path == "jakarta.data.repository"),
                        [i] => &*i.path == "jakarta.data.repository.Repository",
                        _ => false,
                    }
                });
                for a in &c.annotations {
                    match a.simple_name() {
                        "ConfigurationProperties" | "EachProperty" | "ConfigurationReader" if a.name.starts_with("io.micronaut") || !a.resolved => source = Some((BeanSource::Component, Framework::Micronaut)),
                        "FeignClient" | "HttpExchange" => source = Some((BeanSource::Client, Framework::Spring)),
                        "Client" if a.name.starts_with("io.micronaut") || !a.resolved => source = Some((BeanSource::Client, Framework::Micronaut)),
                        "Repository" | "JdbcRepository" | "MongoRepository" | "R2dbcRepository" if a.name.starts_with("io.micronaut") => source = Some((BeanSource::Repository, Framework::Micronaut)),
                        // Jakarta Data: the implementation is generated at build time (Hibernate, Quarkus, ...)
                        "Repository" if a.name.starts_with("jakarta.data") || (!a.resolved && &*a.name == "Repository" && imports_jakarta_data) => source = Some((BeanSource::Repository, Framework::Jakarta)),
                        "Mapper" if mapper_is_bean(a) => source = Some((BeanSource::Mapper, Framework::Spring)),
                        _ => {}
                    }
                }
                let mut test_only = false;
                if source.is_none() {
                    if let Some(t) = in_mapper_scan(c.package()) {
                        source = Some((BeanSource::Mapper, Framework::Spring));
                        test_only = t;
                    }
                }
                if let Some((src, fw)) = source {
                    let qualifiers = self.qualifiers_of(&c.annotations);
                    let name = decapitalize(c.simple_name());
                    let i = self.add_bean(Bean {
                        name,
                        aliases: Vec::new(),
                        ty: c.this_type(),
                        declaring_class: c.name.clone(),
                        source: src,
                        qualifiers,
                        // @FeignClient beans are primary by default (so fallbacks do not clash)
                        primary: c.annotations.iter().any(|a| a.simple_name() == "Primary")
                            || c.annotations.iter().any(|a| a.simple_name() == "FeignClient" && a.arg("primary").and_then(|v| v.as_bool()) != Some(false)),
                        conditional: self.conditional_meta(&c.annotations),
                        file: file.clone(),
                        is_test: is_test || test_only,
                        autowire_candidate: true,
                        framework: fw,
                        dynamic_names: false,
                        replaces: false,
                        static_factory: false,
                    });
                    self.model.component_classes.insert(c.name.clone(), i);
                }
                continue;
            }
            if c.flags.has(Flags::ANON) {
                continue;
            }
            if c.is_abstract() {
                if c.annotations.iter().any(|a| is_mapstruct_mapper(a) && mapper_is_bean(a)) {
                    let i = self.add_bean(Bean {
                        name: decapitalize(c.simple_name()),
                        aliases: Vec::new(),
                        ty: c.this_type(),
                        declaring_class: c.name.clone(),
                        source: BeanSource::Mapper,
                        qualifiers: Vec::new(),
                        primary: false,
                        conditional: false,
                        file: file.clone(),
                        is_test,
                        autowire_candidate: true,
                        framework: Framework::Spring,
                        dynamic_names: false,
                        replaces: false,
                        static_factory: false,
                    });
                    self.model.component_classes.insert(c.name.clone(), i);
                }
                continue;
            }
            let anns = c.annotations.clone();
            if c.constructors().any(|m| m.annotations.iter().any(|a| a.is("jakarta.inject.Inject") || a.is("javax.inject.Inject"))) {
                self.model.jit.push(c.name.clone());
            }
            let is_spring_props = |a: &AnnotationInfo| a.is("org.springframework.boot.context.properties.ConfigurationProperties") || (!a.resolved && a.simple_name() == "ConfigurationProperties");
            // a Spring @ConfigurationProperties class without another stereotype
            // is registered by @EnableConfigurationProperties or
            // @ConfigurationPropertiesScan (named `<prefix>-<class name>`)
            // (composed stereotypes such as an annotation carrying @Component count)
            let properties = anns.iter().any(is_spring_props) && !anns.iter().any(|a| !is_spring_props(a) && self.stereotype(a, 0) == Some(Framework::Spring));
            // plain classes listed in @Import are beans too
            let stereotyped = anns.iter().find_map(|a| self.stereotype(a, 0));
            let fw = stereotyped.or_else(|| imported.contains(&c.name).then_some(Framework::Spring));
            // a class with @Bean methods but no recognized stereotype may still be
            // registered (an annotation from a missing jar, spring.factories,
            // auto-configuration imports, programmatic registration)
            let bean_methods = c.methods.iter().any(|m| self.is_factory_method(m));
            // ... with evidence: an annotation we could not resolve (a stereotype
            // from a missing jar) or a listing in META-INF registration files;
            // a plain class with a leftover @Bean method is not a bean
            let evidence = anns.iter().any(|a| !a.resolved && !matches!(a.simple_name(), "SuppressWarnings" | "Deprecated")) || self.registered.contains(&*c.name) || self.registered.contains(&c.name.replace('$', "."));
            let unrecognized_config = fw.is_none() && bean_methods && evidence && !is_framework_class(&c.name);
            let Some(fw) = fw.or(unrecognized_config.then_some(Framework::Spring)) else { continue };
            // may or may not be registered at runtime: satisfies injections but
            // its own dependencies are not judged
            let maybe_registered = unrecognized_config || (fw == Framework::Spring && (outside_scan(&c) || (properties && !imported.contains(&c.name))));
            // nested non-static classes cannot be beans
            if c.is_inner() {
                continue;
            }
            let explicit = anns.iter().find_map(|a| {
                // @ConfigurationProperties("x") is a property prefix, not a bean name
                if (SPRING_STEREOTYPES.iter().any(|s| a.is(s)) && !is_spring_props(a)) || a.simple_name() == "Named" {
                    first_string(a, &["value"])
                } else {
                    None
                }
            });
            let name = explicit.unwrap_or_else(|| {
                if properties {
                    let prefix = anns.iter().find(|a| is_spring_props(a)).and_then(|a| first_string(a, &["prefix", "value"])).unwrap_or_default();
                    if prefix.is_empty() { c.name.to_string() } else { format!("{prefix}-{}", c.name) }
                } else if fw == Framework::Spring && imported.contains(&c.name) && (!in_scan(c.package()) || stereotyped.is_none()) {
                    // classes registered by @Import (not by scanning) are named by their class name
                    c.name.to_string()
                } else {
                    decapitalize(c.simple_name())
                }
            });
            let qualifiers = self.qualifiers_of(&anns);
            let primary = anns.iter().any(|a| a.simple_name() == "Primary");
            // registered only by scans on conditional classes (@Profile("broker")
            // scanning the broker package) means conditional too
            let pkg = c.package().to_string();
            // roots declared in this module decide; roots from other modules
            // (an example app scanning `io.camunda`) only if this one has none
            let own_module = file.as_deref().map(module_of).unwrap_or("").to_string();
            let local = root_conditions.iter().any(|(_, _, m)| *m == own_module);
            let covering: Vec<bool> = root_conditions.iter().filter(|(r, _, m)| (!local || *m == own_module) && package_covered(r, &pkg)).map(|(_, c, _)| *c).collect();
            let scanned_conditionally = fw == Framework::Spring && !covering.is_empty() && covering.iter().all(|c| *c);
            let conditional = self.conditional_meta(&anns) || maybe_registered || scanned_conditionally;
            let i = self.add_bean(Bean {
                name,
                aliases: Vec::new(),
                ty: c.this_type(),
                declaring_class: c.name.clone(),
                source: BeanSource::Component,
                qualifiers,
                primary,
                conditional,
                file: file.clone(),
                is_test,
                autowire_candidate: true,
                framework: fw,
                dynamic_names: anns.iter().any(|a| matches!(a.simple_name(), "EachBean" | "EachProperty")),
                replaces: anns.iter().any(|a| a.simple_name() == "Replaces"),
                static_factory: false,
            });
            self.model.component_classes.insert(c.name.clone(), i);
            let class_dynamic = anns.iter().any(|a| matches!(a.simple_name(), "EachBean" | "EachProperty"));

            // factory / @Bean methods, including ones inherited from source
            // superclasses (`class RedisConfig extends BaseRedisConfig`)
            let mut factory_methods: Vec<MethodInfo> = c.methods.clone();
            // Return types are not substituted: Spring matches an inherited
            // `@Bean T value()` by its erased type, so `App extends Base<Store>`
            // does not provide a Store (verified against Spring 6.1). A
            // subclass method hides an inherited one only with the same
            // parameter types.
            let same_sig = |a: &MethodInfo, b: &MethodInfo| a.name == b.name && a.params.len() == b.params.len() && a.params.iter().zip(&b.params).all(|(x, y)| x.ty.erasure() == y.ty.erasure());
            let mut sup: Option<Ty> = c.superclass.clone();
            let mut hops = 0;
            while let Some(st) = sup.take() {
                hops += 1;
                let Some(sc) = st.class_name().and_then(|sn| self.index.class(sn)).filter(|sc| sc.is_source()) else { break };
                // a base that is a bean itself registers its own @Bean methods
                let base_anns = sc.annotations.clone();
                if base_anns.iter().any(|a| self.stereotype(a, 0).is_some()) {
                    break;
                }
                for m in &sc.methods {
                    let m_factory = self.is_factory_method(m);
                    match factory_methods.iter_mut().find(|x| same_sig(x, m)) {
                        // Spring registers the superclass's @Bean methods on the
                        // subclass and calls the override: an override without
                        // @Bean (of an abstract @Bean method) still defines the bean
                        Some(x) => {
                            // (the override's own annotations, @Primary or a
                            // qualifier, stay)
                            if m_factory && !self.is_factory_method(x) {
                                let missing: Vec<AnnotationInfo> = m.annotations.iter().filter(|a| !x.annotations.iter().any(|o| o.name == a.name)).cloned().collect();
                                x.annotations.extend(missing);
                            }
                        }
                        None => factory_methods.push(m.clone()),
                    }
                }
                sup = sc.superclass.clone();
                if hops > 8 {
                    break;
                }
            }
            for m in &factory_methods {
                if !self.is_factory_method(m) {
                    continue;
                }
                if m.ret == Ty::Void || m.ret.is_unknown() {
                    continue;
                }
                let bean_ann = m.annotations.iter().find(|a| a.simple_name() == "Bean");
                let names: Vec<String> = bean_ann.map(|a| ["name", "value"].iter().flat_map(|k| a.arg(k).map(|v| v.strings()).unwrap_or_default()).collect()).unwrap_or_default();
                let (name, aliases) = match names.split_first() {
                    Some((f, rest)) => (f.clone(), rest.to_vec()),
                    None => (m.name.to_string(), Vec::new()),
                };
                let mut qualifiers = self.qualifiers_of(&m.annotations);
                if let Some(nm) = m.annotations.iter().find(|a| a.simple_name() == "Named").and_then(|a| first_string(a, &["value"])) {
                    qualifiers.push(nm);
                }
                let autowire_candidate = bean_ann.and_then(|a| a.arg("autowireCandidate")).and_then(|v| v.as_bool()) != Some(false);
                let mbean = Bean {
                    name,
                    aliases,
                    ty: m.ret.clone(),
                    declaring_class: c.name.clone(),
                    source: BeanSource::FactoryMethod(m.name.clone()),
                    qualifiers,
                    primary: m.annotations.iter().any(|a| a.simple_name() == "Primary"),
                    conditional: conditional || self.conditional_meta(&m.annotations),
                    file: file.clone(),
                    is_test,
                    autowire_candidate,
                    framework: fw,
                    dynamic_names: class_dynamic || m.annotations.iter().any(|a| matches!(a.simple_name(), "EachBean" | "EachProperty")),
                    replaces: m.annotations.iter().any(|a| a.simple_name() == "Replaces"),
                    static_factory: m.flags.is_static(),
                };
                self.add_bean(mbean);
                self.produced.insert((c.name.clone(), m.name.clone()));
                // factory method parameters are injection points
                for (pi, p) in m.params.iter().enumerate() {
                    if !maybe_registered {
                        self.add_point(&c.name, PointKind::FactoryParam(m.name.clone(), pi), &p.ty, &p.annotations, &p.name, m.range, fw, false);
                    }
                }
            }

            // constructor injection
            let ctors: Vec<&MethodInfo> = c.constructors().filter(|m| !m.flags.is_private() || c.constructors().count() == 1).collect();
            let annotated: Vec<&MethodInfo> = ctors.iter().copied().filter(|m| m.annotations.iter().any(|a| INJECT_ANNOTATIONS.iter().any(|x| a.is(x)))).collect();
            let chosen = if annotated.len() == 1 {
                Some(annotated[0])
            } else if annotated.is_empty() && ctors.len() == 1 {
                Some(ctors[0])
            } else {
                None
            };
            if let Some(ctor) = chosen {
                // records/ConfigurationProperties bind properties, not beans;
                // prototype beans may get constructor arguments from
                // getBean(Class, args...)
                let binds_properties = anns.iter().any(|a| a.simple_name() == "ConfigurationProperties" || a.simple_name() == "EachProperty");
                let prototype = anns.iter().any(|a| a.simple_name() == "Scope" && a.arg("value").or_else(|| a.arg("scopeName")).is_some_and(|v| format!("{v:?}").to_ascii_lowercase().contains("prototype")));
                if !binds_properties && !prototype {
                    for (pi, p) in ctor.params.iter().enumerate() {
                        if !maybe_registered {
                            self.add_point(&c.name, PointKind::ConstructorParam(pi), &p.ty, &p.annotations, &p.name, ctor.range, fw, false);
                        }
                    }
                }
            }
            // field injection
            let raw_inits: Vec<Name> = self.index.raw_class(&c.name).map(|(_, r)| r.fields.iter().filter(|f| f.has_initializer).map(|f| f.name.clone()).collect()).unwrap_or_default();
            for f in &c.fields {
                if f.annotations.iter().any(|a| INJECT_ANNOTATIONS.iter().any(|x| a.is(x))) {
                    // Micronaut keeps the field's default value when no bean exists
                    let defaulted = fw != Framework::Spring && raw_inits.contains(&f.name);
                    let optional = defaulted || f.annotations.iter().any(|a| a.simple_name() == "Autowired" && a.arg("required").and_then(|v| v.as_bool()) == Some(false));
                    if !maybe_registered {
                        self.add_point(&c.name, PointKind::Field, &f.ty, &f.annotations, &f.name, f.range, fw, optional);
                    }
                }
            }
            // setter / method injection
            for m in &c.methods {
                if m.is_constructor() || !m.annotations.iter().any(|a| INJECT_ANNOTATIONS.iter().any(|x| a.is(x))) {
                    continue;
                }
                let optional = m.annotations.iter().any(|a| a.simple_name() == "Autowired" && a.arg("required").and_then(|v| v.as_bool()) == Some(false));
                for (pi, p) in m.params.iter().enumerate() {
                    if !maybe_registered {
                        self.add_point(&c.name, PointKind::MethodParam(m.name.clone(), pi), &p.ty, &p.annotations, &p.name, m.range, fw, optional);
                    }
                }
            }
        }
        // Spring test mocks: @MockitoBean / @MockBean / spy fields (and the
        // class-level `types = ...` form) put a bean of that type into the
        // test's context
        const MOCKS: &[&str] = &["MockitoBean", "MockitoSpyBean", "MockBean", "SpyBean"];
        for n in &names {
            let Some(c) = self.index.class(n) else { continue };
            let file = c.source_file().map(Arc::from);
            let mut mocks: Vec<(String, Ty)> = c.fields.iter().filter(|f| f.annotations.iter().any(|a| MOCKS.contains(&a.simple_name()))).map(|f| (f.name.to_string(), f.ty.clone())).collect();
            for a in c.annotations.iter().filter(|a| MOCKS.contains(&a.simple_name())) {
                for v in a.arg("types").or_else(|| a.arg("value")).map(|v| v.values()).unwrap_or_default() {
                    if let AnnotationValue::Class(t) = v {
                        mocks.push((t.class_name().map(|n| decapitalize(simple_name_of(n))).unwrap_or_default(), t.clone()));
                    }
                }
            }
            // Micronaut's @MockBean on a method of a test class: a factory
            // method whose result replaces the bean in that test
            // (io.micronaut.test.annotation.MockBean is @Bean @Replaces)
            let mut method_mocks: Vec<(String, Ty)> = Vec::new();
            for m in &c.methods {
                let micronaut_mock = m.annotations.iter().any(|a| a.is("io.micronaut.test.annotation.MockBean") && (a.name.starts_with("io.micronaut") || !a.resolved));
                if micronaut_mock && !m.is_constructor() && m.ret != Ty::Void && !m.ret.is_unknown() {
                    method_mocks.push((m.name.to_string(), m.ret.clone()));
                    self.produced.insert((c.name.clone(), m.name.clone()));
                }
            }
            for (name, ty, micronaut) in mocks.into_iter().map(|(n, t)| (n, t, false)).chain(method_mocks.into_iter().map(|(n, t)| (n, t, true))) {
                self.add_bean(Bean {
                    name,
                    aliases: Vec::new(),
                    ty,
                    declaring_class: c.name.clone(),
                    source: BeanSource::Component,
                    qualifiers: Vec::new(),
                    primary: false,
                    conditional: false,
                    file: file.clone(),
                    is_test: true,
                    autowire_candidate: true,
                    framework: if micronaut { Framework::Micronaut } else { Framework::Spring },
                    dynamic_names: false,
                    replaces: micronaut,
                    static_factory: false,
                });
            }
        }
        // Micronaut @Replaces: the replaced bean no longer exists where the
        // replacement does. That is certain only for an unconditional
        // replacement in the main sources of the replaced bean's module; a
        // conditional one (`@Requires(property = ...)`), one in another
        // module (another application may not include it) or one in test
        // sources (a @MockBean method replaces the bean in its own test
        // only) leaves the replaced bean possibly present: it satisfies
        // injections but makes nothing ambiguous.
        // (replaced class, module of the replacement, certain)
        let mut replaced: Vec<(Name, String, bool)> = Vec::new();
        for n in &names {
            let Some(c) = self.index.class(n) else { continue };
            let file = c.source_file().unwrap_or("");
            let module = module_of(file).to_string();
            let class_conditional = self.conditional_meta(&c.annotations);
            let sites: Vec<(&[AnnotationInfo], bool)> =
                std::iter::once((c.annotations.as_slice(), false)).chain(c.methods.iter().map(|m| (m.annotations.as_slice(), self.conditional_meta(&m.annotations)))).collect();
            for (anns, site_conditional) in sites {
                for a in anns.iter().filter(|a| a.simple_name() == "Replaces") {
                    for k in ["value", "bean"] {
                        if let Some(AnnotationValue::Class(t)) = a.arg(k) {
                            if let Some(tn) = t.class_name() {
                                let certain = !is_test_path(file) && !class_conditional && !site_conditional;
                                replaced.push((Arc::from(tn), module.clone(), certain));
                            }
                        }
                    }
                }
            }
        }
        for b in &mut self.model.beans {
            if b.source != BeanSource::Component {
                continue;
            }
            let module = b.file.as_deref().map(module_of).unwrap_or("");
            for (_, m, certain) in replaced.iter().filter(|(r, _, _)| *r == b.declaring_class) {
                if *certain && m == module {
                    b.autowire_candidate = false;
                } else {
                    b.conditional = true;
                }
            }
        }
        let mut by_class: FxHashMap<Name, Vec<usize>> = FxHashMap::default();
        for (i, b) in self.model.beans.iter().enumerate() {
            if let Some(n) = b.ty.class_name() {
                by_class.entry(Arc::from(n)).or_default().push(i);
            }
        }
        self.model.by_class = by_class;
        self.model.contexts = self.app_contexts(&names);
        self.model.spring_application = !self.model.contexts.is_empty();
        self.unseen_registrations(&names, &imported_anywhere);
    }

    /// The classes Spring application contexts are started from, in main
    /// sources: a `@SpringBootApplication`, `@SpringBootConfiguration` or
    /// `@ComponentScan` class with a `main` method, and the configuration
    /// classes named in a file whose `main` starts a context from classes
    /// (`SpringApplication.run(App.class)`, `new
    /// AnnotationConfigApplicationContext(Config.class)`) or in a servlet
    /// initializer (`sources(App.class)`, `getRootConfigClasses()`). An
    /// application class nothing starts (a sample, a class only tests
    /// start) is no root, nor is a context read from XML.
    fn context_roots(&mut self, names: &[Name]) -> Vec<Name> {
        const STARTERS: &[&str] = &[
            "SpringApplication", "SpringApplicationBuilder", "AnnotationConfigApplicationContext", "AnnotationConfigWebApplicationContext",
            "AnnotationConfigServletWebServerApplicationContext", "AnnotationConfigReactiveWebServerApplicationContext", "GenericApplicationContext",
        ];
        const INITIALIZERS: &[&str] = &["org.springframework.boot.web.servlet.support.SpringBootServletInitializer", "org.springframework.web.WebApplicationInitializer"];
        // (without the jars: the base classes Spring offers, by name)
        const INITIALIZER_BASES: &[&str] = &[
            "SpringBootServletInitializer", "WebApplicationInitializer", "AbstractAnnotationConfigDispatcherServletInitializer", "AbstractDispatcherServletInitializer",
            "AbstractContextLoaderInitializer",
        ];
        let mut roots: Vec<Name> = Vec::new();
        for n in names {
            let Some(c) = self.index.class(n) else { continue };
            let Some(f) = c.source_file() else { continue };
            if is_test_path(f) || is_framework_class(n) || c.is_interface() {
                continue;
            }
            let main = has_main(&c);
            let app_annotation = c.annotations.iter().any(|a| {
                a.is("org.springframework.boot.autoconfigure.SpringBootApplication") || a.is("org.springframework.boot.SpringBootConfiguration") || a.is("org.springframework.context.annotation.ComponentScan")
            });
            if main && app_annotation {
                roots.push(n.clone());
            }
            let Some(stub) = self.index.file(f).cloned() else { continue };
            let mentions = |x: &str| stub.referenced_names.binary_search_by(|r| (**r).cmp(x)).is_ok();
            let starts = main && STARTERS.iter().any(|s| mentions(s));
            let initializer = INITIALIZERS.iter().any(|i| self.index.is_subclass(n, i).is_yes())
                || self.index.raw_class(n).is_some_and(|(_, r)| r.superclass.iter().chain(&r.interfaces).any(|t| t.last_name().is_some_and(|l| INITIALIZER_BASES.contains(&l))));
            if !(starts || initializer) {
                continue;
            }
            if initializer && self.configuration_like(&c) {
                roots.push(n.clone());
            }
            // the configuration classes the file names
            let Some(raw) = stub.classes.first() else { continue };
            let scope = self.index.scope_for_raw(&stub, raw);
            for r in &stub.referenced_names {
                let Some(q) = self.index.resolve_simple_type(r, &scope) else { continue };
                if !self.index.is_source_class(&q) || self.index.source_file_of(&q).is_some_and(|f| is_test_path(f)) {
                    continue;
                }
                if self.index.class(&q).is_some_and(|rc| !rc.is_interface() && self.configuration_like(&rc)) {
                    roots.push(q);
                }
            }
        }
        roots.sort();
        roots.dedup();
        roots
    }

    /// Would Spring process the class as a configuration when handed to it:
    /// a Spring stereotype, a scan, an import, or factory methods?
    fn configuration_like(&mut self, c: &ClassInfo) -> bool {
        c.annotations.iter().any(|a| {
            matches!(a.simple_name(), "ComponentScan" | "ComponentScans" | "Import" | "ImportResource" | "SpringBootApplication" | "SpringBootConfiguration" | "EnableAutoConfiguration")
                || self.stereotype(a, 0) == Some(Framework::Spring)
        }) || c.methods.iter().any(|m| self.is_factory_method(m))
    }

    /// The scans declared on a class whose result the model can name:
    /// (package pattern, declared by `@SpringBootApplication`). A scan with
    /// exclude filters or without the default filters, or on a conditional
    /// class, registers what the model cannot name and is left out.
    fn certain_scans(&self, c: &ClassInfo) -> Vec<(String, bool)> {
        if self.conditional_meta(&c.annotations) {
            return Vec::new();
        }
        let mut scans: Vec<AnnotationInfo> = Vec::new();
        for a in &c.annotations {
            if a.is("org.springframework.boot.autoconfigure.SpringBootApplication") || a.is("org.springframework.context.annotation.ComponentScan") {
                scans.push(a.clone());
            } else if a.simple_name() == "ComponentScans" {
                for v in a.arg("value").map(|v| v.values()).unwrap_or_default() {
                    if let AnnotationValue::Annotation(x) = v {
                        scans.push((**x).clone());
                    }
                }
            }
        }
        let mut out = Vec::new();
        for a in &scans {
            if a.arg("excludeFilters").is_some() || a.arg("useDefaultFilters").and_then(|v| v.as_bool()) == Some(false) {
                continue;
            }
            let boot = a.is("org.springframework.boot.autoconfigure.SpringBootApplication");
            out.extend(scan_packages(c, a).into_iter().map(|p| (p, boot)));
        }
        out
    }

    fn app_contexts(&mut self, names: &[Name]) -> Vec<AppContext> {
        let roots = self.context_roots(names);
        if roots.is_empty() {
            return Vec::new();
        }
        // the components a scan may register: (class, bean framework, module, in tests)
        let components: Vec<(Name, usize)> = {
            let mut v: Vec<(Name, usize)> = self.model.component_classes.iter().map(|(k, &i)| (k.clone(), i)).collect();
            v.sort();
            v
        };
        let mut out = Vec::new();
        for root in roots {
            let Some(rf) = self.index.source_file_of(&root).cloned() else { continue };
            let root_module = module_of(&rf).to_string();
            let mut ctx = AppContext { root: root.clone(), ..Default::default() };
            ctx.members.insert(root.clone());
            let mut scans: Vec<(String, bool)> = Vec::new();
            let mut work = vec![root.clone()];
            loop {
                while let Some(n) = work.pop() {
                    let Some(c) = self.index.class(&n) else { continue };
                    // @Import and @EnableConfigurationProperties name classes
                    // registered wherever they live
                    for a in c.annotations.iter().filter(|a| matches!(a.simple_name(), "Import" | "EnableConfigurationProperties")) {
                        for v in a.arg("value").map(|v| v.values()).unwrap_or_default() {
                            if let AnnotationValue::Class(t) = v {
                                if let Some(t) = t.class_name().filter(|t| self.index.is_source_class(t)) {
                                    let t: Name = Arc::from(t);
                                    if ctx.members.insert(t.clone()) {
                                        work.push(t);
                                    }
                                }
                            }
                        }
                    }
                    scans.extend(self.certain_scans(&c));
                }
                let mut grew = false;
                for (k, bi) in &components {
                    if ctx.members.contains(k) || ctx.test_members.contains(k) {
                        continue;
                    }
                    let b = &self.model.beans[*bi];
                    let Some(file) = b.file.as_deref() else { continue };
                    let module = module_of(file);
                    let test = is_test_path(file);
                    if test && module != root_module {
                        continue;
                    }
                    // what Spring's scanner registers: Spring stereotypes and
                    // JSR-330 @Named / @ManagedBean classes, and the
                    // repositories, clients and mappers of the scanned packages
                    let Some(kc) = self.index.class(k) else { continue };
                    let scanned_kind = b.framework == Framework::Spring || kc.annotations.iter().any(|a| matches!(a.simple_name(), "Named" | "ManagedBean"));
                    let auto_configuration = self.model.auto_configurations.contains(k);
                    if !scanned_kind || !scans.iter().any(|(r, boot)| !(*boot && auto_configuration) && package_covered(r, kc.package())) {
                        continue;
                    }
                    if test {
                        // a test's configurations and the classes nested in tests
                        // are excluded from an application's scan (TypeExcludeFilter)
                        if k.contains('$') || kc.annotations.iter().any(|a| matches!(a.simple_name(), "TestConfiguration" | "TestComponent")) {
                            continue;
                        }
                        ctx.test_members.insert(k.clone());
                    } else {
                        ctx.members.insert(k.clone());
                        work.push(k.clone());
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
            out.push(ctx);
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn add_point(&mut self, owner: &Name, kind: PointKind, ty: &Ty, anns: &[AnnotationInfo], name: &str, range: Option<(u32, u32)>, fw: Framework, optional_flag: bool) {
        // @Value / property bindings are not bean lookups
        if anns.iter().any(|a| matches!(a.simple_name(), "Value" | "Property" | "ConfigValue" | "Parameter" | "EachProperty")) {
            return;
        }
        let (elem, optional, multiple, lazy) = unwrap_injected(ty);
        let nullable = anns.iter().any(|a| matches!(a.simple_name(), "Nullable" | "Lazy"));
        let qualifier = self.injection_qualifier(anns);
        self.model.points.push(InjectionPoint {
            owner: owner.clone(),
            kind,
            ty: elem,
            declared_ty: ty.clone(),
            qualifier,
            name: name.to_string(),
            optional: optional || optional_flag || nullable,
            multiple,
            lazy,
            range,
            framework: fw,
            file: self.index.source_file_of(owner).cloned(),
        });
    }
}

/// Micronaut/JSR-330 style implicit names: `@Named("v8")` or `@V8` selects
/// `V8Engine` when injecting `Engine` (class name minus the type's name).
fn micronaut_name_matches(b: &Bean, want_simple: &str, q: &str) -> bool {
    let q = q.trim_start_matches('@');
    let q = q.rsplit(['.', '$']).next().unwrap_or(q).to_ascii_lowercase();
    let simple = match &b.source {
        BeanSource::FactoryMethod(m) => m.to_string(),
        _ => simple_name_of(&b.declaring_class).to_string(),
    };
    let base = simple.strip_suffix(want_simple).unwrap_or(&simple).to_ascii_lowercase();
    !base.is_empty() && base == q
}

/// Unwrap Optional<T>, Provider<T>, List<T>... Returns (element, optional, multiple, lazy).
pub fn unwrap_injected(ty: &Ty) -> (Ty, bool, bool, bool) {
    match ty {
        Ty::Array(e) => ((**e).clone(), true, true, false),
        Ty::Class { name, args } => {
            let a0 = || args.first().map(|a| a.upper_bound()).unwrap_or(Ty::Unknown);
            match &**name {
                "java.util.Optional" => (a0(), true, false, false),
                "org.springframework.beans.factory.ObjectProvider" | "io.micronaut.context.BeanProvider" => (a0(), true, false, true),
                // (a Supplier is an ordinary type: Spring wants a Supplier bean)
                "jakarta.inject.Provider" | "javax.inject.Provider" | "org.springframework.beans.factory.ObjectFactory" => (a0(), false, false, true),
                "java.util.List" | "java.util.Collection" | "java.util.Set" | "java.lang.Iterable" | "java.util.SortedSet" | "java.util.stream.Stream" => (a0(), true, true, false),
                "java.util.Map" if args.first().is_some_and(|k| k.is_string()) => (args.get(1).map(|a| a.upper_bound()).unwrap_or(Ty::Unknown), true, true, false),
                "jakarta.enterprise.inject.Instance" | "javax.enterprise.inject.Instance" => (a0(), true, true, true),
                _ => (ty.clone(), false, false, false),
            }
        }
        t => (t.clone(), false, false, false),
    }
}

impl DiModel {
    pub fn build(index: &Index) -> DiModel {
        Self::build_with(index, &[], &[])
    }

    /// `registered`: classes listed in META-INF/spring.factories or
    /// META-INF/spring/*.imports (auto-configurations and similar);
    /// `auto_registered`: the auto-configurations among them.
    pub fn build_with(index: &Index, registered: &[String], auto_registered: &[String]) -> DiModel {
        let mut b = Builder {
            index,
            model: DiModel::default(),
            stereo: FxHashMap::default(),
            qualifier_ann: FxHashMap::default(),
            factory_ann: FxHashMap::default(),
            produced: Default::default(),
            filter_annotations: Default::default(),
            unexpanded_mapper_scan: false,
            unexpanded_mapper_scan_in_tests: false,
            filter_types: Vec::new(),
            opaque_filter: false,
            opaque_filter_in_tests: false,
            registered: registered.iter().cloned().collect(),
            auto_registered: auto_registered.iter().cloned().collect(),
        };
        b.scan();
        let mut m = b.model;
        m.duplicated = index.source_class_names().filter(|n| index.is_duplicated(n)).cloned().collect();
        if !m.duplicated.is_empty() {
            let mut names: rustc_hash::FxHashSet<&str> = rustc_hash::FxHashSet::default();
            for (_, stub) in index.files() {
                if !stub.classes.iter().any(|c| m.duplicated.contains(&c.name)) {
                    continue;
                }
                for c in &stub.classes {
                    names.extend(c.superclass.iter().chain(&c.interfaces).filter_map(|t| t.last_name()));
                    let factory = |m: &&grounds_index::stubs::RawMethod| m.annotations.iter().any(|a| matches!(a.simple_name(), "Bean" | "Produces" | "Provides" | "Singleton"));
                    names.extend(c.methods.iter().filter(factory).filter_map(|m| m.ret.last_name()));
                }
            }
            let named: Vec<Name> = index.source_class_names().filter(|n| names.contains(simple_name_of(n))).cloned().collect();
            for n in named {
                for st in index.all_supertypes(&Ty::class(&n)) {
                    if let Some(sn) = st.class_name() {
                        m.duplicated_provides.insert(Arc::from(sn));
                    }
                }
                m.duplicated_provides.insert(n);
            }
        }
        m.same_name_factories = m.same_name_factories();
        m.index_providers(index);
        m.ctor_edges = m.constructor_edges(index);
        m
    }

    pub fn is_empty(&self) -> bool {
        self.beans.is_empty()
    }

    fn same_name_factories(&self) -> FxHashMap<usize, Vec<usize>> {
        let module = |b: &Bean| b.file.as_deref().map(module_of).unwrap_or("").to_string();
        let mut factories: FxHashMap<(String, &str), Vec<usize>> = FxHashMap::default();
        for (i, b) in self.beans.iter().enumerate() {
            if b.framework == Framework::Spring && matches!(b.source, BeanSource::FactoryMethod(_)) {
                factories.entry((module(b), b.name.as_str())).or_default().push(i);
            }
        }
        let mut out: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
        for (class, &i) in &self.component_classes {
            let b = &self.beans[i];
            if b.framework != Framework::Spring || b.declaring_class != *class || b.source != BeanSource::Component {
                continue;
            }
            if let Some(f) = factories.get(&(module(b), b.name.as_str())) {
                out.insert(i, f.clone());
            }
        }
        out
    }

    pub fn bean_of_class(&self, class: &str) -> Option<&Bean> {
        self.component_classes.get(class).map(|&i| &self.beans[i])
    }

    pub fn points_of(&self, class: &str) -> impl Iterator<Item = &InjectionPoint> {
        let c = class.to_string();
        self.points.iter().filter(move |p| *p.owner == *c)
    }

    fn index_providers(&mut self, lk: &dyn ClassLookup) {
        let mut providers: FxHashMap<Name, Vec<usize>> = FxHashMap::default();
        let mut incomplete = Vec::new();
        for (i, b) in self.beans.iter().enumerate() {
            let Some(bn) = b.ty.class_name() else {
                incomplete.push(i);
                continue;
            };
            for st in lk.all_supertypes(&Ty::class(bn)) {
                if let Some(n) = st.class_name() {
                    providers.entry(Arc::from(n)).or_default().push(i);
                }
            }
            if !lk.hierarchy_complete(bn) {
                incomplete.push(i);
            }
        }
        self.providers = providers;
        self.incomplete = incomplete;
    }

    /// Beans that can be injected where `ty` is requested.
    /// Returns (definite matches, number of possible matches with an
    /// incomplete hierarchy).
    pub fn candidates(&self, lk: &dyn ClassLookup, ty: &Ty, include_test: bool) -> (Vec<usize>, usize) {
        let Some(want) = ty.class_name() else { return (Vec::new(), 1) };
        let usable = |b: &Bean| !(b.is_test && !include_test) && b.autowire_candidate;
        let mut out = Vec::new();
        for &i in self.providers.get(want).map(|v| v.as_slice()).unwrap_or(&[]) {
            let b = &self.beans[i];
            if !usable(b) {
                continue;
            }
            // generic arguments act as qualifiers in Spring
            if !ty.args().is_empty() {
                if let Some(sup) = lk.as_super(&b.ty, want) {
                    let compatible = ty.args().iter().zip(sup.args()).all(|(w, h)| {
                        let w = w.upper_bound().erasure();
                        let h = h.upper_bound().erasure();
                        w.is_unknown() || h.is_unknown() || w == h || w.is_class("java.lang.Object") || matches!(h, Ty::TypeVar(_))
                    });
                    if !compatible {
                        continue;
                    }
                }
            }
            out.push(i);
        }
        // beans with unknown supertypes might also provide the type
        let maybe = self.incomplete.iter().filter(|&&i| usable(&self.beans[i]) && !out.contains(&i)).count();
        (out, maybe)
    }

    pub fn resolve(&self, lk: &dyn ClassLookup, p: &InjectionPoint, include_test: bool) -> Resolution {
        if p.ty.is_unknown() || p.ty.mentions_type_var() || self.duplicated.contains(&p.owner) {
            return Resolution::Unknown;
        }
        // JSR-330 points in a project wired (also) by a container whose
        // bindings we cannot see: Guice resolves cycles through proxies,
        // Sisu picks the highest-ranked of several implementations, and
        // any module, binder or build step may bind the type
        if p.framework == Framework::Jakarta && (self.unmodeled_container.is_some() || (self.unmodeled_container_in_tests.is_some() && p.file.as_deref().is_some_and(is_test_code))) {
            return Resolution::Unknown;
        }
        // a library's beans are wired by the application using it
        if p.framework == Framework::Spring && !p.file.as_deref().is_some_and(is_test_path) && !self.in_application(&p.owner) {
            return Resolution::Unknown;
        }
        if let Some(t) = p.ty.class_name() {
            if self.duplicated.contains(t) || self.duplicated_provides.contains(t) {
                return Resolution::Unknown;
            }
        }
        let (mut cands, mut maybe) = self.candidates(lk, &p.ty, include_test);
        if cands.iter().any(|&i| self.duplicated.contains(&self.beans[i].declaring_class)) {
            return Resolution::Unknown;
        }
        // library supertypes can never lead to a project type, so beans with an
        // incomplete (library) hierarchy cannot provide one
        if self.provable_type(lk, &p.ty) {
            maybe = 0;
        }
        if let Some(q) = &p.qualifier {
            if q.is_empty() {
                return Resolution::Unknown;
            }
            let before = cands.clone();
            let want_simple = p.ty.class_name().map(simple_name_of).unwrap_or("");
            cands.retain(|&i| {
                let b = &self.beans[i];
                b.matches_qualifier(q) || (b.framework != Framework::Spring && micronaut_name_matches(b, want_simple, q))
            });
            if cands.is_empty() && before.iter().any(|&i| self.beans[i].replaces) {
                return Resolution::Unknown;
            }
            // JSR-330 containers bind classes with @Inject constructors just in time
            if cands.is_empty() && p.framework != Framework::Spring && self.jit.iter().any(|j| p.ty.class_name().is_some_and(|t| lk.is_subclass(j, t).is_yes())) {
                return Resolution::Unknown;
            }
            if cands.is_empty() && before.iter().any(|&i| self.beans[i].dynamic_names) {
                return Resolution::Unknown;
            }
            if cands.is_empty() {
                if before.is_empty() {
                    return if maybe > 0 || self.opaque_imports || p.lazy || !self.provable_type(lk, &p.ty) || self.dynamically_registered(&p.ty, include_test) || self.may_be_registered_unseen(lk, p, include_test) {
                        Resolution::Unknown
                    } else if p.optional {
                        Resolution::Absent
                    } else {
                        Resolution::NotFound
                    };
                }
                if maybe > 0 {
                    return Resolution::Unknown;
                }
                // @Autowired(required = false), Optional<...>: left unset
                if p.optional {
                    return Resolution::Absent;
                }
                // a library type may get named beans from its own
                // auto-configuration (clients configured by properties)
                if !self.provable_type(lk, &p.ty) || self.dynamically_registered(&p.ty, include_test) || p.lazy || self.may_be_registered_unseen(lk, p, include_test) {
                    return Resolution::Unknown;
                }
                return Resolution::QualifierNotFound { qualifier: q.clone(), candidates: before };
            }
        }
        // a component and a @Bean method of the same name are one bean: the
        // @Bean definition when both match, and when only the component
        // does, whether it survives depends on the Spring version and on
        // spring.main.allow-bean-definition-overriding
        for &i in &cands.clone() {
            // (a test configuration's @Bean exists only in tests)
            if let Some(f) = self.same_name_factories.get(&i).filter(|f| f.iter().any(|&j| include_test || !self.beans[j].is_test)) {
                if f.iter().any(|j| cands.contains(j)) {
                    cands.retain(|&c| c != i);
                } else {
                    return Resolution::Unknown;
                }
            }
        }
        // Spring skips self-references and falls back to them only when
        // nothing else matches a single-valued point
        if p.framework == Framework::Spring {
            let own = self.self_references(p);
            if cands.iter().any(|&i| !own.contains(&i)) || p.multiple {
                cands.retain(|i| !own.contains(i));
            }
        }
        if p.multiple {
            return Resolution::Multiple(cands);
        }
        // ObjectProvider, Provider, ObjectFactory: the bean is looked up when
        // the code asks for it (often through orderedStream() or getIfUnique()),
        // which may or may not be during startup: not judged
        if p.lazy && cands.len() != 1 {
            return if cands.is_empty() && p.optional { Resolution::Absent } else { Resolution::Unknown };
        }
        let jit_possible = || self.jit.iter().any(|j| p.ty.class_name().is_some_and(|t| lk.is_subclass(j, t).is_yes()));
        match cands.len() {
            0 => {
                if p.optional {
                    Resolution::Absent
                } else if maybe > 0
                    || self.opaque_imports
                    || !self.provable_type(lk, &p.ty)
                    || jit_possible()
                    || self.factory_may_return(lk, &p.ty)
                    || self.dynamically_registered(&p.ty, include_test)
                    || self.may_be_registered_unseen(lk, p, include_test)
                {
                    Resolution::Unknown
                } else {
                    Resolution::NotFound
                }
            }
            1 => Resolution::Unique(cands[0]),
            _ => {
                let primaries: Vec<usize> = cands.iter().copied().filter(|&i| self.beans[i].primary).collect();
                if primaries.len() == 1 {
                    return Resolution::Unique(primaries[0]);
                }
                // Spring falls back to matching the parameter/field name
                if p.framework == Framework::Spring {
                    if let Some(&i) = cands.iter().find(|&&i| self.beans[i].matches_qualifier(&p.name)) {
                        return Resolution::Unique(i);
                    }
                }
                // conditional beans (profiles, @ConditionalOn...) may be mutually exclusive;
                // beans from other modules may not be on the same runtime classpath
                let point_module = p.file.as_deref().map(module_of).unwrap_or("");
                if cands.iter().any(|&i| self.beans[i].replaces) {
                    return Resolution::Unknown;
                }
                let unconditional: Vec<usize> = cands
                    .iter()
                    .copied()
                    .filter(|&i| !self.beans[i].conditional && !self.beans[i].dynamic_names)
                    .filter(|&i| self.beans[i].file.as_deref().map(module_of).unwrap_or("") == point_module)
                    .collect();
                if unconditional.len() < cands.len() && unconditional.len() < 2 {
                    return Resolution::Unknown;
                }
                if unconditional.len() < 2 || maybe > 0 {
                    return Resolution::Unknown;
                }
                // configurations nested in test classes belong to that test's
                // own context; they do not compete with each other
                if unconditional.iter().any(|&i| self.beans[i].is_test && self.beans[i].declaring_class.contains('$')) {
                    return Resolution::Unknown;
                }
                // Spring: only beans of one context compete, and only in a
                // context that also holds the injecting bean
                if p.framework == Framework::Spring {
                    let shared = self.contexts.iter().filter(|ctx| ctx.holds(&p.owner, include_test)).map(|ctx| unconditional.iter().copied().filter(|&i| self.context_holds(ctx, &self.beans[i], include_test)).collect::<Vec<usize>>()).find(|v| v.len() >= 2);
                    return match shared {
                        Some(v) => Resolution::Ambiguous(v),
                        None => Resolution::Unknown,
                    };
                }
                Resolution::Ambiguous(unconditional)
            }
        }
    }

    /// Beans Spring treats as self-references of the bean receiving `p`
    /// (`DefaultListableBeanFactory.isSelfReference`): that bean itself and,
    /// for a class bean, the beans of its own instance `@Bean` methods.
    fn self_references(&self, p: &InjectionPoint) -> Vec<usize> {
        let own = |b: &Bean| b.declaring_class == p.owner;
        if let PointKind::FactoryParam(m, _) = &p.kind {
            return (0..self.beans.len()).filter(|&i| own(&self.beans[i]) && matches!(&self.beans[i].source, BeanSource::FactoryMethod(n) if n == m)).collect();
        }
        if !self.beans.iter().any(|b| own(b) && !matches!(b.source, BeanSource::FactoryMethod(_))) {
            return Vec::new();
        }
        // static @Bean methods have no factory bean
        (0..self.beans.len())
            .filter(|&i| {
                let b = &self.beans[i];
                own(b)
                    && match &b.source {
                        BeanSource::FactoryMethod(_) => !b.static_factory,
                        _ => true,
                    }
            })
            .collect()
    }

    /// Could we see every provider of this type? Only types declared in the
    /// project's sources (not library types usually provided by
    /// auto-configuration) qualify.
    fn provable_type(&self, lk: &dyn ClassLookup, ty: &Ty) -> bool {
        let Some(n) = ty.class_name() else { return false };
        // container infrastructure (contexts, environments, conversion services...)
        // is registered programmatically by the framework itself
        if is_framework_class(n) {
            return false;
        }
        let Some(c) = lk.class(n) else { return false };
        c.is_source() && !matches!(c.kind, ClassKind::Annotation)
    }

    /// A @Bean method declared to return a supertype of `ty` may return a `ty`
    /// instance (`@Bean Store store() { return new RedisStore(); }`), which
    /// Spring matches by the created instance's class.
    fn factory_may_return(&self, lk: &dyn ClassLookup, ty: &Ty) -> bool {
        let Some(want) = ty.class_name() else { return false };
        self.beans.iter().any(|b| {
            matches!(b.source, BeanSource::FactoryMethod(_)) && b.autowire_candidate && b.ty.class_name().is_some_and(|r| r != want && r != "java.lang.Object" && lk.is_subclass(want, r).is_yes())
        })
    }

    /// "No bean of this type" needs positive evidence: the model must have
    /// seen how every class or method that could provide the type is (or is
    /// not) registered. When one of them is a registration it could not
    /// turn into a bean (see `unseen`), or Spring XML may define beans, the
    /// answer is unknown.
    fn may_be_registered_unseen(&self, lk: &dyn ClassLookup, p: &InjectionPoint, include_test: bool) -> bool {
        if (self.xml_config || (include_test && self.xml_config_in_tests)) && p.framework == Framework::Spring {
            return true;
        }
        let Some(want) = p.ty.class_name() else { return true };
        self.unseen.iter().any(|(n, is_test)| (include_test || !is_test) && (&**n == want || lk.is_subclass(n, want).is_yes()))
            // a producer declared to return a supertype may return a subtype
            || self.unseen_producers.iter().any(|(n, is_test)| {
                (include_test || !is_test) && (&**n == want || lk.is_subclass(n, want).is_yes() || (&**n != "java.lang.Object" && lk.is_subclass(want, n).is_yes()))
            })
    }

    /// Constructor-injection dependency edges between component classes.
    pub fn constructor_edges(&self, lk: &dyn ClassLookup) -> Vec<(Name, Name, usize)> {
        let mut out = Vec::new();
        for (pi, p) in self.points.iter().enumerate() {
            if !matches!(p.kind, PointKind::ConstructorParam(_)) || p.lazy || p.optional || p.multiple {
                continue;
            }
            if let Resolution::Unique(b) = self.resolve(lk, p, false) {
                let bean = &self.beans[b];
                if bean.source == BeanSource::Component {
                    out.push((p.owner.clone(), bean.declaring_class.clone(), pi));
                }
            }
        }
        out
    }

    /// Beans sharing a name (Spring refuses to start without overriding enabled).
    fn dynamically_registered(&self, ty: &Ty, include_test: bool) -> bool {
        ty.class_name().is_some_and(|c| self.dynamic_types.contains(simple_name_of(c)) || (include_test && self.dynamic_types_in_tests.contains(simple_name_of(c))))
    }

    /// Is the class registered in an application context the model can name?
    /// Elsewhere (a library, a context only tests or XML assemble) its
    /// dependencies are provided by a context the model cannot see.
    pub fn in_application(&self, class: &str) -> bool {
        self.contexts.iter().any(|c| c.members.contains(class))
    }

    /// Is the bean registered in the context (in a test run with `test`)?
    fn context_holds(&self, ctx: &AppContext, b: &Bean, test: bool) -> bool {
        ctx.holds(&b.declaring_class, test)
    }

    /// Pairs of beans with the same name in one application context
    /// (Spring refuses to start without overriding enabled). Only beans
    /// certainly registered in the same context clash: configurations that
    /// separate contexts load (one per test, one per sample) do not.
    pub fn name_conflicts(&self) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = Vec::new();
        for ctx in &self.contexts {
            let mut seen: FxHashMap<&str, usize> = FxHashMap::default();
            for (i, b) in self.beans.iter().enumerate() {
                if b.framework != Framework::Spring || b.conditional || b.is_test || !self.context_holds(ctx, b, false) {
                    continue;
                }
                match seen.get(b.name.as_str()) {
                    Some(&j) => out.push((j, i)),
                    None => {
                        seen.insert(&b.name, i);
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grounds_syntax::ParsedFile;

    fn model(files: &[(&str, &str)]) -> (Index, DiModel) {
        model_with(files, &[], &[])
    }

    fn model_with(files: &[(&str, &str)], registered: &[String], auto_registered: &[String]) -> (Index, DiModel) {
        let cp = Arc::new(Classpath::build(grounds_classfile::find_java_home().as_deref(), 21, &[]));
        let mut idx = Index::new(cp);
        let stubs = [
            ("org/springframework/stereotype/Component.java", "package org.springframework.stereotype; public @interface Component { String value() default \"\"; }"),
            ("org/springframework/stereotype/Service.java", "package org.springframework.stereotype; @Component public @interface Service { String value() default \"\"; }"),
            ("org/springframework/beans/factory/annotation/Qualifier.java", "package org.springframework.beans.factory.annotation; public @interface Qualifier { String value() default \"\"; }"),
            ("org/springframework/context/annotation/Bean.java", "package org.springframework.context.annotation; public @interface Bean { String[] name() default {}; }"),
            ("org/springframework/context/annotation/Configuration.java", "package org.springframework.context.annotation; @org.springframework.stereotype.Component public @interface Configuration {}"),
            ("org/springframework/context/annotation/Primary.java", "package org.springframework.context.annotation; public @interface Primary {}"),
        ];
        for (p, s) in stubs.iter().chain(files.iter()) {
            let pf = ParsedFile::parse(*s);
            idx.set_file(Arc::from(*p), Arc::new(extract(&pf)));
        }
        let m = DiModel::build_with(&idx, registered, auto_registered);
        (idx, m)
    }

    #[test]
    fn a_library_leaves_wiring_to_its_application() {
        // no application class in main sources: the application using the
        // library provides the missing beans. An auto-configuration scanning
        // the library's packages does not make it an application.
        let (idx, m) = model(&[(
            "lib/A.java",
            "package lib; import org.springframework.stereotype.*;
             interface Store {}
             @Service class DbStore implements Store {}
             @Service class MemStore implements Store {}
             interface Missing {}
             @Service class Uses { Uses(Store s, Missing m) {} }
             @org.springframework.boot.autoconfigure.AutoConfiguration
             @org.springframework.context.annotation.ComponentScan(\"lib\")
             class LibAutoConfiguration {}",
        )]);
        assert!(!m.spring_application);
        assert!(m.points.iter().all(|p| m.resolve(&idx, p, false) == Resolution::Unknown));
        assert!(m.name_conflicts().is_empty());
        // a plain Spring application: @ComponentScan next to `main`
        let (_, app) = model(&[("app/Main.java", "package app; @org.springframework.context.annotation.ComponentScan class Main { public static void main(String[] args) {} }")]);
        assert!(app.spring_application);
    }

    #[test]
    fn resolves_spring_beans() {
        let (idx, m) = model(&[(
            "app/A.java",
            "package app; import org.springframework.stereotype.*; import org.springframework.beans.factory.annotation.Qualifier; import org.springframework.context.annotation.*;
             interface Store {}
             @Service class DbStore implements Store {}
             @Service(\"mem\") class MemStore implements Store {}
             @Service class Uses { Uses(@Qualifier(\"mem\") Store s, Store dbStore) {} }
             @Service class Broken { Broken(Store s, @Qualifier(\"nope\") Store t, Missing m) {} }
             interface Missing {}
             @Configuration class Cfg { @Bean @Primary Clock clock() { return null; } @Bean Clock other() { return null; } }
             interface Clock {}
             @Service class NeedsClock { NeedsClock(Clock c) {} }
             @org.springframework.boot.autoconfigure.SpringBootApplication class App { public static void main(String[] args) {} }",
        )]);
        assert_eq!(m.beans.len(), 9);
        let res: Vec<(String, Resolution)> = m.points.iter().map(|p| (format!("{}.{}", simple_name_of(&p.owner), p.name), m.resolve(&idx, p, false))).collect();
        let get = |k: &str| res.iter().find(|(n, _)| n == k).unwrap().1.clone();
        assert!(matches!(get("Uses.s"), Resolution::Unique(_)));
        assert!(matches!(get("Uses.dbStore"), Resolution::Unique(_)));
        assert!(matches!(get("Broken.s"), Resolution::Ambiguous(_)));
        assert!(matches!(get("Broken.t"), Resolution::QualifierNotFound { .. }));
        assert_eq!(get("Broken.m"), Resolution::NotFound);
        assert!(matches!(get("NeedsClock.c"), Resolution::Unique(_)));
    }

    const CDI: [(&str, &str); 2] = [
        ("jakarta/inject/Inject.java", "package jakarta.inject; public @interface Inject {}"),
        ("jakarta/enterprise/context/ApplicationScoped.java", "package jakarta.enterprise.context; public @interface ApplicationScoped {}"),
    ];

    fn resolution_of(idx: &Index, m: &DiModel, owner_file: &str, name: &str) -> Resolution {
        let p = m.points.iter().find(|p| p.name == name && p.file.as_deref() == Some(owner_file)).expect("injection point");
        m.resolve(idx, p, false)
    }

    #[test]
    fn a_builder_application_is_an_application() {
        // main starts the context; @ComponentScan is on another class
        let (idx, m) = model(&[
            ("app/src/main/java/demo/Main.java", "package demo; import org.springframework.boot.builder.SpringApplicationBuilder;
             public class Main { public static void main(String[] args) { new SpringApplicationBuilder(Config.class).run(args); } }"),
            ("app/src/main/java/demo/Config.java", "package demo; @org.springframework.context.annotation.Configuration @org.springframework.context.annotation.ComponentScan(\"demo\") class Config {}"),
            ("app/src/main/java/demo/Uses.java", "package demo; interface Missing {} @org.springframework.stereotype.Service class Uses { Uses(Missing m) {} }"),
        ]);
        assert_eq!(resolution_of(&idx, &m, "app/src/main/java/demo/Uses.java", "m"), Resolution::NotFound);
    }

    #[test]
    fn an_unrelated_application_leaves_a_library_alone() {
        let (idx, m) = model(&[
            ("app/src/main/java/other/App.java", "package other; @org.springframework.boot.autoconfigure.SpringBootApplication public class App { public static void main(String[] args) {} }"),
            ("lib/src/main/java/demo/Config.java", "package demo; @org.springframework.context.annotation.ComponentScan(\"demo\") class Config {}"),
            ("lib/src/main/java/demo/Uses.java", "package demo; interface Supplied {} @org.springframework.stereotype.Service class Uses { Uses(Supplied s) {} }"),
            // a library the application scans is judged
            ("core/src/main/java/other/core/Svc.java", "package other.core; interface Missing {} @org.springframework.stereotype.Service class Svc { Svc(Missing m) {} }"),
        ]);
        assert_eq!(resolution_of(&idx, &m, "lib/src/main/java/demo/Uses.java", "s"), Resolution::Unknown);
        assert_eq!(resolution_of(&idx, &m, "core/src/main/java/other/core/Svc.java", "m"), Resolution::NotFound);
    }

    #[test]
    fn an_application_scan_skips_auto_configurations() {
        // a nested plugin project shares the application's base package:
        // @SpringBootApplication's scan excludes auto-configurations, whose
        // beans the plugin's consumers provide
        let files = [
            ("org/springframework/boot/autoconfigure/AutoConfiguration.java", "package org.springframework.boot.autoconfigure; @org.springframework.context.annotation.Configuration public @interface AutoConfiguration {}"),
            ("shop/src/main/java/com/example/shop/App.java", "package com.example.shop; @org.springframework.boot.autoconfigure.SpringBootApplication public class App { public static void main(String[] args) {} }"),
            ("shop/plugins/core/src/main/java/com/example/shop/plugin/Extension.java", "package com.example.shop.plugin; public interface Extension<T> {}"),
            (
                "shop/plugins/core/src/main/java/com/example/shop/plugin/PluginAutoConfiguration.java",
                "package com.example.shop.plugin; import org.springframework.context.annotation.Bean;
                 @org.springframework.boot.autoconfigure.AutoConfiguration
                 public class PluginAutoConfiguration { @Bean Object client(Extension<?> extension) { return null; } }",
            ),
            ("shop/plugins/core/src/main/java/com/example/shop/plugin/Registered.java", "package com.example.shop.plugin; @org.springframework.context.annotation.Configuration public class Registered { @org.springframework.context.annotation.Bean Object other(Extension<?> c) { return null; } }"),
            // registered under another key (spring.factories listeners and
            // the like): the scan still picks it up
            ("shop/plugins/core/src/main/java/com/example/shop/plugin/Listed.java", "package com.example.shop.plugin; @org.springframework.context.annotation.Configuration public class Listed { @org.springframework.context.annotation.Bean Object listed(Extension<?> l) { return null; } }"),
            // a plain @ComponentScan does not skip them
            ("other/src/main/java/com/example/other/Main.java", "package com.example.other; @org.springframework.context.annotation.ComponentScan public class Main { public static void main(String[] args) {} }"),
            ("other/src/main/java/com/example/other/OtherAutoConfiguration.java", "package com.example.other; interface Missing {} @org.springframework.boot.autoconfigure.AutoConfiguration class OtherAutoConfiguration { @org.springframework.context.annotation.Bean Object x(Missing m) { return null; } }"),
        ];
        let registered = ["com.example.shop.plugin.Registered".to_string(), "com.example.shop.plugin.Listed".to_string()];
        let (idx, m) = model_with(&files, &registered, &registered[..1]);
        assert_eq!(resolution_of(&idx, &m, "shop/plugins/core/src/main/java/com/example/shop/plugin/PluginAutoConfiguration.java", "extension"), Resolution::Unknown);
        assert_eq!(resolution_of(&idx, &m, "shop/plugins/core/src/main/java/com/example/shop/plugin/Registered.java", "c"), Resolution::Unknown);
        assert_eq!(resolution_of(&idx, &m, "shop/plugins/core/src/main/java/com/example/shop/plugin/Listed.java", "l"), Resolution::NotFound);
        assert_eq!(resolution_of(&idx, &m, "other/src/main/java/com/example/other/OtherAutoConfiguration.java", "m"), Resolution::NotFound);
    }

    #[test]
    fn test_sets_are_classified_like_the_workspace() {
        // a functionalTest application does not make the library's main code an application
        let (idx, m) = model(&[
            ("src/functionalTest/java/demo/TestApp.java", "package demo; @org.springframework.boot.autoconfigure.SpringBootApplication public class TestApp { public static void main(String[] args) {} }"),
            ("src/main/java/demo/Uses.java", "package demo; interface Supplied {} @org.springframework.stereotype.Service class Uses { Uses(Supplied s) {} }"),
            ("src/e2e/java/demo/E2e.java", "package demo; interface Missing2 {} @org.springframework.stereotype.Service class E2e { E2e(Missing2 m) {} }"),
        ]);
        assert!(!m.spring_application);
        assert_eq!(resolution_of(&idx, &m, "src/main/java/demo/Uses.java", "s"), Resolution::Unknown);
        // test code is judged as before
        assert_eq!(m.resolve(&idx, m.points.iter().find(|p| p.name == "m").unwrap(), true), Resolution::NotFound);
    }

    #[test]
    fn jakarta_data_repositories_are_beans() {
        // jakarta.data is not on the classpath: the annotation is judged by the import
        let user = "package app; import jakarta.inject.Inject; import jakarta.enterprise.context.ApplicationScoped;
             @ApplicationScoped class Resource { @Inject FruitRepository repository; @Inject Other other; }";
        let (idx, m) = model(&[CDI[0], CDI[1],
            ("app/FruitRepository.java", "package app; import jakarta.data.repository.Repository; @Repository public interface FruitRepository {}"),
            ("app/Other.java", "package app; public interface Other {}"),
            ("app/Resource.java", user)]);
        assert!(matches!(resolution_of(&idx, &m, "app/Resource.java", "repository"), Resolution::Unique(_)));
        assert_eq!(resolution_of(&idx, &m, "app/Resource.java", "other"), Resolution::NotFound);
    }

    #[test]
    fn classes_duplicated_across_modules_are_not_judged() {
        // two independent applications in one repository, same class names;
        // the index keeps one `app.Resource`, so neither is judged
        let a = "package app; import jakarta.inject.Inject; import jakarta.enterprise.context.ApplicationScoped;
             @ApplicationScoped class Resource { @Inject Store store; }";
        let b = "package app; import jakarta.inject.Inject; import jakarta.enterprise.context.ApplicationScoped;
             @ApplicationScoped class Resource { @Inject Cache cache; }";
        let (idx, m) = model(&[CDI[0], CDI[1],
            ("a/src/main/java/app/Resource.java", a),
            ("a/src/main/java/app/Store.java", "package app; public interface Store {}"),
            ("b/src/main/java/app/Resource.java", b),
            ("b/src/main/java/app/Cache.java", "package app; public interface Cache {}")]);
        for p in &m.points {
            assert_eq!(m.resolve(&idx, p, false), Resolution::Unknown, "{}", p.name);
        }
    }

    #[test]
    fn a_left_out_copy_may_provide_the_bean() {
        // the index keeps a's `app.Impl`; b's copy implements Repo
        let (idx, m) = model(&[
            ("a/src/main/java/app/Impl.java", "package app; @org.springframework.stereotype.Service public class Impl {}"),
            ("b/src/main/java/app/Impl.java", "package app; @org.springframework.stereotype.Service public class Impl implements Repo {}"),
            ("b/src/main/java/app/Repo.java", "package app; public interface Repo {}"),
            ("b/src/main/java/app/Consumer.java", "package app; @org.springframework.stereotype.Service class Consumer { Consumer(Repo repo) {} }"),
        ]);
        assert_eq!(resolution_of(&idx, &m, "b/src/main/java/app/Consumer.java", "repo"), Resolution::Unknown);
    }

    #[test]
    fn duplicates_that_provide_nothing_suppress_nothing() {
        let (idx, m) = model(&[
            ("one/src/main/java/sample/Example.java", "package sample; import java.time.Clock; public class Example { Clock c; }"),
            ("two/src/main/java/sample/Example.java", "package sample; import java.time.Clock; public class Example { Clock c; }"),
            ("app/src/main/java/billing/Clock.java", "package billing; public interface Clock {}"),
            ("app/src/main/java/billing/Uses.java", "package billing; @org.springframework.stereotype.Service class Uses { Uses(Clock clock) {} }"),
            ("app/src/main/java/billing/App.java", "package billing; @org.springframework.boot.autoconfigure.SpringBootApplication public class App { public static void main(String[] args) {} }"),
        ]);
        assert_eq!(resolution_of(&idx, &m, "app/src/main/java/billing/Uses.java", "clock"), Resolution::NotFound);
    }

    #[test]
    fn a_left_out_copy_provides_supertypes_too() {
        let (idx, m) = model(&[
            ("a/src/main/java/app/Impl.java", "package app; @org.springframework.stereotype.Service public class Impl {}"),
            ("b/src/main/java/app/Impl.java", "package app; @org.springframework.stereotype.Service public class Impl implements SubRepo {}"),
            ("b/src/main/java/app/SubRepo.java", "package app; public interface SubRepo extends Repo {}"),
            ("b/src/main/java/app/Repo.java", "package app; public interface Repo {}"),
            ("b/src/main/java/app/Consumer.java", "package app; @org.springframework.stereotype.Service class Consumer { Consumer(Repo repo) {} }"),
        ]);
        assert_eq!(resolution_of(&idx, &m, "b/src/main/java/app/Consumer.java", "repo"), Resolution::Unknown);
    }

    #[test]
    fn providers_do_not_fail_at_startup() {
        let (idx, m) = model(&[
            ("org/springframework/beans/factory/ObjectProvider.java", "package org.springframework.beans.factory; public interface ObjectProvider<T> {}"),
            ("jakarta/inject/Provider.java", "package jakarta.inject; public interface Provider<T> { T get(); }"),
            ("app/App.java", "package app; @org.springframework.boot.autoconfigure.SpringBootApplication public class App { public static void main(String[] args) {} }"),
            ("app/A.java", "package app; import org.springframework.stereotype.Service; import org.springframework.beans.factory.ObjectProvider; import jakarta.inject.Provider;
             interface Configurer {} interface Missing {}
             @Service class One implements Configurer {} @Service class Two implements Configurer {}
             @Service class Uses { Uses(ObjectProvider<Configurer> all, Provider<Configurer> one, Provider<Missing> missing, Configurer direct,
                 @org.springframework.beans.factory.annotation.Qualifier(\"absent\") Provider<Missing> qualified) {} }"),
        ]);
        assert_eq!(resolution_of(&idx, &m, "app/A.java", "all"), Resolution::Unknown);
        assert_eq!(resolution_of(&idx, &m, "app/A.java", "one"), Resolution::Unknown);
        assert_eq!(resolution_of(&idx, &m, "app/A.java", "missing"), Resolution::Unknown);
        assert!(matches!(resolution_of(&idx, &m, "app/A.java", "direct"), Resolution::Ambiguous(_)));
        assert_eq!(resolution_of(&idx, &m, "app/A.java", "qualified"), Resolution::Unknown);
    }

    #[test]
    fn a_spring_repository_import_shadows_jakarta_data() {
        // neither jar is on the classpath: the single-type import wins
        let (idx, m) = model(&[
            ("app/Store.java", "package app; import org.springframework.stereotype.Repository; import jakarta.data.repository.*; @Repository public interface Store {}"),
        ]);
        assert!(m.beans.iter().all(|b| !(b.framework == Framework::Jakarta && simple_name_of(&b.declaring_class) == "Store")));
        let _ = idx;
    }

    #[test]
    fn spring_skips_self_references() {
        // DefaultListableBeanFactory.isSelfReference: the bean itself and the
        // beans of its own instance @Bean methods are candidates only when
        // nothing else matches a single-valued point
        let (idx, m) = model(&[(
            "app/A.java",
            "package app; import org.springframework.stereotype.*; import org.springframework.context.annotation.*; import java.util.List;
             interface A {}
             @Service class B implements A {}
             @Service class C implements A { C(A decorated) {} }
             @Service class D implements A { D(List<A> all) {} }
             interface Solo {}
             @Service class S implements Solo { S(Solo self) {} }
             interface K {}
             @Service class Plain implements K {}
             @Configuration class Cfg { @org.springframework.beans.factory.annotation.Autowired K fromCfg; @Bean K made() { return null; } @Bean K wrap(K inner) { return null; } }
             interface J {}
             @Configuration class StaticCfg { @org.springframework.beans.factory.annotation.Autowired J own; @Bean static J st() { return null; } J st(int helper) { return null; } @Bean J inst() { return null; } }
             interface H {}
             @Service class OtherH implements H {}
             class BaseCfg { @Bean H inherited() { return null; } }
             @Configuration class SubCfg extends BaseCfg { @org.springframework.beans.factory.annotation.Autowired H viaSuper; H inherited(int helper) { return null; } }
             interface G {}
             @Service class OtherG1 implements G {}
             @Service class OtherG2 implements G {}
             class MakeBase { @Bean(\"primaryG\") @Primary static G make() { return null; } @Bean(\"number\") Integer make(String unused) { return 1; } }
             @Configuration class MakeCfg extends MakeBase { @org.springframework.beans.factory.annotation.Autowired G viaStatic; }
             interface F {}
             @Service @Primary class ExternalF implements F {}
             class FBase { @Bean(\"ownF\") @Primary F make(int unused) { return null; } }
             @Configuration class FCfg extends FBase { @Bean(\"staticF\") static F make() { return null; } @Bean(\"numberF\") Integer make(String unused) { return 2; } @org.springframework.beans.factory.annotation.Autowired F viaHierarchy; }
             @ComponentScan class Main { public static void main(String[] args) {} }",
        )]);
        let bean = |n: &str| m.beans.iter().position(|b| b.name == n).unwrap();
        let sorted = |r: Resolution| match r {
            Resolution::Ambiguous(mut v) => Resolution::Ambiguous({ v.sort(); v }),
            Resolution::Multiple(mut v) => Resolution::Multiple({ v.sort(); v }),
            r => r,
        };
        let res = |n: &str| sorted(resolution_of(&idx, &m, "app/A.java", n));
        assert_eq!(res("decorated"), sorted(Resolution::Ambiguous(vec![bean("b"), bean("d")])));
        assert_eq!(res("all"), sorted(Resolution::Multiple(vec![bean("b"), bean("c")])));
        assert_eq!(resolution_of(&idx, &m, "app/A.java", "self"), Resolution::Unique(bean("s")));
        assert_eq!(resolution_of(&idx, &m, "app/A.java", "fromCfg"), Resolution::Unique(bean("plain")));
        assert_eq!(res("inner"), sorted(Resolution::Ambiguous(vec![bean("plain"), bean("made")])));
        assert_eq!(resolution_of(&idx, &m, "app/A.java", "own"), Resolution::Unique(bean("st")));
        // an inherited instance @Bean is the config's own, beside a helper overload
        assert_eq!(resolution_of(&idx, &m, "app/A.java", "viaSuper"), Resolution::Unique(bean("otherH")));
        // an inherited static @Bean is no self-reference beside an instance one of the same name
        assert_eq!(resolution_of(&idx, &m, "app/A.java", "viaStatic"), Resolution::Unique(bean("primaryG")));
        // the inherited instance factory is a self-reference, the static one beside it is not
        assert_eq!(resolution_of(&idx, &m, "app/A.java", "viaHierarchy"), Resolution::Unique(bean("externalF")));
        // the issue's case: a decorator wrapping the only other implementation
        let (idx, m) = model(&[(
            "app/A.java",
            "package app; import org.springframework.stereotype.*;
             interface A {}
             @Service class B implements A {}
             @Service class C implements A { C(A x) {} }
             @org.springframework.context.annotation.ComponentScan class Main { public static void main(String[] args) {} }",
        )]);
        assert_eq!(resolution_of(&idx, &m, "app/A.java", "x"), Resolution::Unique(m.beans.iter().position(|b| b.name == "b").unwrap()));
    }

    const JSR330: [(&str, &str); 3] = [
        ("jakarta/inject/Inject.java", "package jakarta.inject; public @interface Inject {}"),
        ("jakarta/inject/Scope.java", "package jakarta.inject; public @interface Scope {}"),
        ("jakarta/inject/Singleton.java", "package jakarta.inject; @Scope public @interface Singleton {}"),
    ];

    /// JSR-330 classes with a missing binding, two implementations of one
    /// interface and a constructor cycle through interfaces, plus `extra`.
    fn jsr330_project(extra: &[(&'static str, &'static str)]) -> (Index, DiModel) {
        let mut files: Vec<(&str, &str)> = JSR330.to_vec();
        files.push((
            "src/main/java/app/A.java",
            "package app; import jakarta.inject.*;
             interface Store {}
             interface Engine {}
             @Singleton class V8 implements Engine {}
             @Singleton class V6 implements Engine {}
             interface Ping {} interface Pong {}
             @Singleton class PingImpl implements Ping { @Inject PingImpl(Pong pong) {} }
             @Singleton class PongImpl implements Pong { @Inject PongImpl(Ping ping) {} }
             @Singleton class Uses { @Inject Uses(Store store, Engine engine) {} }",
        ));
        files.push(("src/test/java/app/UsesTest.java", "package app; import jakarta.inject.*; @Singleton class InTest { @Inject InTest(Store testStore) {} }"));
        files.extend_from_slice(extra);
        model(&files)
    }

    fn point_resolution(idx: &Index, m: &DiModel, name: &str) -> Resolution {
        let p = m.points.iter().find(|p| p.name == name).expect("injection point");
        m.resolve(idx, p, p.file.as_deref().is_some_and(is_test_path))
    }

    #[test]
    fn jsr330_wiring_is_judged_when_the_model_implements_the_container() {
        let (idx, m) = jsr330_project(&[]);
        assert_eq!(point_resolution(&idx, &m, "store"), Resolution::NotFound);
        assert!(matches!(point_resolution(&idx, &m, "engine"), Resolution::Ambiguous(_)));
        assert_eq!(point_resolution(&idx, &m, "testStore"), Resolution::NotFound);
        assert_eq!(m.ctor_edges.len(), 2);
    }

    #[test]
    fn a_container_the_model_does_not_implement_leaves_jsr330_wiring_unknown() {
        // a Guice/Sisu module, an HK2 binder, a Dagger module, Maven 4 DI or
        // a Quarkus build step may bind any type: nothing missing, no
        // ambiguity (Sisu picks the highest-ranked implementation), no
        // cycle (Guice proxies interfaces)
        for evidence in [
            "package app.wiring; import com.google.inject.AbstractModule; public class Wiring extends AbstractModule {}",
            "package app.wiring; import com.google.inject.*; public class Wiring extends AbstractModule {}",
            "package app.wiring; import org.eclipse.sisu.space.SpaceModule; public class Wiring {}",
            "package app.wiring; import org.glassfish.hk2.utilities.binding.AbstractBinder; public class Wiring extends AbstractBinder {}",
            "package app.wiring; import dagger.Module; @Module public class Wiring {}",
            "package app.wiring; import org.apache.maven.api.di.Provides; public class Wiring {}",
            "package app.wiring; import io.quarkus.runtime.annotations.Recorder; @Recorder public class Wiring {}",
        ] {
            let (idx, m) = jsr330_project(&[("src/main/java/app/wiring/Wiring.java", evidence)]);
            assert!(m.unmodeled_container.is_some(), "{evidence}");
            for name in ["store", "engine", "testStore", "pong", "ping"] {
                assert_eq!(point_resolution(&idx, &m, name), Resolution::Unknown, "{name}: {evidence}");
            }
            assert!(m.ctor_edges.is_empty(), "{evidence}");
        }
        // a Guice module in the tests wires the tests only
        let (idx, m) = jsr330_project(&[("src/test/java/app/TestWiring.java", "package app; import com.google.inject.AbstractModule; class TestWiring extends AbstractModule {}")]);
        assert_eq!(point_resolution(&idx, &m, "store"), Resolution::NotFound);
        assert_eq!(point_resolution(&idx, &m, "testStore"), Resolution::Unknown);
        // ... and so does one in a module that exists only for testing
        let (idx, m) = jsr330_project(&[("tests/fixtures/src/main/java/fixture/Wiring.java", "package fixture; import com.google.inject.AbstractModule; class Wiring extends AbstractModule {}")]);
        assert!(m.unmodeled_container.is_none());
        assert_eq!(point_resolution(&idx, &m, "store"), Resolution::NotFound);
        assert_eq!(point_resolution(&idx, &m, "testStore"), Resolution::Unknown);
    }

    #[test]
    fn spring_wiring_is_judged_beside_an_unmodeled_container() {
        // a Guice module binds nothing into a Spring context
        let (idx, m) = model(&[
            ("src/main/java/app/A.java", "package app; import org.springframework.stereotype.*;
             interface Missing {}
             @Service class Uses { Uses(Missing missing) {} }
             @org.springframework.boot.autoconfigure.SpringBootApplication class App { public static void main(String[] args) {} }"),
            ("src/main/java/app/Wiring.java", "package app; import com.google.inject.AbstractModule; class Wiring extends AbstractModule {}"),
        ]);
        assert_eq!(point_resolution(&idx, &m, "missing"), Resolution::NotFound);
    }

    #[test]
    fn micronaut_scopes_define_beans() {
        // @RequestScope (from a jar that is not on the classpath) and a
        // project scope meta-annotated @Scope make beans like @Singleton
        let (idx, m) = model(&[
            JSR330[0], JSR330[1], JSR330[2],
            ("src/main/java/app/PerTenant.java", "package app; @jakarta.inject.Scope public @interface PerTenant {}"),
            ("src/main/java/app/A.java", "package app; import jakarta.inject.*; import io.micronaut.runtime.http.scope.RequestScope;
             abstract class Wrapper {}
             @RequestScope class RequestWrapper extends Wrapper {}
             interface Tenant {}
             @PerTenant class CurrentTenant implements Tenant {}
             @Singleton class Uses { @Inject Uses(Wrapper wrapper, Tenant tenant) {} }"),
            ("src/main/java/app/Micronaut.java", "package app; import io.micronaut.runtime.Micronaut; class Application {}"),
        ]);
        assert!(matches!(point_resolution(&idx, &m, "wrapper"), Resolution::Unique(_)));
        assert!(matches!(point_resolution(&idx, &m, "tenant"), Resolution::Unique(_)));
        // only JSR-330's @Scope, by its full name, even when it does not
        // resolve; an unresolved Spring @Scope on a composed annotation is
        // not one
        let (idx, m) = model(&[
            ("src/main/java/app/PerTenant.java", "package app; import jakarta.inject.Scope; @Scope public @interface PerTenant {}"),
            ("src/main/java/app/Odd.java", "package app; import org.springframework.context.annotation.Scope; @Scope(\"request\") public @interface Odd {}"),
            ("src/main/java/app/A.java", "package app; import jakarta.inject.*;
             interface Tenant {}
             @PerTenant class CurrentTenant implements Tenant {}
             interface Session {}
             @Odd class OddSession implements Session {}
             @Singleton class Uses { @Inject Uses(Tenant tenant, Session session) {} }"),
        ]);
        assert!(matches!(point_resolution(&idx, &m, "tenant"), Resolution::Unique(_)));
        assert_eq!(point_resolution(&idx, &m, "session"), Resolution::NotFound);
    }

    #[test]
    fn a_bean_method_and_a_component_of_the_same_name_are_one_bean() {
        // ConfigurationClassBeanDefinitionReader.isOverriddenByExistingDefinition:
        // since Spring 6.1 a @Bean method returning the scanned class replaces
        // it even with overriding disabled; otherwise registration fails
        // (overriding disabled) or the @Bean wins (enabled). Never two beans.
        let (idx, m) = model(&[
            ("src/main/java/app/A.java", "package app; import org.springframework.stereotype.*; import org.springframework.context.annotation.*;
             @Component class Registry {}
             @Configuration class RegistryConfig { @Bean Registry registry() { return new Registry(); } }
             interface Store {}
             @Component class Store1 implements Store {}
             @Configuration class StoreConfig { @Bean Object store1() { return new Object(); } }
             @Component class Cache {}
             @Component class Other {}
             @Service class Uses { Uses(Registry reg, Store1 st, Cache cache, Other o) {} }
             @org.springframework.boot.autoconfigure.SpringBootApplication class App { public static void main(String[] args) {} }"),
            // a test configuration's @Bean of the same name exists only in tests
            ("src/test/java/app/TestConfig.java", "package app; import org.springframework.context.annotation.*;
             @Configuration class TestConfig { @Bean Other other() { return new Other(); } }"),
        ]);
        let bean = |n: &str, factory: bool| m.beans.iter().position(|b| b.name == n && matches!(b.source, BeanSource::FactoryMethod(_)) == factory).unwrap();
        assert_eq!(point_resolution(&idx, &m, "reg"), Resolution::Unique(bean("registry", true)));
        // the @Bean of another type replaces the component or fails startup
        // (bean-name-conflict), depending on a setting we may not see
        assert_eq!(point_resolution(&idx, &m, "st"), Resolution::Unknown);
        assert_eq!(point_resolution(&idx, &m, "cache"), Resolution::Unique(bean("cache", false)));
        assert_eq!(point_resolution(&idx, &m, "o"), Resolution::Unique(bean("other", false)));
    }

    #[test]
    fn an_override_of_an_inherited_bean_method_defines_the_bean() {
        // Spring registers a superclass's @Bean methods on the configuration
        // and calls the override, which needs no @Bean of its own
        let (idx, m) = model(&[(
            "src/test/java/app/A.java",
            "package app; import org.springframework.stereotype.*; import org.springframework.context.annotation.*; import org.springframework.beans.factory.annotation.Qualifier;
             interface Props {}
             @Component class DefaultProps implements Props {}
             abstract class BaseConfig { @Bean(name = \"testProps\") public abstract Props props(); }
             @Configuration class TestConfig extends BaseConfig { @Override public Props props() { return null; } }
             @Service class Uses { Uses(@Qualifier(\"testProps\") Props p) {} }",
        )]);
        let test_props = m.beans.iter().position(|b| b.name == "testProps").expect("bean from the inherited @Bean method");
        assert_eq!(point_resolution(&idx, &m, "p"), Resolution::Unique(test_props));
        // the override keeps its own annotations
        let (idx, m) = model(&[(
            "src/test/java/app/A.java",
            "package app; import org.springframework.stereotype.*; import org.springframework.context.annotation.*;
             interface Props {}
             @Component class DefaultProps implements Props {}
             abstract class BaseConfig { @Bean(name = \"testProps\") public abstract Props props(); }
             @Configuration class TestConfig extends BaseConfig { @Override @Primary public Props props() { return null; } }
             @Service class Uses { Uses(Props p) {} }",
        )]);
        let test_props = m.beans.iter().position(|b| b.name == "testProps").expect("bean from the inherited @Bean method");
        assert_eq!(point_resolution(&idx, &m, "p"), Resolution::Unique(test_props));
    }

    #[test]
    fn factory_methods_are_recognized_through_meta_annotations() {
        // a project annotation carrying @Singleton and @Bean (Micronaut) or
        // @Bean (Spring) makes the annotated factory method produce a bean,
        // as it makes an annotated class one
        let (idx, m) = model(&[
            JSR330[0], JSR330[1], JSR330[2],
            ("src/main/java/app/Channel.java", "package app; import io.micronaut.context.annotation.Bean; import jakarta.inject.Singleton;
             @Singleton @Bean(preDestroy = \"close\") public @interface Channel {}"),
            ("src/main/java/app/Plain.java", "package app; public @interface Plain {}"),
            ("src/main/java/app/A.java", "package app; import jakarta.inject.*; import io.micronaut.context.annotation.Factory;
             interface Pipe {}
             interface Hose {}
             @Factory class Pipes { @Channel Pipe pipe() { return null; } @Plain Hose hose() { return null; } }
             @Singleton class Uses { @Inject Uses(Pipe pipe, Hose hose) {} }"),
        ]);
        assert!(matches!(point_resolution(&idx, &m, "pipe"), Resolution::Unique(_)));
        // an annotation carrying neither makes no bean
        assert_eq!(point_resolution(&idx, &m, "hose"), Resolution::NotFound);
        let (idx, m) = model(&[
            ("src/main/java/app/Exported.java", "package app; @org.springframework.context.annotation.Bean public @interface Exported {}"),
            ("src/main/java/app/A.java", "package app; import org.springframework.stereotype.*; import org.springframework.context.annotation.*;
             interface Pipe {}
             @Configuration class Pipes { @Exported Pipe pipe() { return null; } }
             @Service class Uses { Uses(Pipe pipe) {} }
             @org.springframework.boot.autoconfigure.SpringBootApplication class App { public static void main(String[] args) {} }"),
        ]);
        assert!(matches!(point_resolution(&idx, &m, "pipe"), Resolution::Unique(_)));
    }

    #[test]
    fn scan_packages_follow_ant_patterns() {
        assert!(package_covered("app", "app") && package_covered("app", "app.x") && !package_covered("app", "apple"));
        assert!(package_covered("app.**.dao", "app.dao") && package_covered("app.**.dao", "app.order.dao") && package_covered("app.**.dao", "app.a.b.dao.impl"));
        assert!(!package_covered("app.**.dao", "app.order") && !package_covered("app.**.dao", "other.dao"));
        assert!(package_covered("app.*.web", "app.shop.web") && !package_covered("app.*.web", "app.a.b.web") && !package_covered("app.*.web", "app.web"));
        assert!(package_covered("app.mod*", "app.module.x") && !package_covered("app.mod*", "app.other"));
        assert!(package_covered("", "any.thing") && !package_covered("${scan.root}", "app"));
        // MyBatis registers every interface under a matching package; Spring
        // scans components under one, and splits a value at commas
        let (idx, m) = model(&[
            ("src/main/java/app/order/dao/OrderDao.java", "package app.order.dao; public interface OrderDao {}"),
            ("src/main/java/app/order/Draft.java", "package app.order; public interface Draft {}"),
            ("src/main/java/app/shop/web/Index.java", "package app.shop.web; public interface Index {} @org.springframework.stereotype.Component class IndexPage implements Index {}"),
            ("src/main/java/app/App.java", "package app; import org.springframework.stereotype.*; import app.order.dao.OrderDao; import app.order.Draft; import app.shop.web.Index;
             @org.mybatis.spring.annotation.MapperScan(\"app.**.dao\")
             @org.springframework.boot.autoconfigure.SpringBootApplication(scanBasePackages = \"app.config, app.*.web\") class App { public static void main(String[] args) {} }"),
            ("src/main/java/app/config/Uses.java", "package app.config; import org.springframework.stereotype.*; import app.order.dao.OrderDao; import app.order.Draft; import app.shop.web.Index;
             @Service class Uses { Uses(OrderDao orderDao, Draft draft, Index index) {} }"),
        ]);
        assert!(matches!(point_resolution(&idx, &m, "orderDao"), Resolution::Unique(_)));
        assert_eq!(point_resolution(&idx, &m, "draft"), Resolution::NotFound);
        let index = point_resolution(&idx, &m, "index");
        assert!(matches!(index, Resolution::Unique(i) if !m.beans[i].conditional), "{index:?}");
    }

    #[test]
    fn a_replacement_removes_a_bean_only_where_it_certainly_exists() {
        // a default implementation nested in its interface is a bean; a
        // test's @Replaces, a conditional replacement or one in another
        // module leaves it in place; an unconditional one beside it removes it
        let files = |extra: (&'static str, &'static str)| {
            vec![
                JSR330[0], JSR330[1], JSR330[2],
                ("core/src/main/java/app/Handler.java", "package app; import jakarta.inject.Singleton; import io.micronaut.context.annotation.Secondary;
                 public interface Handler { void handle();
                   @Singleton @Secondary class NoopHandler implements Handler { public void handle() {} } }"),
                ("core/src/main/java/app/Uses.java", "package app; import jakarta.inject.*; @Singleton class Uses { @Inject Uses(Handler handler, Handler.NoopHandler noop) {} }"),
                ("core/src/test/java/app/UsesTest.java", "package app; import jakarta.inject.*; import io.micronaut.context.annotation.Replaces;
                 class UsesTest { @Inject Handler testHandler;
                   @Replaces(Handler.NoopHandler.class) Handler mockHandler() { return null; } }
                 @Singleton class TestUses { @Inject TestUses(Handler.NoopHandler testNoop) {} }"),
                extra,
            ]
        };
        let (idx, m) = model(&files(("core/src/main/java/app/Other.java", "package app; class Other {}")));
        let noop = m.beans.iter().position(|b| b.declaring_class.ends_with("Handler$NoopHandler")).expect("nested bean class");
        assert_eq!(point_resolution(&idx, &m, "handler"), Resolution::Unique(noop));
        assert_eq!(point_resolution(&idx, &m, "testNoop"), Resolution::Unique(noop));
        for replacement in [
            ("core/src/main/java/app/Remote.java", "package app; import jakarta.inject.Singleton; import io.micronaut.context.annotation.*;
             @Singleton @Requires(property = \"mode\", value = \"remote\") @Replaces(Handler.NoopHandler.class) class RemoteHandler implements Handler { public void handle() {} }"),
            ("worker/src/main/java/app/Remote.java", "package app; import jakarta.inject.Singleton; import io.micronaut.context.annotation.*;
             @Singleton @Replaces(Handler.NoopHandler.class) class RemoteHandler implements Handler { public void handle() {} }"),
        ] {
            let (idx, m) = model(&files(replacement));
            let noop = m.beans.iter().position(|b| b.declaring_class.ends_with("Handler$NoopHandler")).unwrap();
            assert_eq!(point_resolution(&idx, &m, "noop"), Resolution::Unique(noop), "{}", replacement.0);
            assert!(matches!(point_resolution(&idx, &m, "handler"), Resolution::Unique(_) | Resolution::Unknown), "{}", replacement.0);
        }
        let (idx, m) = model(&files(("core/src/main/java/app/Remote.java", "package app; import jakarta.inject.Singleton; import io.micronaut.context.annotation.*;
             @Singleton @Replaces(Handler.NoopHandler.class) class RemoteHandler implements Handler { public void handle() {} }")));
        let remote = m.beans.iter().position(|b| b.declaring_class.ends_with("RemoteHandler")).unwrap();
        assert_eq!(point_resolution(&idx, &m, "handler"), Resolution::Unique(remote));
        assert_eq!(point_resolution(&idx, &m, "noop"), Resolution::NotFound);
    }

    #[test]
    fn a_micronaut_mock_bean_method_provides_the_bean_in_tests() {
        let (idx, m) = model(&[
            JSR330[0], JSR330[1], JSR330[2],
            ("src/main/java/app/Listener.java", "package app; public interface Listener { void on(String event); }"),
            ("src/test/java/app/HandlerTest.java", "package app; import jakarta.inject.Inject; import io.micronaut.test.annotation.MockBean;
             @io.micronaut.context.annotation.Factory class HandlerTest {
               @Inject Listener listener;
               @MockBean(Listener.class) Listener listener() { return null; }
             }"),
            ("src/test/java/app/OtherTest.java", "package app; import jakarta.inject.*; @Singleton class OtherTest { @Inject OtherTest(Listener otherListener) {} }"),
            ("src/main/java/app/Uses.java", "package app; import jakarta.inject.*; @Singleton class Uses { @Inject Uses(Listener mainListener) {} }"),
        ]);
        assert!(matches!(point_resolution(&idx, &m, "listener"), Resolution::Unique(_)));
        // main code never sees a test's mock
        assert_eq!(point_resolution(&idx, &m, "mainListener"), Resolution::NotFound);
    }

    #[test]
    fn cdi_and_faces_scopes_define_beans() {
        // normal scopes (Faces view and flow scopes, conversation, OmniFaces),
        // stereotypes (@Model, a project @Stereotype) and project normal scopes
        let (idx, m) = model(&[
            JSR330[0], JSR330[1], JSR330[2],
            ("src/main/java/app/Wizard.java", "package app; @jakarta.enterprise.inject.Stereotype @jakarta.enterprise.context.SessionScoped public @interface Wizard {}"),
            ("src/main/java/app/PerTab.java", "package app; import jakarta.enterprise.context.NormalScope; @NormalScope public @interface PerTab {}"),
            ("src/main/java/app/Views.java", "package app; import jakarta.inject.Named;
             interface Upload {} interface Cart {} interface Flow {} interface Steps {} interface Tab {} interface Old {}
             @Named @jakarta.faces.view.ViewScoped class UploadView implements Upload {}
             @jakarta.enterprise.inject.Model class CartModel implements Cart {}
             @org.omnifaces.cdi.ViewScoped class FlowView implements Flow {}
             @Wizard class StepsView implements Steps {}
             @PerTab class TabView implements Tab {}
             @javax.enterprise.context.ConversationScoped class OldView implements Old {}
             @jakarta.enterprise.context.RequestScoped class Page { @jakarta.inject.Inject Page(Upload upload, Cart cart, Flow flow, Steps steps, Tab tab, Old old) {} }"),
        ]);
        for name in ["upload", "cart", "flow", "steps", "tab", "old"] {
            assert!(matches!(point_resolution(&idx, &m, name), Resolution::Unique(_)), "{name}");
        }
    }

    #[test]
    fn no_bean_needs_evidence_of_how_beans_of_the_type_are_registered() {
        let app = ("src/main/java/app/App.java", "package app; @org.springframework.boot.autoconfigure.SpringBootApplication class App { public static void main(String[] args) {} }");
        let uses = ("src/main/java/app/Uses.java", "package app; import org.springframework.stereotype.*; @Service class Uses { Uses(Port port) {} }");
        let resolution = |files: &[(&'static str, &'static str)]| {
            let mut all = vec![app, uses];
            all.extend_from_slice(files);
            let (idx, m) = model(&all);
            point_resolution(&idx, &m, "port")
        };
        let port = ("src/main/java/app/Port.java", "package app; public interface Port {}");
        // nothing implements or produces it: certain
        assert_eq!(resolution(&[port]), Resolution::NotFound);
        // implementations with annotations known to register nothing, a plain
        // class and a plain class's leftover @Bean method: still certain
        assert_eq!(
            resolution(&[
                port,
                ("src/main/java/app/Impl.java", "package app; import lombok.Data; @Data @Deprecated class PortImpl implements Port {}"),
                ("src/main/java/app/Tagged.java", "package app; @java.lang.annotation.Retention(java.lang.annotation.RetentionPolicy.RUNTIME) @interface Tagged {}"),
                ("src/main/java/app/Other.java", "package app; @Tagged class OtherPort implements Port {}"),
                ("src/main/java/app/Forgotten.java", "package app; class Forgotten { @org.springframework.context.annotation.Bean Port port() { return null; } }"),
                ("src/test/java/app/TestPort.java", "package app; @com.acme.Registered class TestPort implements Port {}"),
            ]),
            Resolution::NotFound
        );
        // a registration the model cannot see through: unknown
        for (why, files) in [
            ("the interface carries a library's annotation", vec![("src/main/java/app/Port.java", "package app; import org.springframework.integration.annotation.MessagingGateway; @MessagingGateway public interface Port {}")]),
            ("an implementation carries one", vec![port, ("src/main/java/app/Impl.java", "package app; import com.acme.Registered; @Registered class PortImpl implements Port {}")]),
            ("through a project annotation", vec![
                port,
                ("src/main/java/app/Exposed.java", "package app; @com.acme.Registered public @interface Exposed {}"),
                ("src/main/java/app/Impl.java", "package app; @Exposed class PortImpl implements Port {}"),
            ]),
            ("a configuration method with one", vec![port, ("src/main/java/app/Ports.java", "package app; @org.springframework.context.annotation.Configuration class Ports { @com.acme.Exported Port port() { return null; } }")]),
            ("a method annotation carrying a stereotype the model does not read on methods", vec![
                port,
                ("src/main/java/app/Exported.java", "package app; @org.springframework.stereotype.Component public @interface Exported {}"),
                ("src/main/java/app/Ports.java", "package app; @org.springframework.context.annotation.Configuration class Ports { @Exported Port port() { return null; } }"),
            ]),
            ("an @Import the model does not follow", vec![
                port,
                ("src/main/java/app/Impl.java", "package app; class PortImpl implements Port {}"),
                ("src/main/java/other/Wiring.java", "package other; @org.springframework.context.annotation.Configuration @org.springframework.context.annotation.Import(app.PortImpl.class) class Wiring {}"),
            ]),
            ("a scan filter naming a project annotation", vec![
                port,
                ("src/main/java/app/Exposed.java", "package app; public @interface Exposed {}"),
                ("src/main/java/app/Impl.java", "package app; @Exposed class PortImpl implements Port {}"),
                ("src/main/java/app/Scan.java", "package app; import org.springframework.context.annotation.*; @Configuration @ComponentScan(includeFilters = @ComponentScan.Filter(Exposed.class)) class Scan {}"),
            ]),
            ("a mapper scan with a placeholder", vec![port, ("src/main/java/app/Mappers.java", "package app; @org.mybatis.spring.annotation.MapperScan(\"${mappers}\") class Mappers {}")]),
            ("a mapper scan covering no package of the project", vec![port, ("src/main/java/app/Mappers.java", "package app; @org.mybatis.spring.annotation.MapperScan(\"com.elsewhere.dao\") class Mappers {}")]),
            ("XML bean definitions", vec![port, ("src/main/java/app/Xml.java", "package app; @org.springframework.context.annotation.ImportResource(\"classpath:beans.xml\") class Xml {}")]),
        ] {
            assert_eq!(resolution(&files), Resolution::Unknown, "{why}");
        }
    }

    #[test]
    fn unseen_registrations_keep_to_their_source_set_and_kind() {
        let app = ("src/main/java/app/App.java", "package app; @org.springframework.boot.autoconfigure.SpringBootApplication class App { public static void main(String[] args) {} }");
        let port = ("src/main/java/app/Port.java", "package app; public interface Port {}");
        let uses = ("src/main/java/app/Uses.java", "package app; import org.springframework.stereotype.*; @Service class Uses { Uses(Port port) {} }");
        let test_uses = ("src/test/java/app/TestUses.java", "package app; import org.springframework.stereotype.*; @Service class TestUses { TestUses(Port testPort) {} }");
        let resolve = |extra: &[(&'static str, &'static str)], name: &str| {
            let mut files = vec![app, port, uses, test_uses];
            files.extend_from_slice(extra);
            let (idx, m) = model(&files);
            point_resolution(&idx, &m, name)
        };
        // evidence found only in test sources leaves main points judged
        for extra in [
            ("src/test/java/app/Xml.java", "package app; @org.springframework.context.annotation.ImportResource(\"beans.xml\") class Xml {}"),
            ("src/test/java/app/Mappers.java", "package app; class Mappers { org.mybatis.spring.mapper.MapperScannerConfigurer c; }"),
            ("src/test/java/app/Registers.java", "package app; class Registers { void r(Object ctx) { registerSingleton(Port.class); } void registerSingleton(Object o) {} }"),
        ] {
            assert_eq!(resolve(&[extra], "port"), Resolution::NotFound, "{}", extra.1);
            assert_eq!(resolve(&[extra], "testPort"), Resolution::Unknown, "{}", extra.1);
        }
        // a test's @MapperScan of its own package registers test mappers only
        let scan = ("src/test/java/app/MapperTestConfig.java", "package app; @org.mybatis.spring.annotation.MapperScan class MapperTestConfig {}");
        assert_eq!(resolve(&[scan], "port"), Resolution::NotFound);
        assert!(matches!(resolve(&[scan], "testPort"), Resolution::Unique(_)));
        // an ASSIGNABLE_TYPE include filter registers the subtypes of a type
        let impl_ = ("src/main/java/app/PortImpl.java", "package app; class PortImpl implements Port {}");
        let filter = ("src/main/java/app/Scan.java", "package app; import org.springframework.context.annotation.*;
             @Configuration @ComponentScan(includeFilters = @ComponentScan.Filter(type = FilterType.ASSIGNABLE_TYPE, classes = Port.class)) class Scan {}");
        assert_eq!(resolve(&[impl_], "port"), Resolution::NotFound);
        assert_eq!(resolve(&[impl_, filter], "port"), Resolution::Unknown);
        // a main class registered only by a test's scan produces for tests only
        let (idx, m) = model(&[
            app, port,
            ("src/main/java/app/Uses.java", "package app; import org.springframework.stereotype.*; @Service class Uses { Uses(Port port) {} }"),
            ("src/main/java/app/Ports.java", "package app; class Ports { @com.acme.Exported Port port() { return null; } }"),
            ("src/test/java/app/Scan.java", "package app; import org.springframework.context.annotation.*;
             @Configuration @ComponentScan(includeFilters = @ComponentScan.Filter(type = FilterType.REGEX, pattern = \".*\")) class Scan {}"),
        ]);
        assert_eq!(point_resolution(&idx, &m, "port"), Resolution::NotFound);
        // a producer declared to return a supertype may return the injected subtype
        let (idx, m) = model(&[
            app,
            ("src/main/java/app/A.java", "package app; import org.springframework.stereotype.*;
             interface Store {} class RedisStore implements Store {}
             @org.springframework.context.annotation.Configuration class Stores { @com.acme.Exported Store store() { return new RedisStore(); } }
             @Service class UsesRedis { UsesRedis(RedisStore redis) {} }"),
        ]);
        assert_eq!(point_resolution(&idx, &m, "redis"), Resolution::Unknown);
    }

    /// The bean-name pairs reported, as "Class.name" or "Class" for components.
    fn conflicts(m: &DiModel) -> Vec<String> {
        let show = |i: usize| {
            let b = &m.beans[i];
            match &b.source {
                BeanSource::FactoryMethod(f) => format!("{}.{}", simple_name_of(&b.declaring_class), f),
                _ => simple_name_of(&b.declaring_class).to_string(),
            }
        };
        let mut v: Vec<String> = m
            .name_conflicts()
            .into_iter()
            .map(|(a, b)| {
                let mut pair = [show(a), show(b)];
                pair.sort();
                pair.join(" ")
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn independent_contexts_are_judged_apart() {
        let job = |pkg: &str, class: &str, extra: &str| {
            format!(
                "package samples.{pkg}; import org.springframework.context.annotation.*;
                 {extra} @Configuration public class {class} {{ @Bean public Runnable job() {{ return null; }} }}"
            )
        };
        let starter = "package samples.a; import org.springframework.context.annotation.*;
             @Configuration @Import(samples.c.CJob.class) public class AJob {
                 @Bean public Runnable job() { return null; }
                 public static void main(String[] args) { new AnnotationConfigApplicationContext(AJob.class); }
             }";
        // B is loaded on its own by its test: not in the context A starts.
        // C is imported by A: the clash is real.
        let (b, c) = (job("b", "BJob", ""), job("c", "CJob", ""));
        let (_, m) = model(&[("src/main/java/samples/a/AJob.java", starter), ("src/main/java/samples/b/BJob.java", &b), ("src/main/java/samples/c/CJob.java", &c)]);
        assert_eq!(m.contexts.len(), 1);
        assert_eq!(conflicts(&m), vec!["AJob.job CJob.job"]);
        // what the root scans clashes too, what it does not scan does not
        let scanning = "package samples; import org.springframework.context.annotation.*;
             @Configuration @ComponentScan(\"samples.b\") public class Main {
                 public static void main(String[] args) { new AnnotationConfigApplicationContext(Main.class); }
             }";
        let d = job("d", "DJob", "");
        let (_, m) = model(&[("src/main/java/samples/Main.java", scanning), ("src/main/java/samples/b/BJob.java", &b), ("src/main/java/samples/b/B2.java", &job("b", "B2Job", "")), ("src/main/java/samples/d/DJob.java", &d)]);
        assert_eq!(conflicts(&m), vec!["B2Job.job BJob.job"]);
        // a scan with exclude filters registers what the model cannot name
        let filtered = scanning.replace("@ComponentScan(\"samples.b\")", "@ComponentScan(value = \"samples.b\", excludeFilters = @ComponentScan.Filter(Deprecated.class))");
        let (_, m) = model(&[("src/main/java/samples/Main.java", &filtered), ("src/main/java/samples/b/BJob.java", &b), ("src/main/java/samples/b/B2.java", &job("b", "B2Job", ""))]);
        assert!(conflicts(&m).is_empty());
    }

    #[test]
    fn an_application_class_nothing_starts_is_no_context() {
        // a rule sample: @SpringBootApplication with no main, started by nothing
        let sample = ("src/main/java/checks/Sample.java", "package checks; @org.springframework.boot.autoconfigure.SpringBootApplication public final class Sample {}");
        let configs = [
            ("src/main/java/checks/SampleA.java", "package checks; import org.springframework.context.annotation.*; @Configuration class SampleA { @Bean String dataSource() { return \"a\"; } }"),
            ("src/main/java/checks/SampleB.java", "package checks; import org.springframework.context.annotation.*; @Configuration class SampleB { @Bean String dataSource() { return \"b\"; } }"),
            ("src/main/java/checks/Uses.java", "package checks; interface Missing {} @org.springframework.stereotype.Service class Uses { Uses(Missing m) {} }"),
        ];
        let (idx, m) = model(&[sample, configs[0], configs[1], configs[2]]);
        assert!(!m.spring_application);
        assert!(conflicts(&m).is_empty());
        assert_eq!(point_resolution(&idx, &m, "m"), Resolution::Unknown);
        // with a main, or started from another class, it is an application
        let with_main = ("src/main/java/checks/Sample.java", "package checks; @org.springframework.boot.autoconfigure.SpringBootApplication class Sample { public static void main(String[] args) {} }");
        let (idx, m) = model(&[with_main, configs[0], configs[1], configs[2]]);
        assert_eq!(conflicts(&m), vec!["SampleA.dataSource SampleB.dataSource"]);
        assert_eq!(point_resolution(&idx, &m, "m"), Resolution::NotFound);
        let started = ("src/main/java/launch/Main.java", "package launch; import org.springframework.boot.SpringApplication; import checks.Sample;
             public class Main { public static void main(String[] args) { SpringApplication.run(Sample.class, args); } }");
        let (_, m) = model(&[sample, started, configs[0], configs[1], configs[2]]);
        assert_eq!(conflicts(&m), vec!["SampleA.dataSource SampleB.dataSource"]);
        let war = ("src/main/java/checks/Init.java", "package checks; public class Init extends org.springframework.boot.web.servlet.support.SpringBootServletInitializer {
             protected Object configure(Object b) { return sources(Sample.class); } }");
        let (_, m) = model(&[sample, war, configs[0], configs[1], configs[2]]);
        assert_eq!(conflicts(&m), vec!["SampleA.dataSource SampleB.dataSource"]);
    }

    #[test]
    fn beans_compete_only_inside_one_context() {
        let files = [
            ("src/main/java/app/App.java", "package app; @org.springframework.boot.autoconfigure.SpringBootApplication class App { public static void main(String[] args) {} }"),
            ("src/main/java/app/Store.java", "package app; public interface Store {}"),
            ("src/main/java/app/DbStore.java", "package app; @org.springframework.stereotype.Service class DbStore implements Store {}"),
            ("src/main/java/app/MemStore.java", "package app; @org.springframework.stereotype.Service class MemStore implements Store {}"),
            ("src/main/java/app/Uses.java", "package app; @org.springframework.stereotype.Service class Uses { Uses(Store store) {} }"),
            // two test configurations, each the root of its own test's
            // context (a test's scan registers them, for tests only)
            ("src/test/java/conf/Scan.java", "package conf; @org.springframework.context.annotation.ComponentScan class Scan {}"),
            ("src/test/java/conf/ConfA.java", "package conf; import org.springframework.context.annotation.*; @Configuration class ConfA { @Bean app.Store a() { return null; } }"),
            ("src/test/java/conf/ConfB.java", "package conf; import org.springframework.context.annotation.*; @Configuration class ConfB { @Bean app.Store b() { return null; } }"),
            ("src/test/java/conf/Security.java", "package conf; import org.springframework.context.annotation.*; @Configuration class Security { @Bean Object filter(app.Store testStore) { return null; } }"),
            // a test component the application's scan picks up in a test run
            ("src/test/java/app/TestUses.java", "package app; @org.springframework.stereotype.Service class TestUses { TestUses(Store anyStore) {} }"),
        ];
        let (idx, m) = model(&files);
        assert!(matches!(point_resolution(&idx, &m, "store"), Resolution::Ambiguous(v) if v.len() == 2));
        assert_eq!(point_resolution(&idx, &m, "testStore"), Resolution::Unknown);
        assert!(matches!(point_resolution(&idx, &m, "anyStore"), Resolution::Ambiguous(v) if v.len() == 2));
        // two applications in one module, each scanning its own package
        let (idx, m) = model(&[
            ("src/main/java/one/App.java", "package one; @org.springframework.boot.autoconfigure.SpringBootApplication class App { public static void main(String[] args) {} }"),
            ("src/main/java/one/DbStore.java", "package one; @org.springframework.stereotype.Service class DbStore implements shared.Store {}"),
            ("src/main/java/one/Uses.java", "package one; @org.springframework.stereotype.Service class Uses { Uses(shared.Store store) {} }"),
            ("src/main/java/two/App.java", "package two; @org.springframework.boot.autoconfigure.SpringBootApplication class App { public static void main(String[] args) {} }"),
            ("src/main/java/two/MemStore.java", "package two; @org.springframework.stereotype.Service class MemStore implements shared.Store {}"),
            ("src/main/java/shared/Store.java", "package shared; public interface Store {}"),
        ]);
        assert_eq!(m.contexts.len(), 2);
        assert!(!matches!(point_resolution(&idx, &m, "store"), Resolution::Ambiguous(_)));
    }
}
