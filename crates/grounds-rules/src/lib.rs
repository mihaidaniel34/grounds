//! Rule framework and rule catalog.
//!
//! Rules are small structs implementing [`framework::Rule`]; they declare
//! which syntax node kinds they want to see and get a [`framework::Ctx`]
//! giving access to the shared semantic model (types, symbols, resolved
//! calls), flow analyses (CFG, nullness, liveness, resources) and the DI
//! model. The [`runner::RuleSet`] dispatches all rules in one traversal.

pub mod framework;
pub mod rules;
pub mod runner;
pub mod util;

pub use framework::{Category, Diagnostic, Fix, Rule, RuleMeta, Scope, Severity, Tier};
pub use runner::RuleSet;

/// Every rule in the catalog.
pub fn all_rules() -> Vec<Box<dyn Rule>> {
    rules::all()
}

/// Metadata of every rule in the catalog.
pub fn catalog() -> &'static [&'static RuleMeta] {
    static C: once_cell::sync::OnceCell<Vec<&'static RuleMeta>> = once_cell::sync::OnceCell::new();
    C.get_or_init(|| all_rules().iter().map(|r| r.meta()).collect())
}

pub fn meta_of(id: &str) -> Option<&'static RuleMeta> {
    catalog().iter().copied().find(|m| m.id == id)
}
