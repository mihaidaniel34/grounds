//! Exception handling.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_semantic::MethodMatcher;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(EmptyCatchBlock),
        Box::new(CatchThrowable),
        Box::new(CatchNullPointer),
        Box::new(CatchGenericException),
        Box::new(ThrowRawException),
        Box::new(LostStackTrace),
        Box::new(AbruptFinally),
        Box::new(InterruptedSwallowed),
        Box::new(ExceptionNotThrown),
        Box::new(RethrowOnly),
        Box::new(LogAndRethrow),
        Box::new(PrintStackTrace),
        Box::new(CatchInstanceof),
        Box::new(FinalizeUsage),
        Box::new(IteratorNextWithoutNoSuchElement),
        Box::new(HasNextCallsNext),
        Box::new(ThrowsInFinallyClose),
        Box::new(ExceptionIgnoredMessage),
        Box::new(ThrowNull),
        Box::new(CatchAndReturnNull),
    ]
}

fn catch_param<'t>(c: Node<'t>) -> Option<Node<'t>> {
    c.kid_of_kind("catch_formal_parameter")
}

fn catch_types<'t>(c: Node<'t>, cx: &Ctx<'_, 't>) -> Vec<Ty> {
    catch_param(c).and_then(|p| p.kid_of_kind("catch_type")).map(|t| t.kids().into_iter().map(|k| cx.fm.resolve_type_node(k)).collect()).unwrap_or_default()
}

fn catch_var_name<'t>(c: Node<'t>, src: &'t str) -> Option<&'t str> {
    catch_param(c).and_then(|p| p.field("name")).map(|n| n.text(src))
}

fn block_has_comment(b: Node) -> bool {
    let mut c = b.walk();
    let r = b.children(&mut c).any(|k| grounds_syntax::is_comment(k.kind()));
    r
}

declare_rule!(EmptyCatchBlock {
    id: "empty-catch-block",
    name: "Empty catch block",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S108", "sonar:S2486", "pmd:EmptyCatchBlock", "spotbugs:DE_MIGHT_IGNORE", "intellij:EmptyCatchBlock"],
    kinds: ["catch_clause"],
    description: "Silently swallowing an exception hides failures and makes debugging painful. Handle it, log it, or at least explain with a comment (or name the variable `ignored`) why it is safe to ignore.",
});

impl Rule for EmptyCatchBlock {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(body) = n.field("body") else { return };
        if !body.kids().is_empty() || block_has_comment(body) {
            return;
        }
        if let Some(v) = catch_var_name(n, cx.src) {
            if v == "_" || v.starts_with("ignore") || v.starts_with("expected") || v == "unused" {
                return;
            }
        }
        // InterruptedException has its own rule
        if catch_types(n, cx).iter().any(|t| t.is_class("java.lang.InterruptedException")) {
            return;
        }
        if cx.is_test() {
            return;
        }
        cx.report(n.child(0).unwrap_or(n), "Empty catch block swallows the exception; handle it or document why it is ignored");
    }
}

declare_rule!(CatchThrowable {
    id: "catch-throwable",
    name: "Catching Throwable or Error",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S1181", "pmd:AvoidCatchingThrowable", "intellij:CatchGenericClass"],
    kinds: ["catch_clause"],
    description: "Catching `Throwable`/`Error` also intercepts `OutOfMemoryError`, `StackOverflowError` and `ThreadDeath`, which the application cannot sensibly recover from. Catch `Exception` (or something narrower).",
});

