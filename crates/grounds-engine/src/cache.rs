//! On-disk cache of declaration stubs keyed by file content hash, so cold
//! starts only re-parse files that changed since the last run.

use grounds_index::FileStub;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const VERSION: u32 = 150; // bump whenever stub extraction changes

#[derive(Default, Serialize, Deserialize)]
pub struct StubCache {
    version: u32,
    entries: FxHashMap<String, (u64, Arc<FileStub>)>,
}

impl StubCache {
    pub fn path(root: &Path) -> PathBuf {
        root.join(".grounds/cache/stubs.bin")
    }

    pub fn load(root: &Path) -> StubCache {
        let p = Self::path(root);
        let Ok(bytes) = std::fs::read(&p) else { return StubCache { version: VERSION, ..Default::default() } };
        match postcard::from_bytes::<StubCache>(&bytes) {
            Ok(c) if c.version == VERSION => c,
            _ => StubCache { version: VERSION, ..Default::default() },
        }
    }

    pub fn get(&self, path: &str, hash: u64) -> Option<Arc<FileStub>> {
        match self.entries.get(path) {
            Some((h, s)) if *h == hash => Some(s.clone()),
            _ => None,
        }
    }

    pub fn put(&mut self, path: String, hash: u64, stub: Arc<FileStub>) {
        self.entries.insert(path, (hash, stub));
    }

    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.entries.retain(|k, _| keep(k));
    }

    pub fn save(&self, root: &Path) -> anyhow::Result<()> {
        let p = Self::path(root);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        // (the daemon and an in-process fallback may save at the same time)
        crate::fsutil::write_replacing(&p, &postcard::to_stdvec(self)?)?;
        Ok(())
    }
}

pub fn content_hash(text: &str) -> u64 {
    xxhash_rust::xxh3::xxh3_64(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The daemon and an in-process fallback may save the cache at the same
    /// time. Through one fixed temporary file, one's rename found the file
    /// already renamed by the other and failed, or renamed the other's half
    /// written content. Each save now writes a file of its own: every one
    /// succeeds, and the cache left is one of them, whole.
    #[test]
    fn caches_saved_at_once_are_each_saved_whole() {
        let root = std::env::temp_dir().join(format!("grounds-cache-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let writers: Vec<_> = (0..8u64)
            .map(|w| {
                let root = root.clone();
                std::thread::spawn(move || {
                    (0..40u64).map(|i| {
                        let mut cache = StubCache { version: VERSION, ..Default::default() };
                        // (of a size of its own, so that a torn file does not read as one)
                        for f in 0..=(w * 40 + i) % 50 {
                            cache.put(format!("F{f}.java"), w, Arc::new(FileStub::default()));
                        }
                        cache.save(&root).map_err(|e| e.to_string())
                    }).collect::<Vec<_>>()
                })
            })
            .collect();
        for w in writers {
            for saved in w.join().unwrap() {
                assert_eq!(saved, Ok(()));
            }
        }
        let left = StubCache::load(&root);
        assert!(!left.entries.is_empty(), "a whole cache of one of the writers");
        let dir = StubCache::path(&root).parent().unwrap().to_path_buf();
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, ["stubs.bin"], "no temporary file left");
        let _ = std::fs::remove_dir_all(&root);
    }
}
