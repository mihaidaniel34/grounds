//! Spring application-context starts a file makes, read when its stub is
//! extracted: the classes each startup call hands to Spring, as written, and
//! whether some start could not be read with certainty.
//!
//! A start is certain only when every part of it is: the receiver or the
//! constructed type is Spring's (written qualified, or imported, a static
//! import included); the sources are class literals, or an array of them,
//! directly in argument position; the call is in `main`, in a servlet
//! initializer's `configure` / `get*ConfigClasses`, or in a method of the
//! file those demonstrably call; a variable is followed by its declaration
//! and scope, not its spelling; and every other call made on the builder or
//! context is one known not to change what the context holds. Anything else
//! (`child(...)`, an unknown receiver, a computed source, the builder passed
//! elsewhere) makes the start uncertain: it names no root.

use crate::model::Name;
use crate::stubs::Imports;
use grounds_syntax::{Node, NodeExt};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Startups {
    /// One entry per context the file certainly starts.
    pub calls: Vec<StartCall>,
    /// A start (or what looks like one) the reader could not read with
    /// certainty: this file's applications have roots the model cannot name.
    pub uncertain: bool,
}

/// One context a file starts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StartCall {
    /// The class names handed to the startup call, as written (`App`,
    /// `app.App`, `Outer.Inner`): certainly among the context's roots.
    pub sources: Vec<Name>,
    /// The builder or context is also handed to code the reader cannot
    /// follow, or given a call it does not know: that code may add sources
    /// or change settings (bean overriding), so the sources are a lower
    /// bound and the context's settings are unknown.
    pub open: bool,
}

/// Spring's types that start a context, by simple name, with their packages.
const CONTEXT_TYPES: &[(&str, &[&str])] = &[
    ("SpringApplication", &["org.springframework.boot"]),
    ("SpringApplicationBuilder", &["org.springframework.boot.builder"]),
    ("AnnotationConfigApplicationContext", &["org.springframework.context.annotation"]),
    ("AnnotationConfigWebApplicationContext", &["org.springframework.web.context.support"]),
    ("AnnotationConfigServletWebServerApplicationContext", &["org.springframework.boot.web.servlet.context", "org.springframework.boot.web.server.servlet.context"]),
    ("AnnotationConfigReactiveWebServerApplicationContext", &["org.springframework.boot.web.reactive.context", "org.springframework.boot.web.server.reactive.context"]),
];

/// Calls on a builder, an application or a context that add sources.
const ADDS_SOURCES: &[&str] = &["sources", "register", "addPrimarySources"];

/// Calls known not to change which classes the context is started from or
/// how it treats them (overriding, scanning and parent contexts do).
const NEUTRAL: &[&str] = &[
    "run", "build", "properties", "profiles", "web", "bannerMode", "logStartupInfo", "headless", "initializers", "listeners", "lazyInitialization",
    "registerShutdownHook", "setWebApplicationType", "setBannerMode", "setAdditionalProfiles", "setDefaultProperties", "setLogStartupInfo", "setHeadless",
    "setLazyInitialization", "setRegisterShutdownHook", "addListeners", "addInitializers", "setEnvironment", "environment", "refresh", "start", "close",
    "getBean", "getEnvironment", "setMainApplicationClass", "main", "applicationStartup", "setApplicationStartup", "banner", "setBanner", "setEnvironmentPrefix",
    "environmentPrefix", "addBootstrapRegistryInitializer", "setAddCommandLineProperties", "setAddConversionService", "setKeepAlive",
];

/// Methods a servlet container or Spring Boot calls on an initializer.
const INITIALIZER_ENTRIES: &[&str] = &["configure", "getRootConfigClasses", "getServletConfigClasses"];

fn strip(t: &str) -> String {
    let t: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    t.split('<').next().unwrap_or("").to_string()
}

/// Is the type written here certainly Spring's context or builder type?
fn spring_type(written: &str, imports: &Imports) -> bool {
    let written = strip(written);
    let simple = written.rsplit('.').next().unwrap_or("");
    let Some((_, pkgs)) = CONTEXT_TYPES.iter().find(|(n, _)| *n == simple) else { return false };
    if written.contains('.') {
        return pkgs.iter().any(|p| written == format!("{p}.{simple}"));
    }
    let single: Vec<_> = imports.iter().filter(|i| !i.is_static && !i.on_demand && i.is_named(simple)).collect();
    if !single.is_empty() {
        return single.iter().all(|i| pkgs.iter().any(|p| *i.path == *format!("{p}.{simple}")));
    }
    imports.iter().any(|i| !i.is_static && i.on_demand && pkgs.contains(&&*i.path))
}