impl Rule for CatchThrowable {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        for t in catch_types(n, cx) {
            if matches!(t.class_name(), Some("java.lang.Throwable" | "java.lang.Error")) {
                // propagating the throwable (rethrow, wrap, hand to a future/callback/addSuppressed)
                // is not swallowing it; only logging-and-continuing is the problem
                if let (Some(b), Some(v)) = (n.field("body"), catch_var_name(n, cx.src)) {
                    let mut throws = Vec::new();
                    find_kind(b, "throw_statement", &mut throws);
                    if throws.iter().any(|t| t.first_kid().is_some_and(|e| e.descendants().any(|d| d.kind() == "identifier" && d.text(cx.src) == v))) {
                        return;
                    }
                    let mut calls = Vec::new();
                    find_kind(b, "method_invocation", &mut calls);
                    let propagated = calls.iter().any(|c| {
                        !is_log_like(*c, cx) && !is_log_level_call_on_unknown(*c, v, cx) && cx.fm.args_of(*c).iter().any(|a| cx.text(*a) == v) && !matches!(cx.fm.call_name(*c), Some("println" | "print" | "printf"))
                    });
                    if propagated {
                        return;
                    }
                }
                if enclosing_method_decl(n).is_some_and(|m| is_main_method(m, cx.src) || method_name(m, cx.src) == "run") {
                    return;
                }
                if let Some(p) = catch_param(n) {
                    cx.report(p, format!("Catching `{}` also catches unrecoverable JVM errors", t.display_simple()));
                }
            }
        }
    }
}

declare_rule!(CatchNullPointer {
    id: "catch-null-pointer",
    name: "Catching NullPointerException or a similar programming error",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S1696", "pmd:AvoidCatchingNPE"],
    kinds: ["catch_clause"],
    description: "Catching `NullPointerException`, `ArrayIndexOutOfBoundsException` or `ClassCastException` masks a programming error instead of preventing it with a check. `ClassCastException` is only reported when the try block casts (or calls `Class.cast()`) itself, where `instanceof` can test first; thrown from inside a library call there is nothing to check.",
});

/// Does the try block of catch clause `c` cast a reference itself (a cast
/// expression or `Class.cast()`), so `instanceof` could replace catching
/// ClassCastException? Primitive casts never throw it.
fn try_casts<'t>(c: Node<'t>, cx: &Ctx<'_, 't>) -> bool {
    let Some(t) = c.parent() else { return false };
    [t.field("resources"), t.field("body")].into_iter().flatten().any(|b| {
        b.descendants().any(|d| match d.kind() {
            "cast_expression" => d.field("type").is_some_and(|ty| !matches!(ty.kind(), "integral_type" | "floating_point_type" | "boolean_type")),
            "method_invocation" => cx.fm.call_matches(d, &MethodMatcher::instance("java.lang.Class").named(&["cast"])),
            _ => false,
        })
    })
}

impl Rule for CatchNullPointer {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        for t in catch_types(n, cx) {
            let msg = match t.class_name() {
                Some("java.lang.NullPointerException" | "java.lang.ArrayIndexOutOfBoundsException") => "check the condition instead",
                Some("java.lang.ClassCastException") if try_casts(n, cx) => "check with `instanceof` before casting",
                _ => continue,
            };
            if let Some(p) = catch_param(n) {
                cx.report(p, format!("Catching `{}` hides a bug; {msg}", t.display_simple()));
            }
        }
    }
}

declare_rule!(CatchGenericException {
    id: "catch-generic-exception",
    name: "Catching generic Exception",
    category: BadPractice,
    severity: Info,
    enabled: false,
    sources: ["pmd:AvoidCatchingGenericException", "sonar:S2221"],
    kinds: ["catch_clause"],
    description: "Catching `Exception`/`RuntimeException` also catches unexpected runtime failures. Catch the specific checked exceptions you can handle.",
});

impl Rule for CatchGenericException {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        for t in catch_types(n, cx) {
            if matches!(t.class_name(), Some("java.lang.Exception" | "java.lang.RuntimeException")) {
                if let Some(p) = catch_param(n) {
                    cx.report(p, format!("Catching generic `{}`", t.display_simple()));
                }
            }
        }
    }
}

declare_rule!(ThrowRawException {
    id: "throw-generic-exception",
    name: "Throwing a generic exception type",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S112", "pmd:AvoidThrowingRawExceptionTypes"],
    kinds: ["throw_statement"],
    description: "`throw new RuntimeException(...)`/`Exception`/`Throwable`/`Error` forces callers to catch everything to handle it. Throw a specific (or custom) exception type.",
});

impl Rule for ThrowRawException {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = n.first_kid().map(|e| e.unparen()) else { return };
        if e.kind() != "object_creation_expression" {
            return;
        }
        let t = cx.type_of(e);
        if matches!(t.class_name(), Some("java.lang.RuntimeException" | "java.lang.Exception" | "java.lang.Throwable" | "java.lang.Error")) {
            cx.report(e, format!("Throwing generic `{}`; use a more specific exception", t.display_simple()));
        }
    }
}

