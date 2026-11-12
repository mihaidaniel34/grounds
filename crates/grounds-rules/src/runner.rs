//! Executes a set of rules over a file in a single tree traversal, then
//! applies suppressions (`@SuppressWarnings`, `// NOSONAR`, `// grounds-ignore`).

use crate::framework::*;
use grounds_di::DiModel;
use grounds_index::doubt;
use grounds_semantic::FileModel;
use grounds_syntax::{Node, NodeExt};
use rustc_hash::FxHashSet;

/// Per-rule cumulative time (enabled with GROUNDS_PROFILE=1).
pub static PROFILE: std::sync::LazyLock<parking_lot::Mutex<rustc_hash::FxHashMap<&'static str, std::time::Duration>>> =
    std::sync::LazyLock::new(Default::default);

/// `GROUNDS_DOUBT=show`, read once: every finding with doubt is reported,
/// its message ending in `{doubt: Kind,..}`, or `{withheld; doubt: Kind,..}`
/// when its rule does not tolerate them all and it would not be reported.
/// A kind observed only inside a remembered computation is `deep:Kind`.
fn doubt_shown() -> bool {
    static S: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("GROUNDS_DOUBT").is_ok_and(|v| v == "show"));
    *S
}

fn profiling() -> bool {
    static P: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("GROUNDS_PROFILE").is_ok());
    *P
}

pub struct RuleSet {
    rules: Vec<Box<dyn Rule>>,
    /// kind id -> rule indices
    by_kind: Vec<Vec<usize>>,
    file_rules: Vec<usize>,
}

impl RuleSet {
    pub fn new(rules: Vec<Box<dyn Rule>>) -> RuleSet {
        let lang = grounds_syntax::language();
        let count = lang.node_kind_count();
        let mut by_kind: Vec<Vec<usize>> = vec![Vec::new(); count];
        let mut file_rules = Vec::new();
        for (i, r) in rules.iter().enumerate() {
            let m = r.meta();
            for k in m.kinds {
                if *k == "<file>" {
                    file_rules.push(i);
                    continue;
                }
                let mut found = false;
                for id in 0..count as u16 {
                    if lang.node_kind_is_named(id) && lang.node_kind_for_id(id) == Some(k) {
                        by_kind[id as usize].push(i);
                        found = true;
                    }
                }
                debug_assert!(found, "rule {} wants unknown node kind {}", m.id, k);
            }
            if m.kinds.is_empty() {
                file_rules.push(i);
            }
        }
        RuleSet { rules, by_kind, file_rules }
    }

