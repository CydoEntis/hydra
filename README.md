<h1 align="center">seshi</h1>

<p align="center"><strong>Every session, one calm place.</strong> Run lots of coding agents side by side, see which ones need you, and keep them going when you leave.</p>

<pre align="center">
███████ ███████ ███████ ██   ██ ██
██      ██      ██      ██   ██ ██
███████ █████   ███████ ███████ ██
     ██ ██           ██ ██   ██ ██
███████ ███████ ███████ ██   ██ ██
</pre>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#use">Use</a> ·
  <a href="#configure">Configure</a> ·
  <a href="#script-it">Script it</a>
</p>

<p align="center"><img src="docs/media/demo.gif" alt="seshi: jumping to the agent that needs you, splitting a pane, keys and settings" width="900"></p>

<table>
  <tr>
    <td><img src="docs/media/02-main.png" alt="An agent asking a question, with one-key answers"></td>
    <td><img src="docs/media/03-go-to.png" alt="The inbox: what needs you, and any session by typing"></td>
  </tr>
  <tr>
    <td align="center">An agent needs you: answer with one key</td>
    <td align="center">Go to any session by typing</td>
  </tr>
  <tr>
    <td><img src="docs/media/05-split.png" alt="An agent and a shell side by side"></td>
    <td><img src="docs/media/06-keys.png" alt="Every key on one screen"></td>
  </tr>
  <tr>
    <td align="center">Split panes</td>
    <td align="center">Every key on one screen</td>
  </tr>
</table>

Run Claude Code, Codex, Gemini and friends side by side, see which ones need you, jump to them,
and keep them running when you close the window. Native on Windows, macOS and Linux (Omarchy
included). Inspired by [herdr](https://github.com/ogulcancelik/herdr),
[nebula](https://github.com/agentSystemLabs/nebula) and [fut](https://github.com/mikker/fut).

- **Agents keep running after you close the UI.** A background daemon owns every pseudoterminal
  (ConPTY on Windows). `<leader> q` detaches, and running `seshi` again reattaches.
- **Survives restarts.** Sessions are saved as they change; after a reboot `seshi` rebuilds them
  in the right folders and resumes agents (`claude --resume <id>`, `codex resume --last`, ...).
  Each pane's recent output is kept on disk too, so its history is still there to scroll
  back through, above a line marking the restart.
- **One screen for everything.** Floating cards: a full-height sidebar of your projects and the
  sessions in each, tabs as pills above the panes, and every pane a rounded card with its name,
  state and ✕ in its border; the one you're in is lit, the others fade. The leader is
  `Ctrl+Space`: a sky-blue pill says it's armed, and a pause (or `?`) shows every key.
- **Find anything.** `<leader> j` is the inbox: what needs you first, then type to go to any
  project or session. `<leader> Space` is a command palette in plain words, `a actions`
  at the foot of the sidebar lists the common commands, and `<leader> ?` maps every key.
- **Status at a glance.** Working (yellow, its name shimmering), needs you (red), done (green until
  you look), idle. An alert or a sound tells you when one needs you or finishes out of view.
- **Usage and limits.** Each agent's card shows how full its context is and what its session
  has cost, and its sidebar row shows the context once it's past half. An agent stopped by a limit
  is told "continue" once the limit resets (Settings → Continue after a limit). Claude's numbers
  come from its status line: `seshi integrate claude` makes seshi's run first and then yours,
  which looks the same as before.
- **Detection without setup.** The process tree finds `claude`, `codex`, `gemini`, `opencode`,
  `cursor-agent`, `copilot`, `amp`, `qwen`, `aider` and others, and screen patterns tell
  working from blocked. Hooks (`seshi integrate claude`) make it exact.
- **Git without leaving.** New agents get their own worktree, including `claude` or `codex`
  typed into a shell in a repo's main folder (turn it off with *Own worktree per agent* in
  Settings; `--continue`/`--resume` stay put). Changes (`d`) shows the diff with
  review marks, then commit, merge, or open a pull request. Files (`f`), find (`F`), search the
  code (`/`), switch branch (`B`).
- **Everything is configurable.** The leader, every binding, commands bound to keys, themes and
  per-colour overrides, icons, the sidebar side, detection patterns and your own agents. Settings
  (`<leader> ,`) save to your config with its comments kept.
- **Scriptable.** `seshi split -- claude`, `seshi send`, `seshi read` and `seshi ls --json`
  work from any shell, including from an agent running inside seshi.

## Install

**macOS, Linux, Omarchy / Arch:**

```sh
curl -fsSL https://raw.githubusercontent.com/CydoEntis/seshi/main/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/CydoEntis/seshi/main/install.ps1 | iex
```

The installers pick the build for your machine, check its checksum, put it in
`~/.local/bin` (Windows: `%LOCALAPPDATA%\Programs\seshi`, added to your PATH) and tell you if
another `seshi` comes first. No admin rights needed. `SESHI_VERSION=v0.3.0` picks a version;
`SESHI_INSTALL_DIR` a folder. If scripts are blocked (a locked-down work PC), download the archive
for your machine from [Releases](https://github.com/CydoEntis/seshi/releases) and put `seshi` on your PATH.
To update later: Seshi checks each time you open it (and every few hours while it stays
open), and when a newer version is out a dot shows after `, settings` at the foot of the
sidebar, and Settings has an **Update now** button by the version (also "Update seshi" in the
command palette). Click it to see your version, the new one and what changed; confirm, and the new
version downloads and the window restarts into it, with your sessions still running. From a
shell it's `seshi update` (or `seshi update --check` to just
look). Turn the check off in Settings (Check for updates).

**Coming from hydra?** Seshi is its new name. Install seshi as above; the first time it starts,
it brings hydra's config, saved sessions and history over (hydra's own folders are left as they
were), and the next server start points your Claude hooks at seshi. Run `seshi integrate mcp` again
if agents used hydra's MCP tools, then uninstall hydra when you're happy.