declare_rule!(LostStackTrace {
    id: "lost-stack-trace",
    name: "Exception rethrown without its cause",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S1166", "pmd:PreserveStackTrace", "errorprone:UnusedException"],
    kinds: ["throw_statement"],
    description: "Throwing a new exception from a catch block without passing the caught one as the cause discards the original stack trace, the most useful debugging information. Pass it: `new XException(msg, e)`.",
});

impl Rule for LostStackTrace {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = n.ancestors().take_while(|a| !matches!(a.kind(), "lambda_expression" | "class_body" | "method_declaration")).find(|a| a.kind() == "catch_clause") else {
            return;
        };
        let Some(v) = catch_var_name(c, cx.src) else { return };
        let Some(e) = n.first_kid().map(|e| e.unparen()) else { return };
        if e.kind() != "object_creation_expression" {
            return;
        }
        let Some(body) = c.field("body") else { return };
        // The exception survives when it (or a Throwable obtained from it, such as
        // `e.getCause()`) is passed on; `e.getMessage()` or `e.getCode()` alone
        // drops the stack trace. Locals initialized from it are aliases.
        let keeps = |d: Node<'t>, names: &[&str]| {
            d.kind() == "identifier"
                && names.contains(&d.text(cx.src))
                && match d.parent() {
                    Some(p) if p.kind() == "method_invocation" && p.field("object") == Some(d) => {
                        if p.field("name").is_some_and(|nm| matches!(nm.text(cx.src), "getMessage" | "getLocalizedMessage" | "toString")) {
                            return false;
                        }
                        let t = cx.type_of(p);
                        t.is_unknown() || t.class_name().is_some_and(|c| c == "java.lang.Throwable" || cx.fm.is_subclass(c, "java.lang.Throwable").is_yes())
                    }
                    _ => true,
                }
        };
        let throwable_or_unknown = |t: Ty| t.is_unknown() || t.class_name().is_some_and(|c| c == "java.lang.Throwable" || cx.fm.is_subclass(c, "java.lang.Throwable").is_yes());
        let mut names = vec![v];
        for d in body.descendants().filter(|d| d.kind() == "variable_declarator") {
            if let (Some(name), Some(value)) = (d.field("name"), d.field("value")) {
                // `Throwable cause = e.getCause()`, not `String msg = String.valueOf(e)`
                let declared = d.parent().and_then(|decl| decl.field("type")).map(|t| if t.text(cx.src) == "var" { cx.type_of(value) } else { cx.fm.resolve_type_node(t) });
                if declared.is_some_and(throwable_or_unknown) && value.descendants().any(|x| keeps(x, &[v])) {
                    names.push(name.text(cx.src));
                }
            }
        }
        // `lastFailure = e;` into a Throwable local, if that is its last
        // assignment before the throw
        let assigns: Vec<Node<'t>> = body.descendants().filter(|d| d.kind() == "assignment_expression" && d.start_byte() < n.start_byte()).collect();
        for d in &assigns {
            if let (Some(l), Some(r)) = (d.field("left").map(|x| x.unparen()), d.field("right")) {
                let Some(li) = cx.fm.local_ref(l) else { continue };
                let last = assigns.iter().filter(|a| a.field("left").and_then(|x| cx.fm.local_ref(x.unparen())) == Some(li)).max_by_key(|a| a.start_byte());
                if last == Some(d) && throwable_or_unknown(cx.type_of(l)) && r.descendants().chain(std::iter::once(r)).any(|x| keeps(x, &[v])) {
                    names.push(l.text(cx.src));
                }
            }
        }
        // `e.getCause() instanceof SocketException se`
        for d in body.descendants().filter(|d| d.kind() == "instanceof_expression") {
            if let (Some(left), Some(name)) = (d.field("left"), d.field("name")) {
                let bound = d.field("right").map(|t| cx.fm.resolve_type_node(t));
                if bound.is_some_and(throwable_or_unknown) && left.descendants().any(|x| keeps(x, &[v])) {
                    names.push(name.text(cx.src));
                }
            }
        }
        if e.descendants().any(|d| keeps(d, &names)) {
            return;
        }
        // handed to a logger or a void (or unresolved) handler by a statement
        // that runs before the throw on its path: an earlier statement of a
        // block enclosing the throw, inside the catch block
        let hands_off = |st: Node<'t>| {
            st.kind() == "expression_statement"
                && st.first_kid().is_some_and(|d| {
                    // kept for later: `lastError = e;`, `failures.add(e);`
                    let stored = d.kind() == "assignment_expression"
                        && d.field("right").is_some_and(|r| names.contains(&cx.text(r.unparen())))
                        && d.field("left").is_some_and(|l| cx.fm.local_ref(l.unparen()).is_none());
                    let collected = d.kind() == "method_invocation"
                        && matches!(cx.fm.call_name(d), Some("add" | "addSuppressed" | "offer" | "put" | "set" | "complete" | "completeExceptionally" | "onError"))
                        && cx.fm.args_of(d).iter().any(|a| names.contains(&cx.text(*a)));
                    stored
                        || collected
                        || (d.kind() == "method_invocation"
                            && !matches!(cx.fm.call_name(d), Some("println" | "print" | "printf"))
                            && cx.fm.args_of(d).iter().any(|a| names.contains(&cx.text(*a)))
                            && (is_log_like(d, cx) || cx.fm.select_method(d).is_none_or(|m| m.method.ret == Ty::Void)))
                })
        };
        let mut handed_off = false;
        let mut cur = n;
        while let Some(block) = cur.parent() {
            if block.kind() == "block" {
                handed_off |= block.kids().iter().take_while(|k| k.start_byte() < cur.start_byte()).any(|k| hands_off(*k));
            }
            if block == body || handed_off {
                break;
            }
            cur = block;
        }
        if handed_off {
            return;
        }
        // initCause / addSuppressed elsewhere in the catch block
        let body_text = cx.text(body);
        if body_text.contains("initCause(") || body_text.contains("addSuppressed(") || body_text.contains("setStackTrace(") {
            return;
        }
        // exception assigned to a variable that later gets the cause
        if catch_types(c, cx).iter().any(|t| matches!(t.class_name(), Some("java.lang.NumberFormatException" | "java.lang.InterruptedException"))) {
            return;
        }
        cx.report(e, format!("The caught exception `{v}` is not passed as the cause; its stack trace is lost"));
    }
}

