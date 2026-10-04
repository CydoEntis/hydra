# Testing

This doc covers what to test, at which level, and how to tell whether a test
proves anything. `AGENTS.md` is the operating contract; this doc holds the
testing practices in depth.

## Test levels

Use the narrowest test that proves the behaviour, then run the full suite.

| Level | Covers | Uses |
| --- | --- | --- |
| Unit | domain rules, pure functions, mappers, validators | plain inputs and outputs, no I/O |
| Service | use cases | fakes for ports (repositories, clock, mailer) |
| Integration | repositories, adapters, migrations, HTTP handlers | a real, isolated instance of the dependency |
| End-to-end | critical user journeys | the built app, driven as a user would |

Most tests should be unit and service tests. Keep end-to-end tests for the few
journeys that must never break.

## What to assert

- **Assert the outcome the user is left with**, not just that something
  changed. A test that checks "a value was written" passes while the value is
  wrong. Assert the exact output, the exact stored record, the exact response
  body, the exact error.
- Assert what was excluded as well as what was included, for filters,
  permissions, and search.
- Test behaviour through public contracts. Don't assert on private functions,
  internal call order, or implementation details that a refactor would change.
- Cover the edges: empty input, a single item, the boundary values, invalid
  input, duplicates, and concurrent or stale updates where they apply.
- A test that passes against broken code proves nothing. When you write a test
  for new behaviour, check that it fails without the change.

## Determinism

- Don't use sleeps, the real clock, randomness without a seed, live network
  calls, or shared mutable state.
- Inject the clock and the id generator. Use a fresh, isolated database or temp
  directory for each test or suite, and clean it up even when the test fails.
- Tests must pass in any order and in parallel.

## Fakes and mocks

- Prefer small in-memory fakes that implement a port over mocks that script
  call sequences.
- Mock only at the boundaries the project owns: its ports. Don't mock the code
  under test or third-party internals.
- Never let a fake drift from the real adapter's contract. Cover the real
  adapter with integration tests.

## Bugs

1. Reproduce the bug with a failing test first, whenever practical.
2. Fix it.
3. Keep the test as a regression guard, and name it after the behaviour, not the
   ticket.

## Rules

- Never delete, skip, or weaken a legitimate test to make a change pass. Change
  a test only when the intended behaviour changed, and state why.
- Keep test names as sentences describing behaviour, for example
  `rejects an expired token` rather than `test3`.
- Structure each test as arrange, act, assert. Put one behaviour in each test.
  Several assertions about that one behaviour are fine.
- Keep fixtures small and local. A shared fixture that every test tweaks is a
  hidden coupling.
- Never put real personal data or secrets in fixtures or snapshots.

## Project-specific: tools and commands

The built-in test harness: `cargo test` runs everything (unit tests sit in
`#[cfg(test)] mod` blocks beside the code). UI tests render frames with
ratatui's `TestBackend` and assert on the text and cell styles;
`HYDRA_SHOW=1 cargo test <name> -- --nocapture` prints the frames. One test:
`cargo test <name>`.

## Project-specific: layout and fixtures

Tests live next to the code they test. Client UI tests build a fake snapshot
with `design_tests::render_with(layout, w, h)` (projects, worktrees, agents in
each state) and drive it with `app.on_key` / `app.act`. Live checks use a nested
server: `HYDRA_SOCKET=test hydra`, then `hydra send` / `hydra read` against it.
Tests never touch the user's real config (`HYDRA_CONFIG` or in-memory configs).

## Project-specific: required coverage

- Status detection (`daemon/scan.rs`, hook handling): every agent's
  working / needs-you / done transitions.
- Protocol: round-trip of every message through msgpack.
- Key handling: every default binding resolves to an action, and new keys don't
  collide.
- Every popup: a render test that it opens, fits at 100×30, and closes on Esc.
- Themes: the contrast audit test covers every built-in theme.
- The daemon loop is thinly tested today; new daemon logic comes with tests.