Then run `seshi doctor` to check your setup. For the best look: a terminal with true colour and a
[Nerd Font](https://www.nerdfonts.com) (Windows Terminal, Ghostty, Alacritty, WezTerm, iTerm2).
Optional: `git` (worktrees, changes), `gh` (opening a pull request from Changes).

From source (any platform with Rust): `cargo install --git https://github.com/CydoEntis/seshi`.

## Use

```sh
seshi            # attach (starts the server if needed); opens the current dir
seshi ~/code/api # open or switch to that directory
```

Agents come first in the sidebar, then terminals, then sessions on other machines (SSH). They are
grouped by **where they are working now**: the git
repo they're in (a subfolder counts as its repo) or the folder itself outside git, and for SSH
the machine they're connected to. Nothing to open or set up: `cd` somewhere in a shell and it moves
to that group; start an agent in a shell (or ssh somewhere) and it moves section. An agent stays in
the group of the folder it started in while it runs, however many folders it works in. A pane
split beside a session stays with it.

```
╭──────────────────────────────╮
│                              │
│  ● shop-api              ● 1 │
│   ● claude      main · 3m    │
│     Allow running npm test?  │
│   ⠹ calm-heron          2m   │
│   › shell                    │
│                              │
│  ● web-shop                  │
│   ✓ quick-fox          40s   │
│                              │
│  ──────────────────────────  │
│  a actions        , settings │
╰──────────────────────────────╯
```

- A project is its dot (in its colour) and name; `● 1` on the right when something there needs
  you, `no git` outside a repo.
- Under it, its sessions: state, name, `branch · age` on the right. A session that needs you has
  its question under it. Every new claude / codex gets its own worktree (named for you), so agents
  never edit the same files.
- Things that need you sort to the top. Agents asleep (see Settings) show `☾ asleep`.
- Drag a group's name, or a session, up or down to reorder (what needs you still comes first);
  drop a session on another group (its name or one of its sessions) to move it there, within its
  section. Click to fold or open. With the sidebar focused
  (`Ctrl+Space e`), a row's menu letters work directly (the sidebar's foot lists them): `x` (or
  Delete) closes, `r` renames. Settings says which seshi you're on.

