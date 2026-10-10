fn main() {
    let path = std::env::args().nth(1).unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    let f = grounds_syntax::ParsedFile::parse(text);
    println!("{}", f.root().to_sexp());
}
