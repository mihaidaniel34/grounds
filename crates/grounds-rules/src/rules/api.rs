//! API misuse: ignored results, collections, streams, Optional, System calls,
//! deprecated APIs.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_semantic::MethodMatcher;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(IgnoredReturnValue),
        Box::new(IgnoredFileResult),
        Box::new(IgnoredReadResult),
        Box::new(StreamNotConsumed),
        Box::new(BooleanReturnsNull),
        Box::new(CollectionReturnsNull),
        Box::new(ToStringReturnsNull),
        Box::new(CollectionAddsItself),
        Box::new(ArraysAsListPrimitive),
        Box::new(ToArrayCast),
        Box::new(ModifyCollectionInForeach),
        Box::new(ImmutableCollectionModification),
        Box::new(MapKeySetGet),
        Box::new(StreamAnyMatch),
        Box::new(StreamForEachOnCollection),
        Box::new(CollectorsToList),
        Box::new(SystemExit),
        Box::new(GcCall),
        Box::new(SystemOut),
        Box::new(DeprecatedApiUsage),
        Box::new(SetAccessible),
        Box::new(ContainsKeyThenPut),
        Box::new(RemoveIntOnIntegerList),
        Box::new(OptionalOrElseCall),
        Box::new(OptionalParameter),
        Box::new(CharsetForName),
        Box::new(DateGetTime),
        Box::new(EnumKeyedHashMap),
        Box::new(OptionalIsPresentGet),
        Box::new(StreamSortedFindFirst),
        Box::new(CollectionToArrayZero),
        Box::new(StreamCountComparison),
    ]
}

/// Pure methods whose result must be used, by owner.
fn pure_method(cx: &Ctx, c: &grounds_semantic::ResolvedCall) -> bool {
    let owner = &*c.owner;
    let name = &*c.method.name;
    if c.method.ret == Ty::Void {
        return false;
    }
    if c.method.annotations.iter().any(|a| matches!(a.simple_name(), "CheckReturnValue" | "CheckResult" | "MustBeClosed")) {
        return true;
    }
    match owner {
        "java.lang.String" => !matches!(name, "getChars" | "getBytes" | "intern" | "hashCode"),
        "java.math.BigInteger" | "java.math.BigDecimal" => true,
        "java.lang.Integer" | "java.lang.Long" | "java.lang.Double" | "java.lang.Float" | "java.lang.Short" | "java.lang.Byte" | "java.lang.Boolean" | "java.lang.Character" => {
            !matches!(name, "getInteger" | "getLong" | "getBoolean" | "decode" | "parseInt" | "parseLong" | "valueOf")
        }
        "java.util.Optional" | "java.util.OptionalInt" | "java.util.OptionalLong" | "java.util.OptionalDouble" => {
            !matches!(name, "ifPresent" | "ifPresentOrElse" | "orElseThrow" | "get" | "getAsInt" | "getAsLong" | "getAsDouble")
        }
        "java.time.LocalDate" | "java.time.LocalDateTime" | "java.time.LocalTime" | "java.time.Instant" | "java.time.Duration" | "java.time.Period" | "java.time.ZonedDateTime" | "java.time.OffsetDateTime" | "java.time.Year" | "java.time.YearMonth" => true,
        "java.util.Objects" => matches!(name, "equals" | "hash" | "hashCode" | "toString" | "isNull" | "nonNull" | "compare"),
        "java.util.Collections" => name.starts_with("unmodifiable") || name.starts_with("synchronized") || name.starts_with("empty") || name.starts_with("singleton"),
        "java.util.Arrays" => matches!(name, "asList" | "stream" | "copyOf" | "copyOfRange" | "equals" | "hashCode" | "toString" | "deepEquals"),
        _ => {
            if matches!(name, "isEmpty" | "contains" | "containsKey" | "containsValue" | "size" | "equals" | "containsAll") && is_subtype_any(cx.fm, &Ty::class(owner), &["java.util.Collection", "java.util.Map"]) {
                return true;
            }
            false
        }
    }
}

declare_rule!(IgnoredReturnValue {
    id: "ignored-return-value",
    name: "Result of a side-effect-free method ignored",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S2201", "spotbugs:RV_RETURN_VALUE_IGNORED", "errorprone:ReturnValueIgnored", "errorprone:CheckReturnValue", "intellij:ResultOfMethodCallIgnored"],
    kinds: ["expression_statement"],
    description: "Strings, BigDecimals, java.time values and Optionals are immutable: `s.trim();` or `date.plusDays(1);` compute a new value and discard it, leaving the original unchanged. Assign the result.",
});

impl Rule for IgnoredReturnValue {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = n.first_kid() else { return };
        if e.kind() != "method_invocation" {
            return;
        }
        // lambda expression bodies and switch rules use the value
        if n.parent().is_some_and(|p| matches!(p.kind(), "switch_rule")) {
            return;
        }
        let Some(c) = cx.fm.select_method(e) else { return };
        if c.ambiguous {
            return;
        }
        // streams have a dedicated rule
        if is_subtype_any(cx.fm, &c.ret, &["java.util.stream.BaseStream"]) {
            return;
        }
        if pure_method(cx, &c) {
            cx.report(e, format!("The result of `{}.{}()` is ignored; the call has no other effect", simple_name_of(&c.owner), c.method.name));
        }
    }
}

const FILE_OPS: MethodMatcher = MethodMatcher::instance("java.io.File").named(&["delete", "mkdir", "mkdirs", "createNewFile", "renameTo", "setReadable", "setWritable", "setExecutable", "setLastModified", "setReadOnly"]);
const LOCK_TRY: MethodMatcher = MethodMatcher::instance("java.util.concurrent.locks.Lock").named(&["tryLock"]);
const QUEUE_OFFER: MethodMatcher = MethodMatcher::instance("java.util.concurrent.BlockingQueue").named(&["offer"]);
const LATCH_AWAIT: MethodMatcher = MethodMatcher::instance("java.util.concurrent.CountDownLatch").named(&["await"]).arity(2);
const EXECUTOR_AWAIT: MethodMatcher = MethodMatcher::instance("java.util.concurrent.ExecutorService").named(&["awaitTermination"]);

