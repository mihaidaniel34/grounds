//! Probe: the build picture of a project, per source root. Which files the
//! build compiles is read from the modules' build output
//! (`grounds_engine::picture`); this prints, for each source root as the
//! picture places it, its state and how many source files have it, so a
//! whole checkout can be read at a glance (a sample directory not in the
//! build, a compiled tree with a root of its own inside it).
//!
//!   picture_dump <project root>
//!
//! One line per source root and state: `<state>\t<files>\t<root>`, the root
//! relative to the project root, sorted by root.

use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: picture_dump <project root>")).canonicalize()?;
    let p = grounds_engine::Project::load(&root)?;
    let mut by_root: BTreeMap<(String, String), usize> = BTreeMap::new();
    for f in &p.workspace.files {
        let key = grounds_engine::project::path_key(&f.path);
        let stub = p.index.file(&key);
        let r = p.picture.root_of(&f.path, stub.map_or("", |s| &s.package));
        let state = format!("{:?}", p.index.in_build(&key));
        *by_root.entry((r.strip_prefix(&root).unwrap_or(&r).display().to_string(), state)).or_default() += 1;
    }
    for ((r, s), n) in by_root {
        println!("{s}\t{n}\t{r}");
    }
    Ok(())
}
