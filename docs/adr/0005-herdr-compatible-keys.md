# 0005. Leader-key bindings follow herdr where they overlap

- **Status:** Accepted
- **Date:** 2026-10-03

## Context

Users come from herdr (and tmux). Hydra's original keys put common actions
(splits, focus, tabs) on letters that clashed with that muscle memory.

## Decision

Default bindings after the leader (`Ctrl+Space`) follow herdr for shared actions:
`v` / `-` split, `h j k l` focus, `x` close, `z` zoom, `b` sidebar, `c` new tab,
`[` / `]` tabs, `q` detach, `g` go to, `?` keys. Hydra's own features take the
remaining letters (`n` new session, `p` palette, `t` new agent, …). Every action
is also in the command palette with plain-language names.

## Alternatives considered

- **Keep Hydra's original keys:** familiar to nobody but early testers.
- **tmux defaults:** `"` and `%` splits are hard to remember.

## Consequences

Easy switch from herdr. User configs from before the change can override the new
defaults; old `[keys.prefix]` entries may need clearing. Revisit if herdr's
defaults change substantially.
