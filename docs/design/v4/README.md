# Handoff: Seshi v4: features in the floating look

## Overview
This is the v4 brief, designed into the shipped floating look (0.15.0). It covers the eight returning features (quick follow-up, tickets, queue, PRs and Ship, dev servers, checkpoints, SSH remotes, config sync) and the five new ones (same-file heads-up, review and merge from the Inbox, why-this-status, Today on the splash, phone alerts). Each comes with its empty, loading, error and many-items states, at 100×30, 160×45 and 240×60.

## About the design files
These are **HTML design references**: an exact cell grid (one character per cell, 24-bit fg/bg, bold/italic/underline). They are not production code. Rebuild them in Seshi's own TUI stack.

**`seshi-app.js` is the spec.** It contains:
- `initialState()`: the data model
- `act(state, action)`: the state machine (keys, clicks and timed effects)
- `render(state)`: draws every surface with integer cell coordinates
- `PRESETS`: the 33 named states

Open `Seshi Prototype.dc.html` in a browser, with all files in one folder:
- Click the grid, then use the keyboard. Ctrl+Space works, or use the button.
- **Size** switches between 100×30, 160×45 and 240×60.
- **Jump to** opens any preset.
- Two toggles compare the open design options (below).

`Seshi Options.dc.html` shows those options side by side.

## Fidelity
**High fidelity.** Glyphs, copy, spacing and colour roles are final. Colours are the Seshi Night roles; map them onto every theme through the same role names.

---

## The big decision: where views live
1. **The sheet:** one card docked right of the panes. It holds the Inbox (`j`), Changes (`d`), Pull request (`P`), Dev server output (`l`), and also Tickets/Queue (`T`/`Q`) and Checkpoints (`C`) unless modal mode is on.
   - It is 72 columns wide at 160 cols and 96 at 240+. Below 140 cols it covers the whole pane column.
   - The panes **reflow** into the remaining width; nothing stacks.
   - Only one sheet is open at a time; `Esc` closes it.
   - Its bottom row is a full-width pill status bar of key hints. A right-aligned note is dropped when it doesn't fit.
2. **Popovers beside the row:** quick follow-up (`m`) and why-this-status (`i`).
   - Anchored to the selected sidebar row (`◂` points at it), just right of the sidebar.
   - The screen is not dimmed.
3. **Centred, dimmed modals:** only for deliberate steps: Ship (`S`), Roll back, Settings (`,`) and the key map (`?`).

### Open options (pick one per row; both are wired into the prototype)
- **`modalTools`:** Tickets, Queue and Checkpoints as centred modals sized to their content, instead of the sheet. Pick-and-go tasks get full width and focus. Inbox, Changes, PR and Server always stay docked.
- **`soft`:** every card gets a filled pill title (lime when focused, `btn` otherwise), the gap grows from 2 to 3 columns, and pane and sheet padding grows from 3 to 4 columns. Corners stay one cell (`╭`); no fake bigger radius.

## Features (states are named presets in seshi-app.js)
1. **Quick follow-up `m`**: replaces Talk, Reply and Answer.
   - **Layout:** a popover from the sidebar row, or inline inside an Inbox row. It shows the question (amber) or the last line, quoted and dim, then a **text area**.
   - **Text area:** a rounded `card2` box, 3 rows to start, growing to 10 (popover) or 6 (Inbox). It wraps on words and keeps blank lines. Past the limit it scrolls, with `↑ n more` in the top border; a word count sits top right.
   - **Keys:** Enter sends and closes; you stay where you were and the agent turns to working. Shift+Enter adds a new line; Esc cancels.
2. **Tickets `T`**: tabs GitHub · Linear · Plane · Queue (Tab cycles).
   - **List:** a search pill and the issues for this repo. Taken issues show `⎇ worktree` or `queued`.
   - **Keys:** Enter starts claude in a new worktree with the ticket as its prompt; `q` adds it to the queue; `o` opens it.
   - **States:** loading (spinner); Linear not set up (masked key input + Save); Plane 401 error (Retry / New key); no matches.
3. **Queue `Q`**: lives in Tickets as a tab, with a one-line summary at the foot of the Inbox.
   - The intro line says it runs 3 at a time (a Settings › Queue option).
   - **States:** running (spinner + worktree), starting `◌`, waiting `·` (with `#n next`), ready for review `✓` (also in the Inbox), failed `✕` (with its reason).
   - **Keys:** `a` adds a typed task (inline input), `r` retries, `x` removes, Enter opens.
4. **Pull request `P` and Ship `S`**:
   - **Row tag:** `#412 ✕` (red) or `#194 ✓` (green) on the sidebar row and the pane footer.
   - **PR sheet:** checks (2 of 4 failing), review comments with `file:line`, and `F` to hand the failures and comments to the agent as its next prompt.
   - **PR states:** loading; no PR yet (offers Ship); `gh` not signed in.
   - **Ship:** a modal listing the exact three steps (commit with message, push with commit count, open or update the PR), then ticks them off as they run, then a toast. Also `e` to edit the message.
