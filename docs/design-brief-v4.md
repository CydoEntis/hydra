# Seshi — design brief v4

Thanks for the floating handoff: it's built and shipped (0.15.0). Seshi has been through a week
of daily use and a big trim, and now a few features come back and five new ones arrive. Please
design how they fit the floating look, so it still feels like one calm thing. Same format as last
time: an interactive cell-grid HTML prototype at 160×45 (plus how it degrades at 100×30 and grows
at 240×60), with the state machine as the spec.

## What Seshi is (one line)

A terminal app where one person runs many AI coding agents (Claude Code, Codex, …) side by side,
each in its own git worktree, and only looks up when one needs them.

## How it's used now (this changed since v3)

You open a shell, `cd`, and run what you like. Sessions group themselves in the sidebar by where
they work; typing `claude` or `codex` in a repo's main folder moves it into its own worktree.
There are no projects to open and no "new agent" chooser in the way.

## What's built today (keep it, refine if you like)

- **Floating cards** with gaps, rounded corners, title and state in the border; unfocused cards
  fade. A tiled style draws the same layout without gaps (Settings → Appearance).
- **Sidebar card**, sections with counts and a rule (`AGENTS 2 ────`, `TERMINALS 1`, `SSH 1`),
  folders with fold carets, one row per session: state glyph, name, `branch · age` on the right,
  the question it asks under it, repeated subagents counted (`↳ workflow-subagent ×11`). One
  highlight at a time. Foot: `a actions` and a settings cog.
- **Tab row**: pills with the tab's number or name, `+`, and the armed-leader pill (`⌨ CTRL+SPACE`,
  sky blue) when the leader is pressed.
- **Inbox** (`j`): NEEDS YOU (question and numbered answers on the row; a number answers), JUST
  FINISHED, then type to find any session.
- **Key map** (`?`, or a pause after the leader): groups AGENTS / PANES / TABS / PROJECT / SESHI,
  searchable, `w` opens a second step (worktrees: new, switch, merge, delete).
- **Actions list** (`a`): new shell here, new pane beside, new tab, inbox, hide sidebar, all keys.
- **Tool windows** over everything: Files, Changes (diff, review marks, commit, merge, open a PR),
  Find, Branches. Settings, the new-agent dialog (task, agent, model, worktree, presets and recipes
  as choices), right-click menus, toasts, notification history.
- **Splash**: SESHI wordmark, "while you were away", Resume / New shell here.
- Keys: ADR 0009 (`docs/adr/0009-leader-keys-after-the-cut.md`). Themes: Seshi Night (default),
  Seshi Day, and ten more; the contrast audit must keep passing.

## Coming back (designed before, cut, now wanted in the floating style)

For each: where it lives, what's on it, its states (empty, loading, error, many items), keyboard
and mouse, and how it looks at 100×30. Several used to be big centred panels; smaller and closer
to where you're looking is better if it works.

1. **Quick follow-up**: send an agent its next prompt without switching to it. From its sidebar
   row or its Inbox row; shows its last line or question for context; Enter sends and you stay
   where you were, so you can do the next one; Shift+Enter is a new line. This replaces the old
   Talk / Reply / Answer: one way to message an agent, not three.
2. **Tickets** from GitHub, Linear and Plane: your issues per project, one tab per source, search,
   and Enter starts an agent on one in its own worktree (the ticket as its prompt). Linear and
   Plane need an API key; show the "not set up" state.
3. **Queue**: work waiting for an agent. Add a ticket or a typed task; the server starts so many
   at a time (a setting, default 3), each in its own worktree, the next when one finishes, also
   with the window closed. States: waiting, starting, running, ready for review, failed. Where it
   lives: its own view, a tab of Tickets (it was), or part of the Inbox?
4. **Pull requests and Ship**: a branch's PR (checks, review comments, the diff), and one key to
   hand failing checks and comments to the branch's agent. A small tag on worktree rows
   (`#412 ✓`, `#412 ✕`). Ship: commit, push and open or update the PR in one step, with a confirm
   that says exactly what will happen.
5. **Dev server per worktree**: a run command per repo (`.seshi.toml`), its own port per worktree
   (3000, 3001, …). On the row: starting / ready :3001 / crashed; Run, Stop, Restart; its output
   somewhere you can look (its own tab today); open it in the browser.
6. **Checkpoints** (off by default): the folder's state after each agent turn; a list to roll a
   folder back to any of them, saying what changes and saving what's there first.
7. **SSH remotes**: the window here, the agents on another machine (`seshi --remote host`). How
   "this is remote" shows (the sidebar card's title today), and what doesn't work over it yet.
8. **Config sync**: the same settings on every machine through a private GitHub repo. Mostly a
   setup flow (in Settings?) and a quiet "synced" state; machine-only settings stay local.

## New

9. **Two agents touching the same files**: agents in different worktrees change the same file on
   different branches, which becomes a merge conflict later. Show it early: on the rows involved,
   in the Inbox, or both? What it says (`orders and rate-limit both changed checkout.ts`), how
   loud it is, how you dismiss it, and where you go from it (the two diffs?).
10. **Review and merge from the Inbox**: a JUST FINISHED row shows what the agent changed
    (`+42 −7 · 3 files`) and its last words; from the row, see the diff, merge (worktree and branch
    cleaned up), throw it away, or open its dev server. The loop from "done" to "merged" without
    leaving the list.
11. **Why this status**: where a session's state came from ("hook: finished 2m ago", "screen:
    matched `esc to interrupt`", "process: claude running"). On hover, on a key, in the row's
    menu? Calm normally, easy to find when a dot looks wrong.
12. **Today, on the splash**: what each agent got done since this morning (or since you were last
    here): tasks finished, time working, what it cost. A glance, not a report.
13. **Alerts on your phone** (off by default): through ntfy (a free app, no account; you pick a
    private topic name). A setup flow with a test button, and a clear choice between "claude in
    AeVox needs you" and also sending the question's text (the topic name is the only secret on
    the public server).

## Things that bug us today (please fix in passing)

- Where do views that used to be big panels go now, so the screen doesn't fill with cards?
- The status bar at the bottom of tool windows; keep the round ends everywhere.
- First-run and empty states for each thing above.

## Constraints

- A terminal grid: one character per cell, 24-bit colour, bold/italic/underline/dim only. Nerd
  Font glyphs are used when the font has them (pill ends, the cog), with plain fallbacks.
- Everything works by keyboard (leader `Ctrl+Space` + a key; bare keys when the sidebar has focus;
  Alt+arrows move between panes) **and** by mouse (every chip clickable, hover states).
- Free keys for new things: the cut features' keys stayed free (`T m 1–3 o g l . i I Q u O A C U P S`);
  ADR 0009 lists the rest.
- Windows Terminal, Ghostty, Alacritty, kitty, iTerm2. Windows, macOS and Linux.
- Every theme keeps working, and the contrast audit keeps passing.
