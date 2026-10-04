# 0001. Run sessions in a daemon, with a thin client

- **Status:** Accepted
- **Date:** 2026-10-03

## Context

Agents run for minutes to hours. Closing a terminal window, a crash in the UI, or
switching machines must not kill them, and several views (a second window, the
CLI, an SSH client) need to see the same sessions.

## Decision

One binary runs as a background daemon (server) that owns every PTY, emulator and
agent process, and as a client that only draws and sends input. They talk over a
local socket. The daemon starts on demand and outlives its clients.

## Alternatives considered

- **Single process (like a plain terminal app):** simpler, but every session dies
  with the window.
- **Wrap tmux / zellij:** no native Windows support, and agent-status detection
  would sit outside the thing that owns the PTY.

## Consequences

Detach / reattach, multiple clients, SSH remotes and the CLI all come for free.
In exchange there is a wire protocol to version, and two processes to debug.
Revisit only if the daemon becomes a reliability problem in itself.
