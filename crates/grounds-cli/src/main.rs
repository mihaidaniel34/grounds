use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "grounds", about = "Fast incremental static analysis for Java")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
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
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Index { root } => {
            let t = std::time::Instant::now();
            let p = grounds_engine::Project::load(&root)?;
            let s = &p.stats;
            println!("modules: {}", p.workspace.modules.len());
            println!("files: {} (parsed {}, cached {})", s.files, s.parsed, s.cached);
            println!("classpath: {} entries, {} classes", s.classpath_entries, s.classpath_classes);
            println!("source classes: {}", p.index.source_class_names().count());
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
        Cmd::TypeStats { root, show_unresolved } => type_stats(&root, show_unresolved)?,
        Cmd::Check { paths, root, format, rules, min_severity, max, stats } => {
            let code = check(&root, &paths, &format, rules, min_severity, max, stats)?;
            std::process::exit(code);
        }
        Cmd::Changed { files, root, edit, all_lines, no_baseline, max, min_severity, json, no_daemon } => {
            let root = find_root(root);
            let req = grounds_engine::daemon::Request::Check {
                mode: if edit { grounds_engine::session::Mode::Edit } else { grounds_engine::session::Mode::Checkpoint },
                files: abs(&files),
                all_lines,
                baseline: !no_baseline,
                max,
                min_severity,
                json,
            };
            let resp = if no_daemon { grounds_engine::daemon::execute(&mut grounds_engine::session::Session::open(&root)?, &req) } else { grounds_engine::daemon::run(&root, &req, true)? };
            print!("{}", resp.stdout);
            eprint!("{}", resp.stderr);
            if resp.stdout.is_empty() && resp.code == 0 && !json {
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
                let mut by: std::collections::BTreeMap<String, Vec<String>> = Default::default();
                for l in resp.stdout.lines() {
                    if let Some((m, t)) = l.split_once('\t') {
                        by.entry(m.to_string()).or_default().push(t.to_string());
                    }
                }
                for (m, ts) in by {
                    let task = if m == ":" || m.is_empty() { "test".to_string() } else { format!(":{}:test", m.replace('/', ":")) };
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
            let di = &p.di;
            let filter: Option<String> = ty.map(|t| {
                if t.contains('.') {
                    t
                } else {
                    p.index.source_class_names().find(|n| grounds_index::simple_name_of(n) == t).map(|n| n.to_string()).unwrap_or(t)
                }
            });
            let idx: Vec<usize> = match &filter {
                Some(t) => di.candidates(&p.index, &grounds_index::Ty::class(t), true).0,
                None => (0..di.beans.len()).collect(),
            };
            for i in idx {
                let b = &di.beans[i];
                let mut flags = Vec::new();
                if b.primary { flags.push("primary"); }
                if b.conditional { flags.push("conditional"); }
                if b.is_test { flags.push("test"); }
                println!("{:<40} {:<60} {:?} {}", b.name, b.ty.to_string(), b.source, flags.join(","));
            }
            eprintln!("{} beans, {} injection points, scan roots: {:?}", di.beans.len(), di.points.len(), di.scan_roots);
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
            let req = Request::Check { mode: Mode::Edit, files: vec![PathBuf::from(file)], all_lines: false, baseline: true, max: 10, min_severity: "warning".into(), json: false };
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
            let req = Request::Check { mode: Mode::Checkpoint, files: Vec::new(), all_lines: false, baseline: true, max: 15, min_severity: "warning".into(), json: false };
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

fn check(root: &std::path::Path, paths: &[PathBuf], format: &str, rules: Option<String>, min_sev: Option<String>, max: Option<usize>, stats: bool) -> anyhow::Result<i32> {
    use grounds_engine::{analyze, report};
    let t = std::time::Instant::now();
    let mut project = grounds_engine::Project::load(root)?;
    if let Some(r) = rules {
        project.config.rules.enable = r.split(',').map(|s| s.trim().to_string()).collect();
    }
    let (ruleset, options) = analyze::build_ruleset(&project);
    let files = analyze::expand_paths(&project, paths);
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
            "sarif" => println!("{}", report::sarif(&findings)),
            _ => print!("{}", report::text(&findings, max)),
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
