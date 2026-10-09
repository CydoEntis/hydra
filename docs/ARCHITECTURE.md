# Architecture

This doc covers how the code is organised, which way dependencies point, and
which design patterns to use. `AGENTS.md` is the operating contract; this doc
holds the architecture rules in depth.

## Principles

- **Dependencies point inward.** Stable, pure code sits at the centre. Volatile
  code — UI, frameworks, databases, networks — sits at the edge. Inner code never
  imports outer code.
- **Organise by feature, then by layer.** A feature's UI, use cases, types, and
  data access live together. Code becomes shared only when a second real
  consumer appears.
- **Boundaries are explicit.** Data crossing a boundary is validated and mapped
  into the receiving side's types. Row shapes, HTTP payloads, and SDK types never
  leak inward.
- **Side effects live at the edges.** Domain logic takes values and returns
  values. I/O happens in adapters that the outer layers call.
- **Choose the simplest structure that holds.** Add a layer, interface, or
  package only when it removes real complexity or has a current second
  consumer.

## Layers

```text
presentation     UI, HTTP handlers, CLI, IPC entry points
  -> application   use cases / services: orchestrate one user intent
    -> domain        entities, value objects, rules; pure and framework-free
application -> ports        interfaces the application needs (repositories, clock, mailer)
adapters    -> implement ports against real infrastructure (DB, HTTP, filesystem, SDKs)
```

- **Presentation** parses input, calls one use case, and renders the result. It
  holds no business rules and does no direct data access.
- **Application** coordinates domain objects and ports to carry out a single
  intent, such as `createInvoice` or `archiveProject`. It owns transactions and
  authorization checks.
- **Domain** holds the rules that would stay the same if the framework changed.
  It has no I/O, no framework imports, and no reads of global state.
- **Ports** are interfaces owned by the application layer. **Adapters** implement
  them, so infrastructure can be swapped or faked in tests.
- **The composition root** is the one place that constructs adapters and wires
  them into services, such as `main`, the app bootstrap, or the DI container. No
  other code calls `new` on an infrastructure class.

A small project can merge application and domain, or skip ports where only one
implementation will ever exist. When you do, say so in *Project-specific:
layers*. Never reverse the direction.

## Where code lives (default)

```text
src/
  main.rs        argument parsing, picks client / daemon / CLI role
  cli.rs         one-shot subcommands (send, read, wait, hook, allow, doctor, …)
  protocol.rs    wire messages and PROTOCOL_VERSION
  ipc.rs         local sockets and the SSH proxy
  config.rs      config.toml (+ config.local.toml) loading, atomic state files
  keys.rs        actions, default bindings, key parsing
  layout.rs      split trees and neighbour finding
  theme.rs       built-in themes and contrast audit
  clock.rs       time helpers
  proc.rs        running other programs: no console window, output or error as text,
                 git, shell command lines
  daemon/        the server
    mod.rs       the event loop, clients, snapshot, spawning panes
    commands.rs  commands from clients and the CLI
    status.rs    hook reports (checked on their own thread) and screen detection
    worktrees.rs making and trusting worktrees, spares, hooks, cleanup
    restore.rs   saving and restoring sessions, sleeping and waking agents
    term.rs      one pane: PTY, input thread, vt100 emulator
    scan.rs, git.rs, persist.rs
  client/        the UI
    mod.rs       App state, the event loop, shared types
    input.rs     keys, pastes, the mouse
    actions.rs   what each action does
    background.rs server messages and background results
    view_keys.rs keys inside full-pane views and the remaining popups
    hydra/       the layout: mod.rs (state, model, click targets), screen.rs (grid, sidebar
                 card, tab row, pane cards), card.rs (cards, pills, fading; the Look from
                 settings), leader.rs (key map, actions list, new tab), popups.rs,
                 dialogs.rs, splash.rs, behaviour.rs (its keys and clicks), pr_map.rs
                 (pull request, ship views)
    render.rs, design.rs  drawing entry point and shared helpers
    menu.rs, views.rs, files.rs, find.rs, branch.rs, recipes.rs, pr.rs, …
    tests.rs     rendering and behaviour tests
  mcp.rs, ext.rs, project.rs, gitfs.rs, alert.rs, reveal.rs, update.rs, sync.rs
docs/            these docs, the roadmap, design briefs
config.example.toml
```

