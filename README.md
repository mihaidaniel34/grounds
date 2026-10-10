# grounds

Fast, incremental static analysis for Java, built for coding agents.

Running `./gradlew build` after every agent edit costs minutes and almost never
says anything new. `grounds` gives the useful part of that signal (does the
change break callers, did it introduce a bug) in milliseconds, without
Gradle, Maven or a JVM:

- **312 rules** ported from SonarQube, SpotBugs, PMD, Error Prone and IntelliJ
  inspections: equality and numeric bugs, format strings and regexes, null
  dereferences, resource leaks, concurrency, exceptions, API misuse, security,
  date/time, tests (JUnit/AssertJ/Mockito), design and style
  ([catalog](docs/RULES.md)).
- **Dependency-injection model** for Spring, Micronaut and JSR-330: missing
  beans, ambiguous beans, qualifiers that match nothing, Lombok constructors
  dropping `@Qualifier`, circular constructor dependencies, bean name
  clashes, `@Transactional` self-invocation, path-variable mismatches.
- **Compile-break detection** for project code: calls, constructors and
  fields that no longer exist, `@Override` methods that override nothing,
  unimplemented abstract methods.
- **Agent loop**: per-edit and end-of-turn checks restricted to changed lines,
  a baseline for pre-existing problems, a list of callers broken by API
  changes elsewhere, and the tests affected by a change.

On micronaut-core (6,000 files, 85 modules): cold project load ~0.9 s,
warm ~0.4 s, all rules on the whole project ~1.8 s, a per-edit check of a
5,000-line file through the daemon ~0.3 s.

## Quick start

```bash
cargo build --release
./target/release/grounds check --root path/to/project            # whole project
./target/release/grounds changed                                   # only what you changed (git)
./target/release/grounds explain null-dereference
```

### Claude Code hooks

```bash
grounds hooks   # prints the JSON to merge into .claude/settings.json
```

| hook | command | what it does |
|---|---|---|
| SessionStart | `grounds hook session-start` | baselines problems already present in uncommitted work, starts the daemon |
| PostToolUse (Edit/Write) | `grounds hook post-edit` | high-confidence rules on the edited file's changed lines, plus callers in other files broken by API changes in it |
| Stop | `grounds hook stop` | all rules on changed lines of all changed files, capped at 15 findings; blocks the turn once if anything new was found |

Hooks exit with code 2 to feed findings back to the model, and do nothing
outside Java projects. Example post-edit feedback after renaming a method:

```
grounds: 1 new problem(s) in changed code (1 errors, 0 warnings, 0 info)
src/main/java/.../Owner.java:102:31: error [null-dereference] `label` is always null here
grounds: API changes in src/main/java/.../Owner.java broke 15 call site(s) in 6 other file(s) not yet updated:
src/main/java/.../PetController.java:103:9: `Owner` has no method `addPet`
...
```

## Commands

| command | purpose |
|---|---|
| `grounds check [paths] [--format text\|json\|sarif] [--rules ids] [--stats]` | batch analysis |
| `grounds changed [files] [--edit] [--all-lines] [--no-baseline] [--json]` | agent-style check of changed code |
| `grounds baseline [files] [--merge]` | accept current findings (default: changed files) |
| `grounds impacted-tests [files] [--gradle]` | tests reaching the changed classes; `--gradle` prints `./gradlew :module:test --tests ...` |
| `grounds classpath [--gradle\|--maven]` | one-off dump of the exact compile/test classpath into `.grounds/classpath.txt` (re-run when dependencies change) |
| `grounds beans [--type T]` | the bean model: beans, injection points and how each resolves |
| `grounds daemon [--stop\|--status] [--idle-minutes N]` | warm analysis server (started automatically by hooks) |
| `grounds rules [--json\|--markdown]`, `grounds explain <id>` | rule catalog |

Suppress a finding with `@SuppressWarnings("rule-id")` (Sonar `java:S1234`,
`PMD.Rule` and SpotBugs pattern names work too), `// NOSONAR`,
`//noinspection IntelliJName` or `// grounds-ignore[: rule-id]`.

## Configuration (`grounds.toml`)

```toml
java_release = 21
exclude = ["**/generated/**"]

[classpath]
entries = ["libs/vendor.jar"]
files = [".grounds/classpath.txt"]   # one path per line or ':'-separated
scan_caches = true                   # newest jar per artifact from ~/.gradle and ~/.m2
build_outputs = true                 # module build/classes dirs

[rules]
disable = ["field-injection", "style"]   # ids or categories
severity = { "todo-comment" = "warning" }
[rules.options.cognitive-complexity]
max = 20
```

The classpath is best-effort by design: a type that cannot be resolved makes
the analysis silent rather than wrong. For exact results, run
`grounds classpath` once (it runs Gradle or Maven a single time, ~20-40 s)
and set `scan_caches = false`; nothing invokes the build tool afterwards.

## Architecture

```
grounds-syntax     tree-sitter Java, byte-preserving grammar fixups, node helpers
grounds-classfile  own jar/zip reader, class file + generic signature parser, JDK ct.sym
grounds-index      declaration stubs (cached by content hash), Lombok model, implicit members,
                   lazily resolved whole-program index, hierarchy & member lookup
grounds-semantic   per-file model: local binder, symbols, expression types, overload
                   resolution with inference, target typing, constant folding, matchers
grounds-flow       expression-level CFG (finally duplicated per completion kind),
                   generic dataflow solver, nullness / liveness / resources / Optional
grounds-di         framework-neutral bean model with Spring, Micronaut, JSR-330 discovery
grounds-rules      rule API (metadata, node-kind dispatch, shared flow cache, fixes,
                   suppressions) and the rule catalog with fixture tests
grounds-engine     workspace discovery, classpath providers, analysis runs, fingerprints,
                   git integration, sessions, baseline, daemon, reports (text/JSON/SARIF)
grounds-cli        the `grounds` binary
```

Design principles:

- **Precision over recall.** Anything depending on an unknown type is never
  reported; framework rules only judge what they can see completely (e.g.
  "bean not found" only for project types).
- **Declarations are cheap, bodies are lazy.** All files are reduced to stubs
  (signatures only); method bodies are analyzed only for files being checked.
- **Read your writes.** Every daemon request re-syncs files whose modification
  time changed before analyzing.

## Writing a rule

```rust
declare_rule!(StringReferenceEquality {
    id: "string-reference-equality",
    name: "Strings compared with == or !=",
    category: Bug,
    severity: Warning,
    tier: Edit,
    sources: ["sonar:S4973", "spotbugs:ES_COMPARING_STRINGS_WITH_EQ"],
    kinds: ["binary_expression"],
    description: "...",
});

impl Rule for StringReferenceEquality {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some((l, r)) = lr(n) else { return };
        if matches!(op(n), "==" | "!=") && cx.type_of(l).is_string() && cx.type_of(r).is_string() {
            cx.report(n, "Strings compared with `==`");
        }
    }
}
```

Add `crates/grounds-rules/tests/fixtures/<rule-id>.java` with `// expect`
markers on the lines that must be reported; `cargo test -p grounds-rules`
checks every fixture and that every rule has one.
