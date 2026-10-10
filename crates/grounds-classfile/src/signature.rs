//! JVM descriptor and generic signature parsing (JVMS §4.3, §4.7.9.1).

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum JType {
    /// Primitive descriptor char: B C D F I J S Z V
    Base(char),
    /// Class type: internal name (with `$` for nested) and type arguments of
    /// the innermost segment plus enclosing segments' arguments flattened
    /// outer-first.
    Class { name: String, args: Vec<JTypeArg> },
    Array(Box<JType>),
    TypeVar(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum JTypeArg {
    Any,
    Exact(JType),
    Extends(JType),
    Super(JType),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JTypeParam {
    pub name: String,
    pub bounds: Vec<JType>,
}

#[derive(Clone, Debug, Default)]
pub struct ClassSig {
    pub type_params: Vec<JTypeParam>,
    pub superclass: Option<JType>,
    pub interfaces: Vec<JType>,
}

#[derive(Clone, Debug)]
pub struct MethodSig {
    pub type_params: Vec<JTypeParam>,
    pub params: Vec<JType>,
    pub ret: JType,
    pub throws: Vec<JType>,
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn ident(&mut self) -> String {
        let start = self.i;
        while let Some(c) = self.peek() {
            if matches!(c, b';' | b'<' | b'>' | b'.' | b'/' | b':' | b'[') {
                break;
            }
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }

    fn type_params(&mut self) -> Option<Vec<JTypeParam>> {
        let mut out = Vec::new();
        if !self.eat(b'<') {
            return Some(out);
        }
        while self.peek()? != b'>' {
            let name = self.ident();
            let mut bounds = Vec::new();
            // class bound (may be empty) then interface bounds
            while self.eat(b':') {
                if matches!(self.peek(), Some(b'L') | Some(b'T') | Some(b'[')) {
                    bounds.push(self.field_type()?);
                }
            }
            out.push(JTypeParam { name, bounds });
        }
        self.i += 1;
        Some(out)
    }

    fn field_type(&mut self) -> Option<JType> {
        match self.peek()? {
            b'L' => {
                self.i += 1;
                let mut name = String::new();
                let mut args = Vec::new();
                loop {
                    // package/class segments
                    let seg = self.ident();
                    name.push_str(&seg);
                    match self.peek()? {
                        b'/' => {
                            self.i += 1;
                            name.push('/');
                        }
                        b'<' => {
                            self.i += 1;
                            while self.peek()? != b'>' {
                                args.push(self.type_arg()?);
                            }
                            self.i += 1;
                        }
                        b'.' => {
                            // inner class of a parameterized type: keep only the
                            // innermost segment's arguments
                            self.i += 1;
                            name.push('$');
                            args.clear();
                        }
                        b';' => {
                            self.i += 1;
                            break;
                        }
                        _ => return None,
                    }
                }
                Some(JType::Class { name, args })
            }
            b'T' => {
                self.i += 1;
                let n = self.ident();
                self.eat(b';');
                Some(JType::TypeVar(n))
            }
            b'[' => {
                self.i += 1;
                Some(JType::Array(Box::new(self.field_type()?)))
            }
            c @ (b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' | b'V') => {
                self.i += 1;
                Some(JType::Base(c as char))
            }
            _ => None,
        }
    }

    fn type_arg(&mut self) -> Option<JTypeArg> {
        match self.peek()? {
            b'*' => {
                self.i += 1;
                Some(JTypeArg::Any)
            }
            b'+' => {
                self.i += 1;
                Some(JTypeArg::Extends(self.field_type()?))
            }
            b'-' => {
                self.i += 1;
                Some(JTypeArg::Super(self.field_type()?))
            }
            _ => Some(JTypeArg::Exact(self.field_type()?)),
        }
    }
}

pub fn parse_field_type(s: &str) -> Option<JType> {
    let mut p = P { s: s.as_bytes(), i: 0 };
    p.field_type()
}

pub fn parse_class_signature(s: &str) -> Option<ClassSig> {
    let mut p = P { s: s.as_bytes(), i: 0 };
    let type_params = p.type_params()?;
    let superclass = p.field_type();
    let mut interfaces = Vec::new();
    while p.peek().is_some() {
        interfaces.push(p.field_type()?);
    }
    Some(ClassSig { type_params, superclass, interfaces })
}

/// Parses both plain method descriptors and generic method signatures.
pub fn parse_method_signature(s: &str) -> Option<MethodSig> {
    let mut p = P { s: s.as_bytes(), i: 0 };
    let type_params = p.type_params()?;
    if !p.eat(b'(') {
        return None;
    }
    let mut params = Vec::new();
    while p.peek()? != b')' {
        params.push(p.field_type()?);
    }
    p.i += 1;
    let ret = p.field_type()?;
    let mut throws = Vec::new();
    while p.eat(b'^') {
        throws.push(p.field_type()?);
    }
    Some(MethodSig { type_params, params, ret, throws })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_sig() {
        let m = parse_method_signature("<T:Ljava/lang/Object;>(Ljava/util/List<+TT;>;I)Ljava/util/Map$Entry<TT;Ljava/lang/String;>;").unwrap();
        assert_eq!(m.type_params[0].name, "T");
        assert_eq!(m.params.len(), 2);
        match &m.ret {
            JType::Class { name, args } => {
                assert_eq!(name, "java/util/Map$Entry");
                assert_eq!(args.len(), 2);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn class_sig() {
        let c = parse_class_signature("<K:Ljava/lang/Object;V:Ljava/lang/Object;>Ljava/util/AbstractMap<TK;TV;>;Ljava/util/Map<TK;TV;>;").unwrap();
        assert_eq!(c.type_params.len(), 2);
        assert_eq!(c.interfaces.len(), 1);
        let c = parse_class_signature("<T::Ljava/lang/Comparable<TT;>;>Ljava/lang/Object;").unwrap();
        assert_eq!(c.type_params[0].bounds.len(), 1);
    }

    #[test]
    fn inner_generic() {
        let t = parse_field_type("Lcom/x/Outer<TT;>.Inner<TU;>;").unwrap();
        match t {
            JType::Class { name, args } => {
                assert_eq!(name, "com/x/Outer$Inner");
                assert_eq!(args.len(), 1);
            }
            _ => panic!(),
        }
    }
}
