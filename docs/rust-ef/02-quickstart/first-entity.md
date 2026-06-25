# 定义第一个实体

实体是 ORM 的核心。`#[derive(EntityType)]` 自动实现 `IEntityType`、`IFromRow`、`IGetKeyValues` 等必要 trait。

## 最小实体

```rust
use rust_ef::prelude::*;

#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key]
    #[auto_increment]
    pub blog_id: i32,

    #[required]
    #[max_length(200)]
    pub url: String,

    pub rating: i32,
}
```

## 属性速查表

| 属性 | 说明 | EF Core 对应 |
|------|------|-------------|
| `#[table("name")]` | 映射表名 | `[Table]` |
| `#[primary_key]` | 主键 | `[Key]` |
| `#[auto_increment]` | 自增 | convention |
| `#[required]` | 非空 | `[Required]` |
| `#[max_length(n)]` | 最大长度 | `[MaxLength]` |
| `#[column("name")]` | 指定列名 | `[Column]` |
| `#[foreign_key(Entity)]` | 外键 | `[ForeignKey]` |
| `#[navigation]` | 导航属性 | implicit |
| `#[not_mapped]` | 不映射 | `[NotMapped]` |
| `#[index]` | 普通索引 | `[Index]` |
| `#[unique]` | 唯一索引 | `[Index(IsUnique = true)]` |
| `#[concurrency_check]` | 并发标记 | `[ConcurrencyCheck]` |

## 带关系的实体

```rust
#[derive(Debug, Clone, EntityType)]
#[table("posts")]
pub struct Post {
    #[primary_key]
    #[auto_increment]
    pub post_id: i32,

    #[required]
    #[max_length(200)]
    pub title: String,

    pub content: Option<String>,

    #[foreign_key(Blog)]
    pub blog_id: i32,

    #[navigation]
    pub blog: BelongsTo<Blog>,
}
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 实体必须 `Clone` | `DbSet` 内部存储需要 |
| 主键类型通常为 `i32` | 自增回填兼容性好 |
| `Option<T>` 自动可空 | 无需额外标注 |
| 导航属性需 `#[navigation]` | 关系元数据由此生成 |

下一节：[DbContext 与 DI 注册](dbcontext-and-di.md)
