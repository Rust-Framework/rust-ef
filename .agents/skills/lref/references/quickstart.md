# 第一层：快速入门

> 90% 的开发场景只需掌握本层内容。

## 1.1 实体定义

```rust
use rust_ef::prelude::*;

#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    #[max_length(200)]
    pub title: String,

    pub is_deleted: bool,

    #[foreign_key(Category)]
    pub category_id: i32,

    #[navigation]
    pub category: BelongsTo<Category>,
}
```

**必知属性速查：**

| 属性 | 用途 | 使用频率 |
|------|------|:---:|
| `#[table("name")]` | 数据库表名 | 必须 |
| `#[primary_key]` | 主键 | 必须 |
| `#[auto_increment]` | 自增主键 | 常用 |
| `#[required]` | NOT NULL | 常用 |
| `#[max_length(N)]` | 字符串最大长度 | 常用 |
| `#[foreign_key(Type)]` | 外键引用 | 常用 |
| `#[navigation]` | 导航属性标记 | 常用 |
| `#[index]` / `#[unique]` | 索引 / 唯一索引 | 常用 |
| `#[column("name")]` | 覆盖列名 | 偶尔 |
| `#[not_mapped]` | 排除映射 | 偶尔 |
| `#[context("key")]` | 多数据库隔离 | 偶尔 |
| `#[concurrency_check]` | 乐观并发令牌 | 罕见 |

> 完整实体定义模板：`templates/entity-definition.rs`

## 1.2 查询

**核心原则：分步绑定，不链式；linq! 表达式绑定，不内联。**

```rust
// === 过滤 + 列表 ===
let set = ctx.set::<Blog>();
let expr = linq!(|b: Blog| b.rating > 3);
let blogs = set.filter(expr).to_list().await?;

// === 条件过滤 + 包含导航 + 排序 + 分页 ===
let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 0;
    include b.category;
    order_by b.created_at desc;
).skip(0).take(20).to_list().await?;

// === 单条查询（主键查询）===
let query = ctx.set::<Blog>().query();
let blog = query.find(1).await?;

// === 首条匹配 ===
let set = ctx.set::<Blog>();
let expr = linq!(|b: Blog| b.slug == "hello");
let blog = set.filter(expr).first_or_default().await?;

// === 计数 ===
let count: i64 = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 3; count).await?;
```

**对比：推荐 vs 不推荐**

```rust
// ❌ 不推荐：链式调用，阅读和调试不友好
let blog = ctx.set::<Blog>().query().find(1).await?.unwrap();

// ✅ 推荐：分步绑定，变量可单独检查
let query = ctx.set::<Blog>().query();
let blog = query.find(1).await?;

// ❌ 不推荐：linq! 内联闭包，抽象不直观
let blog = linq!(ctx.set::<Blog>(), |b: Blog| b.slug == "hello")
    .first_or_default().await?;

// ✅ 推荐：表达式绑定，过滤逻辑独立命名
let expr = linq!(|b: Blog| b.slug == "hello");
let set = ctx.set::<Blog>();
let blog = set.filter(expr).first_or_default().await?;
```

## 1.3 增删改

```rust
// === 新增 ===
let now = chrono::Utc::now().timestamp();
let blog = Blog {
    id: 0,  // 自增主键，INSERT 后自动回填
    title: "新博客".into(),
    is_deleted: false,
    category_id: 1,
    category: BelongsTo::default(),
};
ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
// blog.id 已自动填充数据库生成的 ID

// === 更新 ===
let mut blog = {
    let query = ctx.set::<Blog>().query();
    query.find(1).await?.unwrap()
};
blog.title = "新标题".into();
ctx.set::<Blog>().detect_changes();  // 仅标记变更字段
ctx.save_changes().await?;

// === 删除（软删除推荐用全局查询过滤器，见第二层）===
let mut blog = {
    let query = ctx.set::<Blog>().query();
    query.find(1).await?.unwrap()
};
blog.is_deleted = true;
ctx.set::<Blog>().detect_changes();
ctx.save_changes().await?;
```

## 1.4 依赖注入

```rust
use rust_dicore::*;
use rust_ef::di::*;
use rust_ef::db_context::DbContext;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

let mut ctx: DbContext = provider.get_owned();
```

`add_dbcontext` 注册为 **Scoped** 生命周期。推荐使用 `get_owned()` 获取 owned `DbContext`，
直接 `&mut self` 访问 `set::<T>()` / `save_changes()`，无需 `Arc<Mutex>`。
也可使用 `scope.get()` 获取 `Arc<DbContext>`（共享，仅 `&self` 访问）。

> 完整 DI 配置模板：`templates/di-setup.rs`