declare_rule!(AbruptFinally {
    id: "abrupt-finally",
    name: "return/throw/break in finally",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S1143", "pmd:ReturnFromFinallyBlock", "pmd:DoNotThrowExceptionInFinally", "errorprone:Finally"],
    kinds: ["finally_clause"],
    description: "Leaving a `finally` block abruptly (return, throw, break, continue) discards any exception or return value from the try block, silently swallowing errors.",
});

impl Rule for AbruptFinally {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(b) = n.kid_of_kind("block") else { return };
        fn walk<'t>(n: Node<'t>, loops: usize, out: &mut Vec<Node<'t>>) {
            for k in n.kids() {
                match k.kind() {
                    "return_statement" | "throw_statement" => out.push(k),
                    "break_statement" | "continue_statement" => {
                        let labeled = k.kids().iter().any(|x| x.kind() == "identifier");
                        if loops == 0 || labeled {
                            out.push(k);
                        }
                    }
                    "lambda_expression" | "class_body" | "class_declaration" => {}
                    "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement" | "switch_expression" => walk(k, loops + 1, out),
                    "try_statement" | "try_with_resources_statement" => {
                        // a throw inside a nested try that catches it is fine
                        for kk in k.kids() {
                            if kk.kind() == "block" && !k.kids_of_kind("catch_clause").is_empty() {
                                continue;
                            }
                            walk(kk, loops, out);
                        }
                    }
                    _ => walk(k, loops, out),
                }
            }
        }
        let mut out = Vec::new();
        walk(b, 0, &mut out);
        for s in out {
            let kw = s.kind().trim_end_matches("_statement");
            cx.report(s, format!("`{kw}` inside finally discards exceptions and return values from the try block"));
        }
    }
}

