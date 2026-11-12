//! Semantic analysis of a single compilation unit against the global
//! declaration index: local binding, name classification, expression types,
//! method resolution with inference, target typing and constant folding.

pub mod binder;
pub mod calls;
pub mod matchers;
mod memo;
pub mod model;
pub mod types;

pub use binder::{LocalDecl, LocalKind};
pub use matchers::{MethodMatcher, OwnerMatch};
pub use model::{FileModel, Lookup, Named, ResolvedCall, Symbol};

#[cfg(test)]
mod tests;
