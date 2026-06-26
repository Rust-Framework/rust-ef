# Changelog

All notable changes to **rust-ef** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [1.0.0] — 2026-06-27 — General Availability

The first production-ready release. The framework is feature-complete for
EF Core-style ORM workflows on SQLite, PostgreSQL, and MySQL, with stable
public APIs and a comprehensive documentation set.

### Highlights

- **Stable API surface**: no `#[deprecated]` residue; `EFError` / `EFResult`
  unified naming. Workspace version bumped to `1.0.0` across all crates.
- **mdBook documentation site** with full-text search, dark theme, and
  automatic GitHub Pages deployment on every push to `main`.
- **Security audit passed**: all runtime values parameterized through
  `DbValue`; identifiers sourced exclusively from compile-time entity
  metadata. See `docs/rust-ef/11-best-practices/security.md`.
- **Criterion performance benchmarks** for batch INSERT / SELECT and
  Include vs N+1 comparison.

### Added — 1.0 GA

- `docs/rust-ef/book.toml` mdBook configuration with search, fold, and
  dark theme (`navy`) defaults.
- `docs/rust-ef/SUMMARY.md` complete table of contents spanning 11
  chapters plus foreword and appendix.
- `.github/workflows/docs.yml` GitHub Pages deployment workflow using
  `peaceiris/action-mdbook` and `actions/deploy-pages@v4`.
- `docs/rust-ef/11-best-practices/security.md` six-section security
  guide: SQL injection defense, migration trust model, connection-string
  handling, sensitive-field mapping, multi-tenant filters, and a
  production hardening checklist.
- `crates/core/benches/bench_insert.rs`, `bench_query.rs`,
  `bench_include.rs` — Criterion `async_tokio` benchmarks parameterized
  over 100/500/1000 rows and 50×10 Include load.
- `CHANGELOG.md` (this file).

### Changed — 1.0 GA

- Workspace `version = "0.3.5"` → `"1.0.0"` in `[workspace.package]`;
  propagated to every inter-crate dependency
  (`rust-ef-macros`, `rust-ef-sqlite`, `rust-ef-postgres`, `rust-ef-mysql`).
- `README.md` Quick Start dependencies updated from `rust-ef = "0.3"` to
  `rust-ef = "1.0"`; added documentation badge and online docs link.
- `docs/PRODUCTION_READINESS_SPEC.md` readiness 98% → 100%, all 1.0 GA
  acceptance criteria marked complete.
- `.gitignore` now excludes `docs/rust-ef/book/` mdBook build output.

### Removed — 1.0 GA

- Deprecated type aliases `LrefError` and `LrefResult` from
  `crates/core/src/error.rs`. Use `EFError` / `EFResult` instead.

### 1.0 GA Acceptance Criteria

| Criterion | Status |
|-----------|:------:|
| chrono + uuid type support | ✅ |
| mdBook docs accessible online | ✅ |
| Performance benchmark report | ✅ |
| Security audit passed | ✅ |
| API stable, no deprecated residue | ✅ |
| ≥ 3 example projects | ✅ (`blog`, `soft_delete`, `audit`) |
| 1.0.0 release | ✅ |

---

## [0.5] — 2026-06-26 — Release Candidate 1

Navigation / advanced features fully ready plus the CLI migration tool.
Overall readiness reached ~98%, removing all P0 blockers for 1.0 GA.

### Added

- **Optimistic concurrency**: `ChangeExecutor::execute_updates` and
  `execute_deletes` now append the `#[concurrency_check]` token column to
  the WHERE clause using the original snapshot value; `rows_affected == 0`
  returns `EFError::ConcurrencyConflict`. Six end-to-end tests in
  `concurrency_tests.rs`.
- **CLI crate** (`rust-ef-cli`) with subcommands:
  - `migration add <Name> --output ./Migrations` — emit migration file
    skeleton.
  - `migration list --connection ... --provider sqlite|postgres|mysql` —
    print applied vs pending migrations.
  - `migration apply --connection ... --provider ...` — apply all
    pending migrations and record history.
  - `migration revert --connection ... --target <Name>` — roll back to
    the specified migration.
  - `migration script --from X --to Y` (or `--name SingleMigration`) —
    emit forward/reverse SQL script.
  - `scaffold dbcontext` — generate entity source from an existing
    database schema.
- **Library migration API**:
  - `MigrationEngine::apply_pending()` reads `__ef_migrations_history`
    and applies only pending migrations.
  - `revert()`, `revert_last()`, `revert_to_target()`.
  - `generate_script(from, to)` produces forward and reverse SQL.
  - `get_applied_migrations()` introspection helper.
