//! Threads, locks and shared mutable state.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_semantic::MethodMatcher;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(ThreadRunDirect),
        Box::new(WaitNotInLoop),
        Box::new(NotifyInsteadOfNotifyAll),
        Box::new(WaitNotifyWithoutLock),
        Box::new(SyncOnValue),
        Box::new(SyncOnGetClass),
        Box::new(SyncOnNonFinalField),
        Box::new(DoubleCheckedLocking),
        Box::new(StaticDateFormat),
        Box::new(SleepWhileLocked),
        Box::new(StartThreadInConstructor),
        Box::new(ConditionWait),
        Box::new(LockWithoutUnlock),
        Box::new(ConcurrentMapCheckThenAct),
        Box::new(VolatileIncrement),
        Box::new(SyncOnLockObject),
        Box::new(LegacyCollections),
        Box::new(VolatileArray),
        Box::new(ThreadLocalNotRemoved),
        Box::new(ExecutorNeverShutdown),
        Box::new(ThreadGroupUsage),
        Box::new(SynchronizedOnLocal),
    ]
}

const THREAD_RUN: MethodMatcher = MethodMatcher::instance("java.lang.Thread").named(&["run"]).arity(0);

declare_rule!(ThreadRunDirect {
    id: "thread-run-called-directly",
    name: "Thread.run() called instead of start()",
    category: Concurrency,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S1217", "spotbugs:RU_INVOKE_RUN", "pmd:DontCallThreadRun"],
    kinds: ["method_invocation"],
    description: "`thread.run()` executes the code synchronously in the current thread; `start()` is what spawns a new thread.",
});

impl Rule for ThreadRunDirect {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !cx.fm.call_matches(n, &THREAD_RUN) {
            return;
        }
        let Some(o) = n.field("object") else { return };
        if matches!(o.kind(), "super" | "this") {
            return;
        }
        if is_subtype_any(cx.fm, &cx.type_of(o), &["java.lang.Thread"]) {
            cx.report_fix(n, "`run()` executes synchronously; call `start()` to run in a new thread", Fix::replace("Call start()", n.field("name").unwrap(), "start"));
        }
    }
}

const OBJECT_WAIT: MethodMatcher = MethodMatcher::exact("java.lang.Object").named(&["wait"]);
const OBJECT_NOTIFY: MethodMatcher = MethodMatcher::exact("java.lang.Object").named(&["notify", "notifyAll"]);

declare_rule!(WaitNotInLoop {
    id: "wait-not-in-loop",
    name: "wait() not called in a loop",
    category: Concurrency,
    severity: Warning,
    sources: ["sonar:S2274", "spotbugs:WA_NOT_IN_LOOP"],
    kinds: ["method_invocation"],
    description: "Threads can wake up spuriously or after the condition changed again; `wait()` must be called in a `while (!condition)` loop.",
});

impl Rule for WaitNotInLoop {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_matches(n, &OBJECT_WAIT) && !in_loop(n) {
            cx.report(n, "`wait()` should be called inside a loop that re-checks the condition");
        }
    }
}

declare_rule!(NotifyInsteadOfNotifyAll {
    id: "notify-instead-of-notifyall",
    name: "notify() instead of notifyAll()",
    category: Concurrency,
    severity: Info,
    sources: ["sonar:S2446", "spotbugs:NO_NOTIFY_NOT_NOTIFYALL", "pmd:UseNotifyAllInsteadOfNotify"],
    kinds: ["method_invocation"],
    description: "`notify()` wakes an arbitrary waiting thread, which may be one waiting for a different condition, causing missed signals. `notifyAll()` is safe.",
});

impl Rule for NotifyInsteadOfNotifyAll {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) == Some("notify") && cx.fm.call_matches(n, &OBJECT_NOTIFY) {
            cx.report_fix(n, "Prefer `notifyAll()`; `notify()` may wake the wrong thread", Fix::replace("Use notifyAll", n.field("name").unwrap(), "notifyAll"));
        }
    }
}

declare_rule!(WaitNotifyWithoutLock {
    id: "wait-notify-without-lock",
    name: "wait()/notify() outside synchronized",
    category: Concurrency,
    severity: Error,
    sources: ["sonar:S2273", "spotbugs:MWN_MISMATCHED_WAIT", "spotbugs:MWN_MISMATCHED_NOTIFY"],
    kinds: ["method_invocation"],
    description: "Calling `wait`, `notify` or `notifyAll` without holding the object's monitor throws `IllegalMonitorStateException`.",
});

