//! Syntax layer: tree-sitter based Java parsing plus small, typed-ish helpers
//! that the rest of the analyzer uses to navigate concrete syntax trees.
//!
//! Everything above this crate talks to syntax through [`ParsedFile`] and the
//! [`NodeExt`] extension trait so that the parser could be swapped later
//! without touching rules.

use std::cell::RefCell;
use std::sync::Arc;

pub use tree_sitter::{Node, Point, Tree, TreeCursor};

mod line_index;
pub use line_index::{LineCol, LineIndex};

pub fn language() -> tree_sitter::Language {
    tree_sitter_java::LANGUAGE.into()
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

/// Work around constructs tree-sitter-java cannot parse, preserving byte
/// offsets by blanking text with spaces:
/// - type annotations before varargs ellipsis: `String @Nullable ... args`
pub fn fixup_for_grammar(text: &str) -> Option<Vec<u8>> {
    if !text.contains("...") || !text.contains('@') {
        return None;
    }
    let b = text.as_bytes();
    let mut out: Option<Vec<u8>> = None;
    let mut search = 0;
    while let Some(pos) = text[search..].find("...") {
        let dots = search + pos;
        search = dots + 3;
        let mut i = dots;
        let mut spans: Vec<(usize, usize)> = Vec::new();
        loop {
            // skip whitespace
            while i > 0 && b[i - 1].is_ascii_whitespace() {
                i -= 1;
            }
            let end = i;
            let mut j = i;
            if j > 0 && b[j - 1] == b')' {
                // annotation arguments
                let mut depth = 0i32;
                while j > 0 {
                    j -= 1;
                    match b[j] {
                        b')' => depth += 1,
                        b'(' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            let name_end = j;
            while j > 0 && (b[j - 1].is_ascii_alphanumeric() || b[j - 1] == b'_' || b[j - 1] == b'.' || b[j - 1] == b'$') {
                j -= 1;
            }
            if j < name_end && j > 0 && b[j - 1] == b'@' {
                spans.push((j - 1, end));
                i = j - 1;
                continue;
            }
            break;
        }
        if spans.is_empty() {
            continue;
        }
        // must be preceded by a type token
        let mut k = i;
        while k > 0 && b[k - 1].is_ascii_whitespace() {
            k -= 1;
        }
        if k == 0 || !(b[k - 1].is_ascii_alphanumeric() || b[k - 1] == b'>' || b[k - 1] == b']' || b[k - 1] == b'_') {
            continue;
        }
        let o = out.get_or_insert_with(|| b.to_vec());
        for (s0, e0) in spans {
            for x in &mut o[s0..e0] {
                if *x != b'\n' {
                    *x = b' ';
                }
            }
        }
    }
    out
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

impl<'t> NodeExt<'t> for Node<'t> {
    fn field(&self, name: &str) -> Option<Node<'t>> {
        self.child_by_field_name(name)
    }
    fn fields(&self, name: &str) -> Vec<Node<'t>> {
        let mut c = self.walk();
        self.children_by_field_name(name, &mut c).collect()
    }
    fn kids(&self) -> Vec<Node<'t>> {
        let mut c = self.walk();
        self.named_children(&mut c).filter(|n| !is_comment(n.kind())).collect()
    }
    fn first_kid(&self) -> Option<Node<'t>> {
        let mut c = self.walk();
        let r = self.named_children(&mut c).find(|n| !is_comment(n.kind()));
        r
    }
    fn kid_of_kind(&self, kind: &str) -> Option<Node<'t>> {
        let mut c = self.walk();
        let r = self.named_children(&mut c).find(|n| n.kind() == kind);
        r
    }
    fn kids_of_kind(&self, kind: &str) -> Vec<Node<'t>> {
        let mut c = self.walk();
        self.named_children(&mut c).filter(|n| n.kind() == kind).collect()
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
                let r = m.children(&mut c).any(|k| !k.is_named() && k.kind() == keyword);
                r
            }
            None => false,
        }
    }
    fn text_range(&self) -> TextRange {
        TextRange::of(*self)
    }
    fn is_kind(&self, kind: &str) -> bool {
        self.kind() == kind
    }
    fn descendants(&self) -> Descendants<'t> {
        Descendants { stack: vec![*self] }
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
pub struct Descendants<'t> {
    stack: Vec<Node<'t>>,
}

impl<'t> Iterator for Descendants<'t> {
    type Item = Node<'t>;
    fn next(&mut self) -> Option<Node<'t>> {
        let n = self.stack.pop()?;
        let kids = n.kids();
        for k in kids.into_iter().rev() {
            self.stack.push(k);
        }
        Some(n)
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
