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
        let lo = e.local_header_offset as usize;
        if lo + 30 > b.len() || u32_at(b, lo) != 0x04034b50 {
            bail!("bad local header for {}", e.name);
        }
        let name_len = u16_at(b, lo + 26) as usize;
        let extra_len = u16_at(b, lo + 28) as usize;
        let start = lo + 30 + name_len + extra_len;
        let end = start + e.compressed_size as usize;
        if end > b.len() {
            bail!("truncated entry {}", e.name);
        }
        let data = &b[start..end];
        match e.method {
            0 => Ok(data.to_vec()),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, (e.uncompressed_size as usize).max(1) * 2 + 1024)
                .map_err(|err| anyhow::anyhow!("inflate {}: {:?}", e.name, err)),
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
    let mut entries = Vec::with_capacity(count as usize);
    let mut p = cd_offset as usize;
    for _ in 0..count {
        if p + 46 > b.len() || u32_at(b, p) != 0x02014b50 {
            bail!("bad central directory entry");
        }
        let method = u16_at(b, p + 10);
        let mut csize = u32_at(b, p + 20) as u64;
        let mut usize_ = u32_at(b, p + 24) as u64;
        let name_len = u16_at(b, p + 28) as usize;
        let extra_len = u16_at(b, p + 30) as usize;
        let comment_len = u16_at(b, p + 32) as usize;
        let mut lho = u32_at(b, p + 42) as u64;
        let name_bytes = &b[p + 46..p + 46 + name_len];
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        // zip64 extra
        let mut e = p + 46 + name_len;
        let extra_end = e + extra_len;
        while e + 4 <= extra_end {
            let id = u16_at(b, e);
            let sz = u16_at(b, e + 2) as usize;
            if id == 0x0001 {
                let mut q = e + 4;
                if usize_ == 0xFFFF_FFFF && q + 8 <= e + 4 + sz {
                    usize_ = u64_at(b, q);
                    q += 8;
                }
                if csize == 0xFFFF_FFFF && q + 8 <= e + 4 + sz {
                    csize = u64_at(b, q);
                    q += 8;
                }
                if lho == 0xFFFF_FFFF && q + 8 <= e + 4 + sz {
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