impl Rule for WaitNotifyWithoutLock {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !(cx.fm.call_matches(n, &OBJECT_WAIT) || cx.fm.call_matches(n, &OBJECT_NOTIFY)) {
            return;
        }
        if in_synchronized(n) {
            return;
        }
        // helpers are commonly called with the caller's lock held: only flag a
        // public method (an entry point) that no synchronized code in the
        // class calls, outside lambdas (which may run under a lock)
        if n.ancestors().take_while(|a| !matches!(a.kind(), "method_declaration" | "constructor_declaration")).any(|a| a.kind() == "lambda_expression") {
            return;
        }
        let Some(m) = enclosing_method_decl(n) else { return };
        if !m.has_modifier("public") {
            return;
        }
        let name = method_name(m, cx.src);
        let called_under_lock = owning_type_decl(m).is_some_and(|cls| {
            cls.descendants().any(|d| d.kind() == "method_invocation" && d.field("name").is_some_and(|x| cx.text(x) == name) && in_synchronized(d))
        });
        if called_under_lock {
            return;
        }
        cx.report(n, format!("`{}()` called without holding the monitor; throws IllegalMonitorStateException", cx.fm.call_name(n).unwrap_or("")));
    }
}

fn lock_expr<'t>(n: Node<'t>) -> Option<Node<'t>> {
    n.kid_of_kind("parenthesized_expression").and_then(|p| p.first_kid()).map(|e| e.unparen())
}

declare_rule!(SyncOnValue {
    id: "synchronized-on-value-object",
    name: "Synchronization on a String literal or boxed value",
    category: Concurrency,
    severity: Warning,
    sources: ["sonar:S1860", "spotbugs:DL_SYNCHRONIZATION_ON_BOXED_PRIMITIVE", "spotbugs:DL_SYNCHRONIZATION_ON_SHARED_CONSTANT", "spotbugs:DL_SYNCHRONIZATION_ON_BOOLEAN"],
    kinds: ["synchronized_statement"],
    description: "Interned strings and cached boxed values are shared JVM-wide, so unrelated code may lock the same object, causing contention or deadlocks. Lock on a private `final Object`.",
});

impl Rule for SyncOnValue {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = lock_expr(n) else { return };
        let t = cx.type_of(e);
        if e.kind() == "string_literal" || t.is_boxed() || (t.is_string() && cx.fm.constant_value(e).is_some()) {
            cx.report(e, format!("Locking on a shared `{}` value; use a dedicated private lock object", t.display_simple()));
        }
    }
}

declare_rule!(SyncOnGetClass {
    id: "synchronized-on-getclass",
    name: "Synchronization on getClass()",
    category: Concurrency,
    severity: Warning,
    sources: ["sonar:S3067", "spotbugs:WL_USING_GETCLASS_RATHER_THAN_CLASS_LITERAL"],
    kinds: ["synchronized_statement"],
    description: "`synchronized (getClass())` locks a different object in each subclass, so instances of different subclasses do not exclude each other. Use the class literal `MyClass.class`.",
});

impl Rule for SyncOnGetClass {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = lock_expr(n) else { return };
        if is_call_named(e, cx.src, &["getClass"]) && e.field("object").is_none_or(|o| o.kind() == "this") {
            cx.report(e, "Lock on the class literal instead of `getClass()`");
        }
    }
}

declare_rule!(SyncOnNonFinalField {
    id: "synchronized-on-non-final-field",
    name: "Synchronization on a non-final field",
    category: Concurrency,
    severity: Warning,
    sources: ["sonar:S2445", "spotbugs:ML_SYNC_ON_FIELD_TO_GUARD_CHANGING_THAT_FIELD"],
    kinds: ["synchronized_statement"],
    description: "If the field is reassigned, threads lock different objects and the critical section is no longer exclusive. Make the lock field `final`.",
});

impl Rule for SyncOnNonFinalField {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = lock_expr(n) else { return };
        if let Some(f) = cx.fm.field_ref(e) {
            if !f.field.flags.is_final() && f.field.range.is_some() {
                cx.report(e, format!("Locking on non-final field `{}`; it may be reassigned", f.field.name));
            }
        }
    }
}

