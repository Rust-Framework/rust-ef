# 自动实体注册与发�?
rust-ef 提供基于 `inventory` 的编译期自动注册机制，对�?EFCore �?`IEntityTypeConfiguration<T>` 配置分离模式�?*v1.1.0 起，`DbContext::from_options()` 自动调用 `discover_entities()`**，开发者只需写好实体和配置，框架自动完成元数据注册——无需任何手动调用�?
## 核心机制

| 组件 | 作用 |
|------|------|
| `#[derive(EntityType)]` | 自动调用 `inventory::submit!` 注册 `EntityRegistration`（含 `meta_fn` 函数指针�?|
| `#[entity(T)]` | 属性宏，应用于 `impl IEntityTypeConfiguration<T>` 块，自动注册 `EntityConfigRegistration` |
| `DbContext::from_options()` | **自动**调用 `discover_entities()`，发现所有注册的实体并应用配�?|
| `DbContext::discover_entities()` | 运行时迭�?`inventory::iter`，填充实体元数据�?Fluent API 配置（幂等，可重复调用） |
| `DbContext::ensure_created()` | 调用 `model_builder.build()` 应用所�?Fluent API 覆盖后建�?|

## 基本用法（v1.1.0 推荐模式�?
定义实体类型时，`#[derive(EntityType)]` 会自动将其注册到全局注册表。`DbContext::from_options()` 自动发现所有注册的实体�?
```rust
use rust_ef::prelude::*;

#[derive(EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub url: String,
}

// from_options() 自动发现 Blog —�?无需手动 discover_entities()
let mut ctx = DbContext::from_options(&options)?;
ctx.ensure_created().await?;  // 元数据已就绪，直接建�?```

无需再为每个实体类型手动调用 `ctx.set::<Blog>()` �?`ctx.discover_entities()`�?
> **�?*：手动调�?`ctx.discover_entities()` 仍然兼容（幂等空操作），�?v1.1.0 起不再需要�?
## 配置分离（IEntityTypeConfiguration�?
将配置逻辑与实体定义分离，对齐 EFCore �?`IEntityTypeConfiguration<T>` 模式�?
```rust
#[derive(Default)]
pub struct BlogConfig;

#[entity(Blog)]
impl IEntityTypeConfiguration<Blog> for BlogConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Blog>) {
        entity.to_table("blogs_v2");
        entity.property_named("url")
            .has_column_name("blog_url")
            .is_required();
        entity.property_named("rating").has_index();

        // 种子数据
        entity.has_data(vec![
            Blog { id: 1, url: "https://example.com".into(), rating: 5 },
        ]);
    }
}
```

`DbContext::from_options()` 自动发现所�?`#[entity(T)]` 配置并应用到 `ModelBuilder`，确�?`ensure_created()` 创建的表结构与配置一致�?
## 多数据库上下文隔离（v1.1.0�?
当应用使用多�?keyed `DbContext` 时，可通过 `#[context("key")]` 属性将实体标记到指定上下文，`#[entity(T, "key")]` 将配置应用到指定上下文：

```rust
// 默认上下文实�?—�?�?#[context] 属性，context_key = None
#[derive(EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key]
    pub id: i32,
    pub url: String,
}

// Keyed 上下文实�?—�?标记�?"logs" 上下�?#[derive(EntityType)]
#[context("logs")]
#[table("log_entries")]
pub struct LogEntry {
    #[primary_key]
    pub id: i32,
    pub message: String,
}

// 默认上下文的配置
#[derive(Default)]
pub struct BlogConfig;

#[entity(Blog)]
impl IEntityTypeConfiguration<Blog> for BlogConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Blog>) {
        entity.to_table("blogs_v2");
    }
}

// Keyed 上下文的配置 —�?第二参数指定 "logs"
#[derive(Default)]
pub struct LogEntryConfig;

#[entity(LogEntry, "logs")]
impl IEntityTypeConfiguration<LogEntry> for LogEntryConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, LogEntry>) {
        entity.property_named("message").has_index();
    }
}
```

注册两个 keyed DbContext�?
```rust
let provider = ServiceCollection::new()
    .add_dbcontext_keyed("primary", |options| {
        options.use_postgres("host=primary/db");
    })
    .add_dbcontext_keyed("logs", |options| {
        options.use_sqlite("logs.db");
    })
    .build()
    .unwrap();

// "primary" 上下文只管理 Blog（context_key = None�?// "logs" 上下文只管理 LogEntry（context_key = Some("logs")�?let primary: Arc<dyn IDbContext> = provider.get_keyed("primary");
let logs: Arc<dyn IDbContext> = provider.get_keyed("logs");
```

### 过滤规则

`discover_entities()` �?`context_key` 过滤�?
| 实体�?`context_key` | DbContext �?`context_key` | 是否注册到该 DbContext |
|----------------------|---------------------------|----------------------|
| `None`（默认） | `None`（默认上下文�?| �?|
| `None`（默认） | `Some("logs")` | �?|
| `Some("logs")` | `None`（默认上下文�?| �?|
| `Some("logs")` | `Some("logs")` | �?|

这确保每�?`DbContext` 只管理属于自己的实体，避免跨数据库污染�?
## 关键约定

