use crate::index::Index;
use crate::model::simple_name_of;
use crate::stubs::{ConstExpr, FileStub, RawAnnValue, RawAnnotation, TypeSyntax};
use rustc_hash::FxHashMap;
use std::sync::Arc;

#[derive(Debug, Default, Clone)]
pub struct ResolveStats {
    pub files: usize,
    pub waves: usize,
    pub group_sizes: Vec<usize>,
    // classes resolved outside their group, 0 means the result is deterministic
    pub outside: usize,
}

// one node per source file, sorted by path so the numbering is the same every run
// an edge a -> b means resolving a's classes may read a class declared in file b (see names_resolution_reads)
pub fn file_graph(index: &Index) -> (Vec<Arc<str>>, Vec<Vec<u32>>) {
    let mut files: Vec<(&Arc<str>, &Arc<FileStub>)> = index.files().collect();
    files.sort_by(|a, b| a.0.cmp(b.0));

    // simple class name
    let mut declared_in: FxHashMap<&str, Vec<u32>> = FxHashMap::default();
    for (i, (_, stub)) in files.iter().enumerate() {
        for c in &stub.classes {
            declared_in.entry(simple_name_of(&c.name)).or_default().push(i as u32);
        }
    }

    let file_index: FxHashMap<&str, u32> = files.iter().enumerate().map(|(i, (p, _))| (&***p, i as u32)).collect();

    // every name resolution may read points to the files declaring it
    let edges = files
        .iter()
        .enumerate()
        .map(|(i, (path, stub))| {
            let mut out: Vec<u32> = names_resolution_reads(stub)
                .into_iter()
                .filter_map(|n| declared_in.get(n))
                .flatten()
                .copied()
                // a class declared in several files (the same class in two modules) belongs to
                // one of them: this file's nested classes may need an outer class that lives there
                .chain(stub.classes.iter().filter_map(|c| index.source_file_of(&c.name)).filter(|p| *p != *path).filter_map(|p| file_index.get(&**p).copied()))
                .filter(|&j| j != i as u32)
                .collect();
            out.sort_unstable();
            out.dedup();
            out
        })
        .collect();

    (files.into_iter().map(|(p, _)| p.clone()).collect(), edges)
}

// supertypes, static and on-demand imports, qualifiers of qualified type name and names in constants, annotation values
fn names_resolution_reads(stub: &FileStub) -> Vec<&str> {
    let mut out = Vec::new();
    for c in &stub.classes {
        for t in c.superclass.iter().chain(&c.interfaces) {
            supertype_names(t, &mut out);
        }
        for t in &c.permits {
            qualifiers(t, &mut out);
        }
        for tp in &c.type_params {
            for t in &tp.bounds {
                qualifiers(t, &mut out);
            }
        }
        for a in &c.annotations {
            annotation_names(a, &mut out);
        }
        for f in &c.fields {
            qualifiers(&f.ty, &mut out);
            for a in &f.annotations {
                annotation_names(a, &mut out);
            }
            if let Some(e) = &f.init {
                constant_names(e, &mut out);
            }
        }
        for m in &c.methods {
            for tp in &m.type_params {
                for t in &tp.bounds {
                    qualifiers(t, &mut out);
                }
            }
            for p in &m.params {
                qualifiers(&p.ty, &mut out);
                for a in &p.annotations {
                    annotation_names(a, &mut out);
                }
            }
            qualifiers(&m.ret, &mut out);
            for t in &m.throws {
                qualifiers(t, &mut out);
            }
            for a in &m.annotations {
                annotation_names(a, &mut out);
            }
        }
    }
    // import static a.X.Foo & import a.Outer.* look inside X and Outer
    for imp in stub.imports.iter().filter(|i| i.is_static || i.on_demand) {
        out.extend(imp.path.split('.'));
    }
    out
}

fn supertype_names<'a>(t: &'a TypeSyntax, out: &mut Vec<&'a str>) {
    match t {
        TypeSyntax::Named(segs) => {
            for (n, args) in segs {
                out.push(n);
                for a in args {
                    qualifiers(a, out);
                }
            }
        }
        _ => qualifiers(t, out),
    }
}

fn qualifiers<'a>(t: &'a TypeSyntax, out: &mut Vec<&'a str>) {
    match t {
        TypeSyntax::Named(segs) => {
            for (n, _) in &segs[..segs.len().saturating_sub(1)] {
                out.push(n);
            }
            for (_, args) in segs {
                for a in args {
                    qualifiers(a, out);
                }
            }
        }
        TypeSyntax::Array(e) => qualifiers(e, out),
        TypeSyntax::Wildcard(Some((_, bound))) => qualifiers(bound, out),
        _ => {}
    }
}

