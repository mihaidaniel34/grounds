//! Orchestration: workspace discovery, classpath, caching, analysis runs,
//! baselines and reporting.

pub mod cache;
pub mod classpath;
pub mod config;
pub mod project;
pub mod workspace;

pub use config::Config;
pub use project::Project;
