# EFCore 实体配置对齐矩阵

本文逐项对照 EF Core 的 `EntityTypeBuilder` / `PropertyBuilder` / `ModelBuilder` 配置能力与 rust-ef 的等价能力，避免业务系统为了补 schema/DDL 能力而自己写迁移或 SQL。

图例：✅ 已支持 · ⚠️ 部分支持/有边界 · ❌ 不适用（Rust 语言模型或本项目三方数据库不支持） · 🔜 规划中

## 1. EntityTypeBuilder

| EF Core API | rust-ef 等价 | 状态 | 说明 |
|---|---|---|---|
| `ToTable("t")` | `entity.to_table("t")` / `#[table("t")]` | ✅ | |
| `ToTable("t", "schema")` | `entity.to_schema("schema")` / `#[table("t", schema = "schema")]` | ✅ | 生成 `"schema"."t"`；派生属性会让导航 `related_table` 也带 schema |
| `ToView("v")` | `entity.to_view("v")` / `#[table("v", view)]` | ✅ | 视图不参与建表/迁移 DDL；视图本身由外部管理 |
| `HasKey(...)` | `entity.has_key_named(...)` / `has_keys(...)` / `#[primary_key]` | ✅ | 复合主键可用 `has_keys(&["a","b"])` |
| `HasAlternateKey(...)` | `entity.has_alternate_key(&[...])` | ✅ | 生成 `ak_<table>_<cols>` 唯一索引，可被复合外键引用 |
| `HasNoKey()` | `entity.has_no_key()` / `#[keyless]` | ✅ | 不生成 PRIMARY KEY；`find/remove` 不可用（无键） |
| `HasIndex(...).IsUnique()` | `entity.has_index(&[...]).is_unique()` | ✅ | |
| `.HasDatabaseName(...)` | `.has_name("...")` | ✅ | |
| `.HasFilter(...)` | `.has_filter(linq!(\|x: T\| ...))` | ✅ | PG/SQLite 原生；MySQL 对唯一索引用函数索引模拟 |
| `.IncludeProperties(...)` | `.include(&[...])` | ✅ | PostgreSQL `INCLUDE`；MySQL/SQLite 忽略并输出 warning |
| `HasOne/HasMany/WithOne/WithMany` | `BelongsTo<T>` / `HasOne<T>` / `HasMany<T>` / `HasMany<T, Join>` | ✅ | 关系由导航字段声明；`through` 用于 M:N |
| `HasForeignKey(...)` | `#[foreign_key(Target)]` / `entity.has_foreign_key(...)` | ✅ | 标量属性与复合外键都进入 DDL |
| `HasPrincipalKey(...)` | `has_foreign_key(..., referenced_columns)` | ✅ | 显式给出被引用列（可指向 alternate key） |
| `OnDelete(...)` | `#[on_delete(Cascade)]` / `.on_delete(DeleteBehavior)` | ✅ | |
| `HasCheckConstraint(...)` | `entity.has_check(name, sql)` | ✅ | PG/MySQL/SQLite 建表内联 |
| `HasQueryFilter(...)` | `entity.has_query_filter(linq!(\|x: T\| ...))` | ✅ | |
| `HasData(...)` | `entity.has_data(&[...])` / `#[derive(Default)]` | ✅ | `ensure_created` 时插入种子 |
| `HasComment(...)` | `entity.has_comment("...")` / `#[table("t", comment = "...")]` | ✅ | MySQL 内联/ALTER；PG `COMMENT ON TABLE`；SQLite 忽略 |
| `Ignore(...)` | `entity.ignore("field")` / `#[not_mapped]` | ✅ | |
| `HasSequence(...)` | `#[sequence("name")]` | ⚠️ | PostgreSQL 序列；无 Fluent 序列名重载 |
| `ExcludeFromMigrations()` | `entity.to_view(...)`（跳过 DDL） | ⚠️ | 没有独立标记；外部管理的表用视图语义跳过 |
| `OwnsOne/OwnsMany` | — | 🔜 | 可用扁平列或 `#[column_type("JSON")]` 存储 |
| `ComplexProperty` (EF8) | — | 🔜 | 同上 |
| `Table splitting` / `Entity splitting` | — | ❌ | 需要跨实体列映射，Rust 侧无稳定语义 |
| `UseTph/UseTpt/UseTpc` + `HasDiscriminator` | — | ❌ | Rust 无继承；组合/枚举标签替代 |
| `HasAnnotation(...)` | — | ❌ | 无 provider 扩展注解模型 |

## 2. PropertyBuilder

