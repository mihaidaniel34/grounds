//! What method bodies refer to: every method call, field access and
//! method handle of an `invokedynamic` in a class file's `Code` attributes,
//! each with its source line. A class file records what javac decided (the
//! method an overload resolution chose, the field a name meant), so this is
//! an oracle for tools that resolve the same things from source.
//!
//! Separate from [`crate::parse_class`], which skips method bodies: loading
//! a classpath reads no code.
//!
//! The input is untrusted: every length and index is checked, and anything
//! malformed is an error, never a panic. The decoder steps through each
//! instruction by its length, so it ends after at most one step per byte.

use crate::classfile::{index_attribute, Cp, Parser, Reader};
use anyhow::{bail, Result};

/// How a body refers to a member.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefKind {
    InvokeVirtual,
    /// Also constructors (`<init>`) and `super.m()` calls.
    InvokeSpecial,
    InvokeStatic,
    InvokeInterface,
    GetField,
    PutField,
    GetStatic,
    PutStatic,
    /// A method handle among the bootstrap arguments of an `invokedynamic`,
    /// with its reference kind (JVMS 4.4.8: 1 getField .. 5 invokeVirtual,
    /// 6 invokeStatic, 7 invokeSpecial, 8 newInvokeSpecial, 9
    /// invokeInterface). For a lambda or method reference
    /// (`LambdaMetafactory`) it is the implementation method: the referenced
    /// method of `String::length`, or the synthetic `lambda$...` method that
    /// holds a lambda's body.
    Handle(u8),
}