Features never import from inside another feature. If feature A needs
something from feature B, B exposes it through its public entry file, or the
shared piece moves to `domain/` or `lib/`.

## Pattern catalog

Use a pattern when its trigger applies. A pattern with no trigger is only
ceremony.

| Pattern | Use when | Avoid when |
| --- | --- | --- |
| **Repository** | a use case needs to load or save aggregates without knowing storage | it only forwards to an ORM that is already a repository |
| **Port / Adapter** | an external system (DB, API, clock, filesystem) must be swappable or faked | only one implementation will exist and tests do not need a fake |
| **Service / Use case** | an operation coordinates several steps, rules, or ports | it only forwards a call. Call the repository directly instead. |
| **Value object** | a primitive carries rules (email, money, id, date range) | the value has no invariant |
| **Result / typed error** | a failure is expected and the caller must handle it | the failure is a bug. Throw and let it surface. |
| **Strategy** | behaviour varies by a known set of cases chosen at runtime | there are two cases and an `if` is clearer |
| **Factory** | construction has rules, defaults, or picks among implementations | a constructor or object literal is enough |
| **Dependency injection** (constructor or parameter) | a unit needs collaborators that tests must control | the dependency is a pure function. Import it. |
| **Domain events** | several independent reactions follow one state change | there is one reaction. Call it directly. |
| **Mapper** | data crosses a boundary with a different shape (row → entity, entity → DTO) | the shapes are identical and will stay so |

### Anti-patterns

- God services or "manager" classes that own unrelated use cases.
- Generic `BaseRepository<T>` or `CrudService<T>` built before two concrete
  repositories need it.
- Service locators, or globals reached from deep inside logic.
- Layers that only forward calls and add nothing.
- Business rules in UI components, HTTP handlers, SQL, or ORM hooks.
- Circular imports between features or layers.

## Cross-cutting concerns

- **Configuration** is read and validated once at startup into a typed object,
  then passed in. Nothing reads environment variables directly outside the
  config module. See [ENVIRONMENT](ENVIRONMENT.md).
- **Logging** is structured, carries a correlation or request id, and never
  includes secrets or personal data.
- **Errors** are typed at the domain and application layers. The presentation
  layer maps them to user-safe output. See [API_PATTERNS](API_PATTERNS.md).
- **Authorization** is checked in the application layer. Checks in the UI alone
  never count.
- **Time and ids** come from injected providers wherever determinism matters.

## Recording decisions

Record a decision that is expensive to reverse in `docs/adr/`, then summarise it
in the relevant project-specific section below with a link to the ADR. Examples
include the architecture style, framework, persistence engine, sync model, and
monorepo split.

## Project-specific: system context

Seshi is one binary that runs in two roles:

- **Daemon (server):** started on demand, one per user (or per `SESHI_SOCKET`
  name). Owns every pane: spawns shells and agents in pseudo-terminals
  (ConPTY on Windows), parses their output with `vt100`, detects agent status,
  manages worktrees, persists sessions, and restores them after a restart.
- **Client (UI):** the full-screen TUI. Connects over a local socket (named pipe
  on Windows, Unix socket elsewhere), receives snapshots and pane output, sends
  commands and keystrokes. Many clients can attach; any can detach.
- **CLI subcommands** (`seshi send`, `seshi read`, `seshi wait`, `seshi worktree`,
  `seshi doctor`, `seshi mcp`, `seshi proxy`, …) talk to the same daemon.

External systems: the agent CLIs it runs (claude, codex, …) and their hook
callbacks (`seshi hook`), `git` and `gh`, `ssh`
for remote machines (`--remote host` runs `ssh host seshi proxy`), the OS
clipboard and notifications, and an MCP server (`seshi mcp`) agents use to talk to
each other.

## Project-specific: stack

Rust, edition 2024, stable toolchain. Key libraries: `tokio` (async runtime),
`ratatui` 0.30 + `crossterm` (TUI), `portable-pty` (PTYs / ConPTY), `vt100`
(terminal emulation), `interprocess` (local sockets), `rmp-serde` (msgpack wire
format) over `tokio-util` length-delimited frames, `serde`/`toml`/`toml_edit`
(config), `sysinfo` (process trees, memory), `ignore` + `regex` (file search),
`arboard` (clipboard), `clap` (CLI), `anyhow` (errors), `tracing` (logs).
`vt100` is a patched copy in `vendor/vt100` (see its `SESHI.md`; wired in with
`[patch.crates-io]`): keep its changes there and listed in that file.
Rules for the language live in `docs/stack/rust.md`.