declare_rule!(InterruptedSwallowed {
    id: "interrupted-exception-swallowed",
    name: "InterruptedException swallowed",
    category: Concurrency,
    severity: Warning,
    sources: ["sonar:S2142"],
    kinds: ["catch_clause"],
    description: "Catching `InterruptedException` clears the thread's interrupt flag. Unless you rethrow it, restore the flag with `Thread.currentThread().interrupt()` so callers can react to the cancellation.",
});

impl Rule for InterruptedSwallowed {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let types = catch_types(n, cx);
        if !types.iter().any(|t| t.is_class("java.lang.InterruptedException")) {
            return;
        }
        let Some(body) = n.field("body") else { return };
        let text = cx.text(body);
        if text.contains("interrupt()") || text.contains("throw ") || text.contains("System.exit") {
            return;
        }
        // helpers that always throw: JUnit fail(e), Guava propagate, rethrow utilities
        let rethrows = body.kids().iter().any(|st| {
            st.kind() == "expression_statement"
                && st.first_kid().is_some_and(|d| d.kind() == "method_invocation" && matches!(cx.fm.call_name(d), Some("fail" | "propagate" | "rethrow" | "sneakyThrow" | "throwException" | "throwUnchecked" | "rethrowUnchecked")))
        });
        if rethrows {
            return;
        }
        if let Some(p) = catch_param(n) {
            cx.report(p, "InterruptedException is swallowed; call `Thread.currentThread().interrupt()` or rethrow");
        }
    }
}

declare_rule!(ExceptionNotThrown {
    id: "exception-not-thrown",
    name: "Exception created but not thrown",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S3984", "spotbugs:RV_EXCEPTION_NOT_THROWN", "errorprone:DeadException"],
    kinds: ["expression_statement"],
    description: "`new IllegalArgumentException(\"...\");` without `throw` does nothing; the intended check silently passes.",
});

impl Rule for ExceptionNotThrown {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = n.first_kid() else { return };
        if e.kind() != "object_creation_expression" || e.kid_of_kind("class_body").is_some() {
            return;
        }
        // arm of a switch expression / lambda body: the value is used
        if n.parent().is_some_and(|p| matches!(p.kind(), "switch_rule" | "lambda_expression")) {
            return;
        }
        let t = cx.type_of(e);
        if is_subtype_any(cx.fm, &t, &["java.lang.Throwable"]) {
            cx.report_fix(e, format!("`{}` is created but never thrown", t.display_simple()), Fix::replace("Throw it", e, format!("throw {}", cx.text(e))));
        }
    }
}

declare_rule!(RethrowOnly {
    id: "useless-catch-rethrow",
    name: "Catch block only rethrows",
    category: Style,
    severity: Info,
    sources: ["sonar:S2737", "pmd:AvoidRethrowingException"],
    kinds: ["catch_clause"],
    description: "A catch block whose only statement is `throw e;` adds nothing; remove it (unless it prevents a broader catch clause below from handling the exception).",
});

impl Rule for RethrowOnly {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(body) = n.field("body") else { return };
        let kids = body.kids();
        if kids.len() != 1 || kids[0].kind() != "throw_statement" {
            return;
        }
        let Some(v) = catch_var_name(n, cx.src) else { return };
        if !kids[0].first_kid().is_some_and(|e| cx.text(e) == v) {
            return;
        }
        // a later catch clause might catch a supertype: then this one is deliberate
        let Some(tr) = n.parent() else { return };
        let clauses = tr.kids_of_kind("catch_clause");
        let pos = clauses.iter().position(|c| *c == n).unwrap_or(0);
        if pos + 1 < clauses.len() {
            return;
        }
        if tr.kid_of_kind("finally_clause").is_some() || tr.kind() == "try_with_resources_statement" {
            // removing the catch still keeps the finally; fine to report
        }
        cx.report(n.child(0).unwrap_or(n), "Catch block only rethrows the exception; remove it");
    }
}

/// A logger call or a logging helper (`Tracer.logError(msg, e)`, `logException(e)`).
fn is_log_like<'t>(n: Node<'t>, cx: &Ctx<'_, 't>) -> bool {
    is_log_call(n, cx) || (n.kind() == "method_invocation" && cx.fm.call_name(n).and_then(|m| m.strip_prefix("log")).is_some_and(|r| r.starts_with(|c: char| c.is_ascii_uppercase())))
}

