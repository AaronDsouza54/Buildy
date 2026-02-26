# Buildy

Buildy is a simple Rust-based build tool for C/C++ projects. It tracks
file dependencies (via `gcc -MM`), computes hashes, and performs incremental
compilation with a daemon mode and a REPL interface.

## Features

- Scans source (`.c`, `.cpp`, `.h`, etc.) files and builds a dependency graph
- Computes file hashes and last-modified times to detect dirtiness
- Topologically sorts changed files and compiles in parallel using all CPU
  cores
- Parses user headers using GCC/G++ flags; ignores system headers
- Maintains a cache to avoid unnecessary recompilation
- Watch mode with interactive REPL (`build`, `run`, `close`, `help`)
- Tracks deleted/renamed files and invalidates cache accordingly
- Supports specifying project root via `--root` option

## Usage

Build from source:

```sh
cargo build --release
```

Run one-shot build in current or specified directory:

```sh
cargo run -- build           # build in current directory
cargo run -- --root=path build # build in given path
```

Start the daemon with REPL:

```sh
cargo run -- watch
```

Commands available in REPL:

- `build` – trigger a build based on changed files
- `run` – execute the linked binary (named after project directory)
- `close` or `exit` – save state and quit the daemon
- `help` – display command list

The tool stores its cache in `.buildy_cache.json` in the project root.

## Notes

Currently only C and C++ compilation is supported (using `gcc`/`g++`).
For release builds you can modify flags or extend configuration.

This repository is a starting point; further enhancements such as
compiler-version detection, custom flags, or more intelligent incremental
scanning can be added.
EOF"
