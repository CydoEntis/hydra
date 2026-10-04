# Stack docs

This folder holds the rules for each technology this project uses. Most are
applied from packs shipped with the `starter` skill. Some are written for this
project alone (source `project`).

Precedence: stack docs override the generic base. A doc's `Project-specific:`
sections override the stack docs.

## Starter

- Version: 1.0.0
- Preset: none (Rust CLI / TUI; no pack matches)
- Applied: 2026-10-03

`/starter update` compares this version with the installed skill and brings the
starter-owned sections forward. It never touches `Project-specific:` sections.

## Applied packs

| Pack | Source | Version |
| --- | --- | --- |
| rust | project | — |

## Not applied

Packs the preset suggested that this project deliberately left out:

- typescript (and every other shipped pack): the project is Rust; `rust.md` is a local stack doc instead.

## Adding, changing, or removing a pack

Follow **Stack packs** in `AGENTS.md`, or run `/starter add <tech>` or
`/starter remove <pack>`.
