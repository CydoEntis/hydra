# Plan: Hydra

Source of truth for scope, order, decisions and rules. Tickets hold the detail.
Read this before starting work. If work conflicts with it, stop and say so.
Last reconciled: 2026-10-05 at `9cfde2c` on `dev`.
Verify: `cargo test && cargo clippy --all-targets -- -D warnings && cargo check --target x86_64-unknown-linux-gnu && cargo check --target aarch64-apple-darwin`

Hydra is a terminal multiplexer built for running many coding agents at once: see
which ones need you, jump to them, and keep them running when you leave. See
`AGENTS.md` for the mission and `docs/ARCHITECTURE.md` for how it is built.

## Now

Active phase: **7 — test-week feedback** (collecting: the user runs Hydra for a week)
Next unblocked: none (no open tickets)

## Phases

### 1 — Working with agents feels effortless (Batch A) · complete 2026-10-03 at `54855dd`

Quick follow-ups, done-not-seen, smarter status, render speed, find file / search
code, agents moving themselves into worktrees. (Tracked as a checklist before
tickets existed.)

### 2 — Launching and steering agents from one place (Batch B) · complete 2026-10-03 at `54855dd`

Task box, agent presets, `hydra send --wait` / `hydra wait`, live model and
last-prompt per row.

### 3 — Code and environment without leaving Hydra (Batch C) · complete 2026-10-03 at `54855dd`

Branch switcher, review marks in Changes, dev server per worktree with hooks,
prewarmed agents.

### 4 — Reach and extras (Batch D) · complete 2026-10-03 at `54855dd`

SSH remotes, extensions, tabs and splits in the hydra layout, memory view,
notification history, `hydra doctor`, more agent types, BEL tracking.

### 5 — Safe and steady · complete 2026-10-04 at `f3b7ae7`

#1, #2, #3, #4, #5, #6, #7, #8, #9, #10, #11, #12

### 6 — Clean code: one layout's worth of code, in files a person can read · complete 2026-10-04 at `0aba78f`

#13, #14, #15, #16, #17, #18

### 7 — Test-week feedback · collecting

What a week of daily use turns up. Bugs found so far were fixed straight away (0.3.1–0.3.4:
closing the last pane, splits in the sidebar, a home shell when everything closes).
#19

## In scope

Everything below is shipped on `dev` unless marked.

**Core**
- Daemon owns sessions; detach / reattach; many clients — shipped — `src/daemon/mod.rs`, ADR-0001
- Sessions survive a daemon restart (agents resumed, commands re-run) — shipped — `src/daemon/persist.rs`, `restore` in `src/daemon/mod.rs`
- Windows (ConPTY, named pipes, PowerShell folder tracking), macOS, Linux — shipped — `src/daemon/term.rs`, `src/ipc.rs`
- SSH remotes (`--remote host`) — shipped — `src/ipc.rs` `connect_remote`

**Agents**
- Status detection: working / needs you / done (until seen) / idle, from screen patterns, OSC progress and agent hooks — shipped — `src/daemon/scan.rs`, `hydra hook` in `src/cli.rs`
- Trusted status reports (process tree or `HYDRA_PANE_TOKEN`) — shipped — `src/daemon/term.rs`, `src/cli.rs`
- Agent kinds: Claude, Codex, Gemini, OpenCode, Cursor, Copilot, Amp, Qwen, Aider, Goose, Crush, Droid, Pi, Kiro, Grok, custom `[[agents]]` — shipped — `src/config.rs`
- New agent task box (project, worktree, agent, model, effort), presets, prewarm — shipped — `src/client/work.rs`
- Message / reply / quick follow-up, answer prompts — shipped — `src/client/hydra.rs`
- Race agents on one task — shipped — `Action::Race`
- Agents talk to agents (MCP server `hydra mcp`) — shipped — `src/mcp.rs`
- `hydra send --wait`, `hydra wait`, `hydra read` — shipped — `src/cli.rs`
- Memory per session — shipped — `Mode::Memory`

**The UI (one layout, ADR-0004)**
- Sidebar: projects → sessions, status glyph + agent icon + name, attention sort, keyboard and mouse, row letter keys — shipped — `src/client/hydra.rs` `draw_side`
- Panes: tabs, any number of splits (grid for 3+), zoom, drag dividers, title bar with ✕ — shipped — `src/client/hydra.rs` `draw_session`
- Go to switcher, command palette (plain-word commands), Jump to what needs you, Keys screen — shipped — `draw_goto`, `draw_palette`, `draw_keys`
- Right-click menus, confirm before closing, toasts — shipped — `src/client/menu.rs`
- Splash: Resume / New / Open a folder — shipped — `draw_splash`
- Settings popup grouped by section, key rebinding, themes with contrast audit — shipped — `draw_settings`, `src/theme.rs`
- herdr-compatible leader keys — shipped — `src/keys.rs` `DEFAULT_PREFIX_KEYS`, ADR-0005
- Copy on select with toast, copy mode, Ctrl+click paths, paste images — shipped — `src/client/copy.rs`, `src/client/pick.rs`

