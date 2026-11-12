//! The kernel and the build picture: no claim in a file the build does not
//! compile, and a class with one copy the build compiles is certain in a
//! file the build compiles, whatever copies test data holds.

use crate::closed::{Jars, Open};
use crate::FileModel;
use grounds_index::*;
use grounds_syntax::{NodeExt, ParsedFile};
use std::sync::{Arc, OnceLock};

fn jdk() -> Arc<Classpath> {
    static CP: OnceLock<Arc<Classpath>> = OnceLock::new();
    CP.get_or_init(|| Arc::new(Classpath::build(grounds_classfile::find_java_home().as_deref(), 21, &[]))).clone()
}

const NODE: &str = "package p; public class Node { public void visit(int i) {} }";
/// The same class as an old snapshot of the sources has it.
const OLD_NODE: &str = "package p; public class Node { public void visit(String s) {} }";
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
const SNAPSHOT: &str = "/r/solver/src/test/test_sourcecode/core/p/Node.java";
const REAL: &str = "/r/solver/src/main/java/s/Use.java";
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

/// javaparser's symbol solver keeps old snapshots of javaparser's sources as
/// test data: `Node` is declared by the real module and by the snapshot.
#[test]
fn a_class_with_one_copy_in_the_build_is_certain_where_the_build_compiles() {
    use InBuild::*;
    // the snapshot's own code: never compiled, no claim (it would resolve
    // `Node` to the real class, not the snapshot it was written against)
    let idx = index(&[(CORE, NODE, Yes), (SNAPSHOT, OLD_NODE, No), (REAL, USE, Yes), (OLD_USE, USE, No)]);
    if idx.class("java.lang.Object").is_none() {
        return;
    }
    assert_eq!(closed_from(&idx, OLD_USE, USE), Err(Open::NotInBuild));
    // real code of another module: the copy the build compiles is the one
    // compiled code sees, and the one the model reads
    assert_eq!(closed_from(&idx, REAL, USE), Ok(()));
    let pf = ParsedFile::parse(USE);
    let fm = FileModel::new(&pf, &idx, Arc::from(REAL));
    assert_eq!(fm.class("p.Node").unwrap().source_file(), Some(CORE));
    // without the picture, two copies and no way to tell
    let idx = index(&[(CORE, NODE, Unknown), (SNAPSHOT, OLD_NODE, Unknown), (REAL, USE, Unknown)]);
    assert_eq!(closed_from(&idx, REAL, USE), Err(Open::CopyUncertain(Name::from("p.Node"))));
    // a copy of unknown state may be compiled too
    let idx = index(&[(CORE, NODE, Yes), (SNAPSHOT, OLD_NODE, Unknown), (REAL, USE, Yes)]);
    assert_eq!(closed_from(&idx, REAL, USE), Err(Open::CopyUncertain(Name::from("p.Node"))));
    // a file of unknown state may compile against either
    let idx = index(&[(CORE, NODE, Yes), (SNAPSHOT, OLD_NODE, No), (REAL, USE, Unknown)]);
    assert_eq!(closed_from(&idx, REAL, USE), Err(Open::CopyUncertain(Name::from("p.Node"))));
    // two copies the build compiles (two modules): still a guess
    let idx = index(&[(CORE, NODE, Yes), ("/r/other/src/main/java/p/Node.java", NODE, Yes), (REAL, USE, Yes)]);
    assert_eq!(closed_from(&idx, REAL, USE), Err(Open::CopyUncertain(Name::from("p.Node"))));
}

/// The index holds the copy whose path sorts first; when that is the test
/// data's, the model reads it, and no claim rests on it.
#[test]
fn the_built_copy_is_certain_only_where_the_model_reads_it() {
    use InBuild::*;
    let snapshot = "/r/a-testing/src/test/test_sourcecode/p/Node.java";
    let idx = index(&[(CORE, NODE, Yes), (snapshot, OLD_NODE, No), (REAL, USE, Yes)]);
    if idx.class("java.lang.Object").is_none() {
        return;
    }
    assert_eq!(idx.source_file_of("p.Node").map(|f| &**f), Some(snapshot));
    assert_eq!(closed_from(&idx, REAL, USE), Err(Open::CopyUncertain(Name::from("p.Node"))));
}

