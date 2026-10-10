//! Rules built on control-flow and dataflow analyses.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_flow::analyses::*;
use grounds_flow::{Analysis, EdgeKind};
use grounds_index::*;
use grounds_semantic::LocalKind;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(NullDereference),
        Box::new(RedundantNullCheck),
        Box::new(DeadStore),
        Box::new(ResourceLeak),
        Box::new(OptionalGetWithoutCheck),
        Box::new(InfiniteRecursion),
        Box::new(UseAfterClose),
        Box::new(UninitializedFieldRead),
        Box::new(NullPassedToNonNull),
        Box::new(NonNullReturnsNull),
    ]
}

declare_rule!(NullDereference {
    id: "null-dereference",
    name: "Null pointer dereference",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S2259", "spotbugs:NP_ALWAYS_NULL", "spotbugs:NP_NULL_ON_SOME_PATH", "spotbugs:NP_GUARANTEED_DEREF", "errorprone:NullAway", "intellij:DataFlowIssue"],
    kinds: ["<file>"],
    description: "A local variable that is null on some path is dereferenced (method call, field access, array access, unboxing). Typical causes: a null check that guards the wrong branch, a variable initialized to null and assigned only conditionally, or a @Nullable value used unchecked.",
});

impl Rule for NullDereference {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        let mut found: Vec<(Node<'t>, String, bool)> = Vec::new();
        for c in cx.callables() {
            let cfg = cx.cfg(c);
            let res = cx.nullness(c);
            let na = NullnessAnalysis { fm: cx.fm, cfg: &cfg };
            let mut reported = rustc_hash::FxHashSet::default();
            res.for_each_before(&cfg, &na, |e, f| {
                for (i, node) in na.derefs(e) {
                    let Some(state) = f.get(&i) else { continue };
                    if matches!(*state, Nullness::NonNull | Nullness::MaybeAssignedNull) || !reported.insert(i) {
                        continue;
                    }
                    let name = cx.fm.binder.decls[i].name;
                    let definite = state.is_definitely_null();
                    let msg = if definite { format!("`{name}` is always null here") } else { format!("`{name}` may be null here") };
                    found.push((node, msg, definite));
                }
            });
        }
        for (n, m, definite) in found {
            if definite {
                cx.report(n, m);
            } else {
                cx.report_with_severity(n, Severity::Warning, m);
            }
        }
    }
}

declare_rule!(RedundantNullCheck {
    id: "redundant-null-check",
    name: "Null check of a value that cannot be null",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2583", "sonar:S2589", "spotbugs:RCN_REDUNDANT_NULLCHECK_OF_NONNULL_VALUE", "spotbugs:RCN_REDUNDANT_NULLCHECK_WOULD_HAVE_BEEN_A_NPE", "spotbugs:RCN_REDUNDANT_NULLCHECK_OF_NULL_VALUE", "intellij:ConstantConditions"],
    kinds: ["<file>"],
    description: "The variable was already dereferenced or assigned a non-null value, so the null check is always true/false. Either the check is dead code, or the earlier dereference should have been guarded.",
});

impl Rule for RedundantNullCheck {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        let mut found: Vec<(Node<'t>, String)> = Vec::new();
        for c in cx.callables() {
            let cfg = cx.cfg(c);
            let res = cx.nullness(c);
            let na = NullnessAnalysis { fm: cx.fm, cfg: &cfg };
            res.for_each_before(&cfg, &na, |e, f| {
                if e.kind() != "binary_expression" || !matches!(op(e), "==" | "!=") {
                    return;
                }
                let Some((l, r)) = lr(e) else { return };
                let var = if is_null_literal(r) { l } else if is_null_literal(l) { r } else { return };
                let var = var.unparen();
                if var.kind() != "identifier" {
                    return;
                }
                let Some(i) = cx.fm.binder.decl_index_of_ref(var) else { return };
                // inside assert statements checks are documentation
                if e.ancestors().any(|a| a.kind() == "assert_statement") {
                    return;
                }
                let d = &cx.fm.binder.decls[i];
                if d.kind == LocalKind::Param || d.kind == LocalKind::LambdaParam {
                    // parameters are only known non-null after a dereference inside the method
                }
                match f.get(&i) {
                    Some(Nullness::NonNull) => {
                        let always = if op(e) == "!=" { "true" } else { "false" };
                        found.push((e, format!("`{}` cannot be null here; this check is always {always}", d.name)));
                    }
                    Some(Nullness::Null) | Some(Nullness::AssignedNull) => {
                        let always = if op(e) == "==" { "true" } else { "false" };
                        found.push((e, format!("`{}` is always null here; this check is always {always}", d.name)));
                    }
                    _ => {}
                }
            });
        }
        for (n, m) in found {
            cx.report(n, m);
        }
    }
}

