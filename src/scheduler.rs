use crate::cache::BuildCache;
use crate::graph::BuildGraph;
use crate::target::FileMeta;
use num_cpus;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Simple scheduler that walks the topologically sorted order and compiles dirty
/// nodes in parallel but respects dependency order.
pub fn build(
    graph: &mut BuildGraph,
    cache: &mut BuildCache,
    root: &std::path::Path,
    is_debug: bool,
) -> Result<bool, String> {
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
            cache.update_file(meta, root);
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
                if let Err(e) = compile_file(&meta, root, is_debug) {
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
            cache.update_file(m, root);
        }
    }

    // also update cache for others (for example, header timestamps)
    for meta in graph.nodes.values() {
        cache.update_file(meta, root);
    }

    Ok(need_link)
}

/// compile a single source file into an object file using gcc/g++ based on
/// extension.  The object file will reside next to the source with a .o
/// extension.  Current simplistic command; flags and include paths should be
/// provided by the graph/config.
fn compile_file(meta: &FileMeta, root: &Path, is_debug: bool) -> Result<(), String> {
    let profile_dir = if is_debug { "debug" } else { "release" };
    let target_dir = root.join("target").join(profile_dir);

    std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;

    let file_stem = meta.path.file_stem().ok_or("invalid file name")?;
    let obj_path = target_dir.join(file_stem).with_extension("o");

    let mut cmd = if meta.path.extension().and_then(|s| s.to_str()) == Some("c") {
        Command::new("gcc")
    } else {
        Command::new("g++")
    };

    cmd.arg("-c");
    cmd.arg(&meta.path);
    cmd.arg("-o");
    cmd.arg(&obj_path);

    if is_debug {
        cmd.arg("-g");
    } else {
        cmd.arg("-O3");
    }

    let status = cmd.status().map_err(|e| e.to_string())?;
    if !status.success() {
        Err(format!("compiler failed on {}", meta.path.display()))
    } else {
        Ok(())
    }
}

/// Link all object files produced by the graph into a single executable.
/// The project name is the filename of the working directory, or provided
/// explicitly by the caller.
pub fn link(
    graph: &BuildGraph,
    root: &Path,
    is_debug: bool,
    output: &PathBuf,
) -> Result<(), String> {
    let profile_dir = if is_debug { "debug" } else { "release" };
    let target_dir = root.join("target").join(profile_dir);

    let mut objs: Vec<PathBuf> = Vec::new();

    for (path, _) in &graph.nodes {
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            if ["c", "cpp", "cc", "cxx"].contains(&ext) {
                let file_stem = path.file_stem().ok_or("invalid source filename")?;

                let obj_path = target_dir.join(file_stem).with_extension("o");

                if obj_path.exists() {
                    objs.push(obj_path);
                }
            }
        }
    }

    if objs.is_empty() {
        return Ok(()); // nothing to link
    }

    let mut use_cpp = false;

    for (path, _) in &graph.nodes {
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            if ["cpp", "cc", "cxx"].contains(&ext) {
                use_cpp = true;
                break;
            }
        }
    }

    let mut cmd = if use_cpp {
        Command::new("g++")
    } else {
        Command::new("gcc")
    };

    for obj in &objs {
        cmd.arg(obj);
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