declare_rule!(DoubleCheckedLocking {
    id: "double-checked-locking",
    name: "Double-checked locking without volatile",
    category: Concurrency,
    severity: Error,
    sources: ["sonar:S2168", "spotbugs:DC_DOUBLECHECK", "pmd:DoubleCheckedLocking"],
    kinds: ["if_statement"],
    description: "Without `volatile`, another thread can see the field non-null before the object's constructor writes are visible, and use a half-initialized object. Declare the field `volatile` or use a holder class.",
});

impl Rule for DoubleCheckedLocking {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        // if (f == null) { synchronized (...) { if (f == null) { f = ...; } } }
        let Some(cond) = n.field("condition").and_then(|c| c.first_kid()) else { return };
        let Some(field) = null_checked_field(cond, cx) else { return };
        let Some(cons) = n.field("consequence") else { return };
        let sync = if cons.kind() == "synchronized_statement" { Some(cons) } else { cons.kids().into_iter().find(|k| k.kind() == "synchronized_statement") };
        let Some(sync) = sync else { return };
        let Some(body) = sync.field("body") else { return };
        let inner = body.kids().into_iter().find(|k| k.kind() == "if_statement");
        let Some(inner) = inner else { return };
        let Some(icond) = inner.field("condition").and_then(|c| c.first_kid()) else { return };
        if null_checked_field(icond, cx).as_ref().map(|f| &f.name) != Some(&field.name) {
            return;
        }
        if !field.flags.has(Flags::VOLATILE) && !field.flags.is_final() {
            cx.report(n.child(0).unwrap_or(n), format!("Double-checked locking on non-volatile field `{}`", field.name));
        }
    }
}

fn null_checked_field<'t>(c: Node<'t>, cx: &Ctx<'_, 't>) -> Option<FieldInfo> {
    let c = c.unparen();
    if c.kind() != "binary_expression" || op(c) != "==" {
        return None;
    }
    let (l, r) = lr(c)?;
    let v = if is_null_literal(r) { l } else if is_null_literal(l) { r } else { return None };
    cx.fm.field_ref(v.unparen()).map(|f| f.field)
}

declare_rule!(StaticDateFormat {
    id: "static-non-thread-safe-field",
    name: "Static field of a non-thread-safe type",
    category: Concurrency,
    severity: Warning,
    sources: ["sonar:S2885", "spotbugs:STCAL_STATIC_SIMPLE_DATE_FORMAT_INSTANCE", "spotbugs:STCAL_STATIC_CALENDAR_INSTANCE", "pmd:UnsynchronizedStaticFormatter"],
    kinds: ["field_declaration"],
    description: "`SimpleDateFormat`, `DateFormat`, `Calendar`, `MessageFormat` and `DecimalFormat` keep internal state; sharing one instance across threads produces garbage results. Use `DateTimeFormatter` or a ThreadLocal.",
});

impl Rule for StaticDateFormat {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !n.has_modifier("static") {
            return;
        }
        let Some(t) = n.field("type") else { return };
        let ty = cx.fm.resolve_type_node(t);
        let bad = ["java.text.DateFormat", "java.util.Calendar", "java.text.MessageFormat", "java.text.NumberFormat"];
        if is_subtype_any(cx.fm, &ty, &bad) {
            cx.report(t, format!("`{}` is not thread-safe; a static instance is shared across threads", ty.display_simple()));
        }
    }
}

const THREAD_SLEEP: MethodMatcher = MethodMatcher::statics("java.lang.Thread").named(&["sleep"]);

declare_rule!(SleepWhileLocked {
    id: "sleep-while-holding-lock",
    name: "Thread.sleep() while holding a lock",
    category: Concurrency,
    severity: Warning,
    sources: ["sonar:S2276", "spotbugs:SWL_SLEEP_WITH_LOCK_HELD"],
    kinds: ["method_invocation"],
    description: "Sleeping inside `synchronized` keeps the monitor locked, blocking every other thread that needs it. Use `wait(timeout)` (which releases the lock) or move the sleep out.",
});

impl Rule for SleepWhileLocked {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_matches(n, &THREAD_SLEEP) && in_synchronized(n) {
            cx.report(n, "`Thread.sleep()` while holding a monitor blocks other threads; use `wait(timeout)`");
        }
    }
}

