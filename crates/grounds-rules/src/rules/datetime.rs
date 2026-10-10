//! Date and time API pitfalls.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(WeekYearPattern),
        Box::new(DayOfYearPattern),
        Box::new(HourWithoutAmPm),
        Box::new(MinuteMonthConfusion),
        Box::new(InvalidDateValue),
        Box::new(InstantUnsupportedUnit),
        Box::new(InvalidTimeZone),
        Box::new(LegacyDateApi),
    ]
}

/// The constant date pattern passed to a formatter, if any.
fn date_pattern<'t>(n: Node<'t>, cx: &Ctx<'_, 't>) -> Option<(Node<'t>, String)> {
    let is_pattern_call = match n.kind() {
        "object_creation_expression" => cx.type_of(n).is_class("java.text.SimpleDateFormat"),
        "method_invocation" => {
            let name = cx.fm.call_name(n)?;
            (name == "ofPattern" && cx.fm.select_method(n).is_some_and(|c| &*c.owner == "java.time.format.DateTimeFormatter"))
                || (matches!(name, "applyPattern" | "applyLocalizedPattern") && n.field("object").is_some_and(|o| cx.type_of(o).is_class("java.text.SimpleDateFormat")))
                || (name == "appendPattern" && n.field("object").is_some_and(|o| cx.type_of(o).is_class("java.time.format.DateTimeFormatterBuilder")))
        }
        _ => false,
    };
    if !is_pattern_call {
        return None;
    }
    let a = *cx.fm.args_of(n).first()?;
    Some((a, cx.fm.string_value(a)?))
}

/// Pattern letters outside quoted literals.
fn pattern_letters(p: &str) -> String {
    let mut out = String::new();
    let mut quoted = false;
    for c in p.chars() {
        if c == '\'' {
            quoted = !quoted;
            out.push(' ');
            continue;
        }
        out.push(if quoted { ' ' } else { c });
    }
    out
}

declare_rule!(WeekYearPattern {
    id: "week-year-in-date-pattern",
    name: "Week-based year (YYYY) used in a date pattern",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S3986", "errorprone:MisusedWeekYear", "spotbugs:FS_BAD_DATE_FORMAT_FLAG_COMBO"],
    kinds: ["object_creation_expression", "method_invocation"],
    description: "`YYYY` is the week-based year: around New Year, `2025-12-29` formats as `2026-12-29`. Use `yyyy` (or `uuuu` with java.time) for calendar years.",
});

impl Rule for WeekYearPattern {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((a, p)) = date_pattern(n, cx) else { return };
        let l = pattern_letters(&p);
        if l.contains("YY") && !l.contains('w') && (l.contains('M') || l.contains('d')) {
            cx.report(a, "`YYYY` is the week-based year; use `yyyy` for the calendar year");
        }
    }
}

declare_rule!(DayOfYearPattern {
    id: "day-of-year-in-date-pattern",
    name: "Day of year (DD) used with month",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["errorprone:MisusedDayOfYear", "spotbugs:FS_BAD_DATE_FORMAT_FLAG_COMBO"],
    kinds: ["object_creation_expression", "method_invocation"],
    description: "`DD` is the day of the year (001-366), so `yyyy-MM-DD` prints `2025-02-032`. Use `dd` for the day of the month.",
});

impl Rule for DayOfYearPattern {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((a, p)) = date_pattern(n, cx) else { return };
        let l = pattern_letters(&p);
        if l.contains("DD") && l.contains('M') {
            cx.report(a, "`DD` is the day of the year; use `dd` for the day of the month");
        }
    }
}

declare_rule!(HourWithoutAmPm {
    id: "twelve-hour-without-am-pm",
    name: "12-hour clock without AM/PM marker",
    category: Bug,
    severity: Warning,
    sources: ["errorprone:MisusedHourOfDay", "spotbugs:FS_BAD_DATE_FORMAT_FLAG_COMBO"],
    kinds: ["object_creation_expression", "method_invocation"],
    description: "`hh` is the 1-12 hour; without an `a` marker 01:00 and 13:00 print the same. Use `HH` for 0-23 or add `a`.",
});

impl Rule for HourWithoutAmPm {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((a, p)) = date_pattern(n, cx) else { return };
        let l = pattern_letters(&p);
        if l.contains("hh") && !l.contains('a') && !l.contains('B') {
            cx.report(a, "`hh` without `a` is ambiguous; use `HH` for a 24-hour clock");
        }
    }
}