    pub fn rules(&self) -> impl Iterator<Item = &dyn Rule> {
        self.rules.iter().map(|r| r.as_ref())
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn check(&self, fm: &FileModel, info: &FileInfo, di: Option<&DiModel>, options: &RuleOptions) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> = Vec::new();
        let flow = FlowCache::default();
        let applies = |m: &RuleMeta| match m.scope {
            // tests are there to run: style rules do not apply to them, and
            // security rules would flag the self-signed certificates, weak
            // ciphers and test credentials they use on purpose
            Scope::All => !(info.is_test && matches!(m.category, Category::Style | Category::Security)),
            Scope::Main => !info.is_test,
            Scope::Test => info.is_test,
        };
        let active: Vec<bool> = self.rules.iter().map(|r| applies(r.meta())).collect();
        {
            let root = fm.root();
            let mut cursor = root.walk();
            let mut descend = true;
            loop {
                if descend {
                    let n = cursor.node();
                    if n.is_named() {
                        if let Some(list) = self.by_kind.get(n.kind_id() as usize) {
                            for &ri in list {
                                if !active[ri] {
                                    continue;
                                }
                                let rule = &self.rules[ri];
                                let before = out.len();
                                let mut cx = Ctx { fm, src: fm.src, info, di, options, rule: rule.meta(), out: &mut out, flow: &flow };
                                let t0 = if profiling() { Some(std::time::Instant::now()) } else { None };
                                let (r, d) = doubt::scope(|| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rule.check_node(n, &mut cx))));
                                mark(&mut out[before..], d);
                                if let Some(t0) = t0 {
                                    *PROFILE.lock().entry(rule.meta().id).or_default() += t0.elapsed();
                                }
                                if r.is_err() {
                                    eprintln!("grounds: rule {} panicked on {}", rule.meta().id, info.path);
                                }
                            }
                        }
                    }
                    if cursor.goto_first_child() {
                        continue;
                    }
                }
                if cursor.goto_next_sibling() {
                    descend = true;
                    continue;
                }
                if !cursor.goto_parent() {
                    break;
                }
                descend = false;
            }
        }
        for &ri in &self.file_rules {
            if !active[ri] {
                continue;
            }
            let rule = &self.rules[ri];
            let before = out.len();
            let mut cx = Ctx { fm, src: fm.src, info, di, options, rule: rule.meta(), out: &mut out, flow: &flow };
            let t0 = if profiling() { Some(std::time::Instant::now()) } else { None };
            let (r, d) = doubt::scope(|| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rule.check_file(&mut cx))));
            mark(&mut out[before..], d);
            if let Some(t0) = t0 {
                *PROFILE.lock().entry(rule.meta().id).or_default() += t0.elapsed();
            }
            if r.is_err() {
                eprintln!("grounds: rule {} panicked on {}", rule.meta().id, info.path);
            }
        }
        // A constant nested past the bound of `constant_value` came back as
        // "not a constant", and what was derived from that (a type, a rule's
        // reason to report) may be cached anywhere in the model. Such a file
        // is not judged, like one nested too deep to parse.
        if fm.constant_cut_offs() > 0 {
            return Vec::new();
        }
        // code the parser could not read is not judged: drop findings that
        // overlap a syntax error
        // in file order, without duplicates (the first of identical ones)
        // a finding one visit made without doubt is grounded, whichever
        // visit's copy is kept
        let grounded: FxHashSet<_> = out.iter().filter(|d| d.doubt.is_empty()).map(|d| (d.rule, d.range.start, d.range.end)).collect();
        let mut seen = FxHashSet::default();
        out.retain_mut(|d| {
            let key = (d.rule, d.range.start, d.range.end);
            if grounded.contains(&key) {
                d.doubt = doubt::Doubts::NONE;
            }
            seen.insert(key)
        });
        // unknown means silent: a finding that rests on a collapsed unknown
        // its rule does not declare it tolerates is withheld (with
        // GROUNDS_DOUBT=show, kept and marked, for inspection)
        let stance: rustc_hash::FxHashMap<&str, &RuleMeta> = self.rules.iter().map(|r| (r.meta().id, r.meta())).collect();
        let show = doubt_shown();
        out.retain_mut(|d| {
            let reported = stance.get(d.rule).is_none_or(|m| m.tolerates_all(d.doubt));
            if show && !d.doubt.is_empty() {
                let mark = if reported { "doubt" } else { "withheld; doubt" };
                d.message = format!("{} {{{mark}: {}}}", d.message, d.doubt);
            }
            reported || show
        });
        out.sort_by_key(|d| (d.range.start, d.rule));
        if fm.root().has_error() {
            // the unparseable parts: tokens directly inside an ERROR node and
            // MISSING nodes. Well-formed named nodes inside an ERROR (a whole
            // method before an unfinished declaration) are still judged.
            let mut errors: Vec<(u32, u32)> = Vec::new();
            let mut stack = vec![fm.root()];
            while let Some(n) = stack.pop() {
                if n.is_missing() {
                    errors.push((n.start_byte() as u32, n.end_byte() as u32));
                    continue;
                }
                let mut c = n.walk();
                for k in n.children(&mut c) {
                    if n.is_error() && (!k.is_named() || k.is_error()) && !k.has_error() {
                        errors.push((k.start_byte() as u32, k.end_byte() as u32));
                    } else if k.has_error() || k.is_error() || k.is_missing() {
                        stack.push(k);
                    } else if n.is_error() && k.is_named() && k.child_count() == 0 {
                        // a bare identifier/type token the parser could not place
                        errors.push((k.start_byte() as u32, k.end_byte() as u32));
                    }
                }
            }
            // merged and sorted, so each diagnostic is one binary search
            errors.sort_unstable();
            let mut merged: Vec<(u32, u32)> = Vec::with_capacity(errors.len());
            for (s, e) in errors {
                match merged.last_mut() {
                    Some(last) if s <= last.1 => last.1 = last.1.max(e),
                    _ => merged.push((s, e)),
                }
            }
            out.retain(|d| {
                let i = merged.partition_point(|(_, e)| *e < d.range.start);
                !merged.get(i).is_some_and(|(s, _)| *s <= d.range.end)
            });
        }
        // unsuppressed findings, the first MAX_PER_RULE of each rule; once a
        // rule has its share, the rest are dropped without the suppression
        // check, which costs a walk up the tree each
        let sup = Suppressions::new(fm);
        let metas: rustc_hash::FxHashMap<&str, &RuleMeta> = self.rules.iter().rev().map(|r| (r.meta().id, r.meta())).collect();
        let mut kept: rustc_hash::FxHashMap<&str, usize> = Default::default();
        out.retain(|d| {
            let n = kept.entry(d.rule).or_default();
            if *n >= MAX_PER_RULE || sup.is_suppressed(fm, d, metas.get(d.rule).copied()) {
                return false;
            }
            *n += 1;
            true
        });
        out
    }
}

/// The findings one rule visit pushed carry what that visit observed.
fn mark(new: &mut [Diagnostic], d: doubt::Doubts) {
    for x in new.iter_mut().filter(|x| !x.attributed) {
        x.doubt = d;
        x.attributed = true;
    }
}

/// Findings of one rule kept per file. The most on the corpus is 161 (an
/// argument order rule in a test of a hundred assertions); a file cut off in
/// a million `;` gave a million `stray-semicolon` findings, each costing a
/// walk over the million siblings in tree-sitter.
pub const MAX_PER_RULE: usize = 2000;

