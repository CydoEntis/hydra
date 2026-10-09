# 0008. The app is called Seshi

- **Status:** Accepted
- **Date:** 2026-10-09

## Context

"hydra" collides with THC-Hydra, a password-cracking tool packaged as `hydra` in Debian,
Ubuntu, Kali, Arch and Homebrew, and is shared by several well-known projects, so the
command can clash on a PATH and the name can't be searched for. The app also needed an
identity of its own beyond the floating design.

## Decision

The app, its command and binary are `seshi` ("a session", many of them at once). Config
and data move to `seshi` folders, environment variables to `SESHI_*`, the per-repo file to
`.seshi.toml`, the MCP server and its tools to `seshi` / `seshi_*`, release archives to
`seshi-<target>`, and the repository to `CydoEntis/seshi`. Its default themes are Seshi
Night and Seshi Day (golden hour: warm driftwood dark or sand light, sunset peach focus,
sun for what needs you, sea glass for leader mode), and its splash is a block SESHI wordmark.

Kept for what's already out there: an old install's config and data are copied over once
on first start (`config::migrate_old_names`, the old default theme moved to the new one);
hooks tagged `_hydra` are recognised and rewritten; `.hydra.toml` is still read; checkpoint
refs stay under `refs/hydra/checkpoints/`. Internal code names (the `client::hydra`
module, `HyHit`, …) are unchanged.

## Alternatives considered

- **Keep "hydra":** the name clash on Linux and macOS package managers stays.
- **Rename the internal modules too:** a large, risky diff with nothing for users; can
  happen gradually.

## Consequences

One-time migration code to keep until no hydra installs remain. Users of the old name
reinstall once (`seshi` doesn't replace `hydra.exe`); old hydra installs can't update into
seshi by themselves. GitHub redirects the old repository URLs.