- **FK / index diff** in `SchemaDiffer`:
  - `SchemaChange::AddForeignKey` / `DropForeignKey` integrated into
    `diff()`; generates `ALTER TABLE ... ADD CONSTRAINT` / `DROP
    CONSTRAINT`.
  - `SchemaChange::CreateIndex` / `DropIndex` integrated; SQLite/PG use
    `IF EXISTS`, MySQL uses `ON table` syntax.
  - `SnapshotColumn` carries `has_index` / `is_unique`; index diff fields
    excluded from `columns_structurally_equal` to avoid spurious
    `AlterColumn` operations. Ten tests in `index_diff_tests.rs`.
- **Subqueries / correlated filtering** via `any` / `none` / `all`
  helpers compiled to `EXISTS` / `NOT EXISTS`. Eight tests in
  `subquery_tests.rs`.
- **Global query filters**:
  - `ModelBuilder::has_query_filter` accepts `BoolExpr` from `linq!`.
  - `query_ignore_filters()` for administrator queries.
  - UPDATE/DELETE WHERE clauses also constrained by the filter.
  - Four tests in `query_filter_exec_tests.rs`.
- **Chrono / uuid / decimal optional features**:
  - `chrono` feature: `DateTime<Utc>`, `NaiveDateTime`, `NaiveDate`
    mapped to RFC3339 / `"YYYY-MM-DD HH:MM:SS"` / `"YYYY-MM-DD"`.
  - `uuid` feature: `uuid::Uuid` (with `v4`).
  - `decimal` feature: `rust_decimal::Decimal`.
  - Three dialect DDL mappings (PG `TIMESTAMPTZ`/`UUID`/`NUMERIC`;
    MySQL `DATETIME`/`CHAR(36)`/`DECIMAL(38,18)`; SQLite `TEXT`).
  - Six feature-gated tests in `extended_types_tests.rs`.
- **`exists_by_id` / `exists_by_key`** convenience methods on
  `IQueryable<T>` returning `EFResult<bool>` via `SELECT 1 ... LIMIT 1`.
  Eight tests in `exists_by_id_tests.rs`.
- **Transaction rollback + composite primary key CRUD** integration
  tests in `transaction_composite_tests.rs` (six tests).
- **GitHub Actions CI** with three-database matrix (SQLite in-process;
  PostgreSQL 16 and MySQL 8 in service containers). Lint job runs
  `cargo fmt --check` and `cargo clippy -- -D warnings` for default and
  `chrono,uuid,decimal` feature sets.
- **Soft delete and audit interceptor examples** under `examples/`.

### Changed

- `DbContext` DI registration now supports `add_dbcontext`,
  `add_dbcontext_keyed`, and `add_dbcontext_from_options`.
- README updated with modern Quick Start, multi-DB keyed registration,
  and SaveChanges interceptor snippet.
- `crates/core/src/error.rs` consolidated around the `EFError` / `EFResult`
  naming; legacy `LrefError` / `LrefResult` aliases marked `#[deprecated]`
  (removed in 1.0).

### Documentation

- All `docs/rust-ef/` chapters refreshed to reflect v0.5 behavior;
  `⚠️` markers removed or annotated with concrete follow-up tasks.

---

## [0.4] — 2026-06-22 — Beta 1

Full CRUD chain and query completeness; example projects modernized.

### Added

- **Modern `examples/blog`** rewritten around the type-map DbContext:
  `ctx.set::<Blog>()` + `ctx.save_changes()`, `linq!` queries, Include
  navigation, bulk operations, and `add_dbcontext` DI registration.
- **SQLite integration test suite expansion**: transaction rollback,
  multi-entity save, composite primary key CRUD, full type mapping
  (bool / Option / String / i32 / f64), global-filter + `linq!`
  combinations.
- **PostgreSQL / MySQL integration tests** under
  `crates/core/tests/postgres_crud_tests.rs` and
  `mysql_crud_tests.rs`, sharing a `tests/common/mod.rs` CRUD lifecycle
  helper. CI matrix executes all three databases in parallel.
- **Crate README consolidation** — every crate's README rebranded to
  `rust-ef-*` (replacing legacy `lref` references).
- **`cargo clippy -- -D warnings`** added to CI; zero warnings across
  core + three providers.

### Changed

- `find_by_id` renamed to `find` based on primary-key metadata.
- `set_property` dead code removed.
- 14 string-based query APIs renamed to `*_internal` with
  `#[doc(hidden)] pub` and `&'static str` constants, replacing the
  removed `*_named` / `filter_raw` surfaces.

---

## [0.3.5] — 2026-06-15 — DSL Unification

`linq!` macro becomes the single entry point for all query and DML
operations; all string-based APIs removed without deprecation transition.

### Added

