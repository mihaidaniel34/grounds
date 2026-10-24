//! Declaration index: the shared model of all classes visible to analysis,
//! from project sources (stubs) and the classpath (class files).

pub mod binary;
pub mod hierarchy;
pub mod index;
pub mod jdk_modules;
pub mod lombok;
pub mod model;
pub mod order;
pub mod sources;
pub mod stubs;

pub use hierarchy::{ClassLookup, FieldRef, MethodRef, Tri};
pub use index::{Classpath, Index, JdkApi, Scope};
pub use model::*;
pub use stubs::{extract, is_assertion_name, FileStub, Import, RawClass, TypeSyntax};

#[cfg(test)]
mod tests;
