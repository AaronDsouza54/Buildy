use clap::{Parser, Subcommand};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rustyline::error::ReadlineError;
use rustyline::Editor;
use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;

mod cache;
mod graph;
mod hasher;
mod scheduler;
mod target;

use cache::BuildCache;
use graph::BuildGraph;

/// CLI for the buildy daemon/tool.
#[derive(Parser)]
struct Cli {
    /// Root directory of the project (defaults to current working directory)
    #[arg(long, default_value = ".")]
    root: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Perform a one-shot build and exit
    Build,
    /// Start the watch daemon with an interactive repl
    Watch,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let cwd = if cli.root.as_os_str() == "." {
        env::current_dir()?
    } else {
        cli.root.clone()
    };

    match cli.command {
        Commands::Build => {
            let mut cache = BuildCache::load();
            run_build(&cwd, &mut cache)?;
            cache.save()?;
        }
        Commands::Watch => {
            watch_mode(cwd)?;
        }
    }

    Ok(())
}

fn run_build(root: &Path, cache: &mut BuildCache) -> Result<(), Box<dyn Error>> {
    println!("scanning sources in {}", root.display());

    let mut graph = BuildGraph::new();
    graph.scan(root, &[])?;
    // remove cache entries for files that no longer exist
    let existing: std::collections::HashSet<String> =
        graph.nodes.keys().map(|p| p.to_string_lossy().to_string()).collect();
    cache.files.retain(|k, _| existing.contains(k));

    // if compiler or flags changed since last cache, invalidate all
    let current_compiler = "gcc".to_string();
    let current_flags: Vec<String> = vec!["-g".into()];
    if cache.compiler.as_ref() != Some(&current_compiler)
        || cache.flags != current_flags
    {
        println!("compiler or flags changed, invalidating cache");
        for meta in graph.nodes.values_mut() {
            meta.dirty = true;
        }
    }
    cache.compiler = Some(current_compiler);
    cache.flags = current_flags.clone();

    graph.update_dirty(cache);

    let need_link = scheduler::build(&mut graph, cache)?;
    if need_link {
        let exe_name = root
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "a.out".into());
        let output = root.join(&exe_name);
        println!("linking -> {}", output.display());
        if let Err(e) = scheduler::link(&graph, &output) {
            eprintln!("link failed: {}", e);
        }
    } else {
        println!("nothing to link");
    }

    Ok(())
}

fn watch_mode(root: PathBuf) -> Result<(), Box<dyn Error>> {
    println!("starting watch daemon in {}", root.display());

    let (tx, rx) = channel();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
        Ok(event) => {
            for path in event.paths {
                let _ = tx.send(path);
            }
        }
        Err(e) => eprintln!("watch error: {:?}", e),
    })?;
    watcher.watch(&root, RecursiveMode::Recursive)?;

    let mut rl: Editor<(), _> = Editor::new()?;
    let mut cache = BuildCache::load();
    let mut changed = HashSet::new();

    loop {
        // drain filesystem events
        while let Ok(path) = rx.try_recv() {
            changed.insert(path);
        }

        let prompt = if changed.is_empty() {
            "buildy> ".to_string()
        } else {
            format!("buildy*({})> ", changed.len())
        };

        match rl.readline(&prompt) {
            Ok(line) => {
                let cmd = line.trim();
                let _ = rl.add_history_entry(cmd);
                match cmd {
                    "build" => {
                        run_build(&root, &mut cache)?;
                        changed.clear();
                        cache.save()?;
                    }
                    "run" => {
                        let exe = root
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| "a.out".into());
                        let path = root.join(&exe);
                        if path.exists() {
                            println!("running {}", path.display());
                            let _ = std::process::Command::new(path).status();
                        } else {
                            println!("executable not found, build first");
                        }
                    }
                    "close" | "exit" => {
                        println!("shutting down");
                        cache.save()?;
                        break;
                    }
                    "help" => {
                        println!("available commands: build, run, close, help");
                    }
                    other => {
                        if !other.is_empty() {
                            println!("unknown command: {}", other);
                        }
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break;
            }
            Err(err) => {
                eprintln!("error reading line: {:?}", err);
                break;
            }
        }
    }

    Ok(())
}
