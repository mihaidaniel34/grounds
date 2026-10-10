# grounds

A static analyzer for Java, written in Rust, meant to run after every edit a
coding agent makes. It reports bugs, compile breaks and Spring/Micronaut
wiring mistakes in changed code in about 0.1–0.3 seconds, without Gradle,
Maven or a JVM.

**Status: experimental.** It has been tested on five open-source codebases
(listed below) and nothing else. Expect false positives on code that looks different from
those, and unsupported setups. Bug reports with a code snippet are the most
useful thing you can send.

## Why

Agents tend to run `./gradlew build` to check their work. On the projects
that motivated this it takes 2–5 minutes and rarely reports anything new.
Skipping it is no better: problems then pile up until the test run at the
end. grounds sits between the two. After each edit it reports what the
compiler or a Sonar scan would have flagged in the lines that changed, and
leaves tests for the end of the task.

## What it checks

- **312 rules**, reimplementations of checks from SonarQube, SpotBugs, PMD,
  Error Prone and IntelliJ inspections. Each rule lists the equivalent IDs
  in those tools. By category: 109 bug, 58 style, 38 bad practice,
  23 performance, 23 concurrency, 22 security, 20 framework, 14 testing,
  5 complexity. Full list: [docs/RULES.md](docs/RULES.md).
- **Dataflow:** null dereferences (with `@Nullable` and null checks), unclosed
  resources, unchecked `Optional.get()`, dead stores. The control flow graph
  works at expression level and models `finally` and exceptions.
- **Compile breaks in project code:** calls to methods, constructors and
  fields that no longer exist, or whose argument types no longer match;
  `@Override` methods that override nothing; abstract methods left
  unimplemented. After an API change, every caller in other files that it
  broke is listed.
- **Dependency injection (Spring, Micronaut, JSR-330):** missing beans,
  ambiguous beans, qualifiers that match no bean, Lombok constructors
  dropping `@Qualifier`, constructor cycles, bean name clashes. It knows
  `@Primary`, profiles and conditions (treated as unknown), generics,
  factory methods, Spring Data repositories, Feign clients, MapStruct mappers
  and Micronaut's `@EachBean`, `@Replaces` and bean naming.
- **Lombok:** generated constructors, getters, setters, `@With` and builders
  are modeled, so code using them type-checks.

"Bean not found" is only reported for injection points whose type is
declared in the project. Beans created by library auto-configuration (a
`DataSource`, an `ObjectMapper`) are not modeled, so for those types
grounds stays silent instead of guessing.

## Numbers

Release build, Apple M4 Pro (14 cores), machine not idle. Each project as
checked out, with no grounds configuration:

| project | Java files | lines | modules | full check | calls resolved | errors / warnings / info |
|---|---:|---:|---:|---:|---:|---|
| spring-petclinic | 50 | 4,297 | 1 | 0.22 s | 96.6% | 0 / 16 / 7 |
| spring-petclinic-rest | 109 | 8,675 | 1 | 0.24 s | 91.6% | 0 / 29 / 22 |
| piggymetrics | 97 | 4,472 | 10 | 0.21 s | 87.4% | 0 / 17 / 61 |
| jhipster-sample-app | 136 | 11,980 | 1 | 0.27 s | 83.6% | 0 / 5 / 19 |
| micronaut-core | 6,373 | 697,624 | 85 | 2.9 s | 95.0% | 17 / 839 / 2,988 |

"Calls resolved" is the share of method calls for which grounds found the
exact target method. For the rest, any rule that needs the target says
nothing. The rate depends mostly on how many dependency jars it can find
(see Classpath).

Per-edit check through the daemon (the `post-edit` hook, including process
start):

| file | lines | time |
|---|---:|---:|
| petclinic `Owner.java` | 188 | 0.08–0.10 s |
| micronaut `HttpStatus.java` | 307 | 0.12 s |
| micronaut `PythonStubGenerator.java` | 6,869 | 0.32 s |

