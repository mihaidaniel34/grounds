//! Syntax layer: tree-sitter based Java parsing plus small, typed-ish helpers
//! that the rest of the analyzer uses to navigate concrete syntax trees.
//!
//! Everything above this crate talks to syntax through [`ParsedFile`] and the
//! [`NodeExt`] extension trait so that the parser could be swapped later
//! without touching rules.

use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::sync::{Arc, OnceLock};

pub use tree_sitter::{Node, Point, Tree, TreeCursor};

mod line_index;
pub use line_index::{LineCol, LineIndex};

pub fn language() -> tree_sitter::Language {
    tree_sitter_java_orchard::LANGUAGE.into()
}

/// Field and node-kind ids of the grammar, looked up by name once: by name,
/// tree-sitter compares the name against its whole field table on every
/// call, and `Node::kind()` re-reads a C string (strlen, UTF-8 check).
struct Grammar {
    fields: FxHashMap<&'static str, u16>,
    /// named kind -> ids (aliases can give one name several ids)
    kinds: FxHashMap<&'static str, Vec<u16>>,
    comments: Vec<u16>,
}

fn grammar() -> &'static Grammar {
    static G: OnceLock<Grammar> = OnceLock::new();
    G.get_or_init(|| {
        let lang: &'static tree_sitter::Language = Box::leak(Box::new(language()));
        let fields = (1..=lang.field_count() as u16).filter_map(|id| lang.field_name_for_id(id).map(|n| (n, id))).collect();
        let mut kinds: FxHashMap<&'static str, Vec<u16>> = FxHashMap::default();
        for id in 0..lang.node_kind_count() as u16 {
            if lang.node_kind_is_named(id) {
                if let Some(n) = lang.node_kind_for_id(id) {
                    kinds.entry(n).or_default().push(id);
                }
            }
        }
        let comments = ["line_comment", "block_comment"].iter().flat_map(|k| kinds.get(k).cloned().unwrap_or_default()).collect();
        Grammar { fields, kinds, comments }
    })
}

fn is_comment_node(n: &Node) -> bool {
    grammar().comments.contains(&n.kind_id())
}

/// Ids of a named kind (empty for a name the grammar does not have).
fn kind_ids(kind: &str) -> &'static [u16] {
    grammar().kinds.get(kind).map(|v| v.as_slice()).unwrap_or(&[])
}

thread_local! {
    static PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&language()).expect("java grammar");
        p
    });
}

/// Byte range inside a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}

impl TextRange {
    pub fn new(start: usize, end: usize) -> Self {
        TextRange { start: start as u32, end: end as u32 }
    }
    pub fn of(node: Node) -> Self {
        TextRange::new(node.start_byte(), node.end_byte())
    }
    pub fn contains(&self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }
    pub fn len(&self) -> u32 {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// A parsed compilation unit.
pub struct ParsedFile {
    pub text: Arc<str>,
    pub tree: Tree,
    pub lines: LineIndex,
}

impl std::fmt::Debug for ParsedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedFile").field("len", &self.text.len()).finish()
    }
}

#[derive(Clone, Debug)]
pub struct SyntaxError {
    pub range: TextRange,
    pub message: String,
}

impl ParsedFile {
    pub fn parse(text: impl Into<Arc<str>>) -> ParsedFile {
        let text: Arc<str> = text.into();
        let fixed = fixup_for_grammar(&text);
        let bytes = fixed.as_deref().unwrap_or(text.as_bytes());
        let tree = PARSER.with(|p| p.borrow_mut().parse(bytes, None)).expect("parse never fails without timeout");
        let lines = LineIndex::new(&text);
        ParsedFile { text, tree, lines }
    }