| EF Core API | rust-ef 等价 | 状态 | 说明 |
|---|---|---|---|
| `IsRequired()` | `#[required]` / `property_named(...).is_required()` | ✅ | |
| `IsNullable()` | `property_named(...).is_nullable()`（别名 `.is_optional()`） | ✅ | |
| `HasMaxLength(n)` | `#[max_length(n)]` / `.has_max_length(n)` | ✅ | |
| `HasPrecision(p, s)` | `#[precision(p, s)]` / `.has_precision(p, s)` | ✅ | Decimal → `DECIMAL(p,s)` / `NUMERIC(p,s)`；时间类型 → `DATETIME(p)` / `TIMESTAMP(p)` |
| `HasColumnType("...")` | `#[column_type("...")]` / `.has_column_type("...")` | ✅ | 原样输出（如 `JSON`、`MEDIUMTEXT`、`ENUM(...)`） |
| `IsFixedLength()` | `#[fixed_length]` / `.is_fixed_length()` | ✅ | `CHAR(n)`（`n = max_length`，缺省 1） |
| `HasCollation("...")` | `#[collation("...")]` / `.has_collation("...")` | ✅ | 列级 `COLLATE` |
| `HasComment("...")` | `#[comment("...")]` / `.has_comment("...")` | ✅ | MySQL 内联；PG `COMMENT ON COLUMN` |
| `HasDefaultValue(v)` | `.has_default("1")` / `.has_default("'PENDING'")` | ✅ | 值是 SQL 字面量（字符串需自带引号） |
| `HasDefaultValueSql("...")` | `.has_default("CURRENT_TIMESTAMP")` | ✅ | 与上一行同一 API（raw SQL） |
| `HasComputedColumnSql("...", stored)` | `#[computed("...", stored)]` / `.has_computed_column_sql("...").is_stored()` | ✅ | 计算列自动排除出 INSERT/UPDATE |
| `IsConcurrencyToken()` | `#[concurrency_check]` / `.is_concurrency_token()` | ✅ | 乐观并发 WHERE 条件 |
| `IsRowVersion()` | `#[concurrency_check]` + 应用递增 | ⚠️ | 三方数据库没有统一的 rowversion 类型 |
| `ValueGeneratedOnAdd()` | `#[auto_increment]` / `#[sequence("...")]` / `.has_default(...)` | ✅ | 键用自增/序列；非键用 DEFAULT |
| `ValueGeneratedOnAddOrUpdate()` | `.has_computed_column_sql(...)` | ✅ | |
| `ValueGeneratedNever()` | 默认即应用写入 | ✅ | |
| `HasConversion<T>()` | `#[derive(DbEnum)]`（字符串枚举） | ⚠️ | 自定义值对象仍实现 `From<X> for DbValue` + `TryFrom<DbValue> for X`（两个 impl） |
| `IsUnicode(false)` | `.charset(...)` / `.has_collation(...)` | ❌ | SQL Server 专属；MySQL 用表/列字符集 |
| `HasField(...)` / `UsePropertyAccessMode(...)` | — | ❌ | Rust 没有 backing field/proxy 模型 |
| `HasSentinel(...)` | — | ❌ | 由 `Option<T>` 表达 |
| Shadow properties | — | ❌ | Rust 无运行时影子属性 |

## 3. ModelBuilder / 全局配置

| EF Core API | rust-ef 等价 | 状态 |
|---|---|---|
| `OnModelCreating` | `#[entity(T)] impl IEntityTypeConfiguration<T>`（自动发现） | ✅ |
| `HasDefaultSchema` | 每个实体 `to_schema(...)` / `#[table(..., schema = "...")]` | ⚠️（无全局默认 schema） |
| `UseCollation` | `entity.collate(...)` / `.has_collation(...)` | ✅（表级/列级） |
| `HasSequence` | `#[sequence]` | ⚠️ |
| `HasDbFunction` | 原生 SQL / `ExecuteUpdateBuilder` / raw query | ⚠️ |
| `HasPostgresExtension` | 安装器/迁移脚本手写 | ❌ |

## 4. 迁移相关

- `ToView` / `#[keyless]`：不生成 `CREATE TABLE`；`ensure_deleted` 也不会删除视图。
- 计算列：`CREATE TABLE` 内联 `GENERATED ALWAYS AS (...)`；`INSERT/UPDATE` 自动排除。
- 注释：MySQL 建表内联/`ALTER TABLE COMMENT`；PG `COMMENT ON TABLE/COLUMN`；SQLite 忽略。
- schema/表重命名后，全局查询过滤仍按**编译期表名**匹配（与导航/懒加载一致）；跨 schema 的导航请用 `#[table(..., schema = "...")]` 派生属性，让 `TABLE`/`related_table` 同步带 schema。

## 5. 安全边界

- 所有接收 SQL 的配置项（`has_column_type`、`has_default`、`has_check`、`has_computed_column_sql`、`has_index_expression`、`has_collation`、`has_comment`、`has_filter`）都是**开发者编译期配置**，绝不能拼接用户输入；运行时值请用 `linq!` / 参数化查询。
- 表名、schema、迁移 id 同样来自代码或 CLI 运维输入：DDL 标识符按方言引用，`MigrationStore` 拒绝空 id、路径分隔符、`..`、`:` 和控制字符（防目录穿越）。
- CLI 生成的迁移 id 带 Unix 时间戳前缀（`{seconds}_{name}`），保证 `load_all` 的应用顺序；并发发布只允许一个 `migrate`/`apply` job。

## 6. 仍需业务侧补的能力（明确列出）

1. **任意值对象的转换**：`HasConversion` 的完整表达式转换未实现；枚举用 `#[derive(DbEnum)]`，其他值对象实现两个转换 trait 即可，不需要改框架。
2. **继承/owned/complex/表拆分**：Rust 无继承与运行时影子属性，建议用组合 + JSON 列（`.has_column_type("JSON")`）建模。
3. **数据库专属特性**（PostgreSQL `xmin`、SQL Server `rowversion`/`unicode`、Postgres 扩展）：通过 `has_column_type` / 原生 SQL 处理。
4. **全局默认 schema**：目前按实体配置；如需全局约定，可在基类 `configure` 中统一 `to_schema`。