declare_rule!(DeadStore {
    id: "dead-store",
    name: "Value assigned to a variable is never used",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S1854", "spotbugs:DLS_DEAD_LOCAL_STORE", "pmd:UnusedAssignment", "intellij:UnusedAssignment"],
    kinds: ["<file>"],
    description: "The assigned value is overwritten or goes out of scope before it is ever read. Either the computation is wasted or a later read uses the wrong variable.",
});

fn trivial_initializer(v: Node, src: &str) -> bool {
    let v = v.unparen();
    match v.kind() {
        "null_literal" | "true" | "false" => true,
        "decimal_integer_literal" | "decimal_floating_point_literal" => matches!(v.text(src).trim_end_matches(['L', 'l', 'd', 'D', 'f', 'F']), "0" | "1" | "0.0" | "1.0"),
        "unary_expression" => v.text(src) == "-1",
        "string_literal" => v.text(src) == "\"\"",
        _ => false,
    }
}

impl Rule for DeadStore {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        let mut found: Vec<(Node<'t>, String)> = Vec::new();
        for c in cx.callables() {
            let cfg = cx.cfg(c);
            let res = cx.liveness(c);
            let lv = Liveness { fm: cx.fm };
            for e in cfg.elements() {
                let Some((i, overwrite)) = local_def(cx.fm, e) else { continue };
                if !overwrite || !matches!(e.kind(), "assignment_expression" | "variable_declarator") {
                    continue;
                }
                let d = &cx.fm.binder.decls[i];
                if d.name.starts_with("unused") || d.name.starts_with("ignored") || d.name.starts_with('_') {
                    continue;
                }
                // never-read locals are reported by unused-local-variable
                if cx.fm.binder.uses[i].iter().all(|u| is_assignment_target(*u)) {
                    continue;
                }
                let value = if e.kind() == "variable_declarator" { e.field("value") } else { e.field("right") };
                let Some(value) = value else { continue };
                if e.kind() == "variable_declarator" && trivial_initializer(value, cx.src) {
                    continue;
                }
                // captured locals are read by lambdas/inner classes we don't track per-path
                if d.owner != Some(c) {
                    continue;
                }
                // compact constructor parameters are assigned to the record fields afterwards
                if c.kind() == "compact_constructor_declaration" && d.kind == LocalKind::Param {
                    continue;
                }
                let Some(after) = res.after_backward(&cfg, &lv, e) else { continue };
                if !after.contains(&i) {
                    found.push((e, format!("The value assigned to `{}` is never read", d.name)));
                }
            }
        }
        for (n, m) in found {
            cx.report(n, m);
        }
    }
}

fn is_assignment_target(u: Node) -> bool {
    matches!(u.parent_unparen(), Some(p) if p.kind() == "assignment_expression" && p.field("left").map(|l| l.unparen()) == Some(u) && p.field("operator").map(|o| o.kind()) == Some("="))
}

declare_rule!(ResourceLeak {
    id: "resource-leak",
    name: "Resource not closed",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2095", "spotbugs:OBL_UNSATISFIED_OBLIGATION", "spotbugs:OS_OPEN_STREAM", "pmd:CloseResource", "errorprone:StreamResourceLeak"],
    kinds: ["<file>"],
    description: "A stream, reader, connection or other Closeable created here is not closed on every path (and not returned or handed off). Leaked file handles and connections exhaust OS limits and pools. Use try-with-resources.",
});