declare_rule!(IgnoredFileResult {
    id: "ignored-status-result",
    name: "Status-returning operation result ignored",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S899", "spotbugs:RV_RETURN_VALUE_IGNORED_BAD_PRACTICE"],
    kinds: ["expression_statement"],
    description: "`File.delete()`, `mkdirs()`, `renameTo()`, `Lock.tryLock()`, `BlockingQueue.offer()` and timed awaits report failure only through their boolean result. Ignoring it hides failures; check it or use the `java.nio.file.Files` equivalents that throw.",
});

impl Rule for IgnoredFileResult {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = n.first_kid() else { return };
        if e.kind() != "method_invocation" {
            return;
        }
        if cx.fm.call_matches_any(e, &[FILE_OPS, LOCK_TRY, QUEUE_OFFER, LATCH_AWAIT, EXECUTOR_AWAIT]) {
            cx.report(e, format!("The boolean result of `{}()` is ignored; failures go unnoticed", cx.fm.call_name(e).unwrap_or("")));
        }
    }
}

const READ_METHODS: MethodMatcher = MethodMatcher::any().named(&["read", "skip", "readNBytes"]);

declare_rule!(IgnoredReadResult {
    id: "ignored-read-result",
    name: "InputStream.read()/skip() result ignored",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2674", "spotbugs:RR_NOT_CHECKED", "spotbugs:SR_NOT_CHECKED"],
    kinds: ["expression_statement"],
    description: "`read(buf)` may fill only part of the buffer and `skip(n)` may skip fewer bytes; the return value says how many. Loop until done, or use `readNBytes`/`readAllBytes`/`DataInputStream.readFully`.",
});

impl Rule for IgnoredReadResult {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = n.first_kid() else { return };
        if e.kind() != "method_invocation" || !cx.fm.call_matches(e, &READ_METHODS) {
            return;
        }
        let args = cx.fm.args_of(e);
        if args.is_empty() {
            return;
        }
        let Some(c) = cx.fm.select_method(e) else { return };
        let owner_ok = ["java.io.InputStream", "java.io.Reader"].iter().any(|o| cx.fm.is_subclass(&c.owner, o).is_yes());
        if owner_ok && c.method.ret.is_prim() {
            cx.report(e, format!("The number of bytes actually processed by `{}()` is ignored", c.method.name));
        }
    }
}

declare_rule!(StreamNotConsumed {
    id: "stream-not-consumed",
    name: "Stream pipeline without a terminal operation",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S3958", "errorprone:StreamResourceLeak"],
    kinds: ["expression_statement"],
    description: "Intermediate stream operations (`map`, `filter`, `peek`...) are lazy; without a terminal operation like `forEach`, `collect` or `count`, nothing runs.",
});

impl Rule for StreamNotConsumed {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(e) = n.first_kid() else { return };
        if e.kind() != "method_invocation" {
            return;
        }
        if n.parent().is_some_and(|p| matches!(p.kind(), "switch_rule" | "lambda_expression")) {
            return;
        }
        let t = cx.type_of(e);
        if !is_subtype_any(cx.fm, &t, &["java.util.stream.BaseStream"]) {
            return;
        }
        // Stream-returning builder/factory methods with side effects are rare; require a receiver that is a stream
        if let Some(o) = e.field("object") {
            if is_subtype_any(cx.fm, &cx.type_of(o), &["java.util.stream.BaseStream"]) || cx.fm.call_name(e) == Some("stream") {
                cx.report(e, "Stream pipeline has no terminal operation; nothing is executed");
            }
        }
    }
}

declare_rule!(BooleanReturnsNull {
    id: "boolean-method-returns-null",
    name: "Boolean method returns null",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2447", "spotbugs:NP_BOOLEAN_RETURN_NULL"],
    kinds: ["return_statement"],
    description: "A method returning `Boolean` that returns null forces callers to handle three states and throws NullPointerException when auto-unboxed in a condition.",
});

impl Rule for BooleanReturnsNull {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !n.first_kid().is_some_and(is_null_literal) {
            return;
        }
        if n.ancestors().any(|a| a.kind() == "lambda_expression") {
            return;
        }
        if cx.fm.return_type_at(n).is_class("java.lang.Boolean") {
            if let Some(m) = enclosing_method_decl(n) {
                if annotation_names(m, cx.src).iter().any(|a| matches!(*a, "Nullable" | "CheckForNull")) {
                    return;
                }
            }
            cx.report(n, "Method returning Boolean returns null; unboxing it throws NullPointerException");
        }
    }
}

declare_rule!(CollectionReturnsNull {
    id: "collection-method-returns-null",
    name: "Collection or array method returns null",
    category: BadPractice,
    severity: Warning,
    sources: ["sonar:S1168", "spotbugs:PZLA_PREFER_ZERO_LENGTH_ARRAYS", "pmd:ReturnEmptyCollectionRatherThanNull"],
    kinds: ["return_statement"],
    description: "Returning null instead of an empty collection or array forces every caller to null-check and invites NullPointerExceptions. Return `List.of()`, `Collections.emptyList()` or an empty array.",
});

