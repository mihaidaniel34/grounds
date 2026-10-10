//! On-disk cache of declaration stubs keyed by file content hash, so cold
//! starts only re-parse files that changed since the last run.

use grounds_index::FileStub;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const VERSION: u32 = 13; // bump whenever stub extraction changes

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
        let bytes = postcard::to_stdvec(self)?;
        let tmp = p.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(tmp, p)?;
        Ok(())
    }
}

pub fn content_hash(text: &str) -> u64 {
    xxhash_rust::xxh3::xxh3_64(text.as_bytes())
}
