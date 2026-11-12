//! Probe: compare what grounds resolves with what javac recorded in the
//! class files compiled from the same sources, on a published library.
//! scripts/bytecode_oracle.py fetches sources and classes of published
//! libraries and runs this on them, and fails when a disagreement appears
//! that its baseline does not list. The comparisons themselves, and what
//! each compares, are in `grounds_engine::oracle`; `grounds selfcheck` makes
//! the same on a project's own build output.
//!
//!   bytecode_oracle [--json] <project root> <classes directory>
//!
//! The sources are under `<project root>/src/main/java`; the classes
//! directory holds the class files compiled from them (the library's jar,
//! unpacked).
//!
//! Each comparison prints what it compared, agreed on, disagreed on and did
//! not compare, then each disagreement on one line; with `--json`, one JSON
//! object with the counts of each comparison and each disagreement as file,
//! line, subject (the name; for members the class and the member) and the
//! two answers, for the script's baseline.

use grounds_engine::oracle::{compare_file, Bytecode, Tally};
use rayon::prelude::*;

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    let json = args.iter().any(|a| a == "--json");
    args.retain(|a| a != "--json");
    anyhow::ensure!(args.len() == 3, "usage: bytecode_oracle [--json] <project root> <classes directory>");
    let root = std::path::PathBuf::from(&args[1]).canonicalize()?;
    let bc = Bytecode::read(&std::path::PathBuf::from(&args[2]).canonicalize()?);
    let p = grounds_engine::Project::load(&root)?;
    let src_root = root.join("src/main/java");
    let files: Vec<_> = p.workspace.files.iter().collect();
    let tallies: Vec<Tally> = files
        .par_iter()
        .filter_map(|f| {
            let rel = f.path.strip_prefix(&src_root).ok()?;
            Some(compare_file(&p, &bc, &f.path, &rel.to_string_lossy()))
        })
        .collect();
    let mut s = Tally::default();
    for t in tallies {
        s.add(t);
    }
    let no_source = bc.without_source_class(&p, &src_root);
    if json {
        println!("{}", s.probe_json(files.len(), no_source.len(), bc.unreadable.len()));
    } else {
        print!("{}", s.probe_text(files.len(), &no_source, &bc.unreadable));
    }
    Ok(())
}