declare_rule!(MinuteMonthConfusion {
    id: "minute-month-confusion",
    name: "Minutes (mm) used where months (MM) were likely intended",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["errorprone:MisusedDateFormat", "sonar:S3986"],
    kinds: ["object_creation_expression", "method_invocation"],
    description: "`mm` is minutes and `MM` months: `yyyy-mm-dd` prints the minute in place of the month.",
});

impl Rule for MinuteMonthConfusion {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((a, p)) = date_pattern(n, cx) else { return };
        let l = pattern_letters(&p);
        let has_hours = l.contains('H') || l.contains('h') || l.contains('k') || l.contains('K');
        if l.contains("mm") && !has_hours && !l.contains('M') && (l.contains("yy") || l.contains("dd")) {
            cx.report(a, "`mm` is minutes; use `MM` for the month");
        }
    }
}

declare_rule!(InvalidDateValue {
    id: "invalid-date-value",
    name: "Date/time value out of range",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["sonar:S2110", "spotbugs:DMI_BAD_MONTH"],
    kinds: ["method_invocation"],
    description: "java.time months are 1-12 (Calendar months 0-11), days 1-31, hours 0-23, minutes/seconds 0-59. Out-of-range constants throw DateTimeException (java.time) or silently roll over (Calendar).",
});

impl Rule for InvalidDateValue {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = cx.fm.select_method(n) else { return };
        let args = cx.fm.args_of(n);
        let v = |i: usize| args.get(i).and_then(|a| cx.fm.int_value(*a));
        let name = &*c.method.name;
        let mut bad: Option<String> = None;
        let check = |val: Option<i64>, lo: i64, hi: i64, what: &str| -> Option<String> {
            val.filter(|x| *x < lo || *x > hi).map(|x| format!("{what} {x} is outside {lo}..{hi}"))
        };
        match (&*c.owner, name) {
            ("java.time.LocalDate", "of") | ("java.time.LocalDateTime", "of") | ("java.time.ZonedDateTime", "of") | ("java.time.OffsetDateTime", "of") => {
                // only the int-month overloads
                if c.method.params.get(1).is_some_and(|p| p.ty.prim() == Some(Prim::Int)) {
                    bad = check(v(1), 1, 12, "month").or_else(|| check(v(2), 1, 31, "day"));
                    if bad.is_none() && args.len() >= 5 {
                        bad = check(v(3), 0, 23, "hour").or_else(|| check(v(4), 0, 59, "minute"));
                    }
                }
            }
            ("java.time.LocalTime", "of") => bad = check(v(0), 0, 23, "hour").or_else(|| check(v(1), 0, 59, "minute")).or_else(|| check(v(2), 0, 59, "second")),
            ("java.time.YearMonth", "of") | ("java.time.MonthDay", "of") => {
                let mi = if &*c.owner == "java.time.YearMonth" { 1 } else { 0 };
                if c.method.params.get(mi).is_some_and(|p| p.ty.prim() == Some(Prim::Int)) {
                    bad = check(v(mi), 1, 12, "month");
                }
            }
            ("java.time.Month", "of") => bad = check(v(0), 1, 12, "month"),
            ("java.time.DayOfWeek", "of") => bad = check(v(0), 1, 7, "day of week"),
            (_, "set") if cx.fm.is_subclass(&c.owner, "java.util.Calendar").is_yes() && args.len() == 2 => {
                let field = cx.text(args[0]);
                bad = match field.rsplit('.').next() {
                    Some("MONTH") => check(v(1), 0, 11, "Calendar month (0-based)"),
                    Some("HOUR_OF_DAY") => check(v(1), 0, 23, "hour"),
                    Some("MINUTE") | Some("SECOND") => check(v(1), 0, 59, "minute/second"),
                    Some("DAY_OF_MONTH") => check(v(1), 1, 31, "day"),
                    _ => None,
                };
            }
            _ => {}
        }
        if let Some(b) = bad {
            cx.report(n, b);
        }
    }
}

declare_rule!(InstantUnsupportedUnit {
    id: "instant-unsupported-unit",
    name: "Instant arithmetic with months/years",
    category: Bug,
    severity: Error,
    tier: Edit,
    sources: ["errorprone:JavaInstantGetSecondsGetNano", "errorprone:JavaTimeDefaultTimeZone"],
    kinds: ["method_invocation"],
    description: "`Instant` has no calendar: `plus(1, ChronoUnit.MONTHS)` (or WEEKS/YEARS...) throws UnsupportedTemporalTypeException. Convert to `ZonedDateTime` first or use days.",
});

