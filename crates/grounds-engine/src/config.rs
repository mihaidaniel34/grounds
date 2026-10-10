//! `grounds.toml` configuration.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Java release used to pick JDK symbols from ct.sym.
    pub java_release: u32,
    pub java_home: Option<PathBuf>,
    pub classpath: ClasspathConfig,
    pub rules: RulesConfig,
    pub report: ReportConfig,
    /// Extra glob patterns (relative to root) to exclude from analysis.
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClasspathConfig {
    /// Explicit jars or class directories.
    pub entries: Vec<PathBuf>,
    /// Files with one classpath entry per line (or `:`-separated), e.g.
    /// produced by `grounds classpath --gradle`.
    pub files: Vec<PathBuf>,
    /// Scan the local Gradle and Maven caches and use the newest version of
    /// every artifact found. Imprecise but cheap; analysis treats missing
    /// types as unknown anyway.
    pub scan_caches: bool,
    /// Include module build output directories (`build/classes/...`,
    /// `target/classes`) for generated classes.
    pub build_outputs: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct RulesConfig {
    /// Rule ids or categories to disable.
    pub disable: Vec<String>,
    /// Only enable these rules/categories (empty = all default rules).
    pub enable: Vec<String>,
    /// Severity overrides: rule id -> "error" | "warning" | "info".
    pub severity: std::collections::BTreeMap<String, String>,
    /// Per-rule options.
    pub options: std::collections::BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ReportConfig {
    /// Maximum findings shown at a checkpoint before summarizing.
    pub max_findings: usize,
    /// Minimum severity shown by default.
    pub min_severity: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            java_release: 21,
            java_home: None,
            classpath: ClasspathConfig::default(),
            rules: RulesConfig::default(),
            report: ReportConfig::default(),
            exclude: Vec::new(),
        }
    }
}

impl Default for ClasspathConfig {
    fn default() -> Self {
        ClasspathConfig { entries: Vec::new(), files: Vec::new(), scan_caches: true, build_outputs: true }
    }
}

impl Default for ReportConfig {
    fn default() -> Self {
        ReportConfig { max_findings: 15, min_severity: "info".into() }
    }
}

impl Config {
    pub fn load(root: &Path) -> anyhow::Result<Config> {
        for name in ["grounds.toml", ".grounds/config.toml"] {
            let p = root.join(name);
            if p.exists() {
                let text = std::fs::read_to_string(&p)?;
                let cfg: Config = toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?;
                return Ok(cfg);
            }
        }
        Ok(Config::default())
    }
}
