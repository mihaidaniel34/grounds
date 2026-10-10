//! Output formats: compact text for agents and terminals, JSON, SARIF.

use crate::analyze::Finding;
use grounds_rules::Severity;
use std::collections::BTreeMap;

pub fn text(findings: &[Finding], max: Option<usize>) -> String {
    let mut out = String::new();
    let shown = max.unwrap_or(usize::MAX).min(findings.len());
    for f in &findings[..shown] {
        out.push_str(&format!("{}:{}:{}: {} [{}] {}\n", f.path, f.line, f.col, f.severity.as_str(), f.rule, f.message));
    }
    if shown < findings.len() {
        let rest = &findings[shown..];
        let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
        for f in rest {
            *by_rule.entry(f.rule).or_default() += 1;
        }
        let summary: Vec<String> = by_rule.iter().map(|(r, n)| format!("{r} x{n}")).collect();
        out.push_str(&format!("... and {} more ({})\n", rest.len(), summary.join(", ")));
    }
    out
}

pub fn summary(findings: &[Finding]) -> String {
    let e = findings.iter().filter(|f| f.severity == Severity::Error).count();
    let w = findings.iter().filter(|f| f.severity == Severity::Warning).count();
    let i = findings.iter().filter(|f| f.severity == Severity::Info).count();
    format!("{} findings ({e} errors, {w} warnings, {i} info)", findings.len())
}

pub fn json(findings: &[Finding]) -> String {
    serde_json::to_string_pretty(findings).unwrap_or_default()
}

pub fn sarif(findings: &[Finding]) -> String {
    use serde_json::json;
    let mut rules_used: Vec<&str> = findings.iter().map(|f| f.rule).collect();
    rules_used.sort();
    rules_used.dedup();
    let rules: Vec<_> = rules_used
        .iter()
        .filter_map(|id| grounds_rules::meta_of(id))
        .map(|m| {
            json!({
                "id": m.id,
                "name": m.name,
                "shortDescription": { "text": m.name },
                "fullDescription": { "text": m.description },
                "properties": { "category": m.category.as_str(), "sources": m.sources },
            })
        })
        .collect();
    let results: Vec<_> = findings
        .iter()
        .map(|f| {
            json!({
                "ruleId": f.rule,
                "level": match f.severity { Severity::Error => "error", Severity::Warning => "warning", Severity::Info => "note" },
                "message": { "text": f.message },
                "partialFingerprints": { "grounds/v1": f.fingerprint },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": f.path },
                        "region": { "startLine": f.line, "startColumn": f.col, "endLine": f.end_line, "endColumn": f.end_col }
                    }
                }]
            })
        })
        .collect();
    let doc = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{ "tool": { "driver": { "name": "grounds", "informationUri": "https://github.com/", "rules": rules } }, "results": results }]
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}
