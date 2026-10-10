//! Orchestration: workspace discovery, classpath, caching, analysis runs,
//! baselines and reporting.

pub mod analyze;
pub mod cache;
pub mod classpath;
pub mod config;
pub mod project;
pub mod report;
pub mod workspace;

pub use config::Config;
pub use project::Project;
