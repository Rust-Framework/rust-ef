# FK DDL ON DELETE 子句增强 — 实施计划

## 概述

完成级联删除功能的最后一步：在迁移 DDL 中生成 FK 的 `ON DELETE` 子句（`CASCADE`/`SET NULL`/`RESTRICT`/`NO ACTION`），使数据库级 FK 约束与应用级级联删除行为保持一致。

**当前状态**：`SnapshotColumn.fk_on_delete: Option<String>` 字段已添加（6a 完成）。运行时级联删除（db_context.rs、cascade.rs、entity.rs 宏）已完成并通过 12 个测试。

## 当前代码分析

### 关键文件与行号

| 位置 | 说明 |
|------|------|
| `migration.rs` L39-64 | `SnapshotColumn` — `fk_on_delete` 字段已存在 |
| `migration.rs` L211-216 | `SchemaChange::AddForeignKey` — 4 字段，**缺少** `on_delete` |
| `migration.rs` L486-506 | `fk_reference_for_property(meta, field_name)` → `(Option<String>, Option<String>)` — 仅查 BelongsTo 导航，不解析 delete_behavior |
| `migration.rs` L270-313 | `create_snapshot` — 构造 `SnapshotColumn`，调用 `fk_reference_for_property` |
| `migration.rs` L430-445 | `append_create_table_fks` — 构造 `AddForeignKey`，从 `fk_target(col)` 获取信息 |
| `migration.rs` L508-572 | `diff_foreign_keys` — 构造 `AddForeignKey`（3 处），交集仅比较 `rt`/`rc` |
| `migration.rs` L815-829 | `generate_up_sql_inner` AddForeignKey 分支 — 生成 `ALTER TABLE ... ADD CONSTRAINT ... FOREIGN KEY ... REFERENCES ...`，**无** ON DELETE |
| `migration.rs` L574-589 | `columns_structurally_equal` — 比较 `fk_referenced_table`/`fk_referenced_column` |
| `migration.rs` L1494-1547 | `snapshot_to_json` — 序列化列，**缺少** `fk_on_delete` |
| `migration.rs` L1549-1610 | `snapshot_from_json` — 反序列化列，**缺少** `fk_on_delete` |
| `migration.rs` L635-642 | `initial_create_with_fks` — 调用 `append_create_table_fks`，无直接构造 |
| `db_context.rs` L797-826 | `resolve_delete_behavior(nav)` — 私有函数，从 HasMany 导航解析 DeleteBehavior |
| `relations.rs` L412-420 | `DeleteBehavior` 枚举 — `Cascade`/`Restrict`/`SetNull`/`NoAction` |
| `metadata.rs` L164-205 | `NavigationMeta` — 含 `delete_behavior: Option<DeleteBehavior>`、`related_entity_meta: Option<fn() -> EntityTypeMeta>` |

### 核心设计挑战

`delete_behavior` 存储在 **principal 的 HasMany 导航**上（如 `Blog.posts`），但 `fk_reference_for_property` 处理的是 **child 的 BelongsTo 导航**（如 `Post.blog`）。需要交叉引用：从 child 的 BelongsTo 导航 → `related_entity_meta()` 获取 principal 元数据 → 在 principal 的 navigations 中查找 `related_type_id == child.type_id` 的 HasMany 导航 → 调用 `resolve_delete_behavior`。

## 设计决策

### 决策 1：始终生成 ON DELETE 子句

**选择**：始终生成（使用 `resolve_delete_behavior` 解析的默认值）。

**理由**：
- 与 EFCore 行为一致（EFCore 始终生成显式 ON DELETE）
- 确保 DB 级 FK 约束与应用级级联删除一致
- DDL 自文档化

**影响**：首次 v1.6 迁移会重建所有 FK 约束以添加 ON DELETE 子句（旧行为无此子句）。

### 决策 2：不将 `fk_on_delete` 加入 `columns_structurally_equal`

**选择**：不加入。

**理由**：
- ON DELETE 是约束属性，非列属性
- 纯 ON DELETE 变更应由 `diff_foreign_keys` 处理（DropForeignKey + AddForeignKey），不应触发冗余的 `AlterColumn`
- `diff_foreign_keys` 对两张表都存在的情况无条件调用（L397），独立于 `columns_structurally_equal`

### 决策 3：`#[on_delete]` 双侧查找

**选择**：先查 child 的 BelongsTo 导航 `delete_behavior`，若为 None 再查 principal 的 HasMany 导航。

**理由**：宏对 BelongsTo 和 HasMany 均调用 `extract_on_delete`，用户可能在任一侧配置。双侧查找更健壮。

## 实施步骤

### 步骤 1：`DeleteBehavior::to_sql_clause` 方法（relations.rs）

