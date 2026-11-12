// The violations of `tests/ablation.rs` known today: findings that appear
// only when information is hidden from the analyzer. One entry per
// (ablation, rule, fixture stem, line), sorted. Delete an entry when a fix
// removes it; regenerate the whole list with
// `GROUNDS_ABLATION_BLESS=1 cargo test -p grounds-rules --test ablation`.
// (cargo also builds this file as a test of its own, with no tests in it.)

#[allow(dead_code)]
#[rustfmt::skip]
const BASELINE: &[(&str, &str, &str, usize)] = &[
    ("no-file:org/apache/commons/lang3/StringUtils.java", "null-dereference", "null-dereference", 156),
    ("no-jdk", "lost-stack-trace", "lost-stack-trace", 45),
    ("no-jdk", "lost-stack-trace", "lost-stack-trace", 63),
    ("no-jdk", "lost-stack-trace", "lost-stack-trace", 75),
    ("no-jdk", "null-dereference", "null-dereference__control-flow", 49),
    ("no-jdk", "print-stack-trace", "print-stack-trace", 10),
    ("no-jdk", "print-stack-trace", "print-stack-trace", 11),
    ("no-libraries", "null-dereference", "null-dereference", 156),
    ("no-package:org.apache.commons.lang3", "null-dereference", "null-dereference", 156),
    ("nothing", "lost-stack-trace", "lost-stack-trace", 45),
    ("nothing", "lost-stack-trace", "lost-stack-trace", 63),
    ("nothing", "lost-stack-trace", "lost-stack-trace", 75),
    ("nothing", "null-dereference", "null-dereference__control-flow", 49),
    ("nothing", "print-stack-trace", "print-stack-trace", 10),
    ("nothing", "print-stack-trace", "print-stack-trace", 11),
];