impl Rule for ResourceLeak {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        let mut found: Vec<(usize, String)> = Vec::new();
        for c in cx.callables() {
            let cfg = cx.cfg(c);
            let res = cx.resources(c);
            if !res.reached[cfg.exit] {
                continue;
            }
            let mut seen = rustc_hash::FxHashSet::default();
            for (i, s) in &res.block_in[cfg.exit] {
                if let ResState::Open(site) = s {
                    if seen.insert(*site) {
                        found.push((*site, format!("`{}` is not closed on every path; use try-with-resources", cx.fm.binder.decls[*i].name)));
                    }
                }
            }
        }
        if found.is_empty() {
            return;
        }
        // map node ids back to nodes
        let root = cx.fm.root();
        for n in root.descendants() {
            if let Some(pos) = found.iter().position(|(id, _)| *id == n.id()) {
                let (_, msg) = found.remove(pos);
                cx.report(n, msg);
                if found.is_empty() {
                    break;
                }
            }
        }
    }
}

declare_rule!(OptionalGetWithoutCheck {
    id: "optional-get-without-check",
    name: "Optional.get() without isPresent() check",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S3655", "intellij:OptionalGetWithoutIsPresent", "errorprone:OptionalNotPresent"],
    kinds: ["<file>"],
    description: "`get()` throws NoSuchElementException on an empty Optional. Check `isPresent()` first, or better use `orElse`, `orElseThrow` with a meaningful exception, `map` or `ifPresent`.",
});

impl Rule for OptionalGetWithoutCheck {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        let mut found: Vec<Node<'t>> = Vec::new();
        for c in cx.callables() {
            let cfg = cx.cfg(c);
            let res = cx.optional_presence(c);
            let oa = OptionalPresence { fm: cx.fm };
            res.for_each_before(&cfg, &oa, |e, f| {
                if e.kind() != "method_invocation" || !matches!(cx.fm.call_name(e), Some("get" | "getAsInt" | "getAsLong" | "getAsDouble")) || !cx.fm.args_of(e).is_empty() {
                    return;
                }
                if let Some(i) = oa.optional_local(e.field("object")) {
                    if !f.contains(&i) {
                        found.push(e);
                    }
                }
            });
        }
        for n in found {
            cx.report(n, "`get()` on an Optional that may be empty; check `isPresent()` or use `orElse`/`orElseThrow`");
        }
    }
}

declare_rule!(InfiniteRecursion {
    id: "infinite-recursion",
    name: "Method recurses unconditionally",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S2190", "spotbugs:IL_INFINITE_RECURSIVE_LOOP", "errorprone:InfiniteRecursion"],
    kinds: ["method_declaration"],
    description: "Every path through the method calls itself on the same object before returning, so it can only end with a StackOverflowError (often a getter calling itself instead of returning the field).",
});

impl Rule for InfiniteRecursion {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.field("body").is_none() {
            return;
        }
        let Some(m) = cx.fm.method_of_decl(n) else { return };
        let range = m.range;
        let cfg = cx.cfg(n);
        // blocks containing a self call (implicit or `this.` receiver)
        let is_self_call = |e: Node<'t>| -> bool {
            if e.kind() != "method_invocation" || cx.fm.call_name(e) != Some(&*m.name) {
                return false;
            }
            if e.field("object").is_some_and(|o| o.kind() != "this") {
                return false;
            }
            cx.fm.select_method(e).is_some_and(|c| c.method.range == range && !c.ambiguous && c.method.params.len() == m.params.len())
        };
        let mut blocked = vec![false; cfg.blocks.len()];
        let mut any = false;
        for (bi, b) in cfg.blocks.iter().enumerate() {
            if b.elems.iter().any(|e| is_self_call(*e)) {
                blocked[bi] = true;
                any = true;
            }
        }
        if !any {
            return;
        }
        // can we reach a normal exit without passing a self call?
        let mut seen = vec![false; cfg.blocks.len()];
        let mut stack = vec![cfg.entry];
        let mut reaches_exit = false;
        while let Some(b) = stack.pop() {
            if seen[b] {
                continue;
            }
            seen[b] = true;
            if b == cfg.exit || b == cfg.exc_exit {
                reaches_exit = true;
                break;
            }
            if blocked[b] {
                // the self call happens before anything after it in this block; exceptional exits
                // from statements before the call could escape, so only treat as blocked
                // if the call precedes any throwing element
                let first_throw = cfg.blocks[b].elems.iter().position(|e| matches!(e.kind(), "throw_statement" | "return_statement"));
                let call_pos = cfg.blocks[b].elems.iter().position(|e| is_self_call(*e));
                if call_pos < first_throw || first_throw.is_none() {
                    continue;
                }
            }
            for (s, k) in &cfg.blocks[b].succs {
                if matches!(k, EdgeKind::Exception) {
                    continue;
                }
                stack.push(*s);
            }
        }
        if !reaches_exit {
            if let Some(nm) = n.field("name") {
                cx.report(nm, format!("`{}` calls itself on every path; this always overflows the stack", m.name));
            }
        }
    }
}

