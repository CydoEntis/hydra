# 0006. Leader keys follow the floating design's key map

- **Status:** Superseded by 0009
- **Date:** 2026-10-09

## Context

The floating-panes design handoff comes with a key map that groups the leader keys
by what they act on (agents, panes, tabs, project, hydra) and teaches them in the UI:
the key map card, its second steps, and the actions list. Several of its keys take
letters ADR 0005 gave to herdr's bindings: `j`/`l` (focus), `p` (palette), `a`
(jump), `t`, and `c` (new tab).

## Decision

The default leader keys follow the design: `j` jump, `T` talk, `1`–`3` answer, `n`
new agent, `p` new pane, `z` zoom, `x` close, `l` layout, arrows move focus, `t` new
tab, `r` rename tab, `o` open project, `w` worktrees (a second step), `f` files, `d`
changes, `,` settings, `?` key map, `q` quit (agents keep running), `a` actions.
`Alt+1`–`9`, with no leader, go to a tab. Hydra's other commands keep the letters
the design leaves free (`v`/`-` split, `H J K L` resize, `[`/`]` tabs, `g` go to,
`Space` palette, `s` a shell, …). Supersedes ADR 0005.

## Alternatives considered

- **Keep herdr's keys (ADR 0005) and only take the design's look:** the key map and
  actions list would teach keys that differ from the design's; the user chose the
  design's keys.

## Consequences

The keys on screen (key map, actions, sidebar foot) match the design. Muscle memory
from herdr breaks for focus (`h j k l` → arrows), the palette (`p` → `Space`) and
new tab (`c` → `t`). User configs can still rebind anything under `[keys.prefix]`.
`Alt+1`–`9` are taken from programs. Revisit if the design's key map changes.
