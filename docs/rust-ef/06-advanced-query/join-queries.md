# JOIN 查询

当 `include` 不足以表达查询需求时，用 `linq!` 宏的 `inner_join` / `left_join` / `right_join` / `full_join` 子句手动 JOIN；动态列名的场景用运行时 `join` / `join_from`。不要拼接 SQL 字符串。

`linq!` 的 JOIN 子句在编译期把闭包里的字段解析为实体 `COLUMN_*` 常量、把实体类型解析为 `TABLE` 常量，因此表名/列名拼错会直接编译失败。

## 单列 INNER JOIN

```rust
// 多参数闭包：|a: T1, b: T2| a.col == b.col，参数必须显式标注类型
let results = linq!(ctx.set::<Blog>();
    inner_join |a: Blog, b: Post| a.blog_id == b.blog_id
).to_list().await?;
```

生成的 SQL（左表固定为闭包第一个参数的表）：

```sql
SELECT * FROM blogs INNER JOIN posts ON blogs.blog_id = posts.blog_id
```

## LEFT / RIGHT / FULL / CROSS JOIN

```rust
let blogs = linq!(ctx.set::<Blog>();
    left_join |a: Blog, b: Post| a.blog_id == b.blog_id
).to_list().await?;

let rows = linq!(ctx.set::<Blog>();
    cross_join b: Post            // CROSS JOIN 没有 ON 条件
).to_list().await?;
```

`right_join` / `full_join` 的 SQL 形状与 `left_join` 对称。MySQL 不支持 `FULL JOIN`（执行时由数据库报错）；SQLite 仅在较新版本支持 RIGHT/FULL JOIN。

## 复合键（多列 ON）

条件用 `&&` 连接多个 `a.col == b.col` 等值对，每对生成一个 `AND`；括号可省略，也可用于分组：

```rust
// ON shipments.order_id = ... AND shipments.region_id = ...
let rows = linq!(ctx.set::<Order>();
    inner_join |o: Order, s: Shipment| o.id == s.order_id && o.region_id == s.region_id
).to_list().await?;
```

```sql
SELECT * FROM orders
INNER JOIN shipments
  ON orders.id = shipments.order_id
 AND orders.region_id = shipments.region_id
```

规则：

- 每个等值对必须是「闭包参数.字段 == 闭包参数.字段」：`a.x == b.x` 或 `b.x == a.x`（反向写法会归一化，ON 始终是 `左表.列 = 被 JOIN 表.列`）；
- 等值对不能引用第三个实体，也不能是 `a.x > b.x` 之类的比较——JOIN 只支持等值键；
- 只有两列都匹配才算命中。上例中 `region_id` 不匹配的行不会出现在 INNER JOIN 结果里（这是复合键与单列键的语义差异）。

## 链式 JOIN（3 张及以上表）

多个 JOIN 子句按书写顺序串联，每个子句的**第一个参数**就是这次 JOIN 的左侧表：它可以是根实体，也可以是上一条 JOIN 引入的实体：

```rust
let rows = linq!(ctx.set::<Order>();
    // 第 1 跳：orders -> shipments（复合键）
    inner_join |o: Order, s: Shipment| o.id == s.order_id && o.region_id == s.region_id;
    // 第 2 跳：shipments -> carrier_notes（左表是上一条 JOIN 的 shipments）
    inner_join |s: Shipment, n: CarrierNote| s.carrier == n.carrier
).to_list().await?;
```

```sql
SELECT * FROM orders
INNER JOIN shipments
  ON orders.id = shipments.order_id
 AND orders.region_id = shipments.region_id
INNER JOIN carrier_notes
  ON shipments.carrier = carrier_notes.carrier
```

## 运行时 `join` / `join_from`

列名需要运行时决定（列名来自配置/白名单）时用公开方法；表名始终来自实体元数据，不需要手写表名：

```rust
use rust_ef::query::JoinKind;

// ON orders.id = shipments.order_id AND orders.region_id = shipments.region_id
let rows = ctx
    .set::<Order>()
    .query()
    .join::<Shipment>(
        JoinKind::Inner,
        &[("id", "order_id"), ("region_id", "region_id")],
    )
    .to_list()
    .await?;

// 链式：左表显式指定为 Shipment（L 泛型参数）
let rows = ctx
    .set::<Order>()
    .query()
    .join::<Shipment>(JoinKind::Inner, &[("id", "order_id")])
    .join_from::<CarrierNote, Shipment>(JoinKind::Inner, &[("carrier", "carrier")])
    .to_list()
    .await?;
```

