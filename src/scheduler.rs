use crate::cache::BuildCache;
use crate::graph::BuildGraph;
use crate::target::FileMeta;
use num_cpus;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Simple scheduler that walks the topologically sorted order and compiles dirty
/// nodes in parallel but respects dependency order.
pub fn build(graph: &mut BuildGraph, cache: &mut BuildCache) -> Result<bool, String> {
    let mut need_link = false;

    // compute a build order for the dirty subset; if nothing is dirty just return
    let order = graph.topo_sort_dirty();
    if order.is_empty() {
        return Ok(false);
    }

    // gather metadata clones for the dirty ones
    let mut work: Vec<FileMeta> = Vec::new();
    for path in &order {
        if let Some(meta) = graph.nodes.get(path) {
            if meta.dirty {
                work.push(meta.clone());
            }
        }
    }

    if work.is_empty() {
        // nothing to compile
        for meta in graph.nodes.values() {
            cache.update_file(meta);
        }
        return Ok(false);
    }

    // create a thread pool using rayon
    let cpus = num_cpus::get();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(cpus)
        .build()
        .map_err(|e| e.to_string())?;

    let built = Arc::new(Mutex::new(Vec::new()));
    let error_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    pool.scope(|s| {
        for meta in work {
            let built_clone = built.clone();
            let err_flag = error_flag.clone();
            s.spawn(move |_| {
                if err_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    // somebody already failed, bail out
                    return;
                }
                if let Err(e) = compile_file(&meta) {
                    eprintln!("Error compiling {}: {}", meta.path.display(), e);
                    err_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
                built_clone.lock().unwrap().push(meta.path.clone());
            });
        }
    });

    let built_obj_files = built.lock().unwrap();
    if !built_obj_files.is_empty() {
        need_link = true;
    }

    if error_flag.load(std::sync::atomic::Ordering::Relaxed) {
        // abort build, keep dirty flags as they were
        return Err("compile failed".into());
    }

    // mark compiled metas as clean and update cache
    for p in built_obj_files.iter() {
        if let Some(m) = graph.nodes.get_mut(p) {
            m.dirty = false;
            cache.update_file(m);
        }
    }

    // also update cache for others (for example, header timestamps)
    for meta in graph.nodes.values() {
        cache.update_file(meta);
    }

    Ok(need_link)
}

/// compile a single source file into an object file using gcc/g++ based on
/// extension.  The object file will reside next to the source with a .o
/// extension.  Current simplistic command; flags and include paths should be
/// provided by the graph/config.
fn compile_file(meta: &FileMeta) -> Result<(), String> {
    let src = meta.path.to_string_lossy().to_string();
    let obj = meta
        .path
        .with_extension("o")
        .to_string_lossy()
        .to_string();

    let mut cmd = if src.ends_with(".c") {
        Command::new("gcc")
    } else {
        Command::new("g++")
    };

    cmd.arg("-c");
    cmd.arg(&src);
    cmd.arg("-o");
    cmd.arg(&obj);

    // always enable debug symbols for now
    cmd.arg("-g");

    let status = cmd.status().map_err(|e| e.to_string())?;
    if !status.success() {
        Err(format!("compiler returned non-zero status"))
    } else {
        Ok(())
    }
}

/// Link all object files produced by the graph into a single executable.
/// The project name is the filename of the working directory, or provided
/// explicitly by the caller.
pub fn link(graph: &BuildGraph, output: &PathBuf) -> Result<(), String> {
    let mut objs: Vec<String> = Vec::new();
    for (path, _meta) in &graph.nodes {
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            if ["c", "cpp", "cc", "cxx"].contains(&ext) {
                let obj = path.with_extension("o");
                if obj.exists() {
                    objs.push(obj.to_string_lossy().to_string());
                }
            }
        }
    }
    if objs.is_empty() {
        return Ok(());
    }

    let mut cmd = Command::new("gcc");
    for o in &objs {
        cmd.arg(o);
    }
    cmd.arg("-o");
    cmd.arg(output);

    let status = cmd.status().map_err(|e| e.to_string())?;
    if !status.success() {
        Err("linker returned non-zero status".into())
    } else {
        Ok(())
    }
}