    pub fn root(&self) -> Node<'_> {
        self.tree.root_node()
    }

    pub fn text_of(&self, node: Node) -> &str {
        &self.text[node.start_byte()..node.end_byte()]
    }

    pub fn slice(&self, range: TextRange) -> &str {
        &self.text[range.start as usize..range.end as usize]
    }

    pub fn has_errors(&self) -> bool {
        self.root().has_error()
    }

    /// Collect ERROR / MISSING nodes as syntax errors.
    pub fn syntax_errors(&self) -> Vec<SyntaxError> {
        let mut out = Vec::new();
        if !self.has_errors() {
            return out;
        }
        let mut stack = vec![self.root()];
        while let Some(n) = stack.pop() {
            if n.is_missing() {
                out.push(SyntaxError { range: TextRange::of(n), message: format!("missing `{}`", n.kind()) });
                continue;
            }
            if n.is_error() {
                let snippet: String = self.text_of(n).chars().take(40).collect();
                out.push(SyntaxError { range: TextRange::of(n), message: format!("syntax error near `{}`", snippet.trim()) });
                continue;
            }
            if n.has_error() {
                let mut c = n.walk();
                for ch in n.children(&mut c) {
                    stack.push(ch);
                }
            }
        }
        out.sort_by_key(|e| e.range.start);
        out
    }
}

/// Work around constructs the grammar cannot parse, preserving byte offsets
/// (node text is read from the original source). Only text blocks need it
/// with tree-sitter-java-orchard; qualified record patterns, `final` and `_`
/// in patterns, and annotated varargs parse natively.
pub fn fixup_for_grammar(text: &str) -> Option<Vec<u8>> {
    fixup_text_blocks(text, None)
}

/// tree-sitter-java lexes `//` inside a text block as a comment start
/// (`"""\n  //comment"""` swallows the closing quotes): in the parse copy,
/// slashes inside text blocks become `x`. String values are read from the
/// original source, so rules still see the real content.
fn fixup_text_blocks(text: &str, mut out: Option<Vec<u8>>) -> Option<Vec<u8>> {
    if !text.contains("\"\"\"") || !text.contains('/') {
        return out;
    }
    let b = text.as_bytes();
    let n = b.len();
    let code = code_mask(b);
    let mut i = 0;
    while i + 2 < n {
        // a text block starts at a non-code `"""` whose previous byte is code
        if b[i] == b'"' && b[i + 1] == b'"' && b[i + 2] == b'"' && !code[i] && (i == 0 || code[i - 1]) {
            let mut j = i + 3;
            while j < n && !code[j] {
                j += 1;
            }
            for k in i + 3..j.saturating_sub(3) {
                if b[k] == b'/' {
                    out.get_or_insert_with(|| b.to_vec())[k] = b'x';
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Which bytes are code (not inside a comment, string, text block or char
/// literal).
fn code_mask(b: &[u8]) -> Vec<bool> {
    let n = b.len();
    let mut code = vec![true; n];
    let mut i = 0;
    let mark = |from: usize, to: usize, code: &mut Vec<bool>| {
        for c in &mut code[from..to.min(n)] {
            *c = false;
        }
    };
    while i < n {
        let start = i;
        match b[i] {
            b'/' if i + 1 < n && b[i + 1] == b'/' => {
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
                mark(start, i, &mut code);
            }
            b'/' if i + 1 < n && b[i + 1] == b'*' => {
                i += 2;
                while i + 1 < n && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(n);
                mark(start, i, &mut code);
            }
            b'"' if i + 2 < n && b[i + 1] == b'"' && b[i + 2] == b'"' => {
                i += 3;
                while i < n && !(i + 2 < n && b[i] == b'"' && b[i + 1] == b'"' && b[i + 2] == b'"') {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 3).min(n);
                mark(start, i, &mut code);
            }
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < n && b[i] != q && b[i] != b'\n' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 1).min(n);
                mark(start, i, &mut code);
            }
            _ => i += 1,
        }
    }
    code
}

pub fn is_comment(kind: &str) -> bool {
    kind == "line_comment" || kind == "block_comment"
}

/// Convenience accessors on tree-sitter nodes.
pub trait NodeExt<'t>: Sized {
    fn field(&self, name: &str) -> Option<Node<'t>>;
    fn fields(&self, name: &str) -> Vec<Node<'t>>;
    /// Named children, excluding comments.
    fn kids(&self) -> Vec<Node<'t>>;
    fn first_kid(&self) -> Option<Node<'t>>;
    fn kid_of_kind(&self, kind: &str) -> Option<Node<'t>>;
    fn kids_of_kind(&self, kind: &str) -> Vec<Node<'t>>;
    fn text<'s>(&self, src: &'s str) -> &'s str;
    fn ancestors(&self) -> Ancestors<'t>;
    fn ancestor_of_kind(&self, kinds: &[&str]) -> Option<Node<'t>>;
    /// Strip enclosing parentheses: `((x))` -> `x`.
    fn unparen(&self) -> Node<'t>;
    /// Parent skipping parenthesized expressions.
    fn parent_unparen(&self) -> Option<Node<'t>>;
    fn modifiers(&self) -> Option<Node<'t>>;
    fn has_modifier(&self, keyword: &str) -> bool;
    fn text_range(&self) -> TextRange;
    fn is_kind(&self, kind: &str) -> bool;
    fn descendants(&self) -> Descendants<'t>;
}

/// The keyword of a child of `modifiers`: a bare token (`public`), or the
/// token inside a `visibility` / `modifier` node, as the grammar wraps them.
pub fn modifier_keyword<'t>(k: Node<'t>) -> Option<&'t str> {
    if !k.is_named() {
        return Some(k.kind());
    }
    if matches!(k.kind(), "visibility" | "modifier") {
        return k.child(0).filter(|t| !t.is_named()).map(|t| t.kind());
    }
    None
}