The leader key is `Ctrl+Space`. Press it and a sky-blue pill appears at the end of the tab row
(the pane you're in turns sky too); press a key, or wait a moment (or press `?`) for the key map.
In the key map a key runs its command, Tab searches them all by name, and keys marked `›` open a
second step (`w` worktrees: new, switch, merge, delete; Backspace goes back).

| keys (after the leader) | action |
|---|---|
| `j` | **inbox**: what needs you (answer a question with its number, Enter goes there), then every session; type to find one |
| `n` / `p` | a shell right where you are (cd and run what you like; it groups itself) / a shell beside this one |
| `z` / `x` | zoom / close |
| arrows | focus the pane that way (left past the edge: the sidebar). `Alt`+arrows do it without the leader (Settings → General turns that off) |
| `v` / `-` / `H J K L` | split right / split down / resize |
| `t` / `r` / `]` `[` / `X` | **tabs**: new tab (a shell where you are) / rename it in its pill (or double-click it) / next, previous / close it. `Alt+1`–`9` (no leader) goes to a tab |
| `w` / `f` / `d` | worktrees › / files / changes |
| `e` / `Space` / `a` | focus the sidebar / command palette / actions |
| `F` / `/` / `B` | find a file / search the code / switch branch |
| `R` / `V` | rename a session / paste the clipboard's image |
| `y` / `{` `}` / `b` | select text with keys / previous, next command in the history / sidebar on or off |
| `,` / `?` / `N` / `q` | settings / key map / history / quit (agents keep running) |

In **Files**: Enter puts the path in the agent's prompt, `e` opens it in your editor (`editor`
in config; nvim, helix … open inside seshi), `y` copies the path. In **Changes**: `c` commit,
`p` push and open a PR, `e` editor, `x` mark the file reviewed (it sinks; the mark clears
if the file changes again), `r` goes to its agent.

Everything also works with the mouse. Hold Shift to select text with your terminal.

## Panes are real terminals

- **Scroll:** the wheel, or `PageUp` / `PageDown` at a prompt (full-screen programs keep those
  keys), or `Shift+PageUp` / `Shift+PageDown` anywhere. A scrollbar shows when there's history;
  click or drag it. Typing (or clicking for the program) goes back to the bottom.
- **Mouse:** programs that use it (vim, lazygit, htop, full-screen agents) get clicks, wheel and
  drags. Hold Shift to select text yourself.
- **Copy:** drag to select; double-click copies a word, path or link. Programs that copy (OSC 52,
  e.g. Claude's `/copy`, nvim) put it on your clipboard. Ctrl+click opens a link.
- **Images:** `Ctrl+Space V` (or `Ctrl+Space Ctrl+V`, or a paste while an image is on the
  clipboard) saves the image and pastes its path; Claude Code and Codex attach it.
- **Keys:** on Windows, combos plain terminals can't send (Ctrl+Shift+letter, Ctrl+Enter, Ctrl+Tab)
  reach programs exactly, as native key records. Shift+Enter is still a newline for agents.
- **Resize:** text re-wraps to the new width (history included). Programs' cursor shape (bar,
  block, underline) shows. Synchronized redraws are drawn whole, so agents don't flicker.
- **Closing** a pane ends everything running in it, instantly.

## Alerts

A desktop notification and a sound when an agent you're not looking at needs you or finishes,
also when no seshi window is open (the server sends it). Sounds: glass, ping, chime, pop, off,
or a path to your own file (`[notify] sound_needs`, `sound_done`). `seshi test-alert` tries them.

Click a notification to go to its session: seshi comes to the front on it (its split too).
That works for the note inside seshi and the desktop pop-ups on Windows and Linux (on
Omarchy / Hyprland the window is brought forward too); on macOS, install
`terminal-notifier` for clickable ones.

## Sleep

`sleep_after = "1h"` (Settings → Sessions) stops agents that have sat finished or idle that long,
to save memory. They keep their place; open one and it resumes its conversation
(`claude --resume`, `codex resume`).

## Merging

In Changes, `m` merges a worktree's branch into the main one, then closes its
sessions and removes the worktree and the branch. Closing the last session in a worktree
seshi made removes the folder; its branch goes too when it's already merged.

## Recipes

```toml
[[recipes]]
name = "feature"
worktree = true                           # its own worktree
run = ["claude", "npm run dev", "lazygit"] # the first is the main one
```

They show up in + New as `⚙ feature`.

## Agents that steer agents (MCP)

```sh
seshi integrate mcp     # registers `seshi mcp` with Claude Code (prints the Codex snippet too)
```

Any agent can then use seshi's tools: `seshi_list` (sessions, status, the question each is
asking), `seshi_read` (a screen), `seshi_send` (type a message), `seshi_answer` (a numbered
prompt), `seshi_start` (a new agent in its own worktree; your screen stays where it was) and
`seshi_interrupt`. There is no merge, push or delete.

You decide how far that goes (Settings → Agents, or `[mcp]`):