fn is_log_call<'t>(n: Node<'t>, cx: &Ctx<'_, 't>) -> bool {
    if n.kind() != "method_invocation" {
        return false;
    }
    let Some(name) = cx.fm.call_name(n) else { return false };
    if !matches!(name, "error" | "warn" | "info" | "debug" | "trace" | "log" | "severe" | "warning") {
        return false;
    }
    let Some(o) = n.field("object") else { return false };
    let t = cx.type_of(o);
    t.class_name().is_some_and(|c| c.contains("Logger") || c.ends_with(".Log")) || matches!(cx.text(o), "log" | "logger" | "LOG" | "LOGGER")
}

/// `x.error(..)`/`warn`/`info`/`debug`/`trace` on a receiver of unknown type,
/// typically a logger held elsewhere whose library is not on the classpath
/// (`Loggers.RAFT.error("..", t)`): a message comes first and the throwable
/// `v` later. A call on a known type (`Flow.error(e)`), a static call on a
/// class from a missing jar (`Results.error("..", e)`) or a sink taking just the
/// throwable (`sink.error(e)`) is not one.
fn is_log_level_call_on_unknown<'t>(n: Node<'t>, v: &str, cx: &Ctx<'_, 't>) -> bool {
    if !matches!(cx.fm.call_name(n), Some("error" | "warn" | "info" | "debug" | "trace")) {
        return false;
    }
    let args = cx.fm.args_of(n);
    if args.len() < 2 || cx.text(args[0]) == v {
        return false;
    }
    let Some(o) = n.field("object") else { return false };
    // a bare or qualified class name (`Results`, `org.lib.Results`), not a constant (`Loggers.RAFT`)
    let last = match o.kind() {
        "identifier" => Some(o),
        "field_access" => o.field("field"),
        _ => None,
    };
    let class_like = last.is_some_and(|l| cx.text(l).starts_with(|c: char| c.is_ascii_uppercase()) && cx.text(l).chars().any(|c| c.is_ascii_lowercase()));
    !class_like && cx.type_of(o).is_unknown() && cx.fm.resolve_call(n).is_none()
}

declare_rule!(LogAndRethrow {
    id: "log-and-rethrow",
    name: "Exception logged and rethrown",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S2139"],
    kinds: ["catch_clause"],
    description: "Logging an exception and then rethrowing it produces duplicate log entries as each layer does the same. Either handle (and log) it, or rethrow it with context, not both.",
});

impl Rule for LogAndRethrow {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(body) = n.field("body") else { return };
        let Some(v) = catch_var_name(n, cx.src) else { return };
        let kids = body.kids();
        let logs = kids.iter().any(|s| {
            s.kind() == "expression_statement" && s.first_kid().is_some_and(|e| is_log_call(e, cx) && cx.fm.args_of(e).iter().any(|a| cx.text(*a) == v))
        });
        let rethrows = kids.iter().any(|s| s.kind() == "throw_statement" && s.first_kid().is_some_and(|e| cx.text(e) == v));
        if logs && rethrows {
            cx.report(n.child(0).unwrap_or(n), "Exception is both logged and rethrown; do one or the other");
        }
    }
}

declare_rule!(PrintStackTrace {
    id: "print-stack-trace",
    name: "printStackTrace() call",
    category: BadPractice,
    severity: Warning,
    scope: Main,
    sources: ["sonar:S1148", "sonar:S4507", "pmd:AvoidPrintStackTrace", "intellij:CallToPrintStackTrace"],
    kinds: ["method_invocation"],
    description: "`printStackTrace()` writes to stderr, bypassing the logging framework (no levels, no correlation, often lost in production). Log the exception instead.",
});

impl Rule for PrintStackTrace {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("printStackTrace") || !cx.fm.args_of(n).is_empty() {
            return;
        }
        if let Some(o) = n.field("object") {
            // only Throwable declares printStackTrace() in practice, so an unknown
            // receiver (library jar missing) counts; a known non-Throwable does not
            if is_throwable_expr(cx.fm, o) || (cx.type_of(o).is_unknown() && cx.fm.resolve_call(n).is_none()) {
                cx.report(n, "Use a logger instead of `printStackTrace()`");
            }
        }
    }
}

