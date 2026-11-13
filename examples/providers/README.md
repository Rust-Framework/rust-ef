# Provider-parity example: probe → generate → apply → introspect

This example runs the same installer sequence on **SQLite, MySQL and
PostgreSQL**:

1. **Model** — two entities (`categories`, `products`) with a primary key,
   unique index, plain index and a foreign key.
2. **Generate** — `MigrationEngine::generate` turns the model into migration
   SQL for the selected dialect.
3. **Probe** — `provider.get_connection()` + `SELECT 1` checks the connection
   before touching the schema.
4. **Apply** — `MigrationEngine::apply` runs the migration (and is called
   twice to show the history table makes an installer restart idempotent).
5. **Introspect** — `IDatabaseProvider::introspect()` reads the live schema
   back as `rust_ef::schema::DbTable` values; the example asserts they match
   the model (columns, auto-increment, indexes, foreign keys) and prints a
   summary.

## Run

```bash
# SQLite: the file is recreated on each run.
cargo run -p rust-ef-providers-example -- \
    --provider sqlite --url /tmp/rust_ef_example.db

# MySQL / PostgreSQL: point at an empty database (CI provides a service).
export RUST_EF_MYSQL_URL='mysql://root:test@127.0.0.1:3306/rust_ef_test'
cargo run -p rust-ef-providers-example -- --provider mysql

export RUST_EF_PG_URL='postgres://test:test@127.0.0.1:5432/rust_ef_test'
cargo run -p rust-ef-providers-example -- --provider postgres
```

Without an environment variable, pass the connection string explicitly with
`--url <connection-string>`.

## Which APIs an installer should call

This example is the reference sequence for installers and readiness probes:

| Step | API |
|---|---|
| Register the provider | `DbContextOptionsBuilder::{use_sqlite, use_mysql, use_postgres}` (provider crate `DbContextOptionsBuilderExt`) |
| Build the model | `DbContext::from_options(&options)` then `ctx.model().build()` |
| Generate SQL | `MigrationEngine::new(dialect).generate(id, &metas, &previous_snapshot)` |
| Probe connectivity | `provider.get_connection()` + `SELECT 1` (or `async_preflight`) |
| Apply | `engine.apply(ctx.provider(), &migration)` — records `__ef_migrations_history` |
| Detect the installed schema | `ctx.provider().introspect()` → `Vec<rust_ef::schema::DbTable>` |
| Diagnose drift | diff the `DbTable` list against the model (EF-9) |

`DbContext::provider()` returns a `SessionProvider` that forwards
`introspect()`, so an installer that already holds a `DbContext` needs no
provider-specific dispatch.

## Provider capability differences

`DbTable::checks` / `DbTable::triggers` are `Option<Vec<_>>`:

| Provider | `checks` | `triggers` |
|---|---|---|
| SQLite | `Some` (parsed from `CREATE TABLE` SQL) | `Some` (from `sqlite_master`) |
| MySQL 8.0.16+ | `Some` (from `information_schema`) | `None` (no re-executable trigger SQL) |
| PostgreSQL | `Some` (from `pg_constraint`) | `Some` (from `pg_trigger`) |

`None` means "this provider cannot report it" — a diff must not interpret it
as "the database has none".

## Evidence boundary

- SQLite is exercised by `cargo test -p rust-ef-sqlite` and by this example on
  the development machine.
- MySQL/PostgreSQL introspection **cannot** be verified without a running
  server. Real-server coverage lives in
  `crates/core/tests/provider_parity_tests.rs` behind `#[ignore]` and in the
  `provider-smoke` CI job (GitHub Actions `services: mysql` / `postgres`).
  Run them locally against an empty database:

  ```bash
  export RUST_EF_MYSQL_URL='mysql://root:test@127.0.0.1:3306/rust_ef_test'
  export RUST_EF_PG_URL='postgres://test:test@127.0.0.1:5432/rust_ef_test'
  cargo test -p rust-ef --features chrono,uuid,decimal \
      --test provider_parity_tests -- --ignored --test-threads=1
  ```
