//! Output formats: compact text for agents and terminals, JSON, SARIF.

use crate::analyze::Finding;
use grounds_rules::Severity;
use std::collections::BTreeMap;

/// Terminal styling. Off for agents, pipes and machine formats; only the
/// severity is colored and the rule id dimmed, so output stays readable and
/// greppable.
#[derive(Clone, Copy, Debug, Default)]
pub struct Style {
    pub color: bool,
}

impl Style {
    /// `--color auto|always|never`, honoring NO_COLOR and CLICOLOR_FORCE.
    pub fn detect(choice: &str) -> Style {
        use std::io::IsTerminal;
        let env = |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty() && v != "0");
        let color = match choice {
            "always" => true,
            "never" => false,
            _ => !env("NO_COLOR") && (env("CLICOLOR_FORCE") || std::io::stdout().is_terminal()),
        };
        Style { color }
    }

    fn paint(&self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    pub fn severity(&self, s: Severity) -> String {
        let code = match s {
            Severity::Error => "31",
            Severity::Warning => "33",
            Severity::Info => "36",
        };
        self.paint(code, s.as_str())
    }

    pub fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }

    pub fn bold(&self, s: &str) -> String {
        self.paint("1", s)
    }

    /// `path:line:col: severity [rule] message`
    pub fn finding(&self, f: &Finding) -> String {
        format!("{}:{}:{}: {} {} {}\n", f.path, f.line, f.col, self.severity(f.severity), self.dim(&format!("[{}]", f.rule)), f.message)
    }
}

pub fn text(findings: &[Finding], max: Option<usize>, style: Style) -> String {
    let mut out = String::new();
    let shown = max.unwrap_or(usize::MAX).min(findings.len());
    for f in &findings[..shown] {
        out.push_str(&style.finding(f));
    }
    if shown < findings.len() {
        let rest = &findings[shown..];
        let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
        for f in rest {
            *by_rule.entry(f.rule).or_default() += 1;
        }
        let summary: Vec<String> = by_rule.iter().map(|(r, n)| format!("{r} x{n}")).collect();
        out.push_str(&style.dim(&format!("... and {} more ({})", rest.len(), summary.join(", "))));
        out.push('\n');
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
