//! String handling, format strings, regular expressions and logging calls.

use crate::framework::*;
use crate::text::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_semantic::MethodMatcher;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(StringConstructor),
        Box::new(BoxedConstructor),
        Box::new(StringToString),
        Box::new(UseIsEmpty),
        Box::new(IndexOfPositive),
        Box::new(ReplaceAllWithLiteral),
        Box::new(SplitOnRegexMetachar),
        Box::new(InvalidRegex),
        Box::new(FileSeparatorRegex),
        Box::new(CaseInsensitiveEquals),
        Box::new(StringConcatInLoop),
        Box::new(StringBuilderCharCtor),
        Box::new(FormatStringMismatch),
        Box::new(LogFormatMismatch),
        Box::new(LogConcatenation),
        Box::new(DefaultCharset),
        Box::new(CaseConversionWithoutLocale),
        Box::new(EqualsEmptyString),
        Box::new(UselessStringValueOf),
        Box::new(TrimIsEmpty),
        Box::new(StringBuilderAppendConcat),
        Box::new(FormatWithoutArgs),
        Box::new(BoxedToStringConversion),
        Box::new(ParseVsValueOf),
        Box::new(UnnecessaryBoxing),
        Box::new(PatternCompileInLoop),
        Box::new(SubstringToEnd),
    ]
}

const STRING: &str = "java.lang.String";

declare_rule!(StringConstructor {
    id: "string-constructor",
    name: "Unnecessary String constructor",
    category: Performance,
    severity: Info,
    sources: ["sonar:S2129", "spotbugs:DM_STRING_CTOR", "spotbugs:DM_STRING_VOID_CTOR", "pmd:StringInstantiation"],
    kinds: ["object_creation_expression"],
    description: "`new String(\"x\")` and `new String()` create needless copies; Strings are immutable. Use the literal directly.",
});

impl Rule for StringConstructor {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !cx.type_of(n).is_string() || n.kid_of_kind("class_body").is_some() {
            return;
        }
        let args = cx.fm.args_of(n);
        match args.len() {
            0 => cx.report_fix(n, "`new String()` is just \"\"", Fix::replace("Use \"\"", n, "\"\"")),
            1 if cx.type_of(args[0]).is_string() => {
                cx.report_fix(n, "Copying a String with `new String(...)` is unnecessary", Fix::replace("Remove constructor", n, cx.text(args[0])))
            }
            _ => {}
        }
    }
}

declare_rule!(BoxedConstructor {
    id: "boxed-primitive-constructor",
    name: "Deprecated boxed primitive constructor",
    category: Performance,
    severity: Warning,
    sources: ["sonar:S2129", "spotbugs:DM_NUMBER_CTOR", "spotbugs:DM_BOOLEAN_CTOR", "pmd:PrimitiveWrapperInstantiation"],
    kinds: ["object_creation_expression"],
    description: "`new Integer(5)` and friends are deprecated for removal and bypass the value cache. Use `Integer.valueOf(5)` or autoboxing.",
});

impl Rule for BoxedConstructor {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let t = cx.type_of(n);
        if !t.is_boxed() || n.kid_of_kind("class_body").is_some() {
            return;
        }
        let args = cx.fm.args_of(n);
        if args.len() != 1 {
            return;
        }
        let simple = t.display_simple();
        let fix = Fix::replace("Use valueOf", n, format!("{simple}.valueOf({})", cx.text(args[0])));
        cx.report_fix(n, format!("`new {simple}(...)` is deprecated; use `{simple}.valueOf(...)`"), fix);
    }
}

declare_rule!(StringToString {
    id: "string-tostring",
    name: "toString() called on a String",
    category: Style,
    severity: Info,
    sources: ["sonar:S1858", "pmd:StringToString"],
    kinds: ["method_invocation"],
    description: "Calling `toString()` on a String returns the same object; it is noise (and hides a NullPointerException risk).",
});

impl Rule for StringToString {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("toString") || !cx.fm.args_of(n).is_empty() {
            return;
        }
        if let Some(o) = n.field("object") {
            if o.kind() != "super" && cx.type_of(o).is_string() {
                cx.report_fix(n, "`toString()` on a String is redundant", Fix::replace("Remove toString()", n, cx.text(o)));
            }
        }
    }
}

declare_rule!(UseIsEmpty {
    id: "use-isempty",
    name: "Use isEmpty() to check emptiness",
    category: Style,
    severity: Info,
    sources: ["sonar:S1155", "sonar:S7158", "pmd:UseCollectionIsEmpty", "intellij:SizeReplaceableByIsEmpty"],
    kinds: ["binary_expression"],
    description: "`isEmpty()` states the intent directly and is O(1) for every collection, unlike `size()` on some concurrent or lazy implementations.",
});