- `join::<J>(kind, on)`：新表是 `J`，左表是查询当前的 FROM 表（根表）；
- `join_from::<J, L>(kind, on)`：新表是 `J`，左表是 `L`（链式 JOIN 用）；
- `on` 的元素是 `(左表列, 被 JOIN 表列)`，多个元素生成 `AND`；
- `JoinKind` 取 `Inner` / `Left` / `Right` / `Full` / `Cross`（`Cross` 忽略 `on`）；
- 列名不匹配实体列是运行期数据库报错，编译期无法校验——能静态写出的列请优先用 `linq!`。

## 过滤被 JOIN 的表：`filter_joined`

普通 `filter` 里的 `linq!(|p: Post| ...)` 生成的是**无限定**列名（`posts` 是 JOIN 进来的表，直接写列名会歧义或过滤到根表）。`filter_joined` 会把谓词里的普通过滤列限定为 `<被 JOIN 表>.<列>`：

```rust
use rust_ef::query::JoinKind;

// ... JOIN posts ON blogs.id = posts.blog_id WHERE posts.published = ?
let blogs = ctx
    .set::<Blog>()
    .query()
    .join::<Post>(JoinKind::Inner, &[("id", "blog_id")])
    .filter_joined(linq!(|p: Post| p.published))
    .to_list()
    .await?;
```

```rust
// 与 linq! JOIN 子句组合
let blogs = linq!(ctx.set::<Blog>();
    inner_join |a: Blog, b: Post| a.id == b.blog_id
)
.filter_joined(linq!(|p: Post| p.title.contains("rust")))
.to_list()
.await?;
```

边界：

- 只限定普通过滤列。`EXISTS` / `IN (SELECT ...)` 子查询节点和 `BoolExpr::Raw` 片段保持原样（它们有自己的表作用域）；
- 已经带 `.` 限定的列不会重复加前缀，因此重复调用/复用谓词是幂等的；
- 谓词的类型必须是被 JOIN 的实体（`Predicate<Post>`），实体表名取自元数据。

## JOIN 后的实体物化

手动 JOIN 时 `to_list()` 仍按**根实体**物化（`SELECT *` 的前 N 列是根表列），关联数据不会自动填充到导航属性：

```rust
// 推荐：需要关联数据完整实体时用 include
let blogs = linq!(ctx.set::<Blog>(); include b.posts).to_list().await?;
```

需要在 JOIN 结果里读取**其他表的列**时，用 `select` 投影 + `IRow` 接收，见
[类型化投影](typed-projections.md)（投影列按 SELECT 顺序位置映射）。

## 设计与边界

| 实践 | 说明 |
|------|------|
| 优先用 `include` | 关联数据物化自动化；JOIN 主要用于「按关联表字段筛选根表」 |
| JOIN 用实体类型表达 | `linq!` 编译期校验表/列；运行时 API 只用于动态列名 |
| 多参数闭包须标注类型 | `|a: Blog, b: Post|` 不能省略类型 |
| JOIN 结果仍按根实体物化 | 跨表列用 `select` + `IRow`（见[类型化投影](typed-projections.md)） |
| 不支持同表自连接 | 框架不生成表别名；同一张表自连接请用原生 SQL + `IRow`，见[原始 SQL 与已知限制](raw-sql-limitations.md) |
| `from_cte` + JOIN | `linq!` 的首个 JOIN 沿用旧语义：左表是 CTE 名（当前 FROM）；运行时 `join` 同样使用当前 FROM，链式续连请用 `join_from` 显式指定 `L` 的表名 |
| `filter_joined` 的谓词边界 | 只限定普通列条件（`FilterCondition`）；被 JOIN 实体的导航子查询（EXISTS/IN）仍按根实体解析，复杂跨表条件用原生 SQL + `IRow` |

上一节：[类型化投影与报表行](typed-projections.md) ｜ 下一节：[CTE 与 Window 函数](cte-window-functions.md)
