//! Rule API: metadata, diagnostics, fixes and the per-file context rules
//! work with. Rules share all semantic and flow information through
//! [`Ctx`]; expensive facts (CFGs, dataflow results) are computed once per
//! callable and cached for every rule that asks.

use grounds_di::DiModel;
use grounds_flow::analyses::{Liveness, NullnessAnalysis, OptionalPresence, ResourceAnalysis};
use grounds_flow::{Cfg, Results};
use grounds_index::*;
use grounds_semantic::FileModel;
use grounds_syntax::{Node, NodeExt, TextRange};
use rustc_hash::FxHashMap;
use serde::Serialize;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Definitely wrong; will misbehave at runtime.
    Error,
    /// Very likely a bug or a serious bad practice.
    Warning,
    /// Code smell, style or maintainability.
    Info,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
    pub fn parse(s: &str) -> Option<Severity> {
        match s.to_ascii_lowercase().as_str() {
            "error" => Some(Severity::Error),
            "warning" | "warn" => Some(Severity::Warning),
            "info" | "note" => Some(Severity::Info),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Bug,
    Concurrency,
    Performance,
    Security,
    BadPractice,
    Style,
    Testing,
    Framework,
    Complexity,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Bug => "bug",
            Category::Concurrency => "concurrency",
            Category::Performance => "performance",
            Category::Security => "security",
            Category::BadPractice => "bad-practice",
            Category::Style => "style",
            Category::Testing => "testing",
            Category::Framework => "framework",
            Category::Complexity => "complexity",
        }
    }
}

/// Which sources a rule applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Scope {
    All,
    Main,
    Test,
}

/// When a rule runs in the agent feedback loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Tier {
    /// Cheap and high confidence: run after every edit.
    Edit,
    /// Run at checkpoints (end of turn / explicit check).
    Checkpoint,
}

/// How far a rule is proven. Stable rules are covered by the project's
/// promises about precision; preview rules are still being measured against
/// them. Every rule states its maturity in [`declare_rule!`] (the field is
/// required), so a new rule cannot be added without that decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Maturity {
    Stable,
    Preview,
}

impl Maturity {
    pub fn as_str(self) -> &'static str {
        match self {
            Maturity::Stable => "stable",
            Maturity::Preview => "preview",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RuleMeta {
    pub id: &'static str,
    pub name: &'static str,
    pub category: Category,
    pub severity: Severity,
    pub maturity: Maturity,
    pub tier: Tier,
    pub scope: Scope,
    /// Origin of the rule idea: `sonar:S1234`, `spotbugs:PATTERN`, `pmd:Rule`,
    /// `errorprone:Check`, `intellij:Inspection`.
    pub sources: &'static [&'static str],
    /// Node kinds this rule wants to visit.
    pub kinds: &'static [&'static str],
    /// Why it matters and how to fix it.
    pub description: &'static str,
    pub default_enabled: bool,
}

/// The rules that say code would not compile or an application would not
/// start, whose findings `grounds changed` holds to the stable promise: there
/// it reports only what is new with the change (a finding on a changed line
/// that the version before the change does not have, a place in another file
/// that the change broke), and a gap in the model is there before and after
/// the change alike. Run over a whole project (`grounds check`) the same rules
/// are preview: each unseen project has framework features the model lacks
/// (docs/holdout). A rule joins this list only with that evidence; the
/// `stable_rules_are_exactly_the_intended_set` test makes it deliberate.
pub const STABLE_IN_CHANGED: &[&str] = &[
    "ambiguous-bean",
    "bean-name-conflict",
    "bean-not-found",
    "circular-constructor-dependency",
    "missing-jdk-import",
    "override-matches-nothing",
    "qualifier-not-found",
    "unimplemented-abstract-method",
    "unresolved-constructor",
    "unresolved-field",
    "unresolved-method",
];

