//! Read-only reader for the JDK runtime image `lib/modules` (jimage format).
//! ct.sym carries no signatures for the JDK's own release, so that release's
//! classes are read from the image. Only classes in packages with an
//! unqualified export in their module (per `module-info.class`) are listed,
//! which is the set ct.sym carries. A compressed class resource is not
//! decoded: the whole image is rejected, so callers fall back to ct.sym
//! rather than miss classes.

use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use rustc_hash::{FxHashMap, FxHashSet};
use std::fs::File;
use std::path::{Path, PathBuf};

const MAGIC: u32 = 0xCAFE_DADA;
const HEADER_LEN: usize = 28;

/// internal class name -> (resource offset, size)
type Classes = FxHashMap<String, (usize, usize)>;

// location attribute kinds (`ImageLocation.ATTRIBUTE_*`)
const ATTR_END: usize = 0;
const ATTR_MODULE: usize = 1;
const ATTR_PARENT: usize = 2;
const ATTR_BASE: usize = 3;
const ATTR_EXTENSION: usize = 4;
const ATTR_OFFSET: usize = 5;
const ATTR_COMPRESSED: usize = 6;
const ATTR_UNCOMPRESSED: usize = 7;

pub struct JImage {
    pub path: PathBuf,
    mmap: Mmap,
    /// Resource offsets are relative to the end of the index.
    data_start: usize,
    classes: Classes,
}

impl JImage {
    pub fn open(path: &Path) -> Result<JImage> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        // SAFETY: as for jars, the image is treated as immutable while indexed.
        let mmap = unsafe { Mmap::map(&file)? };
        let (data_start, classes) = index(&mmap).with_context(|| format!("read {}", path.display()))?;
        Ok(JImage { path: path.to_path_buf(), mmap, data_start, classes })
    }

    pub fn class_names(&self) -> impl Iterator<Item = &str> {
        self.classes.keys().map(String::as_str)
    }

    pub fn read_class_bytes(&self, internal_name: &str) -> Option<Vec<u8>> {
        let &(offset, size) = self.classes.get(internal_name)?;
        let start = self.data_start.checked_add(offset)?;
        self.mmap.get(start..start.checked_add(size)?).map(<[u8]>::to_vec)
    }
}

/// Header, then the redirect and offset tables, the location attributes and
/// the string table; resource data follows. Returns the start of the data and
/// the exported classes.
fn index(b: &[u8]) -> Result<(usize, Classes)> {
    // written in native byte order: detect it from the magic
    let be = match (u32_at(b, 0, false), u32_at(b, 0, true)) {
        (Some(MAGIC), _) => false,
        (_, Some(MAGIC)) => true,
        _ => bail!("not a jimage"),
    };
    let field = |o: usize| u32_at(b, o, be).map(|v| v as usize).context("truncated jimage header");
    let table_len = field(16)?;
    let locations_at = HEADER_LEN + 8 * table_len;
    let strings_at = locations_at + field(20)?;
    let data_start = strings_at + field(24)?;
    if data_start > b.len() {
        bail!("truncated jimage");
    }
    let locations = &b[locations_at..strings_at];
    let strings = &b[strings_at..data_start];
    let string = |o: u64| -> Option<&str> {
        let s = strings.get(usize::try_from(o).ok()?..)?;
        std::str::from_utf8(&s[..s.iter().position(|&c| c == 0)?]).ok()
    };

    // module -> its module-info location; every `.class` location as
    // (module, package, internal name, offset, size)
    let mut module_infos: FxHashMap<&str, (usize, usize)> = FxHashMap::default();
    let mut found: Vec<(&str, &str, String, usize, usize)> = Vec::new();
    for i in 0..table_len {
        let loc = u32_at(b, HEADER_LEN + 4 * table_len + 4 * i, be).context("truncated jimage")? as usize;
        let attrs = location_attrs(locations, loc).context("corrupt jimage location")?;
        let attr = |kind: usize| string(attrs[kind]).context("corrupt jimage string");
        let module = attr(ATTR_MODULE)?;
        if attr(ATTR_EXTENSION)? != "class" || module.is_empty() {
            continue;
        }
        let (parent, base) = (attr(ATTR_PARENT)?, attr(ATTR_BASE)?);
        let (offset, size) = (attrs[ATTR_OFFSET] as usize, attrs[ATTR_UNCOMPRESSED] as usize);
        if base == "module-info" && parent.is_empty() {
            module_infos.insert(module, (offset, size));
            continue;
        }
        if attrs[ATTR_COMPRESSED] != 0 {
            bail!("compressed resource {module}/{parent}/{base}.class is not supported");
        }
        let internal = if parent.is_empty() { base.to_string() } else { format!("{parent}/{base}") };
        found.push((module, parent, internal, offset, size));
    }

    let mut exported: FxHashMap<&str, FxHashSet<String>> = FxHashMap::default();
    for (module, (offset, size)) in module_infos {
        let start = data_start + offset;
        let data = b.get(start..start.checked_add(size).context("corrupt module-info")?).context("corrupt module-info")?;
        let packages = unqualified_exports(data).context("corrupt module-info")?;
        exported.insert(module, packages.into_iter().collect());
    }
    let classes = found
        .into_iter()
        .filter(|(module, package, ..)| exported.get(module).is_some_and(|p| p.contains(*package)))
        .map(|(_, _, internal, offset, size)| (internal, (offset, size)))
        .collect();
    Ok((data_start, classes))
}

