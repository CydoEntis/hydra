# Environment and configuration

This doc covers how the project is configured, how a new machine gets it
running, and how environments differ. `AGENTS.md` is the operating contract;
this doc holds the environment practices in depth.

## Configuration rules

- Configuration comes from the environment. Code is identical across local, CI,
  staging, and production. Only config changes between them.
- **Load once, validate once.** A single config module reads the environment at
  startup, validates it with a schema, and exports a typed object. The app
  fails fast with a clear message when a required value is missing or invalid.
- Nothing else reads environment variables directly (`process.env`,
  `import.meta.env`, `os.environ`). Everything imports the typed config.
- Give optional values their defaults in the config module, not at the call
  site.
- Name variables in `UPPER_SNAKE_CASE`, grouped by prefix (`DATABASE_URL`,
  `STRIPE_SECRET_KEY`, `FEATURE_EXPORT_ENABLED`).
- Variables exposed to a client bundle (`VITE_*`, `NEXT_PUBLIC_*`, and similar)
  are public. Only non-secret values get the public prefix. See
  [SECURITY](SECURITY.md).

## `.env` files

- Commit `.env.example`. It lists every variable the app reads, each with a
  placeholder value and a one-line comment.
- Never commit real `.env` files. They are listed in `.gitignore`.
- Adding, renaming, or removing a variable updates the config schema,
  `.env.example`, and this doc's variable table in the same change.

```bash
# .env.example
# Postgres connection string for the app role (not the admin role).
DATABASE_URL=postgres://user:password@localhost:5432/app
# Public: base URL the client uses for API calls.
VITE_API_URL=http://localhost:3000
```

## Local setup

A new contributor or agent gets from a fresh clone to a running app and a green
test suite by following *Project-specific: setup*, and nothing else. When setup
changes, update those steps in the same change. If a step can be scripted,
script it.

## Project-specific: prerequisites

Rust stable (edition 2024, so 1.85+) with `cargo`; for the cross-checks, the
`x86_64-unknown-linux-gnu` and `aarch64-apple-darwin` targets
(`rustup target add …`). Optional at runtime: `git`, `gh` (PRs, issues), the
agent CLIs you want to run, a Nerd Font for icons.

## Project-specific: setup

```sh
git clone https://github.com/CydoEntis/hydra && cd hydra
cargo build
cargo test
cargo install --path .   # then run: hydra
```

Settings live in `config.toml` (`%APPDATA%\hydra\` on Windows,
`~/.config/hydra/` elsewhere), with `config.local.toml` beside it for
machine-only values. `config.example.toml` documents every key.

## Project-specific: variables

No `.env` file: Hydra is configured through `config.toml`. Environment variables
it reads:

| Variable | Purpose | Set by |
| --- | --- | --- |
| `HYDRA_CONFIG` | use this config file instead of the default | user |
| `HYDRA_SOCKET` | name of a separate server (like `tmux -L`); used for nested test servers | user |
| `HYDRA_REMOTE` | the SSH host this client works on (from `--remote`) | hydra |
| `HYDRA_SSH`, `HYDRA_REMOTE_CMD` | replace `ssh` / the remote `hydra` command | user |
| `HYDRA_TERM_ID`, `HYDRA_PANE_TOKEN` | the pane's id and secret, set inside every pane | daemon |
| `LINEAR_API_KEY` (and Plane's) | ticket sources, when not in `config.local.toml` | user |
| `HYDRA_SHOW` | tests print rendered frames | developer |
| `HYDRA_PR`, `HYDRA_PR_DIR` | the live pull-request test (`cargo test pr_live -- --ignored`) | developer |
| `EDITOR`, `VISUAL`, `SHELL`, `COMSPEC` | editor and shell defaults | OS / user |

## Project-specific: environments

Local only: Hydra is a desktop tool. Builds come from `cargo install --path .`
(no published releases yet). `dev` is the working branch.
