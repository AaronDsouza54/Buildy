use clap::{Parser, Subcommand};
use colored::Colorize;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rustyline::Editor;
use rustyline::error::ReadlineError;
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
    /// Perform a build and exit
    Build {
        #[arg(long)]
        release: bool,
    },
    /// Start the watch daemon with an interactive repl
    Watch,

    Run {
        /// Build in release mode
        #[arg(long)]
        release: bool,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let cwd = if cli.root.as_os_str() == "." {
        env::current_dir()?
    } else {
        cli.root.clone()
    };

    match cli.command {
        Commands::Build { release } => {
            let mut cache = BuildCache::load();
            let is_debug = !release;
            run_build(&cwd, &mut cache, is_debug)?;
            cache.save()?;
        }
        Commands::Run { release } => {
            let mut cache = BuildCache::load();
            let is_debug = !release;
            let exe_path = run_build(&cwd, &mut cache, is_debug)?;
            println!("executable path: {}", exe_path.display());
            cache.save()?;
            run_executable(&exe_path)?;
        }
        Commands::Watch => {
            watch_mode(cwd)?;
        }
    }

    Ok(())
}

/// Build the project and return the path to the executable if linking occurred.
fn run_build(
    root: &Path,
    cache: &mut BuildCache,
    is_debug: bool,
) -> Result<PathBuf, Box<dyn Error>> {
    println!("scanning sources in {}", root.display());

    let mut graph = BuildGraph::new();
    graph.scan(root, &[])?;
    // remove cache entries for files that no longer exist
    let existing: HashSet<String> = graph
        .nodes
        .keys()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    cache.files.retain(|k, _| existing.contains(k));

    // if compiler or flags changed since last cache, invalidate all
    let current_compiler = "gcc".to_string();
    let current_flags: Vec<String> = vec!["-g".into()];
    if cache.compiler.as_ref() != Some(&current_compiler) || cache.flags != current_flags {
        println!("compiler or flags changed, invalidating cache");
        for meta in graph.nodes.values_mut() {
            meta.dirty = true;
        }
    }
    cache.compiler = Some(current_compiler);
    cache.flags = current_flags.clone();

    // graph.update_dirty(cache);
    graph.update_dirty(&cache);

    let need_link = scheduler::build(&mut graph, cache, root, is_debug)?;
    let exe_name = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "a.out".into());

    let profile_dir = if is_debug { "debug" } else { "release" };
    let output_dir = root.join("target").join(profile_dir);
    std::fs::create_dir_all(&output_dir)?;
    let output_path = output_dir.join(&exe_name);

    if need_link {
        scheduler::link(&graph, root, is_debug, &output_path)?;
    } else {
        println!("nothing to link");
    }

    Ok(output_path)
}

/// Run an executable from a given path.
fn run_executable(exe_path: &Path) -> Result<(), Box<dyn Error>> {
    if exe_path.exists() {
        std::process::Command::new(exe_path).status()?;
    } else {
        println!("executable not found, build first");
    }
    Ok(())
}

fn watch_mode(root: PathBuf) -> Result<(), Box<dyn Error>> {
    println!("starting watch daemon in {}", root.display());

    let (tx, rx) = channel();
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
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

    let result: Result<(), Box<dyn Error>> = (|| {
        loop {
            // drain filesystem events
            while let Ok(path) = rx.try_recv() {
                changed.insert(path);
            }
            let prompt = "buildy> ".red().bold().to_string();

            match rl.readline(&prompt) {
                Ok(line) => {
                    let args = shell_words::split(line.trim())
                        .unwrap_or_else(|_| vec![line.trim().to_string()]);
                    if args.is_empty() {
                        continue;
                    }

                    let mut argv = vec!["repl".to_string()];
                    argv.extend(args);

                    let trimmed = line.trim();

                    if trimmed == "exit" || trimmed == "close" {
                        println!("shutting down");
                        break;
                    } else if trimmed == "help" {
                        println!("available commands: build, run, close, help");
                        println!("flags available are --release")
                    }

                    match Cli::try_parse_from(&argv) {
                        Ok(cli) => match cli.command {
                            Commands::Build { release } => {
                                let is_debug = !release;
                                run_build(&root, &mut cache, is_debug)?;
                                changed.clear();
                            }
                            Commands::Run { release } => {
                                let is_debug = !release;
                                let exe_path = run_build(&root, &mut cache, is_debug)?;
                                changed.clear();
                                run_executable(&exe_path)?;
                            }
                            Commands::Watch => println!("Already in watch mode."),
                        },
                        Err(e) => println!("{}", e),
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    println!("CTRL-C");
                    break;
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
    })();

    // ✅ save cache no matter how we exited the loop
    cache.save()?;
    println!("Cache saved. Goodbye!");

    result
}