fn constant_names<'a>(e: &'a ConstExpr, out: &mut Vec<&'a str>) {
    match e {
        ConstExpr::Name(n) => out.extend(n.split('.')),
        ConstExpr::Unary(_, x) | ConstExpr::Cast(_, x) | ConstExpr::CastString(x) => constant_names(x, out),
        ConstExpr::Binary(_, x, y) => {
            constant_names(x, out);
            constant_names(y, out);
        }
        ConstExpr::Ternary(x, y, z) => {
            constant_names(x, out);
            constant_names(y, out);
            constant_names(z, out);
        }
        ConstExpr::Lit(_) => {}
    }
}

fn annotation_names<'a>(a: &'a RawAnnotation, out: &mut Vec<&'a str>) {
    let segs: Vec<&str> = a.name.split('.').collect();
    out.extend(&segs[..segs.len() - 1]);
    for (_, v) in &a.args {
        value_names(v, out);
    }
}

fn value_names<'a>(v: &'a RawAnnValue, out: &mut Vec<&'a str>) {
    match v {
        RawAnnValue::Name(n) => out.extend(n.split('.')),
        RawAnnValue::ClassLit(t) => qualifiers(t, out),
        RawAnnValue::Annotation(a) => annotation_names(a, out),
        RawAnnValue::Array(vs) => {
            for x in vs {
                value_names(x, out);
            }
        }
        RawAnnValue::Const(_) | RawAnnValue::Expr(_) => {}
    }
}