impl Rule for UseIsEmpty {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((l, r)) = lr(n) else { return };
        let o = op(n);
        for (call, k, flipped) in [(l, r, false), (r, l, true)] {
            let call = call.unparen();
            if call.kind() != "method_invocation" || !cx.fm.args_of(call).is_empty() {
                continue;
            }
            let name = cx.fm.call_name(call).unwrap_or("");
            let Some(recv) = call.field("object") else { continue };
            let rt = cx.type_of(recv);
            let ok_recv = match name {
                "size" => is_subtype_any(cx.fm, &rt, &["java.util.Collection", "java.util.Map"]),
                "length" => rt.is_string() || is_subtype_any(cx.fm, &rt, &["java.lang.StringBuilder"]) && false,
                _ => false,
            };
            if !ok_recv {
                continue;
            }
            let Some(v) = cx.fm.int_value(k) else { continue };
            let o = if flipped {
                match o {
                    "<" => ">",
                    ">" => "<",
                    "<=" => ">=",
                    ">=" => "<=",
                    x => x,
                }
            } else {
                o
            };
            let empty = match (o, v) {
                ("==", 0) | ("<", 1) | ("<=", 0) => Some(true),
                ("!=", 0) | (">", 0) | (">=", 1) => Some(false),
                _ => None,
            };
            if let Some(e) = empty {
                // isEmpty() on CharSequence only exists since Java 15 for non-String
                let repl = format!("{}{}.isEmpty()", if e { "" } else { "!" }, cx.text(recv));
                cx.report_fix(n, format!("Use `{}isEmpty()` instead of comparing `{name}()`", if e { "" } else { "!" }), Fix::replace("Use isEmpty()", n, repl));
                return;
            }
        }
    }
}

declare_rule!(IndexOfPositive {
    id: "indexof-positive-check",
    name: "indexOf() result compared with > 0",
    category: Bug,
    severity: Warning,
    sources: ["sonar:S2692"],
    kinds: ["binary_expression"],
    description: "`indexOf` returns 0 when the match is at the start; `> 0` ignores that case. Use `>= 0` (or `contains()`) unless skipping the first position is intended.",
});

impl Rule for IndexOfPositive {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((l, r)) = lr(n) else { return };
        let o = op(n);
        let (call, k) = match o {
            ">" => (l, r),
            "<" => (r, l),
            _ => return,
        };
        let call = call.unparen();
        if !is_call_named(call, cx.src, &["indexOf"]) || cx.fm.int_value(k) != Some(0) || cx.fm.args_of(call).len() != 1 {
            return;
        }
        let Some(recv) = call.field("object") else { return };
        if cx.type_of(recv).is_string() || is_subtype_any(cx.fm, &cx.type_of(recv), &["java.util.List"]) {
            cx.report(n, "`indexOf(...) > 0` ignores a match at position 0; did you mean `>= 0`?");
        }
    }
}

const STRING_REGEX_METHODS: MethodMatcher = MethodMatcher::exact(STRING).named(&["replaceAll", "replaceFirst", "matches", "split"]);
const PATTERN_METHODS: MethodMatcher = MethodMatcher::statics("java.util.regex.Pattern").named(&["compile", "matches"]);

/// The regex argument of a regex-taking call, if any.
fn regex_arg<'t>(n: Node<'t>, cx: &Ctx<'_, 't>) -> Option<Node<'t>> {
    if cx.fm.call_matches(n, &STRING_REGEX_METHODS) || cx.fm.call_matches(n, &PATTERN_METHODS) {
        return cx.fm.args_of(n).first().copied();
    }
    None
}

declare_rule!(ReplaceAllWithLiteral {
    id: "replaceall-with-literal",
    name: "replaceAll() with a non-regex literal",
    category: Performance,
    severity: Info,
    sources: ["sonar:S5361", "pmd:UseStringReplace"],
    kinds: ["method_invocation"],
    description: "`replaceAll` compiles a regular expression on every call. When the pattern has no regex metacharacters, `replace()` does the same thing faster.",
});

impl Rule for ReplaceAllWithLiteral {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("replaceAll") {
            return;
        }
        let args = cx.fm.args_of(n);
        if args.len() != 2 || !cx.fm.call_matches(n, &STRING_REGEX_METHODS) {
            return;
        }
        let (Some(p), Some(rep)) = (cx.fm.string_value(args[0]), cx.fm.string_value(args[1]).or(Some("x".into()))) else { return };
        if p.is_empty() || has_regex_meta(&p) || rep.contains('$') || rep.contains('\\') {
            return;
        }
        let name = n.field("name").unwrap();
        cx.report_fix(n, "Pattern has no regex metacharacters; use `replace()`", Fix::replace("Use replace()", name, "replace"));
    }
}

declare_rule!(SplitOnRegexMetachar {
    id: "regex-metachar-literal",
    name: "Single regex metacharacter used as a literal",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["spotbugs:RE_POSSIBLE_UNINTENDED_PATTERN", "errorprone:StringSplitter"],
    kinds: ["method_invocation"],
    description: "`split(\".\")` treats the argument as a regex: `.` matches every character (so the result is empty) and `|` the empty string. Escape it (`\"\\\\.\"`) or use `Pattern.quote`.",
});