- **`linq!` macro three forms**:
  - **Form A** — filter closure (reusable expression tree or direct
    query): `linq!(|b: Blog| b.rating > 0.5)`.
  - **Form B** — multi-clause query (`;`-separated): `include`,
    `order_by`, `group_by`, `having`, `select`, `inner_join` /
    `left_join`, `sum` / `avg` / `min` / `max` / `count`, `set` +
    `execute_update`, `take` / `skip`, etc.
  - **Form C** — value producer for `ModelBuilder` configuration:
    `filter`, `index`, `key`.
- **`LinqClause` enum** covering all query semantics; `expand_query`
  unifies expansion.
- **LINQ terminal methods**: `last`, `last_or_default`, `single`,
  `single_or_default`, `long_count`, `all`, `contains`,
  `to_dictionary`.
- **`ModelBuilder` DSL**: `has_query_filter` accepts `BoolExpr`;
  `has_index` / `has_key` accept `&'static [&'static str]` produced by
  `linq!(index ...)` / `linq!(key ...)`.

### Removed

- String-based APIs `include_named`, `then_include_named`,
  `set_column`, `filter_raw`, `sum("col")`, `avg("col")`,
  `inner_join(...)`, `group_by(&[...])`, `having("...")` removed
  immediately (no `#[deprecated]` transition). All user code must
  migrate to the `linq!` macro.

---

## [0.3] — 2026-05-20 — Type-Map DbContext

Architectural refactor: `DbContext` no longer holds typed `DbSet<T>`
fields. Sets are lazily created via `set::<T>()` against a type-map,
enabling generic `save_changes()` iteration.

### Added

- **Type-map `DbContext`** with `ctx.set::<T>()` lazy initialization.
- **`Arc<dyn IDbContext>` DI** integration with `rust-dicore`.
- **`SetOps<T>` type-erased dispatcher** — `save_changes()` iterates
  all registered entity types without per-entity code generation.
- **`IDbContext` object-safe trait** with `provider()`,
  `save_changes()`, `change_tracker()`.
- **`IDbContextExt`** for non-object-safe generic helpers such as
  `use_transaction(f)`.
- **Keyed multi-database registration** via `add_dbcontext_keyed` and
  `provider.get_keyed("name")`.
- **`FromDbContextOptions` DI bridge** with `from_options(&DbContextOptions) -> Self`.
- **SaveChanges interceptor pipeline**: `ISaveChangesInterceptor`
  with `on_saving`, `on_saved`, `on_save_failed` hooks.
- **`MigrationEngine`** library API with model-snapshot diff,
  three-dialect Up/Down SQL generation, and history tracking.
- **Three providers**: `rust-ef-sqlite`, `rust-ef-postgres`,
  `rust-ef-mysql`, each following the unified module structure
  (`sql_generator.rs`, `provider.rs`, `connection.rs`,
  `type_conversion.rs`, `type_mapping.rs`, `introspection.rs`,
  `di_extension.rs`).

---

## [0.2] — 2026-04-10 — Alpha 2

Early scaffold with manual per-entity `save_changes` and a `linq!`
prototype.

### Added

- `#[derive(EntityType)]` macro with 12 attributes (`table`,
  `primary_key`, `auto_increment`, `required`, `max_length`, `column`,
  `foreign_key`, `navigation`, `not_mapped`, `index`, `unique`,
  `concurrency_check`).
- Initial `BoolExpr` AST (Filter / Raw / And / Or / Not) and IN /
  BETWEEN / IS NULL / `contains` support.
- SQLite provider with CRUD lifecycle test coverage.

---

## [0.1] — 2026-03-01 — Initial Alpha

Project skeleton, workspace layout, and the `IDbContext` / `IDbSet<T>`
/ `IEntityType` trait hierarchy.

### Added

- Workspace with `crates/core`, `crates/macros`, `crates/sqlite`,
  `crates/postgres`, `crates/mysql`, `crates/cli`.
- `IEntityType` / `IFromRow` / `IGetKeyValues` / `IEntitySnapshot`
  trait hierarchy.
- `IDatabaseProvider` abstraction with `ISqlGenerator`,
  `IAsyncConnection`, `execute_migration_command(sql)`.
- Initial `MigrationStore` and migration history table DDL.

---

[1.0.0]: https://gitcode.com/rf2026/rust-ef/releases/tag/v1.0.0
[0.5]: https://gitcode.com/rf2026/rust-ef/releases/tag/v0.5
[0.4]: https://gitcode.com/rf2026/rust-ef/releases/tag/v0.4
[0.3.5]: https://gitcode.com/rf2026/rust-ef/releases/tag/v0.3.5
[0.3]: https://gitcode.com/rf2026/rust-ef/releases/tag/v0.3
[0.2]: https://gitcode.com/rf2026/rust-ef/releases/tag/v0.2
[0.1]: https://gitcode.com/rf2026/rust-ef/releases/tag/v0.1
