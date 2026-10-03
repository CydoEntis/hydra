# Hydra — design brief v3

Thanks for the last handoff — it's built. Hydra is now in daily use and we're adding a round of
features. Please design how they fit, so the app still feels like one calm thing and not a pile of
panels. Same format as last time works perfectly: an interactive cell-grid HTML prototype at
160×45 (plus how it degrades at 100×30 and grows at 240×60), with the state machine as the spec.

## What Hydra is (one line)

A terminal app where one person runs many AI coding agents (Claude Code, Codex, …) side by side,
each in its own git worktree, and only looks up when one needs them.

## What's built today (keep it, refine if you like)

- **Top bar:** `>_ hydra` · crumb `▌project › ⎇ branch › agent ● state · task`.
- **Sidebar** (drag to resize, 24–60 cols), one tree:
  ```
  + New n   Jump ●1 j
  ▾ ▌shop-api                 ●1 ⠹1
     BRANCHES
     ⎇ main
       ● claude  needs you        3m
         Run npm test -- checkout?
       › shell   shop-api
     WORKTREES
     ⑂ rate-limit  #412 ✓
       ⠹ codex   working          2m
         Rate limit /login
  + open a project  o
  ─────────
  Settings ,
  ```
- **Main area:** the focused session full size, or two split (draggable divider). Scrollbar in the
  pane margin; "↑ 30 lines up" marker; answer bar on an agent that needs you (its numbered
  choices as buttons).
- **Overlays (centered, screen dimmed):** + New (run · project · where · open), Jump, Open a
  project (path finder), Talk (message an agent), Settings (tabs), Keys, Ideas, Tickets
  (GitHub / Linear / Plane), Race, Ship confirm, right-click menus.
- **Views that replace the main area:** Files, Changes, Pull request, Map.
- **Splash** with the braille hydra.
- Tokens, glyphs and themes are unchanged from your handoff (acc lime, needs amber, ok green; ●
  ⠹ ✓ ○ ☾ for needs / working / done / idle / asleep).

## New things to design

For each: where it lives, what's on it, its states (empty, loading, error, many items), keyboard
and mouse, and how it looks in the narrow (100×30) layout.

1. **Quick follow-up** — the fastest way to nudge an agent without leaving what you're looking at.
   Space on any agent row → a small input box *near that row* (not a big centered modal?), Enter
   sends, the box closes, the cursor stays so `↓ Space type Enter` handles the next agent. Should
   show the agent's last line or question for context. Multi-line with Shift+Enter.
2. **"Done, not seen"** — finished agents you haven't looked at yet vs ones you have. Needs a
   distinct look in rows, project rows (counts) and Jump, without adding a new colour if possible.
3. **Find file / search code** — fuzzy file finder and a live `git grep` with results grouped by
   file, preview of the matching lines, actions: put the path in the agent's prompt, open in
   editor at the line. One overlay with two tabs, or two? Your call.
4. **Task box (replaces the + New chooser?)** — type the task first; pickers above the text for
   project · worktree (or new worktree) · agent · model/effort; a new-worktree state that clearly
   names the branch it'll create; keeps a draft when closed; "duplicate" prefilled from another
   agent. Should feel like the most-used thing in the app.
5. **Agent presets** — saved launches ("commit and push", "review this PR", "write tests"), each
   with agent, model, effort and a prompt template. A list to pick from, and a small editor.
6. **Agent row extras** — the agent's last prompt and its live model (e.g. `opus · high`) on the
   row, without making rows taller than two lines. Auto-titles.
7. **Branch switcher** — fuzzy local + remote branches, newest first; when there are uncommitted
   changes, a clear choice: stash / bring them along / commit first / discard.
8. **Review marks in Changes** — mark a file reviewed (it sinks to the bottom, dimmed); progress
   like "4 of 11 reviewed".
9. **Dev server per worktree** — a run command per project; status on the worktree row
   (launching / running / crashed), Run / Stop / Restart, and its logs (a view or a split?).
10. **Memory view** — RAM per agent and in total; maybe a small live readout in the status line.
11. **Notification history** — the last N toasts and alerts, clickable to jump to the agent.
12. **Tabs and more splits** — a session can now hold several tabs and more than two splits. Where
    do tabs show (top of the pane?) without bringing back the clutter we removed?
13. **Remote machines** — agents running on another computer over SSH. How do they appear in the
    sidebar (a machine level above projects?), and how is "this is remote" shown?
14. **Extensions** — small labels/buttons an extension can put on a worktree row or the status line,
    and their commands in the palette. Just the slots and rules, not specific extensions.
15. **Command palette** (Space) — it exists but in the old style; please restyle to match.

## Things that bug us today (please fix in passing)

- It can still feel dense. More air where it helps, but rows must stay scannable at 30+ agents.
- Two ways to message an agent (Talk popup vs follow-up box) — merge them if you can.
- The status line is underused.
- First-run / empty states (no projects yet, nothing running).

## Constraints

- A terminal grid: one character per cell, 24-bit colour, bold/italic/underline/dim only.
- Everything works by keyboard (leader key `Ctrl+Space` + a letter, or bare letters when the sidebar
  has focus) **and** by mouse (every chip clickable, hover states).
- Windows Terminal is the main target; also Ghostty / Alacritty / kitty / iTerm2.
- Keep the five themes working (Default, PaperColor Dark, Tango Dark, Monokai, Tokyo Night).