declare_rule!(CatchInstanceof {
    id: "catch-instanceof-dispatch",
    name: "instanceof on a caught exception",
    category: Style,
    severity: Info,
    sources: ["sonar:S1193"],
    kinds: ["catch_clause"],
    description: "Dispatching on the exception type with `instanceof` inside one catch block is what multiple catch clauses are for; separate clauses are clearer and checked by the compiler.",
});

impl Rule for CatchInstanceof {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(body) = n.field("body") else { return };
        let Some(v) = catch_var_name(n, cx.src) else { return };
        let mut ios = Vec::new();
        find_kind(body, "instanceof_expression", &mut ios);
        for io in ios {
            if io.field("left").is_some_and(|l| cx.text(l) == v) {
                let rt = io.field("right").map(|r| cx.fm.resolve_type_node(r)).unwrap_or(Ty::Unknown);
                if is_subtype_any(cx.fm, &rt, &["java.lang.Throwable"]) {
                    cx.report(io, "Use a separate catch clause instead of `instanceof` on the caught exception");
                }
            }
        }
    }
}

declare_rule!(FinalizeUsage {
    id: "finalize-usage",
    name: "finalize() overridden or called",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S1113", "sonar:S1111", "pmd:AvoidCallingFinalize", "spotbugs:FI_EXPLICIT_INVOCATION"],
    kinds: ["method_declaration", "method_invocation"],
    description: "Finalization is deprecated for removal: finalizers run late or never, slow down GC and resurrect objects. Use try-with-resources or `java.lang.ref.Cleaner`.",
});

impl Rule for FinalizeUsage {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.kind() == "method_declaration" {
            if method_name(n, cx.src) == "finalize" && param_count(n) == 0 {
                if let Some(name) = n.field("name") {
                    cx.report(name, "Overriding `finalize()` is deprecated; use try-with-resources or a Cleaner");
                }
            }
        } else if cx.fm.call_name(n) == Some("finalize") && cx.fm.args_of(n).is_empty() {
            if n.field("object").is_some_and(|o| o.kind() == "super") {
                return;
            }
            cx.report(n, "Never call `finalize()` explicitly");
        }
    }
}

fn iterator_class<'t>(m: Node<'t>, cx: &Ctx<'_, 't>) -> bool {
    owning_type_decl(m).and_then(|d| cx.fm.class_of_decl(d)).is_some_and(|c| cx.fm.is_subclass(&c.name, "java.util.Iterator").is_yes())
}

declare_rule!(IteratorNextWithoutNoSuchElement {
    id: "iterator-next-no-such-element",
    name: "Iterator.next() never throws NoSuchElementException",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2272", "spotbugs:IT_NO_SUCH_ELEMENT"],
    kinds: ["method_declaration"],
    description: "The `Iterator` contract requires `next()` to throw `NoSuchElementException` when exhausted. Returning null or garbage breaks callers that rely on the exception.",
});

impl Rule for IteratorNextWithoutNoSuchElement {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if method_name(n, cx.src) != "next" || param_count(n) != 0 || !iterator_class(n, cx) {
            return;
        }
        let Some(body) = n.field("body") else { return };
        let text = cx.text(body);
        // delegation to another iterator/next(), or any throw, is fine
        if ["throw ", "next(", "hasNext", ".get(", ".remove", "poll(", "pop(", "Element("].iter().any(|k| text.contains(k)) {
            return;
        }
        if let Some(name) = n.field("name") {
            cx.report(name, "`next()` should throw NoSuchElementException when the iteration has no more elements");
        }
    }
}

declare_rule!(HasNextCallsNext {
    id: "hasnext-calls-next",
    name: "Iterator.hasNext() calls next()",
    category: Bug,
    severity: Error,
    sources: ["sonar:S1849", "spotbugs:DMI_CALLING_NEXT_FROM_HASNEXT"],
    kinds: ["method_declaration"],
    description: "`hasNext()` must not change iterator state, but calling `next()` advances it, so elements get skipped.",
});