declare_rule!(UseAfterClose {
    id: "use-after-close",
    name: "Resource used after close()",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2095", "spotbugs:OBL_UNSATISFIED_OBLIGATION_EXCEPTION_EDGE"],
    kinds: ["<file>"],
    description: "A method is called on a stream/reader/connection after it was closed on every path leading here; this throws an IOException or silently does nothing.",
});

impl Rule for UseAfterClose {
    rule_meta!();
    fn check_file<'t>(&self, cx: &mut Ctx<'_, 't>) {
        let mut found: Vec<Node<'t>> = Vec::new();
        for c in cx.callables() {
            let cfg = cx.cfg(c);
            let res = cx.resources(c);
            let ra = ResourceAnalysis { fm: cx.fm };
            res.for_each_before(&cfg, &ra, |e, f| {
                if e.kind() != "method_invocation" {
                    return;
                }
                let name = cx.fm.call_name(e).unwrap_or("");
                if matches!(name, "close" | "isClosed" | "toString" | "hashCode" | "equals" | "getClass") {
                    return;
                }
                let Some(o) = e.field("object").map(|o| o.unparen()) else { return };
                if o.kind() != "identifier" {
                    return;
                }
                if let Some(i) = cx.fm.binder.decl_index_of_ref(o) {
                    if f.get(&i) == Some(&ResState::Closed) {
                        found.push(e);
                    }
                }
            });
        }
        for n in found {
            cx.report(n, "Resource is used after being closed");
        }
    }
}

declare_rule!(UninitializedFieldRead {
    id: "uninitialized-field-read",
    name: "Final field read before initialization in constructor",
    category: Bug,
    severity: Error,
    sources: ["spotbugs:UR_UNINIT_READ", "sonar:S3077"],
    kinds: ["constructor_declaration"],
    description: "The constructor reads a final field before assigning it, so it sees the default value (null/0/false).",
});

