# hydra

An agent-aware terminal multiplexer you can bend to your workflow. It runs
natively on Windows, macOS and Linux, and is inspired by
[herdr](https://github.com/ogulcancelik/herdr), [nebula](https://github.com/agentSystemLabs/nebula)
and [fut](https://github.com/mikker/fut).

- **Agents keep running after you close the UI.** A background daemon owns every pseudoterminal
  (ConPTY on Windows). `<prefix> d` detaches, and running `hydra` again reattaches with scrollback replayed.
- **Survives restarts.** The layout is saved as it changes. After a reboot, `hydra` rebuilds every
  workspace, tab and pane in the right directory and resumes agents (`claude --resume <id>`,
  `codex resume --last`, ...).
- **Workspaces → tabs → split panes**, tmux style, with a sidebar listing every workspace (with
  its git branch and uncommitted-change count) and every agent.
- **A tree of everything.** The sidebar groups each repo's workspaces and worktrees, with the
  agents running in each underneath, plus the repo's worktrees you haven't opened yet. Click any
  row, or `<prefix> e` and use the arrow keys, to jump to a workspace or agent or open a worktree.
- **Agent dock.** Along the bottom, one card per agent shows its workspace, whether it needs you,
  and the last thing you asked it. Click a card to jump. `<prefix> B` hides it.
  (`[ui] layout` = `tree` (default), `dock` for no sidebar, or `sidebar` for a classic flat list.)
- **Leader key + key for everything**, with popups that list what you can do:
  - a command palette (`<prefix> Space`) that searches every command, workspace, agent and pane
  - a quick prompt (`<prefix> q`): type a task, Enter, and an agent starts on it in a split, a
    tab or a fresh worktree, or the task goes to the agent you're looking at
  - a pane menu (`<prefix> m`, or right-click)
  - a settings screen (`<prefix> S`) that saves to your config with its comments kept
- **Colour per workspace.** Every workspace gets its own colour: a faint wash behind its panes,
  the focused border, its tab and its sidebar marker. A glance tells you which workspace you're
  typing into. `<prefix> *` cycles the colour.
- **Workspace-scoped agents.** The sidebar lists the agents in the workspace you're in, plus one
  line summarising what the others need (`elsewhere ▲1 ◆2`, click to jump).
- **Git worktrees in one key.** `<prefix> W` lists the repo's worktrees: Enter opens one as a
  workspace (or switches to it), and typing a new branch name creates it. `<prefix> R` removes it.
- **Copy mode and search.** `<prefix> [` gives vim-style motion over the whole history, `v`/`V` select
  and `y` copies to the clipboard. `<prefix> /` searches history. Dragging with the mouse also
  selects and copies.
- **Status at a glance.** Each agent shows as working ● / blocked ▲ / done ◆ / idle ○ in the sidebar, the tab
  bar and the pane border. A bell rings when one needs you, and `<prefix> a` jumps to the next one.
- **Detection without setup.** The process tree finds `claude`, `codex`, `gemini`, `opencode`,
  `cursor-agent`, `copilot`, `amp`, `qwen`, `aider` and others, and screen patterns tell
  working from blocked. Hooks (`hydra integrate claude`) make it exact.
- **Everything is configurable.** Prefix, every binding, commands bound to keys, themes and
  per-colour overrides, icons, spinner, sidebar side and width, border style, detection
  patterns and your own agents.
- **Scriptable.** `hydra split -- claude`, `hydra send`, `hydra read` and `hydra ls --json`
  work from any shell, including from an agent running inside hydra.

## Install

```sh
cargo install --path .
```

## Use

```sh
hydra            # attach (starts the server if needed); opens a session in the current dir
hydra ~/code/api # open or switch to a session in that directory
```

The screen is **projects → worktrees → sessions**:

- **Projects** (top of the sidebar) are git repos. They appear by themselves from where your
  sessions run, plus any folder you open (`o`). Each shows how many sessions need you (`●1 ✓1 ⠹2`).
- **Worktrees** of the selected project, each with its **sessions** (agents and shells) under it,
  sorted by attention: needs you → done → working → idle. A session that needs you shows its
  question under it; subagents show as `↳ Explore`.
- The **main area** is the focused session, full size, or two side by side (stacked on narrow
  windows). Its title bar shows the agent, the session and its state; an agent that's waiting gets
  an answer bar with its own numbered choices.

The leader key is `Ctrl+Space`. Press it and wait a moment to see every binding.

| keys (after the leader) | action |
|---|---|
| `↑` `↓` | move through the sidebar's sessions; then Enter opens one, and bare keys work (`T`, `n`, …) until Esc |
| `j` | **Jump**: everything that needs you, then what finished, across all projects (`1`–`9` to go) |
| `T` / `r` | message the session (under the cursor, or the focused one) / reply to the focused one |
| `1` `2` `3` | answer the focused agent's numbered prompt |
| `n` | new session (your first agent) in this worktree |
| `p` | **New pane**: project, worktree (or *+ new worktree*, named for you), what to run; opens beside |
| `o` | **Open a project**: type a path, `cd ..`, fuzzy (`~\c\sa` → `~\code\shop-api`); Tab completes |
| `x` / `X` | close the split (both keep running) / end the focused session |
| `%` `\|` / `"` `-` | a shell beside this session |
| `←` `→` | the other session of a split |
| `$` | rename the session |
| `d` / `f` / `v` | changes (review, commit, PR) / files / tasks & review |
| `i` / `b` | inbox (PRs, issues, tickets) / toolbox (MCP, skills, plugins) |
| `q` | quick prompt: type a task, an agent starts on it |
| `Space` or `:` | command palette |
| `z` or `B` | hide / show the sidebar |
| `[` / `/` | copy mode / search history |
| `W` / `R` | worktree picker / remove worktree |
| `m` | menu |
| `,` / `?` | settings / every key |
| `D` / `Ctrl+R` | detach / reload config |

Everything also works with the mouse: projects, worktrees (click to fold), sessions, ` T `, the
answer buttons, Jump, + Pane, ✕ and Settings are all buttons. Hold Shift to select text with your
terminal.

## Splash and settings

hydra opens on the splash: the hydra, what happened while you were away (`● 2 need you ⠹ 3 still
working ✓ 1 finished`) and buttons: Open your last project (Enter), Jump to what needs you (`j`),
Open a folder (`o`), Settings (`,`). Turn it off in Settings → General.

`Ctrl+Space ,` opens **Settings**, with tabs (Tab cycles): General · Sessions · Appearance · Agents ·
Projects · Keys. Values are chips you click (or ←→ / Enter). Appearance has the themes (Default,
PaperColor Dark, Tango Dark, Monokai, Tokyo Night) with live swatches; the theme recolours agent
output's ANSI colours too. On Keys, Enter then press a new key to rebind a shortcut. Projects lists
the folders you opened (Enter forgets one). Everything is saved to `config.toml` (comments kept)
and applies at once.

## Command name clash

`hydra` is also the name of a Linux password-testing tool (THC-Hydra). If you have that installed,
give this one its own name; the app doesn't care what it's called:

```sh
alias hy='~/.cargo/bin/hydra'                          # bash / zsh
Set-Alias hy "$env:USERPROFILE\.cargo\bin\hydra.exe"   # PowerShell ($PROFILE)
```

## Configure

```sh
hydra config init   # writes the annotated example to the config path
hydra config path   # %APPDATA%\hydra\config.toml, or ~/.config/hydra/config.toml
```

See [`config.example.toml`](config.example.toml) for every option. Some highlights:

```toml
prefix = "ctrl+a"
theme = "tokyo-night"          # catppuccin-mocha/latte, tokyo-night, gruvbox, nord, dracula, mono

[theme_overrides]
accent = "#ff9e64"

[keys.prefix]
C = "spawn-right:claude"       # any command, in a split or a new tab
g = "spawn-tab:lazygit"
x = "none"                     # unbind a default

[keys.global]                  # no prefix needed
"alt+h" = "focus-left"

[[agents]]                     # teach it a new agent, or tune a built-in by name
name = "my-agent"
process = ["my-agent"]
working_patterns = ["esc to interrupt"]
blocked_patterns = ["\\(y/n\\)"]
```

## Never leave hydra

| Leader + | Panel | What it does |
|---|---|---|
| `f` | **Files** | *Recent*: files that just appeared in Downloads, Desktop, Documents or the project (docs, zips, screenshots). *Project*: fuzzy search over the project's files. Enter types the path into your prompt; `^O` opens it, `^F` shows it in the file manager, `^Y` copies the path. |
| `t` | **Tasks** | Every worktree task by stage: needs you, ready for review, working, no changes yet. Enter opens **review**: changed files and the diff, then `c` commit, `m` merge into the base branch, `p` push and open a pull request, `r` reply to the agent, `x` throw it away. `n` starts a new task. |
| `i` | **Inbox** | GitHub pull requests (checks, review state, conflicts) and issues via the `gh` CLI, and your Linear tickets when `LINEAR_API_KEY` is set. Enter opens it in the browser; `^T` turns it into a task (a PR is checked out with the agent briefed to fix its checks). |
| `T` | **Toolbox** | What Claude Code, Codex, Cursor and Gemini are set up with for this project: MCP servers, skills, plugins, sub-agents, hooks and instruction files, with where each comes from. Secret values are never shown. Enter opens the config file. |

Typing in a panel filters it; the action keys that would collide with typing use Ctrl.

## Quick prompt

`<prefix> q` opens a box: type the task and press Enter. `Tab` picks the agent (from
`[quick] agents`), `Shift+Tab` picks where it goes:

- **split right / down / new tab**: the agent starts there with your task
- **new worktree**: a branch named after the task (`q/fix-login-bug-3f2a`), its own checkout,
  and the agent working in it
- **this pane's agent**: the text is typed into the agent you're looking at, then Enter

`Alt+Enter` adds a line break. Agents are just commands, so anything works:

```toml
[quick]
place = "worktree"
agents = [
  { name = "claude", command = "claude {prompt}" },
  { name = "codex", command = "codex {prompt}" },
  { name = "opus", command = "claude --model opus {prompt}" },
]
```

## Restarts

The session (workspaces, tabs, splits, each pane's directory and command, agent sessions) is
saved to the data directory a couple of seconds after it changes. On the next start:

- Panes reopen in their last known directory. For PowerShell, that requires your prompt to report it
  via OSC 7 or OSC 9;9, as oh-my-posh and starship can. Other shells are tracked automatically.
- Agents resume. With hooks installed, hydra knows the exact session (`claude --resume <id>`).
  Otherwise it uses the agent's "most recent" form (`claude --continue`, `codex resume --last`).
- Panes started with a command (`spawn-right:lazygit`, `hydra split -- x`) run it again.

`hydra kill-server` keeps the session for next time, and `hydra kill-server --forget` discards it.
Closing every pane yourself also starts fresh. A burst of panes dying at once, as at logoff or a
crash, is not recorded, so the last good layout survives. Configure all of this under `[restore]`,
and per agent with `resume` / `resume_last`.

## Earlier layouts

`ui.layout` also accepts `workspaces` (panes and groups with tabs), `tree`, `dock` and `sidebar`.

## Worktrees

```sh
hydra worktree feat/login -- claude      # new branch from HEAD, opened with claude running
hydra worktree fix-123 --base origin/main
hydra worktree-remove [--force]          # closes the workspace and removes the checkout
```

New worktrees go to `{repo_parent}/{repo}-worktrees/{branch}` (set `[worktree] dir`). An existing
local or `origin/` branch is checked out instead of created. Set `[worktree] command = "claude"`
to start an agent in every new worktree.

## Copy mode

| key | |
|---|---|
| `h j k l`, arrows, `w b`, `0 ^ $`, `g G`, `H M L` | move |
| `Ctrl-u/d`, `Ctrl-b/f`, `PgUp/PgDn`, wheel | scroll |
| `v` / `V` | select characters / lines |
| `y`, `Enter` | copy selection (or the current line) and leave |
| `/` `?` then `n` `N` | search forward / backward (smart case) |
| `q`, `Esc` | leave |

Copies go to the system clipboard and, through OSC 52, to your outer terminal (works over SSH).

## Agent status

Each pane's process tree is scanned about once a second. Detected agents get a status from
these sources, in order:

1. **Hooks**, which are exact. `hydra integrate claude` adds hooks to `~/.claude/settings.json`.
   They stay inert outside hydra panes and are tagged so a re-run or `--uninstall` replaces them cleanly.
   Any tool can report its own state with `hydra hook <name> --status working|blocked|done|idle`.
   For Codex, `hydra integrate codex` prints the `notify` line to add.
2. **Screen patterns**: regexes matched against the bottom of the screen, such as "esc to interrupt".
3. **Activity**: recent output that isn't the echo of your own typing.

A turn that finishes while you're looking elsewhere is **done** until you focus that pane.

## Script it

```sh
hydra ls [--json]                      # tree of workspaces / tabs / panes with agent status
hydra split [--down] [-p ID] -- codex  # inside a pane, targets that pane by default
hydra send -p 3 "run the tests"        # types text and presses Enter
hydra read -p 3                        # the pane's screen as text
hydra new ~/code/web -- claude
hydra worktree feat/x -- claude        # worktree workspace for this pane's repo
hydra focus 3 | close 3 | kill-server [--forget]
```

`HYDRA_SOCKET=name` runs a separate server, like `tmux -L`.

## How it works

```
hydra (TUI client) ──┐      named pipe (Windows) / unix socket
hydra ls/send/hook ──┼──►   hydra daemon
                      │        ├─ workspace / tab / split-tree model (source of truth)
                      │        ├─ one PTY per pane (portable-pty → ConPTY / openpty)
                      │        │    reader thread → vt100 parser + replay ring
                      │        ├─ answers terminal queries (DSR/DA) itself, even with nobody attached
                      │        ├─ process-tree scanner (sysinfo) → agent detection
                      │        └─ status machine: hooks > screen patterns > activity
```

- `src/daemon/`: event-loop actor; PTYs (`term.rs`); process scan (`scan.rs`); session file
  (`persist.rs`); git status and worktrees (`git.rs`)
- `src/client/`: event loop, keys, actions (`mod.rs`); drawing (`render.rs`); copy mode (`copy.rs`)
- `src/protocol.rs`: length-delimited MessagePack messages
- `src/layout.rs`: split tree, rects, neighbour search
- `src/config.rs`, `src/keys.rs`, `src/theme.rs`: everything user-facing

## Roadmap ideas

- Restore scrollback contents after a restart, not just the layout
- Worktree setup hooks (copy `.env`, install deps) and PR status per branch
- Mouse passthrough for TUI apps that request it
- Extensions and workspace templates (fut's extension model)
- Remote: `hydra --remote ssh://host`