impl Rule for SplitOnRegexMetachar {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = cx.fm.call_name(n) else { return };
        // split(".") always yields an empty array, and `|` alone matches the
        // empty string; but replaceAll(".", "*") is a deliberate "every
        // character" (masking) as often as a mistake
        if !matches!(name, "split" | "replaceAll" | "replaceFirst") {
            return;
        }
        let only_bar = name != "split";
        let Some(a) = regex_arg(n, cx) else { return };
        if let Some(p) = cx.fm.string_value(a) {
            if p == "|" || (!only_bar && (p == "." || p == "$" || p == "^")) {
                cx.report(a, format!("`\"{p}\"` is a regex metacharacter here; escape it or use `Pattern.quote`"));
            }
        }
    }
}

declare_rule!(InvalidRegex {
    id: "invalid-regex",
    name: "Invalid regular expression",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S5856", "errorprone:InvalidPatternSyntax"],
    kinds: ["method_invocation"],
    description: "The constant regular expression is not valid java.util.regex syntax and will throw `PatternSyntaxException` at runtime.",
});

impl Rule for InvalidRegex {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(a) = regex_arg(n, cx) else { return };
        let Some(p) = cx.fm.string_value(a) else { return };
        // Pattern.LITERAL disables syntax; comment mode (COMMENTS flag or an
        // inline `(?x)`) makes `#...` comments and whitespace insignificant,
        // which the validator does not model
        if cx.fm.args_of(n).iter().skip(1).any(|f| cx.text(*f).contains("LITERAL") || cx.text(*f).contains("COMMENTS")) {
            return;
        }
        let comment_mode = p.match_indices("(?").any(|(i, _)| p[i + 2..].chars().take_while(|c| c.is_ascii_alphabetic()).any(|c| c == 'x'));
        if comment_mode {
            return;
        }
        if let Err(e) = check_java_regex(&p) {
            cx.report(a, format!("Invalid regex: {e}"));
        }
    }
}

declare_rule!(FileSeparatorRegex {
    id: "file-separator-as-regex",
    name: "File.separator used as a regex",
    category: Bug,
    severity: Error,
    sources: ["spotbugs:RE_CANT_USE_FILE_SEPARATOR_AS_REGULAR_EXPRESSION"],
    kinds: ["method_invocation"],
    description: "On Windows `File.separator` is `\\`, an invalid regex on its own. Use `Pattern.quote(File.separator)`.",
});

impl Rule for FileSeparatorRegex {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(a) = regex_arg(n, cx) else { return };
        if let Some(f) = cx.fm.field_ref(a.unparen()) {
            // pathSeparator (`:` / `;`) is regex-safe
            if &*f.owner == "java.io.File" && &*f.field.name == "separator" {
                cx.report(a, "`File.separator` is `\\` on Windows, which is not a valid regex; use `Pattern.quote()`");
            }
        }
    }
}

declare_rule!(CaseInsensitiveEquals {
    id: "case-insensitive-equals",
    name: "Case conversion before equals()",
    category: Performance,
    severity: Info,
    sources: ["sonar:S1157", "pmd:UnnecessaryCaseChange"],
    kinds: ["method_invocation"],
    description: "`a.toLowerCase().equals(b.toLowerCase())` allocates two strings and is locale-sensitive. Use `a.equalsIgnoreCase(b)`.",
});

impl Rule for CaseInsensitiveEquals {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("equals") {
            return;
        }
        let args = cx.fm.args_of(n);
        let Some(o) = n.field("object") else { return };
        if args.len() != 1 {
            return;
        }
        let is_case = |x: Node<'t>| {
            let x = x.unparen();
            is_call_named(x, cx.src, &["toLowerCase", "toUpperCase"]) && cx.fm.args_of(x).is_empty() && x.field("object").is_some_and(|r| cx.type_of(r).is_string())
        };
        if is_case(o) {
            let base = o.field("object").map(|r| cx.text(r)).unwrap_or("");
            let other = if is_case(args[0]) { args[0].unparen().field("object").map(|r| cx.text(r)).unwrap_or("") } else { cx.text(args[0]) };
            cx.report_fix(n, "Use `equalsIgnoreCase()` instead of converting case", Fix::replace("Use equalsIgnoreCase", n, format!("{base}.equalsIgnoreCase({other})")));
        }
    }
}

declare_rule!(StringConcatInLoop {
    id: "string-concat-in-loop",
    name: "String concatenation in a loop",
    category: Performance,
    severity: Info,
    sources: ["sonar:S1643", "pmd:UseStringBufferForStringAppends", "intellij:StringConcatenationInLoops"],
    kinds: ["assignment_expression"],
    description: "Each `s += x` in a loop copies the whole string, making the loop quadratic. Accumulate in a `StringBuilder`.",
});

impl Rule for StringConcatInLoop {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((l, r)) = lr(n) else { return };
        if !cx.type_of(l).is_string() || cx.fm.local_ref(l).is_none() {
            return;
        }
        let is_concat = match op(n) {
            "+=" => true,
            "=" => {
                let r = r.unparen();
                r.kind() == "binary_expression" && op(r) == "+" && r.field("left").is_some_and(|x| normalized(x, cx.src) == normalized(l, cx.src))
            }
            _ => false,
        };
        if !is_concat {
            return;
        }
        // variable declared outside the loop
        let Some(lp) = enclosing_loop(n) else { return };
        if let Some(i) = cx.fm.local_ref(l) {
            let d = &cx.fm.binder.decls[i];
            if lp.text_range().contains(d.name_node.start_byte() as u32) {
                return;
            }
        }
        cx.report(n, "String built by concatenation inside a loop; use a StringBuilder");
    }
}