**Code and git**
- Worktrees per agent, create / move / remove with hooks — shipped — `src/daemon/git.rs`, `src/project.rs`
- Files (tree, preview, in-place edit, external editor), find file, search code — shipped — `src/client/files.rs`, `src/client/find.rs`, `src/client/views.rs`
- Changes (diff, review marks, commit, git init offer), branch switcher, pull requests, Ship — shipped — `src/client/branch.rs`, `src/client/pr.rs`
- Tickets: GitHub issues/PRs, Linear, Plane — shipped — `src/client/inbox.rs`
- Dev server per worktree (`.hydra.toml`) — shipped — `src/project.rs`
- Ideas, map of the project, tasks — shipped — `Action::Ideas`, `Action::Map`, `src/client/tasks.rs`
- Agent tools view (MCP, skills, plugins; per project / global) — shipped — `src/client/toolbox.rs`

**Extras**
- Extensions (manifest, commands, hooks) — shipped — `src/ext.rs`
- Desktop alerts and sounds, notification history — shipped — `src/alert.rs`
- Click a notification to jump to its session — shipped — `src/reveal.rs`, `src/alert.rs` (#19)
- Sessions stay where they were started (`Home` on each workspace); quick shells outside projects (`s`) — shipped — `src/protocol.rs`, `src/client/hydra/mod.rs`
- Releases and installers (Windows, macOS, Linux; checksums), `hydra update` and a daily update check — shipped — `.github/workflows/release.yml`, `install.sh`, `install.ps1`, `src/update.rs`
- CI: test and clippy on Linux and Windows, a macOS check — shipped — `.github/workflows/ci.yml`
- Config sync across machines (local file never synced) — shipped — `src/sync.rs`
- `hydra doctor` — shipped — `src/cli.rs`

## Out

- Other UI layouts (workspaces, tree, dock, sidebar) — declined 2026-10-03: one layout to build and test (ADR-0004). Bringing them back needs a new decision.

## Later

- none.

## Decisions

- [ADR-0001](docs/adr/0001-daemon-owns-sessions.md) — A daemon owns every session; clients only draw. Rules out: sessions inside the UI process.
- [ADR-0002](docs/adr/0002-portable-pty-and-vt100.md) — portable-pty (ConPTY) and vt100 for terminals. Rules out: a home-grown PTY layer.
- [ADR-0003](docs/adr/0003-msgpack-over-local-sockets.md) — msgpack frames over local sockets, versioned. Rules out: unversioned message changes; network listeners.
- [ADR-0004](docs/adr/0004-one-layout.md) — One UI layout. Rules out: a layout setting, new code for old layouts.
- [ADR-0005](docs/adr/0005-herdr-compatible-keys.md) — Leader keys follow herdr where they overlap. Rules out: default bindings that clash with herdr's for shared actions.

## Rules

- The daemon's socket only accepts the user who started it — `src/ipc.rs` — SECURITY.md
- Status reports need the pane's token or its process chain — `src/daemon/mod.rs` hook handling — AGENTS.md non-negotiable 5
- Nothing in a cloned repo runs without the user's approval — `src/project.rs` — SECURITY.md
- Client and daemon refuse to talk across protocol versions — `src/protocol.rs` `PROTOCOL_VERSION`, checked on `Hello` — ADR-0003
- Config always loads the hydra layout — `src/config.rs` `Config::load` — ADR-0004
- Every built-in theme passes the contrast audit — `cargo test` (theme audit test in `src/theme.rs`) — readability
- Both platform sides compile — `cargo check --target x86_64-unknown-linux-gnu` / `aarch64-apple-darwin` — AGENTS.md non-negotiable 3
- clippy is clean — `cargo clippy --all-targets -- -D warnings` — CODE-STANDARDS

## Records

Docs: `AGENTS.md`, `docs/` (architecture, standards, testing, security, UI, API patterns), `docs/stack/rust.md`.
Design references: [design brief](design-brief.md), [design brief v3](design-brief-v3.md).
Glossary: none yet.
Feature docs: `docs/features/` (none yet).

## Rework

- none recorded yet.
