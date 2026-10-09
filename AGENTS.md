# AGENTS.md

This file is the operating contract for AI coding agents and human contributors
in this repository. Read it in full before you plan or edit anything. It tells
you which docs to read, how they fit together, how to keep them current, and how
to report back.

A nested `AGENTS.md` in a subdirectory may add narrower rules for that
directory. When rules conflict, the closest file wins.

## Documentation map

Read this file first. Then read the docs that match the change you are making.
Load them when the work touches their area, not all of them up front.

| Doc | Read before | Owns |
| --- | --- | --- |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | adding modules, moving code, crossing layers, picking a pattern | layers, dependency direction, where code lives, pattern catalog |
| [`docs/CODE-STANDARDS.md`](docs/CODE-STANDARDS.md) | writing or reviewing any code | naming, functions, types, errors, **comments**, prohibited shortcuts |
| [`docs/TESTING.md`](docs/TESTING.md) | writing tests or fixing bugs | test levels, determinism, what to assert |
| [`docs/API_PATTERNS.md`](docs/API_PATTERNS.md) | touching endpoints, services, IPC, or client data flow | request flow, validation, response and error shapes |
| [`docs/DATABASE.md`](docs/DATABASE.md) | touching schema, queries, or migrations | persistence rules, migrations, data safety |
| [`docs/SECURITY.md`](docs/SECURITY.md) | handling input, auth, secrets, permissions, or adding a dependency | threat checklist, auth rules, secrets, supply chain |
| [`docs/ENVIRONMENT.md`](docs/ENVIRONMENT.md) | adding config or env vars, changing setup | config loading, `.env.example`, local setup, environments |
| [`docs/UI.md`](docs/UI.md) | building or changing any user interface | component structure, UI state, async states, accessibility |
| [`docs/COMMIT-STANDARDS.md`](docs/COMMIT-STANDARDS.md) | committing, branching, opening a PR | message format, commit workflow, git safety |
| the plan doc (see *Planning*) | starting, finishing, or scoping work | what is in, out, and in what order |
| [`docs/stack/`](docs/stack/) | touching a technology that has a stack doc | the rules for each technology used in this project |
| [`docs/adr/`](docs/adr/) | changing a recorded decision | decisions that are expensive to reverse |

Each rule lives in exactly one doc. If two docs seem to disagree, the doc that
**owns** the topic in the table above wins. Report the conflict so it can be
fixed.

## How the docs are structured

Rules come in three layers:

- **Generic base.** These are rules that hold for any well-built project. Treat
  them as defaults.
- **Stack docs** in `docs/stack/`. Each one holds the rules for one technology
  or kind of app this project uses, such as React, Supabase, or a browser game.
  See *Stack packs* below.
- **Project-specific sections**, headed `## Project-specific: …`. This is where
  a project records its own choices, layout, and exceptions.

An unfilled project-specific section contains an HTML comment that starts with
`<!-- FILL:` at the beginning of a line. The comment says what goes there and
gives an example. A section can be partly filled: settled facts first, then a
narrower `FILL` marker for what is still open.

A doc that doesn't apply to this project yet opens with the line
`> **Not applicable:** <reason>`, right under its title. Its rules take effect
once the need arrives.

How to interpret them:

1. A filled project-specific section overrides the stack docs, and the stack
   docs override the generic base on the same topic.
2. An unfilled `FILL` section means the project has not decided. Follow the
   generic base. Do not invent a project rule to fill the gap.
3. If a task needs a decision that a `FILL` section would hold, such as which
   database, which test runner, or which error shape, ask the user. Record the
   answer in that section, following the next section, *Maintaining these docs*.
4. Run `grep -rnE "^<!-- FILL:" AGENTS.md CLAUDE.md docs/` to list every open decision.

Precedence, from highest to lowest:

1. The user's explicit instruction for the current task.
2. The nearest nested `AGENTS.md`.
3. Project-specific sections.
4. Stack docs in `docs/stack/`.
5. The generic base.

If following an instruction would break a non-negotiable rule, stop and say so
before you act.

## Maintaining these docs

The docs are part of the codebase. Keep them true.

**Update a doc in the same change when you:**

- add, move, or remove a module, layer, or top-level directory;
- change a public contract (API shape, error codes, schema, config, CLI flags);
- add a significant dependency or replace a tool (see *Stack packs*);
- start, finish, or split planned work (see *Planning*);
- establish a convention that the next contributor has to follow;
- discover that a doc is wrong about the code.

**How to edit:**