```toml
[mcp]
approve = "never"    # never | safe | always: may agents answer "yes" to other agents' prompts?
safe = ["npm test", "cargo test", "git status"]   # with "safe": only prompts that mention these
scope = "project"    # project: only sessions in the calling agent's repo | all
```

## Sync between machines

```sh
seshi sync setup          # first machine: makes a private GitHub repo seshi-config
seshi sync setup          # other machines: picks it up (your old config is kept as a backup)
seshi sync                # pull + push now (it also happens on its own)
```

Shared: `config.toml`. Anything for one machine only (a shell path, keys) goes in
`config.local.toml` next to it, which is never synced and wins over `config.toml`.

## Splash and settings

seshi opens on the splash: the seshi, what happened while you were away (`● 2 need you ⠹ 3 still
working ● 1 finished`) and buttons: Resume where you left off (`r`), New session (`n`).
Turn it off in Settings → General. Settings → General → Start folder sets where plain `seshi`
opens (e.g. `~/code`); empty means wherever you run it.

`Ctrl+Space ,` opens **Settings**, with tabs (Tab cycles): General · Sessions · Appearance · Agents ·
Projects · Keys. Values are chips you click (or ←→ / Enter). Appearance has how panes look and
the themes: **Seshi Night** (the default: warm driftwood dark, sunset peach) and **Seshi Day** (its
light twin), then Hydra, PaperColor Dark, Tango Dark, Monokai, Tokyo Night and more, with live
swatches; the theme recolours agent output's ANSI colours too. On Keys, Enter then press a new key to rebind a shortcut. Projects lists
the folders you opened (Enter forgets one). Everything is saved to `config.toml` (comments kept)
and applies at once.

## Command name clash

`seshi` is also the name of a Linux password-testing tool (THC-Seshi). If you have that installed,
give this one its own name; the app doesn't care what it's called:

```sh
alias hy='~/.cargo/bin/seshi'                          # bash / zsh
Set-Alias hy "$env:USERPROFILE\.cargo\bin\seshi.exe"   # PowerShell ($PROFILE)
```

## Configure

```sh
seshi config init   # writes the annotated example to the config path
seshi config path   # %APPDATA%\seshi\config.toml, or ~/.config/seshi/config.toml
```

See [`config.example.toml`](config.example.toml) for every option. Some highlights:

```toml
prefix = "ctrl+a"
theme = "tokyo-night"          # seshi-night (default), seshi-day, catppuccin-mocha/latte, gruvbox, nord, dracula, mono

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

## Never leave seshi

| Leader + | Panel | What it does |
|---|---|---|
| `f` | **Files** | *Recent*: files that just appeared in Downloads, Desktop, Documents or the project (docs, zips, screenshots). *Project*: fuzzy search over the project's files. Enter types the path into your prompt; `^O` opens it, `^F` shows it in the file manager, `^Y` copies the path. |
| `d` | **Changes** | A worktree's changed files and the diff: `c` commit, `m` merge into the base branch, `p` push and open a pull request, `r` go to its agent, `x` throw it away. |

Typing in a panel filters it; the action keys that would collide with typing use Ctrl.

## Agents

The new-agent dialog starts any agent in `[quick] agents`; `{prompt}` is the task, quoted for
your shell. Agents are just commands, so anything works:

```toml
[quick]
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
- Agents resume. With hooks installed, seshi knows the exact session (`claude --resume <id>`).
  Otherwise it uses the agent's "most recent" form (`claude --continue`, `codex resume --last`).
- Panes started with a command (`spawn-right:lazygit`, `seshi split -- x`) run it again.

`seshi kill-server` keeps the session for next time, and `seshi kill-server --forget` discards it.
Closing every pane yourself also starts fresh. A burst of panes dying at once, as at logoff or a
crash, is not recorded, so the last good layout survives. Configure all of this under `[restore]`,
and per agent with `resume` / `resume_last`.

## Worktrees

