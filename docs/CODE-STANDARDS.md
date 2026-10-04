# Code standards

This doc covers how to write code in this repository. `AGENTS.md` is the
operating contract; this doc holds the coding practices in depth. The rules are
language-agnostic. Examples use TypeScript.

## Core principles

When the rules below don't settle a question, these do. They are listed in
priority order, so when two conflict, the higher one wins.

1. **Readability over cleverness.** Code is read far more often than it is
   written. Choose the version a new contributor understands on first read, even
   when it is longer. Avoid dense one-liners, clever operator tricks, and
   metaprogramming used where a plain loop or `if` would do.
2. **Keep it simple (KISS).** Solve today's problem with the least machinery.
   Before you add a class, generic, config option, or layer, name the concrete
   problem it removes right now.
3. **You aren't gonna need it (YAGNI).** Don't build for imagined requirements:
   no hooks, flags, extension points, or parameters without a current caller.
   Adding one later, when it's needed, is cheap.
4. **Separation of concerns.** Each module, function, and layer has one reason
   to change. Keep apart:
   - presentation, business rules, and I/O (see [ARCHITECTURE](ARCHITECTURE.md));
   - deciding what to do from actually doing it: compute a plan, then apply it;
   - parsing or validating input from acting on it.
5. **Don't repeat yourself (DRY), applied to knowledge rather than text.** Every
   business rule, constant, and schema has one authoritative home. Two pieces of
   code that happen to look alike but change for different reasons are **not**
   duplication; merging them couples unrelated things. Extract on the second
   real repetition of the same knowledge, not the first time two lines look
   alike.
6. **Small, pure functions.** Default to functions that take inputs and return
   outputs, with no hidden reads (globals, clock, env, I/O) and no hidden writes.
   Push side effects out to the edges (handlers, services, adapters), so the
   logic in the middle is pure and trivially testable. Split a function when it
   does more than one job or mixes levels of abstraction, not when it reaches an
   arbitrary line count.
7. **Explicit over implicit.** Make dependencies, side effects, and failure
   modes visible in names and signatures. No action at a distance, no magic
   conventions that only work if you already know them.
8. **Composition over inheritance.** Build behaviour from small functions and
   objects passed in. Use inheritance only for a true "is-a" relationship with a
   shallow hierarchy.
9. **Least surprise.** Code does what its name says, and nothing more. Follow
   the patterns already in the codebase, even where you'd have chosen
   differently. Consistency beats local optimisation.

## Naming

- Use precise domain terms. Avoid `data`, `info`, `item`, `thing`, `helper`,
  `manager`, `util`, and `handle` when a more exact word exists.
- Use the same term for the same concept across UI, services, persistence, and
  tests. If the domain calls it an "invoice", don't call it a "bill" in one
  layer.
- Name booleans as assertions: `isActive`, `hasAccess`, `canRetry`.
- Name functions as verbs: `createOrder`, `parseHeader`, `sendReceipt`. A getter
  that computes something expensive is not `getX`. Call it `computeX` or
  `loadX`.
- Put units in the name when the type doesn't carry them: `timeoutMs`,
  `sizeBytes`, `priceCents`.
- Avoid abbreviations unless they are standard in the domain (`id`, `url`,
  `http`).
- Name a file after its one responsibility. Follow
  *Project-specific: naming* for file casing.

## Functions

- Each function does one job at one level of abstraction.
- Guard edge cases early and return. Avoid deep nesting, and never nest
  ternaries.
- Make side effects visible in the name and the signature. A function named
  `format…` never writes to disk.
- Prefer a few explicit parameters. With more than three, or with optional
  flags, take a named options object.
- Avoid boolean flag parameters that switch behaviour. Write two functions
  instead.
- Don't split readable logic into single-use wrappers only to shorten a
  function.

## Types and data

- Use strict mode or its equivalent. Don't weaken compiler settings.
- Use `unknown` with validation instead of `any`. Parse external data with a
  schema at the boundary.
- Model states as discriminated unions, and handle every case with an exhaustive
  check.
- Make illegal states unrepresentable. Prefer a union of states over a bag of
  optional fields.
- Don't use casts or non-null assertions just to silence the compiler. If an
  invariant makes one safe, the reason should be locally obvious or stated in a
  comment.
- Treat values as immutable by default. Local mutation inside a small, contained
  algorithm is fine.
- Store and transmit timestamps as UTC ISO 8601. Convert to local time only for
  display.

## No magic numbers or strings

A literal that carries meaning gets a name. The name says what the value
*means*, and changing the value then happens in one place.

- **Name** any number other than an obvious `0`, `1`, or `-1`. That covers
  limits, timeouts, sizes, thresholds, retry counts, and status codes.
- **Name** strings that act as identifiers: statuses, roles, event names,
  route paths, storage keys, header names, feature flags, and error codes. Model
  a closed set as a union or enum, not as scattered literals.
- **Put the constant next to its owner.** It lives in the feature or module
  that owns the concept. Promote it to a shared module only when a second
  feature needs it. A single `constants` file holding everything is itself a
  smell.
