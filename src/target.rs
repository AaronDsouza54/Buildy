use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Metadata about a single source file (C/C++).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMeta {
    /// Absolute path to the source file.
    pub path: PathBuf,
    /// Hash of the contents (sha256 of file plus dependencies when computed by graph).
    pub hash: String,
    /// Last modified time (stored as RFC3339 string because SystemTime doesn't
    /// serialize directly).
    pub last_modified: DateTime<Utc>,
    /// Direct dependencies (headers) that this file includes.
    pub deps: Vec<PathBuf>,
    /// Reverse dependencies: other files that depend on this one.
    #[serde(default)]
    pub dependents: Vec<PathBuf>,
    /// Whether the file is considered dirty and needs to be (re)compiled.
    #[serde(default)]
    pub dirty: bool,
}

impl FileMeta {
    pub fn new(path: PathBuf) -> io::Result<Self> {
        let metadata = fs::metadata(&path)?;
        let modified = metadata.modified()?;
        let last_modified: DateTime<Utc> = modified.into();

        Ok(FileMeta {
            path,
            hash: String::new(),
            last_modified,
            deps: Vec::new(),
            dependents: Vec::new(),
            dirty: true,
        })
    }

    pub fn refresh<T>(&mut self, hash_fn: T) -> io::Result<()>
    where
        T: Fn(&Path) -> io::Result<String>,
    {
        let metadata = fs::metadata(&self.path)?;
        let modified = metadata.modified()?;
        self.last_modified = modified.into();

        self.hash = hash_fn(&self.path)?; // always computes hash to ensure it reflects current content

        Ok(())
    }
}