declare_rule!(StartThreadInConstructor {
    id: "thread-started-in-constructor",
    name: "Thread started in a constructor",
    category: Concurrency,
    severity: Warning,
    scope: Main,
    sources: ["spotbugs:SC_START_IN_CTOR", "sonar:S2693"],
    kinds: ["method_invocation"],
    description: "Starting a thread in a constructor lets it observe `this` before construction (and subclass constructors) completes. Start it from a separate `start()`/init method.",
});

impl Rule for StartThreadInConstructor {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("start") || !cx.fm.args_of(n).is_empty() {
            return;
        }
        if !enclosing_method_decl(n).is_some_and(|m| m.kind() == "constructor_declaration") || n.ancestors().any(|a| a.kind() == "lambda_expression") {
            return;
        }
        let Some(o) = n.field("object") else { return };
        let class_final = enclosing_method_decl(n).and_then(owning_type_decl).is_some_and(|d| is_final_class(cx.fm, d));
        if class_final {
            return;
        }
        if is_subtype_any(cx.fm, &cx.type_of(o), &["java.lang.Thread"]) {
            cx.report(n, "Thread started in a constructor can see a partially constructed object");
        }
    }
}

declare_rule!(ConditionWait {
    id: "condition-wait-instead-of-await",
    name: "wait() called on a Condition",
    category: Concurrency,
    severity: Error,
    tier: Edit,
    sources: ["spotbugs:DM_MONITOR_WAIT_ON_CONDITION", "errorprone:WaitNotInLoop"],
    kinds: ["method_invocation"],
    description: "`Condition.wait()` is `Object.wait()` on the Condition object itself, not `await()`; it throws IllegalMonitorStateException or waits on the wrong monitor.",
});

impl Rule for ConditionWait {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(cx.fm.call_name(n), Some("wait" | "notify" | "notifyAll")) {
            return;
        }
        if let Some(o) = n.field("object") {
            if is_subtype_any(cx.fm, &cx.type_of(o), &["java.util.concurrent.locks.Condition"]) {
                let fix_name = if cx.fm.call_name(n) == Some("wait") { "await" } else { "signalAll" };
                cx.report_fix(n, format!("Use `{fix_name}()` on a Condition"), Fix::replace(format!("Use {fix_name}"), n.field("name").unwrap(), fix_name));
            }
        }
    }
}

const LOCK_LOCK: MethodMatcher = MethodMatcher::instance("java.util.concurrent.locks.Lock").named(&["lock", "lockInterruptibly"]);

declare_rule!(LockWithoutUnlock {
    id: "lock-without-unlock-in-finally",
    name: "Lock acquired without unlock() in finally",
    category: Concurrency,
    severity: Warning,
    sources: ["sonar:S2222", "spotbugs:UL_UNRELEASED_LOCK", "spotbugs:UL_UNRELEASED_LOCK_EXCEPTION_PATH"],
    kinds: ["method_invocation"],
    description: "If code between `lock()` and `unlock()` throws, the lock is never released and every other thread blocks forever. Release it in a `finally` block.",
});

impl Rule for LockWithoutUnlock {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !cx.fm.call_matches(n, &LOCK_LOCK) {
            return;
        }
        let Some(o) = n.field("object") else { return };
        // `this.lock` and `lock` are the same field
        let key = |x: Node<'t>| {
            let t: String = cx.text(x).chars().filter(|c| !c.is_whitespace()).collect();
            t.strip_prefix("this.").map(str::to_string).unwrap_or(t)
        };
        let lock_text = key(o);
        let Some(m) = cx.owning_callable(n) else { return };
        // unlock in any finally block of the method, or in a catch that rethrows
        // (the lock is deliberately kept on success: check-then-hold)
        let mut finals = Vec::new();
        find_kind(m, "finally_clause", &mut finals);
        let mut catches = Vec::new();
        find_kind(m, "catch_clause", &mut catches);
        finals.extend(catches.into_iter().filter(|c| c.field("body").is_some_and(|b| b.kids().last().is_some_and(|l| l.kind() == "throw_statement"))));
        let released = finals.iter().any(|f| {
            let mut calls = Vec::new();
            find_kind(*f, "method_invocation", &mut calls);
            calls.iter().any(|c| cx.fm.call_name(*c) == Some("unlock") && c.field("object").is_some_and(|x| key(x) == lock_text))
        });
        if released {
            return;
        }
        // lock()/unlock() split across methods (e.g. acquire()/release() helpers) is deliberate
        let mut calls = Vec::new();
        find_kind(m, "method_invocation", &mut calls);
        let unlocked_here = calls.iter().any(|c| cx.fm.call_name(*c) == Some("unlock"));
        if !unlocked_here {
            return;
        }
        cx.report(n, "Lock is not released in a finally block; an exception leaves it locked");
    }
}

