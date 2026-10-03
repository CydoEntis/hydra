# Hydra roadmap: what nebula and fut have that Hydra didn't

Ticked when built, tested and pushed (dev branch).

## Batch A: working with agents, everyday feel
- [x] Quick follow-ups: Space on an agent row opens a small box; Enter sends; the view never moves; ↓ Space type Enter for the next one. Shift+Enter for a new line (sent as one paste).
- [x] "Done, not seen": finished agents stay marked until you look (cursor rests on it or you open it); counted on project rows.
- [x] Smarter status: no "done" while subagents still run (with a drain timeout); Esc-cancel noticed (OSC 9;4 progress); late duplicate permission prompts ignored; status reports only trusted from the pane's own process tree.
- [x] Speed: output in panes you can't see doesn't redraw the screen; measure and keep frame times low.
- [x] Find file (fuzzy) and search code (git grep) popups that open at the line or put the path in the agent's prompt.
- [x] Agents move themselves into a worktree: `hydra worktree --move <name>` from inside a pane; when the turn ends the agent restarts, resumed, in the new worktree.

## Batch B: launching and steering
- [x] Task box: one box for the task with pickers for project, worktree (or new), agent, model/effort; keeps a draft; Duplicate from an existing agent.
- [x] Agent presets: saved launches (agent, model, effort, prompt prefix/suffix, ask-for-task or not), e.g. "commit and push" in one key; usable from tickets and PRs.
- [x] Prompt and wait: `hydra send --wait` (and the MCP `hydra_send` wait / `hydra_wait`) waits for a real new turn to finish; `hydra wait --regex`.
- [x] Each agent row shows its last prompt and the live model; auto-title on the first prompt; names sync with Claude's /rename.

## Batch C: code and environment
- [x] Branch switcher for the repo folder (fuzzy, local and remote) that handles uncommitted changes (stash / bring along / commit / discard).
- [x] Review marks in Changes: mark a file reviewed; it sinks; marks clear when the file changes.
- [ ] Dev server per worktree (`.hydra.toml` run command): Run / Stop / Restart, ready marker, logs; worktree create/delete hooks (e.g. pick a port).
- [ ] Prewarm: a booted agent waiting so + New starts instantly.

## Batch D: reach and extras
- [ ] Remote machines over SSH (UI here, agents there).
- [ ] Extensions: a manifest plus commands; palette entries, sidebar labels, lifecycle hooks.
- [ ] Real tabs and any number of splits in the hydra layout.
- [ ] Memory view: RAM per agent.
- [ ] Extras: notification history, `hydra doctor`, more agent types (Cursor, OpenCode, Grok, …), BEL tracking.