在 `DeleteBehavior` 枚举后添加 `impl` 块：

```rust
impl DeleteBehavior {
    /// Maps to the SQL `ON DELETE` clause keyword.
    pub fn to_sql_clause(self) -> &'static str {
        match self {
            DeleteBehavior::Cascade => "CASCADE",
            DeleteBehavior::Restrict => "RESTRICT",
            DeleteBehavior::SetNull => "SET NULL",
            DeleteBehavior::NoAction => "NO ACTION",
        }
    }
}
```

**文件**：`crates/core/src/relations.rs`（L420 之后）
**验证**：`cargo check -p rust-ef-core`

### 步骤 2：`resolve_delete_behavior` 改为 `pub(crate)`（db_context.rs）

将 L802 的 `fn resolve_delete_behavior` 改为 `pub(crate) fn resolve_delete_behavior`，使 migration 模块可调用。

**文件**：`crates/core/src/db_context.rs` L802
**验证**：`cargo check -p rust-ef-core`

### 步骤 3：`SchemaChange::AddForeignKey` 添加 `on_delete` 字段（migration.rs）

```rust
AddForeignKey {
    table: String,
    column: String,
    referenced_table: String,
    referenced_column: String,
    on_delete: Option<String>,  // 新增
},
```

**文件**：`crates/core/src/migration.rs` L211-216

### 步骤 4：`fk_reference_for_property` 扩展为 3 元组返回（migration.rs）

将返回类型改为 `(Option<String>, Option<String>, Option<String>)` — (fk_table, fk_col, on_delete_clause)。

```rust
fn fk_reference_for_property(
    meta: &EntityTypeMeta,
    field_name: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    for nav in &meta.navigations {
        if nav.kind != NavigationKind::BelongsTo {
            continue;
        }
        let matches = nav
            .foreign_key_field
            .as_ref()
            .is_some_and(|fk| fk.as_ref() == field_name);
        if matches {
            let on_delete = resolve_fk_on_delete_clause(nav, meta);
            return (
                nav.related_table.as_ref().map(|s| s.to_string()),
                nav.referenced_key_column.as_ref().map(|s| s.to_string()),
                on_delete,
            );
        }
    }
    (None, None, None)
}
```

新增辅助函数 `resolve_fk_on_delete_clause`（紧跟 `fk_reference_for_property` 之后）：

```rust
/// Resolves the `ON DELETE` SQL clause for a FK by checking:
/// 1. The child's BelongsTo navigation `delete_behavior` (if user configured it here)
/// 2. The principal's inverse HasMany navigation via `resolve_delete_behavior`
/// 3. Fallback: nullability of the FK property on the child entity
fn resolve_fk_on_delete_clause(
    belongs_to_nav: &NavigationMeta,
    child_meta: &EntityTypeMeta,
) -> Option<String> {
    use crate::relations::DeleteBehavior;

    // 1. Explicit on the BelongsTo side
    if let Some(b) = belongs_to_nav.delete_behavior {
        return Some(b.to_sql_clause().to_string());
    }

    // 2. Find the principal's HasMany navigation pointing back
    if let Some(meta_fn) = belongs_to_nav.related_entity_meta {
        let principal_meta = meta_fn();
        for principal_nav in &principal_meta.navigations {
            if principal_nav.kind == NavigationKind::HasMany
                && principal_nav.related_type_id == child_meta.type_id
            {
                let behavior = crate::db_context::resolve_delete_behavior(principal_nav);
                return Some(behavior.to_sql_clause().to_string());
            }
        }
    }

    // 3. Fallback: nullability from the FK property's Rust type
    if let Some(fk_prop) = child_meta.properties.iter().find(|p| p.is_foreign_key) {
        let is_nullable = fk_prop.type_name.contains("Option");
        let behavior = if is_nullable {
            DeleteBehavior::Restrict
        } else {
            DeleteBehavior::Cascade
        };
        return Some(behavior.to_sql_clause().to_string());
    }

    None
}
```

**文件**：`crates/core/src/migration.rs` L486-506（替换 `fk_reference_for_property`）+ 新增 `resolve_fk_on_delete_clause`

### 步骤 5：`create_snapshot` 填充 `fk_on_delete`（migration.rs）

L286-303 的 `SnapshotColumn` 构造改为：

```rust
let (fk_table, fk_col, fk_on_delete) =
    fk_reference_for_property(et, p.field_name.as_ref());
SnapshotColumn {
    field_name: p.field_name.to_string(),
    column_name: p.column_name.to_string(),
    type_name: p.type_name.to_string(),
    is_primary_key: p.is_primary_key,
    is_required: p.is_required,
    is_foreign_key: p.is_foreign_key,
    max_length: p.max_length,
    is_auto_increment: p.is_auto_increment,
    is_sequence: p.is_sequence,
    sequence_name: p.sequence_name.as_ref().map(|s| s.to_string()),
    fk_referenced_table: fk_table,
    fk_referenced_column: fk_col,
    has_index: p.has_index,
    is_unique: p.is_unique,
    fk_on_delete,  // 新增
}
```

