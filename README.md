# grounds

A static analyzer for Java, written in Rust, meant to run after every edit a
coding agent makes. It reports bugs, compile breaks and Spring/Micronaut
wiring mistakes in changed code in about 0.1–0.3 seconds, without Gradle,
Maven or a JVM.

**Status: experimental.** It has been tested on twenty-one open-source codebases
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
  5 complexity. Full list: [docs/RULES.md](docs/RULES.md). Style and
  security rules and the null-dereference check do not run on test
  sources: tests are there to run, and they use self-signed certificates
  or a null pointer that just fails the test on purpose.
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
  and Micronaut's `@EachBean`, `@Replaces` and bean naming. A Spring
  library (no `@SpringBootApplication` in its main sources, nor a
  `@ComponentScan` next to a `main` method; auto-configurations scanning
  their own packages count as library code) is not judged: the
  application using it wires its beans.
- **Lombok:** generated constructors (including `staticName`/`staticConstructor`
  factories), getters, setters, `@With`, `@Builder` on classes, constructors
  and methods (with `@Builder.Default`), `@SuperBuilder` across files,
  `@UtilityClass`, and the `lombok.config` keys that change signatures
  (fluent/chained accessors, prefixes, copyable annotations).

"Bean not found" is only reported for injection points whose type is
declared in the project. Beans created by library auto-configuration (a
`DataSource`, an `ObjectMapper`) are not modeled, so for those types
grounds stays silent instead of guessing.

## Numbers

Release build, Apple M4 Pro (14 cores), machine not idle. Each project at
the commit pinned in [corpus/projects.tsv](corpus/projects.tsv), with no
grounds configuration:

| project | Java files | lines | modules | full check | calls resolved | errors / warnings / info |
|---|---:|---:|---:|---:|---:|---|
| spring-petclinic | 50 | 4,297 | 1 | 0.22 s | 99.8% | 0 / 3 / 5 |
| spring-petclinic-rest | 109 | 8,675 | 1 | 0.25 s | 92.8% | 0 / 26 / 11 |
| piggymetrics | 97 | 4,472 | 10 | 0.22 s | 87.4% | 0 / 14 / 56 |
| jhipster-sample-app | 136 | 11,980 | 1 | 0.28 s | 86.4% | 0 / 2 / 11 |
| micronaut-core | 6,371 | 697,519 | 85 | 3.02 s | 94.9% | 10 / 454 / 2,666 |
| buckpal | 46 | 1,992 | 1 | 0.21 s | 90.3% | 0 / 3 / 2 |
| realworld-springboot-java | 89 | 4,638 | 1 | 0.22 s | 98.2% | 3 / 0 / 10 |
| eladmin | 274 | 23,195 | 6 | 0.29 s | 88.3% | 12 / 48 / 101 |
| start.spring.io | 377 | 23,857 | 5 | 0.29 s | 54.4% | 0 / 1 / 15 |
| mall | 524 | 85,204 | 8 | 0.52 s | 96.3% | 0 / 36 / 801 |
| halo | 1,381 | 134,807 | 7 | 0.94 s | 90.1% | 1 / 272 / 499 |
| dolphinscheduler | 2,560 | 251,783 | 145 | 1.26 s | 92.7% | 10 / 436 / 1,789 |
| thingsboard | 6,884 | 776,052 | 111 | 4.94 s | 88.3% | 6 / 1,117 / 8,156 |
| apollo | 756 | 98,135 | 15 | 0.64 s | 88.0% | 1 / 130 / 265 |
| camunda | 15,743 | 2,299,131 | 149 | 10.26 s | 87.7% | 9 / 1,322 / 6,316 |
| helidon | 6,325 | 1,037,072 | 522 | 4.35 s | 91.9% | 4 / 1,480 / 4,471 |
| langchain4j | 3,632 | 516,267 | 120 | 2.49 s | 91.2% | 11 / 537 / 2,856 |
| nacos | 5,293 | 813,767 | 63 | 3.50 s | 98.7% | 13 / 1,252 / 4,613 |
| rewrite | 3,764 | 899,832 | 36 | 4.11 s | 97.5% | 11 / 873 / 5,117 |
| spring-ai | 2,309 | 403,772 | 169 | 2.18 s | 83.3% | 3 / 264 / 1,542 |
| spring-modulith | 704 | 67,972 | 49 | 0.44 s | 86.5% | 5 / 46 / 418 |

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

