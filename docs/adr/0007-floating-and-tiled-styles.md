# 0007. One layout, drawn floating or tiled

- **Status:** Accepted
- **Date:** 2026-10-09

## Context

ADR 0004 made the hydra layout (sidebar of projects and sessions, panes with tabs
and splits, popups for the rest) the only UI and ruled out a layout setting. The
floating-panes design keeps that layout but draws it as rounded cards with gaps
between them, and asks for a setting to pack the same cards edge to edge ("tiled"),
plus corners, gap, dimming and focus-border options.

## Decision

The hydra layout stays the only layout. How it is drawn is a style: `ui.panes`
chooses floating (cards with margins and gaps, the default) or tiled (the same
cards edge to edge), next to `ui.corners`, `ui.gap`, `ui.dim`, `ui.focus_border`
and `ui.pill_caps`. A style changes spacing and drawing only, never what is on
screen or where things live. Supersedes ADR 0004.

## Alternatives considered

- **Floating only:** keeps ADR 0004 as it was, but drops the design's tiled option.
- **Bring back other layouts:** the reason for ADR 0004 still holds; every feature
  would have to be built and tested per layout.

## Consequences

One layout to build and test; its drawing takes a few settings (`Look` in
`src/client/hydra/card.rs`). Anything that would move or hide parts of the UI is a
new layout and needs a new decision.
