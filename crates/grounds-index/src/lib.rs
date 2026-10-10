//! Declaration index: the shared model of all classes visible to analysis,
//! from project sources (stubs) and the classpath (class files).

pub mod binary;
pub mod hierarchy;
pub mod index;
pub mod lombok;
pub mod model;
pub mod stubs;

pub use hierarchy::{ClassLookup, FieldRef, MethodRef, Tri};
pub use index::{Classpath, Index, Scope};
pub use model::*;
pub use stubs::{extract, FileStub, Import, RawClass, TypeSyntax};

#[cfg(test)]
mod tests;