5. **Dev server**:
   - **Row tag:** `▶ :3001`, a spinner while starting, `✕ :3000 crashed`, or `■` stopped.
   - **Keys:** `u` run/stop, `U` restart, `O` open in browser, `l` output sheet.
   - **States:** crashed (error log + `m` to ask the agent to fix it); no run command (an input saved to `.seshi.toml`; port per worktree via `$PORT`).
6. **Checkpoints `C`** (off by default):
   - **Off:** an explanation of what it does, plus a Turn on button.
   - **On:** a list of turns (time, turn, summary, change size); the current one is marked `●`.
   - **Roll back:** a confirm modal that lists the files that change or are removed, and says the current state is saved first.
7. **SSH remote**:
   - The sidebar card title becomes `⇄ build-box remote` (teal).
   - A dim note lists what doesn't work yet; Checkpoints shows a "not over SSH yet" state, and `O` explains the missing port forward.
8. **Config sync** (Settings › Sync): repo name (private, created if missing) → uses the `gh` login → Connect → spinner → `✓ Synced 2 minutes ago · 3 machines`.
   - It lists what's synced and what stays on this machine.
   - **Error:** the repo is public. The settings status bar shows `✓ synced` / `not synced`.
9. **Same-file heads-up** `⇆`:
   - **Where:** a quiet `⇆ checkout.ts` tag (amber mixed with text, not full amber) on both rows, and a HEADS UP section in the Inbox. It is not counted as needs-you.
   - **Keys:** `d` both diffs (a sheet with each branch's hunk), `m` tell one agent (pre-filled follow-up), `k` dismiss (it comes back if they touch the file again).
10. **Review and merge from the Inbox**: a JUST FINISHED row shows the change size, the agent's last words in quotes, and `d` diff · `M` merge · `x` throw away · `O :port`.
    - Merge and throw-away confirm inside the row (`[Merge Enter] [Cancel Esc]`), then a spinner, then a toast; the worktree and branch are removed.
11. **Why this status `i`**: a popover with three sources (hook, screen, process), each with its detail and age. The source that won is bold, marked `◂`, and the rule is spelled out ("most specific wins").
12. **Today** (splash): a TODAY card per agent (tasks, agent time, a small bar, cost) and a total line, under the wordmark and "while you were away".
13. **Phone alerts** (Settings › Alerts, off by default): on/off, a random topic (`n` makes a new one), and the warning that the topic is the only secret.
    - A radio choice: the name only (`claude in AeVox needs you`), or also the question text, with a privacy note.
    - **Send a test** (`t`): sending → `✓ Sent. Check your phone.` / an error.

**Inbox `j`** sections: search (type letters to find any session) → NEEDS YOU (answer pills `[Yes 1] [Always 2] [No 3]` + `m`) → HEADS UP → JUST FINISHED → QUEUE summary. It also has an empty state.

**First run:** an empty AGENTS section explains "Run claude or codex in any repo…", and the pane shows a centred welcome with `cd`, `claude`/`codex` and `Ctrl+Space ?`.

## Keys added (from the free list)
| Key | Action |
|---|---|
| `m` | follow-up |
| `i` | why this status |
| `T` | tickets |
| `Q` | queue |
| `P` | pull request |
| `S` | ship |
| `C` | checkpoints |
| `u` | run/stop dev server |
| `U` | restart dev server |
| `O` | open in browser |
| `l` | server output |
| `1–3` | answer |

Contextual keys inside the sheet: `d M x k F a r q`.

The key map (`Ctrl+Space ?`) groups them: AGENTS / WORK / SERVER / PANES / PROJECT / SESHI.

## Colour roles
- lime `acc`: focus and primary
- amber `needs`: needs you
- amber mixed with text: the same-file heads-up
- red `err`: failed, crashed, destructive
- green `ok`: done, passing, ready
- sky: leader mode only
- teal `ws.teal`: remote

Run every new role through the contrast audit in all 12 themes; only Seshi Night was checked here.

## Not covered
- Files, Find, Branches, the new-agent dialog and notification history keep their 0.15 design.
- Light theme (Seshi Day) not checked.

## Files
- `seshi-app.js`: the spec (state, actions, render, presets)
- `Seshi Prototype.dc.html`: the interactive viewer
- `Seshi Options.dc.html`: the sheet/modal and line/soft comparisons
- `seshi-grid.js`: the cell-grid renderer
- `seshi-tokens.js`: the Seshi Night colour roles
- `support.js`: runtime for the HTML viewer only