**文件**：`crates/core/src/migration.rs` L286-303

### 步骤 6：`append_create_table_fks` 传递 `on_delete`（migration.rs）

L436-442 改为：

```rust
if let Some((ref_table, ref_col)) = fk_target(col) {
    changes.push(SchemaChange::AddForeignKey {
        table: table.to_string(),
        column: col.column_name.clone(),
        referenced_table: ref_table,
        referenced_column: ref_col,
        on_delete: col.fk_on_delete.clone(),  // 新增
    });
}
```

**文件**：`crates/core/src/migration.rs` L435-443

### 步骤 7：`diff_foreign_keys` 支持 ON DELETE（migration.rs）

**7a. 扩展 HashMap 元组为 4 元素**（L514-529）：

```rust
let old_fks: HashMap<&str, (&SnapshotColumn, String, String, Option<String>)> = old_et
    .columns
    .iter()
    .filter_map(|c| {
        let (rt, rc) = fk_target(c)?;
        Some((c.column_name.as_str(), (c, rt, rc, c.fk_on_delete.clone())))
    })
    .collect();
let new_fks: HashMap<&str, (&SnapshotColumn, String, String, Option<String>)> = new_et
    .columns
    .iter()
    .filter_map(|c| {
        let (rt, rc) = fk_target(c)?;
        Some((c.column_name.as_str(), (c, rt, rc, c.fk_on_delete.clone())))
    })
    .collect();
```

**7b. 新增 FK 传递 `on_delete`**（L534-542）：

```rust
for col in new_names.difference(&old_names) {
    let (_, rt, rc, od) = &new_fks[col];
    changes.push(SchemaChange::AddForeignKey {
        table: table.to_string(),
        column: (*col).to_string(),
        referenced_table: rt.clone(),
        referenced_column: rc.clone(),
        on_delete: od.clone(),  // 新增
    });
}
```

**7c. 交集比较增加 ON DELETE 检测**（L553-568）：

```rust
for col in old_names.intersection(&new_names) {
    let (_, old_rt, old_rc, old_od) = &old_fks[col];
    let (_, new_rt, new_rc, new_od) = &new_fks[col];
    if old_rt != new_rt || old_rc != new_rc || old_od != new_od {
        changes.push(SchemaChange::DropForeignKey {
            table: table.to_string(),
            column: (*col).to_string(),
            referenced_table: old_rt.clone(),
        });
        changes.push(SchemaChange::AddForeignKey {
            table: table.to_string(),
            column: (*col).to_string(),
            referenced_table: new_rt.clone(),
            referenced_column: new_rc.clone(),
            on_delete: new_od.clone(),  // 新增
        });
    }
}
```

**文件**：`crates/core/src/migration.rs` L508-572

### 步骤 8：`generate_up_sql_inner` AddForeignKey 分支附加 ON DELETE（migration.rs）

L815-829 改为：

```rust
SchemaChange::AddForeignKey {
    table,
    column,
    referenced_table,
    referenced_column,
    on_delete,
} => {
    let fk_name = Self::foreign_key_name(table, column, referenced_table);
    let on_delete_clause = on_delete
        .as_deref()
        .map(|c| format!(" ON DELETE {}", c))
        .unwrap_or_default();
    sql.push_str(&format!(
        "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}){};\n",
        q(table),
        q(&fk_name),
        q(column),
        q(referenced_table),
        q(referenced_column),
        on_delete_clause
    ));
}
```

**文件**：`crates/core/src/migration.rs` L815-829

### 步骤 9：`generate_down_sql` AddForeignKey 分支（migration.rs）

L931-950 的 `AddForeignKey` match arm 已使用 `..` 忽略额外字段，无需修改。验证 `..` 是否存在；若不存在则添加。

**文件**：`crates/core/src/migration.rs` L931-950

### 步骤 10：`snapshot_to_json` 序列化 `fk_on_delete`（migration.rs）

L1516-1541 的 format! 字符串末尾（`"is_unique":{}}}` 之前）追加 `,"fk_on_delete":{}`，并添加参数：

