//! Minimal read-only zip archive reader over a memory map. Only supports what
//! jars and `ct.sym` use: stored and deflated entries, optional zip64.

use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use rustc_hash::FxHashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ZipEntry {
    pub name: String,
    method: u16,
    compressed_size: u64,
    uncompressed_size: u64,
    local_header_offset: u64,
}

pub struct ZipArchive {
    pub path: PathBuf,
    mmap: Mmap,
    entries: Vec<ZipEntry>,
    by_name: FxHashMap<String, usize>,
}

impl std::fmt::Debug for ZipArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ZipArchive({}, {} entries)", self.path.display(), self.entries.len())
    }
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

impl ZipArchive {
    pub fn open(path: &Path) -> Result<ZipArchive> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        // SAFETY: jars are treated as immutable while indexed; a concurrently
        // modified jar can at worst produce garbage entries which we validate.
        let mmap = unsafe { Mmap::map(&file)? };
        let entries = read_central_directory(&mmap).with_context(|| format!("read zip {}", path.display()))?;
        let by_name = entries.iter().enumerate().map(|(i, e)| (e.name.clone(), i)).collect();
        Ok(ZipArchive { path: path.to_path_buf(), mmap, entries, by_name })
    }

    pub fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        let idx = *self.by_name.get(name)?;
        self.read_entry(&self.entries[idx]).ok()
    }

    pub fn read_entry(&self, e: &ZipEntry) -> Result<Vec<u8>> {
        let b = &self.mmap[..];
        let Some(lo) = usize::try_from(e.local_header_offset).ok().filter(|lo| lo.checked_add(30).is_some_and(|end| end <= b.len())) else {
            bail!("bad local header offset for {}", e.name);
        };
        if u32_at(b, lo) != 0x04034b50 {
            bail!("bad local header for {}", e.name);
        }
        let name_len = u16_at(b, lo + 26) as usize;
        let extra_len = u16_at(b, lo + 28) as usize;
        let start = lo + 30 + name_len + extra_len;
        let end = usize::try_from(e.compressed_size).ok().and_then(|c| start.checked_add(c));
        let Some(end) = end.filter(|&end| end <= b.len()) else {
            bail!("truncated entry {}", e.name);
        };
        let data = &b[start..end];
        // class files are small: cap inflation (and distrust the declared size)
        const MAX_ENTRY: usize = 64 << 20;
        let limit = usize::try_from(e.uncompressed_size).unwrap_or(MAX_ENTRY).clamp(1, MAX_ENTRY).saturating_mul(2).saturating_add(1024);
        match e.method {
            0 => Ok(data.to_vec()),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, limit).map_err(|err| anyhow::anyhow!("inflate {}: {:?}", e.name, err)),
            m => bail!("unsupported compression method {m} for {}", e.name),
        }
    }
}

