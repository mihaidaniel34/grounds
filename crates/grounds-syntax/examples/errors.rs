//! Print where tree-sitter could not parse a file: `cargo run --example errors -- File.java...`

use grounds_syntax::ParsedFile;

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let pf = ParsedFile::parse(text.clone());
        let mut stack = vec![pf.root()];
        while let Some(n) = stack.pop() {
            if n.is_error() || n.is_missing() {
                let p = n.start_position();
                let line = text.lines().nth(p.row).unwrap_or("").trim();
                let what = if n.is_missing() { format!("missing {}", n.kind()) } else { "error".to_string() };
                println!("{path}:{}:{}: {what}: {}", p.row + 1, p.column + 1, line.chars().take(140).collect::<String>());
                continue;
            }
            if n.has_error() {
                let mut c = n.walk();
                stack.extend(n.children(&mut c));
            }
        }
    }
}