Loading micronaut-core (discovering files, reading jar indexes, extracting
declarations from all 6,373 files) takes about 0.9 s, with or without the
on-disk cache.

## How it was tested

- **One fixture per rule:** 315 small Java files with `// expect` markers on
  the lines that must be reported. The tests fail when a rule reports
  anything else, or when a rule has no fixture. Fixtures also cover
  suppressions and options.
- **End-to-end session test:** a temporary git repo goes through edits, the
  baseline, broken-caller detection and impacted tests.
- **Five unmodified open-source projects** (the table above), taken as
  published. Since they compile and their applications start, any
  compile-break or DI finding on them would be a false positive. There are
  none. Error-level findings were reviewed by hand. The remaining 17 on
  micronaut-core are real by each rule's definition: `TrustManager`s that
  trust every certificate (in test and TCK code), `split(File.separator)`,
  `compareTo(...) != -1` and `assertSame(x, x)`. Warnings were sampled, not
  reviewed one by one.
- **Planted bugs:** in a copy of spring-petclinic-rest, all of these were
  reported: an ambiguous bean (9 injection sites), a misspelled qualifier, a
  missing bean, a constructor cycle, and Lombok dropping `@Qualifier`.
  Renaming a method in petclinic listed all 15 broken callers. Changing
  `getPet(String)` to take two parameters flagged every call with the old
  arguments.
- **Precision fixes:** every false positive found during testing became a
  fixture case. Recent ones: `@Value` placeholders built by string
  concatenation; Micronaut introduction interfaces; MockMvc `andExpect` as
  an assertion; Spring's empty `contextLoads()` test; a trailing
  `// grounds-ignore` also silencing the next line.

Not tested: Windows (the daemon uses Unix sockets), Kotlin or mixed
Kotlin/Java projects (Kotlin sources are ignored, so types defined in them
are unknown), Android, Java modules (JPMS) beyond what the JDK uses.

## Install

```bash
cargo install --locked --path crates/grounds-cli
```

With Nix: `nix profile install .`, or `nix run . -- check` without installing.
`nix develop` provides Rust, clippy, rust-analyzer and a JDK for the tests.

grounds does not run Java, but it reads the JDK's API signatures (`ct.sym`).
It finds a JDK through `JAVA_HOME`, `/usr/libexec/java_home` or `java` on
PATH. Without one, most rules have little to work with.

## Usage

```bash
grounds check                 # whole project, from the project root
grounds changed               # changed lines since HEAD (git), baseline applied
grounds explain null-dereference
grounds beans --type MailSender   # beans of a type, and how each injection point of it resolves
grounds impacted-tests --gradle   # ./gradlew :module:test --tests ... for affected tests
```

Output is one finding per line, `path:line:col: severity [rule-id] message`.
It is colored when stdout is a terminal (`--color auto|always|never`,
`NO_COLOR`). `--format json|sarif` is available for tools.

### Claude Code hooks

`grounds hooks` prints the configuration to merge into
`.claude/settings.json` (per project, or `~/.claude/settings.json` for all
projects; outside Java projects the hooks do nothing).

| hook | runs | does |
|---|---|---|
| SessionStart | `grounds hook session-start` | records problems already in uncommitted work as a baseline and starts the daemon |
| PostToolUse (Edit/Write) | `grounds hook post-edit` | high-confidence rules on the edited file's changed lines, plus callers in other files broken by API changes in it |
| Stop | `grounds hook stop` | all rules on changed lines of all changed files, at most 15 findings; blocks the end of the turn once if anything new was found |

Findings reach the agent through exit code 2. Example after an edit that
renamed a method and added a bug:

```
grounds: 1 new problem(s) in changed code (1 errors, 0 warnings, 0 info)
src/main/java/.../Owner.java:102:31: error [null-dereference] `label` is always null here
grounds: API changes in src/main/java/.../Owner.java broke 15 call site(s) in 6 other file(s) not yet updated:
src/main/java/.../PetController.java:103:9: `Owner` has no method `addPet`
...
```

