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
    if pf.too_deep() {
        return Vec::new();
    }
    let key = path_key(path);
    let fm = FileModel::new(&pf, &project.index, key.clone());
    let kind = classify(path);
    let info = FileInfo { path: key.to_string(), is_test: kind == SourceKind::Test, is_generated: kind == SourceKind::Generated };
    let diags = rules.check(&fm, &info, Some(&project.di), options);
    let rel = path.strip_prefix(&project.workspace.root).unwrap_or(path).to_string_lossy().to_string();
    let mut out: Vec<Finding> = Vec::with_capacity(diags.len());
    let mut fps = Fingerprints::default();
    for d in diags {
        out.push(to_finding(project, &pf, &fm, &rel, d, &mut fps));
    }
    out
}

/// Fingerprints of a file's findings, numbered per identical key. A key is
/// `rule|path|member|line text`; the line text is hashed once per line and
/// the hash state of a key is kept, so a finding costs the same on a line of
/// a million characters (a generated table, a file cut mid-edit) as on a
/// short one: before, 100,000 stray semicolons on one line took a minute.
#[derive(Default)]
struct Fingerprints<'t> {
    /// line (0-based) -> trimmed text, whole line ASCII?, hash of the trimmed text
    lines: rustc_hash::FxHashMap<u32, (&'t str, bool, u128)>,
    /// (legacy?, rule, member, line text hash) -> findings so far
    seen: rustc_hash::FxHashMap<(bool, &'static str, String, u128), u32>,
    /// (legacy?, rule, member, line) -> xxh3 state after the key
    states: rustc_hash::FxHashMap<(bool, &'static str, String, u32), xxhash_rust::xxh3::Xxh3Default>,
}

impl<'t> Fingerprints<'t> {
    fn line(&mut self, pf: &'t ParsedFile, line0: u32) -> (&'t str, bool, u128) {
        *self.lines.entry(line0).or_insert_with(|| {
            let raw = pf.lines.line_text(&pf.text, line0 as usize);
            let t = raw.trim();
            (t, raw.is_ascii(), xxhash_rust::xxh3::xxh3_128(t.as_bytes()))
        })
    }

    /// `xxh3_64("{rule}|{rel}|{member}|{line text}|{n}")` with `n` counting
    /// findings of the same key.
    fn next(&mut self, legacy: bool, rule: &'static str, rel: &str, member: &str, line0: u32, line: (&str, bool, u128)) -> String {
        let n = self.seen.entry((legacy, rule, member.to_string(), line.2)).or_insert(0);
        *n += 1;
        let n = *n;
        let state = self.states.entry((legacy, rule, member.to_string(), line0)).or_insert_with(|| {
            let mut h = xxhash_rust::xxh3::Xxh3Default::new();
            for part in [rule, "|", rel, "|", member, "|", line.0] {
                h.update(part.as_bytes());
            }
            h
        });
        let mut h = state.clone();
        h.update(format!("|{n}").as_bytes());
        format!("{:016x}", h.digest())
    }
}

fn to_finding<'t>(project: &Project, pf: &'t ParsedFile, fm: &FileModel, rel: &str, d: Diagnostic, fps: &mut Fingerprints<'t>) -> Finding {
    let line_col = |fps: &mut Fingerprints<'t>, offset: u32| {
        let line0 = pf.lines.line_of(offset) as u32;
        let start = pf.lines.line_start(line0 as usize) as usize;
        let end = (offset as usize).min(pf.text.len());
        // a column counts characters: on an ASCII line, bytes
        if fps.line(pf, line0).1 && end >= start && end <= start + pf.lines.line_text(&pf.text, line0 as usize).len() {
            grounds_syntax::LineCol { line: line0 + 1, col: (end - start) as u32 + 1 }
        } else {
            pf.lines.line_col(&pf.text, offset)
        }
    };
    let s = line_col(fps, d.range.start);
    let e = line_col(fps, d.range.end);
    let line0 = s.line - 1;
    let line = fps.line(pf, line0);
    // enclosing member for fingerprint stability: every enclosing type and
    // the member with its parameter types (`Outer.Inner.f(String,int)`), so
    // the same statement in another class or overload is a different finding
    let node = pf.root().descendant_for_byte_range(d.range.start as usize, d.range.start as usize);
    let member = node.map(|n| member_path(n, fm.src, false)).unwrap_or_default();
    let fp = fps.next(false, d.rule, rel, &member, line0, line);
    // the legacy fingerprint numbers repeated findings by its own key, as
    // 0.0.5 did (two overloads that differ only in a qualified varargs type
    // shared one legacy member path)
    let legacy_member = node.map(|n| member_path(n, fm.src, true)).unwrap_or_default();
    let legacy_fp = fps.next(true, d.rule, rel, &legacy_member, line0, line);
    let legacy_fingerprint = (legacy_fp != fp).then_some(legacy_fp);
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
        .flat_map_iter(|p| match crate::fsutil::read_text(p) {
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
    // `ancestors()`, not a `parent()` loop, which is quadratic in the depth
    for a in std::iter::once(n).chain(n.ancestors()) {
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