/// Is an unqualified `run(...)` the statically imported SpringApplication.run?
fn static_run(imports: &Imports) -> bool {
    imports.iter().any(|i| i.is_static && (&*i.path == "org.springframework.boot.SpringApplication.run" || (i.on_demand && &*i.path == "org.springframework.boot.SpringApplication")))
}

/// The class names of an argument that is a class literal or an array of
/// them; None for anything else.
fn literals(arg: Node, src: &str) -> Option<Vec<Name>> {
    match arg.kind() {
        "class_literal" => {
            let t = arg.kids().into_iter().next()?;
            Some(vec![Arc::from(strip(t.text(src)).as_str())])
        }
        "array_creation_expression" => {
            let init = arg.field("value")?;
            let mut out = Vec::new();
            for e in init.kids() {
                out.extend(literals(e, src)?);
            }
            Some(out)
        }
        "array_initializer" => {
            let mut out = Vec::new();
            for e in arg.kids() {
                out.extend(literals(e, src)?);
            }
            Some(out)
        }
        "parenthesized_expression" => literals(arg.kids().into_iter().next()?, src),
        _ => None,
    }
}

/// All arguments of a call as sources (None: one is not a literal).
fn all_literals(call: Node, src: &str) -> Option<Vec<Name>> {
    let mut out = Vec::new();
    for a in call.field("arguments").map(|a| a.kids()).unwrap_or_default() {
        out.extend(literals(a, src)?);
    }
    Some(out)
}

struct Group {
    sources: Vec<Name>,
    /// which classes are roots, or which context they go to, is unknown
    uncertain: bool,
    /// the sources are a lower bound (see `StartCall::open`)
    open: bool,
}

struct Reader<'s> {
    src: &'s str,
    groups: Vec<Group>,
    /// declarator or parameter node id -> group
    vars: FxHashMap<usize, usize>,
}

impl<'s> Reader<'s> {
    fn group(&mut self, sources: Option<Vec<Name>>) -> usize {
        let uncertain = sources.is_none();
        self.groups.push(Group { sources: sources.unwrap_or_default(), uncertain, open: false });
        self.groups.len() - 1
    }

    /// A call made on a builder or context of group `g`.
    fn on_call(&mut self, g: usize, call: Node) {
        let name = call.field("name").map(|n| n.text(self.src)).unwrap_or("");
        if ADDS_SOURCES.contains(&name) {
            match all_literals(call, self.src) {
                Some(more) => self.groups[g].sources.extend(more),
                // more sources the reader cannot name
                None => self.groups[g].open = true,
            }
        } else if matches!(name, "child" | "parent" | "sibling") {
            // which sources go to which context is not followed
            self.groups[g].uncertain = true;
        } else if !NEUTRAL.contains(&name) {
            self.groups[g].open = true;
        }
    }

    /// Follow the chain of calls made on `node` (a builder or context of
    /// group `g`) and where its value ends up.
    fn follow(&mut self, g: usize, mut node: Node) {
        loop {
            let Some(p) = node.parent() else { return };
            match p.kind() {
                "method_invocation" if p.field("object").is_some_and(|o| o.id() == node.id()) => {
                    let name = p.field("name").map(|n| n.text(self.src)).unwrap_or("");
                    self.on_call(g, p);
                    // the value of run/refresh/getBean is a context or a bean,
                    // no longer the builder
                    if matches!(name, "run" | "refresh" | "start" | "getBean" | "close" | "getEnvironment") {
                        return;
                    }
                    node = p;
                }
                "parenthesized_expression" => node = p,
                "expression_statement" | "return_statement" => return,
                "variable_declarator" if p.field("value").is_some_and(|v| v.id() == node.id()) => {
                    self.vars.insert(p.id(), g);
                    return;
                }
                // passed elsewhere, assigned to a field, ...: what is done
                // to it there is unknown
                _ => {
                    self.groups[g].open = true;
                    return;
                }
            }
        }
    }
}