## Project-specific: layers

- **Domain / shared types:** `protocol.rs` (every message between client and
  daemon), `layout.rs` (split trees), `keys.rs` (actions and bindings),
  `theme.rs`, `config.rs`. No I/O beyond reading config.
- **Infrastructure:** `ipc.rs` (sockets, SSH proxy), `daemon/term.rs` (PTY +
  emulator per pane), `daemon/scan.rs` (status detection), `daemon/git.rs`,
  `daemon/persist.rs`, `gitfs.rs`, `alert.rs`, `reveal.rs` (notification links,
  bringing the window forward), `update.rs`, `sync.rs`, `ext.rs`, `mcp.rs`,
  `daemon/usage.rs` (agents' usage, limits, continue after a limit).
- **Application:** `daemon/mod.rs` (the server loop: commands in, state and
  output out) and `client/mod.rs` (the client loop: events in, commands out).
- **Presentation:** `client/hydra/` (the layout and its popups),
  `client/design.rs` (shared drawing helpers), `client/render.rs`, and the
  feature views in `client/` (files, changes, branches, PRs, …).

The client never touches a PTY; the daemon never draws. They meet only through
`protocol.rs` messages.

## Project-specific: directory layout

```text
src/
  main.rs        argument parsing, picks client / daemon / CLI role
  cli.rs         one-shot subcommands (send, read, wait, hook, doctor, …)
  protocol.rs    wire messages and PROTOCOL_VERSION
  ipc.rs         local sockets and the SSH proxy
  config.rs      config.toml (+ config.local.toml) loading
  keys.rs        actions, default bindings, key parsing
  layout.rs      split trees and neighbour finding
  theme.rs       built-in themes and contrast audit
  daemon/        the server: mod.rs (loop), term.rs (PTY + vt100), scan.rs
                 (status), git.rs, persist.rs
  client/        the UI: mod.rs (loop, input), hydra.rs (layout), design.rs
                 (drawing helpers), render.rs, menu.rs, views.rs, and one file
                 per feature (files, find, branch, recipes, pr, …)
  mcp.rs, ext.rs, project.rs, gitfs.rs, alert.rs, reveal.rs, update.rs, sync.rs
docs/            these docs, the roadmap, design briefs
config.example.toml
```

## Project-specific: patterns and conventions

- **Commands and snapshots.** The client sends `Command`s; the daemon answers
  with full `Snapshot`s plus per-pane output. The client derives its view model
  (`hy_model`) from the snapshot each frame instead of caching parallel state.
- **Modes.** Client UI state is one `Mode` enum (popups, sidebar focus, settings,
  …); each mode has a `draw_*` and an `on_*_key` function.
- **Hit testing.** Drawing registers clickable rects (`hit(app, rect, HyHit::…)`);
  mouse handling looks them up, so drawing and clicking can't drift.
- **Background work.** Slow work runs in `spawn_bg` and returns a `Bg` message.
- **Other programs** go through `proc.rs` (`proc::git`, `proc::run`, `proc::shell`,
  `proc::quiet`), never a hand-built `Command` with its own window flag or error parsing.
- **Filterable popups** use `query_list` + `list_row` (client/hydra/popups.rs).
- **Cards and pills** come from `card.rs`: `card()` for every pane, the sidebar and popups
  (title in the border, ✕ on the right), `pill()` / `row_pill()` for tabs, buttons and
  selected rows. Colours follow the design's rule: accent = focus, amber = needs you, sky =
  leader mode, red = destructive.
- **Popups** use the shared `panel()` helper (a lit card with ✕), `dim_all` behind, and
  `hints()` for the key line.


Decisions: [0001 daemon owns sessions](adr/0001-daemon-owns-sessions.md),
[0002 portable-pty + vt100](adr/0002-portable-pty-and-vt100.md),
[0003 msgpack over local sockets](adr/0003-msgpack-over-local-sockets.md),
[0004 one layout](adr/0004-one-layout.md),
[0005 herdr-compatible keys](adr/0005-herdr-compatible-keys.md).