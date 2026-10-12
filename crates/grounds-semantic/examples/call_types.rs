//! Print the type grounds gives each call starting with a prefix, one file at
//! a time with only the JDK on the classpath (the overload fuzzer's grounds
//! side): `cargo run --example call_types -- pick( A.java B.java ...`
//! Output: `path<TAB>type` per call; `?unknown` (possibly nested) means unsure.

use grounds_index::{extract, Classpath, Index};
use grounds_semantic::FileModel;
use grounds_syntax::{NodeExt, ParsedFile};
use std::sync::Arc;

fn main() {
    let mut args = std::env::args().skip(1);
    let prefix = args.next().expect("usage: call_types PREFIX FILE...");
    let release = std::env::var("JAVA_RELEASE").ok().and_then(|r| r.parse().ok()).unwrap_or(21);
    let cp = Arc::new(Classpath::build(grounds_classfile::find_java_home().as_deref(), release, &[]));
    // `@list` reads paths from a file (thousands of cases)
    let paths: Vec<String> = args
        .flat_map(|a| match a.strip_prefix('@') {
            Some(list) => std::fs::read_to_string(list).unwrap_or_default().lines().map(str::to_string).collect(),
            None => vec![a],
        })
        .collect();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let pf = ParsedFile::parse(text);
        let mut idx = Index::new(cp.clone());
        let key: Arc<str> = Arc::from(path.as_str());
        idx.set_file(key.clone(), Arc::new(extract(&pf)));
        let fm = FileModel::new(&pf, &idx, key);
        for n in pf.root().descendants().filter(|n| n.kind() == "method_invocation") {
            if fm.text(n).starts_with(&prefix) {
                println!("{path}\t{}", fm.type_of(n));
            }
        }
    }
}
