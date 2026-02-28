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
    /// Load cache from disk, normalizing any stored paths relative to the
    /// provided project `root`.  Older caches may contain absolute paths;
    /// those are converted during load so that the in-memory representation
    /// always uses paths relative to `root`.
    pub fn load(root: &std::path::Path) -> Self {
        if let Ok(s) = fs::read_to_string(CACHE_FILENAME) {
            if let Ok(mut c) = serde_json::from_str::<BuildCache>(&s) {
                c.normalize_paths(root);
                return c;
            }
        }
        BuildCache::default()
    }

    pub fn save(&mut self) -> io::Result<()> {
        self.saved_at = Utc::now();

        if let Some(parent) = std::path::Path::new(CACHE_FILENAME).parent() {
            fs::create_dir_all(parent)?;
        }

        let s = serde_json::to_string_pretty(self)?;
        let mut f = fs::File::create(CACHE_FILENAME)?;
        f.write_all(s.as_bytes())?;
        Ok(())
    }

    /// Update a cache entry for `meta`.  Internally the key is stored as a
    /// path _relative_ to the project root so that the cache file is
    /// transportable across machines or workspace relocations.
    pub fn update_file(&mut self, meta: &FileMeta, root: &std::path::Path) {
        let key = BuildCache::make_relative(&meta.path, root);
        self.files.insert(
            key,
            CachedEntry {
                hash: meta.hash.clone(),
                last_modified: meta.last_modified,
            },
        );
    }

    /// Check whether a given file matches the cached hash.  `meta.path` is
    /// converted to the corresponding relative key before lookup.
    pub fn file_matches(&self, meta: &FileMeta, root: &std::path::Path) -> bool {
        let key = BuildCache::make_relative(&meta.path, root);
        if let Some(entry) = self.files.get(&key) {
            entry.hash == meta.hash
        } else {
            false
        }
    }

    pub fn config_matches(&self, compiler: &str, flags: &[String]) -> bool {
        self.compiler.as_deref() == Some(compiler) && self.flags == flags
    }

    /// Iterate over the cached file paths as absolute `PathBuf`s, converting
    /// each stored relative key into an absolute path joined with `root`.
    pub fn iter_absolute_paths<'a>(
        &'a self,
        root: &'a std::path::Path,
    ) -> impl Iterator<Item = std::path::PathBuf> + 'a {
        self.files.keys().map(move |k| BuildCache::make_absolute(k, root))
    }

    /// Convert an absolute path to one relative to the project root.  If the
    /// path is not under `root` or the operation fails, fall back to the
    /// original string.
    /// Return a path string relative to the provided `root` (or the
    /// original path if it cannot be made relative).  This helper is public
    /// because callers (e.g. `main.rs`) need to generate relative keys when
    /// comparing the set of existing files.
    pub fn make_relative(path: &std::path::Path, root: &std::path::Path) -> String {
        if let Ok(rel) = path.strip_prefix(root) {
            rel.to_string_lossy().to_string()
        } else {
            path.to_string_lossy().to_string()
        }
    }

    /// Given a stored (relative) path string, return an absolute path by
    /// joining it with `root` when appropriate.
    pub fn make_absolute(rel: &str, root: &std::path::Path) -> std::path::PathBuf {
        let p = std::path::PathBuf::from(rel);
        if p.is_absolute() {
            p
        } else {
            root.join(p)
        }
    }

    /// Normalize any existing keys stored in `self.files` so they are all
    /// relative to `root`.  This is used when loading a cache that may have
    /// been written with absolute paths in older versions of the tool.
    fn normalize_paths(&mut self, root: &std::path::Path) {
        let mut newfiles = HashMap::new();
        for (k, v) in self.files.drain() {
            let p = std::path::PathBuf::from(&k);
            let key = if p.is_absolute() {
                if let Ok(rel) = p.strip_prefix(root) {
                    rel.to_string_lossy().to_string()
                } else {
                    k.clone()
                }
            } else {
                k.clone()
            };
            newfiles.insert(key, v);
        }
        self.files = newfiles;
    }
}
