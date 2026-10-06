# vt100 0.16.2, patched for hydra

A copy of [vt100](https://crates.io/crates/vt100) 0.16.2 (MIT, see LICENSE) with one change,
in `src/grid.rs` `scroll_up`:

- A line scrolled off the top of a scroll region that starts at the first row goes to the
  scrollback, as in xterm and Ghostty. Upstream keeps history only with no region set, so
  programs that print above a fixed input box through a region (Codex) had none to scroll.

Drop this copy (and the `[patch.crates-io]` entry in Cargo.toml) once upstream does the same.
- `scroll_region_active` (now unused) is removed.