impl Rule for InstantUnsupportedUnit {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !matches!(cx.fm.call_name(n), Some("plus" | "minus" | "truncatedTo" | "until")) {
            return;
        }
        let Some(o) = n.field("object") else { return };
        if !cx.type_of(o).is_class("java.time.Instant") {
            return;
        }
        for a in cx.fm.args_of(n) {
            let t = cx.text(a);
            if ["WEEKS", "MONTHS", "YEARS", "DECADES", "CENTURIES", "MILLENNIA", "ERAS"].iter().any(|u| t.ends_with(u)) {
                cx.report(a, format!("Instant does not support `{}`; this throws UnsupportedTemporalTypeException", t.rsplit('.').next().unwrap_or(t)));
            }
        }
    }
}

const TZ_AREAS: &[&str] = &["Africa", "America", "Antarctica", "Arctic", "Asia", "Atlantic", "Australia", "Europe", "Indian", "Pacific", "Etc", "Brazil", "Canada", "Chile", "Mexico", "US"];

declare_rule!(InvalidTimeZone {
    id: "invalid-time-zone-id",
    name: "Unrecognized time zone id",
    category: Bug,
    severity: Warning,
    sources: ["errorprone:InvalidTimeZoneID", "errorprone:InvalidZoneId"],
    kinds: ["method_invocation"],
    description: "`TimeZone.getTimeZone` silently falls back to GMT for unknown ids (e.g. `America/NewYork`), and `ZoneId.of` throws. Use a valid IANA id like `America/New_York`.",
});

impl Rule for InvalidTimeZone {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = cx.fm.select_method(n) else { return };
        let ok_owner = (&*c.owner == "java.util.TimeZone" && &*c.method.name == "getTimeZone") || (&*c.owner == "java.time.ZoneId" && &*c.method.name == "of");
        if !ok_owner {
            return;
        }
        let Some(a) = cx.fm.args_of(n).first().copied() else { return };
        if !cx.type_of(a).is_string() {
            return;
        }
        let Some(id) = cx.fm.string_value(a) else { return };
        let valid = match id.split_once('/') {
            Some((area, city)) => TZ_AREAS.contains(&area) && !city.is_empty() && !city.contains(' ') && city.chars().next().is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
            None => {
                matches!(id.as_str(), "UTC" | "GMT" | "Z" | "UT" | "CET" | "EET" | "MET" | "WET" | "EST" | "MST" | "HST" | "EST5EDT" | "CST6CDT" | "MST7MDT" | "PST8PDT" | "PST" | "CST" | "EDT" | "PDT" | "JST" | "IST" | "CTT" | "Japan" | "Singapore" | "Turkey" | "Egypt" | "Eire" | "Hongkong" | "Iceland" | "Iran" | "Israel" | "Jamaica" | "Libya" | "Navajo" | "NZ" | "Poland" | "Portugal" | "PRC" | "ROK" | "Zulu" | "GB" | "Cuba" | "Greenwich" | "Universal" | "UCT")
                    || id.starts_with("GMT+")
                    || id.starts_with("GMT-")
                    || id.starts_with("UTC+")
                    || id.starts_with("UTC-")
                    || id.starts_with('+')
                    || id.starts_with('-')
            }
        };
        if !valid {
            cx.report(a, format!("`{id}` is not a recognized time zone id"));
        }
    }
}

declare_rule!(LegacyDateApi {
    id: "legacy-date-api",
    name: "java.util.Date/Calendar instead of java.time",
    category: BadPractice,
    severity: Info,
    enabled: false,
    sources: ["sonar:S2143"],
    kinds: ["object_creation_expression"],
    description: "`Date` and `Calendar` are mutable, not thread-safe and have confusing APIs (0-based months, years since 1900). Use java.time (`Instant`, `LocalDate`, `ZonedDateTime`).",
});

impl Rule for LegacyDateApi {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let t = cx.type_of(n);
        if matches!(t.class_name(), Some("java.util.Date" | "java.util.GregorianCalendar")) && n.kid_of_kind("class_body").is_none() {
            let _ = is_literal(n);
            cx.report(n, format!("Use java.time instead of `{}`", t.display_simple()));
        }
    }
}