impl RefKind {
    pub fn is_field(self) -> bool {
        matches!(self, RefKind::GetField | RefKind::PutField | RefKind::GetStatic | RefKind::PutStatic | RefKind::Handle(1..=4))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub kind: RefKind,
    /// Internal name of the class named in the instruction (`java/util/List`,
    /// or an array descriptor such as `[Ljava/lang/Object;` for `clone`)
    pub owner: String,
    pub name: String,
    pub descriptor: String,
    /// Offset of the instruction in the method's code.
    pub pc: u32,
    /// Source line from `LineNumberTable`: the entry with the greatest start
    /// not above `pc`. None without such an entry.
    pub line: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct MethodReferences {
    pub access: u16,
    pub name: String,
    pub descriptor: String,
    /// In the order of the code.
    pub refs: Vec<Reference>,
}

#[derive(Clone, Debug, Default)]
pub struct ClassReferences {
    /// Internal name, e.g. `java/util/Map$Entry`
    pub name: String,
    /// The `SourceFile` attribute: the file name without its directory.
    pub source_file: Option<String>,
    pub methods: Vec<MethodReferences>,
}

/// An `invokedynamic` whose bootstrap arguments are read once the class's
/// `BootstrapMethods` attribute, which follows the methods, has been.
struct Pending {
    method: usize,
    pc: u32,
    line: Option<u32>,
    bootstrap: u16,
}

/// The references in every method body of one class file.
pub fn parse_references(bytes: &[u8]) -> Result<ClassReferences> {
    let mut p = Parser { r: Reader { b: bytes, p: 0 }, cp: Vec::new() };
    if p.r.u32()? != 0xCAFEBABE {
        bail!("not a class file");
    }
    p.r.u16()?; // minor
    p.r.u16()?; // major
    p.parse_constant_pool()?;
    p.r.u16()?; // access
    let this = p.r.u16()?;
    let mut out = ClassReferences { name: p.class_name(this)?, ..Default::default() };
    p.r.u16()?; // super
    let ni = p.r.u16()? as usize;
    p.r.bytes(ni * 2)?;
    let nf = p.r.u16()?;
    for _ in 0..nf {
        p.r.bytes(6)?; // access, name, descriptor
        skip_attributes(&mut p.r)?;
    }
    let mut pending: Vec<Pending> = Vec::new();
    let nm = p.r.u16()?;
    for _ in 0..nm {
        let access = p.r.u16()?;
        let (name, descriptor) = (p.r.u16()?, p.r.u16()?);
        let (name, descriptor) = (p.utf8(name)?, p.utf8(descriptor)?);
        let mut m = MethodReferences { access, name, descriptor, refs: Vec::new() };
        let na = p.r.u16()?;
        for _ in 0..na {
            let (attr, data) = attribute(&mut p)?;
            if attr == "Code" {
                code(&p, data, &mut m.refs, |pc, line, bootstrap| pending.push(Pending { method: out.methods.len(), pc, line, bootstrap }))?;
            }
        }
        out.methods.push(m);
    }
    // bootstrap method -> its arguments' constant-pool indices
    let mut bootstrap: Option<Vec<Vec<u16>>> = None;
    let na = p.r.u16()?;
    for _ in 0..na {
        let (attr, data) = attribute(&mut p)?;
        match attr.as_str() {
            "SourceFile" => out.source_file = Some(p.utf8(index_attribute(data)?)?),
            "BootstrapMethods" => {
                let mut r = Reader { b: data, p: 0 };
                let n = r.u16()?;
                let mut all = Vec::new();
                for _ in 0..n {
                    r.u16()?; // the bootstrap method's own handle
                    let k = r.u16()?;
                    let mut args = Vec::new();
                    for _ in 0..k {
                        args.push(r.u16()?);
                    }
                    all.push(args);
                }
                bootstrap = Some(all);
            }
            _ => {}
        }
    }
    for d in pending {
        let Some(args) = bootstrap.as_ref().and_then(|b| b.get(d.bootstrap as usize)) else {
            bail!("invokedynamic names bootstrap method {} the class does not have", d.bootstrap);
        };
        for &a in args {
            match p.cp.get(a as usize) {
                Some(Cp::MethodHandle(kind, target)) => {
                    let (owner, name, descriptor) = handle_target(&p, *kind, *target)?;
                    out.methods[d.method].refs.push(Reference { kind: RefKind::Handle(*kind), owner, name, descriptor, pc: d.pc, line: d.line });
                }
                Some(_) => {}
                None => bail!("bootstrap argument {a} outside the constant pool"),
            }
        }
    }
    // the handles were added after the instructions around them
    for m in &mut out.methods {
        m.refs.sort_by_key(|r| r.pc);
    }
    Ok(out)
}

fn skip_attributes(r: &mut Reader) -> Result<()> {
    let n = r.u16()?;
    for _ in 0..n {
        r.u16()?;
        let len = r.u32()? as usize;
        r.bytes(len)?;
    }
    Ok(())
}

/// One attribute: its name and its data.
fn attribute<'a>(p: &mut Parser<'a>) -> Result<(String, &'a [u8])> {
    let name = p.r.u16()?;
    let len = p.r.u32()? as usize;
    let data = p.r.bytes(len)?;
    Ok((p.utf8(name)?, data))
}

fn name_and_type(p: &Parser, idx: u16) -> Result<(String, String)> {
    match p.cp.get(idx as usize) {
        Some(Cp::NameAndType(n, d)) => Ok((p.utf8(*n)?, p.utf8(*d)?)),
        _ => bail!("expected name and type at {idx}"),
    }
}

/// Owner, name and descriptor of a Fieldref (`tags` 9), Methodref (10) or
/// InterfaceMethodref (11) among `tags`.
fn member(p: &Parser, idx: u16, tags: &[u8]) -> Result<(String, String, String)> {
    match p.cp.get(idx as usize) {
        Some(Cp::Member(tag, class, nat)) if tags.contains(tag) => {
            let (name, descriptor) = name_and_type(p, *nat)?;
            Ok((p.class_name(*class)?, name, descriptor))
        }
        _ => bail!("expected a member reference ({tags:?}) at {idx}"),
    }
}

/// What a method handle refers to (JVMS 4.4.8): a field for kinds 1-4, a
/// method for 5-9.
fn handle_target(p: &Parser, kind: u8, target: u16) -> Result<(String, String, String)> {
    match kind {
        1..=4 => member(p, target, &[9]),
        5 | 8 => member(p, target, &[10]),
        6 | 7 => member(p, target, &[10, 11]),
        9 => member(p, target, &[11]),
        k => bail!("bad method handle kind {k}"),
    }
}

/// The references in one `Code` attribute. `indy` receives each
/// `invokedynamic`: pc, line and bootstrap method index.
fn code(p: &Parser, data: &[u8], refs: &mut Vec<Reference>, mut indy: impl FnMut(u32, Option<u32>, u16)) -> Result<()> {
    let mut r = Reader { b: data, p: 0 };
    r.u16()?; // max_stack
    r.u16()?; // max_locals
    let len = r.u32()? as usize;
    let code = r.bytes(len)?;
    let handlers = r.u16()? as usize;
    r.bytes(handlers * 8)?;
    let mut lines: Vec<(u32, u32)> = Vec::new();
    let na = r.u16()?;
    for _ in 0..na {
        let name = r.u16()?;
        let alen = r.u32()? as usize;
        let adata = r.bytes(alen)?;
        if p.utf8(name)? == "LineNumberTable" {
            let mut t = Reader { b: adata, p: 0 };
            let n = t.u16()?;
            for _ in 0..n {
                let start = t.u16()? as u32;
                let line = t.u16()? as u32;
                if start as usize >= code.len() {
                    bail!("line table entry at {start} outside code of {} bytes", code.len());
                }
                lines.push((start, line));
            }
        }
    }
    // several entries at one pc: the last in this order, as javap lists it
    lines.sort_unstable();
    let line_at = |pc: u32| -> Option<u32> {
        let i = lines.partition_point(|&(start, _)| start <= pc);
        i.checked_sub(1).map(|i| lines[i].1)
    };
    let u16_at = |i: usize| u16::from_be_bytes([code[i], code[i + 1]]);
    let mut pc = 0usize;
    while pc < code.len() {
        let op = code[pc];
        let n = instruction_length(code, pc)?;
        if n > code.len() - pc {
            bail!("instruction {op:#04x} at {pc} runs past the end of the code");
        }
        let kind = match op {
            0xb2 => Some(RefKind::GetStatic),
            0xb3 => Some(RefKind::PutStatic),
            0xb4 => Some(RefKind::GetField),
            0xb5 => Some(RefKind::PutField),
            0xb6 => Some(RefKind::InvokeVirtual),
            0xb7 => Some(RefKind::InvokeSpecial),
            0xb8 => Some(RefKind::InvokeStatic),
            0xb9 => Some(RefKind::InvokeInterface),
            _ => None,
        };
        let line = line_at(pc as u32);
        if let Some(kind) = kind {
            // invokespecial and invokestatic may name an interface's method (class files since Java 8)
            let tags: &[u8] = match kind {
                RefKind::InvokeVirtual => &[10],
                RefKind::InvokeSpecial | RefKind::InvokeStatic => &[10, 11],
                RefKind::InvokeInterface => &[11],
                _ => &[9],
            };
            let (owner, name, descriptor) = member(p, u16_at(pc + 1), tags)?;
            refs.push(Reference { kind, owner, name, descriptor, pc: pc as u32, line });
        } else if op == 0xba {
            let idx = u16_at(pc + 1);
            match p.cp.get(idx as usize) {
                Some(Cp::Dynamic(18, bootstrap, nat)) => {
                    name_and_type(p, *nat)?;
                    indy(pc as u32, line, *bootstrap)
                }
                _ => bail!("invokedynamic at {pc} names no InvokeDynamic constant"),
            }
        }
        pc += n;
    }
    Ok(())
}

fn i32_at(code: &[u8], i: usize) -> Result<i64> {
    match code.get(i..i + 4) {
        Some(b) => Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64),
        None => bail!("switch table runs past the end of the code"),
    }
}

