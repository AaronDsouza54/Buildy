use crate::target::FileMeta;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};

const CACHE_FILENAME: &str = "target/.buildy_cache.json";

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildCache {
    /// Entries keyed by source path string.
    pub files: HashMap<String, CachedEntry>,
    /// Compiler (gcc/g++) used for last build.
    pub compiler: Option<String>,
    /// Flags used for compilation.
    pub flags: Vec<String>,
    /// When saved, store timestamp.
    pub saved_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CachedEntry {
    pub hash: String,
    pub last_modified: DateTime<Utc>,
}

impl Default for BuildCache {
    fn default() -> Self {
        BuildCache {
            files: HashMap::new(),
            compiler: None,
            flags: Vec::new(),
            saved_at: Utc::now(),
        }
    }
}

impl BuildCache {
    pub fn load() -> Self {
        if let Ok(s) = fs::read_to_string(CACHE_FILENAME) {
            if let Ok(c) = serde_json::from_str(&s) {
                return c;
            }
        }
        BuildCache::default()
    }

    pub fn save(&mut self) -> io::Result<()> {
        self.saved_at = Utc::now(); // update timestamp
        let s = serde_json::to_string_pretty(self)?;
        let mut f = fs::File::create(CACHE_FILENAME)?;
        f.write_all(s.as_bytes())?;
        Ok(())
    }

    pub fn update_file(&mut self, meta: &FileMeta) {
        self.files.insert(
            meta.path.to_string_lossy().to_string(),
            CachedEntry {
                hash: meta.hash.clone(),
                last_modified: meta.last_modified,
            },
        );
    }

    pub fn file_matches(&self, meta: &FileMeta) -> bool {
        if let Some(entry) = self.files.get(&meta.path.to_string_lossy().to_string()) {
            entry.hash == meta.hash && entry.last_modified == meta.last_modified
        } else {
            false
        }
    }
}
