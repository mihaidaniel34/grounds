/// Maps byte offsets to 1-based line/column positions.
#[derive(Clone, Debug, Default)]
pub struct LineIndex {
    starts: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineCol {
    /// 1-based line
    pub line: u32,
    /// 1-based column in characters
    pub col: u32,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i as u32 + 1);
            }
        }
        LineIndex { starts }
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// 0-based line index for an offset.
    pub fn line_of(&self, offset: u32) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        }
    }

    pub fn line_start(&self, line0: usize) -> u32 {
        self.starts[line0]
    }

    pub fn line_col(&self, text: &str, offset: u32) -> LineCol {
        let line0 = self.line_of(offset);
        let start = self.starts[line0] as usize;
        let end = (offset as usize).min(text.len());
        let col = text.get(start..end).map(|s| s.chars().count()).unwrap_or(end - start);
        LineCol { line: line0 as u32 + 1, col: col as u32 + 1 }
    }

    /// Text of the 0-based line without the trailing newline.
    pub fn line_text<'a>(&self, text: &'a str, line0: usize) -> &'a str {
        let start = self.starts[line0] as usize;
        let end = self.starts.get(line0 + 1).map(|e| *e as usize).unwrap_or(text.len());
        text[start..end].trim_end_matches(['\n', '\r'])
    }
}