declare_rule!(ConcurrentMapCheckThenAct {
    id: "concurrent-map-check-then-act",
    name: "Non-atomic check-then-act on a concurrent map",
    category: Concurrency,
    severity: Warning,
    sources: ["spotbugs:AT_OPERATION_SEQUENCE_ON_CONCURRENT_ABSTRACTION", "sonar:S3824"],
    kinds: ["if_statement"],
    description: "`if (!map.containsKey(k)) map.put(k, v)` on a ConcurrentMap is a race: two threads can both see the key missing. Use `putIfAbsent`/`computeIfAbsent`.",
});

impl Rule for ConcurrentMapCheckThenAct {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = n.field("condition").and_then(|c| c.first_kid()).map(|c| c.unparen()) else { return };
        let call = if c.kind() == "unary_expression" && op(c) == "!" { c.field("operand").map(|o| o.unparen()) } else { Some(c) };
        let Some(call) = call else { return };
        if !matches!(cx.fm.call_name(call), Some("containsKey" | "get")) {
            return;
        }
        let Some(map) = call.field("object") else { return };
        if !is_subtype_any(cx.fm, &cx.type_of(map), &["java.util.concurrent.ConcurrentMap"]) {
            return;
        }
        let Some(cons) = n.field("consequence") else { return };
        let mut calls = Vec::new();
        find_kind(cons, "method_invocation", &mut calls);
        if cons.kind() == "expression_statement" {
            if let Some(e) = cons.first_kid() {
                calls.push(e);
            }
        }
        let mt = normalized(map, cx.src);
        if calls.iter().any(|x| cx.fm.call_name(*x) == Some("put") && x.field("object").is_some_and(|o| normalized(o, cx.src) == mt)) {
            cx.report(call, "Check-then-put on a ConcurrentMap is not atomic; use `putIfAbsent`/`computeIfAbsent`");
        }
    }
}

declare_rule!(VolatileIncrement {
    id: "volatile-increment",
    name: "Non-atomic increment of a volatile field",
    category: Concurrency,
    severity: Warning,
    sources: ["sonar:S3078", "spotbugs:VO_VOLATILE_INCREMENT"],
    kinds: ["update_expression", "assignment_expression"],
    description: "`volatile` guarantees visibility, not atomicity: `count++` is read-modify-write and concurrent increments get lost. Use `AtomicInteger`/`LongAdder` or synchronize.",
});

impl Rule for VolatileIncrement {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let target = match n.kind() {
            "update_expression" => n.first_kid(),
            _ => {
                if matches!(op(n), "+=" | "-=" | "*=" | "|=" | "&=") {
                    n.field("left")
                } else {
                    None
                }
            }
        };
        let Some(t) = target else { return };
        if let Some(f) = cx.fm.field_ref(t.unparen()) {
            // an explicit lock held around the update (`lock.lock(); try { x++; } finally { lock.unlock(); }`)
            let under_lock = cx.owning_callable(n).is_some_and(|m| {
                let mut calls = Vec::new();
                find_kind(m, "method_invocation", &mut calls);
                let last_before = calls.iter().filter(|c| c.start_byte() < n.start_byte() && matches!(cx.fm.call_name(**c), Some("lock" | "unlock"))).max_by_key(|c| c.start_byte());
                // an unconditional `lock();` statement of a block enclosing the update
                last_before.is_some_and(|c| {
                    cx.fm.call_name(*c) == Some("lock")
                        && c.parent().filter(|p| p.kind() == "expression_statement").and_then(|st| st.parent()).is_some_and(|b| b.kind() == "block" && b.byte_range().contains(&n.start_byte()))
                })
                    && calls.iter().any(|c| c.start_byte() > n.start_byte() && cx.fm.call_name(*c) == Some("unlock"))
            });
            if f.field.flags.has(Flags::VOLATILE) && !in_synchronized(n) && !under_lock {
                cx.report(n, format!("`{}` is volatile but this update is not atomic", f.field.name));
            }
        }
    }
}