fn u32_at(b: &[u8], o: usize, be: bool) -> Option<u32> {
    let a: [u8; 4] = b.get(o..o + 4)?.try_into().ok()?;
    Some(if be { u32::from_be_bytes(a) } else { u32::from_le_bytes(a) })
}

/// The attributes of the location at `o`: (kind, value) pairs ended by kind 0.
/// A byte holds the kind in its high 5 bits and the value's length minus one
/// in its low 3; values are big-endian.
fn location_attrs(locs: &[u8], mut o: usize) -> Option<[u64; 8]> {
    let mut attrs = [0u64; 8];
    loop {
        let byte = *locs.get(o)?;
        let kind = (byte >> 3) as usize;
        if kind == ATTR_END {
            return Some(attrs);
        }
        let n = (byte & 7) as usize + 1;
        let value = locs.get(o + 1..o + 1 + n)?.iter().fold(0u64, |v, &x| v << 8 | x as u64);
        if let Some(a) = attrs.get_mut(kind) {
            *a = value;
        }
        o += 1 + n;
    }
}

/// Big-endian reader over class file bytes.
struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        let s = self.bytes(2)?;
        Some(u16::from_be_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        let s = self.bytes(4)?;
        Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
}

#[derive(Clone, Copy)]
enum Constant<'a> {
    Utf8(&'a str),
    /// CONSTANT_Module or CONSTANT_Package: the index of its name.
    Named(u16),
    Other,
}

/// Packages with an unqualified export in a `module-info.class`, from its
/// `Module` attribute (the constant pool names the packages).
fn unqualified_exports(data: &[u8]) -> Option<Vec<String>> {
    let mut r = Reader { b: data, p: 8 }; // magic, minor, major
    let count = r.u16()? as usize;
    let mut pool = vec![Constant::Other; count];
    let mut i = 1;
    while i < count {
        pool[i] = match r.u8()? {
            1 => {
                let n = r.u16()? as usize;
                Constant::Utf8(std::str::from_utf8(r.bytes(n)?).ok()?)
            }
            19 | 20 => Constant::Named(r.u16()?),
            7 | 8 | 16 => {
                r.bytes(2)?;
                Constant::Other
            }
            15 => {
                r.bytes(3)?;
                Constant::Other
            }
            3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => {
                r.bytes(4)?;
                Constant::Other
            }
            5 | 6 => {
                // long and double take two pool slots
                r.bytes(8)?;
                i += 1;
                Constant::Other
            }
            _ => return None,
        };
        i += 1;
    }
    let name = |idx: u16| match pool.get(idx as usize)? {
        Constant::Named(n) => match pool.get(*n as usize)? {
            Constant::Utf8(s) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    };

    r.bytes(6)?; // access flags, this class, super class
    let interfaces = r.u16()? as usize;
    r.bytes(2 * interfaces)?;
    for _ in 0..2 {
        // fields, then methods
        for _ in 0..r.u16()? {
            r.bytes(6)?;
            skip_attributes(&mut r)?;
        }
    }
    for _ in 0..r.u16()? {
        let attr_name = r.u16()?;
        let len = r.u32()? as usize;
        let body = r.bytes(len)?;
        if let Some(Constant::Utf8("Module")) = pool.get(attr_name as usize) {
            return module_exports(body, &name);
        }
    }
    Some(Vec::new())
}

fn skip_attributes(r: &mut Reader) -> Option<()> {
    for _ in 0..r.u16()? {
        r.bytes(2)?;
        let len = r.u32()? as usize;
        r.bytes(len)?;
    }
    Some(())
}

/// `Module` attribute: header, requires, then exports (package, flags,
/// target modules).
fn module_exports(body: &[u8], name: &impl Fn(u16) -> Option<String>) -> Option<Vec<String>> {
    let mut r = Reader { b: body, p: 0 };
    r.bytes(6)?; // module name, flags, version
    for _ in 0..r.u16()? {
        r.bytes(6)?; // requires: module, flags, version
    }
    let mut out = Vec::new();
    for _ in 0..r.u16()? {
        let package = r.u16()?;
        r.bytes(2)?; // flags
        let targets = r.u16()? as usize;
        r.bytes(2 * targets)?;
        if targets == 0 {
            out.push(name(package)?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_location_attributes() {
        // kind 1 with a 2-byte value 0x0102, kind 5 with a 1-byte value 7, end
        let locs = [1 << 3 | 1, 0x01, 0x02, 5 << 3, 7, 0];
        let attrs = location_attrs(&locs, 0).unwrap();
        assert_eq!((attrs[ATTR_MODULE], attrs[ATTR_OFFSET]), (0x0102, 7));
    }

    #[test]
    fn rejects_non_jimage() {
        assert!(index(&[0u8; 64]).is_err());
    }
}
