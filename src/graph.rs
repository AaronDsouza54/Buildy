use crate::cache::BuildCache;
use crate::hasher::hash_file;
use crate::target::FileMeta;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

/// BuildGraph keeps metadata for every source/header file we know about.
#[derive(Debug)]
pub struct BuildGraph {
    pub nodes: HashMap<PathBuf, FileMeta>,
}

impl BuildGraph {
    pub fn new() -> Self {
        BuildGraph {
            nodes: HashMap::new(),
        }
    }

    /// Scan the filesystem for C/C++ sources and headers and populate the
    /// graph. `extra_flags` are forwarded to the compiler when querying
    /// dependencies.
    pub fn scan(&mut self, root: &Path, extra_flags: &[String]) -> io::Result<()> {
        let exts = ["c", "cpp", "cc", "cxx", "h", "hpp"];
        for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                if exts.contains(&ext) {
                    let path = entry.path().canonicalize()?;
                    let meta = FileMeta::new(path.clone())?;
                    self.nodes.entry(path.clone()).or_insert(meta);
                }
            }
        }

        let keys: Vec<PathBuf> = self.nodes.keys().cloned().collect();
        for path in keys {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ["c", "cpp", "cc", "cxx"].contains(&ext) {
                    let deps = self.parse_deps(&path, extra_flags)?;
                    if let Some(node) = self.nodes.get_mut(&path) {
                        node.deps = deps.clone();
                    }
                    for d in deps {
                        self.nodes.entry(d.clone()).or_insert_with(|| FileMeta {
                            path: d.clone(),
                            hash: String::new(),
                            last_modified: chrono::Utc::now(),
                            deps: Vec::new(),
                            dependents: Vec::new(),
                            dirty: true,
                        });
                        if let Some(depnode) = self.nodes.get_mut(&d) {
                            depnode.dependents.push(path.clone());
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn parse_deps(&self, file: &Path, extra_flags: &[String]) -> io::Result<Vec<PathBuf>> {
        let compiler = if file
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "c")
            .unwrap_or(false)
        {
            "gcc"
        } else {
            "g++"
        };
        let mut cmd = Command::new(compiler);
        cmd.arg("-MM");
        for f in extra_flags {
            cmd.arg(f);
        }
        cmd.arg(file);
        let output = cmd.output()?;
        if !output.status.success() {
            return Ok(Vec::new());
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mut deps = Vec::new();
        for token in text.split_whitespace().skip(1) {
            let tok = token.trim_end_matches(['\\', ':'].as_ref());
            if tok.is_empty() {
                continue;
            }
            if tok.starts_with('/') || tok.starts_with('<') {
                continue;
            }
            let candidate = PathBuf::from(tok);
            if candidate.exists() {
                deps.push(candidate);
            }
        }
        Ok(deps)
    }

    pub fn update_dirty(&mut self, cache: &BuildCache, root: &std::path::Path) {
        for meta in self.nodes.values_mut() {
            let _ = meta.refresh(|p| hash_file(p));
            if !cache.file_matches(meta, root) {
                meta.dirty = true;
            }
        }
        let mut queue: VecDeque<PathBuf> = self
            .nodes
            .iter()
            .filter(|(_, m)| m.dirty)
            .map(|(p, _)| p.clone())
            .collect();
        let mut seen = HashSet::new();
        while let Some(p) = queue.pop_front() {
            if !seen.insert(p.clone()) {
                continue;
            }
            // copy dependents list to avoid borrowing conflict when mutably accessing nodes later
            let dependents = if let Some(node) = self.nodes.get(&p) {
                node.dependents.clone()
            } else {
                Vec::new()
            };
            for dep in dependents {
                if let Some(dnode) = self.nodes.get_mut(&dep) {
                    if !dnode.dirty {
                        dnode.dirty = true;
                        queue.push_back(dep.clone());
                    }
                }
            }
        }
    }

    pub fn topo_sort_dirty(&self) -> Vec<PathBuf> {
        // determining the set of files we actually care about (dirty or dependent on dirty)
        let mut dirty_set: HashSet<PathBuf> = self
            .nodes
            .iter()
            .filter(|(_, m)| m.dirty)
            .map(|(p, _)| p.clone())
            .collect();
        let mut queue: Vec<PathBuf> = dirty_set.iter().cloned().collect();
        while let Some(p) = queue.pop() {
            if let Some(node) = self.nodes.get(&p) {
                for dep in &node.dependents {
                    if dirty_set.insert(dep.clone()) {
                        queue.push(dep.clone());
                    }
                }
            }
        }

        // compute in-degrees restricted to dirty_set
        let mut indeg: HashMap<PathBuf, usize> = HashMap::new();
        for path in &dirty_set {
            indeg.insert(path.clone(), 0);
        }
        for path in &dirty_set {
            if let Some(node) = self.nodes.get(path) {
                for dep in &node.deps {
                    if dirty_set.contains(dep) {
                        *indeg.get_mut(path).unwrap() += 1;
                    }
                }
            }
        }

        // Kahn's algorithm
        let mut q: VecDeque<PathBuf> = indeg
            .iter()
            .filter_map(|(p, &d)| if d == 0 { Some(p.clone()) } else { None })
            .collect();
        let mut order = Vec::new();
        while let Some(n) = q.pop_front() {
            order.push(n.clone());
            if let Some(node) = self.nodes.get(&n) {
                for dep in &node.dependents {
                    if dirty_set.contains(dep) {
                        let e = indeg.get_mut(dep).unwrap();
                        *e -= 1;
                        if *e == 0 {
                            q.push_back(dep.clone());
                        }
                    }
                }
            }
        }

        // filter to sources
        order
            .into_iter()
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|ext| matches!(ext, "c" | "cpp" | "cc" | "cxx"))
                    .unwrap_or(false)
            })
            .collect()
    }
}