- Edit the doc that owns the topic. Link to it from elsewhere; never copy a rule
  into a second doc.
- When you fill a `FILL` section, delete the marker. If the section does not
  apply to this project, replace the marker with one line saying so and why, for
  example `Not applicable: no persistent storage.`
- Write rules and current contracts, not history. "Migrations are append-only"
  belongs in a doc. "In March we changed X because Y" belongs in an ADR or a
  commit message.
- Keep entries short and imperative. If one entity or endpoint needs a
  paragraph, it should probably live in code as types or tests.
- Add an ADR in `docs/adr/` for any decision that is expensive to reverse. That
  includes choosing a database, an auth model, an architecture style, or a major
  framework.

**What needs the user's approval first:**

- Changing or removing a rule in the generic base or in a filled project-specific
  section. Propose the change and your reason, then wait. Once the user approves
  a change to a generic-base or stack-doc section, add
  `<!-- local: <reason> -->` under that section's heading. `/starter update`
  then asks before it replaces the section.
- Filling a `FILL` section with a decision the code does not already show.
  Filling one from facts in the repo, such as which test runner `package.json`
  already uses, is fine; say so in your handoff.

**Never:**

- loosen a rule so that your change passes;
- delete a `Project-specific` heading, which leaves nowhere to record the
  decision later;
- let this file and `CLAUDE.md` drift apart. `CLAUDE.md` imports this file and
  adds only tool-specific notes.

## Stack packs

Technology-specific rules come from **packs**: reusable docs shipped with the
`starter` skill (`/starter` in Claude Code, `$starter` in Codex). A project
copies the packs it uses into `docs/stack/`. `docs/stack/README.md` records the
starter version, the preset, and each applied pack.

**When a task brings in a technology the project doesn't cover yet** (a new
database, a renderer, a payments SDK, a realtime server):

1. Stop before you write code that depends on it.
2. Tell the user, and suggest `/starter add <tech>`. That command applies a
   matching pack, or writes a project-local stack doc when no pack exists.
3. If the skill isn't available, do the same by hand. Ask the user for the key
   decisions, write `docs/stack/<tech>.md` in the pack format (a `Use for`,
   `Requires`, `Fills` header, then rules, then its own `Project-specific`
   section), record expensive choices as an ADR, add it to
   `docs/stack/README.md`, and point the relevant `Project-specific` sections
   at it.

**When a technology is removed**, delete its stack doc, restore the `FILL`
markers or `Not applicable` notes it answered, and update
`docs/stack/README.md`.

A project that turns out not to need a doc, such as a static game with no
database, marks it `Not applicable: <reason>` rather than keeping rules that
don't apply. When the need arrives, fill it in again.

## Planning

Scope and order live in one **plan doc**, created and kept by the `roadmap`
skill (`/roadmap init`, then `/roadmap feature …`, `/roadmap reconcile`). Its
path is recorded in *Project-specific: workflow and tooling*. The default is
`docs/PLANNING.md`.

- Work only on what the current task names. Don't start the next item because
  it looks close or convenient.
- Mark work shipped or done in the plan only after its acceptance criteria are
  met **and** the verification gate has passed in the real worktree. Partial,
  unverified, or throwaway-branch work is not done.
- Record follow-ups that surface mid-task in the plan. Don't silently widen the
  current task.
- Reordering, adding, or cutting planned work is the user's decision. Propose
  it; don't make it.

## Workflow

### Before changing code

1. Read this file and the docs that apply to the task.
2. Inspect the relevant code, scripts, and `git status`. Find the existing
   pattern before you create a new one.
3. Keep user-authored and unrelated changes. Never discard or reformat them.
4. Identify the smallest coherent change that delivers the requested outcome.
5. If the requirements conflict with the docs, or the change is destructive,
   stop and ask.

### While changing code

- Deliver one vertical slice or one concern per change.
- Don't refactor code the task doesn't need, add speculative abstractions, or
  add configuration points with no current consumer.
- Prefer boring, explicit code over clever indirection.
- Reuse existing dependencies before adding one. Justify every new dependency.
- Never hand-edit generated files.
- Don't reformat untouched code.

### Before declaring done

Run the verification gate in the next section. If a command does not exist or
cannot run, say so exactly. Never report a check as passing unless it ran and
passed in this worktree.

### Handoff

Report:

1. the outcome, in plain language;
2. the important files changed;
3. the verification commands you ran and their actual results;
4. docs you updated, and any `FILL` sections you filled or found missing;
5. new dependencies, migrations, permissions, or config;
6. known limitations and the logical next step.

