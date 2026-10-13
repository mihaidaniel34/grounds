//! Output formats: compact text for agents and terminals, JSON, SARIF.

use crate::analyze::Finding;
use grounds_rules::Severity;
use std::collections::BTreeMap;
use std::path::Path;

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

    pub fn paint_error(&self, s: &str) -> String {
        self.paint("31", s)
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

const HOME: &str = "https://github.com/mihaidaniel34/grounds";

/// SARIF for `grounds check`.
pub fn sarif(findings: &[Finding], root: &Path) -> String {
    let mut doc = SarifDoc::new(root);
    for f in findings {
        doc.result(f, Vec::new());
    }
    doc.render()
}

/// SARIF for `grounds changed`: findings on changed lines, the places in
/// other files the change broke, and on each changed line that broke
/// something a `breaks-other-files` result listing them as related locations.
pub fn sarif_report(report: &crate::session::Report, root: &Path) -> String {
    let mut doc = SarifDoc::new(root);
    for f in &report.findings {
        doc.result(f, Vec::new());
    }
    for f in &report.pending {
        doc.result(f, Vec::new());
    }
    for b in &report.breaks {
        let related = b.broken.iter().enumerate().map(|(k, &i)| (k, &report.pending[i])).collect();
        doc.result(&b.finding, related);
    }
    doc.render()
}

struct SarifDoc {
    rules: Vec<serde_json::Value>,
    rule_index: BTreeMap<&'static str, usize>,
    results: Vec<serde_json::Value>,
    /// The project root relative to the repository root (`app/`), since code
    /// scanning and reviewdog resolve paths from the repository root.
    prefix: String,
}

/// A relative path as a URI reference: everything but unreserved
/// characters and `/` percent-encoded (`my app/A.java` -> `my%20app/A.java`).
fn uri_encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn level(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    }
}

impl SarifDoc {
    fn new(root: &Path) -> SarifDoc {
        let prefix = crate::git::toplevel(root)
            .and_then(|top| {
                let (r, t) = (root.canonicalize().ok()?, top.canonicalize().ok()?);
                r.strip_prefix(t).ok().map(|p| p.to_string_lossy().replace('\\', "/"))
            })
            .filter(|p| !p.is_empty())
            .map(|p| format!("{p}/"))
            .unwrap_or_default();
        SarifDoc { rules: Vec::new(), rule_index: BTreeMap::new(), results: Vec::new(), prefix }
    }

    fn rule(&mut self, id: &'static str, severity: Severity) -> usize {
        use serde_json::json;
        if let Some(&i) = self.rule_index.get(id) {
            return i;
        }
        let rule = match grounds_rules::meta_of(id) {
            Some(m) => json!({
                "id": m.id,
                "name": m.name,
                "shortDescription": { "text": m.name },
                "fullDescription": { "text": m.description },
                "helpUri": format!("{HOME}/blob/master/docs/RULES.md"),
                "defaultConfiguration": { "level": level(m.severity) },
                "properties": { "category": m.category.as_str(), "sources": m.sources },
            }),
            // findings only CI mode (`changed --base`) produces
            None => {
                let (name, text) = match id {
                    "removed-class-still-used" => ("Removed class still used", "A class this change deleted or renamed is still imported or used in a file the change did not touch, so that file no longer compiles."),
                    _ => ("Change breaks other files", "A change on this line broke code in files the change did not touch: calls to members that no longer exist, overrides of changed supertypes, or injection points whose bean is gone. The broken places are related locations, and each is also reported on its own."),
                };
                json!({
                    "id": id,
                    "name": name,
                    "shortDescription": { "text": name },
                    "fullDescription": { "text": text },
                    "helpUri": format!("{HOME}#ci"),
                    "defaultConfiguration": { "level": level(severity) },
                })
            }
        };
        self.rules.push(rule);
        self.rule_index.insert(id, self.rules.len() - 1);
        self.rules.len() - 1
    }

    fn location(&self, f: &Finding) -> serde_json::Value {
        serde_json::json!({
            "physicalLocation": {
                "artifactLocation": { "uri": uri_encode(&format!("{}{}", self.prefix, f.path)), "uriBaseId": "%SRCROOT%" },
                "region": { "startLine": f.line, "startColumn": f.col.max(1), "endLine": f.end_line.max(f.line), "endColumn": f.end_col.max(1) }
            }
        })
    }

    fn result(&mut self, f: &Finding, related: Vec<(usize, &Finding)>) {
        use serde_json::json;
        let idx = self.rule(f.rule, f.severity);
        let mut r = json!({
            "ruleId": f.rule,
            "ruleIndex": idx,
            "level": level(f.severity),
            "message": { "text": f.message },
            "partialFingerprints": { "grounds/v1": f.fingerprint },
            "locations": [self.location(f)],
        });
        if !related.is_empty() {
            let rel: Vec<_> = related
                .into_iter()
                .map(|(k, p)| {
                    let mut l = self.location(p);
                    l["id"] = json!(k);
                    l["message"] = json!({ "text": p.message });
                    l
                })
                .collect();
            r["relatedLocations"] = json!(rel);
        }
        self.results.push(r);
    }

    fn render(self) -> String {
        let doc = serde_json::json!({
            "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
            "version": "2.1.0",
            "runs": [{
                "tool": { "driver": { "name": "grounds", "version": env!("CARGO_PKG_VERSION"), "informationUri": HOME, "rules": self.rules } },
                "originalUriBaseIds": { "%SRCROOT%": { "description": { "text": "The repository root." } } },
                "results": self.results
            }]
        });
        serde_json::to_string_pretty(&doc).unwrap_or_default()
    }
}
