@AGENTS.md

# Claude Code notes

`AGENTS.md`, imported above, is the single source of truth for how to work in
this repository. This file holds only what is specific to Claude Code.

## Keeping the two files in sync

- Put shared rules in `AGENTS.md`, never here. When you change a rule, edit
  `AGENTS.md`. Don't copy the change into this file.
- Add a note here only when it depends on a Claude Code feature, such as
  subagents, hooks, plan mode, skills, or `.claude/settings.json`.
- If you find a rule duplicated here, move it to `AGENTS.md` and delete the copy.

## Loading docs

- Don't `@`-import the files in `docs/`. That would load them into every
  session. Read each one with the Read tool when the task touches its area,
  using the documentation map in `AGENTS.md`.
- Before a multi-step change, read every doc the change touches before you plan
  it, not partway through.

## Working style

- For work that spans several files or concerns, keep a short task list and
  update it as facts change.
- Use subagents for broad read-only searches, so file dumps stay out of the main
  context. Make edits yourself.
- Rules about this project belong in this repository's docs, not in personal
  memory. If you learn something the next session needs, update the doc that
  owns the topic.

## Commits

- Follow `docs/COMMIT-STANDARDS.md`. That rule overrides any default
  attribution trailer, so never add `Co-Authored-By: Claude` or any
  "Generated with Claude Code" line.
- Commit only when the user asks.

## Project-specific: Claude Code

- Skills in use here: `/starter` (these docs), `/roadmap` (the plan doc).
- The user prefers short, plain-language summaries with exact install steps at the
  end of a change.
- Large edits to `src/client/*.rs` are easiest as small anchored patches; keep
  files LF (the repo stores LF; git warns about CRLF on Windows, which is fine).