declare_rule!(SyncOnLockObject {
    id: "synchronized-on-lock",
    name: "synchronized on a java.util.concurrent Lock",
    category: Concurrency,
    severity: Error,
    sources: ["spotbugs:JLM_JSR166_LOCK_MONITORENTER", "spotbugs:JLM_JSR166_UTILCONCURRENT_MONITORENTER", "errorprone:SynchronizeOnNonFinalField"],
    kinds: ["synchronized_statement"],
    description: "`synchronized (lock)` on a `Lock` or concurrent collection uses its object monitor, not the lock itself, so it does not exclude code using `lock()`. Call `lock()`/`unlock()`.",
});

impl Rule for SyncOnLockObject {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = lock_expr(n) else { return };
        let t = cx.type_of(e);
        if is_subtype_any(cx.fm, &t, &["java.util.concurrent.locks.Lock", "java.util.concurrent.locks.ReadWriteLock", "java.util.concurrent.Semaphore", "java.util.concurrent.CountDownLatch"]) {
            cx.report(e, format!("Synchronizing on a `{}` uses its monitor, not the lock; call `lock()`/`unlock()`", t.display_simple()));
        }
    }
}

declare_rule!(LegacyCollections {
    id: "legacy-synchronized-collection",
    name: "Legacy synchronized collection or StringBuffer",
    category: Performance,
    severity: Info,
    sources: ["sonar:S1149", "pmd:ReplaceVectorWithList", "pmd:ReplaceHashtableWithMap", "pmd:AvoidStringBufferField"],
    kinds: ["object_creation_expression"],
    description: "`Vector`, `Hashtable`, `Stack` and `StringBuffer` synchronize every call, which is slow and rarely the right thread-safety model. Use `ArrayList`, `HashMap`/`ConcurrentHashMap`, `ArrayDeque` and `StringBuilder`.",
});

impl Rule for LegacyCollections {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let t = cx.type_of(n);
        let repl = match t.class_name() {
            Some("java.util.Vector") => "ArrayList",
            Some("java.util.Hashtable") => "HashMap or ConcurrentHashMap",
            Some("java.util.Stack") => "ArrayDeque",
            Some("java.lang.StringBuffer") => "StringBuilder",
            _ => return,
        };
        if n.kid_of_kind("class_body").is_some() {
            return;
        }
        cx.report(n, format!("`{}` is synchronized legacy API; use {repl}", t.display_simple()));
    }
}

declare_rule!(VolatileArray {
    id: "volatile-array-or-collection",
    name: "volatile array or mutable collection",
    category: Concurrency,
    severity: Info,
    sources: ["sonar:S3077", "spotbugs:VO_VOLATILE_REFERENCE_TO_ARRAY"],
    kinds: ["field_declaration"],
    description: "`volatile` only makes the reference visible; writes to array elements or collection contents are still unsynchronized. Use `AtomicReferenceArray` or a concurrent collection.",
});

impl Rule for VolatileArray {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !n.has_modifier("volatile") {
            return;
        }
        let Some(t) = n.field("type") else { return };
        let ty = cx.fm.resolve_type_node(t);
        let mutable_coll = matches!(ty.class_name(), Some("java.util.ArrayList" | "java.util.HashMap" | "java.util.HashSet" | "java.util.LinkedList" | "java.util.List" | "java.util.Map" | "java.util.Set"));
        if ty.is_array() || mutable_coll {
            cx.report(t, "`volatile` does not make the elements thread-safe");
        }
    }
}

declare_rule!(ThreadLocalNotRemoved {
    id: "threadlocal-not-removed",
    name: "ThreadLocal set but never removed",
    category: Concurrency,
    severity: Info,
    sources: ["sonar:S5164"],
    kinds: ["field_declaration"],
    description: "In thread pools, ThreadLocal values outlive the request that set them, leaking memory and data between requests. Call `remove()` when done (typically in finally).",
});

impl Rule for ThreadLocalNotRemoved {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(t) = n.field("type") else { return };
        let ty = cx.fm.resolve_type_node(t);
        if !is_subtype_any(cx.fm, &ty, &["java.lang.ThreadLocal"]) || ty.is_class("java.lang.InheritableThreadLocal") && false {
            return;
        }
        let Some(decl) = owning_type_decl(n) else { return };
        for d in n.fields("declarator") {
            let Some(name) = d.field("name").map(|x| x.text(cx.src)) else { continue };
            let text = cx.text(decl);
            let set = text.contains(&format!("{name}.set(")) || text.contains(&format!("{name}.get()"));
            let removed = text.contains(&format!("{name}.remove()"));
            if set && !removed && text.contains(&format!("{name}.set(")) {
                cx.report(d, format!("ThreadLocal `{name}` is set but never removed"));
            }
        }
    }
}