impl Rule for CollectionReturnsNull {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !n.first_kid().is_some_and(is_null_literal) {
            return;
        }
        if n.ancestors().take_while(|a| a.kind() != "method_declaration").any(|a| a.kind() == "lambda_expression") {
            return;
        }
        let Some(m) = enclosing_method_decl(n) else { return };
        if m.kind() != "method_declaration" || annotation_names(m, cx.src).iter().any(|a| matches!(*a, "Nullable" | "CheckForNull")) {
            return;
        }
        let rt = cx.fm.return_type_at(n);
        if rt.is_array() || is_subtype_any(cx.fm, &rt, &["java.util.Collection", "java.util.Map"]) {
            cx.report(n, format!("Return an empty `{}` instead of null", rt.display_simple()));
        }
    }
}

declare_rule!(ToStringReturnsNull {
    id: "tostring-returns-null",
    name: "toString() or clone() returns null",
    category: Bug,
    severity: Error,
    sources: ["sonar:S2225", "spotbugs:NP_TOSTRING_COULD_RETURN_NULL", "spotbugs:NP_CLONE_COULD_RETURN_NULL"],
    kinds: ["return_statement"],
    description: "Callers (string concatenation, logging, collections) never expect `toString()` or `clone()` to return null. Return an empty string or a proper copy.",
});

impl Rule for ToStringReturnsNull {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !n.first_kid().is_some_and(is_null_literal) {
            return;
        }
        let Some(m) = enclosing_method_decl(n) else { return };
        if n.ancestors().take_while(|a| *a != m).any(|a| a.kind() == "lambda_expression") {
            return;
        }
        let name = method_name(m, cx.src);
        if matches!(name, "toString" | "clone") && param_count(m) == 0 {
            cx.report(n, format!("`{name}()` must not return null"));
        }
    }
}

declare_rule!(CollectionAddsItself {
    id: "collection-contains-itself",
    name: "Collection passed to its own method",
    category: Bug,
    severity: Error,
    sources: ["sonar:S2114", "spotbugs:DMI_COLLECTIONS_SHOULD_NOT_CONTAIN_THEMSELVES", "spotbugs:DMI_USING_REMOVEALL_TO_CLEAR_COLLECTION"],
    kinds: ["method_invocation"],
    description: "`list.add(list)` makes the collection contain itself, so `hashCode()`/`toString()` overflow the stack; `list.containsAll(list)` is always true and `removeAll(list)` is `clear()`.",
});

impl Rule for CollectionAddsItself {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = cx.fm.call_name(n) else { return };
        if !matches!(name, "add" | "addAll" | "contains" | "containsAll" | "removeAll" | "retainAll" | "remove") {
            return;
        }
        let Some(o) = n.field("object") else { return };
        let args = cx.fm.args_of(n);
        if args.len() != 1 || !is_subtype_any(cx.fm, &cx.type_of(o), &["java.util.Collection"]) {
            return;
        }
        if same_pure_expr(o, args[0], cx.src) {
            cx.report(n, format!("Collection passed to its own `{name}()`"));
        }
    }
}

declare_rule!(ArraysAsListPrimitive {
    id: "arrays-aslist-primitive-array",
    name: "Arrays.asList() with a primitive array",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["errorprone:ArraysAsListPrimitiveArray", "sonar:S3878"],
    kinds: ["method_invocation"],
    description: "`Arrays.asList(intArray)` returns a `List<int[]>` with a single element, not a list of the numbers. Use `IntStream.of(a).boxed().toList()` or a boxed array.",
});

impl Rule for ArraysAsListPrimitive {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !cx.fm.call_matches(n, &MethodMatcher::statics("java.util.Arrays").named(&["asList"])) && !cx.fm.call_matches(n, &MethodMatcher::statics("java.util.List").named(&["of"])) && !cx.fm.call_matches(n, &MethodMatcher::statics("java.util.Set").named(&["of"])) {
            return;
        }
        let args = cx.fm.args_of(n);
        if args.len() == 1 {
            if let Ty::Array(e) = cx.type_of(args[0]) {
                if e.is_prim() {
                    cx.report(n, format!("`{}` of a `{}[]` creates a single-element list containing the array", cx.text(n.field("name").unwrap()), e));
                }
            }
        }
    }
}

declare_rule!(ToArrayCast {
    id: "toarray-downcast",
    name: "toArray() result cast to a typed array",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["spotbugs:BC_IMPOSSIBLE_DOWNCAST_OF_TOARRAY", "sonar:S3878"],
    kinds: ["cast_expression"],
    description: "`(String[]) list.toArray()` always throws ClassCastException because `toArray()` returns an `Object[]`. Use `list.toArray(new String[0])` or `toArray(String[]::new)`.",
});

impl Rule for ToArrayCast {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(v) = n.field("value").map(|v| v.unparen()) else { return };
        if !is_call_named(v, cx.src, &["toArray"]) || !cx.fm.args_of(v).is_empty() {
            return;
        }
        let t = cx.type_of(n);
        if let Ty::Array(e) = &t {
            if !e.is_class("java.lang.Object") && e.is_known() {
                cx.report(n, format!("`toArray()` returns Object[]; casting to `{}` throws ClassCastException", t.display_simple()));
            }
        }
    }
}

declare_rule!(ModifyCollectionInForeach {
    id: "collection-modified-during-iteration",
    name: "Collection modified while iterating over it",
    category: Bug,
    severity: Error,
    sources: ["errorprone:ModifyCollectionInEnhancedForLoop", "sonar:S6466", "spotbugs:DMI_ENTRY_SETS_MAY_REUSE_ENTRY_OBJECTS"],
    kinds: ["enhanced_for_statement", "method_invocation"],
    description: "Adding to or removing from a collection inside a for-each loop (or `forEach` lambda) over it throws ConcurrentModificationException. Use `Iterator.remove()`, `removeIf`, or collect changes and apply them afterwards.",
});

const MUTATORS: &[&str] = &["add", "remove", "addAll", "removeAll", "clear", "put", "retainAll", "removeIf", "addFirst", "addLast", "putAll"];

