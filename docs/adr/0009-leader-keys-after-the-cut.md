# 0009. Leader keys: a shell is one key, and only kept features have keys

- **Status:** Accepted
- **Date:** 2026-10-09

## Context

ADR 0006 took its keys from the floating design's key map. Since then the way you
start work changed (open a shell, cd, run what you like; sessions group themselves),
and on 2026-10-09 the user cut the features that repeated others or went unused:
talk / reply / answer, open project, go to, the layouts, the queue, tickets, ideas,
races, checkpoints, past chats, agent tools, memory, presets as a list, the
pull-request view and Ship, SSH remotes, extensions, sync, the dev-server runner and
the tmux stand-in. Their keys (`T` `m` `1`–`3` `o` `g` `l` `.` `i` `I` `Q` `u` `O` `A`
`C` `U` `P` `S`) pointed at nothing. The user chose to keep the new keys rather than go
back to herdr's.

## Decision

After the leader: `j` the inbox (what needs you; type to go anywhere; a number
answers), `n` a shell where you are, `p` (or `v`, `|`) a shell beside this one, `-`
below, `z` zoom, `x` close, arrows move focus, `H J K L` resize, `b` the sidebar,
`t` new tab, `r` rename it, `]` `[` next and previous, `X` close it, `w` worktrees (a
second step), `W` a new worktree, `f` files, `d` changes, `F` find a file, `/` search
the code, `B` switch branch, `e` the sidebar, `Space` (or `:`) the palette, `a`
actions, `R` rename a session, `V` paste an image, `y` select text, `{` `}` previous
and next command, `,` settings, `?` the key map, `N` history, `q` quit (agents keep
running). Without the leader: `Alt+1`–`9` go to a tab and `Alt`+arrows move focus
(Settings turns the arrows off). The cut features' keys stay free. Supersedes ADR
0006.

## Alternatives considered

- **Go back to herdr's keys (ADR 0005):** `h j k l` focus, `c` new tab, `p` palette,
  `1`–`9` tabs. The user had learned the new ones and chose to keep them.
- **Keep keys for the cut features as unbound placeholders:** keys that do nothing
  only confuse the key map; a feature that comes back gets a key with it.

## Consequences

The key map, actions list and Settings → Keys show only what exists. Freed letters
can go to new features without moving anyone's habits. Configs that bind a removed
action get a warning and keep working. `Alt`+arrows are taken from programs in panes
unless turned off; shells that jump words with them need that setting. Revisit if a
cut feature returns or the design's key map changes.
