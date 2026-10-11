//! Running rules over files and turning diagnostics into stable findings.

use crate::project::{path_key, Project};
use crate::workspace::{classify, SourceKind};
use grounds_rules::framework::{FileInfo, RuleOptions};
use grounds_rules::{all_rules, Diagnostic, Fix, RuleSet, Severity};
use grounds_semantic::FileModel;
use grounds_syntax::{NodeExt, ParsedFile};
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub path: String,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub rule: &'static str,
    pub severity: Severity,
    pub category: &'static str,
    pub message: String,
    /// Stable identity across unrelated edits (line shifts).
    pub fingerprint: String,
    /// The fingerprint 0.0.5 and earlier computed, when it differs (member
    /// paths of methods with qualified-type varargs parameters): baselines
    /// written by those versions still match.
    #[serde(skip)]
    pub legacy_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
    /// Byte range, for fix application.
    #[serde(skip)]
    pub start: u32,
    #[serde(skip)]
    pub end: u32,
}

/// The rules the project configuration selects (`[rules] enable/disable`
/// by id, category or source id).
pub fn selected_rules(project: &Project) -> Vec<Box<dyn grounds_rules::Rule>> {
    let cfg = &project.config.rules;
    all_rules()
        .into_iter()
        .filter(|r| {
            let m = r.meta();
            let named = |list: &[String]| list.iter().any(|x| x == m.id || x == m.category.as_str() || m.sources.iter().any(|s| s == x));
            if !cfg.enable.is_empty() {
                return named(&cfg.enable);
            }
            m.default_enabled && !named(&cfg.disable)
        })
        .collect()
}

pub fn build_ruleset(project: &Project) -> (RuleSet, RuleOptions) {
    let cfg = &project.config.rules;
    let rules = selected_rules(project);
    let mut options = RuleOptions::default();
    // jar versions are exact only with a dumped classpath and no cache guessing
    if !project.config.classpath.scan_caches && !project.config.classpath.files.is_empty() {
        options.insert("classpath.exact".to_string(), "true".to_string());
    }
    for (k, v) in &cfg.options {
        if let Some(t) = v.as_table() {
            for (ok, ov) in t {
                let val = match ov {
                    toml::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                options.insert(format!("{k}.{ok}"), val);
            }
        }
    }
    (RuleSet::new(rules), options)
}

fn severity_override(project: &Project, rule: &str, sev: Severity) -> Severity {
    project.config.rules.severity.get(rule).and_then(|s| Severity::parse(s)).unwrap_or(sev)
}

/// Analyze one file whose current text is `text`.
pub fn analyze_text(project: &Project, rules: &RuleSet, options: &RuleOptions, path: &Path, text: &str) -> Vec<Finding> {
    let pf = ParsedFile::parse(text);
    let key = path_key(path);
    let fm = FileModel::new(&pf, &project.index, key.clone());
    let kind = classify(path);
    let info = FileInfo { path: key.to_string(), is_test: kind == SourceKind::Test, is_generated: kind == SourceKind::Generated };
    let diags = rules.check(&fm, &info, Some(&project.di), options);
    let rel = path.strip_prefix(&project.workspace.root).unwrap_or(path).to_string_lossy().to_string();
    let mut out: Vec<Finding> = Vec::with_capacity(diags.len());
    let mut seen_fp: rustc_hash::FxHashMap<String, u32> = Default::default();
    for d in diags {
        out.push(to_finding(project, &pf, &fm, &rel, d, &mut seen_fp));
    }
    out
}

fn to_finding(project: &Project, pf: &ParsedFile, fm: &FileModel, rel: &str, d: Diagnostic, seen: &mut rustc_hash::FxHashMap<String, u32>) -> Finding {
    let s = pf.lines.line_col(&pf.text, d.range.start);
    let e = pf.lines.line_col(&pf.text, d.range.end);
    let line_text = pf.lines.line_text(&pf.text, (s.line - 1) as usize).trim().to_string();
    // enclosing member for fingerprint stability: every enclosing type and
    // the member with its parameter types (`Outer.Inner.f(String,int)`), so
    // the same statement in another class or overload is a different finding
    let node = pf.root().descendant_for_byte_range(d.range.start as usize, d.range.start as usize);
    let member = node.map(|n| member_path(n, fm.src, false)).unwrap_or_default();
    let base = format!("{}|{}|{}|{}", d.rule, rel, member, line_text);
    let n = seen.entry(base.clone()).or_insert(0);
    *n += 1;
    let fp = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(format!("{base}|{n}").as_bytes()));
    let legacy_member = node.map(|n| member_path(n, fm.src, true)).unwrap_or_default();
    let legacy_fingerprint = (legacy_member != member).then(|| {
        let legacy_base = format!("{}|{}|{}|{}", d.rule, rel, legacy_member, line_text);
        format!("{:016x}", xxhash_rust::xxh3::xxh3_64(format!("{legacy_base}|{n}").as_bytes()))
    });
    let category = grounds_rules::meta_of(d.rule).map(|m| m.category.as_str()).unwrap_or("bug");
    Finding {
        path: rel.to_string(),
        line: s.line,
        col: s.col,
        end_line: e.line,
        end_col: e.col,
        rule: d.rule,
        severity: severity_override(project, d.rule, d.severity),
        category,
        message: d.message,
        fingerprint: fp,
        legacy_fingerprint,
        fix: d.fix,
        start: d.range.start,
        end: d.range.end,
    }
}

