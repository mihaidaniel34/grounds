use clap::{Parser, Subcommand};
use grounds_engine::report;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "grounds", about = "Fast incremental static analysis for Java", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
    /// Colored output: auto (terminal only, honors NO_COLOR), always, never.
    #[arg(long, global = true, default_value = "auto", value_parser = ["auto", "always", "never"])]
    color: String,
}

#[derive(Subcommand)]
enum Cmd {
    /// Load a project and print index statistics.
    Index {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    /// Analyze files (default: whole project) and print findings.
    Check {
        /// Files or directories; default is all sources.
        paths: Vec<PathBuf>,
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// text | json | sarif
        #[arg(long, default_value = "text")]
        format: String,
        /// Only run these rules (comma separated ids or categories).
        #[arg(long)]
        rules: Option<String>,
        /// Minimum severity: error | warning | info
        #[arg(long)]
        min_severity: Option<String>,
        /// Maximum findings to print (text format).
        #[arg(long)]
        max: Option<usize>,
        /// Print per-rule counts instead of findings.
        #[arg(long)]
        stats: bool,
    },
    /// Check changed code (git) the way an agent needs it: only changed lines,
    /// baseline applied, capped output, plus callers broken by API changes.
    Changed {
        /// Files to check (default: files changed vs HEAD).
        files: Vec<PathBuf>,
        #[arg(long)]
        root: Option<PathBuf>,
        /// Only the fast, high-confidence per-edit rules.
        #[arg(long)]
        edit: bool,
        /// Report on all lines of the files, not only changed lines.
        #[arg(long)]
        all_lines: bool,
        #[arg(long)]
        no_baseline: bool,
        #[arg(long, default_value_t = 15)]
        max: usize,
        #[arg(long, default_value = "warning")]
        min_severity: String,
        #[arg(long)]
        json: bool,
        /// text | json | sarif (`--json` is `--format json`).
        #[arg(long, default_value = "text")]
        format: String,
        /// Compare with the merge base of this revision and HEAD instead of
        /// with HEAD: the lines a branch or pull request changed, and places
        /// in unchanged files that its changes broke (for CI).
        #[arg(long)]
        base: Option<String>,
        /// Do not use or start the background daemon.
        #[arg(long)]
        no_daemon: bool,
    },
    /// Accept the current findings of the given (default: changed) files.
    Baseline {
        files: Vec<PathBuf>,
        #[arg(long)]
        root: Option<PathBuf>,
        /// Add to the existing baseline instead of replacing it.
        #[arg(long)]
        merge: bool,
    },
    /// Test classes affected by the given (default: changed) files.
    ImpactedTests {
        files: Vec<PathBuf>,
        #[arg(long)]
        root: Option<PathBuf>,
        /// Print one gradle command per module.
        #[arg(long)]
        gradle: bool,
    },
    /// Run the background analysis server for a workspace.
    Daemon {
        #[arg(long)]
        root: Option<PathBuf>,
        #[arg(long, default_value_t = 30)]
        idle_minutes: u64,
        #[arg(long)]
        stop: bool,
        #[arg(long)]
        status: bool,
    },
    /// Claude Code hook entry points (reads the hook JSON on stdin):
    /// post-edit | stop | session-start
    Hook { event: String },
    /// Print the Claude Code hook configuration for .claude/settings.json.
    Hooks,
    /// List beans of the dependency-injection model (optionally those that
    /// can be injected as a given type).
    Beans {
        #[arg(long)]
        root: Option<PathBuf>,
        /// Simple or qualified type name to look up.
        #[arg(long = "type")]
        ty: Option<String>,
    },
    /// Dump the exact compile classpath once through Gradle or Maven into
    /// .grounds/classpath.txt (re-run when dependencies change).
    Classpath {
        #[arg(long)]
        root: Option<PathBuf>,
        #[arg(long)]
        gradle: bool,
        #[arg(long)]
        maven: bool,
    },
    /// List all rules.
    Rules {
        #[arg(long)]
        json: bool,
        /// Markdown catalog grouped by category.
        #[arg(long)]
        markdown: bool,
    },
    /// Explain a rule.
    Explain { rule: String },
    /// Debug: measure expression typing coverage over the project.
    TypeStats {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        show_unresolved: bool,
        /// Group unresolved calls by cause (missing jar, unknown receiver, ...)
        #[arg(long)]
        causes: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let style = grounds_engine::report::Style::detect(&cli.color);
    match cli.cmd {
        Cmd::Index { root } => {
            let t = std::time::Instant::now();
            let p = grounds_engine::Project::load(&root)?;
            let s = &p.stats;
            println!("modules: {}", p.workspace.modules.len());
            println!("files: {} (parsed {}, cached {})", s.files, s.parsed, s.cached);
            println!("classpath: {} entries, {} classes", s.classpath_entries, s.classpath_classes);
            println!("source classes: {}", p.index.source_class_names().count());
            // tree-sitter recovers from syntax it cannot parse; analysis of those
            // regions is degraded, so make the count visible
            let broken: Vec<String> = p.index.files().filter(|(_, s)| s.has_syntax_errors).map(|(k, _)| k.strip_prefix(&*p.workspace.root.to_string_lossy()).unwrap_or(k).trim_start_matches('/').to_string()).collect();
            println!("files with parse errors: {}", broken.len());
            for f in broken.iter().take(5) {
                println!("  {f}");
            }
            println!(
                "timing: discover {:?}, classpath {:?}, stubs {:?}, index {:?}, total {:?}",
                s.t_discover, s.t_classpath, s.t_stubs, s.t_index, t.elapsed()
            );
            // resolve everything to measure lazy resolution cost
            let t = std::time::Instant::now();
            let names: Vec<_> = p.index.source_class_names().cloned().collect();
            let mut unknown = 0usize;
            let mut total = 0usize;
            for n in &names {
                if let Some(c) = p.index.class(n) {
                    for m in &c.methods {
                        for pa in &m.params {
                            total += 1;
                            if pa.ty.has_unknown() {
                                unknown += 1;
                            }
                        }
                    }
                }
            }
            println!("resolved {} classes in {:?}; param types unknown {}/{}", names.len(), t.elapsed(), unknown, total);
        }
        Cmd::TypeStats { root, show_unresolved, causes } => {
            type_stats(&root, show_unresolved)?;
            if causes {
                unresolved_causes(&root)?;
            }
        }
        Cmd::Check { paths, root, format, rules, min_severity, max, stats } => {
            let code = check(&root, &paths, &format, rules, min_severity, max, stats, style)?;
            std::process::exit(code);
        }
        Cmd::Changed { files, root, edit, all_lines, no_baseline, max, min_severity, json, format, base, no_daemon } => {
            let root = find_root(root);
            if !matches!(format.as_str(), "text" | "json" | "sarif") {
                anyhow::bail!("unknown --format {format} (text, json or sarif)");
            }
            let sarif = format == "sarif";
            let json = json || format == "json";
            let req = grounds_engine::daemon::Request::Check {
                mode: if edit { grounds_engine::session::Mode::Edit } else { grounds_engine::session::Mode::Checkpoint },
                files: abs(&files),
                all_lines,
                baseline: !no_baseline,
                max,
                min_severity,
                json,
                color: style.color && !json && !sarif,
                base,
                sarif,
            };
            let resp = if no_daemon { grounds_engine::daemon::execute(&mut grounds_engine::session::Session::open(&root)?, &req) } else { grounds_engine::daemon::run(&root, &req, true)? };
            print!("{}", resp.stdout);
            eprint!("{}", resp.stderr);
            if resp.stdout.is_empty() && resp.code == 0 && !json && !sarif {
                eprintln!("grounds: no new problems in changed code");
            }
            std::process::exit(resp.code);
        }
        Cmd::Baseline { files, root, merge } => {
            let root = find_root(root);
            let resp = grounds_engine::daemon::run(&root, &grounds_engine::daemon::Request::Baseline { files: abs(&files), merge }, false)?;
            print!("{}", resp.stdout);
            eprint!("{}", resp.stderr);
            std::process::exit(resp.code);
        }
        Cmd::ImpactedTests { files, root, gradle } => {
            let root = find_root(root);
            let resp = grounds_engine::daemon::run(&root, &grounds_engine::daemon::Request::ImpactedTests { files: abs(&files) }, false)?;
            if gradle {
                let mut by: std::collections::BTreeMap<(String, String), Vec<String>> = Default::default();
                for l in resp.stdout.lines() {
                    let mut parts = l.splitn(3, '\t');
                    if let (Some(m), Some(task), Some(t)) = (parts.next(), parts.next(), parts.next()) {
                        by.entry((m.to_string(), task.to_string())).or_default().push(t.to_string());
                    }
                }
                for ((m, task), ts) in by {
                    let task = if m == ":" || m.is_empty() { task } else { format!(":{}:{task}", m.replace('/', ":")) };
                    let filters: Vec<String> = ts.iter().map(|t| format!("--tests '{}'", t.replace('$', "."))).collect();
                    println!("./gradlew {task} {}", filters.join(" "));
                }
            } else {
                print!("{}", resp.stdout);
            }
        }
        Cmd::Daemon { root, idle_minutes, stop, status } => {
            let root = find_root(root);
            if stop {
                let r = grounds_engine::daemon::request(&root, &grounds_engine::daemon::Request::Shutdown);
                println!("{}", if r.is_some() { "stopped" } else { "not running" });
                return Ok(());
            }
            if status {
                match grounds_engine::daemon::request(&root, &grounds_engine::daemon::Request::Ping) {
                    Some(r) => print!("running: {}", r.stdout),
                    None => println!("not running"),
                }
                return Ok(());
            }
            grounds_engine::daemon::serve(&root, std::time::Duration::from_secs(idle_minutes * 60))?;
        }
        Cmd::Hook { event } => std::process::exit(hook(&event)?),
        Cmd::Hooks => println!("{}", HOOKS_JSON),
        Cmd::Classpath { root, gradle, maven } => {
            let root = find_root(root);
            let use_maven = maven || (!gradle && root.join("pom.xml").exists() && !root.join("build.gradle").exists() && !root.join("build.gradle.kts").exists());
            let n = if use_maven { grounds_engine::dump::maven(&root)? } else { grounds_engine::dump::gradle(&root)? };
            println!("wrote {} classpath entries to .grounds/classpath.txt", n);
            println!("tip: set `scan_caches = false` in grounds.toml to use only the exact classpath");
        }
        Cmd::Beans { root, ty } => {
            let root = find_root(root);
            let p = grounds_engine::Project::load(&root)?;
            beans(&p, ty.as_deref(), style);
        }
        Cmd::Rules { json, markdown } => {
            let cat = grounds_rules::catalog();
            if markdown {
                let mut by: std::collections::BTreeMap<&str, Vec<&grounds_rules::RuleMeta>> = Default::default();
                for m in cat {
                    by.entry(m.category.as_str()).or_default().push(m);
                }
                println!("# Rule catalog\n\n{} rules. `grounds explain <id>` prints the full description.\n", cat.len());
                println!("Tier `edit` rules run after every edit; all rules run at checkpoints. Rules marked *off* are disabled by default (enable in `grounds.toml`).\n");
                for (c, ms) in by {
                    println!("## {c} ({})\n\n| id | severity | tier | origin |\n|---|---|---|---|", ms.len());
                    for m in ms {
                        let tier = match m.tier { grounds_rules::Tier::Edit => "edit", _ => "checkpoint" };
                        let off = if m.default_enabled { "" } else { " *off*" };
                        println!("| `{}`{off} | {} | {tier} | {} |", m.id, m.severity.as_str(), m.sources.join(", "));
                    }
                    println!();
                }
            } else if json {
                println!("{}", serde_json::to_string_pretty(&cat).unwrap());
            } else {
                for m in cat {
                    println!("{:<40} {:<8} {:<13} {}", m.id, m.severity.as_str(), m.category.as_str(), m.name);
                }
                println!("{} rules", cat.len());
            }
        }
        Cmd::Explain { rule } => match grounds_rules::meta_of(&rule) {
            Some(m) => {
                println!("{} ({}, {})\n\n{}\n\nsources: {}", m.name, m.severity.as_str(), m.category.as_str(), m.description, m.sources.join(", "));
            }
            None => {
                eprintln!("unknown rule {rule}");
                std::process::exit(2);
            }
        },
    }
    Ok(())
}

fn find_root(root: Option<PathBuf>) -> PathBuf {
    if let Some(r) = root {
        return r.canonicalize().unwrap_or(r);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    grounds_engine::git::toplevel(&cwd).unwrap_or(cwd)
}

fn abs(files: &[PathBuf]) -> Vec<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    files.iter().map(|f| if f.is_absolute() { f.clone() } else { cwd.join(f) }).collect()
}

const HOOKS_JSON: &str = r#"{
  "hooks": {
    "SessionStart": [
      { "hooks": [ { "type": "command", "command": "grounds hook session-start", "timeout": 60 } ] }
    ],
    "PostToolUse": [
      { "matcher": "Edit|Write|MultiEdit",
        "hooks": [ { "type": "command", "command": "grounds hook post-edit", "timeout": 30 } ] }
    ],
    "Stop": [
      { "hooks": [ { "type": "command", "command": "grounds hook stop", "timeout": 120 } ] }
    ]
  }
}"#;

/// Claude Code hooks. Exit code 2 feeds stderr back to the model.
fn hook(event: &str) -> anyhow::Result<i32> {
    use grounds_engine::daemon::{run, Request};
    use grounds_engine::session::Mode;
    use std::io::Read;
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let v: serde_json::Value = serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);
    let cwd = v.get("cwd").and_then(|c| c.as_str()).map(PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let root = grounds_engine::git::toplevel(&cwd).unwrap_or(cwd);
    // only act inside Java workspaces
    let java_project = ["build.gradle", "build.gradle.kts", "pom.xml", "settings.gradle", "settings.gradle.kts"].iter().any(|f| root.join(f).exists());
    if !java_project {
        return Ok(0);
    }
    match event {
        "post-edit" => {
            let Some(file) = v.pointer("/tool_input/file_path").and_then(|f| f.as_str()) else { return Ok(0) };
            if !file.ends_with(".java") {
                return Ok(0);
            }
            let req = Request::Check { mode: Mode::Edit, files: vec![PathBuf::from(file)], all_lines: false, baseline: true, max: 10, min_severity: "warning".into(), json: false, color: false, base: None, sarif: false };
            let resp = run(&root, &req, true)?;
            if resp.code == 2 {
                eprint!("{}", resp.stdout);
                return Ok(2);
            }
            Ok(0)
        }
        "stop" => {
            // avoid loops: when Claude is already continuing because of this hook, let it stop
            if v.get("stop_hook_active").and_then(|b| b.as_bool()) == Some(true) {
                return Ok(0);
            }
            let req = Request::Check { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, baseline: true, max: 15, min_severity: "warning".into(), json: false, color: false, base: None, sarif: false };
            let resp = run(&root, &req, true)?;
            if resp.code == 2 {
                eprint!("{}", resp.stdout);
                eprintln!("(fix these or explain why they are fine; accept intentional ones with `grounds baseline <file>`)");
                return Ok(2);
            }
            Ok(0)
        }
        "session-start" => {
            // accept whatever is already wrong in uncommitted work, so the session only sees new problems
            let _ = run(&root, &Request::Baseline { files: Vec::new(), merge: true }, true)?;
            Ok(0)
        }
        _ => {
            eprintln!("unknown hook event {event}");
            Ok(1)
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Relative paths are taken from the working directory, or else from the
/// project root (`--root app src/...` from elsewhere); a path that exists in
/// neither is an error rather than a silent "0 findings".
fn resolve_paths(root: &std::path::Path, paths: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    paths
        .iter()
        .map(|p| {
            if p.exists() {
                Ok(p.clone())
            } else if p.is_relative() && root.join(p).exists() {
                Ok(root.join(p))
            } else {
                anyhow::bail!("no such file or directory: {} (also not under --root {})", p.display(), root.display())
            }
        })
        .collect()
}

fn check(root: &std::path::Path, paths: &[PathBuf], format: &str, rules: Option<String>, min_sev: Option<String>, max: Option<usize>, stats: bool, style: report::Style) -> anyhow::Result<i32> {
    use grounds_engine::analyze;
    let t = std::time::Instant::now();
    let mut project = grounds_engine::Project::load(root)?;
    if let Some(r) = rules {
        project.config.rules.enable = r.split(',').map(|s| s.trim().to_string()).collect();
    }
    let (ruleset, options) = analyze::build_ruleset(&project);
    let files = analyze::expand_paths(&project, &resolve_paths(root, paths)?);
    let t_load = t.elapsed();
    let t = std::time::Instant::now();
    let mut findings = analyze::analyze_files(&project, &ruleset, &options, &files);
    let min = min_sev.as_deref().and_then(grounds_rules::Severity::parse).unwrap_or(grounds_rules::Severity::Info);
    findings.retain(|f| f.severity <= min);
    if stats {
        let mut by: std::collections::BTreeMap<&str, usize> = Default::default();
        for f in &findings {
            *by.entry(f.rule).or_default() += 1;
        }
        let mut v: Vec<_> = by.into_iter().collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        for (r, n) in v {
            println!("{n:6} {r}");
        }
    } else {
        match format {
            "json" => println!("{}", report::json(&findings)),
            "sarif" => println!("{}", report::sarif(&findings, &project.workspace.root)),
            _ => print!("{}", report::text(&findings, max, style)),
        }
    }
    if std::env::var("GROUNDS_PROFILE").is_ok() {
        let prof = grounds_rules::runner::PROFILE.lock();
        let mut v: Vec<_> = prof.iter().collect();
        v.sort_by_key(|x| std::cmp::Reverse(*x.1));
        for (r, d) in v.iter().take(25) {
            eprintln!("{:>10.1?} {r}", d);
        }
    }
    if format == "text" {
        eprintln!("{} in {} files (load {:?}, analyze {:?}, {} rules)", report::summary(&findings), files.len(), t_load, t.elapsed(), ruleset.len());
    }
    Ok(if findings.iter().any(|f| f.severity == grounds_rules::Severity::Error) { 1 } else { 0 })
}

fn type_stats(root: &std::path::Path, show: bool) -> anyhow::Result<()> {
    use grounds_syntax::NodeExt;
    use rayon::prelude::*;
    let p = grounds_engine::Project::load(root)?;
    let t = std::time::Instant::now();
    let files: Vec<_> = p.workspace.files.iter().filter(|f| f.kind != grounds_engine::workspace::SourceKind::Generated).collect();
    let res: Vec<(usize, usize, usize, usize, Vec<String>)> = files
        .par_iter()
        .map(|f| {
            let Ok(text) = std::fs::read_to_string(&f.path) else { return (0, 0, 0, 0, vec![]) };
            let pf = grounds_syntax::ParsedFile::parse(text);
            let key = grounds_engine::project::path_key(&f.path);
            let fm = grounds_semantic::FileModel::new(&pf, &p.index, key);
            let (mut calls, mut resolved, mut exprs, mut typed) = (0, 0, 0, 0);
            let mut unresolved = Vec::new();
            for n in pf.root().descendants() {
                if n.kind() == "method_invocation" {
                    calls += 1;
                    if fm.resolve_call(n).is_some() {
                        resolved += 1;
                    } else if show && unresolved.len() < 3 {
                        // only report when the receiver is known
                        let recv_known = match n.child_by_field_name("object") {
                            None => true,
                            Some(o) => fm.type_of(o).is_known() || fm.is_type_name(o),
                        };
                        if recv_known {
                            let rt = n.child_by_field_name("object").map(|o| fm.type_of(o).to_string()).unwrap_or_default();
                            let args: Vec<String> = fm.args_of(n).iter().map(|a| fm.type_of(*a).to_string()).collect();
                            unresolved.push(format!("{}:{} recv={} args={:?}: {}", f.path.file_name().unwrap().to_string_lossy(), n.start_position().row + 1, rt, args, fm.text(n).lines().next().unwrap_or("")));
                        }
                    }
                }
                if matches!(n.kind(), "identifier") && n.parent().is_some_and(|p| matches!(p.kind(), "argument_list" | "binary_expression" | "expression_statement")) {
                    exprs += 1;
                    if fm.type_of(n).is_known() {
                        typed += 1;
                    }
                }
            }
            (calls, resolved, exprs, typed, unresolved)
        })
        .collect();
    let (c, r, e, ty) = res.iter().fold((0, 0, 0, 0), |a, x| (a.0 + x.0, a.1 + x.1, a.2 + x.2, a.3 + x.3));
    println!("files {} in {:?}", files.len(), t.elapsed());
    println!("calls resolved {}/{} ({:.1}%)", r, c, 100.0 * r as f64 / c.max(1) as f64);
    println!("identifier exprs typed {}/{} ({:.1}%)", ty, e, 100.0 * ty as f64 / e.max(1) as f64);
    if show {
        for x in res.iter().flat_map(|x| x.4.iter()).take(60) {
            println!("  {x}");
        }
    }
    Ok(())
}

/// Why calls stay unresolved: each unresolved call gets one cause, and the
/// classes behind "missing" causes are grouped by package, so a missing jar
/// (fixed by `grounds classpath`) stands apart from gaps in grounds itself.
fn unresolved_causes(root: &std::path::Path) -> anyhow::Result<()> {
    use grounds_syntax::NodeExt;
    use rayon::prelude::*;
    use std::collections::HashMap;
    let p = grounds_engine::Project::load(root)?;
    let files: Vec<_> = p.workspace.files.iter().filter(|f| f.kind != grounds_engine::workspace::SourceKind::Generated).collect();
    let pkg = |fqn: &str| fqn.split('.').take_while(|s| s.chars().next().is_some_and(|c| c.is_lowercase())).take(3).collect::<Vec<_>>().join(".");
    let per_file: Vec<(HashMap<String, usize>, HashMap<String, usize>, HashMap<String, usize>)> = files
        .par_iter()
        .map(|f| {
            let (mut cause, mut missing, mut known) = (HashMap::new(), HashMap::new(), HashMap::new());
            let Ok(text) = std::fs::read_to_string(&f.path) else { return (cause, missing, known) };
            let pf = grounds_syntax::ParsedFile::parse(text);
            let fm = grounds_semantic::FileModel::new(&pf, &p.index, grounds_engine::project::path_key(&f.path));
            // a simple type name as this file sees it: imported, or unknown
            let fqn_of = |simple: &str| -> Option<String> {
                fm.stub.imports.iter().find(|i| !i.is_static && !i.on_demand && i.path.rsplit('.').next() == Some(simple)).map(|i| i.path.to_string())
            };
            let missing_type = |simple: &str, cause: &mut HashMap<String, usize>, missing: &mut HashMap<String, usize>| match fqn_of(simple) {
                Some(fqn) if p.index.class(&fqn).is_none() => {
                    *cause.entry("receiver type: imported class not on classpath".into()).or_default() += 1;
                    *missing.entry(pkg(&fqn)).or_default() += 1;
                }
                Some(_) => *cause.entry("receiver type: class known, local not typed".into()).or_default() += 1,
                None => *cause.entry("receiver type: not imported (same package / wildcard / nested)".into()).or_default() += 1,
            };
            for n in pf.root().descendants().filter(|n| n.kind() == "method_invocation") {
                if fm.resolve_call(n).is_some() {
                    continue;
                }
                let name = n.child_by_field_name("name").map(|x| fm.text(x)).unwrap_or("");
                let Some(o) = n.child_by_field_name("object").map(|o| o.unparen()) else {
                    *cause.entry("unqualified call (inherited / static import)".into()).or_default() += 1;
                    continue;
                };
                let t = fm.type_of(o);
                if let Some(c) = t.class_name() {
                    if p.index.class(c).is_none() {
                        *cause.entry("receiver type: class not on classpath".into()).or_default() += 1;
                        *missing.entry(pkg(c)).or_default() += 1;
                    } else {
                        *cause.entry("receiver class known, method not matched".into()).or_default() += 1;
                        *known.entry(format!("{c}.{name}")).or_default() += 1;
                    }
                    continue;
                }
                if t.is_known() {
                    *cause.entry(format!("receiver is {}", match t { grounds_index::Ty::TypeVar(_) => "a type variable", grounds_index::Ty::Array(_) => "an array", _ => "other" })).or_default() += 1;
                    continue;
                }
                match o.kind() {
                    "method_invocation" if fm.resolve_call(o).is_none() => *cause.entry("receiver is an unresolved call (cascade)".into()).or_default() += 1,
                    "method_invocation" => {
                        let r = fm.resolve_call(o).unwrap();
                        let m = format!("{}.{}", r.owner, r.method.name);
                        if r.ambiguous {
                            let unknown_arg = fm.args_of(o).iter().any(|a| fm.type_of(*a).is_unknown());
                            *cause.entry(format!("receiver call is an ambiguous overload ({})", if unknown_arg { "an argument type unknown" } else { "argument types known" })).or_default() += 1;
                            if !unknown_arg {
                                *known.entry(format!("ambiguous: {m}({})", fm.args_of(o).iter().map(|a| fm.type_of(*a).to_string()).collect::<Vec<_>>().join(", "))).or_default() += 1;
                            }
                            continue;
                        }
                        match &r.method.ret {
                            grounds_index::Ty::Unknown => {
                                *cause.entry("receiver call resolved, declared return type unknown".into()).or_default() += 1;
                                *missing.entry(format!("(return of) {}", pkg(&r.owner))).or_default() += 1;
                            }
                            grounds_index::Ty::Class { name: c, .. } if p.index.class(c).is_none() => {
                                *cause.entry("receiver call resolved, returns class not on classpath".into()).or_default() += 1;
                                *missing.entry(pkg(c)).or_default() += 1;
                            }
                            grounds_index::Ty::TypeVar(_) => {
                                *cause.entry("receiver call resolved, generic return not inferred".into()).or_default() += 1;
                                *known.entry(format!("generic: {m}({})", fm.args_of(o).iter().map(|a| fm.type_of(*a).to_string()).collect::<Vec<_>>().join(", "))).or_default() += 1;
                            }
                            _ => {
                                *cause.entry("receiver call resolved, return type lost".into()).or_default() += 1;
                                *known.entry(format!("lost: {m} -> {}", r.method.ret)).or_default() += 1;
                            }
                        }
                    }
                    "identifier" if fm.text(o).chars().next().is_some_and(|c| c.is_uppercase()) && !fm.is_type_name(o) => missing_type(fm.text(o), &mut cause, &mut missing),
                    "identifier" => match fm.local_decl(o) {
                        Some(d) => match d.type_node {
                            Some(tn) => {
                                let simple = fm.text(tn).split(['<', '[']).next().unwrap_or("").rsplit('.').next().unwrap_or("").trim().to_string();
                                if simple == "var" {
                                    *cause.entry("receiver local: var, initializer untyped".into()).or_default() += 1;
                                } else {
                                    missing_type(&simple, &mut cause, &mut missing);
                                }
                            }
                            None => *cause.entry("receiver local: lambda parameter, untyped".into()).or_default() += 1,
                        },
                        None => *cause.entry("receiver identifier: field or unknown".into()).or_default() += 1,
                    },
                    k => *cause.entry(format!("receiver expression untyped: {k}")).or_default() += 1,
                }
            }
            (cause, missing, known)
        })
        .collect();
    let mut sum = [HashMap::<String, usize>::new(), HashMap::new(), HashMap::new()];
    for (a, b, c) in per_file {
        for (i, m) in [a, b, c].into_iter().enumerate() {
            for (k, v) in m {
                *sum[i].entry(k).or_default() += v;
            }
        }
    }
    let total: usize = sum[0].values().sum();
    for (title, i, n) in [("unresolved calls by cause", 0, 20), ("missing classes by package", 1, 20), ("grounds gaps by method", 2, 40)] {
        let mut v: Vec<_> = sum[i].iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        println!("\n{title}{}:", if i == 0 { format!(" ({total})") } else { String::new() });
        for (k, c) in v.into_iter().take(n) {
            println!("  {c:>7}  {k}");
        }
    }
    Ok(())
}

/// `grounds beans [--type T]`: the bean model as grounds sees it. With a
/// type: the beans assignable to it and every injection point requesting it,
/// with how each resolves.
fn beans(p: &grounds_engine::Project, ty: Option<&str>, style: report::Style) {
    use grounds_di::{BeanSource, PointKind, Resolution};
    use grounds_index::simple_name_of;
    let di = &p.di;
    let root = p.workspace.root.to_string_lossy().to_string();
    let rel = |f: &Option<std::sync::Arc<str>>| f.as_deref().map(|f| f.strip_prefix(&root).unwrap_or(f).trim_start_matches('/').to_string()).unwrap_or_default();
    let is_test_file = |f: &Option<std::sync::Arc<str>>| f.as_deref().is_some_and(|f| grounds_engine::workspace::classify(std::path::Path::new(f)) == grounds_engine::workspace::SourceKind::Test);
    let describe = |i: usize| {
        let b = &di.beans[i];
        let how = match &b.source {
            BeanSource::Component => "component".to_string(),
            BeanSource::FactoryMethod(m) => format!("@Bean {}.{}()", simple_name_of(&b.declaring_class), m),
            BeanSource::Repository => "repository".to_string(),
            BeanSource::Client => "client".to_string(),
            BeanSource::Mapper => "mapper".to_string(),
        };
        let mut flags: Vec<String> = Vec::new();
        if b.primary { flags.push("primary".into()); }
        if b.conditional { flags.push("conditional".into()); }
        if b.is_test { flags.push("test".into()); }
        if !b.qualifiers.is_empty() { flags.push(format!("qualifier={}", b.qualifiers.join(","))); }
        [b.name.clone(), simple_name_of(&b.ty.to_string()).to_string(), how, flags.join(" "), rel(&b.file)]
    };
    let table = |rows: Vec<[String; 5]>| {
        let mut w = [0usize; 4];
        for r in &rows {
            for (i, c) in r.iter().take(4).enumerate() {
                w[i] = w[i].max(c.chars().count());
            }
        }
        for r in rows {
            println!("  {:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {}", r[0], r[1], r[2], r[3], style.dim(&r[4]), w0 = w[0], w1 = w[1], w2 = w[2], w3 = w[3]);
        }
    };
    let Some(t) = ty else {
        let mut idx: Vec<usize> = (0..di.beans.len()).collect();
        idx.sort_by(|a, b| di.beans[*a].name.cmp(&di.beans[*b].name));
        table(idx.into_iter().map(describe).collect());
        eprintln!("{} beans, {} injection points; scan roots: {}", di.beans.len(), di.points.len(), di.scan_roots.join(", "));
        return;
    };
    let full = if t.contains('.') {
        t.to_string()
    } else {
        let matches: Vec<String> = p.index.source_class_names().filter(|n| simple_name_of(n) == t).map(|n| n.to_string()).collect();
        if matches.len() > 1 {
            eprintln!("`{t}` is ambiguous, use the qualified name: {}", matches.join(", "));
            std::process::exit(2);
        }
        matches.into_iter().next().unwrap_or_else(|| t.to_string())
    };
    if p.index.class(&full).is_none() {
        eprintln!("type `{t}` not found in the project or on the classpath");
        std::process::exit(2);
    }
    let want = grounds_index::Ty::class(&full);
    let library = !p.index.is_source_class(&full);
    let mut cands = di.candidates(&p.index, &want, true).0;
    cands.sort_by(|a, b| di.beans[*a].name.cmp(&di.beans[*b].name));
    println!("{}", style.bold(&format!("Beans assignable to {} ({}):", simple_name_of(&full), cands.len())));
    if cands.is_empty() {
        println!("  none");
    }
    table(cands.into_iter().map(describe).collect());

    let mut points: Vec<&grounds_di::InjectionPoint> = di.points.iter().filter(|pt| pt.ty.class_name() == Some(full.as_str())).collect();
    points.sort_by_key(|pt| (rel(&pt.file), pt.range.map(|r| r.0)));
    println!();
    println!("{}", style.bold(&format!("Injection points requesting {} ({}):", simple_name_of(&full), points.len())));
    if points.is_empty() {
        println!("  none");
    }
    let mut texts: std::collections::HashMap<String, String> = Default::default();
    let names = |v: &[usize]| v.iter().map(|i| di.beans[*i].name.as_str()).collect::<Vec<_>>().join(", ");
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for pt in points {
        let owner = simple_name_of(&pt.owner);
        let site = match &pt.kind {
            PointKind::ConstructorParam(_) => format!("{owner}({})", pt.name),
            PointKind::Field => format!("{owner}.{}", pt.name),
            PointKind::MethodParam(m, _) | PointKind::FactoryParam(m, _) => format!("{owner}.{m}({})", pt.name),
        };
        let mut site = site;
        if let Some(q) = &pt.qualifier { site.push_str(&format!(" @{q}")); }
        let file = rel(&pt.file);
        let line = match (pt.file.as_deref(), pt.range) {
            (Some(f), Some((start, _))) => {
                let text = texts.entry(f.to_string()).or_insert_with(|| std::fs::read_to_string(f).unwrap_or_default());
                text.as_bytes()[..(start as usize).min(text.len())].iter().filter(|b| **b == b'\n').count() + 1
            }
            _ => 0,
        };
        let outcome = match di.resolve(&p.index, pt, is_test_file(&pt.file)) {
            Resolution::Unique(i) => format!("-> {}", di.beans[i].name),
            Resolution::Multiple(v) => format!("-> all of: {}", names(&v)),
            Resolution::Absent => "-> nothing (optional)".to_string(),
            Resolution::NotFound => style.paint_error("-> no bean of this type"),
            Resolution::Ambiguous(v) => style.paint_error(&format!("-> ambiguous: {}", names(&v))),
            Resolution::QualifierNotFound { qualifier, candidates } => style.paint_error(&format!("-> qualifier \"{qualifier}\" matches none of: {}", names(&candidates))),
            Resolution::Unknown if !library => style.dim("-> not judged (depends on something grounds cannot see)"),
            Resolution::Unknown => style.dim("-> not judged (library type: beans from auto-configuration are not modeled)"),
        };
        rows.push((site, outcome, format!("{file}:{line}")));
    }
    let w0 = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
    // the outcome may carry color codes: pad on its visible width
    let visible = |s: &str| if style.color { s.chars().count().saturating_sub(9) } else { s.chars().count() };
    let w1 = rows.iter().map(|r| visible(&r.1)).max().unwrap_or(0);
    for (site, outcome, at) in rows {
        let pad = w1 - visible(&outcome);
        println!("  {site:<w0$}  {outcome}{}  {}", " ".repeat(pad), style.dim(&at));
    }
}