impl<'t> NodeExt<'t> for Node<'t> {
    fn field(&self, name: &str) -> Option<Node<'t>> {
        let id = *grammar().fields.get(name)?;
        self.child_by_field_id(id)
    }
    fn fields(&self, name: &str) -> Vec<Node<'t>> {
        let Some(&id) = grammar().fields.get(name) else { return Vec::new() };
        let mut c = self.walk();
        self.children_by_field_id(std::num::NonZeroU16::new(id).expect("field ids start at 1"), &mut c).collect()
    }
    fn kids(&self) -> Vec<Node<'t>> {
        let mut c = self.walk();
        self.named_children(&mut c).filter(|n| !is_comment_node(n)).collect()
    }
    fn first_kid(&self) -> Option<Node<'t>> {
        let mut c = self.walk();
        let r = self.named_children(&mut c).find(|n| !is_comment_node(n));
        r
    }
    fn kid_of_kind(&self, kind: &str) -> Option<Node<'t>> {
        let ids = kind_ids(kind);
        let mut c = self.walk();
        let r = self.named_children(&mut c).find(|n| if ids.is_empty() { n.kind() == kind } else { ids.contains(&n.kind_id()) });
        r
    }
    fn kids_of_kind(&self, kind: &str) -> Vec<Node<'t>> {
        let ids = kind_ids(kind);
        let mut c = self.walk();
        self.named_children(&mut c).filter(|n| if ids.is_empty() { n.kind() == kind } else { ids.contains(&n.kind_id()) }).collect()
    }
    fn text<'s>(&self, src: &'s str) -> &'s str {
        &src[self.start_byte()..self.end_byte()]
    }
    fn ancestors(&self) -> Ancestors<'t> {
        Ancestors { cur: self.parent() }
    }
    fn ancestor_of_kind(&self, kinds: &[&str]) -> Option<Node<'t>> {
        self.ancestors().find(|n| kinds.contains(&n.kind()))
    }
    fn unparen(&self) -> Node<'t> {
        let mut n = *self;
        while n.kind() == "parenthesized_expression" {
            match n.first_kid() {
                Some(k) => n = k,
                None => break,
            }
        }
        n
    }
    fn parent_unparen(&self) -> Option<Node<'t>> {
        let mut p = self.parent()?;
        while p.kind() == "parenthesized_expression" {
            p = p.parent()?;
        }
        Some(p)
    }
    fn modifiers(&self) -> Option<Node<'t>> {
        self.kid_of_kind("modifiers")
    }
    fn has_modifier(&self, keyword: &str) -> bool {
        match self.modifiers() {
            Some(m) => {
                let mut c = m.walk();
                let r = m.children(&mut c).any(|k| modifier_keyword(k) == Some(keyword));
                r
            }
            None => false,
        }
    }
    fn text_range(&self) -> TextRange {
        TextRange::of(*self)
    }
    fn is_kind(&self, kind: &str) -> bool {
        let ids = kind_ids(kind);
        if ids.is_empty() || !self.is_named() {
            return self.kind() == kind;
        }
        ids.contains(&self.kind_id())
    }
    fn descendants(&self) -> Descendants<'t> {
        Descendants { cursor: self.walk(), depth: 0, started: false }
    }
}