impl Rule for ModifyCollectionInForeach {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let (coll, body) = if n.kind() == "enhanced_for_statement" {
            let Some(v) = n.field("value").map(|v| v.unparen()) else { return };
            // iterating a copy or a view (keySet/values/entrySet) of a map is about the map
            let target = if is_call_named(v, cx.src, &["keySet", "values", "entrySet"]) { v.field("object") } else { Some(v) };
            let Some(t) = target else { return };
            (t, n.field("body"))
        } else {
            if cx.fm.call_name(n) != Some("forEach") {
                return;
            }
            let Some(o) = n.field("object") else { return };
            let lam = cx.fm.args_of(n).first().copied().filter(|a| a.kind() == "lambda_expression");
            (o, lam.and_then(|l| l.field("body")))
        };
        let Some(body) = body else { return };
        if !is_subtype_any(cx.fm, &cx.type_of(coll), &["java.util.Collection", "java.util.Map"]) {
            return;
        }
        // concurrent collections allow modification
        let ct_ty = cx.type_of(coll);
        if ct_ty.class_name().is_some_and(|c| c.starts_with("java.util.concurrent.")) {
            return;
        }
        // a Map declared by interface is often a ConcurrentHashMap; only trust a known initializer
        if ct_ty.class_name().is_some_and(|c| matches!(c, "java.util.Map" | "java.util.Collection" | "java.util.Set" | "java.util.Queue" | "java.util.Deque")) {
            let init = cx.fm.field_ref(coll.unparen()).and(None::<Node>).or_else(|| {
                let i = cx.fm.local_ref(coll.unparen())?;
                cx.fm.binder.decls[i].decl_node.field("value")
            });
            let known_plain = init.is_some_and(|v| {
                let t = cx.type_of(v);
                t.class_name().is_some_and(|c| c.starts_with("java.util.") && !c.starts_with("java.util.concurrent.") && !matches!(c, "java.util.Map" | "java.util.Collection" | "java.util.Set"))
            });
            if !known_plain {
                return;
            }
        }
        let ct = normalized(coll, cx.src);
        let mut calls = Vec::new();
        find_kind(body, "method_invocation", &mut calls);
        if body.kind() == "method_invocation" {
            calls.push(body);
        }
        for c in calls {
            let Some(name) = cx.fm.call_name(c) else { continue };
            if !MUTATORS.contains(&name) {
                continue;
            }
            if !c.field("object").is_some_and(|o| normalized(o, cx.src) == ct) {
                continue;
            }
            // removing and immediately leaving the loop is safe
            let stmt = c.parent().filter(|p| p.kind() == "expression_statement");
            if let Some(s) = stmt {
                if let Some(next) = s.next_named_sibling() {
                    if matches!(next.kind(), "break_statement" | "return_statement" | "throw_statement") {
                        continue;
                    }
                }
            }
            if c.ancestors().take_while(|a| *a != body).any(|a| a.kind() == "return_statement") {
                continue;
            }
            cx.report(c, format!("`{name}()` modifies the collection being iterated; this throws ConcurrentModificationException"));
        }
    }
}

const IMMUTABLE_FACTORIES: &[(&str, &[&str])] = &[
    ("java.util.List", &["of", "copyOf"]),
    ("java.util.Set", &["of", "copyOf"]),
    ("java.util.Map", &["of", "copyOf", "ofEntries"]),
    ("java.util.Collections", &["emptyList", "emptySet", "emptyMap", "singletonList", "singleton", "singletonMap", "unmodifiableList", "unmodifiableSet", "unmodifiableMap", "unmodifiableCollection"]),
];

fn immutable_source<'t>(v: Node<'t>, cx: &Ctx<'_, 't>) -> Option<&'static str> {
    let v = v.unparen();
    if v.kind() != "method_invocation" {
        return None;
    }
    let c = cx.fm.select_method(v)?;
    for (owner, names) in IMMUTABLE_FACTORIES {
        if &*c.owner == *owner && names.contains(&&*c.method.name) {
            return Some(if *owner == "java.util.Collections" { "an immutable Collections view" } else { "an immutable collection" });
        }
    }
    if &*c.owner == "java.util.Arrays" && &*c.method.name == "asList" {
        return Some("Arrays.asList (fixed size)");
    }
    if cx.fm.call_name(v) == Some("toList") && is_subtype_any(cx.fm, &cx.type_of(v.field("object")?), &["java.util.stream.Stream"]) {
        return Some("Stream.toList() (immutable)");
    }
    None
}

declare_rule!(ImmutableCollectionModification {
    id: "immutable-collection-modified",
    name: "Modification of an immutable collection",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S5838", "errorprone:DoNotCall", "intellij:ModifyingImmutableCollection"],
    kinds: ["method_invocation"],
    description: "Collections from `List.of`, `Map.of`, `Collections.unmodifiable*`, `emptyList()` and `Stream.toList()` throw UnsupportedOperationException on modification (`Arrays.asList` on add/remove). Copy into a mutable collection first.",
});

impl Rule for ImmutableCollectionModification {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = cx.fm.call_name(n) else { return };
        if !MUTATORS.contains(&name) && !matches!(name, "set" | "sort" | "replaceAll" | "putIfAbsent" | "computeIfAbsent" | "merge") {
            return;
        }
        let Some(o) = n.field("object").map(|o| o.unparen()) else { return };
        let src = if let Some(s) = immutable_source(o, cx) {
            Some(s)
        } else if let Some(i) = cx.fm.local_ref(o) {
            if !grounds_flow::analyses::is_effectively_final(cx.fm, i) {
                return;
            }
            let d = &cx.fm.binder.decls[i];
            d.decl_node.field("value").and_then(|v| immutable_source(v, cx))
        } else {
            None
        };
        let Some(src) = src else { return };
        if src.starts_with("Arrays.asList") && matches!(name, "set" | "sort" | "replaceAll") {
            return;
        }
        cx.report(n, format!("`{name}()` on {src} throws UnsupportedOperationException"));
    }
}