declare_rule!(StringBuilderCharCtor {
    id: "stringbuilder-char-constructor",
    name: "StringBuilder constructed with a char",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S1317", "pmd:StringBufferInstantiationWithChar", "spotbugs:DM_STRING_BUFFER_CHAR"],
    kinds: ["object_creation_expression"],
    description: "`new StringBuilder('a')` calls the `int capacity` constructor (97), producing an empty builder. Use `new StringBuilder(\"a\")`.",
});

impl Rule for StringBuilderCharCtor {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(cx.type_of(n).class_name(), Some("java.lang.StringBuilder" | "java.lang.StringBuffer")) {
            return;
        }
        let args = cx.fm.args_of(n);
        if args.len() == 1 && cx.type_of(args[0]).prim() == Some(Prim::Char) {
            let fix = if args[0].kind() == "character_literal" {
                let t = cx.text(args[0]);
                Some(Fix::replace("Use a String", args[0], format!("\"{}\"", &t[1..t.len() - 1])))
            } else {
                None
            };
            let msg = "char argument selects the capacity constructor; the builder starts empty";
            match fix {
                Some(f) => cx.report_fix(n, msg, f),
                None => cx.report(n, msg),
            }
        }
    }
}

const FORMAT_STATIC: MethodMatcher = MethodMatcher::statics(STRING).named(&["format"]);
const PRINTF: MethodMatcher = MethodMatcher::any().named(&["printf", "format"]);

/// (format string node, arguments after it) for a formatting call.
fn format_call<'t>(n: Node<'t>, cx: &Ctx<'_, 't>) -> Option<(Node<'t>, Vec<Node<'t>>)> {
    let name = cx.fm.call_name(n)?;
    let args = cx.fm.args_of(n);
    if name == "formatted" {
        let o = n.field("object")?;
        if cx.type_of(o).is_string() {
            return Some((o, args));
        }
        return None;
    }
    if !matches!(name, "format" | "printf") {
        return None;
    }
    let c = cx.fm.select_method(n)?;
    let owner_ok = &*c.owner == STRING
        || ["java.io.PrintStream", "java.io.PrintWriter", "java.util.Formatter"].iter().any(|o| cx.fm.is_subclass(&c.owner, o).is_yes());
    if !owner_ok || !(cx.fm.call_matches(n, &FORMAT_STATIC) || cx.fm.call_matches(n, &PRINTF)) {
        return None;
    }
    let mut i = 0;
    if c.method.params.first().is_some_and(|p| p.ty.is_class("java.util.Locale")) {
        i = 1;
    }
    let fmt = *args.get(i)?;
    if !cx.type_of(fmt).is_string() {
        return None;
    }
    Some((fmt, args[i + 1..].to_vec()))
}

declare_rule!(FormatStringMismatch {
    id: "format-string-mismatch",
    name: "Format string does not match its arguments",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S2275", "sonar:S3457", "spotbugs:VA_FORMAT_STRING_MISSING_ARGUMENT", "spotbugs:VA_FORMAT_STRING_EXTRA_ARGUMENTS_PASSED", "spotbugs:VA_FORMAT_STRING_BAD_CONVERSION", "errorprone:FormatString"],
    kinds: ["method_invocation"],
    description: "`String.format`/`printf` throw at runtime for invalid specifiers, missing arguments or incompatible types (e.g. `%d` with a String), and silently ignore extra arguments.",
});

impl Rule for FormatStringMismatch {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((fmt, args)) = format_call(n, cx) else { return };
        let Some(f) = cx.fm.string_value(fmt) else { return };
        // a single array argument is spread as varargs
        if args.len() == 1 && cx.type_of(args[0]).is_array() && !matches!(cx.type_of(args[0]).element(), Some(Ty::Prim(_))) {
            return;
        }
        let parsed = match parse_format(&f) {
            Ok(p) => p,
            Err(e) => {
                cx.report(fmt, format!("Invalid format string: {e}"));
                return;
            }
        };
        if parsed.required > args.len() {
            cx.report(n, format!("Format string needs {} argument(s) but {} given; this throws MissingFormatArgumentException", parsed.required, args.len()));
            return;
        }
        if parsed.required < args.len() {
            cx.report_with_severity(n, Severity::Warning, format!("Format string uses {} argument(s) but {} given; extras are ignored", parsed.required, args.len()));
            return;
        }
        for (idx, spec) in spec_args(&parsed) {
            let Some(a) = args.get(idx) else { continue };
            let t = cx.type_of(*a);
            if t.is_unknown() || t == Ty::Null {
                continue;
            }
            let integral = matches!(t.unboxed(), Some(p) if p.is_integral() && p != Prim::Char) || t.is_class("java.math.BigInteger");
            let floating = matches!(t.unboxed(), Some(Prim::Float | Prim::Double)) || t.is_class("java.math.BigDecimal");
            let bad = match spec.conversion {
                'd' | 'o' | 'x' | 'X' => !integral && (t.is_string() || floating || t.is_boolean() || t.unboxed() == Some(Prim::Char)),
                'e' | 'E' | 'f' | 'g' | 'G' | 'a' | 'A' => !floating && (t.is_string() || integral || t.is_boolean()),
                'c' | 'C' => t.is_string() || floating || t.is_boolean(),
                _ => false,
            };
            if bad {
                cx.report(*a, format!("`%{}` cannot format a `{}`; this throws IllegalFormatConversionException", spec.conversion, t.display_simple()));
            }
        }
    }
}