/// The local variable or parameter an identifier refers to: the nearest
/// declaration of that name in an enclosing block (before the use) or in
/// the enclosing method's parameters.
fn declaration<'t>(id: Node<'t>, src: &str) -> Option<Node<'t>> {
    let name = id.text(src);
    let at = id.start_byte();
    for a in id.ancestors() {
        match a.kind() {
            "block" | "switch_block_statement_group" => {
                for st in a.kids() {
                    if st.start_byte() >= at {
                        break;
                    }
                    if st.kind() == "local_variable_declaration" {
                        for d in st.fields("declarator") {
                            if d.field("name").is_some_and(|n| n.text(src) == name) {
                                return Some(d);
                            }
                        }
                    }
                }
            }
            "method_declaration" | "constructor_declaration" | "lambda_expression" => {
                if let Some(ps) = a.field("parameters") {
                    for p in ps.kids() {
                        if p.field("name").is_some_and(|n| n.text(src) == name) {
                            return Some(p);
                        }
                    }
                }
                if a.kind() != "lambda_expression" {
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

fn is_main(m: Node, src: &str) -> bool {
    m.field("name").is_some_and(|n| n.text(src) == "main")
        && m.modifiers().is_some_and(|mods| mods.text(src).split_whitespace().any(|w| w == "static"))
        && m.field("parameters").is_some_and(|ps| {
            let ps: Vec<Node> = ps.kids().into_iter().filter(|k| matches!(k.kind(), "formal_parameter" | "spread_parameter")).collect();
            ps.len() == 1 && strip(ps[0].text(src)).starts_with("String")
        })
}

pub fn extract(root: Node, src: &str, imports: &Imports) -> Startups {
    let methods: Vec<Node> = root.descendants().filter(|n| n.kind() == "method_declaration").collect();
    // methods reachable from the entries, by unqualified calls in the file
    let mut by_name: FxHashMap<&str, Vec<Node>> = FxHashMap::default();
    for m in &methods {
        if let Some(n) = m.field("name") {
            by_name.entry(n.text(src)).or_default().push(*m);
        }
    }
    let mut reach: FxHashSet<usize> = FxHashSet::default();
    let mut work: Vec<Node> = methods.iter().copied().filter(|m| is_main(*m, src) || m.field("name").is_some_and(|n| INITIALIZER_ENTRIES.contains(&n.text(src)))).collect();
    while let Some(m) = work.pop() {
        if !reach.insert(m.id()) {
            continue;
        }
        for c in m.descendants().filter(|c| c.kind() == "method_invocation") {
            let unqualified = c.field("object").is_none_or(|o| o.kind() == "this");
            if unqualified {
                if let Some(ms) = c.field("name").and_then(|n| by_name.get(n.text(src))) {
                    work.extend(ms.iter().copied());
                }
            }
        }
    }
    let in_reach = |n: Node| n.ancestors().find(|a| a.kind() == "method_declaration").is_some_and(|m| reach.contains(&m.id()));
    let mut r = Reader { src, groups: Vec::new(), vars: FxHashMap::default() };
    let mut outside = false;
    // starts: SpringApplication.run(...) and a created application, builder or context
    for n in root.descendants() {
        let start = match n.kind() {
            "method_invocation" => {
                let name = n.field("name").map(|x| x.text(src)).unwrap_or("");
                let object = n.field("object");
                name == "run" && match object {
                    Some(o) => matches!(o.kind(), "identifier" | "field_access" | "scoped_identifier") && spring_type(o.text(src), imports),
                    None => static_run(imports),
                }
            }
            "object_creation_expression" => n.field("type").is_some_and(|t| spring_type(t.text(src), imports)),
            _ => false,
        };
        if !start {
            continue;
        }
        if !in_reach(n) {
            outside = true;
            continue;
        }
        if n.kind() == "method_invocation" {
            // run(Class<?> source | Class<?>[] sources, String... args)
            let first = n.field("arguments").and_then(|a| a.kids().into_iter().next());
            r.group(first.and_then(|a| literals(a, src)));
        } else {
            let g = r.group(all_literals(n, src));
            r.follow(g, n);
        }
    }
    // initializers: the builder Spring Boot hands to configure(...)
    for m in methods.iter().filter(|m| reach.contains(&m.id())) {
        for p in m.field("parameters").map(|ps| ps.kids()).unwrap_or_default() {
            if p.kind() == "formal_parameter" && p.field("type").is_some_and(|t| spring_type(t.text(src), imports)) {
                let g = r.group(Some(Vec::new()));
                r.vars.insert(p.id(), g);
            }
        }
        // get*ConfigClasses: the classes returned
        let name = m.field("name").map(|n| n.text(src)).unwrap_or("");
        if matches!(name, "getRootConfigClasses" | "getServletConfigClasses") {
            for ret in m.descendants().filter(|d| d.kind() == "return_statement") {
                let Some(v) = ret.kids().into_iter().next() else { continue };
                if v.kind() == "null_literal" {
                    continue;
                }
                r.group(literals(v, src));
            }
        }
    }
    // uses of the variables (and parameters) holding a builder or context
    let decls: Vec<(usize, usize)> = r.vars.iter().map(|(k, v)| (*k, *v)).collect();
    if !decls.is_empty() {
        for id in root.descendants().filter(|n| n.kind() == "identifier") {
            let Some(p) = id.parent() else { continue };
            // the declaration itself, or a member name (`x.app`)
            if matches!(p.kind(), "variable_declarator" | "formal_parameter") && p.field("name").is_some_and(|n| n.id() == id.id()) {
                continue;
            }
            if p.kind() == "field_access" && p.field("field").is_some_and(|f| f.id() == id.id()) {
                continue;
            }
            if p.kind() == "method_invocation" && p.field("name").is_some_and(|f| f.id() == id.id()) {
                continue;
            }
            let Some(d) = declaration(id, src) else { continue };
            let Some(g) = decls.iter().find(|(k, _)| *k == d.id()).map(|(_, g)| *g) else { continue };
            r.follow(g, id);
        }
    }
    let mut out = Startups { calls: Vec::new(), uncertain: outside };
    for g in r.groups {
        if g.uncertain {
            out.uncertain = true;
        } else if !g.sources.is_empty() {
            out.calls.push(StartCall { sources: g.sources, open: g.open });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use grounds_syntax::ParsedFile;

    fn read(src: &str) -> Startups {
        let pf = ParsedFile::parse(src);
        let stub = crate::stubs::extract(&pf);
        stub.startups
    }

    fn calls(src: &str) -> (Vec<Vec<String>>, bool) {
        let s = read(src);
        assert!(s.calls.iter().all(|c| !c.open), "{s:?}");
        (s.calls.iter().map(|c| c.sources.iter().map(|n| n.to_string()).collect()).collect(), s.uncertain)
    }

    #[test]
    fn a_plain_application_is_certain() {
        let s = "package app; import org.springframework.boot.SpringApplication;
            @org.springframework.boot.autoconfigure.SpringBootApplication
            public class App { public static void main(String[] args) { SpringApplication.run(App.class, args); } }";
        assert_eq!(calls(s), (vec![vec!["App".to_string()]], false));
        // qualified in place, statically imported, several sources at once
        let s = "package l; public class Main { public static void main(String[] args) { org.springframework.boot.SpringApplication.run(app.App.class, args); } }";
        assert_eq!(calls(s), (vec![vec!["app.App".to_string()]], false));
        let s = "package l; import static org.springframework.boot.SpringApplication.run;
            public class Main { public static void main(String[] args) { run(new Class<?>[] { A.class, B.class }, args); } }";
        assert_eq!(calls(s), (vec![vec!["A".to_string(), "B".to_string()]], false));
        let s = "package l; import org.springframework.boot.*;
            public class Main { public static void main(String[] args) { new SpringApplication(A.class, B.class).run(args); } }";
        assert_eq!(calls(s), (vec![vec!["A".to_string(), "B".to_string()]], false));
    }

    #[test]
    fn variables_are_followed_by_declaration() {
        // S13: initialised through a chain, then used
        let s = "package l; import org.springframework.boot.builder.SpringApplicationBuilder;
            public class Main { public static void main(String[] args) {
                var b = new SpringApplicationBuilder(A.class).web(null); b.sources(B.class); b.run(args); } }";
        assert_eq!(calls(s), (vec![vec!["A".to_string(), "B".to_string()]], false));
        // S12: two blocks each with their own `app`
        let s = "package l; import org.springframework.boot.builder.SpringApplicationBuilder;
            public class Main { public static void main(String[] args) {
                if (args.length > 0) { var app = new SpringApplicationBuilder(A.class); app.sources(B.class); app.run(args); }
                else { var app = new SpringApplicationBuilder(C.class); app.run(args); } } }";
        let (c, u) = calls(s);
        assert!(!u);
        assert!(c.contains(&vec!["A".to_string(), "B".to_string()]) && c.contains(&vec!["C".to_string()]), "{c:?}");
        // a context assembled step by step
        let s = "package l; import org.springframework.context.annotation.AnnotationConfigApplicationContext;
            public class Main { public static void main(String[] args) {
                AnnotationConfigApplicationContext ctx = new AnnotationConfigApplicationContext(); ctx.register(A.class); ctx.register(B.class); ctx.refresh(); } }";
        assert_eq!(calls(s), (vec![vec!["A".to_string(), "B".to_string()]], false));
    }

    #[test]
    fn what_is_not_certain_names_no_root() {
        for (why, s) in [
            // S8: a computed source
            ("conditional source", "package l; import org.springframework.boot.SpringApplication;
              public class Main { public static void main(String[] args) { SpringApplication.run(args.length > 0 ? A.class : B.class, args); } }"),
            // S9: a child context
            ("child context", "package l; import org.springframework.boot.builder.SpringApplicationBuilder;
              public class Main { public static void main(String[] args) { new SpringApplicationBuilder(A.class).child(C.class).sources(B.class).run(args); } }"),
            // S11: a start in a method main does not call
            ("unreached", "package l; import org.springframework.boot.SpringApplication;
              public class Main { public static void main(String[] args) { } static void other() { SpringApplication.run(Combined.class); } }"),
        ] {
            let (c, u) = calls(s);
            assert!(c.is_empty() && u, "{why}: {c:?} {u}");
        }
        // a start whose builder meets code the reader cannot follow: its
        // sources are a lower bound, its settings unknown
        for s in [
            "package l; import org.springframework.boot.SpringApplication;
              public class Main { public static void main(String[] args) { SpringApplication app = new SpringApplication(A.class); app.setAllowBeanDefinitionOverriding(true); app.run(args); } }",
            "package l; import org.springframework.boot.SpringApplication;
              public class Main { public static void main(String[] args) { var app = new SpringApplication(A.class); Profiles.addDefaultProfile(app); app.run(args).getEnvironment(); } }",
        ] {
            let st = read(s);
            assert_eq!(st.calls.len(), 1, "{s}");
            assert!(st.calls[0].open && !st.uncertain && st.calls[0].sources.len() == 1, "{st:?}");
        }
        // a start in a method main calls is reached
        let s = "package l; import org.springframework.boot.SpringApplication;
            public class Main { public static void main(String[] args) { start(args); } static void start(String[] a) { SpringApplication.run(A.class, a); } }";
        assert_eq!(calls(s), (vec![vec!["A".to_string()]], false));
    }

    #[test]
    fn calls_that_are_not_spring_s_start_nothing() {
        // S10: another `sources`; S14 without the static import; a library's `run`
        for s in [
            "package l; public class Main { public static void main(String[] args) { Catalog.sources(Combined.class); } }",
            "package l; public class Main { public static void main(String[] args) { run(A.class, args); } static void run(Class<?> c, String[] a) {} }",
            "package l; import com.acme.SpringApplication; public class Main { public static void main(String[] args) { SpringApplication.run(A.class, args); } }",
        ] {
            assert_eq!(calls(s), (vec![], false), "{s}");
        }
    }

    #[test]
    fn initializers_hand_their_sources() {
        let s = "package w; import org.springframework.boot.builder.SpringApplicationBuilder;
            public class Init extends org.springframework.boot.web.servlet.support.SpringBootServletInitializer {
                protected SpringApplicationBuilder configure(SpringApplicationBuilder application) { return application.sources(App.class); } }";
        assert_eq!(calls(s), (vec![vec!["App".to_string()]], false));
        let s = "package w; public class Init extends org.springframework.web.servlet.support.AbstractAnnotationConfigDispatcherServletInitializer {
                protected Class<?>[] getRootConfigClasses() { return new Class<?>[] { A.class, app.B.class }; }
                protected Class<?>[] getServletConfigClasses() { return null; } }";
        assert_eq!(calls(s), (vec![vec!["A".to_string(), "app.B".to_string()]], false));
    }
}