fn read_central_directory(b: &[u8]) -> Result<Vec<ZipEntry>> {
    if b.len() < 22 {
        bail!("too small");
    }
    // Find end of central directory record.
    let min = b.len().saturating_sub(22 + 65535);
    let mut eocd = None;
    let mut i = b.len() - 22;
    loop {
        if u32_at(b, i) == 0x06054b50 {
            eocd = Some(i);
            break;
        }
        if i == min {
            break;
        }
        i -= 1;
    }
    let eocd = eocd.context("no end of central directory")?;
    let mut count = u16_at(b, eocd + 10) as u64;
    let mut cd_size = u32_at(b, eocd + 12) as u64;
    let mut cd_offset = u32_at(b, eocd + 16) as u64;
    if (count == 0xFFFF || cd_offset == 0xFFFF_FFFF) && eocd >= 20 && u32_at(b, eocd - 20) == 0x07064b50 {
        let z64 = u64_at(b, eocd - 20 + 8) as usize;
        if z64 + 56 <= b.len() && u32_at(b, z64) == 0x06064b50 {
            count = u64_at(b, z64 + 32);
            cd_size = u64_at(b, z64 + 40);
            cd_offset = u64_at(b, z64 + 48);
        }
    }
    let _ = cd_size;
    // every entry takes at least 46 bytes: never trust the declared count for allocation
    let mut entries = Vec::with_capacity((count as usize).min(b.len() / 46));
    let Ok(mut p) = usize::try_from(cd_offset) else { bail!("bad central directory offset") };
    for _ in 0..count {
        if p.checked_add(46).is_none_or(|end| end > b.len()) || u32_at(b, p) != 0x02014b50 {
            bail!("bad central directory entry");
        }
        let method = u16_at(b, p + 10);
        let mut csize = u32_at(b, p + 20) as u64;
        let mut usize_ = u32_at(b, p + 24) as u64;
        let name_len = u16_at(b, p + 28) as usize;
        let extra_len = u16_at(b, p + 30) as usize;
        let comment_len = u16_at(b, p + 32) as usize;
        let mut lho = u32_at(b, p + 42) as u64;
        // the variable-length fields must lie inside the archive too
        if p + 46 + name_len + extra_len + comment_len > b.len() {
            bail!("truncated central directory entry");
        }
        let name_bytes = &b[p + 46..p + 46 + name_len];
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        // zip64 extra
        let mut e = p + 46 + name_len;
        let extra_end = e + extra_len;
        while e + 4 <= extra_end {
            let id = u16_at(b, e);
            let sz = u16_at(b, e + 2) as usize;
            // a field claiming more than the extra area holds ends the parse
            let field_end = (e + 4 + sz).min(extra_end);
            if id == 0x0001 {
                let mut q = e + 4;
                if usize_ == 0xFFFF_FFFF && q + 8 <= field_end {
                    usize_ = u64_at(b, q);
                    q += 8;
                }
                if csize == 0xFFFF_FFFF && q + 8 <= field_end {
                    csize = u64_at(b, q);
                    q += 8;
                }
                if lho == 0xFFFF_FFFF && q + 8 <= field_end {
                    lho = u64_at(b, q);
                }
            }
            e += 4 + sz;
        }
        entries.push(ZipEntry { name, method, compressed_size: csize, uncompressed_size: usize_, local_header_offset: lho });
        p += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::read_central_directory;

    fn eocd(count: u16, cd_size: u32, cd_offset: u32) -> Vec<u8> {
        let mut v = vec![0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0];
        v.extend(count.to_le_bytes());
        v.extend(count.to_le_bytes());
        v.extend(cd_size.to_le_bytes());
        v.extend(cd_offset.to_le_bytes());
        v.extend([0, 0]);
        v
    }

    #[test]
    fn malformed_archives_are_errors_not_panics() {
        // central directory entry claiming a 65535-byte name in a 68-byte file
        let mut header = vec![0u8; 46];
        header[..4].copy_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
        header[28..30].copy_from_slice(&u16::MAX.to_le_bytes());
        let mut jar = header.clone();
        jar.extend(eocd(1, 46, 0));
        assert_eq!(jar.len(), 68);
        assert!(read_central_directory(&jar).is_err());

        // a huge declared entry count must not allocate for it
        let mut jar = Vec::new();
        jar.extend(eocd(u16::MAX, 0, 0));
        assert!(read_central_directory(&jar).is_err());

        // extra field claiming more bytes than the entry has
        let mut header = vec![0u8; 46];
        header[..4].copy_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
        header[30..32].copy_from_slice(&8u16.to_le_bytes()); // extra_len
        header[20..24].copy_from_slice(&u32::MAX.to_le_bytes()); // csize -> zip64
        let mut jar = header;
        jar.extend([0x01, 0x00, 0xff, 0xff, 0, 0, 0, 0]); // zip64 field, size 65535
        let cd_size = jar.len() as u32;
        jar.extend(eocd(1, cd_size, 0));
        let _ = read_central_directory(&jar);
    }
}