declare_rule!(MapKeySetGet {
    id: "map-keyset-then-get",
    name: "Iterating keySet() and calling get()",
    category: Performance,
    severity: Info,
    sources: ["sonar:S2864", "spotbugs:WMI_WRONG_MAP_ITERATOR"],
    kinds: ["enhanced_for_statement"],
    description: "Looping over `map.keySet()` and looking up each value with `map.get(key)` does a second lookup per entry. Iterate `entrySet()` instead.",
});

impl Rule for MapKeySetGet {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(v) = n.field("value").map(|v| v.unparen()) else { return };
        if !is_call_named(v, cx.src, &["keySet"]) {
            return;
        }
        let Some(map) = v.field("object") else { return };
        let (Some(var), Some(body)) = (n.field("name").map(|x| x.text(cx.src)), n.field("body")) else { return };
        let mt = normalized(map, cx.src);
        let mut calls = Vec::new();
        find_kind(body, "method_invocation", &mut calls);
        for c in calls {
            if cx.fm.call_name(c) == Some("get") && c.field("object").is_some_and(|o| normalized(o, cx.src) == mt) && cx.fm.args_of(c).first().is_some_and(|a| cx.text(*a) == var) {
                cx.report(v, "Iterate `entrySet()` instead of `keySet()` followed by `get()`");
                return;
            }
        }
    }
}

declare_rule!(StreamAnyMatch {
    id: "stream-find-is-present",
    name: "filter().findFirst().isPresent() instead of anyMatch()",
    category: Style,
    severity: Info,
    sources: ["sonar:S4034", "intellij:SimplifyStreamApiCallChains"],
    kinds: ["method_invocation"],
    description: "`stream.filter(p).findFirst().isPresent()` is `stream.anyMatch(p)`; `!...isPresent()` is `noneMatch(p)`.",
});

impl Rule for StreamAnyMatch {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(cx.fm.call_name(n), Some("isPresent" | "isEmpty")) {
            return;
        }
        let Some(f) = n.field("object").map(|o| o.unparen()) else { return };
        if !is_call_named(f, cx.src, &["findFirst", "findAny"]) {
            return;
        }
        let Some(filter) = f.field("object").map(|o| o.unparen()) else { return };
        if is_call_named(filter, cx.src, &["filter"]) && is_subtype_any(cx.fm, &cx.type_of(filter), &["java.util.stream.BaseStream"]) {
            cx.report(n, "Use `anyMatch()`/`noneMatch()` instead of `filter().findFirst().isPresent()`");
        }
    }
}

declare_rule!(StreamForEachOnCollection {
    id: "stream-foreach-on-collection",
    name: "stream().forEach() on a collection",
    category: Style,
    severity: Info,
    sources: ["sonar:S1612", "intellij:SimplifyStreamApiCallChains"],
    kinds: ["method_invocation"],
    description: "`collection.stream().forEach(...)` creates a stream for nothing; `collection.forEach(...)` does the same.",
});

impl Rule for StreamForEachOnCollection {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("forEach") {
            return;
        }
        let Some(s) = n.field("object").map(|o| o.unparen()) else { return };
        if is_call_named(s, cx.src, &["stream"]) && cx.fm.args_of(s).is_empty() {
            if let Some(c) = s.field("object") {
                if is_subtype_any(cx.fm, &cx.type_of(c), &["java.util.Collection"]) {
                    cx.report(n, "Call `forEach` on the collection directly");
                }
            }
        }
    }
}

declare_rule!(CollectorsToList {
    id: "collectors-tolist",
    name: "collect(Collectors.toList()) instead of toList()",
    category: Style,
    severity: Info,
    enabled: false,
    sources: ["sonar:S6204"],
    kinds: ["method_invocation"],
    description: "Since Java 16 `stream.toList()` is shorter and returns an unmodifiable list (note: `Collectors.toList()` returns a mutable one).",
});

impl Rule for CollectorsToList {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("collect") {
            return;
        }
        if cx.fm.args_of(n).first().is_some_and(|a| cx.text(*a).ends_with("Collectors.toList()") || cx.text(*a) == "toList()") {
            cx.report(n, "Use `Stream.toList()` (Java 16+)");
        }
    }
}

const SYSTEM_EXIT: MethodMatcher = MethodMatcher::statics("java.lang.System").named(&["exit"]);
const RUNTIME_EXIT: MethodMatcher = MethodMatcher::instance("java.lang.Runtime").named(&["exit", "halt"]);

declare_rule!(SystemExit {
    id: "system-exit",
    name: "System.exit() outside main",
    category: BadPractice,
    severity: Warning,
    scope: Main,
    sources: ["sonar:S1147", "spotbugs:DM_EXIT", "pmd:DoNotTerminateVM"],
    kinds: ["method_invocation"],
    description: "Calling `System.exit` from library or server code kills the whole JVM, skipping cleanup of other components. Throw an exception and let the application's entry point decide.",
});

impl Rule for SystemExit {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !(cx.fm.call_matches(n, &SYSTEM_EXIT) || cx.fm.call_matches(n, &RUNTIME_EXIT)) {
            return;
        }
        if enclosing_method_decl(n).is_some_and(|m| is_main_method(m, cx.src)) {
            return;
        }
        cx.report(n, "Avoid terminating the JVM outside the application entry point");
    }
}