/// Analyze files in parallel (reading current disk contents).
pub fn analyze_files(project: &Project, rules: &RuleSet, options: &RuleOptions, files: &[PathBuf]) -> Vec<Finding> {
    let mut all: Vec<Finding> = files
        .par_iter()
        .filter(|p| classify(p) != SourceKind::Generated)
        .flat_map_iter(|p| match std::fs::read_to_string(p) {
            Ok(text) => analyze_text(project, rules, options, p, &text),
            Err(_) => Vec::new(),
        })
        .collect();
    all.sort_by(|a, b| (&a.path, a.line, a.col, a.rule).cmp(&(&b.path, b.line, b.col, b.rule)));
    all
}

pub fn all_source_files(project: &Project) -> Vec<PathBuf> {
    project.workspace.files.iter().filter(|f| f.kind != SourceKind::Generated).map(|f| f.path.clone()).collect()
}

/// Resolve CLI path arguments (files or directories) to Java files.
pub fn expand_paths(project: &Project, paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        return all_source_files(project);
    }
    let mut out = Vec::new();
    for p in paths {
        let abs = if p.is_absolute() { p.clone() } else { std::env::current_dir().map(|c| c.join(p)).unwrap_or_else(|_| p.clone()) };
        let abs = abs.canonicalize().unwrap_or(abs);
        if abs.is_dir() {
            out.extend(project.workspace.files.iter().filter(|f| f.path.starts_with(&abs) && f.kind != SourceKind::Generated).map(|f| f.path.clone()));
        } else if abs.extension().is_some_and(|e| e == "java") {
            out.push(abs);
        }
    }
    out.sort();
    out.dedup();
    out
}

impl Finding {
    /// Whether a baseline entry (current or legacy fingerprint) covers it.
    pub fn in_baseline(&self, base: &rustc_hash::FxHashSet<&str>) -> bool {
        base.contains(self.fingerprint.as_str()) || self.legacy_fingerprint.as_deref().is_some_and(|l| base.contains(l))
    }
}

pub fn shared_key(p: &Path) -> Arc<str> {
    path_key(p)
}

/// `legacy`: the member path of 0.0.5 and earlier, whose varargs parameter
/// type came from a child-kind search that missed qualified types.
fn member_path(n: grounds_syntax::Node, src: &str, legacy: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = Some(n);
    while let Some(a) = cur {
        let name = || a.field("name").map(|x| x.text(src).to_string()).unwrap_or_default();
        match a.kind() {
            "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration" | "annotation_type_declaration" => parts.push(name()),
            "object_creation_expression" if a.kid_of_kind("class_body").is_some() => parts.push("<anon>".into()),
            "method_declaration" | "constructor_declaration" => {
                let params: Vec<String> = a
                    .field("parameters")
                    .map(|p| {
                        p.kids()
                            .into_iter()
                            .filter(|k| matches!(k.kind(), "formal_parameter" | "spread_parameter"))
                            .map(|k| {
                                // the type node is not a field of spread parameters
                                let fallback = || k.kids().into_iter().find(|t| t.kind().ends_with("type") || t.kind() == "type_identifier");
                                let ty = if legacy && k.kind() == "spread_parameter" { fallback() } else { k.field("type").or_else(fallback) };
                                let mut t: String = ty.map(|t| t.text(src).split_whitespace().collect()).unwrap_or_default();
                                // `Object p[]`: dimensions after the name
                                if let Some(d) = k.field("dimensions").or_else(|| k.kid_of_kind("variable_declarator").and_then(|v| v.field("dimensions"))) {
                                    t.push_str(&d.text(src).split_whitespace().collect::<String>());
                                }
                                if k.kind() == "spread_parameter" {
                                    t.push_str("...");
                                }
                                t
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                parts.push(format!("{}({})", name(), params.join(",")));
            }
            "field_declaration" => parts.push(a.fields("declarator").into_iter().filter_map(|d| d.field("name")).map(|x| x.text(src).to_string()).collect::<Vec<_>>().join(",")),
            _ => {}
        }
        cur = a.parent();
    }
    parts.reverse();
    parts.join(".")
}

#[cfg(test)]
mod fingerprint_tests {
    use super::member_path;
    use grounds_syntax::{NodeExt, ParsedFile};

    #[test]
    fn legacy_member_path_drops_qualified_varargs_type() {
        let src = "class T { boolean same(java.lang.String... xs) { return xs[0] == \"x\"; } }";
        let pf = ParsedFile::parse(src);
        let ret = pf.root().descendants().find(|n| n.kind() == "return_statement").unwrap();
        assert_eq!(member_path(ret, src, false), "T.same(java.lang.String...)");
        assert_eq!(member_path(ret, src, true), "T.same(...)");
    }
}
