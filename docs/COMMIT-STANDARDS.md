# Commit standards

This doc covers how changes are committed, branched, and proposed. A commit is
never created just because code changed. Committing is user-invoked and
confirmation-based.

## Message format

```text
<type>(<scope>): <subject>

<body — only when the reason is not obvious from the diff>

<footer — breaking changes and issue references only>
```

**Types:**

| Type | For |
| --- | --- |
| `feat` | new user-facing behaviour |
| `fix` | a bug fix |
| `refactor` | a code change with no behaviour change |
| `perf` | a performance improvement |
| `test` | adding or fixing tests only |
| `docs` | documentation only |
| `style` | formatting only, no logic change |
| `build` | build system, dependencies, packaging |
| `ci` | CI configuration |
| `chore` | maintenance that fits nothing above |

**Subject:**

- lowercase, imperative mood ("add", not "added" or "adds");
- no trailing period;
- 50 characters or fewer;
- specific: `fix(auth): reject expired refresh tokens`, not
  `fix: bug fixes`.

**Scope** is the affected area: a feature, layer, or package. The project lists
its scopes in *Project-specific: scopes*. Leave the scope out only for truly
repo-wide changes.

**Body** explains *why*: the constraint, trade-off, or behaviour change that the
diff can't show. Wrap it at 72 characters. Don't list the files changed.

**Breaking changes** add `!` after the scope and a `BREAKING CHANGE:` footer
saying what callers must do.

```text
feat(api)!: require cursor for order listing

Offset pagination returned duplicates while orders were being inserted.

BREAKING CHANGE: `GET /orders` no longer accepts `page`; pass `cursor`.
```

## Atomic commits

- One commit is one logical change. It builds and passes tests on its own.
- Keep refactors, formatting, and behaviour changes in separate commits.
- Commit a code change together with its tests and doc updates.
- When a staged diff mixes concerns, propose a split before committing.

## Attribution

Commits carry no AI or tooling attribution. Never add:

- `Co-Authored-By: Claude`, `Co-Authored-By: Codex`,
  `Co-Authored-By: ChatGPT`, or any other AI co-author;
- `Generated-By:`, `Generated with …`, or any equivalent footer.

The configured git author and committer identity is the only authorship record.

## Commit workflow (for agents)

When the user asks for a commit:

1. Inspect `git status`, the staged diff, and the current branch.
2. Refuse forbidden files (see *Safety*) and check for secrets.
3. Classify the changes by type and scope.
4. Propose one commit, or an atomic split, and show each message with its
   files.
5. Wait for confirmation.
6. Stage and commit only the confirmed files, by explicit path.
7. Show the resulting commit or commits and the final status.

Never push unless asked separately.

## Safety

- Never commit `.env` files (other than `.env.example`), private keys,
  certificates, credentials, local databases, build output, dependency
  directories, or editor or OS artifacts.
- Abort if the diff contains credible secrets: tokens, API keys, or connection
  strings with passwords.
- Never use `git add .`, `git add -A`, `git commit --amend`,
  `git commit --no-verify`, `git reset --hard`, `git clean -f`, force-push, or
  any command that rewrites history or discards work, unless the user explicitly
  asks for that specific command.
- Commits go on a feature branch. A direct commit to a protected branch needs
  explicit, one-time authorization.

## Branches and pull requests

- Branch names use the form `<type>/<short-kebab-description>`, for example
  `feat/order-export` or `fix/token-refresh-race`. Follow
  *Project-specific: branches and tickets* if the project links ticket ids.
- Each PR covers one concern and can be reviewed in one sitting.
- The PR description covers what changed and why, how it was verified (the
  actual commands), risks, and follow-ups. List migrations, new dependencies,
  permission changes, and config changes explicitly.

## Project-specific: scopes

`client`, `daemon`, `protocol`, `cli`, `keys`, `config`, `theme`, `ui` (drawing
and popups), `git` (worktrees, branches, changes), `agents` (detection, hooks,
presets), `mcp`, `ssh`, `docs`, `deps`. Example: `feat(ui): go-to switcher`.

## Project-specific: branches and tickets

Work goes to `dev`; `main` is for releases. No ticket ids (no tracker yet).
History before this doc uses plain-sentence messages; Conventional Commits apply
from here on.

## Project-specific: tooling

None: no commit hooks or release tooling yet. The verification gate in
`AGENTS.md` is run by hand (or by the agent) before each commit.