declare_rule!(GcCall {
    id: "explicit-gc",
    name: "Explicit garbage collection",
    category: Performance,
    severity: Warning,
    sources: ["sonar:S1215", "spotbugs:DM_GC", "pmd:DoNotCallGarbageCollectionExplicitly"],
    kinds: ["method_invocation"],
    description: "`System.gc()` triggers a full stop-the-world collection (or nothing at all), hurting latency without fixing memory problems.",
});

impl Rule for GcCall {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_matches(n, &MethodMatcher::statics("java.lang.System").named(&["gc", "runFinalization"])) || cx.fm.call_matches(n, &MethodMatcher::instance("java.lang.Runtime").named(&["gc"])) {
            if cx.is_test() {
                return;
            }
            cx.report(n, "Do not call the garbage collector explicitly");
        }
    }
}

declare_rule!(SystemOut {
    id: "system-out-println",
    name: "Printing to System.out/err",
    category: BadPractice,
    severity: Info,
    scope: Main,
    sources: ["sonar:S106", "pmd:SystemPrintln", "intellij:UseOfSystemOutOrSystemErr"],
    kinds: ["method_invocation"],
    description: "Direct console output bypasses log levels, formatting and routing. Use a logger (CLI tools writing user output are the exception).",
});

impl Rule for SystemOut {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = cx.fm.call_name(n) else { return };
        if !matches!(name, "println" | "print" | "printf" | "format") {
            return;
        }
        let Some(o) = n.field("object") else { return };
        let t = cx.text(o);
        if t != "System.out" && t != "System.err" {
            return;
        }
        if enclosing_method_decl(n).is_some_and(|m| is_main_method(m, cx.src)) {
            return;
        }
        cx.report(n, format!("Use a logger instead of `{t}.{name}`"));
    }
}

declare_rule!(DeprecatedApiUsage {
    id: "deprecated-api-usage",
    name: "Use of deprecated API",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S1874", "sonar:S5738", "intellij:Deprecation"],
    kinds: ["method_invocation", "object_creation_expression"],
    description: "The called method, constructor or its class is marked `@Deprecated`; it may be removed or has a better replacement. APIs deprecated `forRemoval` are reported as warnings.",
});

impl Rule for DeprecatedApiUsage {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = cx.fm.select_method(n) else { return };
        if c.ambiguous {
            return;
        }
        let method_dep = c.method.flags.has(Flags::DEPRECATED);
        let owner = cx.fm.class(&c.owner);
        let class_dep = owner.as_ref().is_some_and(|o| o.flags.has(Flags::DEPRECATED));
        if !method_dep && !class_dep {
            return;
        }
        // usage from inside the deprecated class itself is not interesting
        if let Some(enc) = cx.fm.enclosing_class_name(n) {
            if enc.starts_with(&*c.owner) {
                return;
            }
            if cx.fm.class(&enc).is_some_and(|e| e.flags.has(Flags::DEPRECATED)) {
                return;
            }
        }
        if let Some(m) = enclosing_method_decl(n) {
            if has_annotation_named(m, cx.src, "Deprecated") {
                return;
            }
        }
        let anns = if method_dep { &c.method.annotations } else { &owner.as_ref().unwrap().annotations };
        let for_removal = anns.iter().any(|a| a.simple_name() == "Deprecated" && a.arg("forRemoval").and_then(|v| v.as_bool()) == Some(true));
        let what = if c.method.is_constructor() { format!("constructor of `{}`", simple_name_of(&c.owner)) } else { format!("`{}.{}()`", simple_name_of(&c.owner), c.method.name) };
        let msg = format!("{what} is deprecated{}", if for_removal { " for removal" } else { "" });
        if for_removal {
            cx.report_with_severity(n, Severity::Warning, msg);
        } else {
            cx.report(n, msg);
        }
    }
}

declare_rule!(SetAccessible {
    id: "reflection-set-accessible",
    name: "Reflection bypasses access control",
    category: Security,
    severity: Info,
    enabled: false,
    sources: ["sonar:S3011"],
    kinds: ["method_invocation"],
    description: "`setAccessible(true)` breaks encapsulation and fails under the module system without `--add-opens`.",
});

impl Rule for SetAccessible {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) == Some("setAccessible") && cx.fm.args_of(n).first().is_some_and(|a| cx.text(*a) == "true") {
            cx.report(n, "Reflective access bypasses encapsulation");
        }
    }
}

declare_rule!(ContainsKeyThenPut {
    id: "map-contains-then-put",
    name: "containsKey() followed by put() instead of computeIfAbsent()",
    category: Style,
    severity: Info,
    sources: ["sonar:S3824"],
    kinds: ["if_statement"],
    description: "`if (!map.containsKey(k)) map.put(k, new V())` does two lookups; `map.computeIfAbsent(k, key -> new V())` does it in one.",
});

impl Rule for ContainsKeyThenPut {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.field("alternative").is_some() {
            return;
        }
        let Some(c) = n.field("condition").and_then(|c| c.first_kid()).map(|c| c.unparen()) else { return };
        let call = if c.kind() == "unary_expression" && op(c) == "!" { c.field("operand").map(|o| o.unparen()) } else { None };
        let Some(call) = call else { return };
        if cx.fm.call_name(call) != Some("containsKey") {
            return;
        }
        let Some(map) = call.field("object") else { return };
        if !is_subtype_any(cx.fm, &cx.type_of(map), &["java.util.Map"]) || is_subtype_any(cx.fm, &cx.type_of(map), &["java.util.concurrent.ConcurrentMap"]) {
            return;
        }
        let Some(cons) = n.field("consequence") else { return };
        let stmts = if cons.kind() == "block" { cons.kids() } else { vec![cons] };
        if stmts.len() != 1 {
            return;
        }
        let Some(e) = stmts[0].first_kid() else { return };
        if cx.fm.call_name(e) == Some("put") && e.field("object").is_some_and(|o| normalized(o, cx.src) == normalized(map, cx.src)) {
            cx.report(call, "Use `computeIfAbsent`/`putIfAbsent` instead of `containsKey` + `put`");
        }
    }
}

