# 0002. Use portable-pty (ConPTY) and vt100 for terminals

- **Status:** Accepted
- **Date:** 2026-10-03

## Context

Hydra must host real shells and full-screen agents on Windows, macOS and Linux,
and needs a parsed screen for each pane to draw it, search it, and detect agent
status from it.

## Decision

`portable-pty` spawns processes in pseudo-terminals (ConPTY on Windows, Unix PTYs
elsewhere), and `vt100` keeps each pane's emulated screen and scrollback in the
daemon.

## Alternatives considered

- **Writing our own PTY layer:** a lot of platform code, especially ConPTY.
- **alacritty_terminal / wezterm-term as the emulator:** more complete, but
  heavier and tied to those projects' internals.

## Consequences

One code path for all platforms. vt100 gaps (some newer escape sequences, image
protocols) are ours to work around. Revisit if vt100 can't render an agent CLI
correctly and a patch upstream isn't practical.