Don't narrate every edit. Separate verified facts from proposals.

## Non-negotiables

These apply in every project built from this starter:

1. Follow the [core principles](docs/CODE-STANDARDS.md#core-principles), in
   their priority order: readability over cleverness, KISS, YAGNI, separation of
   concerns, DRY for knowledge, small pure functions, explicit over implicit.
   Named constants replace magic values.
2. Dependencies point inward. Domain logic never imports frameworks, I/O, or UI.
   See [ARCHITECTURE](docs/ARCHITECTURE.md).
3. Validate all external input at the boundary where it enters. Run the
   [security checklist](docs/SECURITY.md#per-change-checklist) on every change.
4. No secrets in source, fixtures, logs, snapshots, or command output.
5. Never swallow an error. Every `catch` handles the error, rethrows it with
   context, or converts it into a typed result.
6. Never weaken a type, lint rule, or test to make a change pass.
7. Comments explain *why*, not *what*. See
   [CODE-STANDARDS § Comments](docs/CODE-STANDARDS.md#comments).
8. Commits follow [COMMIT-STANDARDS](docs/COMMIT-STANDARDS.md) and carry no AI
   or tool attribution.
9. Never run destructive git commands, rewrite history, commit, push, or change
   remote resources unless asked.

## Communicating with the user

- State your assumptions early, especially when you are working from an
  unfilled `FILL` section.
- When blocked, say what is blocking you. Never change the requirement to route
  around a blocker.
- When the docs and the user's request disagree, name the rule and ask which to
  follow. Don't pick one silently.
- Offer to record any decision the user makes that the docs lack.

## Project-specific: mission

Seshi is a terminal multiplexer built for running many coding agents at once
(Claude Code, Codex, Gemini, OpenCode, …): see which ones need you, jump to them,
and keep them running when you leave. It serves developers who run several agents
across several projects, on Windows, macOS and Linux. The one thing it must do
well: never lose or misreport an agent; status (working, needs you, done, idle)
has to be right, and sessions survive the UI closing.

## Project-specific: non-negotiables

1. **The daemon owns every session.** The client is a view; closing, crashing or
   detaching the client must never kill a pane. Anything that touches PTYs or
   process lifetime lives in `src/daemon/`.
2. **Bump `PROTOCOL_VERSION`** (`src/protocol.rs`) on any change to a message
   shape. A client and daemon of different versions must refuse to talk, not
   misparse.
3. **Windows is a first-class target.** Every change builds and behaves on
   Windows (ConPTY, named pipes, PowerShell) as well as Linux and macOS; the
   cross-checks in the gate are not optional.
4. **One layout.** The `seshi` layout is the only UI. Floating and tiled are two
   styles of drawing it (ADR 0007); don't add other layouts or switches between them.
5. **Status reports are only trusted from the pane itself**: its process tree, or
   its `SESHI_PANE_TOKEN`. Never accept a status change from an unverified source.
6. **No blocking work on the UI thread.** Git, file walks, network and process
   spawns go through background tasks (`spawn_bg`) and report back as messages.

## Project-specific: verification gate

Run from the repo root, in order; all must pass:

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo check --target x86_64-unknown-linux-gnu
cargo check --target aarch64-apple-darwin
```

For UI changes, also look at the screen: tests render frames with `TestBackend`
(`SESHI_SHOW=1 cargo test <name> -- --nocapture` prints them), and a live check
runs a nested server with `SESHI_SOCKET=<name> seshi`.

## Project-specific: workflow and tooling

- **Plan:** `docs/ROADMAP.md` (the plan doc: phases, scope, decisions, rules). Design references: `docs/design-brief.md`, `docs/design-brief-v3.md`, `docs/design-brief-v4.md` (phase 8, out for design).
- **Branches:** work lands on `dev` at https://github.com/CydoEntis/seshi; one
  commit per logical change, pushed after the gate passes.
- **Install locally:** `cargo install --path .` (stop a running `seshi` first on
  Windows, or the binary is locked).
- **Releases:** set `version` in `Cargo.toml`, commit, bring `main` up to `dev`, then tag
  `vX.Y.Z` on `main` and push the tag. `.github/workflows/release.yml` builds Windows, macOS
  (Apple Silicon and Intel) and Linux (x64, ARM; static musl) archives with checksums and
  publishes the GitHub Release that `install.sh` / `install.ps1` download from.
- **Tickets:** GitHub issues on CydoEntis/seshi, labelled by phase (`phase-5`, …) and linked from the plan.
