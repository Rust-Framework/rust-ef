# Changelog

All notable changes to **rust-ef** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> 注：IDbContext trait 已在后续版本中移除，本文档中相关内容仅作历史记录。

---

## [2.5.0] — 2026-11-14 — 上游真库缺陷修复：PG NULL、MySQL DDL、网络 diff、诊断、连接串

> **English** · **简体中文**
>
> Defect-fix release driven by the 2026-11-14 Travel real-database verification
> (MySQL 8.0.22, PostgreSQL 16, SQLite). Correctly-used MySQL/PostgreSQL paths no
> longer need application-side workarounds: PostgreSQL binds NULL for every
> nullable type, MySQL indexes enum/expression columns, network `bootstrap →
> introspect → diff` no longer produces spurious `MODIFY`/`DROP CHECK`, driver
> errors keep SQLSTATE/SQL, and connection-string/TLS construction is consistent.
> Additive public API; the documented behaviour changes are listed under
> Compatibility.

#### English

**Fixed**

- **EF-PG-01 (P0) — typed NULL binding.** `DbValue::Null` is serialized as a real
  untyped NULL (`IsNull::Yes`) instead of `None::<String>`, so `None` can be written to
  nullable `BIGINT/INTEGER/SMALLINT/NUMERIC/BOOLEAN/DATE/TIMESTAMP/TIMESTAMPTZ/UUID/BYTEA`
  columns. `error serializing parameter N` is gone; regression
  `postgres_null_binding` (ignored, live PG) writes and reads back NULL for every type.
- **NUMERIC read/write.** The PostgreSQL provider now binds `DbValue::Decimal` natively
  and decodes non-NULL `NUMERIC` cells through `rust_decimal` (`db-tokio-postgres`), instead
  of (a) binding a string that the binary protocol rejects for a NUMERIC slot and (b)
  silently materializing non-NULL NUMERIC cells as `DbValue::Null` (indistinguishable from
  a SQL NULL).
- **EF-MYSQL-01 (P1) — indexable enum columns.** On MySQL, a custom/contract type with
  `max_length(n)` (or an explicit `TEXT` + `max_length`) now renders `VARCHAR(n)`
  (`CHAR(n)` for fixed-length) instead of unbounded `TEXT`, so ordinary/composite indexes
  build without error 1170. PostgreSQL/SQLite keep `TEXT`.
- **EF-MYSQL-02 (P1) — mixed expression indexes.** `has_index_expression` classifies bare
  identifiers as columns and everything else as expressions: `(tenant_id, (coalesce(...)))`
  instead of `((tenant_id), ...)` (MySQL 8 error 3762).
- **EF-DDL-01 (P1) — portable identifier names.** All auto-derived index/foreign-key names
  (including single-column `ix_<table>_<col>` and FK names) are sanitized to 63 bytes with a
  deterministic 32-bit hash suffix; explicit `has_name` still wins. No MySQL 1059, no
  PostgreSQL silent truncation, stable across runs.
- **EF-DIFF-01 (P1) — network diff fidelity.** MySQL introspection reads
  `COLUMN_TYPE` (full `varchar(64)`/`tinyint(1)`/`decimal(10,2)`/`datetime(6)` spelling) and
  functional-index `EXPRESSION` (with a pre-8.0.13 fallback); comparison normalizes integer
  display widths, BOOLEAN, UNSIGNED, server-canonicalized defaults, identifier quotes, casts,
  charset introducers and redundant parentheses. Implicit defaults on auto-increment columns
  and live-only server defaults are not drift. PostgreSQL/MySQL never emit
  `DROP CHECK`/`DROP CONSTRAINT` from a textual expression mismatch — they report
  `constraint_not_introspected` info instead. A freshly bootstrapped schema diffs to zero
  statements.
- **EF-ERR-01 (P2) — structured driver errors.** New boxed `EFError::Database(DatabaseErrorInfo)`
  keeps SQLSTATE, vendor code, message/detail/hint, table/column/constraint and the failing
  SQL (≤512 chars, `Display` includes them) for PostgreSQL, MySQL and SQLite. Unique
  violations stay `EFError::UniqueViolation`. `webx` maps `Database` to 500 with
  `public_detail() = None`.
- **EF-PG-02 (P2) — TLS defaults/API consistency.** `use_postgres` / `PostgresProvider::new`
  now honor `sslmode`: absent means TLS required, `disable`/`allow` select plaintext,
  `verify-*` are handled by the provider instead of being rejected by `tokio_postgres`.
  `use_mysql` / `MySqlProvider::new` likewise honor an explicit `ssl-mode` (new
  `MySqlTlsMode::Preferred`) instead of always overriding it; absent means TLS required.

**Added**

- **EF-DIFF-02 (P2) — read-only drift reporting.** `MigrationDiff::has_drift()`,
  `summary()` (`DiffSummary`), `drift_tables()`. Drift is strict: pending statements or
  actionable (`Warning`) warnings; info-level “extra object” / “expression not comparable
  verbatim” notes do not turn a readiness probe red. No DDL is executed.
- **EF-CONN-01 (P2) — ADO.NET connection strings.**
  `ConnectionString::parse_ado(input, protocol)` and `parse_auto(...)` support
  `source/server/host/data source/address`, `uid/user/user id/username`, `pwd/password`,
  `database/initial catalog`, quoted values, `;` in quoted values, trailing/doubled
  semicolons, `host:port` / `[IPv6]:port`, `port=`, and `sslmode`/`ssl-mode` normalization.
  ADO → `build()` → `parse()` round-trips.
- **EF-DDL-02 (P2) — portable CHECK builder.** `rust_ef::check_expr::CheckExpr`
  (`in_list`, `not_in_list`, `exactly_one_of`, `at_most_one_of`, `at_least_one_of`,
  `all_not_null`, `between`, `non_negative`, `positive`, `all_of`, `any_of`, `raw`) plus
  `EntityTypeBuilder::has_check_expr` / `PropertyBuilder::has_check_expr`. Raw `has_check`
  expressions get generation-time dialect validation
  (`MigrationEngine::validate_check_expression`) with `-- WARNING` comments in DDL.
- **Provider capability matrix.** `rust_ef::migration::provider_capabilities(dialect)` /
  `ProviderCapabilities` / `SupportLevel` describe enum columns, expression/partial/INCLUDE
  indexes, CHECK, table options, comments, generated columns, composite FKs and ALTER COLUMN
  per dialect.
- **CLI dry run.** `rust-ef migration ddl --snapshot <file> [--dialect all|sqlite|postgres|mysql] [--out <file>]`
  renders the code-first DDL, report and capability matrix without a database;
  `rust-ef migration capabilities [--dialect ...]` prints the matrix.
- **CI.** New optional `db-integration` job runs the ignored MySQL/PostgreSQL parity tests
  and the PostgreSQL NULL-binding matrix against service containers.

**Compatibility**

- `EFError` gained a `Database(Box<DatabaseErrorInfo>, Option<BoxError>)` variant:
  exhaustive `match EFError` needs a `_ => ...` arm (the enum is still not
  `#[non_exhaustive]`). The payload is boxed so `EFError` stays small
  (`clippy::result_large_err`).
- Auto-derived names longer than 63 bytes now truncate + hash. Databases created with the
  old longer names get an informational `skipped_removal` for the old index (never a DROP);
  add an explicit `has_name` with the old name or write a versioned rename if strict parity
  is required.
- MySQL `DbColumn::data_type` is now `COLUMN_TYPE` (full spelling); dispatch on it by
  containment (as scaffold does), not equality.
- Runtime diff no longer drops CHECK constraints on PostgreSQL/MySQL; change them through a
  versioned migration.
- `MySqlTlsMode` gained `Preferred`; exhaustive matches need one more arm.
- Application-side workarounds made obsolete: the enum `code_first_model_for` VARCHAR patch,
  the mixed-expression-index `cast(concat_ws(...))` collapse, explicit long index names, and
  “skip runtime diff on network providers”.

#### 简体中文

**修复**

- **EF-PG-01（P0）**：`DbValue::Null` 以真正的无类型 NULL 绑定，`None` 可写入任意可空
  `BIGINT/INTEGER/SMALLINT/NUMERIC/BOOLEAN/DATE/TIMESTAMP/TIMESTAMPTZ/UUID/BYTEA`；新增真库回归
  `postgres_null_binding`。
- **NUMERIC 读写**：PostgreSQL `DbValue::Decimal` 以原生 NUMERIC 绑定；非空 `NUMERIC`
  通过 `rust_decimal` 解码为 `DbValue::Decimal`，不再因字符串参数被二进制协议拒绝，
  也不再静默变成 `DbValue::Null`。
- **EF-MYSQL-01（P1）**：自定义枚举/新类型 + `max_length(n)` 在 MySQL 生成 `VARCHAR(n)`
  （定长 `CHAR(n)`），可建普通/复合索引；PG/SQLite 保持 `TEXT`。
- **EF-MYSQL-02（P1）**：`has_index_expression` 裸标识符按列、其余按表达式，混合索引不再生成
  `((tenant_id), ...)`（MySQL 3762）。
- **EF-DDL-01（P1）**：自动索引/外键名统一 63 字节 + 稳定哈希截断，显式 `has_name` 优先。
- **EF-DIFF-01（P1）**：MySQL 内省改用 `COLUMN_TYPE` 全拼写并读取函数索引 `EXPRESSION`
  （旧版本回退）；列类型/默认值/CHECK/表达式索引按方言归一化；网络方言不再生成
  `DROP CHECK`；`bootstrap` 后 `introspect + diff` 无语句。
- **EF-ERR-01（P2）**：新增 boxed `EFError::Database(DatabaseErrorInfo)`，保留
  SQLSTATE/厂商码/message/detail/hint/table/column/constraint/失败 SQL（≤512 字符）；
  唯一冲突仍为 `UniqueViolation`；`webx` 映射为 500 且不泄露。
- **EF-PG-02（P2）**：`use_postgres`/`PostgresProvider::new` 按 URL `sslmode` 决定 TLS：
  缺省要求 TLS，`disable`/`allow` 明文，`verify-*` 由 provider 处理；`use_mysql` 同样尊重
  URL `ssl-mode`（新增 `MySqlTlsMode::Preferred`），缺省要求 TLS。

**新增**

- **EF-DIFF-02（P2）**：`MigrationDiff::has_drift()` / `summary()` / `drift_tables()`，
  只读 drift 可接 `/health/ready`；info 级“无法逐字验证”不算 drift。
- **EF-CONN-01（P2）**：`ConnectionString::parse_ado` / `parse_auto` 支持 ADO.NET 键值串
  （别名、引号值、引号内分号、尾随/重复分号、host:port、`[IPv6]:port`、TLS 归一），
  ADO → `build()` → `parse()` 往返。
- **EF-DDL-02（P2）**：`CheckExpr` 可移植 CHECK 构造器 + `has_check_expr`；
  `validate_check_expression` 在生成期标记已知方言陷阱。
- **能力矩阵**：`provider_capabilities(dialect)` / `ProviderCapabilities` / `SupportLevel`。
- **CLI**：`migration ddl`（三方言 dry-run，不连库）与 `migration capabilities`。
- **CI**：可选 `db-integration` job，跑 MySQL/PG ignored 真库用例与 PG NULL 矩阵。

**兼容性**

- `EFError` 新增 `Database` 变体（payload 已 box），下游穷举 `match` 补 `_ => ...`。
- 超过 63 字节的自动名改为截断 + 哈希；旧库同名索引只会报 `skipped_removal` info，不会 DROP。
- MySQL `DbColumn::data_type` 变为 `COLUMN_TYPE` 全拼写（按包含关系匹配）。
- 运行时 diff 不再 DROP PG/MySQL CHECK（改 CHECK 走版本化迁移）。
- `MySqlTlsMode` 新增 `Preferred`，穷举 match 补分支。
- 应用侧规避可移除：枚举 `code_first_model_for` 补丁、表达式索引折叠、超长索引名 `has_name`、
  网络 provider 跳过运行期 diff。

---

## [2.4.0] — 2026-11-13 — 依赖图外键边、聚合空集中性值、运行时 diff 反向计划

> **English** · **简体中文**
>
> Additive release for the 2.3 public API: no signature removed or changed, and
> `MigrationDiff` keeps its fields (struct literals stay compatible). Two behaviour fixes
> (EF-16 insertion order, EF-17 empty-set aggregates) and one new optional feature
> (`webx`, zero cost unless enabled).

#### English

**Fixed**

- **EF-16 — save order for single-sided relationships.** `DependencyGraph::build` now
  derives type-level dependencies from three metadata sources: `HasMany`/`ManyToMany`
  (existing), `BelongsTo` navigations (new — the declaring entity is the dependent, so the
  edge is reversed relative to `HasMany`), and foreign-key metadata (new: scalar
  `#[foreign_key(Target)]` and `has_foreign_key` composite keys, reverse-looked-up from the
  referenced table name, excluding views/keyless entities). A child that declares the
  relationship twice gets one deduplicated edge. `tx.add(User) + tx.add(Member)` with only
  `#[foreign_key(User)]`/`BelongsTo<User>` (no `HasMany<Member>` on the parent) now inserts
  the parent first in one `save_changes()`. Self references/cycles keep the existing
  ignore/fallback strategy; `HasOne` does not add an edge (its FK lives on the other side).
- **EF-17 — aggregates over empty input.** `linq!(…; sum col)` / `avg` now normalise the SQL
  `NULL` cell of an empty result set (or an all-`NULL` column) to the neutral value `0.0`
  instead of failing with `EFError::TypeConversion("SUM result is not f64")`. Non-empty
  results, SQL text, signatures and error text are unchanged; `min`/`max` still return
  `None` and `count` is untouched.

**Added**

- **EF-9 supplement — `MigrationDiff::reverse(dialect)`.** Purely additive runtime diffs
  can render a reverse plan (`CREATE TABLE`→`DROP TABLE`, `ADD COLUMN`→`DROP COLUMN`,
  `CREATE INDEX`→`DROP INDEX`, `ADD CONSTRAINT`→the matching drop, `CREATE SEQUENCE`→
  `DROP SEQUENCE`, in reverse order). Diffs with drops, column changes, table rebuilds,
  `manual_action` warnings or comment changes on pre-existing objects return a typed
  `EFError::Migration` that lists the irreversible items.
- **`MigrationDiff::irreversible_items()`**; `report()` now ends with an `irreversible
  items` section so an upgrade report states what cannot be rolled back automatically.
- **Optional `webx` feature (XR-1)**: `impl From<EFError> for rust_webx_core::error::Error`
  plus `rust_ef::webx::map_ef_error`, so a web handler can `?`-propagate an `EFError` into
  `ActionResult<T>`. Default off (no dependency in the default build); enable with
  `rust-ef = { version = "2.4", features = ["webx"] }`. Mapping: `NotFound` 404,
  `ModelValidation`/`Query`/`TypeConversion` 400, `ConcurrencyConflict`/`UniqueViolation`
  409 (driver constraint/table metadata is never exposed), `Connection`/`Transaction` 503,
  `Migration`/`Configuration`/`Provider`/`ChangeTracking`/`Other` 500 with `public_detail()`
  `None`. The `From` impl cannot live in a separate crate (orphan rule E0117); a future
  shared cross-framework crate belongs in a `rust-core` crate under the Rust-Framework
  organisation.

**Compatibility**

- `EFError` stays **not** `#[non_exhaustive]` in 2.4 (adding it is a breaking change for
  exhaustive downstream matches); it is scheduled for 3.0. The `webx` mapping matches all
  13 variants exhaustively, so a future variant fails compilation instead of being
  silently mis-mapped.
- Docs: relationship save order (EF-16), aggregate empty-set semantics (EF-17), runtime
  reverse plans/irreversible items (EF-9) and the `webx` feature.

#### 简体中文

**修复**

- **EF-16 — 单侧关系声明的同批保存顺序**：`DependencyGraph::build` 现在从三类元数据推导
  类型级依赖：`HasMany`/`ManyToMany`（原有）、`BelongsTo` 导航（新增，方向与 `HasMany` 相反）、
  外键元数据（新增：标量 `#[foreign_key(Target)]` 与 `has_foreign_key` 复合键，按被引用表名
  反查 principal，排除 view/keyless）。同一对类型重复声明只保留一条去重边。仅子侧声明
  `#[foreign_key(User)]`/`BelongsTo<User>`（父侧无 `HasMany<Member>`）时，单次
  `save_changes()` 也会先插父行。自引用/环沿用既有忽略/降级；`HasOne` 不加边。
- **EF-17 — 空集聚合**：`sum`/`avg` 对空结果集（或全 `NULL` 列）的 SQL `NULL` 单元格在结果层
  归一为中性值 `0.0`，不再报 `EFError::TypeConversion("SUM result is not f64")`；非空结果、
  SQL 文本、签名、错误文案均不变；`min`/`max` 仍 `None`，`count` 不受影响。

**新增**

- **EF-9 附加 — `MigrationDiff::reverse(dialect)`**：纯加性运行时 diff 可生成逆序反向计划；
  含删除/改列/表重建/`manual_action`/已有对象注释变更时返回 typed `EFError::Migration`
  并列出不可逆项。
- **`MigrationDiff::irreversible_items()`**；`report()` 末尾新增 `irreversible items` 段。
- **可选 `webx` feature（XR-1）**：`impl From<EFError> for rust_webx_core::Error` +
  `rust_ef::webx::map_ef_error`，Web Handler 里 `?` 可直接把 `EFError` 传播进
  `ActionResult<T>`；默认关闭、零依赖。映射：404/400/409/503/500（唯一冲突不泄露
  constraint/table，5xx 走 `public_detail() = None`）。`From` 不能放在独立 crate
  （孤儿规则 E0117）；未来如需跨框架公共库，另立 `rust-core` 归 Rust-Framework 组织。

**兼容性**

- `EFError` 在 2.4 仍**不是** `#[non_exhaustive]`（对下游穷举 match 属破坏性变更），推迟到
  3.0；`webx` 映射穷举 13 个变体，未来新增变体会编译失败而不是被静默误映射。

## [2.3.0] — 2026-11-13 — 运行时升级闭环、安装可脚本化、错误与查询模型化

> **English** · **简体中文**
>
> Additive release for the 2.2 public API. Two behavior changes to note:
> EF-12 now infers `NOT NULL` from the Rust field type (`Option<T>` is nullable,
> everything else is required) unless explicitly overridden; EF-10 changes the
> *order* of statements in generated scripts (same semantics, dependency-safe).

#### Compatibility notes (read first)

- `EFError` is **not** `#[non_exhaustive]`; the new `UniqueViolation` variant makes an exhaustive
  `match` on `EFError` a compile error — add a `_ => ...` arm when upgrading (same shape as the
  `ConcurrencyConflict` variant added in 2.2).
- The canonical schema DTO is now `rust_ef::schema::DbTable` (table + columns/indexes/foreign
  keys/checks/triggers). `rust_ef_mysql::DbTable` and `rust_ef_postgres::DbTable` are kept as
  re-exports for source compatibility, but a struct literal written against the old 2-field
  `DbTable { name, columns }` must move to the new type.
- `Migration` keeps its fields (struct-literal construction is not broken); runtime differences
  use the new `MigrationDiff`, and `into_migration()` bridges it into the history-based flow.
- `IDatabaseProvider::introspect()` is additive with a default implementation returning a typed
  error, so third-party providers keep compiling.

### English

#### Added — runtime model diff and upgrades (EF-9)

- **`MigrationEngine::diff` / `diff_metas`** compares the model snapshot with the
  live schema introspected by the provider and returns
  `MigrationDiff { statements, warnings, rebuilds }`. `apply_diff` executes it.