pub struct Ancestors<'t> {
    cur: Option<Node<'t>>,
}

impl<'t> Iterator for Ancestors<'t> {
    type Item = Node<'t>;
    fn next(&mut self) -> Option<Node<'t>> {
        let n = self.cur?;
        self.cur = n.parent();
        Some(n)
    }
}

/// Pre-order traversal over named, non-comment descendants (including self).
/// One cursor for the whole walk: no child list per node.
pub struct Descendants<'t> {
    cursor: TreeCursor<'t>,
    /// depth below the start node; the walk ends when it would leave it
    depth: u32,
    started: bool,
}

impl<'t> Descendants<'t> {
    /// Next node in pre-order (any node), or None when the subtree is done.
    fn advance(&mut self) -> Option<Node<'t>> {
        if !self.started {
            self.started = true;
            return Some(self.cursor.node());
        }
        // into the first child, unless this is an unnamed token or a comment
        let n = self.cursor.node();
        if (n.is_named() && !is_comment_node(&n)) && self.cursor.goto_first_child() {
            self.depth += 1;
            return Some(self.cursor.node());
        }
        loop {
            if self.depth == 0 {
                return None;
            }
            if self.cursor.goto_next_sibling() {
                return Some(self.cursor.node());
            }
            self.cursor.goto_parent();
            self.depth -= 1;
        }
    }
}

impl<'t> Iterator for Descendants<'t> {
    type Item = Node<'t>;
    fn next(&mut self) -> Option<Node<'t>> {
        loop {
            let n = self.advance()?;
            // the start node itself is yielded whatever it is, as before
            if self.depth == 0 || (n.is_named() && !is_comment_node(&n)) {
                return Some(n);
            }
        }
    }
}

/// Visit all named descendants with the ability to skip subtrees.
pub fn walk<'t>(root: Node<'t>, mut f: impl FnMut(Node<'t>) -> WalkControl) {
    let mut cursor = root.walk();
    let mut descend = true;
    loop {
        if descend {
            let node = cursor.node();
            let ctl = if node.is_named() && !is_comment(node.kind()) { f(node) } else { WalkControl::Continue };
            if ctl == WalkControl::Continue && cursor.goto_first_child() {
                continue;
            }
        }
        if cursor.goto_next_sibling() {
            descend = true;
            continue;
        }
        if !cursor.goto_parent() {
            return;
        }
        if cursor.node() == root {
            return;
        }
        descend = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkControl {
    Continue,
    SkipChildren,
}

/// Is this node a declaration that introduces a new class-like scope?
pub fn is_type_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration" | "annotation_type_declaration"
    )
}

pub fn is_method_like(kind: &str) -> bool {
    matches!(kind, "method_declaration" | "constructor_declaration" | "compact_constructor_declaration" | "lambda_expression")
}