const LOG_LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error", "fatal", "atTrace", "log"];

fn is_logger_type<'t>(cx: &Ctx<'_, 't>, t: &Ty) -> bool {
    match t.class_name() {
        Some(n) => matches!(n, "org.slf4j.Logger" | "org.apache.logging.log4j.Logger" | "org.slf4j.ext.XLogger") || cx.fm.is_subclass(n, "org.slf4j.Logger").is_yes() || cx.fm.is_subclass(n, "org.apache.logging.log4j.Logger").is_yes(),
        None => false,
    }
}

/// (message node, args after it) for an SLF4J/Log4j2 placeholder logging call.
fn log_call<'t>(n: Node<'t>, cx: &Ctx<'_, 't>) -> Option<(Node<'t>, Vec<Node<'t>>)> {
    let name = cx.fm.call_name(n)?;
    if !LOG_LEVELS.contains(&name) || name == "log" || name == "atTrace" {
        return None;
    }
    let recv = n.field("object")?;
    let rt = cx.type_of(recv);
    let is_logger = is_logger_type(cx, &rt) || (rt.is_unknown() && matches!(cx.text(recv), "log" | "logger" | "LOG" | "LOGGER"));
    if !is_logger {
        return None;
    }
    let args = cx.fm.args_of(n);
    let mut i = 0;
    if args.first().is_some_and(|a| cx.type_of(*a).class_name().is_some_and(|c| c.ends_with("Marker"))) {
        i = 1;
    }
    let msg = *args.get(i)?;
    Some((msg, args[i + 1..].to_vec()))
}

declare_rule!(LogFormatMismatch {
    id: "log-placeholder-mismatch",
    name: "Logging placeholders do not match arguments",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S2275", "sonar:S3457", "pmd:InvalidLogMessageFormat", "errorprone:Slf4jFormatShouldBeConst"],
    kinds: ["method_invocation"],
    description: "SLF4J/Log4j2 messages with more `{}` than arguments print literal `{}`; extra arguments are silently dropped (except a trailing exception, which is logged with its stack trace).",
});

impl Rule for LogFormatMismatch {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((msg, args)) = log_call(n, cx) else { return };
        let Some(text) = cx.fm.string_value(msg) else { return };
        if args.len() == 1 && cx.type_of(args[0]).is_array() {
            return;
        }
        let placeholders = count_placeholders(&text);
        let mut nargs = args.len();
        let last_is_throwable = args.last().is_some_and(|a| is_throwable_expr(cx.fm, *a));
        if last_is_throwable && placeholders < nargs {
            nargs -= 1;
        }
        if placeholders != nargs {
            // a lone throwable argument without placeholders is the normal exception overload
            if placeholders == 0 && args.len() == 1 && last_is_throwable {
                return;
            }
            cx.report(n, format!("Log message has {placeholders} placeholder(s) but {nargs} argument(s)"));
        }
    }
}

declare_rule!(LogConcatenation {
    id: "log-string-concatenation",
    name: "String concatenation in a logging call",
    category: Performance,
    severity: Info,
    sources: ["sonar:S2629", "pmd:GuardLogStatement", "intellij:LoggingStringTemplateAsArgument"],
    kinds: ["method_invocation"],
    description: "Concatenating the message builds the string even when the level is disabled. Use placeholders: `log.debug(\"user {} logged in\", user)`.",
});

impl Rule for LogConcatenation {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((msg, _)) = log_call(n, cx) else { return };
        let m = msg.unparen();
        if m.kind() == "binary_expression" && op(m) == "+" && cx.fm.constant_value(m).is_none() {
            // guarded by isDebugEnabled() etc.
            let guarded = n.ancestors().take(6).any(|a| a.kind() == "if_statement" && a.field("condition").is_some_and(|c| cx.text(c).contains("Enabled")));
            if !guarded {
                cx.report(msg, "Use placeholders instead of string concatenation in log messages");
            }
        }
        if m.kind() == "method_invocation" && cx.fm.call_matches(m, &FORMAT_STATIC) {
            cx.report(msg, "Use placeholders instead of String.format in log messages");
        }
    }
}

declare_rule!(DefaultCharset {
    id: "default-charset",
    name: "Reliance on the platform default charset",
    category: BadPractice,
    severity: Info,
    sources: ["spotbugs:DM_DEFAULT_ENCODING", "errorprone:DefaultCharset", "sonar:S1943"],
    kinds: ["method_invocation", "object_creation_expression"],
    description: "`getBytes()`, `new String(bytes)`, `new FileReader(f)` and similar use the platform default charset, which differs between machines (before Java 18). Pass `StandardCharsets.UTF_8` explicitly.",
});