- **SQLite table rebuild**: constraints SQLite cannot `ALTER` in place (composite
  FK, CHECK, changed nullability) are applied through a shadow-table rebuild
  (`__ef_rebuild_<table>` → copy common columns → rename → recreate
  indexes/triggers) inside one transaction, with `PRAGMA foreign_keys` disabled
  outside the transaction, a `foreign_key_check` before commit, and a typed
  pre-check for new `NOT NULL` columns without defaults. A failed plan rolls back
  completely (no half-applied schema, no leftover shadow table).
- `rebuilds` reports the copied columns, the new columns and the triggers kept,
  so an installer can show an upgrade report instead of guessing.

#### Added — install/bootstrap without migration history (EF-8, EF-11)

- **`MigrationEngine::bootstrap`** creates the full schema (tables, indexes,
  constraints) on an empty database without creating or writing
  `__ef_migrations_history`; `UpScriptMode` selects the script variant.
- **Model-declared seed data**: `SeedData` + `bootstrap_with_seed` +
  `DbContext::bootstrap()` give one install path (`bootstrap → seed`) instead of
  the caller assembling `EntitySnapshot`s by hand.

#### Added — schema introspection on every provider (EF-5)

- `IDatabaseProvider::introspect()` (default implementation returns a typed
  error for third-party providers) returns `rust_ef::schema::DbTable`;
  `DbContext::provider().introspect()` works directly. SQLite implements it with
  `sqlite_master` + `PRAGMA table_info/index_list/index_info`; MySQL and
  PostgreSQL reuse their existing introspection and now sit behind the same trait.

#### Added — connection and provider primitives (EF-15)

- `ConnectionString` (`parse`/`build`/`validate`, percent-encoded userinfo,
  typed port/scheme errors, PG `sslmode` / MySQL `ssl-mode` allow-lists) and
  `ProviderKind` (`sqlite`/`mysql`/`postgres`, URL schemes, default ports,
  migration dialect).

#### Added — typed uniqueness errors (EF-2)

- `EFError::UniqueViolation { constraint, table }`; SQLite (2067/1555), MySQL
  (1062) and PostgreSQL (23505) map driver errors to it, so callers branch on a
  variant instead of sniffing messages.

#### Added — save interceptor with mutable entities, audit filler (EF-7)

- `ISaveChangesInterceptor::on_saving_mut(&mut MutableSaveChangesContext)` runs
  before change detection inside the save transaction, so an interceptor can set
  `updated_at`/`updated_by`/`version` and have them persisted in the same
  transaction. The read-only interceptor API is unchanged.
- `IAuditable` + `IClock`/`SystemClock` + `IAuditActor` + `AuditInterceptor`
  (`.track::<T>()`) — the minimal framework-side filler Travel used to hand-code.

#### Added — query and materialization (EF-1, EF-3, EF-6)

- `linq!` compiles `.contains()` as `IN` before extracting the receiver, so the
  IN receiver may be any collection expression (`ids`, `self.ids`,
  `make_ids()`, `&ids[..]`), not only a field path (EF-1).
- `ExecuteUpdateBuilder::set_column` is public and semver-covered; the old
  `set_column_internal` stays as a deprecated alias for one release (EF-3).
- `IFromRow::from_row_named(&[(name, DbValue)])` materializes by column name;
  the derive emits it and `materialize_entities_named` uses it, so reordered
  columns/views cannot silently shift values (EF-6).

#### Added — provider smoke examples and CI (EF-14)

- `examples/providers` runs `generate → apply → introspect` against SQLite,
  MySQL and PostgreSQL with one command; a CI `provider-smoke` job starts MySQL
  and PostgreSQL services; real-database parity tests are `#[ignore]`d for local
  runs and documented in the example README.

#### Changed

- **EF-12**: `#[entity]` derives requiredness from the Rust field type
  (`Option<T>` → nullable, other scalars → `NOT NULL`); explicit
  `#[required]`/`.is_required()`/`.is_nullable()` still win. Existing installed
  databases are unaffected, but a previously exported snapshot diffed against a
  re-derived model will report nullable→NOT NULL; re-export the snapshot and
  review the first migration (see `docs/.../auto-upgrade.md`).
- **EF-10**: generated scripts are grouped by dependency
  (`CREATE TABLE → CREATE INDEX → ADD CONSTRAINT`, plus drop/alter sections).
  Semantics are unchanged; MySQL no longer fails when an FK references an index
  created later in the script.
- **EF-13**: a partial MySQL migration failure (`PartialMigrationFailure`) lists
  executed/failed/not-executed statements in the error, making deterministic
  cleanup possible. MySQL DDL is still non-transactional — that is now documented
  instead of implied.
- **EF-4**: `SqliteOptions::foreign_keys(true)` forces `PRAGMA foreign_keys=ON`
  on every acquired connection (same init path as WAL/busy_timeout); bundled
  SQLite may already default to ON, this makes it explicit and build-independent.
- Macro-generated paths now resolve the crate's real dependency name
  (`proc-macro-crate`), so `ef = { package = "rust-ef" }` works without
  `extern crate ef as rust_ef;` (MACRO-1).
- `rust-dix` requirement raised to `1.0` (macro crate-name resolution, trait-object
  `instance<T: ?Sized>`, fallible typed factory resolution).

#### Docs

- `auto-upgrade.md` (runtime diff/bootstrap), `migrations-seed.md`
  (model-declared seeds), and the provider example README (installer API map).

### 简体中文

#### 新增
- **EF-9 运行时差量升级**：`MigrationEngine::diff/diff_metas`（模型 ↔ 活库 schema）→
  `MigrationDiff{statements,warnings,rebuilds}`，`apply_diff` 执行；SQLite 用影子表重建
  实现无法原地 ALTER 的约束（单事务、事务外 FK OFF、提交前 foreign_key_check、失败全回滚、
  NOT NULL 无默认预检）；PG/MySQL 走原生 ALTER 并给出结构化 warnings/rebuilds。
- **EF-8/EF-11 安装闭环**：`bootstrap` 空库建全量 schema 且不建/不写迁移历史表；
  `SeedData` + `bootstrap_with_seed` + `DbContext::bootstrap()` 一条 `bootstrap→seed` 路径。
- **EF-5 统一内省**：`IDatabaseProvider::introspect()`（第三方 provider 默认 typed error）
  返回 `schema::DbTable`，`DbContext::provider().introspect()` 可直接用；SQLite 补齐实现。
- **EF-15**：`ConnectionString`（parse/build/validate、百分号编码、typed 错误、SSL 白名单）
  与 `ProviderKind`（URL scheme/default port/migration dialect）。
- **EF-2**：`EFError::UniqueViolation{constraint,table}`，三方言错误码映射。
- **EF-7 + 审计填充**：`on_saving_mut(&mut MutableSaveChangesContext)`（变更检测前、同事务）
  + `IAuditable`/`IClock`/`IAuditActor`/`AuditInterceptor`，业务可删手写审计赋值。
- **EF-1/EF-3/EF-6**：`contains` 的 IN 形态支持任意集合表达式；`set_column` 公开且 semver
  覆盖（旧名 deprecated 别名保留一版）；`from_row_named` 按列名物化（防静默错位）。
- **EF-14**：`examples/providers` 一命令跑三方言 + CI services 冒烟 + `#[ignore]` 真库测试。

#### 变更（必读）
- **EF-12 行为变更**：非 Option 标量推断为 NOT NULL；已装库不受影响，但旧快照对新模型 diff
  会报「可空→NOT NULL」，请重导 `model_snapshot.json` 并 review 首个迁移。
- **EF-10**：生成脚本按依赖分段（语义不变；MySQL FK 引用索引不再顺序失败）。
- **EF-13**：MySQL 部分失败时 `PartialMigrationFailure` 逐条列出已执行/失败/未执行语句；
  引擎级非原子语义改为文档固定。
- **EF-4**：`SqliteOptions::foreign_keys(true)` 显式打开连接级 FK（与 WAL 同一 acquire 路径）。
- **MACRO-1**：宏展开按真实依赖名解析，`ef = { package = "rust-ef" }` 改名后无需 extern 垫片。

#### 兼容性说明
- `EFError` 未加 `#[non_exhaustive]`，新增 `UniqueViolation` 变体对下游穷举 `match` 是
  名义上的编译期破坏（与 2.2 的 `ConcurrencyConflict` 同型）；升级时补一个 `_ => ...` 分支即可。
- `schema::DbTable/DbColumn` 为内省 DTO，新增字段对结构体字面量构造者是名义破坏；正常使用
  introspection 的调用方不受影响。
- `Migration` 结构体未加字段（避免破坏字面量构造）：差量结果使用新的 `MigrationDiff`。

## [2.2.0] — 2026-11-12 — 数据库级约束、EFCore 实体配置对齐与可组合条件

> **English** · **简体中文**

### English

#### 1. Database-level constraints & migration automation

##### Added — model-level DDL constraints

- **Declarative indexes**: `EntityTypeBuilder::has_index(&[cols])` now emits one
  composite index instead of degrading to single-column indexes. The returned
  `IndexBuilder` supports `.is_unique()`, `.has_name(name)`, and
  `.has_filter(linq!(|x: T| ...))` (partial index).
- **Partial indexes**: PostgreSQL/SQLite emit `... WHERE <predicate>`. MySQL has
  no partial indexes, so a filtered **unique** index is emulated with a
  functional `CASE WHEN <predicate> THEN <col> END` unique index; a filtered
  non-unique index is skipped with a warning comment instead of producing
  invalid SQL.
- **Expression indexes**: `has_index_expression(&["coalesce(channel_id, '')"])`
  (`.is_unique()` supported).
- **Composite foreign keys**: `has_foreign_key(&[cols], Ref::TABLE, &[ref_cols])`
  with `.on_delete(...)` / `.has_name(...)`.
- **Scalar `#[foreign_key(Target)]` now participates in DDL**: it generates a
  single-column FK to `Target`'s primary key (previously only the `FK_<Target>`
  constant was generated).
- **CHECK constraints**: `entity.has_check(name, sql)` and
  `property_named(col).has_check(sql)` (auto-named `CK_<table>_<column>`), emitted
  inline in `CREATE TABLE` for PostgreSQL/MySQL/SQLite.
- **Column defaults**: `property_named(col).has_default(sql)` emits `DEFAULT`
  in `CREATE TABLE`, `ADD COLUMN`, and `ALTER COLUMN` (PG `SET/DROP DEFAULT`,
  MySQL `MODIFY COLUMN`).
- **MySQL table options**: `entity.engine("InnoDB").charset("utf8mb4").collate("utf8mb4_bin")`.
  MySQL DDL always emits `ENGINE`/`DEFAULT CHARSET` (defaults `InnoDB`/`utf8mb4`).
- **`Option<T>` inner type mapping**: `Option<String>` + `max_length` now maps to
  `VARCHAR(n)` (indexable on MySQL) instead of `TEXT`; `Option<Uuid>` /
  `Option<Decimal>` map to their inner types.
- **Migration diffing** covers declared indexes, composite FKs, CHECK
  constraints, column defaults, and MySQL table options; Down migrations restore
  dropped index/CHECK/FK definitions.
- **SQLite migration scripts** emit foreign keys inline in `CREATE TABLE`
  (SQLite cannot `ALTER TABLE ADD CONSTRAINT`).

##### Fixed

- **rust-ef-mysql**: `cell_to_db_value` probed `uuid::Uuid` before `String`;
  sqlx-mysql's UUID decoder accepts *any* 16-byte slice, so 16-character
  `VARCHAR` values were silently decoded as UUIDs (36-char text). `String` is
  now probed first; UUID columns (`CHAR(36)`) parse back via
  `TryFrom<DbValue> for Uuid`.
- PostgreSQL `ALTER COLUMN` now syncs column defaults (`SET DEFAULT` /
  `DROP DEFAULT`) instead of leaving them stale.
- Migration snapshots now round-trip `max_length`, column defaults, indexes,
  FKs, CHECKs, and table options (serde_json-based snapshot IO).
- **Partial-index filter literals** (`has_filter(...)`) are rendered correctly on
  every dialect: MySQL backslashes are escaped inside string literals,
  PostgreSQL `bytea` values use `decode('<hex>', 'hex')` (its `X'..'` is a *bit*
  string), floats always keep a decimal point, and non-finite floats,
  value-less conditions and subquery nodes are rejected instead of emitting
  invalid or silently-broadened predicates.
