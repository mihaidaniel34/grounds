//! Parsers for embedded mini-languages: java.util.Formatter format strings,
//! SLF4J placeholders, java.util.regex syntax and date/time patterns.

#[derive(Debug, Clone, PartialEq)]
pub struct FormatSpec {
    /// Explicit 1-based index, or None for ordinary/relative.
    pub index: Option<usize>,
    pub relative: bool,
    pub conversion: char,
    /// For %t/%T, the date conversion suffix.
    pub date_suffix: Option<char>,
}

#[derive(Debug)]
pub struct ParsedFormat {
    pub specs: Vec<FormatSpec>,
    /// Number of arguments required.
    pub required: usize,
}

/// Parse a java.util.Formatter format string.
pub fn parse_format(s: &str) -> Result<ParsedFormat, String> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut specs = Vec::new();
    let mut ordinary = 0usize;
    let mut max_explicit = 0usize;
    while i < chars.len() {
        if chars[i] != '%' {
            i += 1;
            continue;
        }
        i += 1;
        if i >= chars.len() {
            return Err("format string ends with a lone '%'".into());
        }
        // argument index
        let start = i;
        let mut index = None;
        let mut relative = false;
        let mut j = i;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        if j < chars.len() && chars[j] == '$' && j > start {
            let n: usize = chars[start..j].iter().collect::<String>().parse().map_err(|_| "bad index")?;
            if n == 0 {
                return Err("argument index 0 is invalid (indices start at 1)".into());
            }
            index = Some(n);
            i = j + 1;
        } else if chars[i] == '<' {
            relative = true;
            i += 1;
        }
        // flags (`<` may also appear among them: `%-<5s`)
        while i < chars.len() && "-#+ 0,(<".contains(chars[i]) {
            if chars[i] == '<' {
                relative = true;
            }
            i += 1;
        }
        // width
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        // precision
        if i < chars.len() && chars[i] == '.' {
            i += 1;
            let ps = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if ps == i {
                return Err("missing precision after '.'".into());
            }
        }
        if i >= chars.len() {
            return Err("incomplete format specifier at end of string".into());
        }
        let conv = chars[i];
        i += 1;
        let mut date_suffix = None;
        match conv {
            '%' | 'n' => continue,
            'b' | 'B' | 'h' | 'H' | 's' | 'S' | 'c' | 'C' | 'd' | 'o' | 'x' | 'X' | 'e' | 'E' | 'f' | 'g' | 'G' | 'a' | 'A' => {}
            't' | 'T' => {
                if i >= chars.len() {
                    return Err("missing date/time conversion after %t".into());
                }
                let d = chars[i];
                if !"HIklMSLNpzZsQBbhAaCYyjmdeRTrDFc".contains(d) {
                    return Err(format!("unknown date/time conversion '%t{d}'"));
                }
                date_suffix = Some(d);
                i += 1;
            }
            other => return Err(format!("unknown format conversion '%{other}'")),
        }
        if relative {
            if specs.is_empty() {
                return Err("'%<' used without a previous specifier".into());
            }
        } else if let Some(n) = index {
            max_explicit = max_explicit.max(n);
        } else {
            ordinary += 1;
        }
        specs.push(FormatSpec { index, relative, conversion: conv, date_suffix });
    }
    Ok(ParsedFormat { required: ordinary.max(max_explicit), specs })
}

/// Map each spec to the 0-based argument it consumes.
pub fn spec_args(p: &ParsedFormat) -> Vec<(usize, &FormatSpec)> {
    let mut out = Vec::new();
    let mut ordinary = 0;
    let mut last: Option<usize> = None;
    for s in &p.specs {
        let idx = if s.relative {
            match last {
                Some(l) => l,
                None => continue,
            }
        } else if let Some(n) = s.index {
            n - 1
        } else {
            ordinary += 1;
            ordinary - 1
        };
        last = Some(idx);
        out.push((idx, s));
    }
    out
}