- **Fixtures:** 361 small Java files, at least one per rule, with `// expect` markers on
  the lines that must be reported. The tests fail when a rule reports
  anything else, or when a rule has no fixture. Fixtures also cover
  suppressions and options.
- **Engine tests:** temporary git repositories go through edits, the
  baseline, broken-caller detection and impacted tests; multi-package
  projects exercise Spring bean registration (`@Import`, scan boundaries).
- **Twenty-one unmodified open-source projects** (the table above), pinned
  to fixed commits in [corpus/projects.tsv](corpus/projects.tsv). Since they
  compile and their applications start, any compile-break or DI finding on
  them would be a false positive. There are none. `scripts/corpus.sh check`
  runs all of them in about a minute and fails on a parse error, on any
  compile-break or DI finding, or when a planted bug (below) is no longer
  reported; every other change in the findings is shown as a diff against
  the last accepted baseline. Error-level findings were reviewed by hand. On the eight newer
  projects they include genuine bugs: an `int` overflow in
  `1000 * 60 * 60 * 24 * 365 * 10`, `String.format` with SLF4J `{}`
  placeholders, `assertThat(x > 0)` without `.isTrue()`, DES with a constant
  IV. The remaining 12 on micronaut-core are real by each rule's definition: `TrustManager`s that
  trust every certificate (in test and TCK code), `compareTo(...) != -1`
  and `assertSame(x, x)`. (An earlier version of this list also counted five
  `split(File.pathSeparator)` calls, which are fine; a reviewer caught it.)
  On the last eight projects added (apollo, camunda, helidon, langchain4j,
  nacos, rewrite, spring-ai, spring-modulith) all error-level findings were
  reviewed too. The first pass found 273 false positives in 12 patterns, all
  fixed with fixtures. Two examples are custom `@…Mapping` annotations and a
  lambda argument that selects another overload. What is left includes
  genuine bugs:
  - two methods in rewrite that call themselves unconditionally;
  - an unsynchronized `wait()` in nacos whose exception is swallowed;
  - a `UserInfo` kept in a `HashSet` with `equals` but no `hashCode`;
  - a `private` `@ParameterizedTest` that never runs;
  - about twenty AssertJ `assertThat(...)` calls that check nothing.

  The remaining error-level findings are trust-all `TrustManager`s and
  similar in test code.
- **Warnings:** not reviewed one by one. Six findings per rule were triaged
  for the 62 warning rules with at least ten findings on the corpus.
  - Rules that were usually right but rarely worth acting on moved to info
    (15, for example `float-equality`, `system-exit`, `octal-literal`).
  - The false-positive patterns found were fixed. The biggest were in the
    null and resource dataflow:
    - ternary guards;
    - null checks stored in a boolean;
    - `close()` inside its own try/catch;
    - exceptions from a try-with-resources skipping an outer `finally`.
  - This removed about 1,500 warnings, among them 600 unchecked
    `Optional.get()` reports behind assertion guards.
- **Planted bugs:** in a copy of spring-petclinic-rest
  ([the patch](corpus/planted/spring-petclinic-rest.patch)), all of these are
  reported: an ambiguous bean (9 injection sites), a misspelled qualifier, a
  missing bean, a constructor cycle, and Lombok dropping `@Qualifier`.
  Renaming a method in petclinic listed all 15 broken callers. Changing
  `getPet(String)` to take two parameters flagged every call with the old
  arguments.
- **Fuzzing against javac:** on every pull request, deterministic seeds;
  known cases are listed in a baseline, and only new ones fail. 8,000
  overload sets (`scripts/fuzz_overloads.py`) and 8,000 calls in context
  (`scripts/fuzz_calls.py`: typed targets, method arguments, lambda returns,
  nested generic calls, lambdas and method references, generic callers, JDK
  generic methods, inherited overloads; javac's own resolution notes give the
  expected type) catch any new case of grounds being certain of a type javac
  disagrees with. 8,000 files of member access (`scripts/fuzz_access.py`:
  private and nested access, `super`/`I.super`, inner, anonymous and local
  classes, interface defaults and statics, static imports, method
  references, records, enums, overrides) check that no "won't compile" rule
  reports anything on a file javac accepts. Every night all three also run
  on new seeds (the date), so the gate is not tuned to fixed cases.
