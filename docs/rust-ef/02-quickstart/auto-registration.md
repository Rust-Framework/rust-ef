# 自动实体注册

rust-ef v0.5.1 引入了基于 `inventory` 的编译期自动注册机制，对齐 EFCore 的 `IEntityTypeConfiguration<T>` 配置分离模式，同时修复了 `ensure_created()` 绕过 Fluent API 配置的历史 Bug。

## 核心机制

| 组件 | 作用 |
|------|------|
| `#[derive(EntityType)]` | 自动调用 `inventory::submit!` 注册 `EntityRegistration`（含 `meta_fn` 函数指针） |
| `#[entity(T)]` | 属性宏，应用于 `impl IEntityTypeConfiguration<T>` 块，自动注册 `EntityConfigRegistration` |
| `DbContext::discover_entities()` | 运行时迭代 `inventory::iter`，将注册表内容填充到 STORE A 与 STORE B |
| `DbContext::ensure_created()` | 调用 `model_builder.build()` 应用所有 Fluent API 覆盖 |

## 基本用法

定义实体类型时，`#[derive(EntityType)]` 会自动将其注册到全局注册表：

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

let mut ctx = DbContext::from_options(&options)?;
ctx.discover_entities()?;       // 自动发现 Blog
ctx.ensure_created().await?;
```

> **注**：`DbContext::from_options()` 现已自动调用 `discover_entities()`，上例中的 `ctx.discover_entities()?;` 可省略（手动调用仍兼容，且为幂等空操作）。

无需再为每个实体类型手动调用 `ctx.set::<Blog>()`。

## 配置分离（IEntityTypeConfiguration）

将配置逻辑与实体定义分离，对齐 EFCore 的 `IEntityTypeConfiguration<T>` 模式：

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

调用 `ctx.discover_entities()` 时，所有 `#[entity(T)]` 配置会自动应用到 `ModelBuilder`，确保 `ensure_created()` 创建的表结构与配置一致。

## 关键约定

1. **属性宏参数是实体类型**：`#[entity(Blog)]` 指定实体类型 `Blog`，而非配置类型 `BlogConfig`
2. **配置类型必须实现 `Default`**：宏生成的 `apply_fn` 通过 `Default::default()` 实例化配置
3. **闭包不捕获环境变量**：`apply_fn` 通过函数指针 + `Default::default()` 工作，可隐式转换为 `fn(&mut ModelBuilder)`

## 与 `set::<T>()` 的关系

| 场景 | `discover_entities()` | `set::<T>()` |
|------|------------------------|--------------|
| 填充元数据 | ✅ 所有 `#[derive(EntityType)]` 类型 | ✅ 仅指定类型 |
| 应用 Fluent API | ✅ 通过 `#[entity]` | ✅ 通过 `ctx.model().entity::<T>()` |
| 创建 `DbSet<T>` 实例 | ❌ 不创建（用于 CRUD 时仍需 `set`） | ✅ 创建 |
| 创建 `SetOps` saver | ❌ 不创建 | ✅ 创建 |
| 用于 `ensure_created()` | ✅ 足够 | ✅ 足够 |
| 用于 CRUD 操作 | ❌ 不足（需要 `set` 创建 `DbSet`） | ✅ 足够 |

**典型用法**：

```rust
let mut ctx = DbContext::from_options(&options)?;
ctx.discover_entities()?;       // 注册元数据
ctx.ensure_created().await?;    // 建表（应用所有配置）

// CRUD 操作仍需按需调用 set::<T>()
let blog = Blog { id: 0, url: "...".into(), rating: 1 };
ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
```

## 向后兼容

- `ctx.set::<T>()` 仍然可用，行为幂等
- 不调用 `discover_entities()` 时，旧代码行为兼容
- **重要**：v0.5.1 修复了 `ensure_created()` 绕过 Fluent API 配置的 Bug。即使不使用 `discover_entities()`，通过 `ctx.model().entity::<T>().to_table("...")` 配置的覆盖现在会真正生效

## 调试技巧

### 检查注册的实体

```rust
use rust_ef::registration::EntityRegistration;

for reg in inventory::iter::<EntityRegistration> {
    println!("registered: {} ({:?})", reg.type_name, reg.type_id);
}
```

### 检查最终的 EntityTypeMeta

```rust
ctx.discover_entities()?;
let metas = ctx.model().build();
for meta in &metas {
    println!("{}: table={}", meta.type_name, meta.table_name);
}
```

### 检查 Fluent API 配置是否应用

```rust
ctx.discover_entities()?;
let metas = ctx.model().build();
let blog_meta = metas.iter()
    .find(|m| m.type_name.contains("Blog"))
    .expect("Blog should be discovered");
assert_eq!(blog_meta.table_name.as_ref(), "blogs_v2");
```

## 工作原理

### 编译期

1. `#[derive(EntityType)]` 在 `quote! {}` 块末尾注入：
   ```rust
   rust_ef::inventory::submit!({
       rust_ef::registration::EntityRegistration {
           type_id: std::any::TypeId::of::<Blog>(),
           type_name: stringify!(Blog),
           meta_fn: <Blog as IEntityType>::entity_meta,
           context_key: None,
       }
   });
   ```

2. `#[entity(Blog)]` 在 `impl` 块后追加：
   ```rust
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
           context_key: None,
       }
   });
   ```

3. `inventory` 通过链接器段（linker section）在编译期收集所有 `submit!` 注册项

### 运行时

1. `ctx.discover_entities()` 迭代 `inventory::iter::<EntityConfigRegistration>` 应用配置
2. 迭代 `inventory::iter::<EntityRegistration>` 填充 STORE A 与 STORE B
3. `ctx.ensure_created()` 调用 `model_builder.build()` 应用所有 `EntityConfig` 覆盖
4. `MigrationEngine` 使用应用覆盖后的 metas 创建表

## 参考链接

- [inventory crate 文档](https://docs.rs/inventory/latest/inventory/)
- [EFCore IEntityTypeConfiguration&lt;T&gt;](https://learn.microsoft.com/en-us/dotnet/api/microsoft.entityframeworkcore.ientitytypeconfiguration-1)
- [常见陷阱与排查第 4 点](../11-best-practices/common-pitfalls.md)