impl Rule for DefaultCharset {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.kind() == "method_invocation" {
            if cx.fm.call_name(n) == Some("getBytes") && cx.fm.args_of(n).is_empty()
                && n.field("object").is_some_and(|o| cx.type_of(o).is_string()) {
                    cx.report(n, "`getBytes()` uses the platform default charset; pass `StandardCharsets.UTF_8`");
                }
            return;
        }
        let t = cx.type_of(n);
        let Some(cn) = t.class_name() else { return };
        let args = cx.fm.args_of(n);
        let arg_types: Vec<Ty> = args.iter().map(|a| cx.type_of(*a)).collect();
        let has_charset = arg_types.iter().any(|a| matches!(a.class_name(), Some("java.nio.charset.Charset" | "java.nio.charset.CharsetDecoder" | "java.nio.charset.CharsetEncoder")) || a.is_string() && cn != STRING && cn != "java.io.FileReader" && cn != "java.io.FileWriter" && cn != "java.io.PrintWriter" && cn != "java.io.PrintStream" && cn != "java.util.Scanner");
        if has_charset {
            return;
        }
        let flagged = match cn {
            STRING => arg_types.first().is_some_and(|a| matches!(a, Ty::Array(e) if matches!(**e, Ty::Prim(Prim::Byte)))) && !arg_types.iter().any(|a| a.is_string()),
            "java.io.InputStreamReader" | "java.io.OutputStreamWriter" => args.len() == 1,
            "java.io.FileReader" | "java.io.FileWriter" => !arg_types.iter().any(|a| a.class_name() == Some("java.nio.charset.Charset")),
            "java.util.Scanner" => args.len() == 1 && arg_types.first().is_some_and(|a| is_subtype_any(cx.fm, a, &["java.io.InputStream", "java.io.File", "java.nio.file.Path"])) && cx.text(args[0]) != "System.in",
            "java.io.PrintStream" | "java.io.PrintWriter" => {
                args.len() == 1 && arg_types.first().is_some_and(|a| is_subtype_any(cx.fm, a, &["java.io.OutputStream", "java.io.File"]) || a.is_string()) && !matches!(cx.text(args[0]), "System.out" | "System.err")
            }
            _ => false,
        };
        if flagged {
            cx.report(n, format!("`new {}(...)` uses the platform default charset; pass a Charset", simple_name_of(cn)));
        }
    }
}

declare_rule!(CaseConversionWithoutLocale {
    id: "case-conversion-without-locale",
    name: "toLowerCase()/toUpperCase() without Locale",
    category: BadPractice,
    severity: Info,
    enabled: false,
    sources: ["spotbugs:DM_CONVERT_CASE", "errorprone:StringCaseLocaleUsage", "sonar:S1449"],
    kinds: ["method_invocation"],
    description: "Case conversion depends on the default locale: in Turkish `\"TITLE\".toLowerCase()` yields `tıtle`. Use `toLowerCase(Locale.ROOT)` for identifiers and protocol keywords.",
});

impl Rule for CaseConversionWithoutLocale {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if is_call_named(n, cx.src, &["toLowerCase", "toUpperCase"]) && cx.fm.args_of(n).is_empty() && n.field("object").is_some_and(|o| cx.type_of(o).is_string()) {
            cx.report(n, "Locale-sensitive case conversion; pass `Locale.ROOT` for locale-independent text");
        }
    }
}

declare_rule!(EqualsEmptyString {
    id: "equals-empty-string",
    name: "Comparison with \"\" via equals()",
    category: Style,
    severity: Info,
    sources: ["pmd:UseStringIsEmpty", "sonar:S7158", "intellij:StringEqualsEmptyString"],
    kinds: ["method_invocation"],
    description: "`s.equals(\"\")` is clearer as `s.isEmpty()` (and `\"\".equals(s)` as a null-safe emptiness check).",
});

impl Rule for EqualsEmptyString {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("equals") {
            return;
        }
        let args = cx.fm.args_of(n);
        let Some(o) = n.field("object") else { return };
        if args.len() == 1 && cx.fm.string_value(args[0]).as_deref() == Some("") && cx.type_of(o).is_string() {
            cx.report_fix(n, "Use `isEmpty()` instead of `equals(\"\")`", Fix::replace("Use isEmpty()", n, format!("{}.isEmpty()", cx.text(o))));
        }
    }
}

declare_rule!(UselessStringValueOf {
    id: "useless-string-valueof",
    name: "String.valueOf() in string concatenation",
    category: Style,
    severity: Info,
    sources: ["sonar:S1153", "pmd:UselessStringValueOf"],
    kinds: ["method_invocation"],
    description: "String concatenation already converts operands; wrapping them in `String.valueOf()` or calling `toString()` is noise.",
});

