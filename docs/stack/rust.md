# Rust

> **Use for:** all code in this repo (one Rust 2024 crate, one binary).
> **Requires:** none
> **Fills:** CODE-STANDARDS § language and framework rules, CODE-STANDARDS § formatting and linting, TESTING § tools and commands

## Rules

### Errors
- Return `anyhow::Result` from fallible functions; add `.context("what was being done")` at I/O boundaries (files, sockets, processes, git).
- No `unwrap()` / `expect()` on values from outside the program: PTY output, the socket, config, the filesystem, environment, other processes. In tests, and on invariants the code just established, they are fine; say why in a comment when it isn't obvious.
- Don't swallow errors with `let _ =` unless failure truly doesn't matter (closing a dead pipe, a best-effort notification); comment the reason when it isn't obvious.
- User-visible failures become a toast (`notify(…, true)`) or a CLI error, never a panic.

### Panics and arithmetic
- Terminal geometry uses `saturating_sub` / `min` / `clamp`: windows can be tiny and rects empty.
- Index slices and strings only after checking bounds; slice strings on char boundaries (use `chars()`, `width()` from `unicode-width`, or the `truncate` helper).

### Async and threads
- No blocking calls (`std::fs`, `std::process::Command::output`, git, network) inside async tasks or the client's event loop; use `spawn_bg` / `spawn_blocking`.
- Never hold a `std::sync::Mutex` guard across an `.await`.
- Child processes are owned (`kill_on_drop` or explicit cleanup) so they can't outlive their purpose.

### Types and structure
- Model states as enums (`Mode`, `Status`, `Action`), not booleans and strings.
- Keep functions under ~80 lines; split a function when it grows a second job.
- `pub(crate)` / `pub(super)` by default; `pub` only for what other modules need.
- Prefer `&str` / `&Path` parameters; clone at the boundary, not deep inside.

### Unsafe
- No `unsafe` except where the platform requires it (`set_var` at startup, FFI); each block has a `// SAFETY:` comment.

### Dependencies
- Add a crate only when it saves real work; prefer ones already in the tree. Turn off default features you don't use.

## Commands

```sh
cargo build
cargo test                       # all tests
cargo test <name>                # one test
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo check --target x86_64-unknown-linux-gnu
cargo check --target aarch64-apple-darwin
```

## Project-specific: rust

- Edition 2024, stable toolchain; `[profile.dev.package."*"] opt-level = 2` keeps the emulator fast in debug builds.
- UI tests use ratatui's `TestBackend`; set `SESHI_SHOW=1` to print frames.
- Platform code is gated with `cfg(windows)` / `cfg(unix)`; both sides must compile (the cross-checks in the gate).