declare_rule!(RemoveIntOnIntegerList {
    id: "list-remove-int-ambiguity",
    name: "List<Integer>.remove(int) removes by index",
    category: Bug,
    severity: Warning,
    sources: ["errorprone:CollectionUndefinedEquality", "sonar:S3878"],
    kinds: ["method_invocation"],
    description: "On a `List<Integer>`, `list.remove(i)` with an `int` removes the element at index i, not the value i. Use `remove(Integer.valueOf(i))` to remove by value.",
});

impl Rule for RemoveIntOnIntegerList {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("remove") {
            return;
        }
        let args = cx.fm.args_of(n);
        let Some(o) = n.field("object") else { return };
        if args.len() != 1 || cx.type_of(args[0]).prim() != Some(Prim::Int) {
            return;
        }
        let Some(list) = cx.fm.as_super(&cx.type_of(o), "java.util.List") else { return };
        if list.arg(0).is_some_and(|e| e.upper_bound().is_class("java.lang.Integer")) {
            // indexes usually come from loop counters / indexOf; values from elsewhere
            let a = cx.text(args[0]);
            if a.contains("indexOf") || a.contains("size()") || matches!(a, "i" | "j" | "k" | "idx" | "index" | "0") {
                return;
            }
            cx.report(n, "`remove(int)` on a List<Integer> removes by index; use `remove(Integer.valueOf(..))` to remove a value");
        }
    }
}

declare_rule!(OptionalOrElseCall {
    id: "optional-orelse-eager-call",
    name: "Optional.orElse() with a computed argument",
    category: Performance,
    severity: Info,
    sources: ["sonar:S3655", "intellij:OptionalGetWithoutIsPresent"],
    kinds: ["method_invocation"],
    description: "The argument of `orElse` is always evaluated, even when the Optional has a value. Use `orElseGet(() -> compute())` for non-trivial defaults.",
});

impl Rule for OptionalOrElseCall {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("orElse") {
            return;
        }
        let Some(o) = n.field("object") else { return };
        if !cx.type_of(o).is_class("java.util.Optional") {
            return;
        }
        let Some(a) = cx.fm.args_of(n).first().map(|a| a.unparen()) else { return };
        let expensive = matches!(a.kind(), "method_invocation" | "object_creation_expression") && !is_call_named(a, cx.src, &["emptyList", "emptySet", "emptyMap", "of", "empty", "valueOf", "getOrDefault"]);
        if expensive {
            cx.report(a, "This default is computed even when the Optional is present; use `orElseGet`");
        }
    }
}

declare_rule!(OptionalParameter {
    id: "optional-as-parameter-or-field",
    name: "Optional used as a parameter or field type",
    category: BadPractice,
    severity: Info,
    sources: ["sonar:S3553", "intellij:OptionalUsedAsFieldOrParameterType"],
    kinds: ["formal_parameter", "field_declaration"],
    description: "Optional is designed as a return type. As a parameter it forces callers to wrap values (and can itself be null); as a field it is not serializable. Use overloads or nullable values instead.",
});

impl Rule for OptionalParameter {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(t) = n.field("type") else { return };
        if n.kind() == "formal_parameter" && !n.parent().and_then(|p| p.parent()).is_some_and(|m| matches!(m.kind(), "method_declaration" | "constructor_declaration")) {
            return;
        }
        if n.kind() == "formal_parameter" {
            if let Some(m) = n.parent().and_then(|p| p.parent()) {
                if has_annotation_named(m, cx.src, "Override") {
                    return;
                }
            }
        }
        let ty = cx.fm.resolve_type_node(t);
        if ty.is_class("java.util.Optional") {
            cx.report(t, if n.kind() == "formal_parameter" { "Optional parameter; prefer an overload or a nullable parameter" } else { "Optional field; store a nullable value and expose an Optional getter" });
        }
    }
}

declare_rule!(CharsetForName {
    id: "charset-forname-constant",
    name: "Charset looked up by name instead of StandardCharsets",
    category: Style,
    severity: Info,
    sources: ["sonar:S4719"],
    kinds: ["method_invocation"],
    description: "`Charset.forName(\"UTF-8\")` and string charset names can fail at runtime; `StandardCharsets.UTF_8` is a compile-time constant.",
});

impl Rule for CharsetForName {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !cx.fm.call_matches(n, &MethodMatcher::statics("java.nio.charset.Charset").named(&["forName"])) {
            return;
        }
        let Some(a) = cx.fm.args_of(n).first().copied() else { return };
        let Some(name) = cx.fm.string_value(a) else { return };
        let constant = match name.to_ascii_uppercase().replace('_', "-").as_str() {
            "UTF-8" | "UTF8" => "UTF_8",
            "US-ASCII" | "ASCII" => "US_ASCII",
            "ISO-8859-1" | "LATIN1" => "ISO_8859_1",
            "UTF-16" => "UTF_16",
            "UTF-16BE" => "UTF_16BE",
            "UTF-16LE" => "UTF_16LE",
            _ => return,
        };
        cx.report_fix(n, format!("Use `StandardCharsets.{constant}`"), Fix::replace("Use StandardCharsets", n, format!("java.nio.charset.StandardCharsets.{constant}")));
    }
}

declare_rule!(DateGetTime {
    id: "new-date-gettime",
    name: "new Date().getTime() instead of currentTimeMillis()",
    category: Performance,
    severity: Info,
    sources: ["pmd:AvoidCallingNewDateGetTime", "sonar:S2143"],
    kinds: ["method_invocation"],
    description: "`new Date().getTime()` allocates an object just to read the clock; `System.currentTimeMillis()` (or `Instant.now()`) does it directly.",
});