- **Values that differ by environment are config, not constants.** URLs,
  credentials, and tunables go through the typed config module.
- **Strings the user sees** belong in the project's copy or i18n mechanism when
  there is one. They are not scattered through logic.
- **Exceptions.** Literals in tests are fine, and often clearer, as are
  self-describing values like `items.length === 0`.

```ts
// Bad
if (attempts > 5) await sleep(30000);
if (order.status === "shpd") notify(order);

// Good
const MAX_SEND_ATTEMPTS = 5;
const RETRY_DELAY_MS = 30_000;
const OrderStatus = { Pending: "pending", Shipped: "shipped" } as const;

if (attempts > MAX_SEND_ATTEMPTS) await sleep(RETRY_DELAY_MS);
if (order.status === OrderStatus.Shipped) notify(order);
```

## Errors

- Split failures into two kinds. **Expected** failures, such as not found,
  validation, conflict, or permission, are typed results or typed errors that
  the caller handles. **Bugs** throw and surface.
- Never write an empty `catch`. Each one handles the error, rethrows it with
  added context, or converts it to a typed result.
- Add context when you rethrow: what was attempted, with which identifiers. Keep
  the original error as the cause.
- Show users safe messages. Never expose stack traces, SQL, or internal paths
  outside the process.
- Fail fast on invalid configuration at startup, not on first use.

## Comments

Most code needs no comments. Names, types, and small functions do the
explaining. A comment earns its place only when it tells the reader something
the code cannot.

**Write a comment for:**

- **Why**, when the reason is non-obvious: a constraint, a trade-off, or a
  rejected alternative that looks better but fails.
- **Invariants and assumptions** that the code relies on but cannot express.
- **Workarounds**, naming the underlying bug or limitation, with a link when one
  exists.
- **Surprising behaviour** a careful reader would get wrong, such as ordering,
  concurrency, timezones, or units.
- **Public API docs** (docstrings or JSDoc) on exported functions and types whose
  contract is not obvious from the signature. Cover the contract: inputs,
  outputs, errors, side effects.

**Don't write a comment that:**

- restates the code (`// increment counter`, `// return the user`);
- narrates the change or its history (`// added for new flow`,
  `// changed from X`). That belongs in the commit message.
- labels sections of a function that should be split instead;
- keeps commented-out code. Delete it; git remembers.
- carries a ticket id or author name as its only content;
- apologises or hedges (`// hacky but works`) without explaining the constraint.

**Form:**

- Write full sentences, placed above the code they describe. Keep them short.
- Update or delete a comment in the same change that makes it wrong. A stale
  comment is worse than none.
- A `TODO` must name the condition that resolves it. For example: `TODO: remove
  once the v1 endpoint is retired`, not `TODO: fix`. Never leave a TODO for
  behaviour the current task requires.

```ts
// Bad: restates the code.
// Loop over the users and check if active
for (const user of users) if (user.isActive) notify(user);

// Good: explains a constraint the code cannot show.
// The provider rejects batches over 100 recipients with a generic 400,
// so chunk here rather than relying on its error.
for (const batch of chunk(recipients, 100)) await mailer.send(batch);
```

## Dependencies

- Reuse what the project already has before you add a package.
- A new dependency must be maintained, well-scoped, and justified in the
  change summary. Commit lockfile changes together with the package change.
- Wrap SDKs and heavy libraries in an adapter, so their types stay out of the
  domain.

## Security

Every change follows [SECURITY](SECURITY.md). The two rules to keep in mind while
typing are these: parameterize everything, and never log secrets or personal data.

## Prohibited shortcuts

- `any`, `@ts-ignore`, disabled lint rules, or lowered strictness used to hide a
  type or design problem;
- swallowed errors or empty catch blocks;
- mocked or fake behaviour presented as finished production code;
- placeholder TODOs for behaviour the current task requires;
- unrelated refactors or mass reformatting inside a feature change;
- copy-pasted blocks where a shared function already exists.

## Project-specific: language and framework rules

See `docs/stack/rust.md`. In short: `anyhow::Result` with `.context()` at I/O
boundaries; no `unwrap`/`expect` on anything that comes from outside (PTY output,
config, the socket, the filesystem); `saturating_sub` for terminal geometry; no
blocking calls inside async tasks or on the UI loop.

## Project-specific: naming

Standard Rust: `snake_case` files, functions and modules; `PascalCase` types and
enum variants; `SCREAMING_SNAKE_CASE` consts. Drawing functions are `draw_*`, key
handlers `on_*_key`, hydra-layout methods on `App` are prefixed `hy_`. Config
action names are `kebab-case` (`split-right`, `go-to`).

## Project-specific: formatting and linting

`rustfmt` with the defaults (`cargo fmt`), and `cargo clippy --all-targets -- -D
warnings`. No lint config file; allow-attributes are local and explained.

## Project-specific: exceptions

- **UI copy is user-facing text, not magic strings.** Labels and hints in the
  `draw_*` functions stay inline where they are drawn.
- **Layout numbers** (popup widths, column offsets) may stay inline in drawing
  code when used once; repeated ones become named consts.