/// Count SLF4J-style `{}` placeholders (ignoring `\{}`).
pub fn count_placeholders(s: &str) -> usize {
    let b = s.as_bytes();
    let mut n = 0;
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'{' && b[i + 1] == b'}' {
            let escaped = i > 0 && b[i - 1] == b'\\' && !(i > 1 && b[i - 2] == b'\\');
            if !escaped {
                n += 1;
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    n
}

/// Validate a regular expression against java.util.regex syntax rules
/// (conservatively: only reports constructs Java definitely rejects).
pub fn check_java_regex(p: &str) -> Result<(), String> {
    let c: Vec<char> = p.chars().collect();
    let n = c.len();
    let mut i = 0;
    let mut groups: Vec<usize> = Vec::new();
    // whether a quantifier may follow (an atom precedes)
    let mut can_quantify = false;
    let mut after_quantifier = false;
    while i < n {
        let ch = c[i];
        match ch {
            '\\' => {
                if i + 1 >= n {
                    return Err("trailing backslash".into());
                }
                let e = c[i + 1];
                i += 2;
                match e {
                    'Q' => {
                        // quote until \E or end
                        let mut j = i;
                        while j + 1 < n && !(c[j] == '\\' && c[j + 1] == 'E') {
                            j += 1;
                        }
                        i = if j + 1 < n { j + 2 } else { n };
                    }
                    'p' | 'P' => {
                        if i < n && c[i] == '{' {
                            match c[i..].iter().position(|&x| x == '}') {
                                Some(k) => i += k + 1,
                                None => return Err("unclosed character property".into()),
                            }
                        } else if i < n {
                            i += 1;
                        } else {
                            return Err("incomplete \\p escape".into());
                        }
                    }
                    'x' => {
                        if i < n && c[i] == '{' {
                            match c[i..].iter().position(|&x| x == '}') {
                                Some(k) => i += k + 1,
                                None => return Err("unclosed hexadecimal escape".into()),
                            }
                        } else {
                            let hex = c[i..].iter().take(2).filter(|x| x.is_ascii_hexdigit()).count();
                            if hex < 2 {
                                return Err("illegal hexadecimal escape sequence".into());
                            }
                            i += 2;
                        }
                    }
                    'u' => {
                        let hex = c[i..].iter().take(4).filter(|x| x.is_ascii_hexdigit()).count();
                        if hex < 4 {
                            return Err("illegal Unicode escape sequence".into());
                        }
                        i += 4;
                    }
                    'k' => {
                        if i < n && c[i] == '<' {
                            match c[i..].iter().position(|&x| x == '>') {
                                Some(k) => i += k + 1,
                                None => return Err("unclosed named back reference".into()),
                            }
                        } else {
                            return Err("\\k is not followed by '<' for named capturing group".into());
                        }
                    }
                    'c' => {
                        if i >= n {
                            return Err("illegal control escape sequence".into());
                        }
                        i += 1;
                    }
                    'N' => {
                        if i < n && c[i] == '{' {
                            match c[i..].iter().position(|&x| x == '}') {
                                Some(k) => i += k + 1,
                                None => return Err("unclosed character name escape".into()),
                            }
                        }
                    }
                    'b' if i < n && c[i] == '{' => match c[i..].iter().position(|&x| x == '}') {
                        Some(k) => i += k + 1,
                        None => return Err("unclosed \\b{} escape".into()),
                    },
                    e if e.is_ascii_alphabetic()
                        && !"tnrfaedDsSwWbBAGZzhHvVRXEgN0123456789".contains(e) => {
                            return Err(format!("illegal/unsupported escape sequence '\\{e}'"));
                        }
                    _ => {}
                }
                can_quantify = true;
                after_quantifier = false;
            }
            '[' => {
                i = skip_class(&c, i)?;
                can_quantify = true;
                after_quantifier = false;
            }
            '(' => {
                groups.push(i);
                i += 1;
                if i < n && c[i] == '?' {
                    i += 1;
                    if i >= n {
                        return Err("unknown inline modifier".into());
                    }
                    match c[i] {
                        ':' | '=' | '!' | '>' => i += 1,
                        '<' => {
                            i += 1;
                            if i < n && (c[i] == '=' || c[i] == '!') {
                                i += 1;
                            } else {
                                // named group
                                let start = i;
                                while i < n && c[i].is_ascii_alphanumeric() {
                                    i += 1;
                                }
                                if i >= n || c[i] != '>' || i == start {
                                    return Err("named capturing group is missing trailing '>'".into());
                                }
                                if !c[start].is_ascii_alphabetic() {
                                    return Err("capturing group name must start with a letter".into());
                                }
                                i += 1;
                            }
                        }
                        _ => {
                            // flags (?idmsuxU-idmsuxU) or (?flags:...)
                            while i < n && "idmsuxUc-".contains(c[i]) {
                                i += 1;
                            }
                            if i >= n {
                                return Err("unclosed group".into());
                            }
                            if c[i] == ')' {
                                groups.pop();
                                i += 1;
                                can_quantify = false;
                                after_quantifier = false;
                                continue;
                            }
                            if c[i] != ':' {
                                return Err(format!("unknown inline modifier '{}'", c[i]));
                            }
                            i += 1;
                        }
                    }
                }
                can_quantify = false;
                after_quantifier = false;
            }
            ')' => {
                if groups.pop().is_none() {
                    return Err("unmatched closing ')'".into());
                }
                i += 1;
                can_quantify = true;
                after_quantifier = false;
            }
            '|' => {
                i += 1;
                can_quantify = false;
                after_quantifier = false;
            }
            '*' | '+' | '?' => {
                if after_quantifier && (ch == '?' || ch == '+') {
                    // lazy / possessive modifier
                    after_quantifier = false;
                    i += 1;
                    continue;
                }
                if !can_quantify {
                    return Err(format!("dangling meta character '{ch}'"));
                }
                i += 1;
                can_quantify = false;
                after_quantifier = true;
            }
            '{' => {
                // must be a valid repetition {n}, {n,}, {n,m}
                let close = c[i..].iter().position(|&x| x == '}');
                let Some(k) = close else { return Err("illegal repetition (unclosed '{')".into()) };
                let body: String = c[i + 1..i + k].iter().collect();
                let ok = {
                    let parts: Vec<&str> = body.splitn(2, ',').collect();
                    let a_ok = !parts[0].is_empty() && parts[0].chars().all(|x| x.is_ascii_digit());
                    let b_ok = parts.len() == 1 || parts[1].chars().all(|x| x.is_ascii_digit());
                    if a_ok && b_ok && parts.len() == 2 && !parts[1].is_empty() {
                        let a: u64 = parts[0].parse().unwrap_or(0);
                        let b: u64 = parts[1].parse().unwrap_or(0);
                        if a > b {
                            return Err(format!("illegal repetition range {{{body}}}"));
                        }
                    }
                    a_ok && b_ok
                };
                if !ok {
                    return Err(format!("illegal repetition '{{{body}}}'"));
                }
                if !can_quantify {
                    return Err("dangling repetition '{'".into());
                }
                i += k + 1;
                can_quantify = false;
                after_quantifier = true;
            }
            _ => {
                i += 1;
                can_quantify = true;
                after_quantifier = false;
            }
        }
    }
    if !groups.is_empty() {
        return Err("unclosed group".into());
    }
    Ok(())
}

fn skip_class(c: &[char], start: usize) -> Result<usize, String> {
    let n = c.len();
    let mut i = start + 1;
    if i < n && c[i] == '^' {
        i += 1;
    }
    // a ']' right after '[' or '[^' is literal
    if i < n && c[i] == ']' {
        i += 1;
    }
    let mut depth = 1;
    while i < n {
        match c[i] {
            '\\' => {
                if i + 1 >= n {
                    return Err("trailing backslash in character class".into());
                }
                if c[i + 1] == 'Q' {
                    let mut j = i + 2;
                    while j + 1 < n && !(c[j] == '\\' && c[j + 1] == 'E') {
                        j += 1;
                    }
                    i = j + 2;
                    continue;
                }
                i += 2;
            }
            '[' => {
                depth += 1;
                i += 1;
                if i < n && c[i] == '^' {
                    i += 1;
                }
            }
            ']' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            _ => i += 1,
        }
    }
    Err("unclosed character class".into())
}

/// Characters with special meaning in a regex (outside classes).
pub fn has_regex_meta(s: &str) -> bool {
    s.chars().any(|c| ".$|()[]{}^?*+\\".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(parse_format("%s and %d%%").unwrap().required, 2);
        assert_eq!(parse_format("%2$s %1$s").unwrap().required, 2);
        assert_eq!(parse_format("%s %<s").unwrap().required, 1);
        assert_eq!(parse_format("%-10.3f%n").unwrap().required, 1);
        assert_eq!(parse_format("%tY-%<tm").unwrap().required, 1);
        assert!(parse_format("%q").is_err());
        assert!(parse_format("100%").is_err());
    }

    #[test]
    fn placeholders() {
        assert_eq!(count_placeholders("a {} b {}"), 2);
        assert_eq!(count_placeholders("a \\{} b {}"), 1);
        assert_eq!(count_placeholders("{x}"), 0);
    }

    #[test]
    fn regexes() {
        for ok in ["a+b*", "(?i)abc", "[a-z]+", "\\d{2,4}", "(?<year>\\d{4})", "a*?", "a++", "[]a]", "\\Q*\\E", "(?:x|y)", "\\p{Alpha}", "[^\\]]", "x{3}", "\\.", "a|b", "(?=x)", "^$"] {
            assert!(check_java_regex(ok).is_ok(), "{ok}");
        }
        for bad in ["*a", "a(b", "a)b", "[abc", "a{", "${user}", "a{3,1}", "\\", "\\y", "+", "(?<1x>a)", "a|*"] {
            assert!(check_java_regex(bad).is_err(), "{bad}");
        }
    }
}