- **External review:** three rounds of review by OpenAI Codex (gpt-6.1-sol)
  on the shared analyses and engine, and several by gpt-6-astra on the rules
  and on the fixes from the corpus and PMD rounds,
  each writing Java test cases, compiling them with javac and running Spring,
  AssertJ or Mockito to confirm every report. The first round found 16 defects (among
  them exception paths that saw assignments made after the throwing call,
  `break` skipping `finally`, `int` overflow in constant folding, `@Import`ed
  configurations ignored, a daemon that one stalled client could block, and
  a crash on malformed jars); the second found 9 more in the fixes, and the
  third 4 smaller edge cases in Spring bean naming and registration. The
  rules review found 24 false positives across 25 rules (for example
  `ArrayList.equals(LinkedList)` reported as always false, `LocalDate.parse(s)`
  used for validation reported as an ignored result, `assertThatThrownBy(..)`
  reported as asserting nothing). All are fixed, each with a regression test.
- **Newer Spring projects:** the first run on the eight projects added last
  (all updated within weeks) produced over 1,400 compile-break and DI false
  positives. Causes: Lombok features not yet modeled (`@SuperBuilder`,
  `@UtilityClass`, `@Builder` on constructors, `lombok.config` fluent
  accessors and copyable `@Lazy`), classpath skew (the newest cached jars are
  Spring Boot 3 / `jakarta.*` while a project is on Boot 2 / `javax.*`),
  MyBatis `@MapperScan`, configurations from annotations whose jar is
  missing, inherited `@Bean` methods, prototype beans, and overloads whose
  parameter types are unknown. All fixed; each pattern has a multi-file test.
- **Comparison with PMD:** `scripts/pmd_compare.py` runs PMD 7 (source
  only, no build) with exactly the PMD rules that grounds rules cite, and
  matches findings by file and line. On the 21 projects, the disagreements
  were triaged by sampling. They turned up three grounds bugs:
  - `builder.count` read from an enclosing class was resolved as the outer
    `count`, giving 1,524 false "unused private field" reports;
  - deeply nested Hamcrest matchers made expression typing exponential (one
    helidon test hung);
  - Javadoc links wrapped across lines left their imports looking unused.

  It also found a dozen recall gaps and eight rules citing the wrong PMD rule.
  Most remaining PMD-only findings are PMD lacking a classpath (on-demand
  imports, supertypes in other files) or scope differences (grounds skips
  test code and commented empty blocks on purpose).
- **Precision fixes:** every false positive found during testing became a
  fixture case, for example `@Value` placeholders built by string
  concatenation, Micronaut introduction interfaces, MockMvc `andExpect` as
  an assertion, and Spring's empty `contextLoads()` test.

- **Linux:** on NixOS (x86_64), the static release binary, a native Nix
  build and the macOS binary report identical findings, fingerprints
  included (6,511 findings over spring-petclinic-rest, halo and rewrite),
  so baselines carry over between systems. The test suite runs on Linux
  and macOS for every pull request.
- **CI mode:** 107 pull-request-shaped changes over 8 corpus projects
  (renames, deletions, signature changes, new and deleted beans, plus
  changes that break nothing): no false reports. Known gaps: a deleted
  class, a method made private, static imports, method references and
  return types assigned to variables are not yet checked across files.

Not tested: Windows (the daemon uses Unix sockets), Kotlin or mixed
Kotlin/Java projects (Kotlin sources are ignored, so types defined in them
are unknown), Android, Java modules (JPMS) beyond what the JDK uses.

## Install

