//! Control flow and dataflow: expression-level CFGs, a generic worklist
//! solver and reusable analyses (liveness, nullness, resources, Optional).

pub mod analyses;
pub mod cfg;
pub mod dataflow;

pub use cfg::{build, Block, BlockId, Cfg, EdgeKind};
pub use dataflow::{solve, Analysis, Results};

#[cfg(test)]
mod tests;
