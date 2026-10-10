use crate::analyses::*;
use crate::*;
use grounds_index::*;
use grounds_semantic::FileModel;
use grounds_syntax::{NodeExt, ParsedFile};
use std::sync::{Arc, OnceLock};

fn jdk() -> Arc<Classpath> {
    static CP: OnceLock<Arc<Classpath>> = OnceLock::new();
    CP.get_or_init(|| Arc::new(Classpath::build(grounds_classfile::find_java_home().as_deref(), 21, &[]))).clone()
}

fn with_method(src: &str, f: impl for<'t> FnOnce(&FileModel<'t>, &Cfg<'t>)) {
    let pf = ParsedFile::parse(src);
    let mut idx = Index::new(jdk());
    idx.set_file(Arc::from("T.java"), Arc::new(extract(&pf)));
    let fm = FileModel::new(&pf, &idx, Arc::from("T.java"));
    let m = pf.root().descendants().find(|n| n.kind() == "method_declaration").unwrap();
    let cfg = build(m, &pf.text);
    f(&fm, &cfg);
}

#[test]
fn cfg_shapes() {
    with_method("class T { int m(int x) { if (x > 0 && x < 10) { return 1; } while (x > 0) { x--; if (x == 5) break; } return 0; } }", |_, cfg| {
        let r = cfg.reachable();
        assert!(r[cfg.exit]);
        assert!(cfg.blocks.len() > 8);
    });
}

#[test]
fn liveness_dead_store() {
    with_method("class T { int m() { int a = 1; a = 2; int b = a; b = 3; return a; } }", |fm, cfg| {
        let lv = Liveness { fm };
        let res = solve(cfg, &lv);
        let mut dead = Vec::new();
        for e in cfg.elements() {
            if let Some((i, true)) = local_def(fm, e) {
                let after = res.after_backward(cfg, &lv, e).unwrap();
                if !after.contains(&i) {
                    dead.push(fm.text(e).to_string());
                }
            }
        }
        assert_eq!(dead, vec!["a = 1", "b = a", "b = 3"]);
    });
}

#[test]
fn nullness_paths() {
    with_method(
        "class T { void m(String s, String t) { if (s == null) { System.out.println(s.length()); } String u = null; if (t != null) { u = t; } u.trim(); String v = t; if (v != null) return; v.isEmpty(); } }",
        |fm, cfg| {
            let na = NullnessAnalysis { fm, cfg };
            let res = solve(cfg, &na);
            let mut hits = Vec::new();
            res.for_each_before(cfg, &na, |e, f| {
                for (i, n) in na.derefs(e) {
                    if let Some(s) = f.get(&i) {
                        if !matches!(*s, Nullness::NonNull | Nullness::MaybeAssignedNull) {
                            hits.push(format!("{}:{:?}", fm.text(n), s));
                        }
                    }
                }
            });
            assert_eq!(hits, vec!["s:Null", "v:Null"]);
        },
    );
}

#[test]
fn resources() {
    with_method(
        "import java.io.*; class T { void m(File f) throws IOException { InputStream a = new FileInputStream(f); InputStream b = new FileInputStream(f); b.close(); InputStream c = new FileInputStream(f); keep(c); Reader r = new StringReader(\"x\"); } void keep(Object o) {} }",
        |fm, cfg| {
            let ra = ResourceAnalysis { fm };
            let res = solve(cfg, &ra);
            let exit = &res.block_in[cfg.exit];
            let open: Vec<String> = exit
                .iter()
                .filter(|(_, s)| matches!(s, ResState::Open(_)))
                .map(|(i, _)| fm.binder.decls[*i].name.to_string())
                .collect();
            assert_eq!(open, vec!["a"]);
        },
    );
}