- **Migration script splitting** understands quoted literals (doubled `''` and
  MySQL `\` escapes), `--` line comments, `/* */` block comments and PostgreSQL
  `$tag$ … $tag$` bodies, so a `;` inside any of them no longer truncates the
  statement. DDL comments now share the same dialect-aware string quoting as
  index predicates (one helper, no duplicated escaping).

##### Notes / compatibility

- `EntityTypeMeta` gained `indexes`, `foreign_keys`, `checks`, and
  `table_options`; manual struct literals must add
  `..EntityTypeMeta::default()`.
- `SnapshotColumn` gained `default_value`; `SnapshotEntityType` gained
  `indexes` / `foreign_keys` / `checks` / `table_options`.
- `EntityTypeBuilder::has_index` now returns `IndexBuilder<'_ , T>` instead of
  `&mut Self` (call sites that used it as a statement are unaffected).
- `ensure_created` still creates tables + seeds only; it does **not** create
  standalone indexes or foreign keys. Use `MigrationEngine::generate` + `apply`
  for the full constraint set.


#### 2. EFCore entity configuration parity

##### Added — EntityTypeBuilder / PropertyBuilder parity

- **`HasColumnType`**: `#[column_type("...")]` / `property_named(...).has_column_type("...")`
  emits the column type verbatim (e.g. `JSON`, `MEDIUMTEXT`, `ENUM(...)`).
- **`HasPrecision`**: `#[precision(p, s)]` / `.has_precision(p, s)` maps
  `Decimal` to `DECIMAL(p,s)`/`NUMERIC(p,s)` and temporal types to
  `DATETIME(p)`/`TIMESTAMP(p)`.
- **`IsFixedLength`**: `#[fixed_length]` / `.is_fixed_length()` → `CHAR(n)`.
- **`HasCollation`**: `#[collation("...")]` / `.has_collation("...")` → column
  `COLLATE` before nullability.
- **`HasComment`**: `#[comment("...")]` / `.has_comment("...")` for columns and
  `#[table("t", comment = "...")]` / `entity.has_comment("...")` for tables
  (MySQL inline/ALTER, PostgreSQL `COMMENT ON`, SQLite ignored).
- **`HasComputedColumnSql`**: `#[computed("expr", stored)]` /
  `.has_computed_column_sql("expr").is_stored()` emits
  `GENERATED ALWAYS AS (...)`; computed columns are automatically excluded from
  INSERT/UPDATE.
- **`HasAlternateKey`**: `entity.has_alternate_key(&[cols])` → unique
  `ak_<table>_<cols>` index usable as an FK target.
- **`ToView` / `HasNoKey` / `ToTable(schema)`**: `#[table("v", view)]`,
  `#[keyless]`, `#[table("t", schema = "...")]` and Fluent `to_view` /
  `has_no_key` / `to_schema`. Views are excluded from table DDL; dotted
  schema-qualified names are quoted per part by all dialects/providers.
- **`Ignore`**: `entity.ignore("field")`.
- **`IsConcurrencyToken` / `IsNullable` via Fluent**:
  `.is_concurrency_token()` / `.is_optional()`.
- **`HasIndex(...).IncludeProperties`**: `.include(&[...])` → PostgreSQL
  `INCLUDE (...)`; MySQL/SQLite emit a warning and create the index without it.
- **`#[derive(DbEnum)]`**: string-backed enum conversion equivalent to
  `HasConversion<string>()` — generates `From<Enum> for DbValue`,
  `TryFrom<DbValue> for Enum`, `as_str()` and `from_db_str()`; supports
  `#[db_value("...")]` per variant.

##### Migration / snapshot / automated upgrade

- **Snapshot export helper**: `build_registered_snapshot` / `save_registered_snapshot`
  build a snapshot from inventory-registered entities (the CLI cannot see the
  application model), enabling `rust-ef migration add --snapshot ...`.
- **`DbContext::migrate(dir)`**: EF Core `Database.Migrate()` equivalent —
  loads `up.sql`/`down.sql` pairs and applies pending migrations.
- **Atomic scripts** (PostgreSQL/SQLite): a migration's statements and history
  row run in one transaction; a failure rolls back. MySQL remains statement-by-
  statement because DDL implicitly commits.
- **`generate_from_snapshot`**: the CLI diffs snapshots directly (preserves
  rendered partial-index predicates and other snapshot-only details).
- SQLite upgrade safety: adding/dropping FK or CHECK on an existing table emits
  a `-- WARNING` comment instead of invalid `ALTER TABLE` SQL; `DROP
  COLUMN`/`DROP TABLE` and unsafe `ADD COLUMN ... NOT NULL` (no default) are
  flagged in comments.
- `split_sql_statements` now respects single-quoted literals (`;` inside
  defaults/comments/CHECK expressions) and strips `--` comments.
- Snapshot JSON now round-trips column type, precision/scale, fixed length,
  collation, comments, computed columns, `INCLUDE`, `is_view`, `is_keyless` and
  table comments.
- Diff/DDL emit table-comment changes, computed-column changes (PostgreSQL
  warns to drop/re-add) and per-part quoting for `schema.table`.
- `ensure_deleted` no longer drops views.

##### Notes / compatibility

- `EntityTypeMeta` gained `is_view`, `is_keyless`, `comment`;
  `PropertyMeta` gained `column_type`, `precision`, `scale`, `is_fixed_length`,
  `collation`, `comment`, `computed_sql`, `computed_stored`, `default_value`
  (column defaults now live on the property instead of a side table);
  `SnapshotColumn` / `SnapshotEntityType` / `SnapshotIndex` gained matching
  fields. Manual struct literals need the new fields (or
  `..EntityTypeMeta::default()`).
- `#[derive(DbEnum)]` requires the enum to also derive `Default` (the
  `EntityType` row parser falls back to `Default` for unknown values).
- See the full parity matrix in `docs/rust-ef/03-entity-design/efcore-parity.md`.


#### 3. Composable predicates & the single condition compiler

##### Breaking

- **`linq!` conditions no longer take a `filter` keyword.** `linq!(filter |b: T| ...)` is now `linq!(|b: T| ...)`: a bare condition closure is the predicate form. It returns `Predicate<T>` instead of `BoolExpr`; APIs that take an expression were widened to `impl Into<BoolExpr>` (`DbSet::set_query_filter`, `with_cte_typed` / `with_recursive_cte_typed`) and `ModelBuilder::has_query_filter` takes `impl Into<Predicate<T>>`, so `has_query_filter(linq!(|b: Blog| ...))` also infers `T` now. Code that annotated the result (`let e: BoolExpr = linq!(...)`) must use `.into_expr()`.
- **`QueryBuilder::filter` / `DbSet::filter` take a predicate, not a closure.** The signature is `filter(impl Into<Predicate<T>>)`; hand-written `FnOnce(QueryBuilder<T>) -> QueryBuilder<T>` closures are no longer accepted. `where_` (introduced while this feature was in development, never released) was folded into `filter`.
- **The source-less `=> order` sugar is gone.** `linq!(|b: T| cond => b.col)` now fails with a hint; use `linq!(ctx.set::<T>(), |b: T| cond => b.col)` or the `; order_by b.col` clause. The sugar still works when a source is present.
- **The undocumented type-first forms are gone.** `linq!(Blog, |b| ...)` and `linq!(Blog, b.rating > 5)` are rejected; write `linq!(|b: Blog| ...)` (predicate) or carry an explicit source (`linq!(ctx.set::<Blog>(); ...)`). The error names the correct form.
- **The old method-chain condition compiler is gone.** Every condition is compiled by one boolean compiler and applied through `QueryBuilder::filter` — one AST, one encoder. Removed (internal or `#[doc(hidden)]`): `QueryBuilder::{filter_column, filter_not, filter_in, filter_not_in, filter_is_null, filter_is_not_null, filter_between, filter_like, filter_not_like, or_where, where_exists_internal, where_in_subquery_internal, having_internal}`, `QueryState::filters`, `QueryState::append_filter`, and `compile::{build_where_clauses, build_where_clause_with_offset, filters_to_and_expr}`.
- **SQL shape changes** (semantics unchanged): `!b.flag` emits `NOT (flag = ?)` instead of `flag = ?`; every condition is emitted as one parenthesized tree, so `OR` groups nested on the right of `&&` keep their parentheses (they used to bind to the whole left-hand side).
- **`FilterCondition::new` is `#[doc(hidden)]`.** Conditions that bind values must be built with `FilterCondition::with_values` (or `linq!(|b: T| ...)`); `FilterCondition::new` remains for value-less operators such as `IS NULL`.

##### Added

- **`Predicate<T>` — composable WHERE predicates (EF Core `PredicateBuilder` style).** `and` / `or` / `not`, `always()` / `never()` identity seeds that never leak `1=1` / `1=0` into the SQL, plus `and_opt` / `or_opt` for optional filters and `is_always` / `is_never` for guards. `QueryBuilder::filter` appends a predicate with `AND` (a bare `BoolExpr` is accepted too) and registers its values in tree order, so `count()` and the page query share one condition set. `into_expr()` and the `BoolExpr ⇄ Predicate<T>` conversions bridge APIs that take an expression.
- **Fail-fast on value-less conditions.** Applying a self-contained `BoolExpr` whose parameter count exceeds its carried values (only possible with a hand-built `FilterCondition::new`) panics with a message pointing at `linq!(|b: T| ...)` / `with_values`, instead of handing the driver a `?` with nothing to bind.
- **`execute_update` resolves correlated subqueries** exactly like `to_sql` / `count` / `execute_delete` (it used to emit `FROM ""`), and returns a configuration error when no `set` column was added.

##### Fixed

- **`x.contains(y)` misclassified a variable argument as a column.** `linq!` treated *any* path argument as a column reference, so `b.url.contains(keyword)` compiled to `url IN (<value expression>)` and failed to compile — the common keyword-search pattern. `.contains()` is now `IN` only when the argument is a field reference (`ids.contains(b.id)`) and `LIKE` otherwise.
- **The condition form's `IN` used the receiver as the column.** `linq!(|b: T| ids.contains(b.id))` now emits `b.id IN (...)`, not `ids IN (...)`.
- **`a && (b || c)` lost the grouping.** The condition chain appended an `OR` group with "OR with everything accumulated so far", so an `OR` on the right of `&&` bound to the whole left-hand side (`((a AND b) OR c)`). Such groups are now compiled as a self-contained predicate and `AND`-ed in.
- **An empty `IN` list emitted `col IN ()`**, which is invalid SQL. It now emits `1 = 0` — an empty candidate set matches no row — so optional ID filters can be passed through without a length check.


### 简体中文

#### 1. 数据库级约束与迁移自动化

##### 新增 — 模型层数据库约束

- **声明式索引**：`EntityTypeBuilder::has_index(&[cols])` 现在生成一个复合索引
  （不再退化为多个单列索引）；返回的 `IndexBuilder` 支持 `.is_unique()`、
  `.has_name(name)`、`.has_filter(linq!(|x: T| ...))`（部分索引）。
- **部分索引**：PostgreSQL/SQLite 生成 `... WHERE <predicate>`；MySQL 没有部分
  索引，过滤的 **UNIQUE** 索引用函数索引
  `(CASE WHEN <predicate> THEN <col> END)` 模拟，过滤的普通索引输出 warning
  注释并跳过（而不是生成非法 SQL）。
- **表达式索引**：`has_index_expression(&["coalesce(channel_id, '')"])`（支持
  `.is_unique()`）。
- **复合外键**：`has_foreign_key(&[cols], Ref::TABLE, &[ref_cols])`，可链式
  `.on_delete(...)` / `.has_name(...)`。
- **标量 `#[foreign_key(Target)]` 参与 DDL**：生成指向 `Target` 主键的单列
  FK（此前只生成 `FK_<Target>` 常量）。
- **CHECK 约束**：`entity.has_check(name, sql)` 与
  `property_named(col).has_check(sql)`（自动命名 `CK_<table>_<column>`），在
  `CREATE TABLE` 内联（PG/MySQL/SQLite）。
- **列默认值**：`property_named(col).has_default(sql)` 在 `CREATE TABLE`、
  `ADD COLUMN`、`ALTER COLUMN` 中生成 `DEFAULT`（PG `SET/DROP DEFAULT`，
  MySQL `MODIFY COLUMN`）。
- **MySQL 表选项**：`entity.engine("InnoDB").charset("utf8mb4").collate("utf8mb4_bin")`；
  MySQL DDL 始终显式输出 `ENGINE`/`DEFAULT CHARSET`（默认 `InnoDB`/`utf8mb4`）。
- **`Option<T>` 内层类型映射**：`Option<String>` + `max_length` 现在生成
  `VARCHAR(n)`（MySQL 可建索引），不再退化为 `TEXT`；`Option<Uuid>` /
  `Option<Decimal>` 同样按内层类型映射。
- **迁移差分**覆盖声明式索引、复合外键、CHECK、列默认值与 MySQL 表选项；
  Down 迁移会按旧定义重建被删除的索引/CHECK/FK。
- **SQLite 迁移脚本**把外键内联到 `CREATE TABLE`（SQLite 不支持
  `ALTER TABLE ADD CONSTRAINT`）。

##### 修复

- **rust-ef-mysql**：`cell_to_db_value` 先探测 `uuid::Uuid` 再探测 `String`；
  sqlx-mysql 的 UUID 解码接受任意 16 字节切片，导致 16 字符 `VARCHAR`
  被静默解码成 UUID（36 字符文本）。现在先探测 `String`；UUID 列（`CHAR(36)`）
  通过 `TryFrom<DbValue> for Uuid` 无损还原。
- PostgreSQL `ALTER COLUMN` 现在同步列默认值（`SET DEFAULT` / `DROP DEFAULT`），
  不再保留旧默认值。
- 迁移快照现在完整往返 `max_length`、列默认值、索引、外键、CHECK 与表选项
  （基于 serde_json 的快照 IO）。
- **部分索引过滤条件字面量**（`has_filter(...)`）在各方言正确渲染：MySQL 字符串中的
  反斜杠转义、PostgreSQL `bytea` 改用 `decode('<hex>', 'hex')`（其 `X'..'` 是位串）、
  浮点字面量始终带小数点；非有限浮点、无值条件与子查询节点会被拒绝，而不是生成
  非法 SQL 或静默放宽索引条件。
- **迁移脚本切分**能正确识别引号字面量（`''` 双写与 MySQL `\` 转义）、`--` 行注释、
  `/* */` 块注释与 PostgreSQL `$tag$ … $tag$` 函数体，其中的 `;` 不再截断语句；
  DDL 注释与索引谓词共用同一套按方言转义的字符串字面量（单一实现，无重复转义逻辑）。

##### 注意 / 兼容性

- `EntityTypeMeta` 新增 `indexes`、`foreign_keys`、`checks`、`table_options`；
  手写结构体字面量需补 `..EntityTypeMeta::default()`。
- `SnapshotColumn` 新增 `default_value`；`SnapshotEntityType` 新增
  `indexes` / `foreign_keys` / `checks` / `table_options`。
- `EntityTypeBuilder::has_index` 返回 `IndexBuilder<'_, T>`（作为语句调用的
  现有代码不受影响）。
- `ensure_created` 仍只建表 + 种子，**不生成**独立索引与外键；完整约束请用
  `MigrationEngine::generate` + `apply`。

---

#### 2. EFCore 实体配置对齐

##### 新增 — EntityTypeBuilder / PropertyBuilder 对齐

- **`HasColumnType`**：`#[column_type("...")]` / `.has_column_type("...")` 原样输出列类型
  （`JSON`、`MEDIUMTEXT`、`ENUM(...)` 等）。
- **`HasPrecision`**：`#[precision(p, s)]` / `.has_precision(p, s)`；Decimal →
  `DECIMAL(p,s)`/`NUMERIC(p,s)`，时间类型 → `DATETIME(p)`/`TIMESTAMP(p)`。
- **`IsFixedLength`**：`#[fixed_length]` / `.is_fixed_length()` → `CHAR(n)`。
- **`HasCollation`**：`#[collation("...")]` / `.has_collation("...")`，列级
  `COLLATE` 放在可空性之前。
- **`HasComment`**：列 `#[comment("...")]` / `.has_comment("...")`；表
  `#[table("t", comment = "...")]` / `entity.has_comment("...")`（MySQL 内联/
  ALTER，PG `COMMENT ON`，SQLite 忽略）。
- **`HasComputedColumnSql`**：`#[computed("expr", stored)]` /
  `.has_computed_column_sql("expr").is_stored()` 生成
  `GENERATED ALWAYS AS (...)`；计算列自动从 INSERT/UPDATE 排除。
- **`HasAlternateKey`**：`entity.has_alternate_key(&[cols])` → 唯一索引
  `ak_<table>_<cols>`，可作为外键引用目标。
- **`ToView` / `HasNoKey` / `ToTable(schema)`**：`#[table("v", view)]`、
  `#[keyless]`、`#[table("t", schema = "...")]` 与 Fluent `to_view` /
  `has_no_key` / `to_schema`。视图不参与建表 DDL；`schema.table` 由各方言/
  Provider 分段引用。
- **`Ignore`**：`entity.ignore("field")`。
- **Fluent 并发/可空**：`.is_concurrency_token()` / `.is_optional()`。
- **`HasIndex(...).IncludeProperties`**：`.include(&[...])` → PostgreSQL
  `INCLUDE (...)`；MySQL/SQLite 输出 warning 并忽略 include。
- **`#[derive(DbEnum)]`**：字符串枚举映射（等价 `HasConversion<string>()`），生成
  `From<Enum> for DbValue`、`TryFrom<DbValue> for Enum`、`as_str()`、
  `from_db_str()`，支持 `#[db_value("...")]`。

##### 迁移 / 快照 / 自动升级

- **快照导出助手**：`build_registered_snapshot` / `save_registered_snapshot`
  从 inventory 注册的实体生成快照（CLI 看不到应用模型），打通
  `rust-ef migration add --snapshot ...`。
- **`DbContext::migrate(dir)`**：等价 EF Core `Database.Migrate()`，加载
  `up.sql`/`down.sql` 并应用待执行迁移。
- **脚本原子性**（PostgreSQL/SQLite）：一次迁移的全部语句 + 历史行在同一事务
  执行，失败回滚；MySQL DDL 隐式提交，仍逐条执行。
- **`generate_from_snapshot`**：CLI 直接对快照做 diff（保留已渲染的部分索引
  谓词等仅存在于快照的字段）。
- SQLite 升级安全：对既有表增删 FK/CHECK 输出 `-- WARNING` 注释而不是非法
  `ALTER TABLE`；`DROP COLUMN`/`DROP TABLE` 与无默认值的 `ADD COLUMN ...
  NOT NULL` 会输出警告注释。
- `split_sql_statements` 尊重单引号字符串（默认值/注释/CHECK 中的 `;`），并
  移除 `--` 注释。
- 快照完整往返列类型、精度/标度、定长、排序规则、注释、计算列、INCLUDE、
  `is_view`、`is_keyless` 与表注释。
- 差分/DDL 支持表注释变更、计算列变更（PostgreSQL 输出 drop/re-add 警告）与
  `schema.table` 分段引用。
- `ensure_deleted` 不再删除视图。

##### 注意 / 兼容性

- `EntityTypeMeta` 新增 `is_view`、`is_keyless`、`comment`；`PropertyMeta` 新增
  `column_type`、`precision`、`scale`、`is_fixed_length`、`collation`、`comment`、
  `computed_sql`、`computed_stored`、`default_value`（列默认值改挂到属性上，移除
  侧表 `column_defaults`）；`SnapshotColumn` / `SnapshotEntityType` /
  `SnapshotIndex` 同步新增字段；手写结构体字面量需补齐（或
  `..EntityTypeMeta::default()`）。
- `#[derive(DbEnum)]` 要求枚举同时 `derive(Default)`（行解析兜底）。
- 完整对照见 `docs/rust-ef/03-entity-design/efcore-parity.md`。

---


#### 3. 可组合谓词与单一条件编译器

##### 破坏性变更

- **`linq!` 条件形式不再需要 `filter` 关键字。** `linq!(filter |b: T| ...)` 改为 `linq!(|b: T| ...)`：裸条件闭包就是谓词形式。它返回 `Predicate<T>` 而不是 `BoolExpr`；接收表达式的 API 放宽为 `impl Into<BoolExpr>`（`DbSet::set_query_filter`、`with_cte_typed` / `with_recursive_cte_typed`），`ModelBuilder::has_query_filter` 改为接收 `impl Into<Predicate<T>>`，因此 `has_query_filter(linq!(|b: Blog| ...))` 还能自动推断 `T`。显式标注过类型的代码（`let e: BoolExpr = linq!(...)`）改为 `.into_expr()`。
- **`QueryBuilder::filter` / `DbSet::filter` 接收谓词而不是闭包。** 签名为 `filter(impl Into<Predicate<T>>)`；手写的 `FnOnce(QueryBuilder<T>) -> QueryBuilder<T>` 闭包不再接受。开发期引入、未发布的 `where_` 已并入 `filter`。
- **无 source 的 `=> order` 糖移除。** `linq!(|b: T| cond => b.col)` 会给出提示并报错；改用 `linq!(ctx.set::<T>(), |b: T| cond => b.col)` 或 `; order_by b.col` 子句。带 source 时该糖照旧可用。
- **未文档化的「类型在前」形式移除。** `linq!(Blog, |b| ...)` 与 `linq!(Blog, b.rating > 5)` 现在报错；请写 `linq!(|b: Blog| ...)`（谓词）或带显式 source（`linq!(ctx.set::<Blog>(); ...)`）。错误信息会给出正确写法。
- **旧的方法链条件编译器已移除。** 所有条件都由同一个布尔编译器编译、经 `QueryBuilder::filter` 应用——一份 AST、一个编码器。删除（内部或 `#[doc(hidden)]`）：`QueryBuilder::{filter_column, filter_not, filter_in, filter_not_in, filter_is_null, filter_is_not_null, filter_between, filter_like, filter_not_like, or_where, where_exists_internal, where_in_subquery_internal, having_internal}`、`QueryState::filters`、`QueryState::append_filter`，以及 `compile::{build_where_clauses, build_where_clause_with_offset, filters_to_and_expr}`。
- **SQL 形态变化**（语义不变）：`!b.flag` 生成 `NOT (flag = ?)` 而非 `flag = ?`；所有条件按表达式树整体加括号，`&&` 右侧的 `OR` 组不再被提升到整条条件。
- **`FilterCondition::new` 标记为 `#[doc(hidden)]`。** 带绑定值的条件请用 `FilterCondition::with_values`（或 `linq!(|b: T| ...)`）；`FilterCondition::new` 仅保留给 `IS NULL` 这类无值算子。

##### 新增

- **`Predicate<T>`：可组合的 WHERE 谓词（对齐 EF Core `PredicateBuilder`）。** 提供 `and` / `or` / `not`、`always()` / `never()` 恒等起点（不会把 `1=1` / `1=0` 写入 SQL），以及可选条件的 `and_opt` / `or_opt` 与状态判断 `is_always` / `is_never`。`QueryBuilder::filter` 以 `AND` 追加谓词（也接受裸 `BoolExpr`），按表达式树顺序登记绑定值，因此 `count()` 与列表查询共用同一套条件。`into_expr()` 与 `BoolExpr ⇄ Predicate<T>` 互转负责与接收表达式的 API 衔接。
- **无值条件 fail-fast。** 自包含 `BoolExpr` 的参数个数大于其携带值个数时（只可能由手写 `FilterCondition::new` 造成），`apply_query_filter` 直接带明确信息 panic，而不是把一个没有值的 `?` 交给驱动。
- **`execute_update` 解析相关子查询**，与 `to_sql` / `count` / `execute_delete` 一致（此前会生成 `FROM ""`）；未添加 `set` 列时返回配置错误。

##### 修复

- **`x.contains(y)` 把变量参数误判成列名。** `linq!` 把任何路径参数都当成列引用，导致 `b.url.contains(keyword)` 编译成 `url IN (<值表达式>)` 而编译失败——这正是最常见的关键字搜索写法。现在只有参数是字段引用（`ids.contains(b.id)`）时才生成 `IN`，否则生成 `LIKE`。
- **条件形式的 `IN` 用错了列。** `linq!(|b: T| ids.contains(b.id))` 现在生成 `b.id IN (...)`，而不是 `ids IN (...)`。
- **`a && (b || c)` 丢失括号分组。** 条件链用「与已有条件整体 OR」来拼 `OR` 组，导致 `&&` 右侧的 `OR` 组绑到了整个左侧（`((a AND b) OR c)`）。这类分组现在编译成自包含谓词再 `AND` 进来。
- **空的 `IN` 列表会生成 `col IN ()`**（非法 SQL）。现在生成 `1 = 0`——空候选集不匹配任何行——可选 ID 过滤条件可以直接传入，不必先判空。

## [2.1.0] — 2026-11-11 — rust-dix 0.10：MEDI scope、类型化 key、动态上下文与异步初始化

> **English** · **简体中文**

### English

#### Breaking

- **DI dependency**: `rust-dix 0.7` → `0.10`. rust-ef 2.1 requires `rust-dix ^0.10`;
  the 0.7–0.9 container types are not compatible, so applications must upgrade in
  lockstep (0.9 → 0.10 is source-compatible).
- **Strict scope validation** (MEDI `ValidateScopes`, default since 0.8): resolving
  the Scoped `DbContext` from the root provider returns
  `RdiError::ScopedFromRootProvider`. Use `provider.scope()` for shared
  resolution, or owned resolution (`get_owned` / `get_keyed_owned` /
  `#[inject(owned)]`), which remains allowed from the root because the instance
  is caller-owned. Temporary escape hatch:
  `ServiceCollection::build_with(ServiceProviderOptions::lenient())`.
- **MEDI registration semantics**: last-registration-wins; `get_all` excludes
  keyed registrations (rust-ef does not rely on it). `try_add_*` keeps `TryAdd`.
- **`RdiError` is `#[non_exhaustive]`** (new variants include
  `ScopedFromRootProvider`, `ProviderDisposed`, `ScopeDisposed`,
  `SyncSingletonDependsOnAsyncService`, `AsyncServiceRequiresGetAsync`).
- **MSRV**: Rust **1.95** (rust-dix 0.10 baseline).
- **`add_dbcontext_keyed` signature**: now accepts
  `impl IDbContextKey + Into<ServiceKey>`. Typed enum keys register a typed
  `ServiceKey`; `&str`/`String` keys remain source-compatible and register as
  string keys. Explicit function pointers or parameter type annotations may need
  adjusting.

#### Added — DI / context features

- **Typed context keys**: `IDbContextKey` plus `DbContextResolver`
  (`get_owned_context` / `get_context`) give compile-time-safe keyed
  registration/resolution while the entity model keeps its stable string identity
  (`#[context("...")]`). `#[derive(DbContextKey)]` implements the official
  rust-dix 0.10 extension contract `From<Key> for ServiceKey`, so rust-dix 0.10
  macro key expressions work end-to-end:
  `#[inject(owned, key = Db::Read)]` resolves the same typed key used by
  `add_dbcontext_keyed(Db::Read, ...)`.
- **Dynamic keyed contexts (rust-dix `AnyKey`)**: `add_dbcontext_anykey(|key,
  options| ...)` registers one fallback for every requested key. Options and the
  connection pool are cached per requested key in a **bounded LRU** (default 256
  keys; `add_dbcontext_anykey_with_capacity` overrides it), reused across scopes;
  exact-key registrations take precedence; AnyKey contexts use the default entity
  model (tenant isolation belongs in query filters). Security: map requested keys
  through an allow-list/tenant table; never interpolate them into SQL or
  connection strings.
- **Async provider initialization**: `add_dbcontext_async` and
  `add_dbcontext_keyed_async` defer provider creation to `build_async().await`
  and surface failures as typed `RdiError` instead of a registration-time panic.
  An optional `DbContextOptionsBuilder::async_preflight` hook runs after the pool
  is created (e.g. async `SELECT 1`) and aborts startup on error. The `DbContext`
  remains a synchronous Scoped service, so owned injection is unchanged; a sync
  `build()` with async registrations returns
  `RdiError::AsyncServiceRequiresBuildAsync`.
- `rust_ef::di` re-exports `ServiceProviderOptions` and `ServiceKey`; all library
  crates opt into `#![forbid(unsafe_code)]` and the repository ships `SECURITY.md`.

#### Added — engineering / security baseline

- CI: RustSec `cargo audit` (replaces the `rustsec/audit-check` action), MSRV
  1.95 job, `rustdoc -D warnings`, and a single-`rust-dix`-version check.
- Publish workflow: version/tag preflight, fmt/clippy/tests/package checks,
  RustSec audit gate, dependency-order publishing with idempotent retries and
  crates.io visibility waits.
- Tests: typed keys (derive + macro `key = Expr` handler injection) and
  entity-model selection, AnyKey per-key isolation/caching, bounded-LRU eviction
  and exact-key precedence, async registration/preflight and typed failure paths,
  `String`/`ServiceKey` compatibility, strict/lenient scope, disposal.
- Docs: crate READMEs rewritten for the current API, `docs/v2.1-migration-guide.md`,
  `docs/rust-dix-0.10-adoption.md`, release checklist, rust-dix 0.10 handbook notes.

#### Migration from 2.0

- Upgrade `rust-dix` to 0.10 and the toolchain to Rust 1.95.
- Replace root `provider.get::<DbContext>()` with `provider.scope()` (shared) or
  `get_owned()` (owned); see `docs/v2.1-migration-guide.md`.
- Typed keys (use `#[derive(DbContextKey)]`), AnyKey and async registration are
  additive; existing string-keyed registration, owned handler patterns and query
  behavior are unchanged.

### 简体中文

#### 破坏性变更

- **DI 依赖**：`rust-dix 0.7` → `0.10`。rust-ef 2.1 要求 `rust-dix ^0.10`；
  0.7–0.9 的容器类型互不兼容，应用必须同步升级（0.9→0.10 源码兼容）。
- **严格 scope 校验**（MEDI `ValidateScopes`，0.8 起默认）：从根 provider 解析
  Scoped `DbContext` 返回 `RdiError::ScopedFromRootProvider`。共享解析请在
  `provider.scope()` 内进行；owned 解析（`get_owned` / `get_keyed_owned` /
  `#[inject(owned)]`）仍可从根解析（实例归调用方所有）。迁移期逃生门：
  `ServiceCollection::build_with(ServiceProviderOptions::lenient())`。
- **MEDI 注册语义**：后注册优先；`get_all` 不再包含 keyed 注册（rust-ef 不依赖）；
  `try_add_*` 保持 `TryAdd` 语义。
- **`RdiError` 标记 `#[non_exhaustive]`**（新增 `ScopedFromRootProvider`、
  `ProviderDisposed`、`ScopeDisposed`、`SyncSingletonDependsOnAsyncService`、
  `AsyncServiceRequiresGetAsync` 等变体）。
- **MSRV**：Rust **1.95**（rust-dix 0.10 基线）。
- **`add_dbcontext_keyed` 签名**：改为接受 `impl IDbContextKey + Into<ServiceKey>`。
  typed enum key 注册为 typed `ServiceKey`；`&str`/`String` 保持源码兼容（仍是 string key）。
  显式函数指针/参数类型标注可能需要调整。

#### 新增 —— DI / 上下文能力

- **类型化 context key**：`IDbContextKey` + `DbContextResolver`
  （`get_owned_context` / `get_context`），keyed 注册/解析获得编译期安全，实体模型
  仍使用稳定字符串（`#[context("...")]`）。`#[derive(DbContextKey)]` 实现 rust-dix 0.10
  官方扩展契约 `From<Key> for ServiceKey`，因此 0.10 的宏 key 表达式可端到端使用：
  `#[inject(owned, key = Db::Read)]` 与 `add_dbcontext_keyed(Db::Read, ...)` 使用同一个
  typed key。
- **动态 keyed 上下文（rust-dix `AnyKey`）**：
  `add_dbcontext_anykey(|key, options| ...)` 为一个兜底注册服务任意 key；options 与
  连接池按请求 key 缓存（有界 LRU，默认 256 个 key，
  `add_dbcontext_anykey_with_capacity` 可调），跨 scope 复用；精确 key 注册优先；
  AnyKey 上下文使用默认实体模型（租户隔离由查询过滤器负责）。安全提示：请求 key 必须
  经白名单/租户表映射，禁止直接拼入 SQL 或连接串。
- **异步 provider 初始化**：`add_dbcontext_async` 与
  `add_dbcontext_keyed_async` 把 provider 创建推迟到 `build_async().await`，失败以
  类型化 `RdiError` 返回（不再在注册期 panic）；可选
  `DbContextOptionsBuilder::async_preflight` 在连接池创建后执行异步自检，失败中止启动。
  `DbContext` 仍是同步 Scoped 服务，owned 注入方式不变；同步 `build()` 遇到异步注册
  返回 `RdiError::AsyncServiceRequiresBuildAsync`。
- `rust_ef::di` 重新导出 `ServiceProviderOptions` 与 `ServiceKey`；全部库 crate 启用
  `#![forbid(unsafe_code)]`，仓库新增 `SECURITY.md`。

#### 新增 —— 工程与安全基线

- CI：RustSec `cargo audit`（替代 `rustsec/audit-check` action）、MSRV 1.95 job、
  `rustdoc -D warnings`、单一 rust-dix 版本检查。
- 发布工作流：版本/tag preflight、fmt/clippy/test/package 检查、RustSec 审计门禁、
  依赖顺序发布 + 幂等重试 + crates.io 可见性等待。
- 测试：类型化 key（派生 + Handler 宏 key 表达式注入）与实体模型选择、AnyKey
  隔离/缓存/有界 LRU 淘汰/精确 key 优先级、异步注册/async preflight 与类型化失败路径、
  `String`/`ServiceKey` 兼容、严格/lenient scope、释放。
- 文档：各 crate README 按当前 API 重写、`docs/v2.1-migration-guide.md`、
  `docs/rust-dix-0.10-adoption.md`、发布清单、rust-dix 0.10 手册说明。

#### 从 2.0 迁移

- 升级 `rust-dix` 到 0.10、工具链到 Rust 1.95。
- 根解析 `provider.get::<DbContext>()` 改为 `provider.scope()`（共享）或
  `get_owned()`（owned）；详见 `docs/v2.1-migration-guide.md`。
- 类型化 key（使用 `#[derive(DbContextKey)]`）、AnyKey、异步注册均为新增能力；既有字符串
  key 注册、owned Handler 模式与查询行为不变。

## [2.0.1] — 2026-11-09 — Database driver security upgrade

### English

#### Security

- **MySQL driver: sqlx 0.8.0 → 0.9.0.** Fixes RUSTSEC-2024-0363 / GHSA-xmrp-424f-vfpx (binary-protocol length truncation could let an oversized value be reinterpreted as protocol commands).
- **SQLite driver: rusqlite 0.31 → 0.41.1 (libsqlite3-sys 0.39.1).** The bundled engine is SQLite 3.54.0, fixing RUSTSEC-2026-0459 / CVE-2026-119375 (uninitialized page-cache memory when a mismatched `-wal` sibling is opened).
- **RSA auth backend upgraded to `rsa` 0.10.0.** sqlx 0.9 makes RSA password auth opt-in; the provider enables the `rsa` feature so MySQL 8's `caching_sha2_password` keeps working over plaintext (local dev/tests). `sqlx-mysql 0.9` accepts `rsa >= 0.10.0-rc.18`, which resolves to the fixed `rsa 0.10.0`; RUSTSEC-2023-0071 (Marvin timing attack) no longer applies.
- **Removed unmaintained `paste`** (RUSTSEC-2024-0436) by dropping the sqlx umbrella crate.
- Audit result: **0 advisories across 290 locked packages** (previously 5 advisories across 330 packages, including `libsqlite3-sys 0.28` and `sqlx 0.8.0`).

#### Changed

- The MySQL provider now depends on `sqlx-core` / `sqlx-mysql` 0.9 directly instead of the `sqlx` umbrella. The umbrella resolves its optional SQLite driver into the lock graph even with `default-features = false`, which conflicts with rusqlite's `libsqlite3-sys 0.39` over `links = "sqlite3"`; depending on the driver crate keeps both providers on their fixed native libraries and removes unused drivers from the build.
- Enabled the `rsa` backend on `sqlx-mysql` (resolves to `rsa 0.10.0`) so plaintext MySQL 8 authentication via `caching_sha2_password` is not silently lost; TLS remains the secure-by-default transport.
- Provider-generated dynamic SQL is wrapped in `AssertSqlSafe` (sqlx 0.9 requirement); runtime values remain bound parameters, identifiers still come from compile-time entity metadata.
- The SQLite provider now ships a small `r2d2::ManageConnection` implementation in `pool_strategy.rs` and no longer depends on `r2d2_sqlite`. The latest `r2d2_sqlite` (0.35) pins rusqlite 0.40 / libsqlite3-sys 0.38, which is affected by RUSTSEC-2026-0459; the manager only opens the file and leaves WAL/busy-timeout setup to the existing customizer.
- PostgreSQL/MySQL integration tests now panic when a configured `RUST_EF_PG_URL` / `RUST_EF_MYSQL_URL` cannot be reached instead of printing `skip` and returning. A broken CI service container can no longer produce a false-green test job.
- PostgreSQL provider dependencies were already current and are unchanged: `tokio-postgres 0.7.18`, `deadpool-postgres 0.14.2`, `postgres-native-tls 0.5.3`, `native-tls 0.2.18`.

#### Compatibility

- `MySqlProvider::from_pool` and `DbContextOptionsBuilderExt::use_mysql_with_pool` now use `sqlx-mysql 0.9`'s `MySqlPool` / `MySqlPoolOptions`. If an application builds a pool with `sqlx 0.8` and passes it to these optional APIs, align the application's `sqlx` dependency to 0.9. No other public API or query behavior changed.
- New MSRV floor inherited from sqlx 0.9: Rust 1.94; the workspace and all crates now declare `rust-version = "1.94"`.

#### Verified

- `cargo audit --file Cargo.lock`: 0 vulnerabilities (RustSec advisory DB, 1429 advisories).
- `cargo test --workspace --all-features -- --skip postgres --skip mysql`: 370 passed, 0 failed, 18 ignored.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: 0 warnings.
- `cargo fmt --all -- --check`: pass.
- `cargo bench --workspace --no-run`: all benchmarks compile.

### 简体中文

#### 安全

- **MySQL 驱动：sqlx 0.8.0 → 0.9.0。** 修复 RUSTSEC-2024-0363 / GHSA-xmrp-424f-vfpx（二进制协议长度截断，超大值可能被解释为协议命令）。
- **SQLite 驱动：rusqlite 0.31 → 0.41.1（libsqlite3-sys 0.39.1）。** 内置 SQLite 3.54.0，修复 RUSTSEC-2026-0459 / CVE-2026-119375（打开页大小不匹配的 `-wal` 伴随文件时可能返回未初始化的页缓存内存）。
- **RSA 后端从 `rsa` 0.9 升级到 `rsa` 0.10.0。** sqlx 0.9 将 RSA 密码认证改为可选；provider 启用 `rsa` feature，使 MySQL 8 的 `caching_sha2_password` 在明文连接（本地开发/测试）下继续可用。`sqlx-mysql 0.9` 接受 `rsa >= 0.10.0-rc.18`，实际解析到已修复的 `rsa 0.10.0`；RUSTSEC-2023-0071（Marvin 时序攻击）不再适用。
- **移除不再维护的 `paste`**（RUSTSEC-2024-0436），不再依赖 sqlx umbrella。
- 审计结果：**290 个锁定依赖 0 个漏洞**（升级前 330 个依赖中 5 个漏洞，含 `libsqlite3-sys 0.28` 与 `sqlx 0.8.0`）。

#### 变更

- MySQL provider 直接从 `sqlx-core` / `sqlx-mysql` 0.9 取驱动，不再使用 `sqlx` umbrella。umbrella 即使关闭默认 feature 也会把可选 SQLite 驱动纳入锁定图，与 rusqlite 的 `libsqlite3-sys 0.39` 在 `links = "sqlite3"` 上冲突；直接依赖驱动 crate 可让两个 provider 各自使用已修复的原生库，并把无用驱动移出构建。
- 启用 `sqlx-mysql` 的 `rsa` 后端（解析到 `rsa 0.10.0`），避免明文 MySQL 8 `caching_sha2_password` 认证能力静默丢失；TLS 仍是默认的安全传输方式。
- provider 生成的动态 SQL 使用 `AssertSqlSafe` 包装（sqlx 0.9 要求）；运行时值仍走绑定参数，标识符仍来自编译期实体元数据。
- SQLite provider 在 `pool_strategy.rs` 内实现了一个很小的 `r2d2::ManageConnection`，不再依赖 `r2d2_sqlite`。最新 `r2d2_sqlite`（0.35）仍锁定 rusqlite 0.40 / libsqlite3-sys 0.38（受 RUSTSEC-2026-0459 影响）；新 manager 只负责打开文件，WAL / busy-timeout 仍由现有 customizer 设置。
- PostgreSQL/MySQL 集成测试在配置了 `RUST_EF_PG_URL` / `RUST_EF_MYSQL_URL` 但连接失败时改为 panic，不再打印 `skip` 后返回；CI 数据库服务异常不会再产生假绿。
- PostgreSQL provider 依赖本已是最新，保持不变：`tokio-postgres 0.7.18`、`deadpool-postgres 0.14.2`、`postgres-native-tls 0.5.3`、`native-tls 0.2.18`。

#### 兼容性

- `MySqlProvider::from_pool` 与 `DbContextOptionsBuilderExt::use_mysql_with_pool` 现在使用 `sqlx-mysql 0.9` 的 `MySqlPool` / `MySqlPoolOptions`。如果应用用 `sqlx 0.8` 构建连接池并传给这两个可选 API，请把应用的 `sqlx` 依赖对齐到 0.9。其余公共 API 与查询行为不变。
- 继承 sqlx 0.9 的 MSRV：Rust 1.94；workspace 与所有 crate 现已声明 `rust-version = "1.94"`。

#### 验证

- `cargo audit --file Cargo.lock`：0 漏洞（RustSec 1429 条公告库）。
- `cargo test --workspace --all-features -- --skip postgres --skip mysql`：370 通过，0 失败，18 忽略。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：0 warning。
- `cargo fmt --all -- --check`：通过。
- `cargo bench --workspace --no-run`：全部基准编译通过。

## [2.0.0] — 2026-10-22 — EF Core-style transactions

### English

#### Breaking

- **`begin_transaction` is now context-scoped**: it returns an `ITransaction` and registers the transaction on the `DbContext`, matching EF Core's `IDbContextTransaction` / `CurrentTransaction` model. Queries, navigation loading, bulk DML, raw SQL and `save_changes()` all join it; nothing is committed until `commit()` is called. The previous standalone behavior (queries and `save_changes()` ignored the handle) is removed.
- **Removed `DbContext::transaction` / `transaction_with`**: use `begin_transaction` / `begin_transaction_with` and explicit `commit()` / `rollback()`, or `use_transaction` for the `Ok`-commit / `Err`-rollback closure form.
- **Removed `TxOutcome`**: a business rollback now calls `txn.rollback().await?` and returns the business value directly. This removes the extra three-state transaction concept.
- **`ITransaction::is_scoped` and `DbTransaction::new` removed**: all transactions returned by `DbContext` are scoped; `DbTransaction` handles are created only by the context.
- **`DbContext::transaction_mut` removed**: use the handle returned by `begin_transaction` for savepoints/isolation; `use_transaction` no longer exposes an ambient handle.
- **`IAsyncConnection::discard` added**: custom providers must implement it to close a connection (forcing DB rollback) when a transaction is dropped unfinished or commit/rollback fails.
- **`save_changes()` inside `begin_transaction` no longer self-commits**: it flushes to the active transaction and uses a savepoint, like `use_transaction`.

#### Reliability

- **Drop never returns an open transaction to the pool**: dropping an unfinished transaction synchronously discards the connection when idle (MySQL/PostgreSQL close it; SQLite runs ROLLBACK); if a statement is in flight, a background task rolls back and discards. `SharedTxn::drop` is a final fallback when the runtime is gone.
- **Failed COMMIT/ROLLBACK discards the connection**: the transaction state may be unknown after an error, so the connection is closed instead of being recycled.
- **Drop captures the runtime**: a transaction handle stores the Tokio runtime at begin time, so dropping it outside a Tokio context can still schedule the busy-path rollback (best effort; `panic = "abort"` cannot run `Drop`).

#### Migration from 1.x

```rust
// 1.x
let result = ctx.transaction(|ctx| Box::pin(async move {
    // ...
    Ok(TxOutcome::Rollback(BizResult::Rejected))
})).await?;

// 2.0
let txn = ctx.begin_transaction().await?;
// ...
txn.rollback().await?;
ctx.clear_tracking();
let result = BizResult::Rejected;
```

- Replace `transaction` with `begin_transaction` + `commit` / `rollback`.
- Replace `transaction_with(level, ...)` with `begin_transaction_with(level)`.
- Replace `TxOutcome::Commit(v)` with `txn.commit().await?` then return `v`.
- Replace `TxOutcome::Rollback(v)` with `txn.rollback().await?`, `ctx.clear_tracking()`, then return `v`.
- Existing `use_transaction` code continues to work: `Ok` commits, `Err` rolls back.

### 简体中文

#### 破坏性变更

- **`begin_transaction` 改为上下文作用域**：返回 `ITransaction`，并把事务注册到 `DbContext`，与 EF Core 的 `IDbContextTransaction` / `CurrentTransaction` 对齐。查询、导航加载、批量 DML、raw SQL 与 `save_changes()` 都会加入；只有调用 `commit()` 才提交。旧的 standalone 行为（查询和 `save_changes()` 不加入句柄）已移除。
- **移除 `DbContext::transaction` / `transaction_with`**：改用 `begin_transaction` / `begin_transaction_with` 显式 `commit()` / `rollback()`，或保留 `use_transaction` 的 `Ok` 提交 / `Err` 回滚闭包形式。
- **移除 `TxOutcome`**：业务回滚改为 `txn.rollback().await?` 后直接返回业务值，去掉额外的三态事务概念。
- **移除 `ITransaction::is_scoped` 与 `DbTransaction::new`**：`DbContext` 返回的事务全部是作用域事务；`DbTransaction` 句柄只由 context 创建。
- **移除 `DbContext::transaction_mut`**：savepoint/isolation 直接使用 `begin_transaction` 返回的句柄；`use_transaction` 不再暴露 ambient 句柄。
- **新增 `IAsyncConnection::discard`**：自定义 provider 必须实现，用于事务未完成即 drop 或 commit/rollback 失败时关闭连接、强制数据库回滚。
- **`begin_transaction` 内的 `save_changes()` 不再自行提交**：只 flush 到当前事务并通过 savepoint 保护，与 `use_transaction` 一致。

#### 可靠性

- **未完成事务的 Drop 不会把开启的事务放回连接池**：连接空闲时同步 `discard`（MySQL/PostgreSQL 关闭连接，SQLite 执行 ROLLBACK）；有语句正在使用时，后台任务 rollback + discard。`SharedTxn::drop` 是 runtime 已关闭时的最终兜底。
- **COMMIT/ROLLBACK 失败会 discard 连接**：错误后事务状态可能不确定，连接直接关闭，不复用。
- **Drop 捕获 runtime**：事务句柄在 begin 时保存 Tokio runtime，因此在非 Tokio 上下文 drop 仍可调度忙路径回滚（best effort；`panic = "abort"` 无法执行 Drop）。

#### 从 1.x 迁移

```rust
// 1.x
let result = ctx.transaction(|ctx| Box::pin(async move {
    // ...
    Ok(TxOutcome::Rollback(BizResult::Rejected))
})).await?;

// 2.0
let txn = ctx.begin_transaction().await?;
// ...
txn.rollback().await?;
ctx.clear_tracking();
let result = BizResult::Rejected;
```

- `transaction` → `begin_transaction` + `commit` / `rollback`；
- `transaction_with(level, ...)` → `begin_transaction_with(level)`；
- `TxOutcome::Commit(v)` → `txn.commit().await?` 后返回 `v`；
- `TxOutcome::Rollback(v)` → `txn.rollback().await?`、`ctx.clear_tracking()` 后返回 `v`；
- 原有 `use_transaction` 代码保持可用：`Ok` 提交，`Err` 回滚。

## [1.9.0] — 2026-10-21 — Transaction completeness & keyed read/write separation

### English

#### Fixed

- **Transaction scope**: statements issued inside `use_transaction` now share the transaction's connection. Previously only the DML executed by `save_changes` joined the ambient transaction; high-level queries, navigation/lazy loading, `sql_query`, `execute_update` and `execute_delete` acquired separate pooled connections. As a result, a query could not observe the handler's uncommitted writes, and bulk DML committed immediately outside the transaction (so it could not be rolled back).
- **Savepoint per scoped save**: `save_changes()` inside `use_transaction` / `transaction` wraps its DML in a savepoint and rolls back to it on failure, leaving the outer transaction usable (matching EF Core). Standalone `begin_transaction` handles keep the legacy behavior.
- **No-op save fast path**: `save_changes()` with nothing pending returns `{ added: 0, updated: 0, deleted: 0 }` without opening a transaction or savepoint.
- **SQLite pool acquisition**: pooled connection acquisition now runs on `spawn_blocking`, so a saturated write pool does not block Tokio workers. The default acquisition timeout remains 30s (r2d2's historical default) and is configurable via `SqliteOptions::pool_acquire_timeout`.
- **MySQL text protocol**: transaction control (`START TRANSACTION`, `SET TRANSACTION`, `COMMIT`, `ROLLBACK`, savepoints) and DDL now use `sqlx::raw_sql`; MySQL 8.4 rejects these statements through the prepared-statement protocol (error 1295).
- **Migration primary keys**: primary-key columns are always emitted as `NOT NULL` (SQL primary keys cannot be nullable), and MySQL `AUTO_INCREMENT` is placed after the nullability clause. Fixes MySQL error 1171 and PostgreSQL `SERIAL NULL` primary keys.
- **Aggregate decoding**: MySQL `COUNT(*)`/`BIGINT` values are decoded as `i64` before the bool candidate, and PostgreSQL row conversion tries the actual Rust type instead of trusting a column OID, avoiding `Row::get` panics on integer/aggregate columns.
- **Numeric parameters**: MySQL only decodes `TINYINT` as bool (so IDs are not corrupted), and PostgreSQL numeric parameters adapt to the server-inferred type, so `double precision` columns can be compared with integer literals.

#### Added

- **`DbContext::transaction` + `TxOutcome`**: handler-level unit of work. The closure returns `TxOutcome::Commit(value)` or `TxOutcome::Rollback(value)`; both yield `Ok(value)`, while `Err` still rolls back and propagates. `Rollback` also detaches tracked entities.
- **`DbContext::clear_tracking`**: detaches every tracked entity and clears the change tracker.
- **Drop-rolls-back**: dropping a transaction handle without committing rolls it back on a background task (best effort; requires a Tokio runtime).
- **Isolation at transaction start**: `begin_transaction_with` / `use_transaction_with` / `transaction_with` apply the isolation level before the first statement. MySQL sets it before `START TRANSACTION`; PostgreSQL applies it after `BEGIN` but before any query; SQLite accepts only `Serializable`.
- **Options APIs for all providers**: `use_sqlite_with(path, |o| ...)`, `use_mysql_with(cs, |o| ...)`, `use_postgres_with(cs, |o| ...)`. Each delegate configures a provider-specific options builder (pool size, TLS, SQLite acquisition timeout), so new features can be added without more `use_*` methods. Existing `use_*` / `use_*_with_pool` / `use_*_with_tls` methods are unchanged.
- **Keyed shared entity model**: `use_default_entity_model()` lets read/write replica contexts share the default entity model instead of being filtered by `#[context(key)]`.

#### Changed

- **`use_transaction` behavior**: queries, navigation loading, `sql_query`, `execute_update` and `execute_delete` now join the ambient transaction. `begin_transaction` remains a standalone handle (queries and `save_changes()` do not join it) for backward compatibility; use `transaction` for a context-scoped unit of work.
- **Isolation semantics**: `set_isolation` keeps its legacy provider-specific behavior (SQLite PRAGMA, MySQL `SET TRANSACTION`, PostgreSQL). The new `*_with` entry points are the portable, validated way to set isolation at transaction start.
- **Read/write separation recipe**: documented as two keyed `DbContext`s (EF Core style); no `use_*_read_write` API. Pool sizing for the write context uses the provider's delegate (e.g. `use_sqlite_with(..., |o| o.max_pool_size(1))`, `use_mysql_with(..., |o| o.max_pool_size(1))`, `use_postgres_with(..., |o| o.pool_size(1))`).

### 简体中文

#### 修复

- **事务覆盖范围**：`use_transaction` 内的所有语句现在共享事务连接。此前只有 `save_changes` 执行的 DML 加入环境事务；查询、导航/懒加载、`sql_query`、`execute_update` 与 `execute_delete` 会各自从连接池取新连接，导致查询读不到本事务未提交的写，批量 DML 也会立即提交、无法随事务回滚。
- **作用域内每次 save 使用 savepoint**：`use_transaction` / `transaction` 内的 `save_changes()` 会把本次 DML 包在 savepoint 中，失败时回滚到该 savepoint，外层事务仍可继续（与 EF Core 一致）。独立的 `begin_transaction` 句柄保持旧行为。
- **空保存快速路径**：没有待保存变更时，`save_changes()` 直接返回 `{ added: 0, updated: 0, deleted: 0 }`，不开启事务或 savepoint。
- **SQLite 池获取**：连接池获取改为 `spawn_blocking`，写池饱和时不会阻塞 Tokio worker；默认超时保持 r2d2 历史的 30s，可通过 `SqliteOptions::pool_acquire_timeout` 配置。
- **MySQL 文本协议**：事务控制（`START TRANSACTION`、`SET TRANSACTION`、`COMMIT`、`ROLLBACK`、savepoint）与 DDL 改用 `sqlx::raw_sql`；MySQL 8.4 的预处理协议会拒绝这些语句（错误 1295）。
- **迁移主键**：主键列始终生成 `NOT NULL`（SQL 主键不能为 NULL），MySQL 的 `AUTO_INCREMENT` 放在可空性之后。修复 MySQL 错误 1171 与 PostgreSQL 的 `SERIAL NULL` 主键。
- **聚合值解码**：MySQL 的 `COUNT(*)`/`BIGINT` 先按 `i64` 解码，再尝试 bool；PostgreSQL 行转换按实际 Rust 类型尝试，而不是只信任列 OID，避免整数/聚合列触发 `Row::get` panic。
- **数值参数**：MySQL 只有 `TINYINT` 才按 bool 解码（避免 ID 被破坏）；PostgreSQL 数值参数按服务端推断的类型适配，`double precision` 列可以和整数字面量比较。

#### 新增

- **`DbContext::transaction` + `TxOutcome`**：以 handler 为单位的工作单元。闭包返回 `TxOutcome::Commit(value)` 或 `TxOutcome::Rollback(value)`，两者都向外返回 `Ok(value)`；`Err` 仍会回滚并传播。`Rollback` 同时 detach 所有跟踪实体。
- **`DbContext::clear_tracking`**：detach 所有跟踪实体并清空 ChangeTracker。
- **Drop 自动回滚**：未提交就 drop 的事务句柄会在后台任务中回滚（best effort，需要 Tokio runtime）。
- **事务开始时指定隔离级别**：`begin_transaction_with` / `use_transaction_with` / `transaction_with` 在首条语句前应用隔离级别。MySQL 在 `START TRANSACTION` 前设置；PostgreSQL 在 `BEGIN` 后、首条查询前设置；SQLite 只接受 `Serializable`。
- **所有 provider 的 options API**：`use_sqlite_with(path, |o| ...)`、`use_mysql_with(cs, |o| ...)`、`use_postgres_with(cs, |o| ...)`。delegate 配置各 provider 的 options（池大小、TLS、SQLite 获取超时），后续扩展无需新增 `use_*` 方法。原有 `use_*` / `use_*_with_pool` / `use_*_with_tls` 保持不变。
- **Keyed 共享实体模型**：`use_default_entity_model()` 让读写副本 context 共享默认实体模型，而不是按 `#[context(key)]` 过滤。

#### 变更

- **`use_transaction` 行为**：查询、导航、`sql_query`、`execute_update`、`execute_delete` 现在加入环境事务。`begin_transaction` 保持独立句柄语义（查询和 `save_changes()` 不加入），以兼容旧代码；需要上下文级工作单元时使用 `transaction`。
- **隔离级别语义**：`set_isolation` 保留各 provider 的旧行为（SQLite PRAGMA、MySQL `SET TRANSACTION`、PostgreSQL）；新的 `*_with` 入口是跨库、经过校验的隔离级别设置方式。
- **读写分离方式**：文档改为 EF Core 推荐的 keyed 双 `DbContext`；不新增 `use_*_read_write` 方法。写 context 的池大小通过各 provider 的 delegate 设置（如 `use_sqlite_with(..., |o| o.max_pool_size(1))`、`use_mysql_with(..., |o| o.max_pool_size(1))`、`use_postgres_with(..., |o| o.pool_size(1))`）。

---

## [1.8.4] — 2026-09-09 — Fix crates.io README mojibake

> **English** · **简体中文**

### English

#### Fixed

- **README / docs encoding**: replaced corrupted `U+FFFD` sequences in crate READMEs and Chinese docs with the intended `—` / `→` (and restored Chinese text), so crates.io no longer renders garbled `????` in package pages.

### 简体中文

#### 修复

- **README / 文档编码**：清除 crate README 与中文文档中的损坏字符 `U+FFFD`，恢复为正确的 `—` / `→` 与中文文案，修复 crates.io 页面 README 乱码。

## [1.8.3] — 2026-09-01 — EntityType String FK compile fix

> **English** · **简体中文**

### English

#### Fixed

- **`#[derive(EntityType)]`**: `set_foreign_key` codegen no longer emits `key as _` for non-integer FK fields (e.g. `String` / UUID). Cascade auto-increment fixup arms are generated only for integer FK types, restoring compile for String-keyed entities such as docbit.

### 简体中文

#### 修复

- **`#[derive(EntityType)]`**：非整数外键（如 `String`/UUID）不再生成 `i64 as _` 赋值；仅整数 FK 生成 cascade 回填臂，修复 docbit 等 String 主键实体的编译失败。

## [1.8.2] — 2026-09-01 — rust-dix 0.7

> **English** · **简体中文**

### English

#### Changed

- **DI**: depend on `rust-dix 0.7` (inject diagnostics, built-in `ScopeFactory`, stricter async `build()`).

### 简体中文

#### 变更

- **DI**：依赖升级到 `rust-dix 0.7`。

## [1.8.1] — 2026-08-16 — crates.io 再发布对齐 · Re-release alignment

> **English** · **简体中文**

### English

On top of 1.8.0 (ChangeTracker architecture redesign + EntitySnapshot), the `rust-ef`
crate family has been re-published to crates.io and is now managed under the
`Rust-Framework` organization.

### Changed

- **crates.io re-publish**: `rust-ef` / `rust-ef-macros` / `rust-ef-sqlite` /
  `rust-ef-postgres` / `rust-ef-mysql` aligned; repository metadata (`license`,
  `repository`) aligned with the GitHub repo.
- **Automated publishing**: new GitHub Actions `publish.yml` publishes crates in
  dependency order on `v*` tag push.
- **Docs landing page**: added `docs/README.md` bilingual navigation.

> Maintenance release; the runtime API is unchanged.

### 简体中文

v1.8.1 为发布维护迭代，在 1.8.0（ChangeTracker 架构重设计 + EntitySnapshot）基础上
将 `rust-ef` 全系 crate 重新发布到 crates.io，归入 `Rust-Framework` 组织统一管理。
不改变运行时 API。

### Changed

- **crates.io 再发布**：`rust-ef` / `rust-ef-macros` / `rust-ef-sqlite` / `rust-ef-postgres`
  / `rust-ef-mysql` 全系重新发布对齐；仓库元数据（`license`、`repository`）对齐 GitHub 仓库。
- **自动化发布**：新增 GitHub Actions `publish.yml`，推送 `v*` tag 时按依赖顺序自动发布。
- **文档落地页**：新增 `docs/README.md` 中英文档导航入口。

---

## [1.8.0] — 2026-07-10 — 深度性能优化迭代（ChangeTracker 架构重设计 + EntitySnapshot）

### Summary

v1.8.0 是深度性能优化迭代，包含两项架构级变更和一项分配瓶颈消除：

1. **EntitySnapshot 类型**（迭代 1）：`snapshot()` / `key_values()` 返回类型从
   `HashMap<String, DbValue>` 改为 `EntitySnapshot`（`Box<[(&'static str, DbValue)]>`），
   消除每次调用的 HashMap 分配 + N 个 String 键分配。22 个调用点全部适配。
2. **ChangeTracker 单一跟踪源**（迭代 2，**Breaking Change**）：跟踪状态从 `DbSet`
   内联移至 `ChangeTracker`，`DbSet` 仅持有 `entity + entry_id`。变更操作入口从
   `db_set.add()` 移至 `ctx.add::<T>()`，符合 EFCore 模式。
3. **sql_cache 移除**（迭代 3）：移除 per-row UPDATE 中命中率 ≈ 0% 的 `sql_cache`，
   消除每行 2 String + 1 Vec 键分配。

> **迁移指南**：详见 [`docs/v1.8-migration-guide.md`](docs/v1.8-migration-guide.md)

### Added

- **`EntitySnapshot` 类型**（`crates/core/src/entity_snapshot.rs`）：不可变实体快照，
  内部用 `Box<[(&'static str, DbValue)]>`，单次堆分配，字段名为编译期 `&'static str`。
  - `get(field)` — O(n) 线性查找（典型 5-10 字段，性能影响可忽略）
  - `iter()` / `len()` / `is_empty()` — 不可变迭代器
  - `PartialEq` — 顺序无关比较
  - `From<Vec<(&'static str, DbValue)>>` / `Default` / `Debug` / `Clone`
  - 8 个单元测试覆盖构造、查找、迭代、等值比较、克隆

- **`DbContext` 变更操作方法**：`add` / `attach` / `update` / `upsert` /
  `remove_at` / `remove_all` / `load_all` — 统一在 `DbContext` 上调用，
  内部自动注册 `ChangeTracker` 状态。

- **`bench_snapshot.rs` 基准测试**：直接基准 `entity.snapshot()` 和
  `entity.key_values()` 的分配开销（5 字段 / 10 字段实体，批量 1000 实体）。

- **迁移指南**（`docs/v1.8-migration-guide.md`）：涵盖 DbContext API 变更、
  `snapshot()` / `key_values()` 返回类型变更、手动 `IEntitySnapshot` impl 迁移、
  正则表达式批量替换表。

### Changed — 架构重设计

#### 迭代 1（P1）：EntitySnapshot 类型

- **`entity.rs` trait 签名变更**：`IEntitySnapshot::snapshot()` 和
  `IGetKeyValues::key_values()` 返回类型 `HashMap<String, DbValue>` → `EntitySnapshot`
- **宏生成代码变更**（`macros/src/entity/gen.rs`）：`HashMap::new() + insert` →
  `EntitySnapshot::new(vec![...])`
- **22 个调用点适配**：`db_set.rs`、`executor_dml.rs`、`executor.rs`、
  `navigation_loader.rs`、`model_builder.rs`、`set_ops.rs` 等

#### 迭代 2（P1）：ChangeTracker 单一跟踪源

- **`tracking.rs` 重设计**：删除 `PropertySnapshot`（String-based 死代码）；
  `TrackerEntry` 改为 `{ id, type_id, type_name, state, original: Option<EntitySnapshot>,
  modified_properties, is_upsert }`，存储在 `HashMap<u64, TrackerEntry>`
- **`db_set.rs` 重设计**：`TrackedEntry<T>` → `DbSetEntry<T> { entity, entry_id }`，
  移除 `state` / `original` / `modified_properties` / `is_upsert` 字段
- **`set_ops.rs` 重设计**：`ErasedSetOps` 方法签名增加 `&ChangeTracker` /
  `&mut ChangeTracker` 参数；`SetOps<E>` impl 通过 `entry_id` join DbSet 与 tracker
- **`save_pipeline.rs` 更新**：所有 saver 方法调用传递 `&self.change_tracker`
- **`context.rs` 更新**：新增 `ctx.add::<T>()` / `ctx.attach::<T>()` /
  `ctx.update::<T>()` / `ctx.upsert::<T>()` / `ctx.remove_at::<T>()` /
  `ctx.remove_all::<T>()` / `ctx.load_all::<T>()`
- **`cascade.rs` 更新**：`resolve_delete_behavior` 移入（保持 set_ops.rs ≤500 行）

#### 迭代 3（P2）：sql_cache 移除

- **`executor_dml.rs`**：移除 `sql_cache: HashMap<(String, Vec<String>, String), String>`
  及 `use std::collections::HashMap` import；per-row UPDATE 直接调用
  `gen.update(table_name, &set_cols, &where_clause)`

### Removed

- **`save_one_set()` 函数**（`db_context/save_phases.rs`）：独立 `DbSet` 保存入口
  已移除，统一使用 `DbContext::save_changes()`
- **`PropertySnapshot` 结构体**（`tracking.rs`）：String-based 死代码
- **`track_entity()` / `detect_changes_with_properties()` / `update_snapshot()`**
  （`tracking.rs`）：String-based 死代码方法
- **`TrackedEntry<T>` 结构体**（`db_set.rs`）：被 `DbSetEntry<T>` 替代
- **`DbSet::add()` / `attach()` / `update()` / `upsert()` / `remove_at()` /
  `remove_all()` / `detect_changes()` / `load_all()` 方法**：移至 `DbContext`
- **`entries_with_state()` 方法**（`db_set.rs`）：通过 `change_tracker().entries()` 替代

### Breaking Changes

| 变更 | Before | After |
|------|--------|-------|
| 变更操作调用位置 | `ctx.set::<T>().add(e)` | `ctx.add::<T>(e)` |
| snapshot() 返回类型 | `HashMap<String, DbValue>` | `EntitySnapshot` |
| key_values() 返回类型 | `HashMap<String, DbValue>` | `EntitySnapshot` |
| save_one_set | `save_one_set(&mut conn, ...)` | `ctx.save_changes()` |
| ChangeTracker.track_entity | `track_entity(type_id, name, state)` | `track(type_id, name, state, None, false)` |
| ChangeTracker.accept_all_changes | `accept_all_changes()` | `accept_all_changes(&[])` |
| entries_with_state | `db_set.entries_with_state()` | `ctx.change_tracker().entries()` |

---

## [1.7.0] — 2026-07-10 — 性能优化迭代（热路径堆分配消除）

### Summary

v1.7.0 是性能优化迭代，外部 API 保持不变（仅新增 `DbValueKey` 公开类型与
`ISqlGenerator::uses_numbered_placeholders()` 带默认实现的方法）。针对代码审查中
识别的 6 处热路径堆分配瓶颈，通过 4 个迭代完成优化：导航加载热路径（`DbValueKey`
+ `hex::encode` 查表法）、SQL 构建缓冲区（`to_sql_with` + `update_batch`）、
保存管线（`scalar_props` 提升 + 过滤器 SQL 缓存）、基准测试补全。

### Added

- **`DbValueKey` 公开类型**（`crates/core/src/provider/db_value_key.rs`）：owned
  枚举，将 `DbValue` 规范化为可哈希形式，用作 `HashMap`/`HashSet` 键。
  - 整数变体（`I16`/`I32`/`I64`）统一为 `Int(i64)`，**零分配**（PK 最常见类型）
  - 浮点变体（`F32`/`F64`）经 `to_bits()` 统一为 `FloatBits(u64)`，零分配且 NaN
    哈希语义正确
  - `String`/`Bytes` 仍有一次 clone，但免去 `format!` 的格式串解析开销
  - feature-gated 变体严格对齐 `DbValue`（`chrono`/`uuid`/`decimal`）
  - 8 个单元测试覆盖整数/浮点/String/Bytes/Null/NaN/UUID/DateTime 转换与哈希一致性
  - 已通过 `provider/mod.rs` 的 `pub use` 导出

- **`ISqlGenerator::uses_numbered_placeholders()`**（`crates/core/src/provider/traits.rs`）：
  新增带默认实现的方法（`false`），标识 provider 是否使用编号占位符。
  - PostgreSQL `$N` → 覆写为 `true`
  - SQLite/MySQL `?` → 保持默认 `false`（占位符与索引无关）
  - 默认实现保证现有 provider 无需改动

- **基准测试**（`crates/core/benches/`）：
  - `bench_save.rs` — 批量 UPDATE（1000 实体 CASE WHEN）+ 批量 DELETE（1000 实体 IN 子句）
  - `bench_detect_changes.rs` — 500/1000 已跟踪实体（半数修改）的 `detect_changes`
  - 在 `crates/core/Cargo.toml` 注册两个 `[[bench]]` 入口

### Changed — 性能优化

#### 迭代 1（P1）：导航加载热路径

- **`navigation_loader.rs` 键类型迁移**：`db_value_key()` 返回类型 `String` → `DbValueKey`；
  `group_rows` / `group_join_rows` / `index_rows` 的 `HashMap<String, ...>` →
  `HashMap<DbValueKey, ...>`；`load_many_to_many` 去重 `HashSet<String>` →
  `HashSet<DbValueKey>`。消除整数 PK 场景的 `format!("{}", v)` 分配（50×10 include
  场景从 500 次分配降为 0）。

- **`db_value.rs` 的 `hex::encode` 查表法**：替换每字节 `format!("{:02x}", b)` 为
  `HEX_CHARS` 查表 + `String::with_capacity(bytes.len() * 2)` 单次分配。N 字节从
  N+1 次分配降为 1 次分配，且无 `format!` 机制开销。4 个单元测试验证输出一致性。

#### 迭代 2（P2）：SQL 构建缓冲区

- **`query/state.rs` 的 `to_sql_with()`**：替换链式 `format!()` 临时 String 为
  `String::with_capacity(256)` + `push_str`/`push(' ')`。CTE 前缀从
  `format!("{} {} {}", ...)` 改为预构建 prefix 缓冲区 + `sql.insert_str(0, &prefix)`。
  消除 SELECT/FROM/JOIN/WHERE/GROUP BY/HAVING/ORDER BY/LIMIT 等子句的中间 String 分配。

- **`provider/traits.rs` 的 `update_batch()`**：替换 `Vec<String>` 收集 + `format!`
  为直接缓冲区构建（`String::with_capacity(256 + set_columns.len() * row_count * 20)`
  + `push_str`）。CASE WHEN 片段直接写入缓冲区。

#### 迭代 3（P2）：保存管线

- **`executor_dml.rs` 的 `scalar_props` 提升**：`execute_updates_per_row` 和
  `execute_deletes_per_row` 中，将 `meta.mapped_scalar_properties().collect()` 从
  per-entity 循环内提升到循环外（使用 `entities[0].1` 的 meta）。同一调用中所有实体
  共享同一 `EntityTypeMeta`，避免每实体重复收集。

- **batch 路径过滤器 SQL 缓存**：`execute_updates` 和 `execute_deletes` 的 batch 路径
  中，当 `!gen.uses_numbered_placeholders()` 时（SQLite/MySQL），在 batch 循环外
  预编译 `compile_bool_expr` 结果供所有 batch 复用。PostgreSQL（`$N`）仍每 batch
  重编译（占位符编号与起始索引相关）。

### Compliance

- 所有 `.rs` 文件 ≤500 行（`db_value_key.rs` ~130 行）
- `provider/mod.rs` 仅添加 `mod db_value_key;` + `pub use db_value_key::DbValueKey;`
- `cargo check --workspace --all-features` 通过
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` 零警告
- `cargo fmt --all --check` 通过
- `cargo test --workspace --all-features` 全量通过
- 基准测试编译通过：`cargo bench -p rust-ef --bench bench_save --no-run`、
  `cargo bench -p rust-ef --bench bench_detect_changes --no-run`

---

## [1.6.0] — 2026-07-10 — 内部清理迭代（大文件拆分 + mod.rs 合规 + panic 收敛 + EFErrorCode）

### Summary

v1.6.0 是内部清理迭代，无新功能开发，外部 API 保持不变。涵盖 6 个迭代周期：
大文件拆分（core + macros + tests）、mod.rs 合规性修复、生产代码 panic 路径收敛、
EFErrorCode 错误码体系、MetadataCache 读锁优化、dead_code 清理。所有 `.rs` 文件
≤500 行，所有 `mod.rs` 仅含 `mod` 声明与 `pub use` 重导出。

### Added

- **EFErrorCode 错误码体系**（`crates/core/src/error.rs`）：新增 `EFErrorCode` 枚举
  （15 变体）和 `EFError::code()` 方法，支持程序化错误分类。按变体 + 消息模式匹配
  子类区分（如 `Connection` → `ConnectionRefused` / `ConnectionTimeout`）。已导出
  至 `prelude`。

### Changed

- **MetadataCache Mutex → RwLock**（`crates/core/src/metadata_cache.rs`）：读多写少
  场景从 `Mutex<HashMap>` 改为 `RwLock<HashMap>`。`get_or_build` 采用 double-check
  模式：`read()` 快速路径查缓存，miss 时降级 `write()` 构建 + 插入。Poison 恢复保留。

- **生产代码 panic 收敛**：生产代码 panic 从 57 处收敛至 11 处（目标 ≤20）。涉及
  `save_pipeline.rs`（30→0）、`save_phases.rs`（8→0）、`set_ops.rs`（4→0）、
  `transaction.rs`（3→1）、`context.rs`（2→1）、`navigation_loader.rs`（1→0）。
  剩余 11 处均为 API 受限方法（返回 `Self` / `String` / `&mut T`，无法改为 `Result`）。

- **dead_code 清理**：9 处 `#[allow(dead_code)]` 全部添加 `reason` 说明。
  - `macros/linq/ast.rs`（1 处）：`CrossJoin.param` 为未来 select-binding 保留
  - `core/tests/fk_on_delete_tests.rs`（8 处）：测试实体结构体为 `#[derive(EntityType)]`
    元数据生成存在，字段不在测试中实例化

### Internal Refactoring — 大文件拆分

**迭代 2（P0）**：删除 `migration.rs`（1569 行，已拆分为 `migration/` 目录）；
`db_context/mod.rs` 和 `tests/common/mod.rs` 合规性修复（业务代码提取到子文件）。

**迭代 3（P0）**：core crate 大文件拆分：
- `query/builder.rs`（1219 行）→ `query/builder/` 目录（6 文件）
- `provider.rs`（731 行）→ `provider/` 目录（3 文件）
- `change_executor.rs`（728 行）→ `change_executor/` 目录（3 文件）
- `query/ast.rs`（518 行）→ 降至 500 行以内

**迭代 4（P1）**：macros crate 大文件拆分：
- `macros/entity.rs`（979 行）→ `entity/` 目录（4 文件）
- `macros/linq/parse.rs`（963 行）→ `linq/parse/` 目录（3 文件）
- `macros/linq/compile.rs`（798 行）→ `linq/compile/` 目录（4 文件）

**迭代 5（P1）**：测试文件拆分：
- `cascade_save_tests.rs`（772 行）→ 4 文件 + `common/cascade_fixtures.rs`
- `navigation_perf_tests.rs`（742 行）→ 2 文件
- `having_pagination_dialect_tests.rs`（606 行）→ 2 文件 + `common/mock_generators.rs`

### Compliance

- 所有 `.rs` 文件 ≤500 行（验证通过）
- 所有 `mod.rs` ≤50 行且仅含 `mod` 声明与 `pub use` 重导出（12 文件全部合规）
- `cargo check --workspace --all-features` 通过
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` 零警告
- `cargo fmt --all --check` 通过
- `cargo test --workspace --all-features` 全量通过

---

## [1.5.3] — 2026-07-09 — 阻断性修复 + 死代码清理 + CI 加固

### Fixed

- **MySQL tracing 编译失败**：`crates/mysql/src/row_conversion.rs` 在 `--features tracing` 下
  缺少 `use sqlx::Column;`，导致 `c.name()` 调用编译失败。已将导入条件化为
  `#[cfg(feature = "tracing")] use sqlx::Column;`。
- **clippy `--all-features` 阻断**：修复 4 个仅在 `--all-features` 下暴露的 clippy 错误：
  - 移除 `ErasedSetOps` trait 中未使用的 `save` 和 `entry_count` 方法（死代码）
  - `SchemaChange::AlterColumn` 的 `old`/`new` 字段改为 `Box<SnapshotColumn>`（large_enum_variant）
  - `update_modified_phase` / `delete_deleted_phase` 添加 `#[allow(clippy::type_complexity)]`
- **测试文件未使用导入**：移除 3 个测试文件中的 `use rust_ef::db_set::IDbSet;`
- **代码格式**：运行 `cargo fmt --all` 修复全部格式偏差

### Removed

- **migration/ 死代码目录**：删除 `crates/core/src/migration/` 下 5 个未编译文件
  （types.rs / engine.rs / engine_sql.rs / engine_exec.rs / diff.rs，共 ~1,298 行）。
  这是一套未完成的重构，缺少 v1.5.2 的 `fk_on_delete` 支持，且引用不存在的
  `super::history` 模块，与 `migration.rs` 内联实现重复。

### Changed

- **CI 加固**：`.github/workflows/ci.yml` 新增 `cargo clippy --all-features` 步骤，
  sqlite 测试改为 `--all-features` 覆盖 tracing feature，防止类似 #1 的编译失败再次遗漏。
- **测试公共模块**：`crates/core/tests/common/mod.rs` 添加 `#![allow(dead_code)]`，
  因多 provider 测试目标共享该模块但各自只使用部分辅助函数。

---

## [1.5.2] — 2026-07-09 — 级联删除 + FK ON DELETE DDL（含未发布的 v1.6.0 生产硬化）

### Summary

v1.5.2 新增应用级与数据库级联删除能力，并包含此前未发布到 crates.io 的 v1.6.0
生产硬化全部改动（连接池单例化、Debug 脱敏、BoolExpr::raw 收窄、TLS 安全优先、
De-String 物化管道、属性级变更追踪、批量 INSERT 主键回填、Upsert API、Raw SQL
映射等——详见下方 [1.6.0] 条目）。从宏属性解析、元数据存储、应用级级联执行到
迁移 DDL 的 `ON DELETE` 子句生成，形成全链路一致的删除策略。新增 `#[on_delete]`
宏属性，支持 `Cascade` / `Restrict` / `SetNull` / `NoAction` 四种行为。

> 注：v1.6.0 曾作为 git tag 标记但未发布到 crates.io，其全部改动包含在 v1.5.2 中。

### Added — 应用级级联删除

新增 `#[on_delete]` 宏属性，在 `BelongsTo` 导航上配置删除行为：

```rust
#[derive(Debug, Clone, EntityType)]
struct Post {
    #[primary_key] #[auto_increment] id: i32,
    #[foreign_key(Blog)]
    #[on_delete(SetNull)]
    blog_id: Option<i32>,
    #[navigation] blog: BelongsTo<Blog>,
}
```

**应用级级联执行逻辑**（`crates/core/src/db_context.rs`）：
- 已加载子实体：标记为 `Deleted`，由 `save_changes` 统一提交
- 未加载子实体：直接执行批量 `DELETE` SQL
- `SetNull` 指令在 PK-based DELETE 之前执行，`Cascade` DELETE 在之后执行
- 多对多关联表自动级联清理

**默认行为推断**（`resolve_delete_behavior`，改为 `pub(crate)` 供迁移模块复用）：
- 多对多关系 → `Cascade`
- 非空外键（`i32`）→ `Cascade`
- 可空外键（`Option<T>`）→ `Restrict`

### Added — FK ON DELETE DDL 生成

迁移系统在生成外键约束时自动附加 `ON DELETE` 子句，确保数据库级约束与应用级
级联行为一致：

```sql
ALTER TABLE posts ADD CONSTRAINT fk_posts_blog_id_blogs
    FOREIGN KEY (blog_id) REFERENCES blogs (id) ON DELETE CASCADE;
```

**实现要点**（`crates/core/src/migration.rs`）：
- `DeleteBehavior::to_sql_clause()` 方法映射到 SQL 关键字
  （`CASCADE` / `RESTRICT` / `SET NULL` / `NO ACTION`）
- `fk_reference_for_property` 通过交叉引用 principal 的 `HasMany` 导航解析
  ON DELETE 子句（BelongsTo → `related_entity_meta()` → principal meta →
  匹配 `related_type_id` 的 HasMany → `resolve_delete_behavior`）
- `SchemaChange::AddForeignKey` 新增 `on_delete: Option<String>` 字段
- `SnapshotColumn` 新增 `fk_on_delete` 字段，支持 JSON 快照序列化往返
- `diff_foreign_keys` 检测 ON DELETE 子句变更并触发 FK 重建

### Fixed — 预存 bug：`foreign_key_field` 始终为 `None`

宏的 `extract_foreign_key_field_name`（`crates/macros/src/entity.rs` L857-859）
历史上始终返回 `None`，导致 `fk_reference_for_property` 无法匹配 BelongsTo
导航，FK 约束从未在 DDL 中生成。改为按 `fk_column`（列名）匹配后，FK 约束
可正常生成。

### Tests

- `cascade_save_tests.rs` — 12 个级联保存/删除测试（含自引用、多对多、
  反向排序、空 HasMany no-op、SetNull、未跟踪子实体）
- `fk_on_delete_tests.rs` — 6 个 ON DELETE DDL 测试：
  1. 必填 FK（`i32`）默认生成 `CASCADE`
  2. 可选 FK（`Option<i32>`）默认生成 `RESTRICT`
  3. 显式 `#[on_delete(SetNull)]` 生成 `SET NULL`
  4. 显式 `#[on_delete(NoAction)]` 生成 `NO ACTION`
  5. ON DELETE 子句变更触发 FK 重建（diff 检测）
  6. `fk_on_delete` 字段 JSON 序列化往返
- 全量回归：265 测试通过，0 回归（仅 8 个 PostgreSQL 测试因无服务器跳过）

### Changed

- `crates/core/src/relations.rs`：`DeleteBehavior` 新增 `to_sql_clause()` 方法
- `crates/core/src/db_context.rs`：`resolve_delete_behavior` 改为 `pub(crate)`
- `crates/core/src/metadata.rs`：`NavigationMeta` 新增 `delete_behavior` 字段
- `crates/macros/src/entity.rs`：BelongsTo 导航元数据生成支持 `#[on_delete]`
  属性解析（`extract_on_delete` 函数）
- `crates/core/src/migration.rs`：`fk_reference_for_property` 改为 3-tuple
  返回值，按 `fk_column` 匹配；新增 `resolve_fk_on_delete_clause` 函数

---

## [1.6.0-aborted] — 2026-07-09 — Production Hardening（4 P0 + 12 P1）

> **注**：此版本曾作为 git tag 标记但未发布到 crates.io，其全部改动已合并到 v1.5.2 中。
> v1.6.0 版本号于 2026-07-10 重新用于内部清理迭代发布（见上方 `[1.6.0]` 条目）。

### Summary

v1.6.0 是生产硬化迭代，修复 4 个 P0 阻塞项和 12 个 P1 改进项。涵盖连接管理、安全
默认值、类型系统物化、性能批量化、完整性 API 五大领域。**含破坏性变更**——详见
`docs/v1.5-semver-migration-guide.md` 的 v1.6 章节。

### P0 — 阻塞项修复

#### M1.1 连接池单例化

`DbContextOptions.provider_cache` 进程级缓存 provider（含连接池），所有同源
`DbContext` 复用同一连接池。避免每请求重建池导致的连接泄漏。

#### M1.2 Debug 脱敏

`DbContextOptions` 的 `Debug` 实现脱敏连接串中的凭据（URL 形式 `user:pass@host`
和 key=value 形式 `Password=...`），防止日志泄漏。

#### M1.3 BoolExpr::raw 收窄

`BoolExpr::raw()` 从 `pub` 收窄为 `pub(crate)`，外部代码无法构造 `RawSql` 变体，
从类型层面消除 SQL 注入风险。

#### M1.4 TLS 安全优先

PostgreSQL TLS 默认从 `Disable` 改为 `Require`；MySQL TLS 默认从 `Disabled` 改为
`Required`。`Disable`/`Disabled` 仍可通过显式 `*_with_tls` API 选用（逃生舱）。

### P1 — 改进项

#### M2 De-String 物化管道

`IFromRow::from_row` 签名从 `&[String]` 改为 `&[DbValue]`，消除全链路 String 物化。
新增 `TryFrom<DbValue>` for `i8/u32/u64`，provider 层 `cell_to_db_value` 替代
`cell_to_string`。**破坏性变更**。

#### M3 P1 稳定性

- 字符串 PK include 测试覆盖
- Layer 8（consumer 层）全量验证

#### M4 P1 性能

- C1: 批量 UPDATE 用 `CASE pk WHEN ? THEN ?` 减少 N 次往返为 1 次
- C2: 批量 INCLUDE 用 UNION 策略减少 N 次查询为 1 次
- C3: 连接池 acquire 指标 tracing
- C4: 批量 INSERT 参数布局优化

#### M5 P1 完整性

**D1: 属性级变更追踪 + 部分 UPDATE**

`detect_changes` 收集所有变更字段名到 `modified_properties: Vec<String>`。
`execute_updates` 仅 SET 脏列（批量 CASE WHEN 用并集，逐行用各实体自己的列表）。
`modified_properties` 为空时回退到全列 SET（向后兼容）。并发令牌安全：令牌递增后
出现在 `modified_properties` 中并被 SET，WHERE 始终检查原始令牌值。

**D2: 批量 INSERT 主键回填**

- PostgreSQL: `INSERT ... RETURNING *` 直接读取生成的主键
- SQLite: `SELECT last_insert_rowid()`（返回最后 rowid，计算 `last-N+1..last`）
- MySQL: `SELECT LAST_INSERT_ID()`（返回首个 ID，计算 `first..first+N-1`）

新增 `ISqlGenerator::supports_returning()` / `last_insert_id_sql()` /
`last_insert_id_returns_first()` 默认方法。新增 `IGetKeyValues::set_auto_increment_key`
trait 方法（默认 no-op，宏为 `#[auto_increment] #[primary_key]` 字段生成覆盖）。

`save_changes` 后行为变更：Added/Modified → Unchanged（带刷新快照），Deleted 移除。
`accept_all_changes` 替代 `clear_entries`，保留已保存实体（含回填主键）可查询。

**D3: Upsert API**

新增 `DbSet::upsert(&mut self, entity: T)` — 标记 Added + `is_upsert: true`。
`save_changes` 将 upsert 条目路由到 `execute_upserts`，生成：

- SQLite/PostgreSQL: `INSERT ... ON CONFLICT(pk) DO UPDATE SET col = EXCLUDED.col`
- MySQL: `INSERT ... ON DUPLICATE KEY UPDATE col = VALUES(col)`

冲突目标为主键列。INSERT 包含所有列（含自增主键）以确保用户提供的键值参与冲突检查。
不做主键回填——upsert 调用者需自行管理键。

新增 `ISqlGenerator::upsert_batch()` 默认方法，三 provider 实现。

**D4: Raw SQL → 实体映射**

新增 `DbContext::sql_query<T: IFromRow + IEntityType>(&self, sql: &str, params: &[DbValue])` —
复杂查询（多表 JOIN、CTE、窗口函数）的逃生舱。复用 M2 后的 `IFromRow`（已支持
`&[DbValue]`），无需新 trait。

### 破坏性变更摘要

| 变更 | 影响 | 迁移方式 |
|------|------|----------|
| `IFromRow::from_row` 签名 `&[String]` → `&[DbValue]` | 自定义实体需重写 `from_row` | 用 `TryFrom<DbValue>` |
| `ParseFromDb` trait 移除 | 使用该 trait 的代码需迁移 | 改用 `TryFrom<DbValue>` |
| TLS 默认值变更（PG `Require`、MySQL `Required`） | 明文连接需显式声明 | 用 `*_with_tls(Disable)` |
| `BoolExpr::raw` 收窄为 `pub(crate)` | 外部无法构造 `RawSql` | 用参数化查询 |
| `save_changes` 后保留实体（不 clear） | 依赖 clear 行为的代码需手动 `clear_entries` | 调用 `db_set.clear_entries()` |
| `DbValue` 新增 `TryFrom` for i8/u32/u64 | 无破坏（additive） | — |

---

## [1.5.0] — 2026-07-08 — tracing 集成 + SemVer 严格化 + MySQL TLS 显式 API

### Added — tracing 集成（慢查询 + 连接池指标）

新增 `tracing` 可选 feature（core + sqlite / postgres / mysql 三个 provider），通过
`tracing` crate 发射结构化事件。feature 关闭时全部 Guard 为 ZST no-op，编译期消除，
零运行时开销。

**3 个 Guard 类型**（`crates/core/src/observability.rs`）：

- `QueryGuard` — 包裹单次 `query` / `execute` 调用；启动时发 `DEBUG` 事件，完成时发
  `DEBUG`（正常）或 `WARN`（超过慢查询阈值）。target：`rust_ef::query`
- `PoolAcquireGuard` — 包裹连接池 acquire；完成时发 `INFO` 事件含 acquire 耗时。
  target：`rust_ef::pool`
- `SaveChangesGuard` — 包裹 `save_changes` 调用；完成时发 `INFO` 事件含总耗时。
  target：`rust_ef::save_changes`

**配置 API**：

```rust
use std::time::Duration;
use rust_ef::DbContextOptionsBuilder;

let mut options = DbContextOptionsBuilder::new();
options
    .use_sqlite("app.db")
    .slow_query_threshold(Duration::from_millis(500)); // 超过 500ms 的查询发 WARN
```

**trait 扩展**（cfg-gated 默认方法，非 breaking）：

- `IAsyncConnection::set_slow_query_threshold(&mut self, Duration)` — 连接级阈值注入
- `IDatabaseProvider::set_slow_query_threshold(&self, Duration)` — provider 级阈值存储

**实现文件**：

- `crates/core/src/observability.rs` — 3 个 Guard 的 cfg-gated 双实现
- `crates/core/src/db_context.rs` — `DbContextOptions.slow_query_threshold` 字段 +
  builder 方法 + `create_provider` 注入 + `save_changes` 加 `SaveChangesGuard`
- `crates/core/src/provider.rs` — 两个 trait 的 cfg-gated 默认方法
- `crates/{sqlite,postgres,mysql}/src/provider.rs` — `AtomicU64` 阈值存储 +
  `PoolAcquireGuard` 在 `get_connection` + `set_slow_query_threshold` impl
- `crates/{sqlite,postgres,mysql}/src/connection.rs` — 阈值字段 + `threshold()` helper
  + `QueryGuard` 在 `execute` / `query` + `set_slow_query_threshold` impl

**使用方式**（应用层初始化 subscriber）：

```rust
use tracing_subscriber;

tracing_subscriber::fmt()
    .with_env_filter("rust_ef::query=warn,rust_ef::pool=info,rust_ef::save_changes=info")
    .init();
// 之后所有 rust-ef 操作自动发射 tracing 事件
```

### Added — MySQL TLS 显式 API

新增 `MySqlTlsMode` 枚举，对齐 PostgreSQL 的 `PgTlsMode` 模式，提供显式 TLS 配置：

```rust
pub enum MySqlTlsMode {
    Disabled,        // 禁用 TLS（明文连接）
    Required,        // 强制 TLS（服务器不支持则失败）
    VerifyCa,        // 强制 TLS + CA 证书验证
    VerifyIdentity,  // 强制 TLS + CA 证书 + 主机名验证
}
```

**新增 API**：

- `MySqlProvider::new_with_tls(connection_string, tls: MySqlTlsMode)` — 异步连接 + TLS
- `MySqlProvider::new_lazy_with_tls(connection_string, tls: MySqlTlsMode)` — 延迟连接 + TLS
- `DbContextOptionsBuilderExt::use_mysql_with_tls(connection_string, tls)` — DI 扩展

**设计要点**：

- 独立枚举（不 wrap sqlx 类型），不携带 `TlsConnector`（sqlx 通过 `tls-native-tls`
  feature 内部管理 TLS）
- 排除 `Preferred` 变体（显式 TLS API 不应含"可选"语义）
- CA 证书通过连接串 `ssl-ca` 参数传递
- `From<MySqlTlsMode> for sqlx::mysql::MySqlSslMode` 实现 trait 桥接

**使用示例**：

```rust
use rust_ef_mysql::{DbContextOptionsBuilderExt, MySqlTlsMode};
use rust_ef::DbContextOptionsBuilder;

let mut options = DbContextOptionsBuilder::new();
options.use_mysql_with_tls(
    "mysql://user:pass@host/db?ssl-ca=/path/to/ca.pem",
    MySqlTlsMode::VerifyIdentity,
);
```

### Added — SemVer 严格化迁移指南

新文档 `docs/v1.5-semver-migration-guide.md`：

- v1.0–v1.4 历史 breaking change 清单与迁移路径
- v1.5 起的 SemVer 2.0.0 严格化策略（1 minor deprecation 期）
- `#[deprecated]` 使用规范与编译期警告示例
- v2.0 候选项清单（Arc 元数据共享、async trait 原生语法、错误码体系、metrics API）

### Changed

- workspace 版本 1.4.1 → 1.5.0（6 个 crate 同步）
- `crates/mysql/Cargo.toml`：sqlx features 增加 `tls-native-tls`

### Production readiness

- 可观测性维度：⚠️ → ✅（tracing 集成完成）
- SemVer 维度：⚠️ → ✅（严格化策略落地 + 迁移指南文档化）
- 详见 `docs/archive/PRODUCTION_READINESS_SPEC_v1.6.md` 第 4.2 节

---

## [1.4.0] — 2026-07-08 — Production hardening (P0+P1) + metadata cache + rust-dix 0.6

### Fixed — P0-1 MySQL `cell_to_string` 类型分发

MySQL Provider 的 `cell_to_string` 之前仅尝试 `String` 反序列化，对非
String 列（BOOL / INTEGER / FLOAT / DATETIME / UUID 等）静默失败并返回
`"NULL"`，导致 `IFromRow::from_row` 解析出错误值。现按 sqlx 类型顺序分发：
bool → i64 → u64 → f64 → NaiveDateTime → NaiveDate → Uuid → String →
Vec\<u8\>，每种类型用 `try_get` 尝试，首个成功者序列化为字符串。覆盖
`crates/mysql/src/connection.rs:20-68`。

### Fixed — P0-2 MetadataCache poison 恢复

`MetadataCache` 使用 `Mutex<HashMap>` 共享进程级元数据缓存。此前
`get_or_build()` 在 mutex 中毒时（前一个持有者 panic）调用
`.expect("MetadataCache poisoned")`，导致整个进程永久不可用。现改为：
捕获 `PoisonError` 后清空缓存中可能不完整的条目，重建后返回。3 个单元
测试覆盖（`test_poison_recovery` / `test_clear_after_poison` /
`test_concurrent_build_safe`），位于 `crates/core/src/metadata_cache.rs`。

### Added — P1-3 SQLite r2d2 连接池

SQLite Provider 引入 `SqliteProviderInner` 枚举区分两种连接策略：

- **`Pooled(r2d2::Pool<SqliteConnectionManager>)`** — 文件模式，默认 8
  连接。每个连接在 acquire 时执行
  `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;`，支持并发读 + 写
  等待，避免 `SQLITE_BUSY` 立即失败。
- **`Single(Arc<Mutex<rusqlite::Connection>>)`** — `:memory:` 模式，保留
  单连接语义以确保测试隔离（SQLite `:memory:` 数据库是 per-connection
  的，必须共享同一连接）。

`SqliteProvider::new(path)` 创建池化 provider；`new_in_memory()` 保留
旧行为。WAL 模式允许读写并发，5s busy_timeout 让写者等待锁释放而非
立即报错。

### Added — P1-6 PostgreSQL TLS 可配置

PostgreSQL Provider 引入 `PgTlsMode` 枚举：

- **`Disable`** — 使用 `tokio_postgres::NoTls`（向后兼容 v1.3，仅用于
  本地开发）
- **`Require(native_tls::TlsConnector)`** — 强制 TLS，使用平台原生实现
  （Windows SChannel / Linux OpenSSL / macOS Secure Transport）

新增 API：

- `PostgresProvider::new_with_tls(connection_string, pool_size, tls: PgTlsMode)`
- `DbContextOptionsBuilderExt::use_postgres_with_tls(connection_string, pool_size, tls)`

TLS 类型在 `deadpool_postgres::Manager` 内部通过 `Box<dyn Connect>`
擦除，因此 `Pool` 与 `PostgresConnection` 保持非泛型 API — TLS 是构造
期决策，非类型参数。依赖 `postgres-native-tls 0.5` + `native-tls 0.2`。

### Added — P1-5 PG/MySQL 集成测试对齐 SQLite 9 场景

`crates/core/tests/common/mod.rs` 此前仅有 `run_crud_lifecycle`
（insert/query/update/delete）。新增 7 个共享 helper 对齐 SQLite 的
9 场景：

- `run_filter_with_in_operator` — linq! 过滤 + IS NULL / IS NOT NULL
- `run_limit_and_offset` — take/skip 分页
- `run_count_and_any` — count + any 存在性检查
- `run_aggregation_queries` — sum/avg via linq! 宏
- `run_empty_result_handling` — 空表 to_list/count/any/first_or_default
- `run_ensure_created_and_deleted` — ensure_created → insert → ensure_deleted → ensure_created 重置
- `run_has_data_seed` — has_data 种子数据在 ensure_created 时物化

PG/MySQL 测试文件各追加 7 个 `#[tokio::test]`，与 SQLite 9 场景对齐
（scenario 5 update/delete 已在 `run_crud_lifecycle` 内）。CI 三库
matrix 全覆盖，本地无 DB 时优雅跳过。

### Added — Process-level metadata cache (priority 1 architecture iteration)

Entity/relationship/settings metadata is now parsed once per `context_key`
and shared as a singleton across all `DbContext` instances created from the
same `DbContextOptions`. Previously, every `from_options()` call (i.e. every
HTTP request via `get_owned()`) re-iterated `inventory::iter` and re-ran
all `IEntityTypeConfiguration::configure()` callbacks.

- **New module** `crates/core/src/metadata_cache.rs`: `MetadataCache` (a
  `Mutex<HashMap<Option<String>, Arc<BuiltMetadata>>>` keyed by `context_key`)
  + `BuiltMetadata` (snapshots `entity_metas`, `model_metas`, `configs`).
- **`DbContextOptions`** gains a `metadata_cache: Arc<MetadataCache>` field.
  Since `DbContextOptions` is already `Arc`-shared per `add_dbcontext`
  registration, the cache is naturally singleton-per-registration.
- **`DbContext::from_options()`** now calls `metadata_cache.get_or_build()`
  instead of re-iterating inventory. First call builds; subsequent calls
  `Arc::clone` the cached `BuiltMetadata`.
- **`ModelBuilder::from_built()`** new `pub(crate)` constructor populates
  `entity_metas` + `configs` from the cache, leaving `build_cache` /
  `filter_cache` lazy. Per-instance mutations (`has_query_filter`, etc.)
  only affect that `ModelBuilder` instance — the cache is never mutated.
- **`DbContext::discover_entities()`** is now a no-op (method retained for
  backward compatibility; metadata is pre-populated by `from_options()`).
- **Removed**: `DbContext.context_key` field (orphaned by the no-op
  `discover_entities()`; the key is read from `DbContextOptions` instead).
- **`EntityConfig` / `PropertyConfigOverride`** changed from private to
  `pub(crate)` so `MetadataCache::build()` can snapshot them.

#### Performance impact

For a 10-entity application, `from_options()` after the first call skips
~10 `configure()` callback executions + ~10 `EntityTypeMeta` constructions
+ 2 `inventory::iter` traversals. Expected ~90% reduction in per-request
metadata setup cost.

#### What is NOT shared (v1 limitation)

- `EntityTypeMeta.property_index` / `navigation_index` `OnceLock` caches
  are per-DbContext (each context clones its own `EntityTypeMeta` from the
  cache). The rebuild cost (~10-20 HashMap insertions per entity) is
  negligible against DB I/O. Arc-wrapping `EntityTypeMeta` is deferred to v2.

#### Backward compatibility

- `from_options()`, `set::<T>()`, `save_changes()`, `model()` public APIs
  are unchanged.
- `discover_entities()` still compiles and runs (as a no-op).
- `multi_db_context_tests` (context_key isolation) still passes — the cache
  is keyed by `context_key`.

### Added — Transaction interface extension (priority 2)

Extends `IAsyncConnection` with savepoint and isolation level capabilities,
and introduces an **ambient transaction** mechanism on `DbContext` so that
`save_changes()` can reuse a transaction opened by the caller — aligning
with EFCore's `Database.BeginTransaction()` / `Transaction.Commit()` pattern.

- **New `IsolationLevel` enum** (`ReadUncommitted` / `ReadCommitted` /
  `RepeatableRead` / `Serializable`) in `crates/core/src/provider.rs`,
  re-exported via `prelude`.
- **`IAsyncConnection` trait**: 4 new required methods —
  `create_savepoint(name)`, `release_savepoint(name)`,
  `rollback_to_savepoint(name)`, `set_transaction_isolation(level)`.
  No default implementations because SQL dialects differ (SQLite uses
  `RELEASE name` / `PRAGMA read_uncommitted`; PG/MySQL use
  `RELEASE SAVEPOINT name` / `SET TRANSACTION ISOLATION LEVEL`).
- **Provider implementations**: SQLite, PostgreSQL, MySQL each implement
  the 4 new methods with dialect-correct SQL.
- **`DbContext` ambient transaction**:
  - New field `ambient_transaction: Option<Box<dyn IAsyncConnection>>`.
  - `begin_transaction(&mut self) -> EFResult<()>` opens a transaction and
    stores it; subsequent `save_changes()` calls reuse this connection and
    do not begin/commit/rollback on their own (uses `take()`/restore pattern
    to avoid `&mut self` borrow conflicts with `self.sets`).
  - New `commit_transaction(&mut self)` / `rollback_transaction(&mut self)`.
  - New `create_savepoint` / `release_savepoint` / `rollback_to_savepoint` /
    `set_transaction_isolation` proxy methods (require active ambient
    transaction; return `EFError::Transaction` otherwise).
- **`save_changes()` integration**: when `ambient_transaction` is `Some`,
  takes the connection, runs all DbSet saves, and restores it without
  committing (the outer scope controls commit/rollback). When `None`,
  behaves as before (self-managed transaction).

### Breaking — `DbContext::begin_transaction` signature change

```rust
// Before:
pub async fn begin_transaction(&self) -> EFResult<Box<dyn IAsyncConnection>>

// After:
pub async fn begin_transaction(&mut self) -> EFResult<()>
```

The old signature returned a raw connection with no ambient tracking —
`save_changes()` could not reuse it (especially in PG/MySQL where each
`get_connection()` returns a different pooled connection). The new signature
stores the transaction in `DbContext` so `save_changes()` can reuse it.
Verified no external callers via grep.

### Migration — priority 2

1. Replace `let conn = ctx.begin_transaction().await?;` with
   `ctx.begin_transaction().await?;` (no return value).
2. Use `ctx.commit_transaction().await?` / `ctx.rollback_transaction().await?`
   to close the ambient transaction.
3. `save_changes()` called between `begin_transaction` and `commit_transaction`
   now reuses the ambient transaction (no code change needed).
4. New savepoint/isolation APIs are available on `DbContext` directly:
   `ctx.create_savepoint("sp1").await?` etc.

### Changed — linq.rs subdirectory split

Split `crates/macros/src/linq.rs` (2643 lines) into a `linq/` subdirectory
with 6 child modules for clearer responsibility separation:

- `ast.rs` (175 lines) — AST types (`LinqInput`, `QueryInput`, `LinqClause`,
  `HavingExprAst`)
- `parse.rs` (965 lines) — `impl Parse` + all `parse_*` functions + `ValueKind`
  + `JoinKind`
- `context.rs` (186 lines) — `LinqCtx` + `FieldKind` + `FieldRef` + field
  extraction helpers
- `compile.rs` (784 lines) — `compile_bool_expr` / `compile_expr` /
  `compile_method` / `compile_order` / `compile_having_expr` + subquery
  compilation
- `expand.rs` (412 lines) — `expand_linq` entry point + `expand_clauses` +
  `expand_join` (code generation)
- `mod.rs` (11 lines) — module declarations + `pub use expand::expand_linq`

Fixed E0027 (non-exhaustive match) in `expand_clauses`: the `LinqClause::With`
arm now destructures all 6 fields (`name`, `entity`, `param`, `body`,
`recursive`, `link`) and generates recursive CTE SQL via
`with_recursive_cte_typed` when `recursive` is true.

All internal items use `pub(crate)` visibility; only `expand_linq` is
re-exported via `pub use`. `crates/macros/src/lib.rs` unchanged — `mod linq;`
transparently resolves to `linq/mod.rs`.

---

## [1.3.1] — 2026-07-07 — rust-dicore → rust-dix 0.6 rename + breaking API sync

`rust-dicore` has been renamed to `rust-dix` upstream. The 0.6.0 release on
crates.io is the renamed successor of `rust-dicore 0.5.1`, but it also
introduces **breaking API changes** beyond the rename. rust-ef has been
migrated to rust-dix 0.6 and updated to match the new resolution API.

### Changed — rust-dix 0.6 sync

- **`rust-dicore` renamed to `rust-dix`** (upstream): crate name, dependency
  declaration, and the `rust_dix::` import path (formerly `rust_dicore::`).
- **Dependency bump**: `rust-dicore = "0.5.1"` → `rust-dix = "0.6"` in
  `crates/core/Cargo.toml`.
- **Import path**: `rust_dicore::*` → `rust_dix::*` in `crates/core/src/di.rs`
  and test files.
- **Re-exports**: `pub use rust_dix::{ServiceCollection, ServiceProvider}` in
  `di.rs`.

### Breaking — rust-dix 0.6 resolution API (affects user code)

The following changes in rust-dix 0.6 affect all rust-ef consumers that call
`provider.get()`, `provider.get_owned()`, or `provider.get_keyed_owned()`
directly. `#[derive(Inject)]`-generated constructors are unaffected — the
macro emits the correct unwrap internally.

1. **`ServiceCollection::build()` now returns `Arc<ServiceProvider>` directly**
   (previously returned `ServiceProvider`, requiring user code to wrap in
   `Arc::new()`). Remove the manual `Arc::new()` wrap:
   ```rust
   // Before (rust-dicore 0.5.1):
   let provider = Arc::new(ServiceCollection::new().add_dbcontext(...).build().unwrap());

   // After (rust-dix 0.6):
   let provider: Arc<ServiceProvider> = ServiceCollection::new().add_dbcontext(...).build().unwrap();
   ```

2. **`get()` / `get_owned()` / `get_keyed_owned()` now return `Result<_, RdiError>`**
   (previously returned the value directly, panicking on failure). Add
   `.unwrap()` or `.expect("...")` (or `?` in functions returning `Result`):
   ```rust
   // Before:
   let ctx: DbContext = provider.get_owned();
   let ctx: Arc<DbContext> = scope.get();

   // After:
   let ctx: DbContext = provider.get_owned()?;
   let ctx: Arc<DbContext> = scope.get()?;
   ```

3. **`create_scope()` moved to the `ScopeFactory` trait** — must be imported
   before calling:
   ```rust
   use rust_dix::scope::ScopeFactory;
   let scope = provider.create_scope();
   ```
   Alternatively, use the inherent `.scope()` method on `ServiceProvider`,
   which does not require a trait import.

### Migration — 1.3.x → Unreleased (rust-dix 0.6)

1. **Cargo.toml**: replace `rust-dicore = "0.5.1"` with `rust-dix = "0.6"`.
2. **Imports**: replace `use rust_dicore::*;` with `use rust_dix::*;` and
   `rust_dicore::ServiceCollection` with `rust_dix::ServiceCollection`.
3. **Provider construction**: drop the `Arc::new()` wrap around
   `ServiceCollection::...build().unwrap()`. `build()` already returns
   `Arc<ServiceProvider>`.
4. **Resolution calls**: append `?` (or `.unwrap()` / `.expect("...")`) to all
   `provider.get()`, `provider.get_owned()`, `provider.get_keyed_owned()`,
   `scope.get()`, `scope.get_owned()`, `scope.get_keyed_owned()` calls.
5. **Scope creation**: add `use rust_dix::scope::ScopeFactory;` before calling
   `provider.create_scope()` (or switch to `.scope()`).
6. **Attribute macros** (if used directly): `#[rust_dicore::inject]` →
   `#[rust_dix::inject]`. rust-ef itself does not use this attribute directly;
   handler structs use `#[derive(Inject)]` from `rust-ef-macros` which is
   unaffected.
7. **Behavior**: identical — ServiceProvider is still the root scope, Scoped
   caches per-scope, `get_owned()` bypasses the cache, `from_injected()`
   collects `#[inject]`-annotated services via `inventory`.

### Documentation

- Updated all `rust-dicore` / `rust_dicore` references in `di.rs`,
  `crates/core/README.md`, top-level `README.md`, and `docs/rust-ef/**` to
  `rust-dix` / `rust_dix`.
- Updated doc examples to reflect the new `Result`-returning resolution API
  (`get_owned().unwrap()` / `get_owned()?` instead of bare `get_owned()`).
- Noted the `ScopeFactory` trait import requirement in scope-creation
  examples.

---

## [1.3.1] — 2026-06-30 — rust-dicore 0.5.1 sync

Upgrades to `rust-dicore 0.5.1`. The 0.5.1 macros enforce **explicit field
marking** (LRDI rule 8): bare `T` fields MUST be marked `#[inject(owned)]` and
`Arc<T>` fields MUST be marked `#[inject]`. Unmarked fields fall back to
`Default::default()`. The previous 0.5.0 "auto-detect bare T" behavior is
removed — this is a breaking change for any handler struct that relied on
implicit owned resolution.

### Changed — 0.5.1 sync

- **`rust-dicore` upgraded from 0.5.0 to 0.5.1**: `rust-dicore-macros` also
  upgraded to 0.5.1. The `gen_field_init` macro now treats unmarked fields as
  internal state (`Default::default()`), matching the documented LRDI rule 8.
- **Handler structs require explicit `#[inject(owned)]`**: every bare
  `ctx: DbContext` field in `#[derive(Inject)]` structs MUST now be marked
  `#[inject(owned)]`. `Arc<T>` fields MUST be marked `#[inject]`.
- **Documentation**: updated all handler examples across `di.rs`,
  `db_context.rs`, README files, `docs/rust-ef/**`, and the `lref` skill
  templates/references to reflect the explicit marking requirement. Fixed
  `rust_dicore::` fully-qualified paths to `use rust_dicore::*;` (LRDI rule 6).
- **`lref` skill**: fixed `keyed_transient` → `keyed_scoped` in
  `architecture.md`; fixed `rust_dicore::ServiceProvider` fully-qualified path
  in `di-setup.rs` template.

### Migration — 1.3.0 → 1.3.1 (0.5.1 sync)

1. **Handler structs**: add `#[inject(owned)]` to every bare `T` field
   (e.g. `ctx: DbContext`). Add `#[inject]` to every `Arc<T>` field.
2. **Unmarked fields**: confirm they implement `Default` — unmarked fields now
   use `Default::default()` instead of auto-detection.
3. **Imports**: ensure `use rust_dicore::*;` at the file top — fully-qualified
   `rust_dicore::` paths are forbidden (LRDI rule 6).

### Added — 0.5.1 (upstream)

- `try_get_owned::<T>() -> Option<T>` — non-panicking owned resolution
  (returns `None` for unregistered or Singleton services).
- `Option<T>` field support in `#[derive(Inject)]` → resolves via
  `try_get_owned`; `Option<Arc<T>>` field → resolves via `try_get`.

---

## [1.3.0] — 2026-06-29 — Owned Resolution + rust-dicore 0.5.0

Upgrades to `rust-dicore 0.5.0` with owned resolution support, eliminating the
`Arc<DbContext>` + `&mut self` tension without interior mutability. Handlers
now own `DbContext` directly via `get_owned()`, enabling idiomatic `&mut self`
access — no `Arc<Mutex>`, no locks, no `unsafe`.

### Added — v1.3

- **Owned resolution**: `IServiceResolver::get_owned::<T>() -> T` bypasses
  the DI cache and returns a fresh owned instance. `#[derive(Inject)]`
  auto-detects bare `T` fields (vs `Arc<T>`) and resolves them via
  `get_owned()`. Scoped/Transient services support owned resolution;
  Singleton returns `None` (shared instance cannot be owned).
- **End-to-end handler pattern**: `#[inject(scoped)]` + `ctx: DbContext`
  (bare field) + `handle(&mut self)` + `self.ctx.set::<T>()` /
  `self.ctx.save_changes()`. Each request gets a fresh `DbContext` —
  aligned with EFCore + ASP.NET Core DI semantics.
- **Keyed owned resolution**: `get_keyed_owned::<T>("key")` for multi-DB
  scenarios.
- 4 integration tests in `owned_injection_tests.rs` verifying the complete
  flow: `#[inject(scoped)]` → `get_owned::<Handler>()` → `handle(&mut self)`
  → `self.ctx.set::<T>().add()` → `self.ctx.save_changes()`.
- 2 new scoped lifecycle tests: `owned_resolution_returns_fresh_instance`
  and `owned_resolution_bypasses_scope_cache`.

### Changed — v1.3

- **`rust-dicore` upgraded from 0.3.2 to 0.5.0**: ServiceProvider is now the
  root scope — Scoped services resolved from root are cached in
  `root_scoped_cache` (same instance per call), matching EFCore semantics.
  Previously root resolution degraded to transient.
- **Handler templates**: All examples updated from `ctx: Arc<DbContext>` +
  `handle(&self)` to `ctx: DbContext` (owned) + `handle(&mut self)` +
  `self.ctx.` prefix. Fixed template bug where `ctx.` was used without
  `self.` prefix in handler methods.
- **`#[inject(scoped)]` requirement**: Handler trait impls MUST use
  `#[inject(scoped)]`, not bare `#[inject]` (which defaults to Singleton
  and causes captive dependency errors with the Scoped `DbContext`).
- **Documentation**: Comprehensive updates across `di.rs`, `db_context.rs`,
  `webapp-integration.md`, `quickstart.md`, `architecture.md`,
  `advanced.md`, `pitfalls.md`, `di-registration.md`, `keyed-databases.md`,
  `dbcontext-and-di.md`, `multi-tenancy-foundation.md`, README files, and
  skill templates. Fixed UTF-8 encoding corruption in multiple doc files.

### Migration — v1.2 → v1.3

1. **Handler structs**: Change `ctx: Arc<DbContext>` to `ctx: DbContext`
   (bare field). `#[derive(Inject)]` auto-detects and uses `get_owned()`.
2. **Handler methods**: Change `handle(&self, ...)` to `handle(&mut self, ...)`
   and prefix all context calls with `self.` (e.g. `self.ctx.set::<T>()`).
3. **Handler registration**: Change `#[inject]` to `#[inject(scoped)]` on
   trait impl blocks to avoid captive dependency errors.
4. **Root resolution**: `provider.get::<DbContext>()` now returns the same
  instance per call (root scope cache). Use `provider.get_owned::<DbContext>()`
  for a fresh instance each call.
5. **`Arc<DbContext>` still supported** for shared `&self`-only scenarios
   (e.g. `ensure_created()`), but cannot call `set::<T>()` or
   `save_changes()` (requires `&mut self`).

---

## [1.1.0] — 2026-06-27 — Query Fidelity + Entity Auto-Discovery

Enhances query expressiveness with lazy loading, native type binding fixes,
dialect bug fixes, IN/NOT IN subqueries, CTE / window function support,
CTE syntax sugar for the `linq!` macro, and introduces automatic entity
discovery with multi-database context key support.

### Added — v1.1

- **Lazy Loading** (opt-in): `DbContextOptionsBuilder::use_lazy_loading(bool)`
  flag propagated through `DbSet` → `QueryBuilder` → `to_list()`. Navigation
  containers (`BelongsTo` / `HasMany` / `HasOne`) gain `is_loaded()`,
  `set_lazy_context()`, and async `load()` methods. `ILazyInit` trait
  auto-generated by `#[derive(EntityType)]`; `MAX_LAZY_DEPTH = 16` recursion
  guard. 7 integration tests in `lazy_loading_tests.rs`.
- **IN / NOT IN subquery support**: `b.field.in_subquery(|p: Post| p.blog_id)`
  syntax in `linq!` macro (Forms A/B and C). New `BoolExpr::InSubquery` /
  `BoolExpr::NotInSubquery` variants and `InSubquerySpec` struct. 6 integration
  tests in `in_subquery_tests.rs`.
- **CTE (Common Table Expressions)**: Two usage modes:
  - **Runtime API (raw mode)**: `QueryBuilder::with_cte_internal(name, sql,
    params, columns)` and `QueryBuilder::from_cte(name)`. CTE parameters are
    prepended to the query parameter vector; `WITH name AS (...)` prefix
    emitted before SELECT. Explicit column lists supported
    (`WITH name (c1, c2) AS (...)`).
  - **`linq!` macro syntax sugar (typed mode)**: `linq!(with name as |e: T|
    ...; from name)` compiles the closure body into a `BoolExpr` via
    `compile_bool_expr`, generating a type-safe CTE whose body
    `SELECT * FROM <table> WHERE <expr>` uses provider-correct placeholders
    (`?` on SQLite/MySQL, `$N` on PostgreSQL). Eliminates raw SQL strings,
    manual parameter management, and PostgreSQL placeholder mismatches.
    `QueryBuilder::with_cte_typed(name, table, where_expr)` is the underlying
    builder method. 9 integration tests in `cte_syntax_tests.rs`.
- **Window functions**: `linq!(window ...)` clause with 10 function kinds
  (`ROW_NUMBER`, `RANK`, `DENSE_RANK`, `LAG`, `LEAD`, `SUM`, `COUNT`, `AVG`,
  `MIN`, `MAX`). `WindowSpec` AST with `PARTITION BY` and `ORDER BY` support.
  `WindowFuncKind` enum and `WindowSpec::to_sql()` compile window projections
  with dialect-specific identifier quoting. 12 integration tests in
  `window_function_tests.rs`.
- `QueryState::all_params()` method returning CTE params ++ WHERE/HAVING params
  in correct placeholder order.
- `WindowFuncKind`, `WindowSpec`, `CteSpec` exported in `prelude`.
- **Automatic entity discovery**: `DbContext::from_options()` now
  auto-discovers all entities registered via `#[derive(EntityType)]` and
  applies all `#[entity(T)]` Fluent configurations. Developers no longer
  need to manually call `ctx.discover_entities()`. The call is idempotent —
  manual `discover_entities()` calls after `from_options()` are safe no-ops.
- **Multi-database context key support**:
  - `#[context("key")]` attribute on entity structs tags them for a
    specific keyed `DbContext`. Entities without this attribute default to
    the default context (`None`).
  - `#[entity(T, "key")]` attribute on config impls applies Fluent
    configurations only to the matching keyed context.
  - `DbContextOptionsBuilder::context_key(key)` sets the context key on
    options. `add_dbcontext_keyed(key, ...)` sets it automatically.
  - `discover_entities()` filters entity registrations and config
    registrations by the context's key, ensuring each `DbContext` only
    manages entities belonging to its context.
- **`#[entity(T)]` macro** (renamed from `#[entity_config(T)]`): The
  attribute macro for `impl IEntityTypeConfiguration<T>` blocks is now
  named `#[entity(T)]` for a cleaner API. The old name `#[entity_config(T)]`
  has been removed without a deprecated alias.
- `DbContext::model_builder()` read-only accessor and
  `DbContext::entity_metas_contains::<T>()` type-check method.
- 6 integration tests in `multi_db_context_tests.rs` covering context key
  tagging, filtering, and Fluent config override isolation.

### Fixed — v1.1

- **PostgreSQL HAVING placeholder bug**: `HavingExpr::to_sql` hardcoded `?`
  placeholder. Fixed to use `ISqlGenerator::parameter_placeholder()` with
  shared `param_idx` for contiguous `$N` numbering across WHERE and HAVING.
- **PostgreSQL LIMIT/OFFSET dialect bug**: `to_sql_with` didn't use
  `gen.pagination()`. Fixed to delegate to the dialect-specific generator so
  PostgreSQL emits `OFFSET x LIMIT y` and MySQL handles offset-only via
  sentinel LIMIT.
- **PostgreSQL native chrono/uuid parameter binding**: `DbValue` now preserves
  `chrono::DateTime` / `chrono::NaiveDateTime` / `uuid::Uuid` types instead of
  collapsing to `String`, enabling native PostgreSQL type inference.
- **PostgreSQL multi typed CTE placeholder collision**: `to_sql_with` reset
  `cte_idx` to 1 inside the CTE `.map()` closure, causing multiple typed CTEs
  to emit duplicate `$1` placeholders on PostgreSQL. Fixed to use a
  `running_idx` that accumulates across CTEs so `$N` stays contiguous with
  `all_params()` order. Regression covered by 4 new PG dialect tests in
  `cte_syntax_tests.rs`.

### Changed — v1.1

- `QueryBuilder::to_list` and all terminal methods now require `T: ILazyInit`
  bound (auto-satisfied by `#[derive(EntityType)]`).
- `QueryState` gains `windows: Vec<WindowSpec>` and `ctes: Vec<CteSpec>` fields.
- `CteSpec` gains `table: String` and `where_expr: Option<BoolExpr>` fields to
  support typed mode, and is now `#[non_exhaustive]` to prevent direct
  construction outside the crate. Existing `with_cte_internal` API is
  unchanged (new fields default to empty). Use `with_cte_internal` /
  `with_cte_typed` to create CTE specifications.
- `to_sql_with` emits window function projections in the SELECT list and CTE
  `WITH` prefix before SELECT; `param_idx` starts at `1 + cte_param_count` for
  PostgreSQL placeholder continuity. Typed CTE bodies are compiled at this
  point via `compile_bool_expr` with provider-correct placeholders.
- All execution sites updated to use `QueryState::all_params()` for CTE
  parameter ordering.
- `DbContext::from_options()` now automatically calls `discover_entities()`,
  populating entity metadata and applying `#[entity(T)]` Fluent configurations
  without requiring manual registration. The call is idempotent — subsequent
  `discover_entities()` calls are safe no-ops.
- `discover_entities()` filters `EntityRegistration` and
  `EntityConfigRegistration` entries by the context's `context_key`, ensuring
  each `DbContext` only manages entities belonging to its context.
- `EntityRegistration` and `EntityConfigRegistration` gain a
  `context_key: Option<&'static str>` field for multi-database context
  filtering. Existing inventory submissions generated by older macro versions
  remain source-compatible because the field is populated by the macro, not by
  user code.
- `DbContextOptions` and `DbContextOptionsBuilder` gain a `context_key`
  field/setter; `DbContext` stores a `context_key: Option<String>` for use
  during `discover_entities()` filtering.

### Removed — v1.1

- **`#[entity_config(T)]` attribute macro** (breaking change): renamed to
  `#[entity(T)]` for a cleaner API. The old name has been removed **without a
  deprecated alias** to keep the framework's final shape clean. Migrate by
  renaming `#[entity_config(T)]` to `#[entity(T)]` (and
  `#[entity_config(T, "key")]` to `#[entity(T, "key")]` for keyed contexts).
  The `rust_ef::entity_config` re-export is gone; use `rust_ef::entity` (or the
  prelude) instead.

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
- `docs/archive/PRODUCTION_READINESS_SPEC_v1.6.md` readiness 98% → 100%, all 1.0 GA
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

[2.0.1]: https://github.com/Rust-Framework/rust-ef/releases/tag/v2.0.1
[1.4.0]: https://github.com/Rust-Framework/rust-ef/releases/tag/v1.4.0
[1.8.1]: https://github.com/Rust-Framework/rust-ef/releases/tag/v1.8.1
[1.1.0]: https://github.com/Rust-Framework/rust-ef/releases/tag/v1.1.0
[1.0.0]: https://github.com/Rust-Framework/rust-ef/releases/tag/v1.0.0
[0.5]: https://github.com/Rust-Framework/rust-ef/releases/tag/v0.5
[0.4]: https://github.com/Rust-Framework/rust-ef/releases/tag/v0.4
[0.3.5]: https://github.com/Rust-Framework/rust-ef/releases/tag/v0.3.5
[0.3]: https://github.com/Rust-Framework/rust-ef/releases/tag/v0.3
[0.2]: https://github.com/Rust-Framework/rust-ef/releases/tag/v0.2
[0.1]: https://github.com/Rust-Framework/rust-ef/releases/tag/v0.1
