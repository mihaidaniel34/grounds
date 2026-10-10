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
    }
    Ok(())
}
