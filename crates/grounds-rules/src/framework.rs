//! Rule API: metadata, diagnostics, fixes and the per-file context rules
//! work with. Rules share all semantic and flow information through
//! [`Ctx`]; expensive facts (CFGs, dataflow results) are computed once per
//! callable and cached for every rule that asks.

use grounds_di::DiModel;
use grounds_flow::analyses::{Liveness, NullnessAnalysis, OptionalPresence, ResourceAnalysis};
use grounds_flow::{Cfg, Results};
use grounds_index::doubt::{self, Doubt, Doubts};
use grounds_index::*;
use grounds_semantic::{Claim, FileModel, Jars};
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
    /// The rule's stance on doubt (see `grounds_index::doubt`): the kinds of
    /// collapsed unknown it reports in spite of, each with the reason. Empty,
    /// the default: strict, a finding with any doubt is withheld. The
    /// declarations are the whole list of rules that report without full
    /// grounds; every other rule only loses findings when information is
    /// hidden.
    #[serde(serialize_with = "serialize_tolerates")]
    pub tolerates: &'static [(Doubt, &'static str)],
    /// The rule's findings say that code does not compile because something
    /// is not there (`absence: true` in `declare_rule!`). Such a claim is
    /// only true in a closed world, which only the kernel proves
    /// (`grounds_semantic::closed`): each finding must carry the kernel's
    /// [`Claim`] ([`Ctx::report_claim`]), and one without it is not
    /// reported. Every rule of [`STABLE_IN_CHANGED`] is one.
    #[serde(skip)]
    pub absence: bool,
}

fn serialize_tolerates<S: serde::Serializer>(t: &&'static [(Doubt, &'static str)], s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(t.len()))?;
    for (kind, reason) in t.iter() {
        seq.serialize_element(&(kind.name(), reason))?;
    }
    seq.end()
}

/// The rules that say code would not compile, whose findings `grounds
/// changed` holds to the stable promise: there it reports only what is new
/// with the change (a finding on a changed line whose claim the project
/// before the change did not get, a place in another file that the change
/// broke), and a gap in the model is there before and after the change alike.
/// The promise holds where the build picture is known: a finding made in a
/// file the build output shows the build compiles is stable; one made where
/// grounds has no evidence of what the build compiles (never built) stays
/// preview, and the kernel makes none in a file the build does not compile.
/// Run over a whole project (`grounds check`) the same rules are preview:
/// each unseen project has features the model lacks (docs/holdout). The
/// wiring rules (`bean-not-found` and the rest) are compared the same way but
/// are not here: which claim they make depends on the order of maps and
/// annotations in the wiring model, so a claim before and after a change can
/// differ with nothing changed. A rule joins this list only with that
/// evidence; the `stable_rules_are_exactly_the_intended_set` test makes it
/// deliberate.
pub const STABLE_IN_CHANGED: &[&str] = &[
    "missing-jdk-import",
    "override-matches-nothing",
    "unimplemented-abstract-method",
    "unresolved-constructor",
    "unresolved-field",
    "unresolved-method",
];

impl RuleMeta {
    /// Whether the rule reports only grounded findings.
    pub fn is_strict(&self) -> bool {
        self.tolerates.is_empty()
    }

    /// Whether a finding of this rule with these doubts is reported: every
    /// kind of them is one the rule declares it tolerates.
    pub fn tolerates_all(&self, d: Doubts) -> bool {
        d.kinds().all(|k| self.tolerates.iter().any(|(t, _)| *t == k))
    }

    /// Whether `grounds changed` holds this rule's findings to the stable
    /// promise, those made in a file the build is known to compile (see
    /// [`STABLE_IN_CHANGED`]).
    pub fn stable_in_changed(&self) -> bool {
        self.maturity == Maturity::Stable || STABLE_IN_CHANGED.contains(&self.id)
    }