struct Suppressions {
    /// (0-based line, lowercase comment text, comment alone on its line)
    comments: Vec<(usize, String, bool)>,
}

impl Suppressions {
    fn new(fm: &FileModel) -> Suppressions {
        let mut comments = Vec::new();
        let mut stack = vec![fm.root()];
        while let Some(n) = stack.pop() {
            let mut c = n.walk();
            for k in n.children(&mut c) {
                if k.kind() == "line_comment" || k.kind() == "block_comment" {
                    let t = fm.text(k).to_ascii_lowercase();
                    if t.contains("nosonar") || t.contains("grounds-ignore") || t.contains("nopmd") || t.contains("noinspection") {
                        let line_start = fm.src[..k.start_byte()].rfind('\n').map_or(0, |i| i + 1);
                        let alone = fm.src[line_start..k.start_byte()].trim().is_empty();
                        comments.push((k.start_position().row, t, alone));
                    }
                } else if k.child_count() > 0 {
                    stack.push(k);
                }
            }
        }
        Suppressions { comments }
    }

    fn comment_suppresses(&self, line: usize, meta: Option<&RuleMeta>, rule: &str) -> bool {
        for (l, t, alone) in &self.comments {
            let same_line = *l == line;
            // a comment on the line before counts only when it stands alone:
            // `code; // grounds-ignore` covers its own line, not the next
            let prev_line = *alone && *l + 1 == line;
            if !(same_line || prev_line) {
                continue;
            }
            if same_line && (t.contains("nosonar") || t.contains("nopmd")) {
                return true;
            }
            if let Some(pos) = t.find("grounds-ignore") {
                let rest = t[pos + "grounds-ignore".len()..].trim_start_matches([':', ' ', '[']);
                if rest.is_empty() || rest.starts_with("*/") {
                    return true;
                }
                if rest.split([',', ' ', ']']).any(|r| r.trim() == rule || r.trim() == format!("grounds:{rule}")) {
                    return true;
                }
            }
            if let (Some(pos), Some(m)) = (t.find("noinspection"), meta) {
                let rest = &t[pos..];
                if m.sources.iter().any(|s| s.strip_prefix("intellij:").is_some_and(|i| rest.contains(&i.to_ascii_lowercase()))) {
                    return true;
                }
            }
        }
        false
    }

    fn is_suppressed(&self, fm: &FileModel, d: &Diagnostic, meta: Option<&RuleMeta>) -> bool {
        let line = fm.file.lines.line_of(d.range.start);
        if self.comment_suppresses(line, meta, d.rule) {
            return true;
        }
        let Some(node) = fm.root().descendant_for_byte_range(d.range.start as usize, d.range.end.max(d.range.start) as usize) else { return false };
        // `ancestors()`, not a `parent()` loop: each `parent()` costs the depth
        // of the tree, which made deeply nested statements cubic
        for n in std::iter::once(node).chain(node.ancestors()) {
            if let Some(m) = n.modifiers() {
                for a in m.kids() {
                    if matches!(a.kind(), "annotation" | "marker_annotation") {
                        let name = a.field("name").map(|x| fm.text(x)).unwrap_or("");
                        let simple = name.rsplit('.').next().unwrap_or(name);
                        if simple == "SuppressWarnings" || simple == "SuppressFBWarnings" {
                            let vals: Vec<String> = string_literals(fm, a);
                            if vals.iter().any(|v| matches_suppression(v, d.rule, meta)) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }
}

fn string_literals(fm: &FileModel, n: Node) -> Vec<String> {
    n.descendants().filter(|k| k.kind() == "string_literal").filter_map(|k| grounds_syntax::string_literal_value(k, fm.src)).collect()
}

fn matches_suppression(v: &str, rule: &str, meta: Option<&RuleMeta>) -> bool {
    let v = v.trim();
    if v == "all" || v == rule || v == format!("grounds:{rule}") || v == "grounds" {
        return true;
    }
    let Some(m) = meta else { return false };
    for s in m.sources {
        if let Some(sonar) = s.strip_prefix("sonar:") {
            if v == sonar || v == format!("java:{sonar}") || v == format!("squid:{sonar}") {
                return true;
            }
        }
        if let Some(pmd) = s.strip_prefix("pmd:") {
            if v == format!("PMD.{pmd}") || v == "PMD" {
                return true;
            }
        }
        if let Some(sb) = s.strip_prefix("spotbugs:") {
            if v == sb || (v.len() >= 2 && sb.starts_with(v)) {
                return true;
            }
        }
        if let Some(ep) = s.strip_prefix("errorprone:") {
            if v == ep {
                return true;
            }
        }
        if let Some(ij) = s.strip_prefix("intellij:") {
            if v == ij {
                return true;
            }
        }
    }
    // common javac lint names
    matches!((v, rule), ("unused", r) if r.starts_with("unused-")) || matches!((v, rule), ("deprecation", "deprecated-api-usage") | ("rawtypes", "raw-type") | ("fallthrough", "switch-fallthrough") | ("removal", "deprecated-api-usage"))
}