impl Rule for DateGetTime {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("getTime") {
            return;
        }
        let Some(o) = n.field("object").map(|o| o.unparen()) else { return };
        if o.kind() == "object_creation_expression" && cx.type_of(o).is_class("java.util.Date") && cx.fm.args_of(o).is_empty() {
            cx.report_fix(n, "Use `System.currentTimeMillis()`", Fix::replace("Use currentTimeMillis", n, "System.currentTimeMillis()"));
        }
    }
}

declare_rule!(EnumKeyedHashMap {
    id: "enum-keyed-hash-collection",
    name: "HashMap/HashSet keyed by an enum",
    category: Performance,
    severity: Info,
    sources: ["sonar:S1640", "pmd:UseEnumMap"],
    kinds: ["object_creation_expression"],
    description: "`EnumMap`/`EnumSet` are array-backed and much faster and smaller than hash collections for enum keys.",
});

impl Rule for EnumKeyedHashMap {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let t = cx.type_of(n);
        let repl = match t.class_name() {
            Some("java.util.HashMap") => "EnumMap",
            Some("java.util.HashSet") => "EnumSet",
            _ => return,
        };
        let Some(k) = t.arg(0) else { return };
        if k.class_name().and_then(|c| cx.fm.class(c)).is_some_and(|c| c.is_enum()) {
            cx.report(n, format!("Use `{repl}` for enum keys"));
        }
    }
}

declare_rule!(OptionalIsPresentGet {
    id: "optional-ispresent-then-get",
    name: "isPresent() check followed by get()",
    category: Style,
    severity: Info,
    sources: ["sonar:S3655", "intellij:OptionalIsPresent"],
    kinds: ["if_statement"],
    description: "`if (opt.isPresent()) { use(opt.get()); }` is the null-check pattern Optional was meant to replace; use `opt.ifPresent(...)`, `map`, or `orElse`.",
});

impl Rule for OptionalIsPresentGet {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.field("alternative").is_some() {
            return;
        }
        let Some(c) = n.field("condition").and_then(|c| c.first_kid()).map(|c| c.unparen()) else { return };
        if cx.fm.call_name(c) != Some("isPresent") {
            return;
        }
        let Some(o) = c.field("object") else { return };
        if !cx.type_of(o).is_class("java.util.Optional") || cx.fm.local_ref(o.unparen()).is_none() {
            return;
        }
        let Some(cons) = n.field("consequence") else { return };
        let stmts = if cons.kind() == "block" { cons.kids() } else { vec![cons] };
        if stmts.len() != 1 || stmts[0].kind() != "expression_statement" {
            return;
        }
        let mut calls = Vec::new();
        find_kind(stmts[0], "method_invocation", &mut calls);
        if calls.iter().any(|x| cx.fm.call_name(*x) == Some("get") && x.field("object").is_some_and(|y| cx.text(y) == cx.text(o))) {
            cx.report(c, "Use `ifPresent(...)` instead of `isPresent()` + `get()`");
        }
    }
}

declare_rule!(StreamSortedFindFirst {
    id: "stream-sorted-findfirst",
    name: "sorted().findFirst() instead of min()/max()",
    category: Performance,
    severity: Info,
    sources: ["sonar:S4266", "intellij:SimplifyStreamApiCallChains"],
    kinds: ["method_invocation"],
    description: "Sorting a whole stream to take its first element is O(n log n); `min(comparator)`/`max` is O(n).",
});

impl Rule for StreamSortedFindFirst {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(cx.fm.call_name(n), Some("findFirst")) {
            return;
        }
        let Some(s) = n.field("object").map(|o| o.unparen()) else { return };
        if is_call_named(s, cx.src, &["sorted"]) && is_subtype_any(cx.fm, &cx.type_of(s), &["java.util.stream.BaseStream"]) {
            cx.report(n, "Use `min()`/`max()` instead of `sorted().findFirst()`");
        }
    }
}

declare_rule!(CollectionToArrayZero {
    id: "toarray-presized",
    name: "toArray(new T[size()]) instead of toArray(new T[0])",
    category: Performance,
    severity: Info,
    enabled: false,
    sources: ["intellij:ToArrayCallWithZeroLengthArrayArgument", "pmd:OptimizableToArrayCall"],
    kinds: ["method_invocation"],
    description: "On modern JVMs `toArray(new T[0])` is as fast or faster than pre-sizing and is thread-safe for concurrent collections.",
});

impl Rule for CollectionToArrayZero {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("toArray") {
            return;
        }
        if let Some(a) = cx.fm.args_of(n).first() {
            if a.kind() == "array_creation_expression" && cx.text(*a).contains(".size()") {
                cx.report(*a, "Use `toArray(new T[0])`");
            }
        }
    }
}

declare_rule!(StreamCountComparison {
    id: "stream-count-comparison",
    name: "stream count() compared with zero",
    category: Performance,
    severity: Info,
    sources: ["sonar:S4034"],
    kinds: ["binary_expression"],
    description: "`stream.count() > 0` consumes the whole stream; `anyMatch(x -> true)` / `findAny().isPresent()` stops at the first element.",
});

impl Rule for StreamCountComparison {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((l, r)) = lr(n) else { return };
        if !matches!(op(n), ">" | "!=" | "==" | ">=") {
            return;
        }
        let l = l.unparen();
        if is_call_named(l, cx.src, &["count"]) && l.field("object").is_some_and(|o| is_subtype_any(cx.fm, &cx.type_of(o), &["java.util.stream.BaseStream"])) && matches!(cx.fm.int_value(r), Some(0) | Some(1)) {
            cx.report(n, "Use `anyMatch()`/`findAny()` instead of counting the whole stream");
        }
    }
}
