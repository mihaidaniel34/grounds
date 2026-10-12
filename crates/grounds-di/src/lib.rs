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

#[derive(Debug, Default)]
pub struct DiModel {
    pub beans: Vec<Bean>,
    pub points: Vec<InjectionPoint>,
    by_class: FxHashMap<Name, Vec<usize>>,
    pub frameworks: Vec<Framework>,
    pub scan_roots: Vec<String>,
    /// The main sources define a Spring application: `@SpringBootApplication`
    /// / `@SpringBootConfiguration`, or `@ComponentScan` on a class with a
    /// `main` method. Without one the project is a library (auto-configurations
    /// and configurations a consumer imports scan packages too): the context is
    /// assembled by an application we cannot see, which may provide any bean,
    /// so wiring in main code is not judged.
    pub spring_application: bool,
    /// (scan root, module declaring it)
    pub scan_root_modules: Vec<(String, String)>,
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
    /// Constructor-injection edges (owner, dependency class, point index),
    /// computed once at build time.
    pub ctor_edges: Vec<(Name, Name, usize)>,
    /// type name -> beans assignable to it (via the full supertype closure)
    providers: FxHashMap<Name, Vec<usize>>,
    /// beans whose supertype closure is incomplete (unknown library types)
    incomplete: Vec<usize>,
    /// classes with an @Inject constructor: just-in-time bindable in JSR-330 containers
    jit: Vec<Name>,
}

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
    "io.micronaut.context.annotation.ConfigurationProperties",
    "io.micronaut.context.annotation.EachProperty",
];

const JAKARTA_SCOPES: &[&str] = &[
    "jakarta.enterprise.context.ApplicationScoped",
    "jakarta.enterprise.context.RequestScoped",
    "jakarta.enterprise.context.SessionScoped",
    "jakarta.enterprise.context.Dependent",
    "javax.enterprise.context.ApplicationScoped",
    "javax.enterprise.context.RequestScoped",
    "javax.enterprise.context.Dependent",
];

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