```rust
out.push_str(&format!(
    "        {{\"field_name\":\"{}\",\"column_name\":\"{}\",\"type_name\":\"{}\",\"is_primary_key\":{},\"is_required\":{},\"is_foreign_key\":{},\"max_length\":{},\"is_auto_increment\":{},\"is_sequence\":{},\"sequence_name\":{},\"fk_referenced_table\":{},\"fk_referenced_column\":{},\"has_index\":{},\"is_unique\":{},\"fk_on_delete\":{}}}\n",
    // ... 现有参数 ...
    col.is_unique,
    col.fk_on_delete
        .as_ref()
        .map(|s| format!("\"{}\"", s.replace('"', "\\\"")))
        .unwrap_or_else(|| "null".into())  // 新增
));
```

**文件**：`crates/core/src/migration.rs` L1516-1541

### 步骤 11：`snapshot_from_json` 反序列化 `fk_on_delete`（migration.rs）

L1569-1590 的 `SnapshotColumn` 构造添加：

```rust
fk_on_delete: extract_json_string(col_chunk, "fk_on_delete"),
```

**文件**：`crates/core/src/migration.rs` L1569-1590

### 步骤 12：编写测试

**新建文件**：`crates/core/tests/fk_on_delete_tests.rs`

使用 `#[derive(EntityType)]` 定义测试实体，通过 `T::entity_meta()` 获取元数据，调用 `MigrationEngine::generate` 验证 DDL。

测试用例：

1. **`test_required_fk_generates_cascade`** — `Blog.posts: HasMany<Post>`（无 `#[on_delete]`），`Post.blog_id: i32` → DDL 含 `ON DELETE CASCADE`
2. **`test_optional_fk_generates_restrict`** — `Post.blog_id: Option<i32>`（无 `#[on_delete]`）→ DDL 含 `ON DELETE RESTRICT`
3. **`test_explicit_set_null`** — `#[on_delete(SetNull)]` on HasMany + `Option<i32>` FK → DDL 含 `ON DELETE SET NULL`
4. **`test_explicit_no_action`** — `#[on_delete(NoAction)]` → DDL 含 `ON DELETE NO ACTION`
5. **`test_on_delete_change_triggers_fk_rebuild`** — 旧快照 CASCADE → 新快照 SET NULL → diff 生成 DropForeignKey + AddForeignKey
6. **`test_snapshot_json_roundtrip`** — 序列化含 `fk_on_delete`，反序列化还原

实体定义示例：
```rust
#[derive(EntityType)]
#[table("fk_cascade_blogs")]
struct FkCascadeBlog {
    #[primary_key]
    id: i32,
    #[has_many(field = "blog_id")]
    posts: HasMany<FkCascadePost>,
}

#[derive(EntityType)]
#[table("fk_cascade_posts")]
struct FkCascadePost {
    #[primary_key]
    id: i32,
    #[foreign_key]
    blog_id: i32,
    #[belongs_to(field = "blog_id")]
    blog: BelongsTo<FkCascadeBlog>,
}
```

类似定义 `FkOptionalBlog`/`FkOptionalPost`（`Option<i32>` FK）、`FkSetNullBlog`（`#[on_delete(SetNull)]`）/`FkSetNullPost`、`FkNoActionBlog`（`#[on_delete(NoAction)]`）/`FkNoActionPost`。

### 步骤 13：全量回归验证

```powershell
cargo check --workspace
cargo test -p rust-ef-core
```

确保所有现有测试（含 12 个级联删除测试）+ 新增 6 个 FK ON DELETE 测试全部通过。

## 假设与约束

1. `resolve_delete_behavior` 逻辑与 db_context.rs 运行时级联删除一致 — 复用同一函数确保一致性
2. `related_entity_meta` 函数指针在 BelongsTo 导航上已由宏正确设置 — 已在 entity.rs 宏代码中确认
3. SQLite/MySQL/PostgreSQL 均支持 `ON DELETE {CASCADE|RESTRICT|SET NULL|NO ACTION}` 语法 — SQL 标准
4. 旧版 JSON 快照（无 `fk_on_delete` 字段）通过 `extract_json_string` 返回 `None` — 向后兼容

## 风险与缓解

| 风险 | 缓解 |
|------|------|
| 首次迁移重建所有 FK | 预期行为，与 EFCore 一致；仅影响升级后的首次迁移 |
| `related_entity_meta` 为 None | 步骤 4 的 fallback 逻辑（步骤 3）基于 FK 属性类型名解析 |
| ON DELETE 子句语法差异 | 所有三种方言支持标准 SQL 语法，无需方言特殊处理 |

## 验证检查点

1. 步骤 1-2 后：`cargo check -p rust-ef-core` 通过
2. 步骤 3-11 后：`cargo check -p rust-ef-core` 通过（所有构造点已修复）
3. 步骤 12 后：`cargo test -p rust-ef-core --test fk_on_delete_tests` 6 个测试通过
4. 步骤 13：`cargo test -p rust-ef-core` 全量通过（无回归）