impl RuleMeta {
    /// Whether `grounds changed` holds this rule's findings to the stable
    /// promise (see [`STABLE_IN_CHANGED`]).
    pub fn stable_in_changed(&self) -> bool {
        self.maturity == Maturity::Stable || STABLE_IN_CHANGED.contains(&self.id)
    }

    /// The maturity of this rule's findings in `grounds changed`
    /// (`changed`) or in a whole-project `grounds check`.
    pub fn maturity_in(&self, changed: bool) -> Maturity {
        if changed && self.stable_in_changed() {
            Maturity::Stable
        } else {
            self.maturity
        }
    }

    /// How `grounds rules`, `grounds explain` and docs/RULES.md name the
    /// maturity: `stable`, `preview`, or `stable in changed` (preview in
    /// `grounds check`).
    pub fn maturity_label(&self) -> &'static str {
        match self.maturity {
            Maturity::Stable => "stable",
            Maturity::Preview if self.stable_in_changed() => "stable in changed",
            Maturity::Preview => "preview",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Edit {
    pub start: u32,
    pub end: u32,
    pub replacement: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Fix {
    pub title: String,
    pub edits: Vec<Edit>,
}

impl Fix {
    pub fn replace(title: impl Into<String>, node: Node, replacement: impl Into<String>) -> Fix {
        Fix { title: title.into(), edits: vec![Edit { start: node.start_byte() as u32, end: node.end_byte() as u32, replacement: replacement.into() }] }
    }
    pub fn delete(title: impl Into<String>, start: u32, end: u32) -> Fix {
        Fix { title: title.into(), edits: vec![Edit { start, end, replacement: String::new() }] }
    }
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub rule: &'static str,
    pub severity: Severity,
    pub range: TextRange,
    pub message: String,
    pub fix: Option<Fix>,
}

#[derive(Clone, Debug, Default)]
pub struct FileInfo {
    pub path: String,
    pub is_test: bool,
    pub is_generated: bool,
}

pub type RuleOptions = FxHashMap<String, String>;

/// Per-file cache of flow facts shared by all rules.
#[derive(Default)]
pub struct FlowCache<'t> {
    cfgs: RefCell<FxHashMap<usize, Rc<Cfg<'t>>>>,
    results: RefCell<FxHashMap<(usize, &'static str), Rc<dyn Any>>>,
}

pub struct Ctx<'a, 't> {
    pub fm: &'a FileModel<'t>,
    pub src: &'t str,
    pub info: &'a FileInfo,
    pub di: Option<&'a DiModel>,
    pub options: &'a RuleOptions,
    pub(crate) rule: &'static RuleMeta,
    pub(crate) out: &'a mut Vec<Diagnostic>,
    pub(crate) flow: &'a FlowCache<'t>,
}

impl<'a, 't> Ctx<'a, 't> {
    pub fn report(&mut self, node: Node<'t>, message: impl Into<String>) {
        self.report_range(TextRange::of(node), message);
    }

    pub fn report_range(&mut self, range: TextRange, message: impl Into<String>) {
        self.out.push(Diagnostic { rule: self.rule.id, severity: self.rule.severity, range, message: message.into(), fix: None });
    }

    pub fn report_fix(&mut self, node: Node<'t>, message: impl Into<String>, fix: Fix) {
        self.out.push(Diagnostic { rule: self.rule.id, severity: self.rule.severity, range: TextRange::of(node), message: message.into(), fix: Some(fix) });
    }

    pub fn report_with_severity(&mut self, node: Node<'t>, severity: Severity, message: impl Into<String>) {
        self.out.push(Diagnostic { rule: self.rule.id, severity, range: TextRange::of(node), message: message.into(), fix: None });
    }

    pub fn text(&self, n: Node<'t>) -> &'t str {
        &self.src[n.start_byte()..n.end_byte()]
    }

    pub fn type_of(&self, n: Node<'t>) -> Ty {
        self.fm.type_of(n)
    }

    pub fn option(&self, key: &str) -> Option<&str> {
        self.options.get(&format!("{}.{}", self.rule.id, key)).map(|s| s.as_str())
    }

    /// Project-wide setting (not namespaced by rule), e.g. `classpath.exact`.
    pub fn global_option(&self, key: &str) -> Option<&str> {
        self.options.get(key).map(|s| s.as_str())
    }

    pub fn option_usize(&self, key: &str, default: usize) -> usize {
        self.option(key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    pub fn is_test(&self) -> bool {
        self.info.is_test
    }

    /// CFG of a callable (method, constructor, lambda, initializer).
    pub fn cfg(&self, callable: Node<'t>) -> Rc<Cfg<'t>> {
        if let Some(c) = self.flow.cfgs.borrow().get(&callable.id()) {
            return c.clone();
        }
        let fm = self.fm;
        // helpers that always throw end the path: a source method ending in
        // `throw`, `@Contract("-> fail")`, JUnit/TestNG/AssertJ `fail(...)`
        let never_returns = |e: Node<'t>| -> bool {
            let Some(c) = fm.select_method(e) else { return false };
            let contract_fails = c.method.annotations.iter().any(|a| {
                a.simple_name() == "Contract" && a.arg("value").is_some_and(|v| {
                    let s = v.strings().join(";");
                    // only unconditional clauses (`-> fail`, `_, _ -> fail`); `false -> fail`
                    // (JUnit's assertTrue) fails just for some arguments
                    !s.trim().is_empty()
                        && s.split(';').all(|clause| {
                            let (lhs, rhs) = clause.rsplit_once("->").unwrap_or(("", clause));
                            rhs.trim() == "fail" && lhs.split(',').all(|a| matches!(a.trim(), "" | "_"))
                        })
                })
            });
            let test_fail = &*c.method.name == "fail" && ["org.junit", "junit.", "org.testng", "org.assertj"].iter().any(|p| c.owner.starts_with(p));
            // an overridable method (an interface default throwing
            // UnsupportedOperationException) may be replaced at runtime
            use grounds_index::Flags;
            let mf = c.method.flags;
            let not_overridable = mf.has(Flags::STATIC) || mf.has(Flags::PRIVATE) || mf.has(Flags::FINAL) || fm.class(&c.owner).is_some_and(|o| o.flags.has(Flags::FINAL) || o.is_record());
            (mf.has(Flags::ALWAYS_THROWS) && not_overridable) || contract_fails || test_fail
        };
        let c = Rc::new(grounds_flow::build_with(callable, self.src, Some(&never_returns)));
        self.flow.cfgs.borrow_mut().insert(callable.id(), c.clone());
        c
    }

    fn cached<T: 'static>(&self, callable: Node<'t>, key: &'static str, f: impl FnOnce() -> T) -> Rc<T> {
        if let Some(r) = self.flow.results.borrow().get(&(callable.id(), key)) {
            if let Ok(v) = r.clone().downcast::<T>() {
                return v;
            }
        }
        let v = Rc::new(f());
        self.flow.results.borrow_mut().insert((callable.id(), key), v.clone() as Rc<dyn Any>);
        v
    }

    pub fn liveness(&self, callable: Node<'t>) -> Rc<Results<rustc_hash::FxHashSet<usize>>> {
        let cfg = self.cfg(callable);
        let fm = self.fm;
        self.cached(callable, "liveness", || grounds_flow::solve(&cfg, &Liveness { fm }))
    }

    pub fn nullness(&self, callable: Node<'t>) -> Rc<Results<grounds_flow::analyses::NullFact>> {
        let cfg = self.cfg(callable);
        let fm = self.fm;
        self.cached(callable, "nullness", || grounds_flow::solve(&cfg, &NullnessAnalysis { fm, cfg: &cfg }))
    }

    pub fn resources(&self, callable: Node<'t>) -> Rc<Results<grounds_flow::analyses::ResFact>> {
        let cfg = self.cfg(callable);
        let fm = self.fm;
        self.cached(callable, "resources", || grounds_flow::solve(&cfg, &ResourceAnalysis { fm }))
    }

    pub fn optional_presence(&self, callable: Node<'t>) -> Rc<Results<grounds_flow::analyses::PresentFact>> {
        let cfg = self.cfg(callable);
        let fm = self.fm;
        self.cached(callable, "optional", || grounds_flow::solve(&cfg, &OptionalPresence { fm }))
    }

    /// All callables in the file whose bodies have CFGs (methods,
    /// constructors, lambdas, initializers).
    pub fn callables(&self) -> Vec<Node<'t>> {
        let mut v = Vec::new();
        for n in self.fm.root().descendants() {
            match n.kind() {
                "method_declaration" | "constructor_declaration" | "compact_constructor_declaration" => {
                    if n.field("body").is_some() {
                        v.push(n);
                    }
                }
                "lambda_expression" | "static_initializer" => v.push(n),
                "block" if n.parent().is_some_and(|p| matches!(p.kind(), "class_body" | "enum_body_declarations")) => v.push(n),
                _ => {}
            }
        }
        v
    }

    /// Enclosing callable node that owns a CFG containing `n`.
    pub fn owning_callable(&self, n: Node<'t>) -> Option<Node<'t>> {
        for a in n.ancestors() {
            match a.kind() {
                "method_declaration" | "constructor_declaration" | "compact_constructor_declaration" | "lambda_expression" | "static_initializer" => return Some(a),
                "block" if a.parent().is_some_and(|p| matches!(p.kind(), "class_body" | "enum_body_declarations")) => return Some(a),
                "class_body" | "class_declaration" | "field_declaration" => return None,
                _ => {}
            }
        }
        None
    }
}

/// A rule. Implementations visit node kinds declared in their metadata
/// and/or inspect the whole file.
pub trait Rule: Send + Sync {
    fn meta(&self) -> &'static RuleMeta;
    fn check_node<'t>(&self, _node: Node<'t>, _cx: &mut Ctx<'_, 't>) {}
    fn check_file<'t>(&self, _cx: &mut Ctx<'_, 't>) {}
}

/// Declare a rule struct with static metadata.
#[macro_export]
macro_rules! declare_rule {
    (
        $(#[$attr:meta])*
        $ty:ident {
            id: $id:literal,
            name: $name:literal,
            category: $cat:ident,
            severity: $sev:ident,
            maturity: $maturity:ident,
            $(tier: $tier:ident,)?
            $(scope: $scope:ident,)?
            $(enabled: $enabled:literal,)?
            sources: [$($src:literal),* $(,)?],
            kinds: [$($kind:literal),* $(,)?],
            description: $desc:literal $(,)?
        }
    ) => {
        $(#[$attr])*
        pub struct $ty;
        impl $ty {
            pub const META: $crate::framework::RuleMeta = $crate::framework::RuleMeta {
                id: $id,
                name: $name,
                category: $crate::framework::Category::$cat,
                severity: $crate::framework::Severity::$sev,
                maturity: $crate::framework::Maturity::$maturity,
                tier: $crate::declare_rule!(@tier $($tier)?),
                scope: $crate::declare_rule!(@scope $($scope)?),
                sources: &[$($src),*],
                kinds: &[$($kind),*],
                description: $desc,
                default_enabled: $crate::declare_rule!(@enabled $($enabled)?),
            };
        }
    };
    (@tier) => { $crate::framework::Tier::Checkpoint };
    (@tier $t:ident) => { $crate::framework::Tier::$t };
    (@scope) => { $crate::framework::Scope::All };
    (@scope $s:ident) => { $crate::framework::Scope::$s };
    (@enabled) => { true };
    (@enabled $e:literal) => { $e };
}

/// Implement `meta()` for rules declared with [`declare_rule!`].
#[macro_export]
macro_rules! rule_meta {
    () => {
        fn meta(&self) -> &'static $crate::framework::RuleMeta {
            &Self::META
        }
    };
}