1. **属性宏参数是实体类�?*：`#[entity(Blog)]` 指定实体类型 `Blog`，而非配置类型 `BlogConfig`
2. **可选第二参数指定上下文 key**：`#[entity(Blog, "logs")]` 将配置应用到 "logs" 上下�?3. **配置类型必须实现 `Default`**：宏生成�?`apply_fn` 通过 `Default::default()` 实例化配�?4. **闭包不捕获环境变�?*：`apply_fn` 通过函数指针 + `Default::default()` 工作，可隐式转换�?`fn(&mut ModelBuilder)`

## �?`set::<T>()` 的关�?
| 场景 | `from_options()` 自动发现 | `set::<T>()` |
|------|--------------------------|--------------|
| 填充元数�?| �?所�?`#[derive(EntityType)]` 类型 | �?仅指定类�?|
| 应用 Fluent API | �?通过 `#[entity]` | �?通过 `ctx.model().entity::<T>()` |
| 创建 `DbSet<T>` 实例 | �?不创建（用于 CRUD 时仍需 `set`�?| �?创建 |
| 创建 `SetOps` saver | �?不创�?| �?创建 |
| 用于 `ensure_created()` | �?足够 | �?足够 |
| 用于 CRUD 操作 | �?不足（需�?`set` 创建 `DbSet`�?| �?足够 |

**典型用法**�?
```rust
let mut ctx = DbContext::from_options(&options)?;  // 自动发现 + 应用配置
ctx.ensure_created().await?;                        // 建表

// CRUD 操作仍需按需调用 set::<T>()
let blog = Blog { id: 0, url: "...".into(), rating: 1 };
ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
```

## 向后兼容

- `ctx.set::<T>()` 仍然可用，行为幂�?- 手动调用 `ctx.discover_entities()` 仍然兼容（v1.1.0 起为幂等空操作）
- 不调�?`discover_entities()` 时，旧代码行为兼�?- **重要**：v0.5.1 修复�?`ensure_created()` 绕过 Fluent API 配置�?Bug。即使不使用 `discover_entities()`，通过 `ctx.model().entity::<T>().to_table("...")` 配置的覆盖现在会真正生效

## 调试技�?
### 检查注册的实体

```rust
use rust_ef::registration::EntityRegistration;

for reg in inventory::iter::<EntityRegistration> {
    println!("registered: {} ({:?}) context_key={:?}", reg.type_name, reg.type_id, reg.context_key);
}
```

### 检�?DbContext 是否已发现某实体

```rust
let ctx = DbContext::from_options(&options)?;
assert!(ctx.entity_metas_contains::<Blog>());
```

### 检查最终的 EntityTypeMeta

```rust
let ctx = DbContext::from_options(&options)?;
let metas = ctx.model_builder().build();
for meta in &metas {
    println!("{}: table={}", meta.type_name, meta.table_name);
}
```

### 检�?Fluent API 配置是否应用

```rust
let ctx = DbContext::from_options(&options)?;
let metas = ctx.model_builder().build();
let blog_meta = metas.iter()
    .find(|m| m.type_name.contains("Blog"))
    .expect("Blog should be discovered");
assert_eq!(blog_meta.table_name.as_ref(), "blogs_v2");
```

## 工作原理

### 编译�?
1. `#[derive(EntityType)]` �?`quote! {}` 块末尾注入：
   ```rust
   rust_ef::inventory::submit!({
       rust_ef::registration::EntityRegistration {
           type_id: std::any::TypeId::of::<Blog>(),
           type_name: stringify!(Blog),
           meta_fn: <Blog as IEntityType>::entity_meta,
           context_key: None,  // �?Some("logs") if #[context("logs")]
       }
   });
   ```

2. `#[entity(Blog)]` �?`impl` 块后追加�?   ```rust
   rust_ef::inventory::submit!({
       rust_ef::registration::EntityConfigRegistration {
           type_id: TypeId::of::<Blog>(),
           type_name: stringify!(Blog),
           apply_fn: |builder: &mut ModelBuilder| {
               let meta = Blog::entity_meta();
               builder.register_entity_meta(meta);
               let config = BlogConfig::default();
               let mut entity_builder = EntityTypeBuilder::new(builder, TypeId::of::<Blog>());
               BlogConfig::configure(&config, &mut entity_builder);
           },
           context_key: None,  // �?Some("logs") if #[entity(Blog, "logs")]
       }
   });
   ```

3. `inventory` 通过链接器段（linker section）在编译期收集所�?`submit!` 注册�?
### 运行�?
1. `DbContext::from_options()` 自动调用 `discover_entities()`
2. `discover_entities()` �?`context_key` 过滤后，迭代 `EntityConfigRegistration` 应用配置，再迭代 `EntityRegistration` 填充元数�?3. `ctx.ensure_created()` 调用 `model_builder.build()` 应用所�?`EntityConfig` 覆盖
4. `MigrationEngine` 使用应用覆盖后的 metas 创建�?
## 参考链�?
- [inventory crate 文档](https://docs.rs/inventory/latest/inventory/)
- [EFCore IEntityTypeConfiguration&lt;T&gt;](https://learn.microsoft.com/en-us/dotnet/api/microsoft.entityframeworkcore.ientitytypeconfiguration-1)
- [多数据库 Keyed 注册](../10-di-interceptors/keyed-databases.md)
- [常见陷阱与排查第 4 点](../11-best-practices/common-pitfalls.md)