/// Text of a string literal with escapes decoded; `None` for text blocks with
/// interpolation or malformed input.
pub fn string_literal_value(node: Node, src: &str) -> Option<String> {
    if node.kind() != "string_literal" {
        return None;
    }
    let raw = node.text(src);
    let mut out = String::new();
    let mut c = node.walk();
    let mut any = false;
    for ch in node.named_children(&mut c) {
        any = true;
        match ch.kind() {
            "string_fragment" | "multiline_string_fragment" => out.push_str(ch.text(src)),
            "escape_sequence" => out.push_str(&decode_escape(ch.text(src))),
            "string_interpolation" => return None,
            _ => {}
        }
    }
    if !any {
        // "" or """ """ with only whitespace
        if raw.starts_with("\"\"\"") {
            return Some(String::new());
        }
        return Some(String::new());
    }
    if raw.starts_with("\"\"\"") {
        return Some(strip_text_block_indent(&out));
    }
    Some(out)
}

fn strip_text_block_indent(s: &str) -> String {
    // Approximation of JLS text block processing: drop the first line break,
    // strip common indentation.
    let s = s.strip_prefix('\n').unwrap_or(s);
    let lines: Vec<&str> = s.split('\n').collect();
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines.iter().map(|l| if l.len() >= indent { &l[indent..] } else { l.trim_start() }).collect::<Vec<_>>().join("\n")
}

pub fn decode_escape(esc: &str) -> String {
    let b = esc.as_bytes();
    if b.len() < 2 {
        return esc.to_string();
    }
    match b[1] {
        b'n' => "\n".into(),
        b't' => "\t".into(),
        b'r' => "\r".into(),
        b'b' => "\u{8}".into(),
        b'f' => "\u{c}".into(),
        b's' => " ".into(),
        b'0'..=b'7' => {
            let v = u32::from_str_radix(&esc[1..], 8).unwrap_or(0);
            char::from_u32(v).map(|c| c.to_string()).unwrap_or_default()
        }
        b'u' => {
            let hex = esc.trim_start_matches('\\').trim_start_matches('u');
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(|c| c.to_string()).unwrap_or_default()
        }
        b'\n' => String::new(),
        c => (c as char).to_string(),
    }
}