impl Rule for HasNextCallsNext {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if method_name(n, cx.src) != "hasNext" || param_count(n) != 0 || !iterator_class(n, cx) {
            return;
        }
        let Some(body) = n.field("body") else { return };
        let mut calls = Vec::new();
        find_kind(body, "method_invocation", &mut calls);
        for c in calls {
            if cx.fm.call_name(c) == Some("next") && cx.fm.args_of(c).is_empty() && c.field("object").is_none_or(|o| o.kind() == "this") {
                cx.report(c, "`hasNext()` calls this iterator's `next()`, advancing it");
            }
        }
    }
}

declare_rule!(ThrowsInFinallyClose {
    id: "close-in-finally-unguarded",
    name: "close() in finally may mask the original exception",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S2093", "pmd:UseTryWithResources"],
    kinds: ["finally_clause"],
    description: "Closing resources manually in `finally` is verbose and an exception from `close()` replaces the original one. Use try-with-resources, which suppresses close failures correctly.",
});

impl Rule for ThrowsInFinallyClose {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(b) = n.kid_of_kind("block") else { return };
        let mut calls = Vec::new();
        find_kind(b, "method_invocation", &mut calls);
        for c in calls {
            if cx.fm.call_name(c) != Some("close") || !cx.fm.args_of(c).is_empty() {
                continue;
            }
            let Some(o) = c.field("object") else { continue };
            // only locals declared in the enclosing method (resources owned here)
            if let Some(i) = cx.fm.local_ref(o.unparen()) {
                let t = cx.fm.local_type(i);
                if is_subtype_any(cx.fm, &t, &["java.lang.AutoCloseable"]) {
                    // already inside a nested try in finally?
                    if c.ancestors().take_while(|a| *a != n).any(|a| a.kind() == "try_statement") {
                        continue;
                    }
                    cx.report(c, "Use try-with-resources instead of closing in finally");
                    return;
                }
            }
        }
    }
}

declare_rule!(ExceptionIgnoredMessage {
    id: "exception-message-only-logged",
    name: "Only the exception message is logged",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S1166", "intellij:ThrowableNotPassedToLogger"],
    kinds: ["method_invocation"],
    description: "Logging `e.getMessage()` loses the exception type, cause chain and stack trace (and the message may be null). Pass the exception itself as the last logger argument.",
});

impl Rule for ExceptionIgnoredMessage {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !is_log_call(n, cx) {
            return;
        }
        if !matches!(cx.fm.call_name(n), Some("error" | "warn")) {
            return;
        }
        let Some(c) = n.ancestors().take_while(|a| !matches!(a.kind(), "lambda_expression" | "class_body" | "method_declaration")).find(|a| a.kind() == "catch_clause") else { return };
        let Some(v) = catch_var_name(c, cx.src) else { return };
        let args = cx.fm.args_of(n);
        let passes_exception = args.iter().any(|a| cx.text(*a) == v);
        let uses_message = args.iter().any(|a| {
            let t = cx.text(*a);
            t.contains(&format!("{v}.getMessage()")) || t.contains(&format!("{v}.getLocalizedMessage()"))
        });
        if uses_message && !passes_exception {
            cx.report(n, format!("Only `{v}.getMessage()` is logged; pass `{v}` to keep the stack trace"));
        }
    }
}

declare_rule!(ThrowNull {
    id: "throw-null",
    name: "throw null",
    category: Bug,
    severity: Error,
    sources: ["sonar:S1696", "errorprone:ThrowNull"],
    kinds: ["throw_statement"],
    description: "`throw null` throws a NullPointerException with no message; throw a meaningful exception instead.",
});

impl Rule for ThrowNull {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.first_kid().is_some_and(is_null_literal) {
            cx.report(n, "`throw null` throws an anonymous NullPointerException");
        }
    }
}

declare_rule!(CatchAndReturnNull {
    id: "catch-return-null",
    name: "Exception converted to a null return",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S1166"],
    kinds: ["catch_clause"],
    description: "Swallowing an exception and returning null pushes the failure to a later NullPointerException far from the cause. Rethrow, return an empty result/Optional, or at least log.",
});

impl Rule for CatchAndReturnNull {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(body) = n.field("body") else { return };
        let kids = body.kids();
        if kids.len() == 1 && kids[0].kind() == "return_statement" && kids[0].first_kid().is_some_and(is_null_literal) && !block_has_comment(body) {
            cx.report(kids[0], "Exception swallowed and turned into `return null`");
        }
    }
}
