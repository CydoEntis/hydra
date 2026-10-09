# Plan: Seshi

Source of truth for scope, order, decisions and rules. Tickets hold the detail.
Read this before starting work. If work conflicts with it, stop and say so.
Last reconciled: 2026-10-05 at `9cfde2c` on `dev`.
Verify: `cargo test && cargo clippy --all-targets -- -D warnings && cargo check --target x86_64-unknown-linux-gnu && cargo check --target aarch64-apple-darwin`

Seshi is a terminal multiplexer built for running many coding agents at once: see
which ones need you, jump to them, and keep them running when you leave. See
`AGENTS.md` for the mission and `docs/ARCHITECTURE.md` for how it is built.

## Now

Active phase: **7 — test-week feedback** (collecting: the user runs Seshi for a week)
Next unblocked: none (the rest is on hold)

## Phases

### 1 — Working with agents feels effortless (Batch A) · complete 2026-10-03 at `54855dd`

Quick follow-ups, done-not-seen, smarter status, render speed, find file / search
code, agents moving themselves into worktrees. (Tracked as a checklist before
tickets existed.)

### 2 — Launching and steering agents from one place (Batch B) · complete 2026-10-03 at `54855dd`

Task box, agent presets, `seshi send --wait` / `seshi wait`, live model and
last-prompt per row.

### 3 — Code and environment without leaving Seshi (Batch C) · complete 2026-10-03 at `54855dd`

Branch switcher, review marks in Changes, dev server per worktree with hooks,
prewarmed agents.

### 4 — Reach and extras (Batch D) · complete 2026-10-03 at `54855dd`

SSH remotes, extensions, tabs and splits in the seshi layout, memory view,
notification history, `seshi doctor`, more agent types, BEL tracking.

### 5 — Safe and steady · complete 2026-10-04 at `f3b7ae7`

#1, #2, #3, #4, #5, #6, #7, #8, #9, #10, #11, #12

### 6 — Clean code: one layout's worth of code, in files a person can read · complete 2026-10-04 at `0aba78f`

#13, #14, #15, #16, #17, #18

### 7 — Test-week feedback · in progress

What a week of daily use turns up. Bugs found were fixed straight away (0.3.1–0.6.6). #19 is
done. Chosen 2026-10-05 after comparing with tuios, in this order:

Done: #20 sidebar sections (0.7.0), #21 Inbox (0.7.5), #22 Claude Code agent teams (0.7.7);
#25 teach an agent / Codex setup / DeepSeek built in (0.7.1, 0.7.6); #24 ask-human (0.7.8).

Next, in order:

1. #25 (rest) Hooks for Gemini, opencode, Qwen in one command
2. #24 (rest) Per-pane permissions for what an agent may do through seshi
3. #28 Scrollback: jump between commands (0.8.1), multi-pane copy (0.9.1) — done
4. #26 Layouts: split, grid, main and stack, columns; popups — done (0.9.0); tabs 1-9 are the workspaces