impl Rule for UselessStringValueOf {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("valueOf") {
            return;
        }
        let args = cx.fm.args_of(n);
        if args.len() != 1 || !cx.fm.call_matches(n, &MethodMatcher::statics(STRING).named(&["valueOf"])) {
            return;
        }
        if cx.type_of(args[0]).is_array() {
            return;
        }
        let Some(p) = n.parent_unparen() else { return };
        if p.kind() == "binary_expression" && op(p) == "+" && cx.type_of(p).is_string() {
            // the other operand must be a String so the conversion still happens
            let other = if p.field("left").map(|x| x.unparen()) == Some(n) { p.field("right") } else { p.field("left") };
            if other.is_some_and(|o| cx.type_of(o).is_string()) {
                cx.report_fix(n, "`String.valueOf()` is redundant in string concatenation", Fix::replace("Remove valueOf", n, cx.text(args[0])));
            }
        }
    }
}

declare_rule!(TrimIsEmpty {
    id: "trim-isempty",
    name: "trim().isEmpty() instead of isBlank()",
    category: Style,
    severity: Info,
    sources: ["sonar:S7158", "intellij:StringOperationCanBeSimplified"],
    kinds: ["method_invocation"],
    description: "`s.trim().isEmpty()` allocates a new string; `s.isBlank()` (Java 11+) checks the same thing without copying and also handles Unicode whitespace.",
});

impl Rule for TrimIsEmpty {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("isEmpty") {
            return;
        }
        let Some(o) = n.field("object").map(|o| o.unparen()) else { return };
        if is_call_named(o, cx.src, &["trim", "strip"]) && cx.fm.args_of(o).is_empty() {
            if let Some(r) = o.field("object") {
                if cx.type_of(r).is_string() {
                    cx.report_fix(n, "Use `isBlank()` instead of `trim().isEmpty()`", Fix::replace("Use isBlank()", n, format!("{}.isBlank()", cx.text(r))));
                }
            }
        }
    }
}

declare_rule!(StringBuilderAppendConcat {
    id: "stringbuilder-append-concat",
    name: "Concatenation inside StringBuilder.append()",
    category: Performance,
    severity: Info,
    sources: ["pmd:InefficientStringBuffering", "intellij:StringConcatenationInsideStringBufferAppend"],
    kinds: ["method_invocation"],
    description: "`sb.append(a + b)` creates a temporary string; chain the appends instead: `sb.append(a).append(b)`.",
});

impl Rule for StringBuilderAppendConcat {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("append") {
            return;
        }
        let args = cx.fm.args_of(n);
        let Some(o) = n.field("object") else { return };
        if args.len() != 1 || !matches!(cx.type_of(o).class_name(), Some("java.lang.StringBuilder" | "java.lang.StringBuffer")) {
            return;
        }
        let a = args[0].unparen();
        if a.kind() == "binary_expression" && op(a) == "+" && cx.type_of(a).is_string() && cx.fm.constant_value(a).is_none() {
            cx.report(a, "Chain `append()` calls instead of concatenating inside `append()`");
        }
    }
}

declare_rule!(FormatWithoutArgs {
    id: "format-without-arguments",
    name: "String.format() without format specifiers",
    category: Style,
    severity: Info,
    sources: ["sonar:S3457"],
    kinds: ["method_invocation"],
    description: "Calling `String.format` with a constant that has no specifiers and no arguments just returns the string (after treating `%` specially). Use the literal.",
});

impl Rule for FormatWithoutArgs {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !cx.fm.call_matches(n, &FORMAT_STATIC) {
            return;
        }
        let Some((fmt, args)) = format_call(n, cx) else { return };
        let Some(f) = cx.fm.string_value(fmt) else { return };
        if args.is_empty() && !f.contains('%') {
            cx.report_fix(n, "`String.format` without specifiers or arguments; use the literal", Fix::replace("Use the literal", n, cx.text(fmt)));
        }
    }
}

declare_rule!(BoxedToStringConversion {
    id: "boxed-tostring",
    name: "Boxing just to call toString()/compare",
    category: Performance,
    severity: Info,
    sources: ["sonar:S2131", "spotbugs:DM_BOXED_PRIMITIVE_TOSTRING", "spotbugs:DM_BOXED_PRIMITIVE_FOR_COMPARE"],
    kinds: ["method_invocation"],
    description: "`Integer.valueOf(x).toString()` boxes needlessly; use `Integer.toString(x)` or `String.valueOf(x)`.",
});

impl Rule for BoxedToStringConversion {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = cx.fm.call_name(n) else { return };
        if name != "toString" && name != "compareTo" {
            return;
        }
        let Some(o) = n.field("object").map(|o| o.unparen()) else { return };
        let boxed_now = match o.kind() {
            "object_creation_expression" => cx.type_of(o).is_boxed(),
            "method_invocation" => cx.fm.call_name(o) == Some("valueOf") && cx.type_of(o).is_boxed() && cx.fm.args_of(o).first().is_some_and(|a| cx.type_of(*a).is_prim()),
            _ => false,
        };
        if boxed_now {
            let t = cx.type_of(o).display_simple();
            cx.report(n, format!("Boxing to call `{name}()`; use `{t}.{}(...)` on the primitive", if name == "toString" { "toString" } else { "compare" }));
        }
    }
}