impl Rule for UninitializedFieldRead {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        let Some(body) = n.field("body") else { return };
        // `this(...)` delegates initialization
        if body.kids().first().is_some_and(|k| k.kind() == "explicit_constructor_invocation" && k.field("constructor").is_some_and(|c| c.kind() == "this")) {
            return;
        }
        let candidates: Vec<&FieldInfo> = cls.fields.iter().filter(|f| f.flags.is_final() && !f.flags.is_static() && !f.flags.has(Flags::RECORD_COMPONENT)).collect();
        if candidates.is_empty() {
            return;
        }
        let raw_init: Vec<String> = cx
            .fm
            .index
            .raw_class(&cls.name)
            .map(|(_, rc)| rc.fields.iter().filter(|f| f.has_initializer).map(|f| f.name.to_string()).collect())
            .unwrap_or_default();
        let mut assigned: rustc_hash::FxHashSet<String> = raw_init.into_iter().collect();
        let this_field = |d: Node<'t>| -> Option<FieldRef> {
            let f = cx.fm.field_ref(d)?;
            if f.owner != cls.name || !candidates.iter().any(|c| c.name == f.field.name) {
                return None;
            }
            match d.kind() {
                "identifier" => {
                    if d.parent().is_some_and(|p| p.kind() == "field_access" && p.field("field") == Some(d)) {
                        return None;
                    }
                    Some(f)
                }
                "field_access" => d.field("object").filter(|o| o.kind() == "this").map(|_| f),
                _ => None,
            }
        };
        let is_assign_target = |d: Node<'t>| d.parent_unparen().is_some_and(|p| p.kind() == "assignment_expression" && p.field("left").map(|l| l.unparen()) == Some(d));
        for stmt in body.kids() {
            // first pass: assignments anywhere in this statement (e.g. in both branches of an if)
            let mut stmt_assigned: Vec<String> = Vec::new();
            for d in stmt.descendants() {
                if matches!(d.kind(), "identifier" | "field_access") && is_assign_target(d) {
                    if let Some(f) = this_field(d) {
                        stmt_assigned.push(f.field.name.to_string());
                    }
                }
            }
            for d in stmt.descendants() {
                if !matches!(d.kind(), "identifier" | "field_access") || d.ancestors().take_while(|a| *a != stmt).any(|a| matches!(a.kind(), "lambda_expression" | "class_body")) {
                    continue;
                }
                let Some(f) = this_field(d) else { continue };
                if assigned.contains(&*f.field.name) || is_assign_target(d) {
                    continue;
                }
                // read inside the statement that also assigns it (e.g. `x = compute(); use(x)` in a block)
                if stmt_assigned.iter().any(|a| a == &*f.field.name) && stmt.kind() != "expression_statement" {
                    continue;
                }
                cx.report(d, format!("Final field `{}` is read before it is assigned", f.field.name));
                assigned.insert(f.field.name.to_string());
            }
            assigned.extend(stmt_assigned);
        }
    }
}

fn is_nonnull_annotation(a: &AnnotationInfo) -> bool {
    matches!(a.simple_name(), "NonNull" | "Nonnull" | "NotNull")
}

declare_rule!(NullPassedToNonNull {
    id: "null-passed-to-nonnull-parameter",
    name: "null passed to a @NonNull parameter",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["spotbugs:NP_NONNULL_PARAM_VIOLATION", "errorprone:NullAway", "intellij:ConstantConditions"],
    kinds: ["method_invocation", "object_creation_expression"],
    description: "The parameter is annotated as non-null, but the call passes a `null` literal.",
});

impl Rule for NullPassedToNonNull {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let args = cx.fm.args_of(n);
        if !args.iter().any(|a| is_null_literal(*a)) {
            return;
        }
        let Some(c) = cx.fm.select_method(n) else { return };
        if c.ambiguous {
            return;
        }
        for (i, a) in args.iter().enumerate() {
            if !is_null_literal(*a) {
                continue;
            }
            if let Some(p) = c.method.params.get(i) {
                if p.annotations.iter().any(is_nonnull_annotation) {
                    cx.report(*a, format!("`null` passed to non-null parameter `{}`", p.name));
                }
            }
        }
    }
}

declare_rule!(NonNullReturnsNull {
    id: "nonnull-method-returns-null",
    name: "@NonNull method returns null",
    category: Bug,
    severity: Error,
    sources: ["spotbugs:NP_NONNULL_RETURN_VIOLATION", "errorprone:NullAway"],
    kinds: ["return_statement"],
    description: "The method is annotated to never return null, yet returns a `null` literal.",
});

impl Rule for NonNullReturnsNull {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !n.first_kid().is_some_and(is_null_literal) {
            return;
        }
        let Some(m) = enclosing_method_decl(n) else { return };
        if n.ancestors().take_while(|a| *a != m).any(|a| a.kind() == "lambda_expression") {
            return;
        }
        if cx.fm.annotations_of(m).iter().any(is_nonnull_annotation) {
            cx.report(n, "Method annotated non-null returns null");
        }
    }
}

#[allow(dead_code)]
fn analysis_type_check<'a, 't>(a: &NullnessAnalysis<'a, 't>) -> NullFact {
    a.bottom()
}