Prebuilt binaries on the
[releases page](https://github.com/mihaidaniel34/grounds/releases): macOS
(Apple Silicon and Intel, 11+) and Linux (x86_64 and arm64, statically
linked, so any distribution works, Alpine and NixOS included). Pick the
target for your machine:

```bash
target=aarch64-apple-darwin   # x86_64-apple-darwin, x86_64-unknown-linux-musl, aarch64-unknown-linux-musl
curl -fsSL https://github.com/mihaidaniel34/grounds/releases/download/v0.0.8/grounds-0.0.8-$target.tar.gz | tar -xz
sudo mv grounds-0.0.8-$target/grounds /usr/local/bin/
```

Each archive has a `.sha256` next to it. The macOS binaries are not
notarized: if you download one with a browser, macOS quarantines it;
`xattr -d com.apple.quarantine grounds` clears that.

From source:

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
grounds: API changes in src/main/java/.../Owner.java broke 15 place(s) in 6 other file(s) not yet updated:
src/main/java/.../PetController.java:103:9: `Owner` has no method `addPet`
...
```

The daemon keeps one warm index per project over a Unix socket, re-reads
files whose modification time changed before every request, and exits
after 30 minutes idle. It is started on demand and replaced automatically
when the `grounds` binary changes.

### CI

`grounds changed --base <ref>` checks what a branch or pull request changed
since it forked from `<ref>` (the merge base), committed or not:

- findings on the changed lines only, so existing code adds no noise;
- places in **unchanged** files that the change broke: calls to removed or
  renamed members, implementations of a changed interface, `@Override`s of a
  changed supertype, and injection points whose bean was deleted or became
  ambiguous. grounds analyzes the other files against both versions of the
  changed ones and keeps what is new (about 1 s for spring-petclinic-rest,
  17 s for camunda's 15,700 files);
- for each changed line that broke something, a `breaks-other-files`
  finding on that line listing the broken places, so the break shows in the
  diff view even though the broken files are not part of it.

`--format sarif` writes SARIF 2.1.0 with paths from the repository root
(also when `--root` is a subdirectory). Exit codes: 0 nothing found, 2
findings, 1 failure (for example no merge base in a shallow clone);
`--exit-zero` turns "findings" into 0 so a job that only uploads them does
not fail, while a real failure still does.

GitHub Actions, as a non-blocking job uploading to code scanning:

```yaml
on: pull_request
jobs:
  grounds:
    runs-on: ubuntu-latest
    permissions: { contents: read, security-events: write }
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }          # --base needs the history to the merge base
      - uses: actions/setup-java@v4       # grounds reads the JDK's API signatures
        with: { distribution: temurin, java-version: 21 }
      - run: ./gradlew compileJava -q     # dependency jars into the Gradle cache (or: grounds classpath)
      - run: |
          curl -fsSL https://github.com/mihaidaniel34/grounds/releases/download/v0.0.8/grounds-0.0.8-x86_64-unknown-linux-musl.tar.gz | tar -xz
          echo "$PWD/grounds-0.0.8-x86_64-unknown-linux-musl" >> "$GITHUB_PATH"
      - run: grounds changed --base "origin/$BASE_REF" --no-daemon --format sarif --exit-zero > grounds.sarif
        env: { BASE_REF: "${{ github.base_ref }}" }   # never interpolate a branch name into the script
      - uses: github/codeql-action/upload-sarif@v3
        with: { sarif_file: grounds.sarif, category: grounds }
```

With reviewdog instead, keep grounds' scope rather than reviewdog's diff
filter, which would drop the breaks in unchanged files:
`grounds changed --base origin/main --no-daemon --format sarif --exit-zero | reviewdog -f=sarif -reporter=github-pr-check -filter-mode=nofilter`.

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

With the guessed classpath, jar versions may not match the project's, so the
compile-break rules skip classes that extend library classes (a method
missing from the cached version may exist in the one the project uses).
With an exact classpath and `scan_caches = false`, those are checked too.

## Design

- **Unknown means silent.** Anything that depends on a type grounds could
  not resolve is not reported. Missing jars lower recall, not precision.
- **Declarations eagerly, bodies lazily.** Every file is reduced to a
  declaration stub (signatures, annotations, constants), cached by content
  hash. Method bodies are analyzed only for the files being checked.
- **No build tool in the loop.** Modules, source sets and the classpath come
  from the directory layout and the local caches, or from the one-off dump.

```
grounds-syntax     Java parser (tree-sitter-java-orchard), node helpers
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
