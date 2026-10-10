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
    /// List all rules.
    Rules {
        #[arg(long)]
        json: bool,
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
        Cmd::Rules { json } => {
            let cat = grounds_rules::catalog();
            if json {
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
