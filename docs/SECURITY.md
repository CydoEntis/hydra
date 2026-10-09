# Security

This doc covers how the project handles untrusted input, secrets,
authentication, authorization, and dependencies. `AGENTS.md` is the operating
contract; this doc holds the security practices in depth.

## Per-change checklist

Ask these questions for every change. If any answer is yes, add validation, a
least-privilege control, safe failure behaviour, and a targeted test. Then call
it out in the handoff.

- Does it accept input from a user, file, URL, network, IPC, webhook, or
  environment?
- Does it add or widen a permission, scope, role, CORS rule, CSP rule, or
  storage policy?
- Does it add an endpoint, a route, or a way to reach data?
- Does it read, store, or log credentials, tokens, or personal data?
- Does it render user-controlled content, or open or redirect to a URL?
- Does it run a command, build a file path, or build a query from input?
- Does it add a dependency or a build step that runs code?

## Input and output

- **Validate at the boundary** with a schema. Reject anything unknown. Don't
  just strip it and continue.
- **Injection.** Parameterize queries. Never build shell commands, file paths,
  SQL, or regexes from input by concatenation. Use allowlists for identifiers,
  such as sort fields or file names.
- **Path traversal.** Resolve paths and confirm they stay inside the intended
  directory.
- **XSS.** Rely on the framework's escaping. Never render raw HTML from input
  (`dangerouslySetInnerHTML`, `innerHTML`, `v-html`). If rich content is
  required, sanitize it with a maintained library.
- **Open redirects.** Redirect only to relative paths or an allowlist of
  hosts.
- **SSRF.** Server-side fetches of user-supplied URLs go through an allowlist,
  and never reach internal addresses.
- **Uploads.** Check size, type by content rather than extension, and
  filename. Store uploads outside the web root, or in object storage with
  private access by default.
- **Errors.** Return safe messages. Stack traces, SQL, and internal paths stay
  in server logs.

## Authentication

- Use a proven provider or library. Never write your own password hashing,
  token signing, or session handling.
- Tokens and session cookies are `HttpOnly`, `Secure`, and `SameSite`. Don't
  keep long-lived tokens in `localStorage`.
- Expire sessions, rotate refresh tokens, and invalidate sessions on logout and
  on password change.
- Rate-limit login, signup, password reset, and any endpoint that sends email
  or SMS.
- Keep responses generic, so they don't reveal whether an account exists.

## Authorization

- **Deny by default.** Every route, action, and query needs an explicit rule.
- Check authorization on the server, against the specific resource, on every
  request. Hidden UI is not access control.
- Never trust a user id, role, tenant id, or price sent by the client. Derive
  each one from the authenticated identity or from the server's own data.
- **IDOR.** Every lookup by id also checks ownership or permission.
- Test the denied path. Every protected operation has a test that shows an
  unauthorized caller is refused.

## Secrets

- Secrets live in the environment or a secret manager, never in source,
  fixtures, snapshots, docs, or examples. See [ENVIRONMENT](ENVIRONMENT.md).
- `.env.example` has keys and placeholder values only.
- Anything shipped to a browser or client bundle is public. That includes
  build-time public env vars. Never put a secret there.
- If a secret is committed, rotate it first, then remove it. Rewriting history
  alone doesn't revoke it.
- Never print secrets in logs, errors, or command output, including during
  debugging.

## Dependencies and supply chain

- Commit the lockfile. CI installs with a frozen lockfile.
- Before adding a package, check that it is maintained, check its download
  count and publisher, check for typosquats, and check any install scripts.
  Prefer fewer, well-known packages.
- Run the audit command in CI (see *Project-specific: tooling*). Fix or
  consciously accept high and critical findings, and record any accepted risk
  with a reason.
- Update dependencies deliberately and regularly, in their own commits.
- Pin CI actions and container base images to a version or digest.

## Data and privacy

- Collect the minimum data the feature needs.
- Never log personal data, tokens, passwords, or request bodies that contain
  them. Redact them in structured logs.
- Add no analytics, telemetry, or third-party scripts without an approved task
  and a documented privacy behaviour.

## Headers and transport

- Serve everything over HTTPS only.
- Set a Content Security Policy, and avoid `unsafe-inline` or `unsafe-eval`
  unless there is a recorded reason.
- Keep the CORS allowlist explicit. Never combine `*` with credentials.

## Project-specific: threat model

Assets: the user's shells and agent sessions (which can run any command as the
user), their source code, and API keys in `config.local.toml`. Threats: another
local user or process driving the daemon's socket; a process inside one pane
forging status for another pane; command injection through names, branches or
paths that end up in shell commands; secrets leaking through logs;
an SSH remote being someone else's machine.

## Project-specific: auth and permissions

No user accounts. Access to the daemon is access to its local socket: a named
pipe on Windows, a Unix socket in the temp directory elsewhere, named per user
(`seshi-<user>-<label>.sock`). Socket access must stay limited to the user who
started the daemon. Status reports (`seshi hook`) are accepted only from a
process inside the pane (traced through its process tree) or carrying that
pane's `SESHI_PANE_TOKEN`, a random secret set only in that pane's environment.
Remote work goes over the user's own `ssh`, so SSH's authentication applies.
Machine-only settings belong in `config.local.toml`.

## Project-specific: tooling

`cargo audit` (install with `cargo install cargo-audit`) before releases; GitHub
push protection for secrets. Not yet in CI (there is no CI yet).