declare_rule!(ExecutorNeverShutdown {
    id: "executor-not-shutdown",
    name: "Local ExecutorService never shut down",
    category: Concurrency,
    severity: Warning,
    sources: ["sonar:S2095", "errorprone:ThreadPriorityCheck"],
    kinds: ["variable_declarator"],
    description: "A thread pool created in a method and never shut down keeps its threads alive, leaking them and preventing JVM exit. Call `shutdown()` (or use try-with-resources on Java 19+).",
});

impl Rule for ExecutorNeverShutdown {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(v) = n.field("value").map(|v| v.unparen()) else { return };
        if v.kind() != "method_invocation" || !cx.text(v).starts_with("Executors.new") {
            return;
        }
        let Some(name) = n.field("name") else { return };
        let Some(i) = cx.fm.binder.decl_index(name) else { return };
        let d = &cx.fm.binder.decls[i];
        if d.kind != grounds_semantic::LocalKind::Local {
            return;
        }
        if n.ancestors().any(|a| a.kind() == "try_with_resources_statement") {
            return;
        }
        let uses = &cx.fm.binder.uses[i];
        let handled = uses.iter().any(|u| {
            let p = u.parent();
            match p {
                Some(p) if p.kind() == "method_invocation" && p.field("object") == Some(*u) => {
                    matches!(cx.fm.call_name(p), Some("shutdown" | "shutdownNow" | "close" | "awaitTermination"))
                }
                // escapes: passed elsewhere, returned or stored
                Some(p) => matches!(p.kind(), "argument_list" | "return_statement" | "assignment_expression" | "variable_declarator"),
                None => false,
            }
        });
        if !handled {
            cx.report(n, "ExecutorService created here is never shut down; its threads leak");
        }
    }
}

declare_rule!(ThreadGroupUsage {
    id: "thread-group-usage",
    name: "ThreadGroup usage",
    category: Concurrency,
    severity: Info,
    sources: ["sonar:S3014"],
    kinds: ["object_creation_expression"],
    description: "`ThreadGroup` is a legacy, mostly non-thread-safe API (many methods are deprecated). Use executors instead.",
});

impl Rule for ThreadGroupUsage {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.type_of(n).is_class("java.lang.ThreadGroup") {
            cx.report(n, "Avoid ThreadGroup; use an ExecutorService");
        }
    }
}

declare_rule!(SynchronizedOnLocal {
    id: "synchronized-on-local",
    name: "Synchronization on a locally created object",
    category: Concurrency,
    severity: Warning,
    sources: ["spotbugs:ML_SYNC_ON_UPDATED_FIELD", "sonar:S2445"],
    kinds: ["synchronized_statement"],
    description: "Locking on an object created in the same method (`synchronized (new Object())` or a fresh local) never excludes other threads; each call has its own lock.",
});

impl Rule for SynchronizedOnLocal {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = lock_expr(n) else { return };
        if e.kind() == "object_creation_expression" {
            cx.report(e, "Locking on a new object never excludes other threads");
            return;
        }
        if let Some(i) = cx.fm.local_ref(e) {
            let d = &cx.fm.binder.decls[i];
            // the object escapes (captured by a lambda/inner class, passed or stored): it may be shared
            let escapes = cx.fm.binder.uses[i].iter().any(|u| {
                u.ancestors().take_while(|a| !matches!(a.kind(), "method_declaration" | "constructor_declaration")).any(|a| matches!(a.kind(), "lambda_expression" | "class_body"))
                    || u.parent().is_some_and(|p| matches!(p.kind(), "argument_list" | "assignment_expression" | "return_statement" | "array_initializer"))
            });
            if escapes {
                return;
            }
            if d.kind == grounds_semantic::LocalKind::Local && d.decl_node.field("value").is_some_and(|v| v.unparen().kind() == "object_creation_expression") {
                cx.report(e, format!("`{}` is created in this method; each call locks a different object", d.name));
            }
        }
    }
}