On hold (2026-10-05, the user's call): #31 package managers (AUR name and account to decide),
#23 more machines, #27 graphics, #29 extras, #30 web terminal and SSH server mode.

## In scope

Everything below is shipped on `dev` unless marked.

**Core**
- Daemon owns sessions; detach / reattach; many clients — shipped — `src/daemon/mod.rs`, ADR-0001
- Sessions survive a daemon restart (agents resumed, commands re-run) — shipped — `src/daemon/persist.rs`, `restore` in `src/daemon/mod.rs`
- Windows (ConPTY, named pipes, PowerShell folder tracking), macOS, Linux — shipped — `src/daemon/term.rs`, `src/ipc.rs`
- SSH remotes (`--remote host`) — shipped — `src/ipc.rs` `connect_remote`

**Agents**
- Status detection: working / needs you / done (until seen) / idle, from screen patterns, OSC progress and agent hooks — shipped — `src/daemon/scan.rs`, `seshi hook` in `src/cli.rs`
- Trusted status reports (process tree or `SESHI_PANE_TOKEN`) — shipped — `src/daemon/term.rs`, `src/cli.rs`
- Agent kinds: Claude, Codex, Gemini, OpenCode, Cursor, Copilot, Amp, Qwen, Aider, Goose, Crush, Droid, Pi, Kiro, Grok, custom `[[agents]]` — shipped — `src/config.rs`
- New agent task box (project, worktree, agent, model, effort), presets and recipes as its choices, prewarm — shipped — `src/client/hydra/dialogs.rs`, `src/client/recipes.rs`
- Agents talk to agents (MCP server `seshi mcp`) — shipped — `src/mcp.rs`
- `seshi send --wait`, `seshi wait`, `seshi read` — shipped — `src/cli.rs`

**The UI (one layout, ADR-0004)**
- Floating look: rounded cards with gaps, title and state in the border, unfocused cards faded, pill tabs, no app footer; floating or tiled, corners, gap, dim, focus border, pill ends in Settings — done, unreleased — `src/client/hydra/card.rs`, `screen.rs`, ADR-0007
- Sidebar card: projects → sessions, state glyph + name, branch · age, question under it, attention sort, keyboard and mouse, row letter keys, `a actions` / `, settings` foot — shipped — `src/client/hydra/screen.rs` `draw_side`
- Panes: tabs as pills (named in their pill), any number of splits (grid for 3+), zoom, drag the gap between them, ✕ in the border — shipped — `src/client/hydra/screen.rs` `draw_session`
- Inbox (what needs you first, type to go anywhere), command palette (plain-word commands), key map (searchable, second steps), actions list — shipped — `draw_goto`, `draw_palette`, `src/client/hydra/leader.rs`
- Right-click menus, confirm before closing, toasts — shipped — `src/client/menu.rs`
- Splash: Resume / New shell here — shipped — `draw_splash`
- Settings popup grouped by section, key rebinding, themes with contrast audit — shipped — `draw_settings`, `src/theme.rs`
- Leader keys from the floating design's key map, armed pill in sky — done, unreleased — `src/keys.rs` `DEFAULT_PREFIX_KEYS`, ADR-0006
- Copy on select with toast, copy mode, Ctrl+click paths, paste images — shipped — `src/client/copy.rs`, `src/client/pick.rs`

**Code and git**
- Worktrees per agent, create / move / remove with hooks — shipped — `src/daemon/git.rs`, `src/project.rs`
- Files (tree, preview, in-place edit, external editor), find file, search code — shipped — `src/client/files.rs`, `src/client/find.rs`, `src/client/views.rs`
- Changes (diff, review marks, commit, git init offer), branch switcher, pull requests, Ship — shipped — `src/client/branch.rs`, `src/client/pr.rs`
- Dev server per worktree (`.seshi.toml`) — shipped — `src/project.rs`
- Tasks (a worktree's review: commit, merge, PR, throw away) — shipped — `src/client/tasks.rs`
- Own worktree for `claude`/`codex` typed into a shell in a repo's main folder — shipped 0.9.12 — `seshi agent-dir`, `daemon/term.rs` `agent_functions`
- Usage and limits meter, context per agent, continue after a limit — shipped 0.10.0 — `seshi statusline`, `src/daemon/usage.rs`
- Merge that cleans up (the worktree and its branch go) — shipped 0.11.0 — `src/client/view_keys.rs`

**Identity**
- Renamed hydra → Seshi: command, folders, `SESHI_*`, `.seshi.toml`, MCP, releases; one-time move of an old install — done, unreleased — `config::migrate_old_names`, ADR-0008
- Seshi Night (default) and Seshi Day themes; block SESHI wordmark on the splash — done, unreleased — `src/theme.rs`, `src/client/hydra/splash.rs`

**Extras**
- Extensions (manifest, commands, hooks) — shipped — `src/ext.rs`
- Desktop alerts and sounds, notification history — shipped — `src/alert.rs`
- Click a notification to jump to its session — shipped — `src/reveal.rs`, `src/alert.rs` (#19)
- Sidebar groups sessions by where they work now (repo, or folder outside git); no projects to open; splits stay with their session — shipped — `src/client/hydra/mod.rs`
- Releases and installers (Windows, macOS, Linux; checksums), `seshi update` and a daily update check — shipped — `.github/workflows/release.yml`, `install.sh`, `install.ps1`, `src/update.rs`
- CI: test and clippy on Linux and Windows, a macOS check — shipped — `.github/workflows/ci.yml`
- Config sync across machines (local file never synced) — shipped — `src/sync.rs`
- `seshi doctor` — shipped — `src/cli.rs`

## Out

- Features cut 2026-10-09 (the user's call: they repeated something else or weren't used): talk / reply / answer, open project, go to, next agent that needs you, arrange panes, undo automatic workspace, quick prompt, the map, ideas, races, the queue, run a preset (presets stay in the new-agent dialog), memory per session, checkpoints, past chats, agent tools, tickets (GitHub, Linear, Plane). Bringing one back needs a new decision.
- Other UI layouts (workspaces, tree, dock, sidebar) — declined 2026-10-03: one layout to build and test (ADR-0004, now ADR-0007: floating and tiled are styles of it). Bringing them back needs a new decision.

## Later

- none.

## Decisions

- [ADR-0001](docs/adr/0001-daemon-owns-sessions.md) — A daemon owns every session; clients only draw. Rules out: sessions inside the UI process.
- [ADR-0002](docs/adr/0002-portable-pty-and-vt100.md) — portable-pty (ConPTY) and vt100 for terminals. Rules out: a home-grown PTY layer.
- [ADR-0003](docs/adr/0003-msgpack-over-local-sockets.md) — msgpack frames over local sockets, versioned. Rules out: unversioned message changes; network listeners.
- [ADR-0006](docs/adr/0006-floating-design-keys.md) — Leader keys follow the floating design's key map (supersedes ADR-0005). Rules out: defaults that differ from the key map shown in the app.
- [ADR-0008](docs/adr/0008-rename-to-seshi.md) — The app is called Seshi; an old hydra install is brought over once. Rules out: new code or paths under the old name.
- [ADR-0007](docs/adr/0007-floating-and-tiled-styles.md) — One layout, drawn floating or tiled (supersedes ADR-0004). Rules out: other layouts, styles that move or hide parts of the UI.

## Rules

- The daemon's socket only accepts the user who started it — `src/ipc.rs` — SECURITY.md
- Status reports need the pane's token or its process chain — `src/daemon/mod.rs` hook handling — AGENTS.md non-negotiable 5
- Nothing in a cloned repo runs without the user's approval — `src/project.rs` — SECURITY.md
- Client and daemon refuse to talk across protocol versions — `src/protocol.rs` `PROTOCOL_VERSION`, checked on `Hello` — ADR-0003
- Config always loads the seshi layout — `src/config.rs` `Config::load` — ADR-0007
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
