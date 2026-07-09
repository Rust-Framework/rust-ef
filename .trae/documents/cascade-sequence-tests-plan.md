# 级联保存测试 + 显式序列支持

## Summary

两个任务：
1. **确认回填触发条件 + 新增显式序列支持** — 当前回填只在 `is_auto_increment && is_primary_key` 时触发。新增 `#[sequence("seq_name")]` 属性，PostgreSQL 使用 `CREATE SEQUENCE + DEFAULT nextval()`，作为回填的另一个触发条件。
2. **规划级联保存测试文件** — `cascade_save_tests.rs` 不存在，需创建 6 个测试用例验证已实现的级联保存全流程。

## Current State Analysis

### 回填触发条件（已确认）

当前回填逻辑在 [change_executor.rs:60-63](file:///d:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs#L60-L63)：
```rust
let auto_inc_pk = scalar_props
    .iter()
    .find(|p| p.is_auto_increment && p.is_primary_key);
```
- **唯一触发条件**：`is_auto_increment && is_primary_key`
- **PostgreSQL SERIAL/BIGSERIAL/SMALLSERIAL**：由 `is_auto_increment` 映射（[type_mapping.rs:34-41](file:///d:/GitCode/RF/rust-ef/crates/postgres/src/type_mapping.rs#L34-L41)）
- **没有独立的序列机制**：无 `#[sequence]` 属性、无 `is_sequence` 元数据、无 `nextval()` 支持
- **回填方式**：PostgreSQL 用 `RETURNING *`；SQLite/MySQL 用 `last_insert_rowid()` / `LAST_INSERT_ID()`

### 级联保存实现（已完成）

[db_context.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs) 已实现完整级联保存：
- `ErasedSetOps` trait 含 9 个级联方法（drain_cascade_children, add_cascade_child, get_pk_at, set_fk_at, insert_added, upsert_added, update_modified, delete_deleted, entry_count）
- `save_changes` 含级联抽取循环 + 拓扑排序 + INSERT/FK修复/M2M连接/UPDATE/DELETE 全流程
- **测试文件 `cascade_save_tests.rs` 不存在** — 这是本计划要创建的

### 关键文件清单

| 文件 | 角色 |
|------|------|
| [crates/core/src/metadata.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/metadata.rs) | PropertyMeta 定义（需加 is_sequence, sequence_name） |
| [crates/macros/src/entity.rs](file:///d:/GitCode/RF/rust-ef/crates/macros/src/entity.rs) | 宏处理属性（需加 #[sequence] 解析） |
| [crates/core/src/change_executor.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs) | 回填触发（需加 is_sequence 条件） |
| [crates/core/src/migration/types.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration/types.rs) | SnapshotColumn + map_column_type（需加序列 DDL） |
| [crates/core/src/migration/engine.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration/engine.rs) | create_snapshot（需复制序列字段） |
| [crates/core/src/migration/engine_sql.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration/engine_sql.rs) | generate_ddl_sql（需加 CREATE SEQUENCE + DEFAULT） |
| [crates/core/tests/cascade_save_tests.rs](file:///d:/GitCode/RF/rust-ef/crates/core/tests/cascade_save_tests.rs) | 测试文件（需创建） |

---

## ⚠️ 需要提醒的问题

### 1. `set_auto_increment_key` 方法名误导
[entity.rs:68](file:///d:/GitCode/RF/rust-ef/crates/core/src/entity.rs#L68) 的 `set_auto_increment_key` 方法将被序列回填复用（功能相同：设置 DB 生成的 PK）。方法名含 "auto_increment" 但实际用于两种场景。**不重命名**（会破坏所有现有实体），但在文档注释中说明它也用于序列回填。

### 2. 非 PostgreSQL 提供程序的序列回退
`#[sequence]` 在 SQLite/MySQL 上**回退为 auto_increment 行为**：
- DDL 使用 `INTEGER PRIMARY KEY AUTOINCREMENT`（SQLite）或 `INT AUTO_INCREMENT`（MySQL）
- 序列名被忽略
- 回填通过 `last_insert_rowid()` / `LAST_INSERT_ID()` 正常工作
- **不报错**（允许跨数据库代码运行），但序列语义仅在 PostgreSQL 上完整

### 3. 序列与 auto_increment 互斥
`#[sequence]` 和 `#[auto_increment]` 同时存在时，宏会**编译错误**。这是设计决策——两者是不同的 DB 生成策略，不应混用。

### 4. 自引用 FK 修复（已在级联保存中处理）
级联保存已处理自引用关系：子实体先以 FK=0 插入，父实体 PK 回填后，通过延迟 `UPDATE child SET fk = parent_pk WHERE pk = child_pk` 修复。这是已有行为，测试将验证。

### 5. M2M 已有子实体
级联抽取时，若 M2M 子实体 PK > 0，`add_cascade_child` 会 `attach` 为 Unchanged（不重新插入），只插入连接表行。测试将覆盖此场景。

---

## Proposed Changes

### Part A: 显式序列支持

#### Step 1: PropertyMeta 扩展
**文件**: [crates/core/src/metadata.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/metadata.rs)

在 `PropertyMeta` struct 中添加两个字段（在 `is_auto_increment` 之后）：
```rust
/// Whether this property is backed by a database sequence (PostgreSQL).
/// On non-PG providers, falls back to auto_increment behavior.
pub is_sequence: bool,
/// The sequence name when `is_sequence` is true (PostgreSQL only).
pub sequence_name: Option<Cow<'static, str>>,
```

在 `PropertyMetaBuilder` 中：
- `new()` 初始化 `is_sequence: false, sequence_name: None`
- 添加 `is_sequence(mut self, v: bool) -> Self`
- 添加 `sequence_name(mut self, name: &'static str) -> Self`

#### Step 2: 宏解析 #[sequence] 属性
**文件**: [crates/macros/src/entity.rs](file:///d:/GitCode/RF/rust-ef/crates/macros/src/entity.rs)

1. 添加 `extract_sequence_name` 辅助函数（仿 `extract_column_name`）：
```rust
fn extract_sequence_name(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("sequence") {
            if let Ok(lit_str) = attr.parse_args::<syn::LitStr>() {
                return Some(lit_str.value());
            }
        }
    }
    None
}
```

2. 在字段处理循环中（约 line 68 附近）：
```rust
let is_sequence = extract_sequence_name(&field.attrs).is_some();
let sequence_name = extract_sequence_name(&field.attrs);
// 互斥检查
if is_auto_increment && is_sequence {
    return syn::Error::new(field.span(), "#[auto_increment] and #[sequence] are mutually exclusive").to_compile_error().into();
}
```

3. 在 PropertyMeta 生成中（约 line 382-398）添加：
```rust
is_sequence: #is_sequence,
sequence_name: #sequence_name_lit,
```
其中 `sequence_name_lit` 是 `Option<Cow::Borrowed("seq_name")>` 或 `None`。

4. 在 `auto_inc_pk_ident` 逻辑中（约 line 82-84），扩展为序列也触发：
```rust
if is_auto_increment || is_sequence {
    auto_inc_pk_ident = Some(field_name);
}
```
这使 `set_auto_increment_key` 宏生成也对序列 PK 生效。

5. 在 `#[derive(EntityType)]` 的 `entity_type.rs` 中注册 `sequence` 为已知属性（避免 unknown attribute 警告）。

#### Step 3: SnapshotColumn 扩展
**文件**: [crates/core/src/migration/types.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration/types.rs)

在 `SnapshotColumn` 中添加（在 `is_auto_increment` 之后）：
```rust
pub is_sequence: bool,
pub sequence_name: Option<String>,
```

更新 `map_column_type`：在 `is_auto_increment` 检查之前，添加序列处理（PostgreSQL only）：
```rust
if col.is_sequence {
    if tn.ends_with("i32") {
        return match self {
            MigrationDialect::Postgres => "INTEGER".into(),  // DEFAULT nextval 在 DDL 中单独处理
            MigrationDialect::MySql => "INT AUTO_INCREMENT".into(),  // 回退
            MigrationDialect::Sqlite => "INTEGER".into(),  // 回退
        };
    }
    if tn.ends_with("i64") {
        return match self {
            MigrationDialect::Postgres => "BIGINT".into(),
            MigrationDialect::MySql => "BIGINT AUTO_INCREMENT".into(),
            MigrationDialect::Sqlite => "INTEGER".into(),
        };
    }
}
```

#### Step 4: create_snapshot 复制序列字段
**文件**: [crates/core/src/migration/engine.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration/engine.rs) (约 line 78-91)

在 `SnapshotColumn` 构造中添加：
```rust
is_sequence: p.is_sequence,
sequence_name: p.sequence_name.as_ref().map(|s| s.to_string()),
```

#### Step 5: generate_ddl_sql 添加序列 DDL
**文件**: [crates/core/src/migration/engine_sql.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration/engine_sql.rs)

在 `SchemaChange::CreateTable` 分支中，CREATE TABLE 之前，为每个序列列生成（PostgreSQL only）：
```rust
// 在 sql.push_str(&format!("{} {} (\n", ...)) 之前
if self.dialect == MigrationDialect::Postgres {
    for c in columns {
        if c.is_sequence {
            if let Some(seq_name) = &c.sequence_name {
                sql.push_str(&format!("CREATE SEQUENCE IF NOT EXISTS {};\n", q(seq_name)));
            }
        }
    }
}
```

在列定义生成中（约 line 117-128），为序列列添加 `DEFAULT nextval('seq_name')`（PostgreSQL only）：
```rust
let col_def = if c.is_sequence && self.dialect == MigrationDialect::Postgres {
    if let Some(seq_name) = &c.sequence_name {
        format!("{} DEFAULT nextval('{}') {}", q(&c.column_name), seq_name, nullable)
    } else {
        // 无序列名回退
        format!("{} {} {}", q(&c.column_name), col_type, nullable)
    }
} else {
    // 现有逻辑
    [q(&c.column_name), col_type, nullable.to_string()]
        .into_iter().filter(|s| !s.is_empty())
        .collect::<Vec<_>>().join(" ")
};
```

#### Step 6: change_executor 回填触发条件
**文件**: [crates/core/src/change_executor.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs)

修改 line 60-63：
```rust
// 旧:
let auto_inc_pk = scalar_props
    .iter()
    .find(|p| p.is_auto_increment && p.is_primary_key);

// 新:
let auto_inc_pk = scalar_props
    .iter()
    .find(|p| (p.is_auto_increment || p.is_sequence) && p.is_primary_key);
```

修改 line 53-55（INSERT 列过滤）：
```rust
// 旧:
let insert_cols: Vec<&str> = scalar_props
    .iter()
    .filter(|p| !p.is_auto_increment || !p.is_primary_key)
    .map(|p| p.column_name.as_ref())
    .collect();

// 新:
let insert_cols: Vec<&str> = scalar_props
    .iter()
    .filter(|p| !p.is_primary_key || (!p.is_auto_increment && !p.is_sequence))
    .map(|p| p.column_name.as_ref())
    .collect();
```

修改 line 81（INSERT 值过滤）：
```rust
// 旧:
if !p.is_auto_increment || !p.is_primary_key {

// 新:
if !p.is_primary_key || (!p.is_auto_increment && !p.is_sequence) {
```

#### Step 7: SnapshotColumn::default() 更新
**文件**: [crates/core/src/migration/types.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration/types.rs)

`SnapshotColumn` derive `Default`，新字段需要默认值：
```rust
is_sequence: false,
sequence_name: None,
```
（derive(Default) 会自动处理，但需确认 `Option<String>` 的 default 是 `None`）

---

### Part B: 级联保存测试文件

**文件**: `crates/core/tests/cascade_save_tests.rs`（新建）

#### 测试实体定义

```rust
#[cfg(test)]
mod cascade_save_tests {
    use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
    use rust_ef::linq;
    use rust_ef::prelude::*;
    use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

    // ── 一对多 ──
    #[derive(Debug, Clone, EntityType)]
    #[table("cascade_blogs")]
    struct CascadeBlog {
        #[primary_key]
        #[auto_increment]
        blog_id: i32,
        #[required]
        url: String,
        #[navigation]
        posts: HasMany<CascadePost>,
    }

    #[derive(Debug, Clone, EntityType)]
    #[table("cascade_posts")]
    struct CascadePost {
        #[primary_key]
        #[auto_increment]
        post_id: i32,
        #[required]
        title: String,
        #[foreign_key(CascadeBlog)]
        blog_id: i32,
        #[navigation]
        blog: BelongsTo<CascadeBlog>,
    }

    // ── 自引用 ──
    #[derive(Debug, Clone, EntityType)]
    #[table("cascade_categories")]
    struct CascadeCategory {
        #[primary_key]
        #[auto_increment]
        category_id: i32,
        #[required]
        name: String,
        #[foreign_key(CascadeCategory)]
        parent_id: i32,
        #[navigation]
        children: HasMany<CascadeCategory>,
    }

    // ── 多对多 ──
    #[derive(Debug, Clone, EntityType)]
    #[table("cascade_students")]
    struct CascadeStudent {
        #[primary_key]
        #[auto_increment]
        student_id: i32,
        #[required]
        name: String,
        #[navigation]
        courses: HasMany<CascadeCourse, CascadeEnrollment>,
    }

    #[derive(Debug, Clone, EntityType)]
    #[table("cascade_courses")]
    struct CascadeCourse {
        #[primary_key]
        #[auto_increment]
        course_id: i32,
        #[required]
        title: String,
    }

    #[derive(Debug, Clone, EntityType)]
    #[table("cascade_enrollments")]
    struct CascadeEnrollment {
        #[primary_key]
        #[auto_increment]
        enrollment_id: i32,
        #[foreign_key(CascadeStudent)]
        student_id: i32,
        #[foreign_key(CascadeCourse)]
        course_id: i32,
    }
```

#### 测试用例

**Test 1: `cascade_insert_blog_with_posts`**
- 创建 Blog，posts 含 2 个 Post
- save_changes 后验证：Blog PK 回填（>0），2 个 Post 的 blog_id == Blog PK
- 验证：`ctx.set::<CascadePost>().query().to_list()` 返回 2 条，blog_id 匹配

**Test 2: `cascade_insert_self_referential_tree`**
- 创建 Category "Root"，children 含 "Child A" 和 "Child B"
- save_changes 后验证：Root PK 回填，Child A/B 的 parent_id == Root PK
- 验证：`ctx.set::<CascadeCategory>().query().to_list()` 返回 3 条，2 个 child 的 parent_id == root category_id

**Test 3: `cascade_insert_many_to_many`**
- 创建 Student，courses 含 2 个 Course（通过 HasMany<Course, Enrollment>）
- save_changes 后验证：Student PK 回填，2 个 Course PK 回填，Enrollment 表有 2 行
- 验证：`ctx.set::<CascadeEnrollment>().query().to_list()` 返回 2 条，student_id 匹配

**Test 4: `cascade_update_ordering`**
- 先插入 Blog + Post（save_changes）
- 修改 Blog.url 和 Post.title
- save_changes 后验证：UPDATE 按拓扑序执行（Blog 先于 Post），数据已更新

**Test 5: `cascade_delete_reverse_order`**
- 先插入 Blog + Post（save_changes）
- remove_at Post，remove_at Blog
- save_changes 后验证：DELETE 按反序执行（Post 先于 Blog），表为空

**Test 6: `cascade_empty_has_many_noop`**
- 创建 Blog，posts 为空 HasMany
- save_changes 后验证：无错误，Blog 正常插入

#### 测试模式（遵循现有测试约定）
- SQLite in-memory: `builder.use_sqlite_in_memory()`
- 注册所有实体类型: `ctx.set::<T>()`
- 建表: `ctx.ensure_created().await`
- 保存: `ctx.save_changes().await`
- 查询验证: `ctx.set::<T>().query().to_list().await` 或 `linq!` 宏

---

## Assumptions & Decisions

| 假设/决策 | 理由 |
|-----------|------|
| `#[sequence("seq_name")]` 使用字符串字面量参数 | 与 `#[column("name")]` 模式一致 |
| 序列与 auto_increment 互斥（编译错误） | 两种不同的 DB 生成策略，不应混用 |
| 非 PG 提供程序回退为 auto_increment | 允许跨数据库代码运行，不报错 |
| `set_auto_increment_key` 方法名不改 | 重命名是破坏性变更；功能相同（设置 DB 生成 PK） |
| 序列 DDL 仅 PostgreSQL 生成 `CREATE SEQUENCE + DEFAULT nextval` | 其他数据库无序列概念 |
| 回填触发条件改为 `(is_auto_increment \|\| is_sequence) && is_primary_key` | 序列也是 DB 生成的 PK，需要回填 |
| INSERT 列排除序列 PK（同 auto_increment） | DB 通过 `DEFAULT nextval()` 自动生成 |
| 测试用 SQLite in-memory | 与现有测试一致；序列 PG 专属功能需单独 PG 测试（本计划不含） |

---

## Verification Steps

### 序列支持验证
1. `cargo check -p rust-ef` — 确认 metadata.rs, change_executor.rs 编译通过
2. `cargo check -p rust-ef-macros` — 确认宏编译通过
3. `cargo test -p rust-ef --test advanced_tests` — 确认现有测试不回归
4. 手动验证：在测试实体上加 `#[sequence("test_seq")]`，用 PostgreSQL 提供程序 `ensure_created`，确认生成 `CREATE SEQUENCE` + `DEFAULT nextval('test_seq')` DDL

### 级联保存测试验证
1. `cargo test -p rust-ef --test cascade_save_tests` — 6 个测试全部通过
2. `cargo test -p rust-ef` — 全量回归测试通过

### 实施顺序
1. Part A Steps 1-7（序列支持）→ verify: `cargo check` 全 crate
2. Part B（测试文件）→ verify: `cargo test --test cascade_save_tests`
3. 全量回归 → verify: `cargo test -p rust-ef`