/// Decode a char literal like `'a'` or `'\n'`.
pub fn char_literal_value(node: Node, src: &str) -> Option<char> {
    let t = node.text(src);
    let inner = t.strip_prefix('\'')?.strip_suffix('\'')?;
    if inner.starts_with('\\') {
        decode_escape(inner).chars().next()
    } else {
        inner.chars().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_walks() {
        let f = ParsedFile::parse("class A { void m() { int x = 1; } }");
        assert!(!f.has_errors());
        let kinds: Vec<_> = f.root().descendants().map(|n| n.kind()).collect();
        assert!(kinds.contains(&"local_variable_declaration"));
        let mut n = 0;
        walk(f.root(), |_| {
            n += 1;
            WalkControl::Continue
        });
        assert_eq!(n, kinds.len());
    }

    #[test]
    fn annotated_varargs() {
        let f = ParsedFile::parse("interface B { B env(String @Nullable ... envs); }");
        assert!(!f.has_errors());
        let sp = f.root().descendants().find(|n| n.kind() == "spread_parameter");
        assert!(sp.is_some());
    }

    #[test]
    fn modifiers_and_strings() {
        let f = ParsedFile::parse("class A { private static final String S = \"a\\tb\"; }");
        let field = f.root().descendants().find(|n| n.kind() == "field_declaration").unwrap();
        assert!(field.has_modifier("static"));
        assert!(!field.has_modifier("public"));
        let lit = f.root().descendants().find(|n| n.kind() == "string_literal").unwrap();
        assert_eq!(string_literal_value(lit, &f.text).unwrap(), "a\tb");
    }
}

#[cfg(test)]
mod fixup_tests {
    use super::*;

    fn kinds(src: &str) -> String {
        ParsedFile::parse(src).root().to_sexp()
    }

    #[test]
    fn qualified_record_patterns_parse() {
        let t = kinds("class T { Object a(Object o) { if (o instanceof A.B(var x)) return x; return switch (o) { case A.B(A.C(var y), var z) when z.isEmpty() -> y; default -> null; }; } }");
        assert!(!t.contains("ERROR") && t.matches("record_pattern ").count() == 3, "{t}");
    }

    #[test]
    fn literals_in_case_labels_are_not_brackets() {
        let src = "class T { void m(char c, StringBuilder s) { switch (c) { case '(', '[', '{' -> { s.append(c); } case '\"' -> s.append(c); default -> {} } } }";
        let pf = ParsedFile::parse(src);
        let t = pf.root().to_sexp();
        assert!(!t.contains("ERROR") && !t.contains("record_pattern"), "{t}");
        assert!(fixup_for_grammar(src).is_none_or(|b| !String::from_utf8_lossy(&b).contains("s$append")));
    }
}

#[cfg(test)]
mod unnamed_tests {
    use super::*;

    #[test]
    fn unnamed_patterns_and_variables_parse() {
        let src = "class T { Object m(Object v) { try { return switch (v) { case Boolean _, Byte _ -> 1; case java.util.Collection<?> _, java.util.Map<?, ?> _ when v != null -> 2; case 1, 2 -> 3; default -> 4; }; } catch (RuntimeException _) { java.util.function.BiFunction<Object, Object, Object> f = (_, b) -> b; return f; } } }";
        let t = ParsedFile::parse(src).root().to_sexp();
        assert!(!t.contains("ERROR"), "{t}");
    }
}

#[cfg(test)]
mod fixup_review_tests {
    use super::*;

    fn clean(src: &str) {
        let t = ParsedFile::parse(src).root().to_sexp();
        assert!(!t.contains("ERROR"), "{t}");
    }

    #[test]
    fn strings_comments_and_unicode_do_not_trigger_rewrites() {
        // `_` and `,` inside a case-label string
        clean("class T { int f(String s) { return switch (s) { case \"_,x\" -> 1; default -> 0; }; } int g() { return 1; } }");
        // a parenthesis in a comment inside a label must not extend the region
        let src = "class T { void run() {} void f(int n) { switch (n) { case 1 /* ( */ -> this.run(); } } }";
        clean(src);
        assert!(fixup_for_grammar(src).is_none_or(|b| !String::from_utf8_lossy(&b).contains("this$run")));
        // `case` as the suffix of a Unicode identifier is not the keyword
        let src = "class T { int run() { return 1; } void f() { int πcase = this.run(); } }";
        clean(src);
        assert!(fixup_for_grammar(src).is_none_or(|b| !String::from_utf8_lossy(&b).contains("this$run")));
        // text blocks and char literals
        clean("class T { String s = \"\"\"\n case A.B( _ , x\n \"\"\"; char c = '_'; int x_1 = 1; }");
    }
}

#[cfg(test)]
mod varargs_mask_tests {
    use super::*;

    #[test]
    fn varargs_fixup_stays_out_of_literals() {
        let src = "class T { void f() { String a = \"String @A(\"; String s = null; s.length(); String b = \")...\"; } void g(String @A ... xs) {} }";
        let fixed = fixup_for_grammar(src).map(|b| String::from_utf8(b).unwrap()).unwrap_or_else(|| src.to_string());
        assert!(fixed.contains("s.length()") && fixed.contains("\"String @A(\""), "{fixed}");
        assert!(!ParsedFile::parse(src).root().to_sexp().contains("ERROR"));
    }
}

#[cfg(test)]
mod final_and_text_block_tests {
    use super::*;

    #[test]
    fn final_in_patterns_and_slashes_in_text_blocks() {
        let src = "class T { int f(Object o) { if (o instanceof final String s) return 1; return switch (o) { case final Integer i -> i; case R(final var x, final int y) -> 2; default -> 0; }; } String t = \"\"\"\n\n        //comment\"\"\"; int after() { return 1; } }";
        let t = ParsedFile::parse(src).root().to_sexp();
        assert!(!t.contains("ERROR"), "{t}");
        // the original text (and so the string value) is untouched
        assert!(ParsedFile::parse(src).text.contains("//comment"));
    }
}