fn is_test_path(p: &str) -> bool {
    p.contains("/src/test/") || p.contains("/src/it/") || p.contains("/src/integrationTest/") || p.contains("/src/testFixtures/")
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
    /// classes registered through META-INF files
    registered: rustc_hash::FxHashSet<String>,
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
            metas.iter().find_map(|m| if &*m.name == n { None } else { self.stereotype(m, depth + 1) })
        });
        self.stereo.insert(ann.name.clone(), r);
        r
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
        for (_, stub) in self.index.files() {
            // the frameworks' own sources (vendored, or test stubs) say nothing
            // about what the project uses
            if is_framework_class(&format!("{}.X", stub.package)) {
                continue;
            }
            for imp in &stub.imports {
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
                    let explicit: Vec<String> = ["scanBasePackages", "basePackages", "value"].iter().flat_map(|k| a.arg(k).map(|v| v.strings()).unwrap_or_default()).collect();
                    let classes: Vec<String> = ["scanBasePackageClasses", "basePackageClasses"]
                        .iter()
                        .flat_map(|k| a.arg(k).map(|v| v.values().into_iter().filter_map(|x| if let AnnotationValue::Class(t) = x { t.class_name().map(|c| package_of(c).to_string()) } else { None }).collect::<Vec<_>>()).unwrap_or_default())
                        .collect();
                    let declared: Vec<String> = if explicit.is_empty() && classes.is_empty() { vec![c.package().to_string()] } else { explicit.into_iter().chain(classes).collect() };
                    let cond = self.conditional_meta(&c.annotations);
                    let has_main = c.methods_named("main").any(|m| m.flags.is_static() && m.params.len() == 1);
                    if c.source_file().is_some_and(|f| !is_test_path(f)) && (a.is("org.springframework.boot.autoconfigure.SpringBootApplication") || has_main) {
                        self.model.spring_application = true;
                    }
                    let module = c.source_file().map(module_of).unwrap_or("").to_string();
                    root_conditions.extend(declared.iter().map(|r| (r.clone(), cond, module.clone())));
                    roots.extend(declared);
                }
            }
        }
        // `@SpringBootConfiguration` (what @SpringBootApplication is made of) marks
        // an application too, scan or not
        if !self.model.spring_application {
            self.model.spring_application = names.iter().filter_map(|n| self.index.class(n)).any(|c| {
                c.source_file().is_some_and(|f| !is_test_path(f)) && c.annotations.iter().any(|a| a.is("org.springframework.boot.SpringBootConfiguration"))
            });
        }
        if spring {
            self.model.frameworks.push(Framework::Spring);
        }
        if micronaut {
            self.model.frameworks.push(Framework::Micronaut);
        }
        self.model.scan_roots = roots.clone();
        self.model.scan_root_modules = root_conditions.iter().map(|(r, _, m)| (r.clone(), m.clone())).collect();

        let in_scan = |pkg: &str| roots.is_empty() || roots.iter().any(|r| pkg == r || pkg.starts_with(&format!("{r}.")));
        // MyBatis @MapperScan: every interface in these packages is a bean
        let mut mapper_pkgs: Vec<String> = Vec::new();
        for n in &names {
            let Some(c) = self.index.class(n) else { continue };
            for a in c.annotations.iter().filter(|a| a.simple_name() == "MapperScan") {
                mapper_pkgs.extend(["value", "basePackages"].iter().flat_map(|k| a.arg(k).map(|v| v.strings()).unwrap_or_default()));
                if let Some(v) = a.arg("basePackageClasses") {
                    mapper_pkgs.extend(v.values().into_iter().filter_map(|x| if let AnnotationValue::Class(t) = x { t.class_name().map(|c| package_of(c).to_string()) } else { None }));
                }
            }
        }
        let in_mapper_scan = |pkg: &str| mapper_pkgs.iter().any(|r| pkg == r || pkg.starts_with(&format!("{r}.")));
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
                for a in &c.annotations {
                    match a.simple_name() {
                        "ConfigurationProperties" | "EachProperty" | "ConfigurationReader" if a.name.starts_with("io.micronaut") || !a.resolved => source = Some((BeanSource::Component, Framework::Micronaut)),
                        "FeignClient" | "HttpExchange" => source = Some((BeanSource::Client, Framework::Spring)),
                        "Client" if a.name.starts_with("io.micronaut") || !a.resolved => source = Some((BeanSource::Client, Framework::Micronaut)),
                        "Repository" | "JdbcRepository" | "MongoRepository" | "R2dbcRepository" if a.name.starts_with("io.micronaut") => source = Some((BeanSource::Repository, Framework::Micronaut)),
                        "Mapper" => {
                            // MapStruct (component model may come from a compiler option) or MyBatis
                            let model = first_string(a, &["componentModel"]).unwrap_or_default();
                            let explicit_default = model == "default" || a.arg("componentModel").is_some_and(|v| matches!(v, AnnotationValue::Expr(e) if e.contains("DEFAULT")));
                            if !explicit_default {
                                source = Some((BeanSource::Mapper, Framework::Spring));
                            }
                        }
                        _ => {}
                    }
                }
                if source.is_none() && in_mapper_scan(c.package()) {
                    source = Some((BeanSource::Mapper, Framework::Spring));
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
                        is_test,
                        autowire_candidate: true,
                        framework: fw,
                        dynamic_names: false,
                        replaces: false,
                    });
                    self.model.component_classes.insert(c.name.clone(), i);
                }
                continue;
            }
            if c.flags.has(Flags::ANON) {
                continue;
            }
            if c.is_abstract() {
                if c.annotations.iter().any(|a| a.simple_name() == "Mapper") {
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
            let bean_methods = c.methods.iter().any(|m| m.annotations.iter().any(|a| BEAN_METHOD_ANNOTATIONS.iter().any(|b| a.is(b))));
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
            let covering: Vec<bool> = root_conditions.iter().filter(|(r, _, m)| (!local || *m == own_module) && (pkg == *r || pkg.starts_with(&format!("{r}.")))).map(|(_, c, _)| *c).collect();
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
                    if !factory_methods.iter().any(|x| same_sig(x, m)) {
                        factory_methods.push(m.clone());
                    }
                }
                sup = sc.superclass.clone();
                if hops > 8 {
                    break;
                }
            }
            for m in &factory_methods {
                if m.is_constructor() || !m.annotations.iter().any(|a| BEAN_METHOD_ANNOTATIONS.iter().any(|b| a.is(b))) {
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
                };
                self.add_bean(mbean);
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
            for (name, ty) in mocks {
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
                    framework: Framework::Spring,
                    dynamic_names: false,
                    replaces: false,
                });
            }
        }
        // Micronaut @Replaces: the replaced bean no longer exists
        let mut replaced: Vec<Name> = Vec::new();
        for n in &names {
            let Some(c) = self.index.class(n) else { continue };
            let anns: Vec<&AnnotationInfo> = c.annotations.iter().chain(c.methods.iter().flat_map(|m| m.annotations.iter())).collect();
            for a in anns {
                if a.simple_name() != "Replaces" {
                    continue;
                }
                for k in ["value", "bean"] {
                    if let Some(AnnotationValue::Class(t)) = a.arg(k) {
                        if let Some(tn) = t.class_name() {
                            replaced.push(Arc::from(tn));
                        }
                    }
                }
            }
        }
        for b in &mut self.model.beans {
            if b.source == BeanSource::Component && replaced.contains(&b.declaring_class) {
                b.autowire_candidate = false;
            }
        }
        let mut by_class: FxHashMap<Name, Vec<usize>> = FxHashMap::default();
        for (i, b) in self.model.beans.iter().enumerate() {
            if let Some(n) = b.ty.class_name() {
                by_class.entry(Arc::from(n)).or_default().push(i);
            }
        }
        self.model.by_class = by_class;
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
                "jakarta.inject.Provider" | "javax.inject.Provider" | "org.springframework.beans.factory.ObjectFactory" | "java.util.function.Supplier" => (a0(), false, false, true),
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
        Self::build_with(index, &[])
    }

    /// `registered`: classes listed in META-INF/spring.factories or
    /// META-INF/spring/*.imports (auto-configurations and similar).
    pub fn build_with(index: &Index, registered: &[String]) -> DiModel {
        let mut b = Builder { index, model: DiModel::default(), stereo: FxHashMap::default(), qualifier_ann: FxHashMap::default(), registered: registered.iter().cloned().collect() };
        b.scan();
        let mut m = b.model;
        m.index_providers(index);
        m.ctor_edges = m.constructor_edges(index);
        m
    }

    pub fn is_empty(&self) -> bool {
        self.beans.is_empty()
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
        if p.ty.is_unknown() || p.ty.mentions_type_var() {
            return Resolution::Unknown;
        }
        // a library's beans are wired by the application using it
        if p.framework == Framework::Spring && !self.spring_application && !p.file.as_deref().is_some_and(is_test_path) {
            return Resolution::Unknown;
        }
        let (mut cands, mut maybe) = self.candidates(lk, &p.ty, include_test);
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
                    return if maybe > 0 || self.opaque_imports || !self.provable_type(lk, &p.ty) || self.dynamically_registered(&p.ty) { Resolution::Unknown } else if p.optional { Resolution::Absent } else { Resolution::NotFound };
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
                if !self.provable_type(lk, &p.ty) || self.dynamically_registered(&p.ty) {
                    return Resolution::Unknown;
                }
                return Resolution::QualifierNotFound { qualifier: q.clone(), candidates: before };
            }
        }
        if p.multiple {
            return Resolution::Multiple(cands);
        }
        let jit_possible = || self.jit.iter().any(|j| p.ty.class_name().is_some_and(|t| lk.is_subclass(j, t).is_yes()));
        match cands.len() {
            0 => {
                if p.optional {
                    Resolution::Absent
                } else if maybe > 0 || self.opaque_imports || !self.provable_type(lk, &p.ty) || jit_possible() || self.factory_may_return(lk, &p.ty) || self.dynamically_registered(&p.ty) {
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
                Resolution::Ambiguous(unconditional)
            }
        }
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
    fn dynamically_registered(&self, ty: &Ty) -> bool {
        ty.class_name().is_some_and(|c| self.dynamic_types.contains(simple_name_of(c)))
    }

    fn share_scan_root(&self, a: &Bean, b: &Bean) -> bool {
        if self.scan_roots.len() < 2 {
            return true;
        }
        let pkg = |x: &Bean| package_of(&x.declaring_class).to_string();
        let (pa, pb) = (pkg(a), pkg(b));
        let covers = |r: &str, p: &str| p == r || p.starts_with(&format!("{r}."));
        let module = a.file.as_deref().map(module_of).unwrap_or("");
        let local = self.scan_root_modules.iter().any(|(_, m)| m == module);
        self.scan_root_modules.iter().filter(|(_, m)| !local || m == module).any(|(r, _)| covers(r, &pa) && covers(r, &pb))
    }

    pub fn name_conflicts(&self) -> Vec<(usize, usize)> {
        // names only clash inside one application (module)
        let mut seen: FxHashMap<(&str, &str), usize> = FxHashMap::default();
        let mut out = Vec::new();
        for (i, b) in self.beans.iter().enumerate() {
            // in a library, which beans end up in one context is the user's choice
            if b.framework != Framework::Spring || b.conditional || b.is_test || !self.spring_application {
                continue;
            }
            let key = (b.file.as_deref().map(module_of).unwrap_or(""), b.name.as_str());
            match seen.get(&key) {
                // with several scan roots (separate application contexts are
                // common, e.g. one per package), both must be under one root
                Some(&j) if self.share_scan_root(&self.beans[j], b) => out.push((j, i)),
                Some(_) => {}
                None => {
                    seen.insert(key, i);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grounds_syntax::ParsedFile;

    fn model(files: &[(&str, &str)]) -> (Index, DiModel) {
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
        let m = DiModel::build(&idx);
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
             @org.springframework.boot.autoconfigure.SpringBootApplication class App {}",
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
}
