//! The kernel and the build picture: no claim in a file the build does not
//! compile.

use crate::closed::{Jars, Open};
use crate::FileModel;
use grounds_index::*;
use grounds_syntax::ParsedFile;
use std::sync::{Arc, OnceLock};

fn jdk() -> Arc<Classpath> {
    static CP: OnceLock<Arc<Classpath>> = OnceLock::new();
    CP.get_or_init(|| Arc::new(Classpath::build(grounds_classfile::find_java_home().as_deref(), 21, &[]))).clone()
}

const NODE: &str = "package p; public class Node { public void visit(int i) {} }";
const USE: &str = "package s; import p.Node; class Use { void f(Node n) { n.visit(\"x\"); } }";

fn stub(src: &str) -> Arc<FileStub> {
    Arc::new(extract(&ParsedFile::parse(src)))
}

/// An index of `files` (path, source, state in the build).
fn index(files: &[(&str, &str, InBuild)]) -> Index {
    let mut idx = Index::new(jdk());
    for (path, src, state) in files {
        idx.set_file(Arc::from(*path), stub(src));
        idx.set_in_build(Arc::from(*path), *state);
    }
    idx.resolve_ordered();
    idx
}

/// Whether `Node` is closed as the file at `path` (holding `src`) sees it.
fn closed_from(idx: &Index, path: &str, src: &str) -> Result<(), Open> {
    if idx.class("java.lang.Object").is_none() {
        // (no JDK: nothing is ever closed, and this test says nothing)
        return Ok(());
    }
    let pf = ParsedFile::parse(src);
    let fm = FileModel::new(&pf, idx, Arc::from(path));
    fm.closed("p.Node", Jars::Exact).map(|_| ())
}

const CORE: &str = "/r/core/src/main/java/p/Node.java";
const OLD_USE: &str = "/r/solver/src/test/test_sourcecode/solver/s/Use.java";

#[test]
fn a_file_the_build_does_not_compile_gets_no_claim() {
    use InBuild::*;
    let idx = index(&[(CORE, NODE, Yes), (OLD_USE, USE, No)]);
    if idx.class("java.lang.Object").is_none() {
        return;
    }
    assert_eq!(closed_from(&idx, OLD_USE, USE), Err(Open::NotInBuild));
    let pf = ParsedFile::parse(USE);
    let fm = FileModel::new(&pf, &idx, Arc::from(OLD_USE));
    assert!(matches!(fm.java_release(), Err(Open::NotInBuild)));
    // unknown: as before there was a picture of the build
    let idx = index(&[(CORE, NODE, Yes), (OLD_USE, USE, Unknown)]);
    assert_eq!(closed_from(&idx, OLD_USE, USE), Ok(()));
}
