//! Class file parser. Reads declarations only: constant pool, members,
//! signatures, annotations and a few other attributes. Method bodies (`Code`)
//! are skipped entirely.

use anyhow::{bail, Result};

pub mod access {
    pub const PUBLIC: u16 = 0x0001;
    pub const PRIVATE: u16 = 0x0002;
    pub const PROTECTED: u16 = 0x0004;
    pub const STATIC: u16 = 0x0008;
    pub const FINAL: u16 = 0x0010;
    pub const SYNCHRONIZED: u16 = 0x0020;
    pub const SUPER: u16 = 0x0020;
    pub const VOLATILE: u16 = 0x0040;
    pub const BRIDGE: u16 = 0x0040;
    pub const TRANSIENT: u16 = 0x0080;
    pub const VARARGS: u16 = 0x0080;
    pub const NATIVE: u16 = 0x0100;
    pub const INTERFACE: u16 = 0x0200;
    pub const ABSTRACT: u16 = 0x0400;
    pub const STRICT: u16 = 0x0800;
    pub const SYNTHETIC: u16 = 0x1000;
    pub const ANNOTATION: u16 = 0x2000;
    pub const ENUM: u16 = 0x4000;
    pub const MODULE: u16 = 0x8000;
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConstValue {
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ElementValue {
    Const(char, ConstValue),
    Enum { type_desc: String, name: String },
    Class(String),
    Annotation(Annotation),
    Array(Vec<ElementValue>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    /// Field descriptor, e.g. `Ljava/lang/Deprecated;`
    pub type_desc: String,
    pub elements: Vec<(String, ElementValue)>,
    pub visible: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FieldInfo {
    pub access: u16,
    pub name: String,
    pub descriptor: String,
    pub signature: Option<String>,
    pub constant: Option<ConstValue>,
    pub annotations: Vec<Annotation>,
    pub deprecated: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MethodInfo {
    pub access: u16,
    pub name: String,
    pub descriptor: String,
    pub signature: Option<String>,
    pub exceptions: Vec<String>,
    pub annotations: Vec<Annotation>,
    pub param_annotations: Vec<Vec<Annotation>>,
    pub param_names: Vec<String>,
    pub annotation_default: Option<ElementValue>,
    pub deprecated: bool,
}

#[derive(Clone, Debug)]
pub struct InnerClass {
    pub inner: String,
    pub outer: Option<String>,
    pub simple_name: Option<String>,
    pub access: u16,
}

#[derive(Clone, Debug, Default)]
pub struct ClassFile {
    pub major: u16,
    pub access: u16,
    /// Internal name, e.g. `java/util/Map$Entry`
    pub this_class: String,
    pub super_class: Option<String>,
    pub interfaces: Vec<String>,
    pub signature: Option<String>,
    pub fields: Vec<FieldInfo>,
    pub methods: Vec<MethodInfo>,
    pub annotations: Vec<Annotation>,
    pub inner_classes: Vec<InnerClass>,
    pub permitted_subclasses: Vec<String>,
    pub record_components: Vec<(String, String, Option<String>)>,
    pub deprecated: bool,
    pub source_file: Option<String>,
}

impl ClassFile {
    /// Access flags for this class as recorded in the InnerClasses attribute
    /// (which carries `static`/`private` etc for nested classes).
    pub fn effective_access(&self) -> u16 {
        for ic in &self.inner_classes {
            if ic.inner == self.this_class {
                return ic.access;
            }
        }
        self.access
    }
}

#[derive(Clone, Debug)]
enum Cp {
    Empty,
    Utf8(String),
    Int(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    Class(u16),
    String(u16),
    Other,
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8> {
        if self.p >= self.b.len() {
            bail!("eof");
        }
        let v = self.b[self.p];
        self.p += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16> {
        if self.p + 2 > self.b.len() {
            bail!("eof");
        }
        let v = u16::from_be_bytes([self.b[self.p], self.b[self.p + 1]]);
        self.p += 2;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32> {
        if self.p + 4 > self.b.len() {
            bail!("eof");
        }
        let v = u32::from_be_bytes([self.b[self.p], self.b[self.p + 1], self.b[self.p + 2], self.b[self.p + 3]]);
        self.p += 4;
        Ok(v)
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.p + n > self.b.len() {
            bail!("eof");
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
}

pub fn decode_modified_utf8(b: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(b) {
        return s.to_string();
    }
    let mut units: Vec<u16> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c & 0x80 == 0 {
            units.push(c as u16);
            i += 1;
        } else if c & 0xE0 == 0xC0 && i + 1 < b.len() {
            units.push((((c & 0x1F) as u16) << 6) | (b[i + 1] & 0x3F) as u16);
            i += 2;
        } else if c & 0xF0 == 0xE0 && i + 2 < b.len() {
            units.push((((c & 0x0F) as u16) << 12) | (((b[i + 1] & 0x3F) as u16) << 6) | (b[i + 2] & 0x3F) as u16);
            i += 3;
        } else {
            units.push(0xFFFD);
            i += 1;
        }
    }
    String::from_utf16_lossy(&units)
}

struct Parser<'a> {
    r: Reader<'a>,
    cp: Vec<Cp>,
}

impl<'a> Parser<'a> {
    fn utf8(&self, idx: u16) -> Result<String> {
        match self.cp.get(idx as usize) {
            Some(Cp::Utf8(s)) => Ok(s.clone()),
            _ => bail!("expected utf8 at {idx}"),
        }
    }
    fn class_name(&self, idx: u16) -> Result<String> {
        match self.cp.get(idx as usize) {
            Some(Cp::Class(n)) => self.utf8(*n),
            _ => bail!("expected class at {idx}"),
        }
    }
    fn const_value(&self, idx: u16) -> Option<ConstValue> {
        match self.cp.get(idx as usize)? {
            Cp::Int(v) => Some(ConstValue::Int(*v)),
            Cp::Long(v) => Some(ConstValue::Long(*v)),
            Cp::Float(v) => Some(ConstValue::Float(*v)),
            Cp::Double(v) => Some(ConstValue::Double(*v)),
            Cp::String(s) => self.utf8(*s).ok().map(ConstValue::String),
            Cp::Utf8(s) => Some(ConstValue::String(s.clone())),
            _ => None,
        }
    }

    fn parse_constant_pool(&mut self) -> Result<()> {
        let count = self.r.u16()? as usize;
        self.cp = Vec::with_capacity(count);
        self.cp.push(Cp::Empty);
        let mut i = 1;
        while i < count {
            let tag = self.r.u8()?;
            let entry = match tag {
                1 => {
                    let len = self.r.u16()? as usize;
                    Cp::Utf8(decode_modified_utf8(self.r.bytes(len)?))
                }
                3 => Cp::Int(self.r.u32()? as i32),
                4 => Cp::Float(f32::from_bits(self.r.u32()?)),
                5 => {
                    let hi = self.r.u32()? as u64;
                    let lo = self.r.u32()? as u64;
                    Cp::Long(((hi << 32) | lo) as i64)
                }
                6 => {
                    let hi = self.r.u32()? as u64;
                    let lo = self.r.u32()? as u64;
                    Cp::Double(f64::from_bits((hi << 32) | lo))
                }
                7 => Cp::Class(self.r.u16()?),
                8 => Cp::String(self.r.u16()?),
                9 | 10 | 11 | 12 | 17 | 18 => {
                    self.r.u32()?;
                    Cp::Other
                }
                15 => {
                    self.r.bytes(3)?;
                    Cp::Other
                }
                16 | 19 | 20 => {
                    self.r.u16()?;
                    Cp::Other
                }
                t => bail!("bad constant pool tag {t}"),
            };
            let wide = matches!(entry, Cp::Long(_) | Cp::Double(_));
            self.cp.push(entry);
            i += 1;
            if wide {
                self.cp.push(Cp::Empty);
                i += 1;
            }
        }
        Ok(())
    }

    fn element_value(&self, r: &mut Reader) -> Result<ElementValue> {
        let tag = r.u8()? as char;
        Ok(match tag {
            'B' | 'C' | 'D' | 'F' | 'I' | 'J' | 'S' | 'Z' | 's' => {
                let idx = r.u16()?;
                ElementValue::Const(tag, self.const_value(idx).unwrap_or(ConstValue::Int(0)))
            }
            'e' => {
                let t = r.u16()?;
                let n = r.u16()?;
                ElementValue::Enum { type_desc: self.utf8(t)?, name: self.utf8(n)? }
            }
            'c' => ElementValue::Class(self.utf8(r.u16()?)?),
            '@' => ElementValue::Annotation(self.annotation(r, true)?),
            '[' => {
                let n = r.u16()?;
                let mut v = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    v.push(self.element_value(r)?);
                }
                ElementValue::Array(v)
            }
            _ => bail!("bad element value tag {tag}"),
        })
    }

    fn annotation(&self, r: &mut Reader, visible: bool) -> Result<Annotation> {
        let t = r.u16()?;
        let n = r.u16()?;
        let mut elements = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let name = self.utf8(r.u16()?)?;
            let v = self.element_value(r)?;
            elements.push((name, v));
        }
        Ok(Annotation { type_desc: self.utf8(t)?, elements, visible })
    }

    fn annotations(&self, data: &[u8], visible: bool) -> Result<Vec<Annotation>> {
        let mut r = Reader { b: data, p: 0 };
        let n = r.u16()?;
        let mut out = Vec::with_capacity(n as usize);
        for _ in 0..n {
            out.push(self.annotation(&mut r, visible)?);
        }
        Ok(out)
    }

    fn param_annotations(&self, data: &[u8], visible: bool) -> Result<Vec<Vec<Annotation>>> {
        let mut r = Reader { b: data, p: 0 };
        let n = r.u8()?;
        let mut out = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let k = r.u16()?;
            let mut v = Vec::with_capacity(k as usize);
            for _ in 0..k {
                v.push(self.annotation(&mut r, visible)?);
            }
            out.push(v);
        }
        Ok(out)
    }

    /// Iterate attributes, calling `f(name, data)`.
    fn attributes(&mut self, mut f: impl FnMut(&Self, &str, &'a [u8]) -> Result<()>) -> Result<()> {
        let n = self.r.u16()?;
        for _ in 0..n {
            let name_idx = self.r.u16()?;
            let len = self.r.u32()? as usize;
            let data = self.r.bytes(len)?;
            let name = match self.cp.get(name_idx as usize) {
                Some(Cp::Utf8(s)) => s.clone(),
                _ => continue,
            };
            // Errors in non-essential attributes are ignored.
            let _ = f(self, &name, data);
        }
        Ok(())
    }
}

pub fn parse_class(bytes: &[u8]) -> Result<ClassFile> {
    let mut p = Parser { r: Reader { b: bytes, p: 0 }, cp: Vec::new() };
    if p.r.u32()? != 0xCAFEBABE {
        bail!("not a class file");
    }
    let _minor = p.r.u16()?;
    let major = p.r.u16()?;
    p.parse_constant_pool()?;
    let mut cf = ClassFile { major, ..Default::default() };
    cf.access = p.r.u16()?;
    let this = p.r.u16()?;
    cf.this_class = p.class_name(this)?;
    let sup = p.r.u16()?;
    cf.super_class = if sup == 0 { None } else { Some(p.class_name(sup)?) };
    let ni = p.r.u16()?;
    for _ in 0..ni {
        let i = p.r.u16()?;
        cf.interfaces.push(p.class_name(i)?);
    }

    let nf = p.r.u16()?;
    for _ in 0..nf {
        let access = p.r.u16()?;
        let ni = p.r.u16()?;
        let di = p.r.u16()?;
        let name = p.utf8(ni)?;
        let descriptor = p.utf8(di)?;
        let mut fi = FieldInfo { access, name, descriptor, ..Default::default() };
        p.attributes(|p, name, data| {
            match name {
                "Signature" => fi.signature = Some(p.utf8(u16::from_be_bytes([data[0], data[1]]))?),
                "ConstantValue" => fi.constant = p.const_value(u16::from_be_bytes([data[0], data[1]])),
                "RuntimeVisibleAnnotations" => fi.annotations.extend(p.annotations(data, true)?),
                "RuntimeInvisibleAnnotations" => fi.annotations.extend(p.annotations(data, false)?),
                "Deprecated" => fi.deprecated = true,
                _ => {}
            }
            Ok(())
        })?;
        cf.fields.push(fi);
    }

    let nm = p.r.u16()?;
    for _ in 0..nm {
        let access = p.r.u16()?;
        let ni = p.r.u16()?;
        let di = p.r.u16()?;
        let name = p.utf8(ni)?;
        let descriptor = p.utf8(di)?;
        let mut mi = MethodInfo { access, name, descriptor, ..Default::default() };
        p.attributes(|p, name, data| {
            match name {
                "Signature" => mi.signature = Some(p.utf8(u16::from_be_bytes([data[0], data[1]]))?),
                "Exceptions" => {
                    let mut r = Reader { b: data, p: 0 };
                    let n = r.u16()?;
                    for _ in 0..n {
                        mi.exceptions.push(p.class_name(r.u16()?)?);
                    }
                }
                "RuntimeVisibleAnnotations" => mi.annotations.extend(p.annotations(data, true)?),
                "RuntimeInvisibleAnnotations" => mi.annotations.extend(p.annotations(data, false)?),
                "RuntimeVisibleParameterAnnotations" | "RuntimeInvisibleParameterAnnotations" => {
                    let visible = name == "RuntimeVisibleParameterAnnotations";
                    let pa = p.param_annotations(data, visible)?;
                    if mi.param_annotations.len() < pa.len() {
                        mi.param_annotations.resize(pa.len(), Vec::new());
                    }
                    for (i, a) in pa.into_iter().enumerate() {
                        mi.param_annotations[i].extend(a);
                    }
                }
                "MethodParameters" => {
                    let mut r = Reader { b: data, p: 0 };
                    let n = r.u8()?;
                    for _ in 0..n {
                        let ni = r.u16()?;
                        let _flags = r.u16()?;
                        mi.param_names.push(if ni == 0 { String::new() } else { p.utf8(ni)? });
                    }
                }
                "AnnotationDefault" => {
                    let mut r = Reader { b: data, p: 0 };
                    mi.annotation_default = Some(p.element_value(&mut r)?);
                }
                "Deprecated" => mi.deprecated = true,
                _ => {}
            }
            Ok(())
        })?;
        cf.methods.push(mi);
    }

    p.attributes(|p, name, data| {
        match name {
            "Signature" => cf.signature = Some(p.utf8(u16::from_be_bytes([data[0], data[1]]))?),
            "RuntimeVisibleAnnotations" => cf.annotations.extend(p.annotations(data, true)?),
            "RuntimeInvisibleAnnotations" => cf.annotations.extend(p.annotations(data, false)?),
            "Deprecated" => cf.deprecated = true,
            "SourceFile" => cf.source_file = Some(p.utf8(u16::from_be_bytes([data[0], data[1]]))?),
            "InnerClasses" => {
                let mut r = Reader { b: data, p: 0 };
                let n = r.u16()?;
                for _ in 0..n {
                    let inner = r.u16()?;
                    let outer = r.u16()?;
                    let name = r.u16()?;
                    let access = r.u16()?;
                    let Ok(inner) = p.class_name(inner) else { continue };
                    cf.inner_classes.push(InnerClass {
                        inner,
                        outer: if outer == 0 { None } else { p.class_name(outer).ok() },
                        simple_name: if name == 0 { None } else { p.utf8(name).ok() },
                        access,
                    });
                }
            }
            "PermittedSubclasses" => {
                let mut r = Reader { b: data, p: 0 };
                let n = r.u16()?;
                for _ in 0..n {
                    cf.permitted_subclasses.push(p.class_name(r.u16()?)?);
                }
            }
            "Record" => {
                let mut r = Reader { b: data, p: 0 };
                let n = r.u16()?;
                for _ in 0..n {
                    let name = p.utf8(r.u16()?)?;
                    let desc = p.utf8(r.u16()?)?;
                    let mut sig = None;
                    let na = r.u16()?;
                    for _ in 0..na {
                        let an = r.u16()?;
                        let len = r.u32()? as usize;
                        let d = r.bytes(len)?;
                        if let Ok(nm) = p.utf8(an) {
                            if nm == "Signature" && d.len() >= 2 {
                                sig = p.utf8(u16::from_be_bytes([d[0], d[1]])).ok();
                            }
                        }
                    }
                    cf.record_components.push((name, desc, sig));
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(cf)
}