    /// The maturity of this rule's findings in `grounds changed`
    /// (`changed`) or in a whole-project `grounds check`: what the rules
    /// selected for a mode are held to. In `grounds changed` a finding of a
    /// rule stable there is stable only when made in a file the build is
    /// known to compile; one made under a guessed build picture is preview.
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
    /// What a compile-break finding asserts, made by the kernel from its
    /// proof that the world is closed (`grounds_semantic::closed`), with
    /// qualified names and whole sets where the message shows simple names
    /// or a few entries. `grounds changed` compares claims to tell a finding
    /// that is new with a change from one that was there before; without a
    /// claim (here or in `wiring_claim`) the message, less its hint and
    /// candidate list, is the claim.
    pub claim: Option<Claim>,
    /// What a wiring finding asserts, for the same comparison: the wiring
    /// model makes it, not the kernel. No absence rule
    /// ([`RuleMeta::absence`]) reports one.
    pub wiring_claim: Option<String>,
    /// What was collapsed from unknown into a definite answer while the rule
    /// derived this finding (see `grounds_index::doubt`): noted in the
    /// [`Ctx::grounded`] part that reported it, else in the whole visit.
    /// Empty: the finding is grounded.
    pub doubt: Doubts,
    /// `doubt` is set (by `Ctx::grounded`); the runner leaves it alone
    pub(crate) attributed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FileInfo {
    pub path: String,
    pub is_test: bool,
    pub is_generated: bool,
}

pub type RuleOptions = FxHashMap<String, String>;

/// Per-file cache of flow facts shared by all rules. Each entry keeps what
/// its computation observed (see `grounds_index::doubt`), replayed on a hit.
#[derive(Default)]
pub struct FlowCache<'t> {
    cfgs: RefCell<FxHashMap<usize, (Rc<Cfg<'t>>, Doubts)>>,
    results: RefCell<FxHashMap<(usize, &'static str), (Rc<dyn Any>, Doubts)>>,
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
    /// Run part of this visit as a derivation of its own: the findings `f`
    /// reports carry only the doubts noted while `f` ran, not everything the
    /// visit observed. For a rule that visits a file or a class and judges
    /// one callable or candidate at a time; whatever `f` reads from a cache
    /// (a callable's flow results, a remembered type) replays its doubts
    /// inside `f`.
    pub fn grounded<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let before = self.out.len();
        let (r, d) = doubt::scope(|| f(self));
        for x in self.out[before..].iter_mut().filter(|x| !x.attributed) {
            x.doubt = d;
            x.attributed = true;
        }
        r
    }

    /// Every report goes through here. A finding of an absence rule
    /// ([`RuleMeta::absence`]) without the kernel's claim rests on no proof
    /// that what it says is missing is missing: it is a bug in the rule, an
    /// assertion in a debug build, and not reported in any build.
    fn push(&mut self, severity: Severity, range: TextRange, message: String, fix: Option<Fix>, claim: Option<Claim>, wiring_claim: Option<String>) {
        if self.rule.absence && claim.is_none() {
            debug_assert!(false, "{} reported a claim of absence without the kernel's proof: {message}", self.rule.id);
            return;
        }
        self.out.push(Diagnostic { rule: self.rule.id, severity, range, message, fix, claim, wiring_claim, doubt: Doubts::NONE, attributed: false });
    }

    pub fn report(&mut self, node: Node<'t>, message: impl Into<String>) {
        self.report_range(TextRange::of(node), message);
    }

    pub fn report_range(&mut self, range: TextRange, message: impl Into<String>) {
        self.push(self.rule.severity, range, message.into(), None, None, None);
    }

    pub fn report_fix(&mut self, node: Node<'t>, message: impl Into<String>, fix: Fix) {
        self.push(self.rule.severity, TextRange::of(node), message.into(), Some(fix), None, None);
    }

    pub fn report_with_severity(&mut self, node: Node<'t>, severity: Severity, message: impl Into<String>) {
        self.push(severity, TextRange::of(node), message.into(), None, None, None);
    }

    /// Report a claim of absence the kernel made (see [`Diagnostic::claim`]),
    /// behind a message that may show only part of it, or name types by
    /// their simple names.
    pub fn report_claim(&mut self, node: Node<'t>, message: impl Into<String>, claim: Claim) {
        self.push(self.rule.severity, TextRange::of(node), message.into(), None, Some(claim), None);
    }

    /// Report a wiring finding with the full claim behind its message (see
    /// [`Diagnostic::wiring_claim`]).
    pub fn report_wiring_claim(&mut self, node: Node<'t>, message: impl Into<String>, claim: String) {
        self.push(self.rule.severity, TextRange::of(node), message.into(), None, None, Some(claim));
    }

    /// Where the versions of the jars come from: exact with a classpath the
    /// build dumped (the project-wide `classpath.exact`), else guessed.
    pub fn jars(&self) -> Jars {
        if self.global_option("classpath.exact") == Some("true") {
            Jars::Exact
        } else {
            Jars::Guessed
        }
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
        if let Some((c, d)) = self.flow.cfgs.borrow().get(&callable.id()) {
            doubt::replay(*d);
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
        let (c, d) = doubt::compute(|| Rc::new(grounds_flow::build_with(callable, self.src, Some(&never_returns))));
        self.flow.cfgs.borrow_mut().insert(callable.id(), (c.clone(), d));
        c
    }

    fn cached<T: 'static>(&self, callable: Node<'t>, key: &'static str, f: impl FnOnce() -> T) -> Rc<T> {
        if let Some((r, d)) = self.flow.results.borrow().get(&(callable.id(), key)) {
            if let Ok(v) = r.clone().downcast::<T>() {
                doubt::replay(*d);
                return v;
            }
        }
        let (v, d) = doubt::compute(|| Rc::new(f()));
        self.flow.results.borrow_mut().insert((callable.id(), key), (v.clone() as Rc<dyn Any>, d));
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
            $(tolerates: [$($tk:ident: $tr:literal),* $(,)?],)?
            $(absence: $absence:literal,)?
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
                tolerates: &[$($((grounds_index::doubt::Doubt::$tk, $tr)),*)?],
                absence: $crate::declare_rule!(@absence $($absence)?),
            };
        }
    };
    (@tier) => { $crate::framework::Tier::Checkpoint };
    (@tier $t:ident) => { $crate::framework::Tier::$t };
    (@scope) => { $crate::framework::Scope::All };
    (@scope $s:ident) => { $crate::framework::Scope::$s };
    (@absence) => { false };
    (@absence $a:literal) => { $a };
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