```sh
seshi worktree feat/login -- claude      # new branch from HEAD, opened with claude running
seshi worktree fix-123 --base origin/main
seshi worktree-remove [--force]          # closes the workspace and removes the checkout
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
| `Tab` / `Shift+Tab` | keep this selection and go to the next / previous pane of the split, the same search there; `y` then copies them all, each under its pane's name |
| `q`, `Esc` | leave |

Copies go to the system clipboard and, through OSC 52, to your outer terminal (works over SSH).

## Agent status

Each pane's process tree is scanned about once a second. Detected agents get a status from
these sources, in order:

1. **Hooks**, which are exact. `seshi integrate claude` adds hooks to `~/.claude/settings.json`.
   They stay inert outside seshi panes and are tagged so a re-run or `--uninstall` replaces them cleanly.
   Any tool can report its own state with `seshi hook <name> --status working|blocked|done|idle`.
   `seshi integrate gemini` and `seshi integrate qwen` do the same for Gemini CLI and Qwen Code;
   `seshi integrate opencode` writes a small opencode plugin that tells seshi what it's doing.
   For Codex, `seshi integrate codex` sets seshi as its `notify` (in `~/.codex/config.toml`;
   another program's notify is left alone).
2. **Screen patterns**: regexes matched against the bottom of the screen, such as "esc to interrupt".
3. **Activity**: recent output that isn't the echo of your own typing.

A turn that finishes while you're looking elsewhere is **done** until you focus that pane.

**An agent seshi doesn't know** (a newer CLI, or one you start with your own alias): right-click
its pane and pick **"… is an agent"**, or run `seshi teach` in it (`seshi teach 4` for pane 4).
Seshi looks at the program actually running, whatever alias started it (or, for a node /
python harness, its package or script), and adds it under `[[agents]]` in your config with the
usual screen signs of working. Edit that entry to tune them.

## Popups

`seshi popup -- fzf` (or `lazygit`, `htop`, any command) opens a floating pane over everything,
in the folder you're in; it has the keys until the command exits, then it's gone.

## Agents asking you

An agent (or any script in a pane) can ask you something with fixed answers and wait:

```sh
seshi ask-human "Deploy to staging?" -o Yes -o "Not yet"   # prints the answer you pick
```

The pane shows **needs you** (with a notification), the question is in the Inbox and on the
pane's answer bar, and a number or click answers. Without `-o` the answers are Yes / No.

## What agents may do

A command an agent runs in its pane (the `seshi` CLI, or `seshi mcp`) acts as that pane, and a
pane may by default **read** other panes, **write** (type) into them and **start** sessions. To
also **respond** (answer another agent's prompt or question) or **admin** (close other panes,
stop seshi), grant it: `seshi grant 4 read,write,start,respond` (`seshi grant 4 default` puts it
back), or change the default in `[mcp] grants`. Only you can grant: not from inside a pane. These
are guardrails for agents that behave, not a sandbox.

## Script it

```sh
seshi ls [--json]                      # tree of workspaces / tabs / panes with agent status
seshi split [--down] [-p ID] -- codex  # inside a pane, targets that pane by default
seshi send -p 3 "run the tests"        # types text and presses Enter
seshi read -p 3                        # the pane's screen as text
seshi new ~/code/web -- claude
seshi worktree feat/x -- claude        # worktree workspace for this pane's repo
seshi focus 3 | close 3 | kill-server [--forget]
seshi send -p 3 --wait "fix the bug"   # waits for the turn to end, prints the agent's reply
seshi wait -p 3 [--regex "passed"]     # the turn ending, or text on the screen (exit 2: timeout)
seshi worktree --move [name]           # run by an agent: move itself into a new worktree
seshi doctor                           # check everything seshi relies on
seshi --remote me@box                  # the UI here, agents on another machine (any command)
```

`SESHI_SOCKET=name` runs a separate server, like `tmux -L`.

## How it works

```
seshi (TUI client) ──┐      named pipe (Windows) / unix socket
seshi ls/send/hook ──┼──►   seshi daemon
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

## Per-repo settings: `.seshi.toml`

Commit one to the repo:

```toml
[hooks]
on_create = "npm install"        # in every new worktree
on_remove = ""
```

Hooks don't run until you allow them: run `seshi allow` in the repo (it shows the
commands). If they change, they wait for `seshi allow` again, so a cloned repo can't run
code on its own. A hook is stopped after 10 minutes.

## Remote

`seshi --remote me@box` runs the UI here and everything else there, over ssh (seshi must be
installed on both; `SESHI_REMOTE_CMD` if it isn't on the far side's PATH, `SESHI_SSH="ssh -p
2222"` for options). Panes, agents, worktrees and statuses work; views that read files (Files,
Changes, find, branches) don't yet.

## Roadmap ideas

- Restore scrollback contents after a restart, not just the layout
- Files, Changes and find over `--remote`
- Workspace templates