/// jdt-core's tests import `junit.framework.ComparisonFailure`, which the
/// JUnit jar declares and grounds does not have; the only source declaring
/// it is test data, an old copy without `getActual`.
const FAILURE: &str = "package junit.framework; public class ComparisonFailure { public String getExpected() { return null; } }";
const FAILURE_PATH: &str = "/r/model/workspace/Converter/src/junit/framework/ComparisonFailure.java";
const CASE_PATH: &str = "/r/compiler/src/t/TestCase.java";
const CASE: &str = "package t; import junit.framework.ComparisonFailure; class TestCase extends ComparisonFailure { void f(ComparisonFailure c) { c.getActual(); getActual(); } static ComparisonFailure make() { return null; } void g() { Other o = make(); } } class Other {}";

fn model_of<'t>(pf: &'t ParsedFile, idx: &'t Index) -> FileModel<'t> {
    FileModel::new(pf, idx, Arc::from(CASE_PATH))
}

#[test]
fn a_class_declared_only_outside_the_build_is_not_known_in_the_build() {
    use InBuild::*;
    let outside = |asking: InBuild| index(&[(FAILURE_PATH, FAILURE, No), (CASE_PATH, CASE, asking)]);
    let idx = outside(Yes);
    if idx.class("java.lang.Object").is_none() {
        return;
    }
    let reason = || Err(Open::DeclaredOutsideBuild(Name::from("junit.framework.ComparisonFailure")));
    let pf = ParsedFile::parse(CASE);
    let fm = model_of(&pf, &idx);
    // `closed`, of the class itself and of a class whose hierarchy passes through it
    assert_eq!(fm.closed("junit.framework.ComparisonFailure", Jars::Exact).map(|_| ()), reason());
    assert_eq!(fm.closed("t.TestCase", Jars::Exact).map(|_| ()), reason());
    // `closed_scope`: an unqualified call inside the subclass
    let call = pf.root().descendants().filter(|n| n.kind() == "method_invocation").nth(1).unwrap();
    assert_eq!(fm.text(call), "getActual()");
    assert_eq!(fm.closed_scope(call, Jars::Exact).map(|_| ()), reason());
    // `closed_import_owner`: a static import of a member of it
    assert_eq!(fm.closed_import_owner(&["junit", "framework", "ComparisonFailure", "getActual"], Jars::Exact).map(|_| ()), reason());
    // `unassignable` (through `supertypes_closed`): what `make()` returns
    let value = pf.root().descendants().filter(|n| n.kind() == "method_invocation").find(|n| fm.text(*n) == "make()").unwrap();
    assert_eq!(fm.unassignable(value, &Ty::class("t.Other")), None);

    // the asking file of unknown state: as before there was a picture
    let idx = outside(Unknown);
    let fm = model_of(&pf, &idx);
    assert!(fm.closed("junit.framework.ComparisonFailure", Jars::Exact).is_ok());
    assert!(fm.closed("t.TestCase", Jars::Exact).is_ok());
    assert!(fm.closed_scope(call, Jars::Exact).is_ok());
    assert!(fm.closed_import_owner(&["junit", "framework", "ComparisonFailure", "getActual"], Jars::Exact).is_ok());
    assert!(fm.unassignable(value, &Ty::class("t.Other")).is_some());
    // and with the declaration in the build, it is the class compiled code sees
    let idx = index(&[(FAILURE_PATH, FAILURE, Yes), (CASE_PATH, CASE, Yes)]);
    let fm = model_of(&pf, &idx);
    assert!(fm.closed("t.TestCase", Jars::Exact).is_ok());
    assert!(fm.unassignable(value, &Ty::class("t.Other")).is_some());
}

/// The same with several copies, all outside the build: none is what
/// compiled code sees.
#[test]
fn copies_all_outside_the_build_are_not_known_in_the_build() {
    use InBuild::*;
    let other = "/r/model/workspace/Old/src/junit/framework/ComparisonFailure.java";
    let idx = index(&[(FAILURE_PATH, FAILURE, No), (other, FAILURE, No), (CASE_PATH, CASE, Yes)]);
    if idx.class("java.lang.Object").is_none() {
        return;
    }
    let pf = ParsedFile::parse(CASE);
    let fm = model_of(&pf, &idx);
    assert!(matches!(fm.closed("t.TestCase", Jars::Exact), Err(Open::CopyUncertain(_) | Open::DeclaredOutsideBuild(_))));
}
