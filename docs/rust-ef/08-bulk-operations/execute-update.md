# 批量更新 ExecuteUpdate

当需要按条件更新大量记录时，`ExecuteUpdate` 直接在数据库端执行 UPDATE，无需先加载实体到内存。普通场景用 `linq!` 宏的 `set` + `execute_update` 子句表达（`set b.rating, 3` 编译期解析为 `Blog::COLUMN_RATING`）；动态列或 CAS（条件更新）用公开的 `ExecuteUpdateBuilder::set_column`，二者最终生成同一条 UPDATE。

2.8 起，`SET` 的右侧可以是**表达式**：另一列（`col = other_col`）或列算术（`col = col + ?`），用于原子增量/减量和 CAS 版本号自增，不需要「先读后写」。

## 基本用法

```rust
// linq! 多子句形式：过滤 + set + execute_update
let affected = linq!(ctx.set::<Blog>(), |b: Blog| b.rating < 3;
    set b.rating, 3;
    execute_update
).await?;

println!("Updated {} blogs", affected);
```

## 多列更新

```rust
let affected = linq!(ctx.set::<Blog>(), |b: Blog| b.url.contains("old-domain");
    set b.url, "https://new-domain.com";
    set b.rating, 5;
    execute_update
).await?;
```

## 生成的 SQL

```sql
UPDATE blogs SET rating = ? WHERE rating < ?
```

多个 `set` 子句生成 `SET col1 = ?, col2 = ?`。

## 表达式更新：列 = 列 ± 表达式（2.8+）

`set` 的右侧识别三种形态：

```rust
// 1) 列自增/自减：views = views + 10、stock = stock - 3
let affected = linq!(ctx.set::<Blog>(), |b: Blog| b.blog_id == id;
    set b.views, b.views + 10;
    execute_update
).await?;

// 2) 列 = 另一列：total = subtotal
let affected = linq!(ctx.set::<Order>(), |o: Order| o.id == id;
    set o.total, o.subtotal;
    execute_update
).await?;

// 3) 列 = 列 ± 另一列：stock = stock + reserved
let affected = linq!(ctx.set::<Product>(), |p: Product| p.id == id;
    set p.stock, p.stock + p.reserved;
    execute_update
).await?;

// 4) 普通值保持原行为：SET col = ?
let affected = linq!(ctx.set::<Blog>(), |b: Blog| b.rating < 3;
    set b.rating, 3;
    execute_update
).await?;
```

生成的 SQL：

```sql
-- 形态 1
UPDATE blogs SET views = views + ? WHERE blog_id = ?
-- 形态 2
UPDATE orders SET total = subtotal WHERE id = ?
-- 形态 3
UPDATE products SET stock = stock + reserved WHERE id = ?
```

- 右侧首操作数必须是字段（`b.col`），它决定被引用的列；`+`/`-` 的另一侧可以是值（绑定参数）或另一个字段；
- 值始终作为绑定参数占位（不会内联到 SQL），用户输入不会造成注入；
- 参数顺序固定：**SET 表达式参数（按赋值顺序）在前，WHERE 参数在后**，与 SQL 文本顺序一致；
- 嵌套减法会自动加括号（`stock = stock - (reserved + ?)`），保证求值顺序正确。

## 动态列 / CAS：`set_column`（公开 API）

当列需要运行时决定，或需要"校验 + 更新"的单语句 CAS（compare-and-swap / 乐观锁）时，用
`ExecuteUpdateBuilder::set_column(column, value)`。参数是实体 `COLUMN_*` 常量（`&'static str`）
与可转成 `DbValue` 的值；`execute()` 返回受影响行数，`0` 表示 WHERE 守卫已不匹配：

```rust
let affected = ctx
    .set::<Order>()
    .query()
    .filter(linq!(|o: Order| o.id == order_id && o.version == expected_version))
    .execute_update()
    .set_column(Order::COLUMN_STATUS, "paid")
    .set_column(Order::COLUMN_VERSION, expected_version + 1)
    .execute()
    .await?;

if affected == 0 {
    // 输掉竞争：重新读取后重试或返回冲突
}
```

- `set_column` 自 2.3.0 起是公开、受 semver 覆盖的入口（用于动态列/CAS；普通写路径仍推荐 `linq! set`）。
- 2.8.0 已删除 2.3 的历史别名 `set_column_internal`；统一使用 `set_column` / `set_column_expr`。

## CAS + 原子自增：`increment` / `decrement`

版本号自增不需要「读 version 再写 version + 1」：`increment(column, by)` 生成
`SET version = version + ?`，与 WHERE 守卫在同一条语句里执行；守卫失配时 0 行受影响，
版本号也不会被多 bump 一次：

```rust
use rust_ef::query::ColumnExpr;

let affected = ctx
    .set::<Order>()
    .query()
    .filter(linq!(|o: Order| o.id == order_id && o.version == expected_version))
    .execute_update()
    .increment(Order::COLUMN_VERSION, 1) // SET version = version + ?
    .execute()
    .await?;

match affected {
    1 => {} // 抢到：version 已原子 +1
    0 => {} // 守卫失配：被其他写者抢先，重新读取/返回冲突
    n => return Err(format!("unexpected affected rows: {n}").into()),
}
```

- `increment(column, by)` / `decrement(column, by)`：`SET col = col + ?` / `col - ?`，`by` 绑定参数；
- `set_column_expr(column, expr)`：通用入口，`expr` 由 `ColumnExpr` 构造：

```rust
// stock = stock + reserved
let affected = ctx
    .set::<Product>()
    .query()
    .filter(linq!(|p: Product| p.id == product_id))
    .execute_update()
    .set_column_expr(
        Product::COLUMN_STOCK,
        ColumnExpr::column(Product::COLUMN_STOCK)
            .add(ColumnExpr::column(Product::COLUMN_RESERVED)),
    )
    .execute()
    .await?;
```

生成的 SQL 与参数顺序（`columns = ?` 表示按 SET 赋值顺序、再 WHERE）：

```sql
UPDATE products SET stock = stock + ? WHERE (id = ?) AND (version = ?)
```

`ColumnExpr` 的构造器：`column(列)`、`value(值)`、`.add(expr)`、`.sub(expr)`；`DbValue` 可
`From` 转换为 `ColumnExpr`（等价于 `value`）。

## 设计要点

| 实践 | 说明 |
|------|------|
| 优先用 `execute_update` 做批量修改 | 比先 `to_list()` 再逐条 `update()` + `save_changes()` 高效得多 |
| 注意不触发拦截器 | `execute_update` 绕过 ChangeTracker，拦截器不会执行 |
| `set` 子句的列字段用闭包访问 | `set b.rating, 3` 编译期解析为 `Blog::COLUMN_RATING`，避免拼写错误 |
| 增量/减量用表达式更新 | `increment` / `set col, col + ?` 是单语句原子操作，避免读到旧值再写回（丢失更新） |
| CAS 用 `WHERE version = ?` + 表达式自增 | 守卫与自增同一条语句；`affected == 0` 即竞争失败，无需再查库确认 |
| 参数顺序固定 | SET 表达式参数按赋值顺序在前，WHERE 参数在后；`to_sql()` 与 `params()` 顺序一致 |
| `linq!` 的表达式 SET 只支持「列在左」 | `set b.col, b.col + 1` / `b.col - x` / `b.col, other.col`；需要 `值 - 列` 或更复杂表达式时用运行时 `ColumnExpr::add/sub` 显式构造 |

上一节：[SaveChanges 与事务边界](../07-change-tracking/save-changes.md) ｜ 下一节：[批量删除 ExecuteDelete](execute-delete.md)