// strongly connected components
pub fn components(edges: &[Vec<u32>]) -> Vec<Vec<u32>> {
    const UNSEEN: u32 = u32::MAX;
    let n = edges.len();
    let mut order = vec![UNSEEN; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut out: Vec<Vec<u32>> = Vec::new();
    let mut next = 0u32;

    for start in 0..n as u32 {
        if order[start as usize] != UNSEEN {
            continue;
        }
        // own call stack
        let mut calls: Vec<(u32, usize)> = vec![(start, 0)];
        order[start as usize] = next;
        low[start as usize] = next;
        next += 1;
        stack.push(start);
        on_stack[start as usize] = true;

        while let Some(&(v, pos)) = calls.last() {
            let vu = v as usize;
            if pos < edges[vu].len() {
                calls.last_mut().unwrap().1 += 1;
                let w = edges[vu][pos];
                let wu = w as usize;
                if order[wu] == UNSEEN {
                    order[wu] = next;
                    low[wu] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[wu] = true;
                    calls.push((w, 0));
                } else if on_stack[wu] {
                    low[vu] = low[vu].min(order[wu]);
                }
                continue;
            }
            // all edges of v followed
            calls.pop();
            if low[vu] == order[vu] {
                // v is the first file of a group
                let mut group = Vec::new();
                loop {
                    let w = stack.pop().unwrap();
                    on_stack[w as usize] = false;
                    group.push(w);
                    if w == v {
                        break;
                    }
                }
                group.sort_unstable();
                out.push(group);
            }
            // tell the caller how far back v reaches
            if let Some(&(parent, _)) = calls.last() {
                low[parent as usize] = low[parent as usize].min(low[vu]);
            }
        }
    }
    out
}

// groups in the same wave never need each other => can be resolved at the same time
pub fn waves(groups: &[Vec<u32>], edges: &[Vec<u32>]) -> Vec<Vec<usize>> {
    // which group each file is in
    let mut group_of = vec![0usize; edges.len()];
    for (g, files) in groups.iter().enumerate() {
        for &f in files {
            group_of[f as usize] = g;
        }
    }
    let mut wave = vec![0usize; groups.len()];
    for (g, files) in groups.iter().enumerate() {
        let w = files
            .iter()
            .flat_map(|&f| &edges[f as usize])
            .map(|&t| group_of[t as usize])
            .filter(|&h| h != g)
            .map(|h| wave[h] + 1)
            .max()
            .unwrap_or(0);
        wave[g] = w;
    }
    let mut out = vec![Vec::new(); wave.iter().max().map_or(0, |m| m + 1)];
    for (g, &w) in wave.iter().enumerate() {
        out[w].push(g);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Classpath;
    use crate::stubs::extract;
    use grounds_syntax::ParsedFile;

    fn index(files: &[(&str, &str)]) -> Index {
        let mut idx = Index::new(Arc::new(Classpath::empty()));
        for (path, src) in files {
            idx.set_file(Arc::from(*path), Arc::new(extract(&ParsedFile::parse(*src))));
        }
        idx
    }

    #[test]
    fn a_file_points_to_the_files_its_resolution_reads() {
        let idx = index(&[
            ("a/A.java", "package a; class A extends B { C c; }"),
            ("a/B.java", "package a; class B { static final int LIMIT = D.MAX; }"),
            ("a/C.java", "package a; class C {}"),
            ("a/D.java", "package a; class D { static final int MAX = 1; }"),
            ("a/E.java", "package a; import static a.C.*; class E {}"),
            ("a/F.java", "package a; class F { G.Inner x; }"),
            ("a/G.java", "package a; class G { static class Inner {} }"),
        ]);
        let (files, edges) = file_graph(&idx);
        assert_eq!(files.len(), 7);
        // A -> B (extends; the field type C is no edge), B -> D (constant),
        // E -> C (static import), F -> G (qualifier of G.Inner)
        assert_eq!(edges, [vec![1], vec![3], vec![], vec![], vec![2], vec![6], vec![]]);
    }

    #[test]
    fn a_chain_gives_one_group_per_file_dependencies_first() {
        // 0 -> 1 -> 2
        assert_eq!(components(&[vec![1], vec![2], vec![]]), [vec![2], vec![1], vec![0]]);
    }

    #[test]
    fn files_that_need_each_other_form_one_group() {
        // 0 <-> 1
        assert_eq!(components(&[vec![1], vec![0]]), [vec![0, 1]]);
    }

    #[test]
    fn a_cycle_comes_before_the_file_that_needs_it() {
        // 0 -> 1 <-> 2
        assert_eq!(components(&[vec![1], vec![2], vec![1]]), [vec![1, 2], vec![0]]);
    }

    #[test]
    fn unrelated_files_are_separate_groups() {
        assert_eq!(components(&[vec![], vec![]]), [vec![0], vec![1]]);
    }

    #[test]
    fn a_diamond_gives_three_waves() {
        // 0 -> 1 -> 3, 0 -> 2 -> 3
        let edges = [vec![1, 2], vec![3], vec![3], vec![]];
        let groups = components(&edges);
        assert_eq!(groups, [vec![3], vec![1], vec![2], vec![0]]);
        assert_eq!(waves(&groups, &edges), [vec![0], vec![1, 2], vec![3]]);
    }

    #[test]
    fn a_cycle_is_one_unit_in_its_wave() {
        // 0 -> 1 <-> 2
        let edges = [vec![1], vec![2], vec![1]];
        let groups = components(&edges);
        assert_eq!(waves(&groups, &edges), [vec![0], vec![1]]);
    }

    #[test]
    fn a_class_declared_twice_points_to_the_file_the_index_keeps() {
        // the same class in two modules, only one copy with a nested class: resolving
        // `Dup$Inner` reads `Dup`, which the index may take from the other file
        let mut checked = false;
        for (first, second) in [("m1/p/Dup.java", "m2/p/Dup.java"), ("m2/p/Dup.java", "m1/p/Dup.java")] {
            let with_inner = "package p; public class Dup { static class Inner {} }";
            let idx = index(&[(first, with_inner), (second, "package p; public class Dup {}")]);
            let (files, edges) = file_graph(&idx);
            let at = |p: &str| files.iter().position(|f| &**f == p).unwrap();
            let kept = at(idx.source_file_of("p.Dup").unwrap());
            if kept != at(first) {
                assert!(edges[at(first)].contains(&(kept as u32)), "{edges:?}");
                checked = true;
            }
        }
        assert!(checked, "the index never kept the copy without the nested class");
    }

    #[test]
    fn ordered_resolution_gives_the_same_classes_on_any_number_of_threads() {
        let sources = [
            ("a/A.java", "package a; public class A extends B { public static final int SIZE = C.LIMIT; Inner i; }"),
            ("a/B.java", "package a; public class B { A a; public static class Inner {} }"),
            ("a/C.java", "package a; public class C { public static final int LIMIT = 10; A a; }"),
            ("a/D.java", "package a; public class D { C c; }"),
            ("b/E.java", "package b; class E { F f; }"),
            ("b/F.java", "package b; class F { E e; }"),
        ];
        let resolve = |threads: usize| {
            let idx = index(&sources);
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            assert_eq!(pool.install(|| idx.resolve_ordered()).outside, 0);
            let mut names: Vec<_> = idx.source_class_names().cloned().collect();
            names.sort();
            names.iter().map(|n| idx.class(n)).collect::<Vec<_>>()
        };
        let one = resolve(1);
        for threads in [2, 8] {
            for _ in 0..5 {
                assert_eq!(resolve(threads), one);
            }
        }
    }
}
