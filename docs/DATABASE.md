# Database

> **Not applicable:** Seshi has no database. Its state is small files: `config.toml`
> (settings), a JSON session snapshot written by the daemon (`daemon/persist.rs`), and
> the client's saved projects list. Add a pack if a database ever arrives.

This doc covers how data is stored, queried, migrated, and protected.
`AGENTS.md` is the operating contract; this doc holds the persistence practices
in depth.

## Principles

- The database is the durable source of truth. Caches and UI stores only
  reflect it.
- Only repositories and adapters touch the database. UI, handlers, and domain
  logic contain no queries.
- Repositories return domain types. Row shapes, ORM entities, and driver types
  stay inside the data layer.
- Validate or map records when they cross from the database into application
  code. Never assume stored data matches the current schema.
- Never silently drop or rewrite user data.

## Queries

- Parameterize every dynamic value. Never build SQL from input by string
  concatenation, including for identifiers such as sort columns. Use an
  allowlist for those.
- Wrap multi-step changes that must succeed or fail together in a single
  transaction, owned by the service or repository method that knows the
  invariant.
- Select only the columns you need. Avoid N+1 queries by batching or joining
  deliberately.
- Paginate or bound every list query. No query returns an unbounded result set.
- Map database errors, such as unique violations and foreign key failures, into
  typed domain errors at the repository.

## Schema design

- Every table has a primary key. Choose one ID strategy and use it everywhere.
- Every table has `created_at` and `updated_at`, stored in UTC.
- Enforce integrity in the database as well as in code: `NOT NULL`, foreign
  keys, unique constraints, and check constraints where they express a real
  rule.
- Store money as integer minor units or a fixed-point decimal, never as float.
- Soft-delete user-owned content by default, with a `deleted_at` column.
  Permanent deletion needs explicit product behaviour and a recovery plan.
- Justify each index by a real query. Index foreign keys used in joins.
- Don't create tables for features that are not being built.

## Migrations

- Migrations are committed, ordered, and deterministic. Once a migration has
  been applied anywhere shared, it is append-only.
- Never edit an applied migration. Write a new one.
- Every migration runs cleanly on a blank database and on the previous released
  schema.
- Destructive changes need a written plan and explicit approval before they are
  written. That covers dropping a column or table, narrowing a type, and
  rewriting data. Prefer expand-then-contract: add the new column, migrate
  reads and writes, backfill, then remove the old column in a later release.
- Long-running or locking migrations on large tables must say how they avoid
  downtime.

## Data safety

- Never run tests against a developer's real database or any shared
  environment.
- Never commit database files, dumps, or connection strings containing
  credentials.
- Stale writes must not overwrite newer data. Use a revision or version column,
  or an equivalent check, wherever concurrent edits are possible.
- Keep backups and a tested restore path in place before any destructive
  migration or sync feature ships.

## Testing

- Test repositories and migrations against a real, isolated instance of the same
  engine: a temp file, a container, or a per-test schema.
- Cover clean install, upgrade from the previous schema, constraint violations,
  transaction rollback, concurrency conflicts, and soft delete and restore where
  they exist.

## Project-specific: engine and access


## Project-specific: conventions


## Project-specific: migrations workflow


## Project-specific: entities

