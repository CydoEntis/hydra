# 0004. One UI layout: the hydra layout

- **Status:** Accepted
- **Date:** 2026-10-03

## Context

Hydra grew several layouts (workspaces, tree, dock, sidebar, hydra). Each feature
had to be built and tested for all of them, and settings exposed choices users
didn't want.

## Decision

The `hydra` layout (sidebar of projects and sessions, panes with tabs and
splits, popups for everything else) is the only layout. Config loading forces it;
the setting is gone, and the other layouts' code has been removed (#15).

## Alternatives considered

- **Keep layouts as options:** multiplies UI work and test surface for little use.

## Consequences

One UI to polish and test. Dead layout code has to be removed over time. Revisit
only if a clearly different mode (for example a minimal single-pane view) is
needed.