/// The length of the instruction at `pc`, operands included (JVMS 6.5).
fn instruction_length(code: &[u8], pc: usize) -> Result<usize> {
    let op = code[pc];
    Ok(match op {
        0x00..=0x0f | 0x1a..=0x35 | 0x3b..=0x83 | 0x85..=0x98 | 0xac..=0xb1 | 0xbe | 0xbf | 0xc2 | 0xc3 => 1,
        0x10 | 0x12 | 0x15..=0x19 | 0x36..=0x3a | 0xa9 | 0xbc => 2,
        0x11 | 0x13 | 0x14 | 0x84 | 0x99..=0xa8 | 0xb2..=0xb8 | 0xbb | 0xbd | 0xc0 | 0xc1 | 0xc6 | 0xc7 => 3,
        0xc5 => 4,
        0xb9 | 0xba | 0xc8 | 0xc9 => 5,
        // tableswitch, lookupswitch: padding to a multiple of four bytes from
        // the start of the code, then the default offset and the table
        0xaa | 0xab => {
            let base = pc + 1 + (4 - (pc + 1) % 4) % 4;
            let table = if op == 0xaa {
                let (low, high) = (i32_at(code, base + 4)?, i32_at(code, base + 8)?);
                if high < low {
                    bail!("tableswitch at {pc} with high {high} below low {low}");
                }
                12 + (high - low + 1) * 4
            } else {
                let pairs = i32_at(code, base + 4)?;
                if pairs < 0 {
                    bail!("lookupswitch at {pc} with {pairs} pairs");
                }
                8 + pairs * 8
            };
            // at most 2^32 * 4 bytes: within usize on a 64-bit target, and checked against the code below
            (base - pc) + usize::try_from(table)?
        }
        // wide: a local variable instruction with a two-byte index; iinc also has a two-byte constant
        0xc4 => match code.get(pc + 1) {
            Some(0x84) => 6,
            Some(0x15..=0x19 | 0x36..=0x3a | 0xa9) => 4,
            Some(o) => bail!("wide at {pc} modifies {o:#04x}"),
            None => bail!("wide at the end of the code"),
        },
        _ => bail!("unknown opcode {op:#04x} at {pc}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    /// Lines are part of what the test checks: keep each where it is.
    const SOURCE: &str = r#"package t;
import java.util.*;
import java.util.concurrent.TimeUnit;
import java.util.function.*;
public class Refs {
    static int counter;
    int size;
    List<String> names = new ArrayList<>(init());
    static List<String> init() { return List.of("a"); }
    String chain(StringBuilder sb) {
        return sb.append("x")
            .reverse().toString();
    }
    Supplier<Integer> lambda(String s) {
        return () ->
            s.length();
    }
    Function<String, Integer> ref() {
        return String::length;
    }
    Object make() {
        return new ArrayList<String>(
            names);
    }
    void fields() {
        counter = size;
        this.size = Refs.counter + 1;
    }
    int strings(String s) {
        switch (s) {
            case "a": return 1;
            case "bb": return 2;
        }
        return s.hashCode();
    }
    int units(TimeUnit u, int i) {
        switch (u) {
            case SECONDS: return 1;
            case MINUTES: return 2;
        }
        switch (i) {
            case 0: return 3;
            case 1: return 4;
            case 2: return 5;
        }
        return u.ordinal();
    }
    class Inner { Inner(int x) { this(x, x); } Inner(int x, int y) {} }
    Inner inner() { return new Inner(3); }
}
"#;

    /// javac from the JDK grounds finds, else from PATH.
    fn javac() -> Option<PathBuf> {
        let home = crate::find_java_home().map(|h| h.join("bin/javac"));
        home.into_iter().chain([PathBuf::from("javac")]).find(|c| Command::new(c).arg("-version").env_remove("JAVA_TOOL_OPTIONS").output().is_ok_and(|o| o.status.success()))
    }

    /// `kind owner.name:descriptor@line` of every reference of a method.
    fn refs(c: &ClassReferences, method: &str, descriptor: &str) -> Vec<String> {
        let m = c.methods.iter().find(|m| m.name == method && m.descriptor == descriptor).unwrap_or_else(|| panic!("{method}{descriptor} in {}", c.name));
        m.refs.iter().map(|r| format!("{:?} {}.{}:{}@{}", r.kind, r.owner, r.name, r.descriptor, r.line.map_or("-".to_string(), |l| l.to_string()))).collect()
    }

    #[test]
    fn references_and_lines_of_compiled_java() {
        let Some(javac) = javac() else {
            eprintln!("skipped: no javac (set JAVA_HOME or put javac on PATH)");
            return;
        };
        let dir = std::env::temp_dir().join(format!("grounds-references-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("t")).unwrap();
        std::fs::write(dir.join("t/Refs.java"), SOURCE).unwrap();
        let out = Command::new(&javac).arg("-d").arg(dir.join("out")).arg(dir.join("t/Refs.java")).env_remove("JAVA_TOOL_OPTIONS").output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let read = |n: &str| parse_references(&std::fs::read(dir.join("out/t").join(n)).unwrap()).unwrap();
        let c = read("Refs.class");
        assert_eq!((c.name.as_str(), c.source_file.as_deref()), ("t/Refs", Some("Refs.java")));

        // a field initializer runs in the constructor, on its own line
        let init = refs(&c, "<init>", "()V");
        assert_eq!(&init[1..], ["InvokeStatic t/Refs.init:()Ljava/util/List;@8", "InvokeSpecial java/util/ArrayList.<init>:(Ljava/util/Collection;)V@8", "PutField t/Refs.names:Ljava/util/List;@8"]);
        // a call on a later line of a chain has that line: the one of its `(`
        assert_eq!(refs(&c, "chain", "(Ljava/lang/StringBuilder;)Ljava/lang/String;"), [
            "InvokeVirtual java/lang/StringBuilder.append:(Ljava/lang/String;)Ljava/lang/StringBuilder;@11",
            "InvokeVirtual java/lang/StringBuilder.reverse:()Ljava/lang/StringBuilder;@12",
            "InvokeVirtual java/lang/StringBuilder.toString:()Ljava/lang/String;@12",
        ]);
        // a lambda: its body is a synthetic method, the handle names it
        assert_eq!(refs(&c, "lambda", "(Ljava/lang/String;)Ljava/util/function/Supplier;"), ["Handle(6) t/Refs.lambda$lambda$0:(Ljava/lang/String;)Ljava/lang/Integer;@15"]);
        assert_eq!(refs(&c, "lambda$lambda$0", "(Ljava/lang/String;)Ljava/lang/Integer;")[0], "InvokeVirtual java/lang/String.length:()I@16");
        // a method reference: the handle names the method itself
        assert_eq!(refs(&c, "ref", "()Ljava/util/function/Function;"), ["Handle(5) java/lang/String.length:()I@19"]);
        // a constructor call and a field read without a line entry of their own: the statement's line
        assert_eq!(refs(&c, "make", "()Ljava/lang/Object;"), ["GetField t/Refs.names:Ljava/util/List;@22", "InvokeSpecial java/util/ArrayList.<init>:(Ljava/util/Collection;)V@22"]);
        assert_eq!(refs(&c, "fields", "()V"), ["GetField t/Refs.size:I@26", "PutStatic t/Refs.counter:I@26", "GetStatic t/Refs.counter:I@27", "PutField t/Refs.size:I@27"]);
        // switches on strings (two lookupswitches), an enum of another class
        // (lookupswitch over a switch map) and dense ints (tableswitch): the
        // calls after the tables are found, so the tables were stepped over
        let strings = refs(&c, "strings", "(Ljava/lang/String;)I");
        assert_eq!(strings.len(), 4, "{strings:?}");
        assert_eq!(strings[3], "InvokeVirtual java/lang/String.hashCode:()I@34");
        assert_eq!(refs(&c, "units", "(Ljava/util/concurrent/TimeUnit;I)I"), [
            "GetStatic t/Refs$1.$SwitchMap$java$util$concurrent$TimeUnit:[I@37",
            "InvokeVirtual java/util/concurrent/TimeUnit.ordinal:()I@37",
            "InvokeVirtual java/util/concurrent/TimeUnit.ordinal:()I@46",
        ]);
        // an inner class's constructor takes the outer instance first
        assert_eq!(refs(&c, "inner", "()Lt/Refs$Inner;"), ["InvokeSpecial t/Refs$Inner.<init>:(Lt/Refs;I)V@49"]);
        let inner = read("Refs$Inner.class");
        assert_eq!(refs(&inner, "<init>", "(Lt/Refs;I)V"), ["InvokeSpecial t/Refs$Inner.<init>:(Lt/Refs;II)V@48"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A class `T` with one method `m()V` whose code is `code`, with a line
    /// table and, if `bootstrap`, a BootstrapMethods attribute. Constant pool:
    /// 1 "T", 2 class T, 3 "m", 4 "()V", 5 "Code", 6 "LineNumberTable", 7 m:()V,
    /// 8 Methodref T.m, 9 Fieldref T.m, 10 "BootstrapMethods", 11 handle
    /// invokeStatic T.m, 12 InvokeDynamic #0 m:()V, 13 "SourceFile".
    fn class(code: &[u8], lines: &[(u16, u16)], bootstrap: bool) -> Vec<u8> {
        let utf8 = |s: &str| [&[1u8][..], &(s.len() as u16).to_be_bytes(), s.as_bytes()].concat();
        let mut c = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 65, 0, 14];
        for e in [utf8("T"), vec![7, 0, 1], utf8("m"), utf8("()V"), utf8("Code"), utf8("LineNumberTable"), vec![12, 0, 3, 0, 4], vec![10, 0, 2, 0, 7], vec![9, 0, 2, 0, 7], utf8("BootstrapMethods"), vec![15, 6, 0, 8], vec![18, 0, 0, 0, 7], utf8("SourceFile")] {
            c.extend(e);
        }
        c.extend([0, 0x21, 0, 2, 0, 0, 0, 0, 0, 0]); // access, this, super, no interfaces, no fields
        let mut table = (lines.len() as u16).to_be_bytes().to_vec();
        for (pc, line) in lines {
            table.extend(pc.to_be_bytes());
            table.extend(line.to_be_bytes());
        }
        let mut body = vec![0, 1, 0, 1];
        body.extend((code.len() as u32).to_be_bytes());
        body.extend(code);
        body.extend([0, 0, 0, 1, 0, 6]);
        body.extend((table.len() as u32).to_be_bytes());
        body.extend(table);
        c.extend([0, 1, 0, 9, 0, 3, 0, 4, 0, 1, 0, 5]); // one method m()V with one attribute
        c.extend((body.len() as u32).to_be_bytes());
        c.extend(body);
        if bootstrap {
            c.extend([0, 1, 0, 10, 0, 0, 0, 8, 0, 1, 0, 11, 0, 1, 0, 11]);
        } else {
            c.extend([0, 0]);
        }
        c
    }

    #[test]
    fn a_hand_made_class_file_is_read() {
        // invokestatic, a wide iinc, getstatic, invokedynamic, return
        let c = parse_references(&class(&[0xb8, 0, 8, 0xc4, 0x84, 0, 1, 0, 1, 0xb2, 0, 9, 0xba, 0, 12, 0, 0, 0xb1], &[(0, 7), (3, 8)], true)).unwrap();
        assert_eq!(refs(&c, "m", "()V"), ["InvokeStatic T.m:()V@7", "GetStatic T.m:()V@8", "Handle(6) T.m:()V@8"]);
        // no line table entry at or before an instruction: no line
        let c = parse_references(&class(&[0xb8, 0, 8, 0xb1], &[(3, 9)], false)).unwrap();
        assert_eq!(refs(&c, "m", "()V"), ["InvokeStatic T.m:()V@-"]);
    }

    #[test]
    fn malformed_class_files_are_errors_not_panics() {
        let err = |code: &[u8], lines: &[(u16, u16)], bootstrap: bool| parse_references(&class(code, lines, bootstrap)).is_err();
        // truncated code: an operand missing
        assert!(err(&[0xb8, 0], &[], false));
        assert!(err(&[0xb9, 0, 8, 1], &[], false));
        // tableswitch at pc 1: three bytes of padding, then 12 bytes the code does not have
        assert!(err(&[0, 0xaa, 0, 0], &[], false));
        assert!(err(&[0xaa], &[], false));
        // lookupswitch whose pair count runs past the end, or is negative
        let mut ls = vec![0xab, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        assert!(err(&ls, &[], false));
        ls[8..12].copy_from_slice(&(-1i32).to_be_bytes());
        assert!(err(&ls, &[], false));
        // tableswitch from i32::MIN to i32::MAX: 2^34 bytes of table; then high below low
        let mut ts = vec![0xaa, 0, 0, 0, 0, 0, 0, 0];
        ts.extend(i32::MIN.to_be_bytes());
        ts.extend(i32::MAX.to_be_bytes());
        assert!(err(&ts, &[], false));
        ts[8..16].copy_from_slice(&[0, 0, 0, 5, 0, 0, 0, 1]);
        assert!(err(&ts, &[], false));
        // a well-formed table of one entry (low = high = 0) is read
        assert!(!err(&[0xaa, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xb1], &[], false));
        // wide of something without a local index, or at the end, or cut short
        assert!(err(&[0xc4, 0x00, 0, 0], &[], false));
        assert!(err(&[0xc4], &[], false));
        assert!(err(&[0xc4, 0x84, 0, 1, 0], &[], false));
        // an opcode the JVM does not define
        assert!(err(&[0xff], &[], false));
        // a line table entry outside the code
        assert!(err(&[0xb1], &[(1, 3)], false));
        // constant-pool indices of the wrong kind or outside the pool
        assert!(err(&[0xb8, 0, 1], &[], false)); // a Utf8
        assert!(err(&[0xb4, 0, 8], &[], false)); // getfield of a Methodref
        assert!(err(&[0xb6, 0, 9], &[], false)); // invokevirtual of a Fieldref
        assert!(err(&[0xb6, 0, 200], &[], false));
        assert!(err(&[0xba, 0, 8, 0, 0], &[], false)); // invokedynamic of a Methodref
        // invokedynamic without the BootstrapMethods attribute
        assert!(err(&[0xba, 0, 12, 0, 0], &[], false));

        // every prefix of a well-formed file is refused, and every byte
        // changed is read or refused, without a panic
        let good = class(&[0xb8, 0, 8, 0xaa, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xba, 0, 12, 0, 0, 0xb1], &[(0, 1), (3, 2)], true);
        assert!(parse_references(&good).is_ok());
        for n in 0..good.len() {
            assert!(parse_references(&good[..n]).is_err(), "prefix of {n} bytes");
            for b in [0x00, 0x01, 0x7f, 0x80, 0xff] {
                let mut bad = good.clone();
                bad[n] = b;
                let _ = parse_references(&bad);
            }
        }
    }
}
