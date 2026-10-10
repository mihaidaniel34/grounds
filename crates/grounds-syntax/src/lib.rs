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
/// offsets (node text is read from the original source):
/// - type annotations before varargs ellipsis: `String @Nullable ... args`
///   (blanked with spaces)
/// - qualified record patterns, `case Shape.Circle(double r)` and
///   `o instanceof Shape.Circle(var r)`, which the grammar parses as method
///   calls: the dots are replaced with `$`, so the parser sees one identifier
///   whose original text is still `Shape.Circle`
pub fn fixup_for_grammar(text: &str) -> Option<Vec<u8>> {
    let varargs = fixup_varargs_annotations(text);
    let patterns = fixup_record_patterns(text, varargs);
    fixup_text_blocks(text, patterns)
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

fn fixup_record_patterns(text: &str, mut out: Option<Vec<u8>>) -> Option<Vec<u8>> {
    let b = text.as_bytes();
    let n = b.len();
    // identifier characters; non-ASCII bytes are parts of Unicode identifiers
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80;
    let lone_underscore = |i: usize| b[i] == b'_' && (i == 0 || !ident(b[i - 1])) && (i + 1 >= n || !ident(b[i + 1]));
    if !(text.contains("case") || text.contains("instanceof") || (0..n).any(lone_underscore)) {
        return out;
    }
    let code = code_mask(b);
    let word_at = |i: usize, w: &[u8]| code[i] && i + w.len() <= n && &b[i..i + w.len()] == w && (i == 0 || !ident(b[i - 1])) && (i + w.len() == n || !ident(b[i + w.len()]));
    // in a pattern region, turn `A.B(` into `A$B(` (code bytes only)
    let patch = |start: usize, end: usize, out: &mut Option<Vec<u8>>| {
        let mut i = start;
        while i < end {
            if code[i] && ident(b[i]) && (i == 0 || !(ident(b[i - 1]) || b[i - 1] == b'.')) {
                let s0 = i;
                let mut dots = Vec::new();
                let mut j = i;
                loop {
                    while j < end && code[j] && ident(b[j]) {
                        j += 1;
                    }
                    if j + 1 < end && code[j] && b[j] == b'.' && code[j + 1] && ident(b[j + 1]) {
                        dots.push(j);
                        j += 1;
                        continue;
                    }
                    break;
                }
                let mut k = j;
                while k < end && (b[k].is_ascii_whitespace() || !code[k]) {
                    k += 1;
                }
                if !dots.is_empty() && k < end && code[k] && b[k] == b'(' && b[s0].is_ascii_alphabetic() {
                    let o = out.get_or_insert_with(|| b.to_vec());
                    for d in dots {
                        o[d] = b'$';
                    }
                }
                i = j.max(i + 1);
            } else {
                i += 1;
            }
        }
    };
    let mut i = 0;
    while i < n {
        if !code[i] {
            i += 1;
            continue;
        }
        if word_at(i, b"case") {
            // the pattern part of the label: up to `->`, `:` or `when` at depth 0
            let start = i + 4;
            let mut j = start;
            let mut depth = 0i32;
            while j < n {
                if code[j] {
                    match b[j] {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        b'-' if depth == 0 && j + 1 < n && b[j + 1] == b'>' => break,
                        b':' if depth == 0 => break,
                        b';' | b'{' | b'}' if depth <= 0 => break,
                        _ if depth == 0 && word_at(j, b"when") => break,
                        _ => {}
                    }
                }
                j += 1;
            }
            patch(start, j, &mut out);
            for k in start..j {
                if code[k] && lone_underscore(k) {
                    out.get_or_insert_with(|| b.to_vec())[k] = b'$';
                }
                // `case final X x`, `case R(final var y)`: the grammar has no
                // modifiers in patterns; `final` changes nothing for analysis
                if word_at(k, b"final") {
                    out.get_or_insert_with(|| b.to_vec())[k..k + 5].fill(b' ');
                }
            }
            // Java 22 unnamed patterns, possibly several per label
            // (`case Boolean _, Byte _ ->`): the grammar supports neither.
            // They bind nothing, so keep the first and blank the others.
            if (start..j).any(|k| code[k] && lone_underscore(k)) {
                let mut depth = 0i32;
                let comma = (start..j).find(|&k| {
                    if !code[k] {
                        return false;
                    }
                    match b[k] {
                        b'(' | b'<' => depth += 1,
                        b')' | b'>' => depth -= 1,
                        b',' if depth == 0 => return true,
                        _ => {}
                    }
                    false
                });
                if let Some(c) = comma {
                    let o = out.get_or_insert_with(|| b.to_vec());
                    for x in &mut o[c..j] {
                        if *x != b'\n' {
                            *x = b' ';
                        }
                    }
                }
            }
            i = j;
            continue;
        }
        if word_at(i, b"instanceof") {
            // `instanceof [final] A.B(...)`: the record pattern and its body
            let mut j = i + 10;
            while j < n && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if word_at(j, b"final") {
                out.get_or_insert_with(|| b.to_vec())[j..j + 5].fill(b' ');
                j += 5;
            }
            let start = j;
            while j < n && code[j] && (ident(b[j]) || b[j] == b'.' || b[j].is_ascii_whitespace()) {
                j += 1;
            }
            if j < n && code[j] && b[j] == b'(' {
                let mut depth = 0i32;
                while j < n {
                    if code[j] {
                        match b[j] {
                            b'(' => depth += 1,
                            b')' => {
                                depth -= 1;
                                if depth == 0 {
                                    j += 1;
                                    break;
                                }
                            }
                            b';' | b'{' => break,
                            _ => {}
                        }
                    }
                    j += 1;
                }
                patch(start, j.min(n), &mut out);
                for k in start..j.min(n) {
                    if word_at(k, b"final") {
                        out.get_or_insert_with(|| b.to_vec())[k..k + 5].fill(b' ');
                    }
                }
            }
            i += 10;
            continue;
        }
        // unnamed variables `_` (patterns, lambdas, catch, locals) become `$`
        if lone_underscore(i) {
            let o = out.get_or_insert_with(|| b.to_vec());
            o[i] = b'$';
        }
        i += 1;
    }
    out
}

fn fixup_varargs_annotations(text: &str) -> Option<Vec<u8>> {
    if !text.contains("...") || !text.contains('@') {
        return None;
    }
    let b = text.as_bytes();
    let code = code_mask(b);
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
        // only real code: never blank across strings or comments
        if spans.is_empty() || !code[dots] || spans.iter().any(|(s0, e0)| (*s0..*e0).any(|k| !code[k])) {
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