The daemon keeps one warm index per project over a Unix socket, re-reads
files whose modification time changed before every request, and exits
after 30 minutes idle. It is started on demand and replaced automatically
when the `grounds` binary changes.

### Suppressing findings

- On a line: `// grounds-ignore` or `// grounds-ignore: rule-id`.
- Existing markers are honored: `@SuppressWarnings` with grounds, Sonar
  (`java:S2259`), PMD (`PMD.RuleName`) or SpotBugs IDs; `// NOSONAR`;
  `// NOPMD`; `//noinspection IntelliJName`.
- `grounds baseline` accepts the current findings in changed files.
- Project-wide, in `grounds.toml`:

```toml
java_release = 21                     # which JDK API version to check against
exclude = ["**/generated/**"]
test_source_sets = ["scenarios"]      # besides test/integration/e2e/functional/...

[rules]
disable = ["field-injection", "style"]   # rule ids or categories
severity = { "todo-comment" = "warning" }

[rules.options.cognitive-complexity]
max = 20

[classpath]
entries = ["libs/vendor.jar"]
files = [".grounds/classpath.txt"]
scan_caches = true      # use the newest version of each artifact in ~/.gradle and ~/.m2
build_outputs = true    # module build/classes directories
```

### Classpath

By default grounds guesses dependency jars from the local Gradle and Maven
caches, which works when the project has been built on the machine before.
For an exact classpath, run once (and again when dependencies change):

```bash
grounds classpath
```

This runs Gradle (through an init script, without touching build files) or
Maven a single time, about 20–40 s. It writes `.grounds/classpath.txt` and,
for Gradle, which test task runs each source set. Set `scan_caches = false`
to use only that file. grounds never runs the build tool on its own.

## Design

- **Unknown means silent.** Anything that depends on a type grounds could
  not resolve is not reported. Missing jars lower recall, not precision.
- **Declarations eagerly, bodies lazily.** Every file is reduced to a
  declaration stub (signatures, annotations, constants), cached by content
  hash. Method bodies are analyzed only for the files being checked.
- **No build tool in the loop.** Modules, source sets and the classpath come
  from the directory layout and the local caches, or from the one-off dump.

```
grounds-syntax     tree-sitter Java parser, node helpers
grounds-classfile  jar/zip reader, class file and generic signature parser, JDK ct.sym
grounds-index      declaration stubs, Lombok model, project-wide index, hierarchy lookup
grounds-semantic   per-file model: name binding, expression types, overload resolution
                   with type inference, constant folding
grounds-flow       control flow graph, dataflow solver, nullness/resource/Optional analyses
grounds-di         bean model for Spring, Micronaut and JSR-330
grounds-rules      rule API (metadata, dispatch, suppressions, fixes) and the rules
grounds-engine     workspace discovery, classpath, git, sessions, baseline, daemon, output
grounds-cli        the `grounds` binary
```

About 27,000 lines of Rust. Most of it was written by Claude (Anthropic's
model) through Claude Code, over roughly a day of agent time.

### Adding a rule

```rust
declare_rule!(StringReferenceEquality {
    id: "string-reference-equality",
    name: "Strings compared with == or !=",
    category: Bug,
    severity: Warning,
    tier: Edit,   // also run on every edit, not only at the end of a turn
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

Add it to the `rules()` list of its module (here
`crates/grounds-rules/src/rules/equality.rs`) and add
`crates/grounds-rules/tests/fixtures/<rule-id>.java`, marking the lines that
must be reported with `// expect`. Then run `cargo test -p grounds-rules`.

## License

Apache-2.0, see [LICENSE](LICENSE).

SonarQube, SpotBugs, PMD, Error Prone and IntelliJ IDEA belong to their
respective owners. grounds is not affiliated with any of them. Rule IDs from
those tools appear only as cross-references; no code or rule descriptions
were copied.