declare_rule!(ParseVsValueOf {
    id: "valueof-to-primitive",
    name: "valueOf() used where parse...() suffices",
    category: Performance,
    severity: Info,
    sources: ["sonar:S2130", "spotbugs:DM_BOXED_PRIMITIVE_FOR_PARSING"],
    kinds: ["method_invocation"],
    description: "`int x = Integer.valueOf(s)` boxes and immediately unboxes; `Integer.parseInt(s)` returns the primitive directly.",
});

impl Rule for ParseVsValueOf {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("valueOf") {
            return;
        }
        let args = cx.fm.args_of(n);
        if args.is_empty() || !cx.type_of(args[0]).is_string() {
            return;
        }
        let t = cx.type_of(n);
        let Some(p) = t.unboxed() else { return };
        if !t.is_boxed() || p == Prim::Char {
            return;
        }
        if cx.fm.expected_type(n).prim() == Some(p) {
            let parse = match p {
                Prim::Int => "parseInt",
                Prim::Long => "parseLong",
                Prim::Double => "parseDouble",
                Prim::Float => "parseFloat",
                Prim::Short => "parseShort",
                Prim::Byte => "parseByte",
                Prim::Boolean => "parseBoolean",
                Prim::Char => return,
            };
            let name = n.field("name").unwrap();
            cx.report_fix(n, format!("Use `{}.{parse}()` to get a primitive without boxing", t.display_simple()), Fix::replace(format!("Use {parse}"), name, parse));
        }
    }
}

declare_rule!(UnnecessaryBoxing {
    id: "unnecessary-boxing",
    name: "Value boxed and immediately unboxed",
    category: Performance,
    severity: Info,
    sources: ["sonar:S2153", "pmd:UnnecessaryBoxing", "spotbugs:BX_BOXING_IMMEDIATELY_UNBOXED"],
    kinds: ["method_invocation"],
    description: "`Integer.valueOf(x).intValue()` boxes then unboxes; use the primitive directly.",
});

impl Rule for UnnecessaryBoxing {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(name) = cx.fm.call_name(n) else { return };
        if !name.ends_with("Value") || !cx.fm.args_of(n).is_empty() {
            return;
        }
        let Some(o) = n.field("object").map(|o| o.unparen()) else { return };
        let boxed_from_prim = match o.kind() {
            "object_creation_expression" | "method_invocation" => {
                cx.type_of(o).is_boxed() && (o.kind() == "object_creation_expression" || cx.fm.call_name(o) == Some("valueOf")) && cx.fm.args_of(o).first().is_some_and(|a| cx.type_of(*a).is_prim())
            }
            _ => false,
        };
        if boxed_from_prim {
            let inner = cx.fm.args_of(o)[0];
            cx.report(n, format!("`{}` is boxed and immediately unboxed", cx.text(inner)));
        }
    }
}

declare_rule!(PatternCompileInLoop {
    id: "regex-compiled-in-loop",
    name: "Regular expression compiled repeatedly in a loop",
    category: Performance,
    severity: Info,
    sources: ["sonar:S4248", "pmd:AvoidRecompilingPatterns"],
    kinds: ["method_invocation"],
    description: "`String.matches`, `replaceAll`, `split` (for multi-char patterns) and `Pattern.compile` with a constant regex recompile it on each iteration. Hoist a `static final Pattern`.",
});

impl Rule for PatternCompileInLoop {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !in_loop(n) {
            return;
        }
        let Some(name) = cx.fm.call_name(n) else { return };
        if !matches!(name, "matches" | "replaceAll" | "replaceFirst" | "compile") {
            return;
        }
        let Some(a) = regex_arg(n, cx) else { return };
        let Some(p) = cx.fm.string_value(a) else { return };
        if has_regex_meta(&p) || name == "compile" {
            cx.report(n, "Constant regex recompiled on every loop iteration; hoist a `static final Pattern`");
        }
    }
}

declare_rule!(SubstringToEnd {
    id: "redundant-substring",
    name: "Redundant substring() call",
    category: Style,
    severity: Info,
    sources: ["sonar:S4635", "intellij:SubstringZero"],
    kinds: ["method_invocation"],
    description: "`s.substring(0)` returns the same string and `s.substring(i, s.length())` equals `s.substring(i)`.",
});

impl Rule for SubstringToEnd {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) != Some("substring") {
            return;
        }
        let Some(o) = n.field("object") else { return };
        if !cx.type_of(o).is_string() {
            return;
        }
        let args = cx.fm.args_of(n);
        if args.len() == 1 && cx.fm.int_value(args[0]) == Some(0) {
            cx.report_fix(n, "`substring(0)` returns the same string", Fix::replace("Remove substring(0)", n, cx.text(o)));
        } else if args.len() == 2 {
            let end = args[1].unparen();
            if is_call_named(end, cx.src, &["length"]) && end.field("object").is_some_and(|e| normalized(e, cx.src) == normalized(o, cx.src)) {
                cx.report(n, "The end index is the string length; use `substring(begin)`");
            }
        }
    }
}
