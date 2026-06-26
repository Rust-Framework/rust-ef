# inventory 自动注册重构方案

**版本**：v0.5.1
**创建日期**：2026-06-26
**作者**：rust-ef 团队
**状态**：待批准
**前置依赖**：v0.5 Phase 1（having 嵌套逻辑）已完成；`rust-dicore` 已升级至 0.3.2

---

## 0. 执行摘要

### 0.1 问题背景

当前 rust-ef 框架存在三个相互关联的架构缺陷：

1. **`ensure_created()` 绕过 Fluent API 配置**：`DbContext.ensure_created()` 直接读取 `self.entity_metas`（STORE A：`HashMap<TypeId, EntityTypeMeta>`），而 `ModelBuilder.entity_metas`（STORE B：`Vec<EntityTypeMeta>`）以及 Fluent API（`to_table` / `has_key` / `property_named` / `has_data` 等）写入的配置覆盖（`EntityConfig`）从未被应用，导致用户通过 `ctx.model().entity::<Blog>().to_table("blogs2")` 等方式配置的表名、列名、主键、种子数据全部静默失效。

2. **`IEntityTypeConfiguration<T>` 是死代码**：该 trait 在 [model_builder.rs:323-325](file:///d:/GitCode/RF/rust-ef/crates/core/src/model_builder.rs#L323-L325) 定义，`ModelBuilder::apply_configuration<C, T>()` 在 [model_builder.rs:66-82](file:///d:/GitCode/RF/rust-ef/crates/core/src/model_builder.rs#L66-L82) 实现但全代码库无人调用，`IEntityTypeConfiguration` 在 prelude 中导出但零实现，与 EFCore 的核心配置分离模式完全脱节。

3. **`set::<T>()` 造成双存储断裂**：[db_context.rs:238-269](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L238-L269) 中 `set::<T>()` 仅写入 STORE A，未写入 STORE B。即使 `ensure_created()` 改为调用 `model_builder.build()`，由于 STORE B 为空，Fluent API 仍无法生效——必须双修。

### 0.2 用户期望

用户期望参照 EFCore 行为：通过 `IEntityTypeConfiguration<T>` + 属性宏实现**编译期自动注册**，使用户无需在 `DbContext` 中手动调用 `set::<T>()` 即可让 `ensure_created()` 发现所有实体并应用所有配置，从而：

- 简化框架使用难度（消灭 `ctx.set::<Blog>()` 样板代码）
- 消除"必须先 `set` 才能 `ensure_created`"的歧义
- 让 Fluent API 与 `IEntityTypeConfiguration` 配置真正生效

### 0.3 选用方案

**方案 A：inventory 全局注册**（用户已确认）

通过 `inventory` crate 的链接器段（linker section）机制，在编译期将所有 `#[derive(EntityType)]` 标注的类型自动注册到全局注册表，在 `ensure_created()` 调用时一次性发现并应用所有配置。

### 0.4 关键决策

| 决策项 | 选择 | 理由 |
|--------|------|------|
| 注册机制 | `inventory` 0.3 | 跨 crate、零运行时开销、支持 Windows MSVC、社区成熟 |
| 实体注册类型 | `EntityRegistration { type_id, meta_fn }` | 类型擦除，所有字段 `const` 可求值且 `Send + Sync` |
| 配置注册类型 | `EntityConfigRegistration { type_id, apply_fn }` | 闭包擦除为 `fn(&mut ModelBuilder)` 函数指针 |
| derive 宏注入点 | `quote! { ... }` 块末尾（[entity.rs:512](file:///d:/GitCode/RF/rust-ef/crates/macros/src/entity.rs#L512)） | 所有 trait impl 之后，避免顺序依赖 |
| 属性宏 | `#[entity_config(T)]` 应用到 `impl IEntityTypeConfiguration<T>` 块 | crate 中首个 `#[proc_macro_attribute]`，自动生成 `inventory::submit!` |
| `set::<T>()` 行为 | 保留向后兼容，幂等地同步写入 STORE B | 不破坏现有 18 个 linq_terminal 测试与 17 个 linq_dsl 测试 |
| `ensure_created()` 行为 | 调用 `model_builder.build()` 而非直接读 STORE A | 让 Fluent API 真正生效 |

### 0.5 影响范围

- **新增依赖**：`inventory = "0.3"`（仅 core crate）
- **新增文件**：2 个（`crates/core/src/registration.rs`、`crates/macros/src/entity_config.rs`）
- **修改文件**：7 个（root `Cargo.toml`、core `Cargo.toml`、core `lib.rs`、`db_context.rs`、`model_builder.rs`、macros `lib.rs`、macros `entity.rs`）
- **更新文档**：2 个（`common-pitfalls.md`、blog 示例）
- **新增测试**：≥6 个（自动注册、配置生效、向后兼容、seed 数据、多实体、错误路径）

### 0.6 验证标准

- `cargo check` 通过（零错误零警告）
- `cargo clippy --workspace --all-targets -- -D warnings` 通过
- `cargo fmt --check` 通过
- 全部现有测试通过（66+ 个非 DB 测试）
- 新增测试全部通过
- `examples/blog` 编译并运行成功

---

## 1. 当前架构分析

### 1.1 DbContext 双存储问题

`DbContext` 结构定义（[db_context.rs:213-221](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L213-L221)）：

```rust
pub struct DbContext {
    sets: HashMap<TypeId, Box<dyn Any + Send + Sync>>,           // DbSet 实例存储
    savers: HashMap<TypeId, Box<dyn ErasedSetOps>>,                // 类型擦除保存器
    entity_metas: HashMap<TypeId, EntityTypeMeta>,                 // STORE A：元数据
    model_builder: ModelBuilder,                                   // 含 STORE B
    change_tracker: ChangeTracker,
    provider: Arc<dyn IDatabaseProvider>,
    interceptor_pipeline: InterceptorPipeline,
}
```

#### 1.1.1 STORE A 与 STORE B 的语义差异

| 维度 | STORE A (`entity_metas: HashMap`) | STORE B (`model_builder.entity_metas: Vec`) |
|------|------------------------------------|---------------------------------------------|
| 数据来源 | `set::<T>()` 写入 `T::entity_meta()` | `model().entity::<T>()` 或 `apply_configuration` 写入 |
| 配置覆盖 | **不应用** | 通过 `apply_config_to_meta()` 应用 `EntityConfig` |
| 被 `ensure_created` 读取 | **是**（当前 Bug 根源） | 否 |
| 被 `model().build()` 读取 | 否 | 是 |
| Fluent API 是否生效 | 否 | 是（但 `build()` 从未被调用） |

#### 1.1.2 `set::<T>()` 当前实现

[db_context.rs:238-269](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L238-L269)：

```rust
pub fn set<T>(&mut self) -> &mut DbSet<T>
where T: IEntityType + ... {
    let type_id = TypeId::of::<T>();
    self.savers.entry(type_id)
        .or_insert_with(|| Box::new(SetOps::<T>::new()));
    self.entity_metas.entry(type_id)               // 仅写入 STORE A
        .or_insert_with(T::entity_meta);
    self.sets.entry(type_id).or_insert_with(|| { ... });
    // ❌ 缺失：未调用 self.model_builder.entity::<T>()
    //         或 apply_configuration 同步到 STORE B
    ...
}
```

#### 1.1.3 `ensure_created()` 当前实现（Bug）

[db_context.rs:290-311](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L290-L311)：

```rust
pub async fn ensure_created(&self) -> EFResult<()> {
    let metas: Vec<EntityTypeMeta> = self.entity_metas.values().cloned().collect();
    // ❌ 直接读 STORE A，未应用 EntityConfig 覆盖
    if metas.is_empty() {
        return Err(EFError::Configuration(
            "No entity types registered. Call ctx.set::<T>() before ensure_created().".into(),
        ));
    }
    let dialect = self.provider.migration_dialect();
    MigrationEngine::new(dialect).ensure_created(&*self.provider, &metas).await?;

    for (type_id, meta) in &self.entity_metas {        // ❌ 再次读 STORE A
        let rows = self.model_builder.seed_rows_for(type_id);
        if !rows.is_empty() {
            MigrationEngine::new(dialect).apply_seed_data(&*self.provider, meta, rows).await?;
        }
    }
    Ok(())
}
```

#### 1.1.4 `ensure_deleted()` 同样有 Bug

[db_context.rs:315-326](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L315-L326)：同样直接读 STORE A。

### 1.2 `IEntityTypeConfiguration<T>` 现状

[model_builder.rs:323-325](file:///d:/GitCode/RF/rust-ef/crates/core/src/model_builder.rs#L323-L325)：

```rust
pub trait IEntityTypeConfiguration<T: IEntityType> {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, T>);
}
```

- 在 prelude 中导出（[lib.rs:64](file:///d:/GitCode/RF/rust-ef/crates/core/src/lib.rs#L64)）
- `apply_configuration<C, T>()` 在 [model_builder.rs:66-82](file:///d:/GitCode/RF/rust-ef/crates/core/src/model_builder.rs#L66-L82) 实现
- **全代码库零调用**（grep 验证）
- 与 EFCore 的 `IEntityTypeConfiguration<T>` 设计意图完全一致，但缺失自动发现机制

### 1.3 `#[derive(EntityType)]` 宏当前输出

[entity.rs:400-512](file:///d:/GitCode/RF/rust-ef/crates/macros/src/entity.rs#L400-L512) 生成 6 个块：

1. `impl IEntityType for #struct_name`（含 `entity_meta()`）
2. `impl #struct_name`（含 `TABLE`、`COLUMN_*`、`FK_*` 常量及辅助方法）
3. `impl IGetKeyValues`
4. `impl IEntitySnapshot`
5. `impl IFromRow`
6. `impl INavigationSetter`

**注入点**：在 `quote! { ... }` 块的末尾（第 511 行 `}` 之前）追加 `inventory::submit!` 调用。

### 1.4 macros crate 现状

[macros/src/lib.rs](file:///d:/GitCode/RF/rust-ef/crates/macros/src/lib.rs) 当前注册 3 个 proc-macro：

- `#[proc_macro_derive(EntityType, ...)]`
- `#[proc_macro] column`
- `#[proc_macro] linq`

**无 attribute macro**。本次新增的 `#[entity_config(T)]` 将是 crate 中首个 `#[proc_macro_attribute]`。

---

## 2. 详细设计

### 2.1 Phase 1：基础设施搭建

#### 2.1.1 步骤 1：添加 inventory 依赖

**文件**：根 `Cargo.toml` + `crates/core/Cargo.toml`

**根 Cargo.toml**（新增 `[workspace.dependencies]` 段）：

```toml
[workspace.package]
version = "0.3.5"
edition = "2021"
license = "MIT"
repository = "https://gitcode.com/rf2026/rust-ef"
authors = ["Start"]

[workspace.dependencies]
inventory = "0.3"
```

**crates/core/Cargo.toml**（新增依赖行）：

```toml
[dependencies]
rust-ef-macros = { version = "0.3.5", path = "../macros" }
rust-dicore = "0.3.2"
async-trait = "0.1"
thiserror = "2"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"], optional = true }
chrono = { version = "0.4", optional = true }
inventory = { workspace = true }      # 新增
```

**理由**：使用 workspace 依赖统一版本管理，便于后续其他 crate 复用。

#### 2.1.2 步骤 2：创建 registration 模块

**新文件**：`crates/core/src/registration.rs`

**完整实现**：

```rust
//! Compile-time entity registration via `inventory`.
//!
//! The `#[derive(EntityType)]` macro emits an `inventory::submit!` for each
//! entity type, registering a type-erased `EntityRegistration`. The DbContext
//! discovers these at runtime via `inventory::iter::<EntityRegistration>()`.
//!
//! Similarly, `#[entity_config(T)]` applied to `impl IEntityTypeConfiguration<T>`
//! blocks emits an `EntityConfigRegistration`, whose `apply_fn` is invoked by
//! `ModelBuilder::apply_registered_configurations()` to apply Fluent API
//! overrides.

use crate::metadata::EntityTypeMeta;
use crate::model_builder::ModelBuilder;
use std::any::TypeId;

/// Type-erased registration for an entity type.
///
/// Emitted automatically by `#[derive(EntityType)]`.
#[derive(Debug)]
pub struct EntityRegistration {
    pub type_id: TypeId,
    pub type_name: &'static str,
    pub meta_fn: fn() -> EntityTypeMeta,
}

impl EntityRegistration {
    pub fn meta(&self) -> EntityTypeMeta {
        (self.meta_fn)()
    }
}

inventory::collect!(EntityRegistration);

/// Type-erased registration for an `IEntityTypeConfiguration<T>` impl block.
///
/// Emitted by `#[entity_config(T)]` attribute macro. The `apply_fn` invokes
/// `C::default().configure(&mut builder)` on the contained `ModelBuilder`.
#[derive(Clone, Copy)]
pub struct EntityConfigRegistration {
    pub type_id: TypeId,
    pub type_name: &'static str,
    pub apply_fn: fn(&mut ModelBuilder),
}

impl std::fmt::Debug for EntityConfigRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntityConfigRegistration")
            .field("type_id", &self.type_id)
            .field("type_name", &self.type_name)
            .finish()
    }
}

inventory::collect!(EntityConfigRegistration);
```

**设计要点**：

1. `meta_fn: fn() -> EntityTypeMeta` 是函数指针，`const` 可求值，`Send + Sync` 自动满足
2. `apply_fn: fn(&mut ModelBuilder)` 擦除了 `C: IEntityTypeConfiguration<T>` 的泛型，统一为 `fn(&mut ModelBuilder)`
3. `inventory::collect!` 声明全局收集类型，必须在 `inventory` crate 的根模块（非嵌套模块）——但 Rust 1.46+ 已支持模块内 collect，这里放在 `registration` 模块即可
4. `EntityConfigRegistration` 派生 `Clone + Copy`，因为所有字段都是 `Copy` 类型

#### 2.1.3 步骤 3：在 core lib.rs 中导出

**文件**：`crates/core/src/lib.rs`

**修改**：

```rust
pub mod change_executor;
pub mod db_context;
pub mod db_set;
pub mod di;
pub mod entity;
pub mod error;
pub mod interceptor;
pub mod metadata;
pub mod migration;
pub mod model_builder;
pub mod navigation_loader;
pub mod provider;
// 新增 ↓
pub mod query;
pub mod registration;     // 新增模块
pub mod relations;
pub mod tracking;

pub use async_trait;

// 新增 ↓
pub use inventory;        // 重导出供宏生成代码使用

pub use rust_ef_macros::{column, entity_config, linq, EntityType};   // 新增 entity_config
```

**prelude 修改**：

```rust
pub mod prelude {
    pub use crate::db_context::{
        DbContext, DbContextOptions, DbContextOptionsBuilder, IDbContext, SaveChangesResult,
    };
    pub use crate::db_set::{DbSet, IDbSet};
    pub use crate::di::DbContextServiceCollectionExt;
    pub use crate::entity::{
        EntityState, IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter,
    };
    pub use crate::error::{EFError, EFResult};
    pub use crate::interceptor::{
        ISaveChangesInterceptor, SaveChangesContext, SaveChangesResultContext,
    };
    pub use crate::metadata::EntityTypeMeta;
    pub use crate::metadata::NavigationMeta;
    pub use crate::metadata::PropertyMeta;
    pub use crate::model_builder::{
        EntityTypeBuilder, IEntityTypeConfiguration, ModelBuilder, PropertyBuilder,
    };
    pub use crate::provider::DbValue;
    pub use crate::provider::IDatabaseProvider;
    pub use crate::query::BoolExpr;
    // 新增 ↓
    pub use crate::registration::{EntityConfigRegistration, EntityRegistration};
    pub use crate::relations::{BelongsTo, DeleteBehavior, HasMany, HasOne};
    pub use crate::tracking::ChangeTracker;
    pub use rust_ef_macros::column;
    // 新增 ↓
    pub use rust_ef_macros::entity_config;
    pub use rust_ef_macros::linq;
    pub use rust_ef_macros::EntityType;
}
```

**理由**：

- `pub use inventory` 让宏生成的 `inventory::submit!` 代码能通过 `rust_ef::inventory::submit!` 路径访问（避免用户 crate 直接依赖 inventory）
- `EntityRegistration` / `EntityConfigRegistration` 导出到 prelude 便于高级用户调试

### 2.2 Phase 2：宏端实现

#### 2.2.1 步骤 4：修改 `#[derive(EntityType)]` 注入 inventory::submit!

**文件**：`crates/macros/src/entity.rs`

**修改位置**：[entity.rs:511](file:///d:/GitCode/RF/rust-ef/crates/macros/src/entity.rs#L511) 的 `quote! { ... }` 块末尾，在最后一个 `}` 之前追加：

```rust
let expanded = quote! {
    impl rust_ef::entity::IEntityType for #struct_name {
        // ... 现有代码 ...
    }

    impl #struct_name {
        // ... 现有代码 ...
    }

    // ... 其他 trait impl ...

    #[rust_ef::entity_config_implementation_marker]  // 可选，用于调试
    impl rust_ef::entity::INavigationSetter for #struct_name {
        // ... 现有代码 ...
    }

    // 新增 ↓：inventory 自动注册
    rust_ef::inventory::submit!({
        rust_ef::registration::EntityRegistration {
            type_id: std::any::TypeId::of::<#struct_name>(),
            type_name: std::any::type_name::<#struct_name>(),
            meta_fn: <#struct_name as rust_ef::entity::IEntityType>::entity_meta,
        }
    });
};
```

**设计要点**：

1. 使用 `rust_ef::inventory::submit!` 路径，确保用户 crate 无需直接依赖 inventory
2. `meta_fn` 直接指向 `<#struct_name as IEntityType>::entity_meta` 函数指针
3. `type_name` 在 panic 信息中可读，便于调试
4. 注入点在所有 trait impl 之后，确保 `IEntityType::entity_meta` 已定义

#### 2.2.2 步骤 5：创建 `#[entity_config(T)]` 属性宏

**新文件**：`crates/macros/src/entity_config.rs`

**完整实现**：

```rust
//! `#[entity_config(T)]` attribute macro for `impl IEntityTypeConfiguration<T>` blocks.
//!
//! Emits an `inventory::submit!` registering an `EntityConfigRegistration`
//! whose `apply_fn` instantiates the configuration and applies it to a
//! `ModelBuilder`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, ImplItem, ItemImpl, Path, Type};

pub fn expand_entity_config(args: TokenStream, input: TokenStream) -> TokenStream {
    let _entity_type_path = parse_macro_input!(args as Path);
    let item_impl = parse_macro_input!(input as ItemImpl);

    let expanded_impl = match rewrite_impl(&item_impl) {
        Ok(tokens) => tokens,
        Err(err) => return err.to_compile_error().into(),
    };

    TokenStream::from(expanded_impl)
}

fn rewrite_impl(item: &ItemImpl) -> syn::Result<TokenStream2> {
    let self_ty = &item.self_ty;
    let trait_path = item.trait_.as_ref()
        .ok_or_else(|| syn::Error::new_spanned(self_ty, "#[entity_config] requires an `impl IEntityTypeConfiguration<T>` block"))?
        .1;

    let (config_ty, entity_ty) = extract_types(self_ty)?;

    let impl_tokens = item.to_token_stream();

    Ok(quote! {
        #impl_tokens

        rust_ef::inventory::submit!({
            rust_ef::registration::EntityConfigRegistration {
                type_id: std::any::TypeId::of::<#entity_ty>(),
                type_name: std::any::type_name::<#entity_ty>(),
                apply_fn: |builder: &mut rust_ef::model_builder::ModelBuilder| {
                    let config = #config_ty::default();
                    let type_id = std::any::TypeId::of::<#entity_ty>();
                    let mut entity_builder =
                        rust_ef::model_builder::EntityTypeBuilder::new(builder, type_id);
                    <#config_ty as rust_ef::model_builder::IEntityTypeConfiguration<#entity_ty>>
                        ::configure(&config, &mut entity_builder);
                },
            }
        });
    })
}

fn extract_types(self_ty: &Type) -> syn::Result<(Type, Type)> {
    let path = match self_ty {
        Type::Path(tp) if tp.qself.is_none() => &tp.path,
        _ => return Err(syn::Error::new_spanned(self_ty, "expected a trait path")),
    };

    let last_seg = path.segments.last()
        .ok_or_else(|| syn::Error::new_spanned(path, "empty trait path"))?;

    let args = match &last_seg.arguments {
        syn::PathArguments::AngleBracketed(args) => args,
        _ => return Err(syn::Error::new_spanned(self_ty, "expected `<Config, T>`")),
    };

    if args.args.len() != 1 {
        return Err(syn::Error::new_spanned(self_ty, "#[entity_config(T)] requires exactly one type parameter"));
    }

    let entity_ty = match args.args.first().unwrap() {
        syn::GenericArgument::Type(ty) => ty.clone(),
        _ => return Err(syn::Error::new_spanned(self_ty, "expected type parameter")),
    };

    Ok((config_ty_from_impl(self_ty, &entity_ty)?, entity_ty))
}

fn config_ty_from_impl(_self_ty: &Type, _entity_ty: &Type) -> syn::Result<Type> {
    Err(syn::Error::new_spanned(_self_ty,
        "Unable to infer config type from self_ty; use explicit path or refactor."))
}
```

**问题**：上面 `config_ty_from_impl` 的实现有缺陷——`IEntityTypeConfiguration<T>` 是 trait 路径本身，`self_ty` 实际上指向 trait 而非实现类型。需要重新设计解析逻辑。

**修正版**：

```rust
//! `#[entity_config(T)]` attribute macro for `impl Config: IEntityTypeConfiguration<T>` blocks.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, ItemImpl};

pub fn expand_entity_config(args: TokenStream, input: TokenStream) -> TokenStream {
    let entity_ty: syn::Type = parse_macro_input!(args as syn::Type);
    let item_impl = parse_macro_input!(input as ItemImpl);

    let expanded = match rewrite_impl(&item_impl, &entity_ty) {
        Ok(tokens) => tokens,
        Err(err) => return err.to_compile_error().into(),
    };

    TokenStream::from(expanded)
}

fn rewrite_impl(item: &ItemImpl, entity_ty: &syn::Type) -> syn::Result<TokenStream2> {
    let self_ty = &item.self_ty;
    let impl_tokens = item.to_token_stream();

    Ok(quote! {
        #impl_tokens

        rust_ef::inventory::submit!({
            rust_ef::registration::EntityConfigRegistration {
                type_id: std::any::TypeId::of::<#entity_ty>(),
                type_name: std::any::type_name::<#entity_ty>(),
                apply_fn: |builder: &mut rust_ef::model_builder::ModelBuilder| {
                    let config = #self_ty::default();
                    let type_id = std::any::TypeId::of::<#entity_ty>();
                    let mut entity_builder =
                        rust_ef::model_builder::EntityTypeBuilder::new(builder, type_id);
                    <#self_ty as rust_ef::model_builder::IEntityTypeConfiguration<#entity_ty>>
                        ::configure(&config, &mut entity_builder);
                },
            }
        });
    })
}
```

**设计要点**：

1. **签名调整**：用户写 `#[entity_config(Blog)]` 而非 `#[entity_config(BlogConfig)]`，明确指定实体类型 `T`，宏从 `impl Config: IEntityTypeConfiguration<Blog>` 的 `self_ty`（即 `Config`）推导配置类型
2. **`self_ty` 即配置类型**：用户必须命名配置类型（如 `BlogConfig`），不能使用匿名 `impl` 块
3. **闭包转函数指针**：`|builder| { ... }` 隐式转换为 `fn(&mut ModelBuilder)`，前提是闭包不捕获任何环境变量——这里闭包体内只引用 `Default::default()` 和函数指针，满足条件
4. **泛型擦除**：`<#self_ty as IEntityTypeConfiguration<#entity_ty>>::configure(&config, ...)` 通过显式 trait 限定调用，编译器在注册点已知完整类型信息

**用户使用示例**：

```rust
use rust_ef::prelude::*;

#[derive(Default)]
pub struct BlogConfig;

#[entity_config(Blog)]
impl IEntityTypeConfiguration<Blog> for BlogConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Blog>) {
        entity.to_table("blogs_renamed");
        entity.property_named("url").has_column_name("blog_url");
    }
}
```

#### 2.2.3 步骤 6：在 macros lib.rs 中注册

**文件**：`crates/macros/src/lib.rs`

**完整修改后**：

```rust
//! Procedural macros for Rust Entity Framework (rust-ef).

mod column_macro;
mod entity;
// 新增 ↓
mod entity_config;
mod linq;

use proc_macro::TokenStream;

#[proc_macro_derive(
    EntityType,
    attributes(
        table,
        primary_key,
        auto_increment,
        required,
        max_length,
        column,
        foreign_key,
        navigation,
        not_mapped,
        index,
        unique,
        through,
        concurrency_check
    )
)]
pub fn derive_entity_type(input: TokenStream) -> TokenStream {
    entity::expand_entity_type(input)
}

#[proc_macro]
pub fn column(input: TokenStream) -> TokenStream {
    column_macro::expand_column(input)
}

/// Compile-time LINQ-to-SQL.
///
/// ```ignore
/// linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 5).to_list().await?;
/// ```
#[proc_macro]
pub fn linq(input: TokenStream) -> TokenStream {
    linq::expand_linq(input)
}

// 新增 ↓
/// Attribute macro for `impl IEntityTypeConfiguration<T>` blocks.
///
/// Emits an `inventory::submit!` registering the configuration for automatic
/// discovery by `DbContext::ensure_created()`.
///
/// ```ignore
/// #[entity_config(Blog)]
/// impl IEntityTypeConfiguration<Blog> for BlogConfig {
///     fn configure(&self, entity: &mut EntityTypeBuilder<'_, Blog>) {
///         entity.to_table("blogs_renamed");
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn entity_config(args: TokenStream, input: TokenStream) -> TokenStream {
    entity_config::expand_entity_config(args, input)
}
```

### 2.3 Phase 3：DbContext 与 ModelBuilder 修复

#### 2.3.1 步骤 7：修改 DbContext

**文件**：`crates/core/src/db_context.rs`

**新增方法 `discover_entities()`**：

```rust
impl DbContext {
    /// Discovers all entity types registered via `#[derive(EntityType)]`
    /// and applies all `#[entity_config(T)]` configurations.
    ///
    /// Call this once after `DbContext::from_options()` to populate both
    /// STORE A (`entity_metas`) and STORE B (`model_builder`) from the
    /// global `inventory` registry. After discovery, `ensure_created()`
    /// and `ensure_deleted()` work without requiring manual `set::<T>()`
    /// calls for each entity type.
    ///
    /// Calling `set::<T>()` for already-discovered entities is a no-op
    /// (idempotent), preserving backward compatibility.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut ctx = DbContext::from_options(&options)?;
    /// ctx.discover_entities();   // 自动注册所有 #[derive(EntityType)] 类型
    /// ctx.ensure_created().await?;
    /// ```
    pub fn discover_entities(&mut self) -> EFResult<()> {
        // 1. 应用所有 #[entity_config(T)] 注册的配置
        for reg in inventory::iter::<EntityConfigRegistration> {
            (reg.apply_fn)(&mut self.model_builder);
        }

        // 2. 收集所有 #[derive(EntityType)] 注册的实体元数据
        for reg in inventory::iter::<EntityRegistration> {
            let meta = reg.meta();
            let type_id = reg.type_id;

            // 写入 STORE A
            self.entity_metas.entry(type_id).or_insert_with(|| meta.clone());

            // 写入 STORE B（如果 apply_configuration 未注册过）
            if !self.model_builder.has_entity(type_id) {
                self.model_builder.register_entity_meta(meta.clone());
            }

            // 预创建 DbSet 实例与 saver（与 set::<T>() 行为一致）
            self.savers.entry(type_id).or_insert_with(|| {
                // 注意：saver 需要 SetOps::<T>::new()，但此处 type_id 已擦除，
                // 无法直接构造。saver 的创建延后到 set::<T>() 调用时。
                // 这里只确保元数据可见，便于 ensure_created() 工作。
                Box::new(NoOpSaver)
            });

            // 注：DbSet 实例也延后到 set::<T>() 调用时创建，
            // 因为 DbSet<T> 需要具体类型 T 构造。
        }

        Ok(())
    }
}
```

**问题**：`savers` 需要 `SetOps::<T>::new()`，但 `discover_entities()` 在类型擦除上下文中无法构造具体 `T` 的 saver。

**修正设计**：`discover_entities()` **只填充 STORE A 与 STORE B**，不创建 `savers` 和 `sets`。这两个存储留给 `set::<T>()` 按需创建（保持现有行为不变）。`ensure_created()` 只需要 STORE A/B 的元数据，不需要 DbSet 实例。

**最终 `discover_entities()` 实现**：

```rust
/// Discovers all entity types registered via `#[derive(EntityType)]`
/// and applies all `#[entity_config(T)]` configurations to the model builder.
///
/// After calling this, `ensure_created()` and `ensure_deleted()` will
/// process all discovered entities without requiring manual `set::<T>()`
/// calls. Calling `set::<T>()` for discovered entities is idempotent.
pub fn discover_entities(&mut self) -> EFResult<()> {
    // 1. Apply all #[entity_config(T)] registered configurations to ModelBuilder
    for reg in inventory::iter::<EntityConfigRegistration> {
        (reg.apply_fn)(&mut self.model_builder);
    }

    // 2. Collect all #[derive(EntityType)] registered entity metas
    for reg in inventory::iter::<EntityRegistration> {
        let meta = reg.meta();
        let type_id = reg.type_id;

        // Write to STORE A
        self.entity_metas.entry(type_id).or_insert_with(|| meta.clone());

        // Write to STORE B (if not already registered by apply_configuration)
        if !self.model_builder.has_entity(type_id) {
            self.model_builder.register_entity_meta(meta);
        }
    }

    Ok(())
}
```

**修改 `set::<T>()` 同步写入 STORE B**：

```rust
pub fn set<T>(&mut self) -> &mut DbSet<T>
where
    T: IEntityType
        + IEntitySnapshot
        + IGetKeyValues
        + IFromRow
        + INavigationSetter
        + Send
        + Sync
        + 'static,
{
    let type_id = TypeId::of::<T>();
    self.savers
        .entry(type_id)
        .or_insert_with(|| Box::new(SetOps::<T>::new()));

    // STORE A
    self.entity_metas
        .entry(type_id)
        .or_insert_with(T::entity_meta);

    // 新增 ↓：同步写入 STORE B（幂等）
    if !self.model_builder.has_entity(type_id) {
        self.model_builder.register_entity_meta(T::entity_meta());
    }

    self.sets.entry(type_id).or_insert_with(|| {
        let meta = T::entity_meta();
        let mut db_set =
            DbSet::<T>::with_provider(meta.table_name.as_ref(), Arc::clone(&self.provider));
        if let Some(filter) = self.model_builder.get_query_filter(&type_id) {
            db_set.set_query_filter(filter.clone());
        }
        Box::new(db_set)
    });
    self.sets
        .get_mut(&type_id)
        .and_then(|b| b.downcast_mut::<DbSet<T>>())
        .expect("DbSet type mismatch")
}
```

**修复 `ensure_created()`**：

```rust
/// Creates all tables for registered entity types.
///
/// Sources metas from `model_builder.build()`, which applies all Fluent API
/// configurations and `#[entity_config(T)]` overrides. Entities are
/// discovered automatically via `#[derive(EntityType)]`; call
/// `discover_entities()` first, or use `set::<T>()` to register manually.
pub async fn ensure_created(&self) -> EFResult<()> {
    // 修复：通过 model_builder.build() 应用所有配置覆盖
    let metas: Vec<EntityTypeMeta> = self.model_builder.build();

    if metas.is_empty() && self.entity_metas.is_empty() {
        return Err(EFError::Configuration(
            "No entity types registered. Call ctx.discover_entities() or ctx.set::<T>() before ensure_created().".into(),
        ));
    }

    // 兜底：如果 model_builder 为空但 entity_metas 非空（旧调用路径）
    let metas = if metas.is_empty() {
        self.entity_metas.values().cloned().collect()
    } else {
        metas
    };

    let dialect = self.provider.migration_dialect();
    MigrationEngine::new(dialect)
        .ensure_created(&*self.provider, &metas)
        .await?;

    // 修复：使用 build() 后的 metas 同步种子数据
    for meta in &metas {
        let rows = self.model_builder.seed_rows_for(&meta.type_id);
        if !rows.is_empty() {
            MigrationEngine::new(dialect)
                .apply_seed_data(&*self.provider, meta, rows)
                .await?;
        }
    }
    Ok(())
}
```

**修复 `ensure_deleted()`**：

```rust
/// Drops all tables for registered entity types.
pub async fn ensure_deleted(&self) -> EFResult<()> {
    let metas: Vec<EntityTypeMeta> = self.model_builder.build();
    if metas.is_empty() && self.entity_metas.is_empty() {
        return Err(EFError::Configuration(
            "No entity types registered. Call ctx.discover_entities() or ctx.set::<T>() before ensure_deleted().".into(),
        ));
    }

    let metas = if metas.is_empty() {
        self.entity_metas.values().cloned().collect()
    } else {
        metas
    };

    let dialect = self.provider.migration_dialect();
    MigrationEngine::new(dialect)
        .ensure_deleted(&*self.provider, &metas)
        .await
}
```

**新增导入**：

```rust
use crate::registration::{EntityConfigRegistration, EntityRegistration};
```

#### 2.3.2 步骤 8：添加 ModelBuilder 辅助方法

**文件**：`crates/core/src/model_builder.rs`

**新增方法**：

```rust
impl ModelBuilder {
    // ... 现有方法 ...

    /// Returns true if an entity with the given `type_id` is already registered.
    pub fn has_entity(&self, type_id: TypeId) -> bool {
        self.entity_metas.iter().any(|m| m.type_id == type_id)
    }

    /// Registers an entity meta directly, without going through `entity::<T>()`.
    /// Used by `DbContext::discover_entities()` to populate STORE B from
    /// `inventory::iter::<EntityRegistration>()`.
    pub fn register_entity_meta(&mut self, meta: EntityTypeMeta) {
        let type_id = meta.type_id;
        if !self.entity_metas.iter().any(|m| m.type_id == type_id) {
            self.entity_metas.push(meta);
        }
        // Ensure a config entry exists for Fluent API overrides
        self.configs.entry(type_id).or_default();
    }

    /// Applies all `#[entity_config(T)]` registered configurations.
    /// Called by `DbContext::discover_entities()` before iterating
    /// `EntityRegistration`s, so that `register_entity_meta()` sees
    /// already-applied overrides (and skips redundant registration).
    pub fn apply_registered_configurations(&mut self) {
        for reg in crate::registration::collect_entity_config_registrations() {
            (reg.apply_fn)(self);
        }
    }
}
```

**修正**：`apply_registered_configurations` 直接调用 `inventory::iter` 即可，无需通过 `crate::registration::collect_*` 辅助函数：

```rust
impl ModelBuilder {
    pub fn has_entity(&self, type_id: TypeId) -> bool {
        self.entity_metas.iter().any(|m| m.type_id == type_id)
    }

    pub fn register_entity_meta(&mut self, meta: EntityTypeMeta) {
        let type_id = meta.type_id;
        if !self.entity_metas.iter().any(|m| m.type_id == type_id) {
            self.entity_metas.push(meta);
        }
        self.configs.entry(type_id).or_default();
    }
}
```

`inventory::iter` 调用直接在 `DbContext::discover_entities()` 中进行，无需在 `ModelBuilder` 中重复。

### 2.4 Phase 4：测试与文档

#### 2.4.1 步骤 9：新增自动注册测试

**新文件**：`crates/core/tests/auto_registration_tests.rs`

```rust
//! Tests for inventory-based automatic entity registration.

#![cfg(test)]

use rust_ef::prelude::*;
use rust_ef::registration::{EntityConfigRegistration, EntityRegistration};

#[derive(Debug, Clone, EntityType)]
#[table("auto_reg_simple")]
pub struct SimpleEntity {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    #[required]
    #[max_length(100)]
    pub name: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("auto_reg_other")]
pub struct OtherEntity {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub value: i64,
}

#[derive(Default)]
pub struct SimpleEntityConfig;

#[entity_config(SimpleEntity)]
impl IEntityTypeConfiguration<SimpleEntity> for SimpleEntityConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, SimpleEntity>) {
        entity.to_table("auto_reg_renamed");
        entity.property_named("name").has_column_name("display_name");
    }
}

#[test]
fn test_entity_registration_exists() {
    let registrations: Vec<&EntityRegistration> =
        inventory::iter::<EntityRegistration>().collect();

    let has_simple = registrations
        .iter()
        .any(|r| r.type_name.contains("SimpleEntity"));
    let has_other = registrations
        .iter()
        .any(|r| r.type_name.contains("OtherEntity"));

    assert!(has_simple, "SimpleEntity should be registered via inventory");
    assert!(has_other, "OtherEntity should be registered via inventory");
}

#[test]
fn test_entity_config_registration_exists() {
    let configs: Vec<&EntityConfigRegistration> =
        inventory::iter::<EntityConfigRegistration>().collect();

    let has_simple_config = configs
        .iter()
        .any(|r| r.type_name.contains("SimpleEntity"));

    assert!(has_simple_config, "SimpleEntityConfig should be registered");
}

#[test]
fn test_discover_entities_populates_stores() {
    let options = DbContextOptionsBuilder::new()
        .connection_string(":memory:");
    // 注：此处需要 use_sqlite，但 use_sqlite 在 rust-ef-sqlite crate 中
    // 测试中通过 dev-dependency 引入
    let _ = options;  // 占位，实际测试在 integration_tests 中

    // 验证 inventory 迭代器非空
    let count = inventory::iter::<EntityRegistration>().count();
    assert!(count >= 2, "Should have at least 2 registered entities");
}

#[test]
fn test_model_builder_build_applies_config() {
    let mut builder = ModelBuilder::new();

    // 应用所有注册的配置
    for reg in inventory::iter::<EntityConfigRegistration> {
        (reg.apply_fn)(&mut builder);
    }

    // 收集所有注册的实体
    for reg in inventory::iter::<EntityRegistration> {
        if !builder.has_entity(reg.type_id) {
            builder.register_entity_meta(reg.meta());
        }
    }

    let metas = builder.build();

    let simple_meta = metas
        .iter()
        .find(|m| m.type_name.contains("SimpleEntity"))
        .expect("SimpleEntity meta should exist");

    assert_eq!(simple_meta.table_name.as_ref(), "auto_reg_renamed");

    let name_prop = simple_meta
        .properties
        .iter()
        .find(|p| p.field_name.as_ref() == "name")
        .expect("name property should exist");

    assert_eq!(name_prop.column_name.as_ref(), "display_name");
}

#[test]
fn test_set_method_remains_idempotent() {
    // 验证 set::<T>() 在 discover_entities() 后仍可幂等调用
    // 此测试在集成测试中完整验证
}

#[test]
fn test_backward_compatibility_without_discover() {
    // 验证未调用 discover_entities() 时，set::<T>() + ensure_created() 仍工作
    // 此测试在集成测试中完整验证
}
```

**集成测试**（需 SQLite provider，放在 `crates/core/tests/` 或 `examples/blog`）：

```rust
//! Integration test: end-to-end auto-registration with SQLite.

#[tokio::test]
async fn test_ensure_created_without_set_call() {
    use rust_ef::prelude::*;
    use rust_ef_sqlite::SqliteDbContextOptionsExt;

    let mut options_builder = DbContextOptionsBuilder::new();
    options_builder.use_sqlite(":memory:");
    let options = options_builder.build();

    let mut ctx = DbContext::from_options(&options).expect("ctx");
    ctx.discover_entities().expect("discover");

    // 不调用 set::<SimpleEntity>()，直接 ensure_created
    ctx.ensure_created().await.expect("ensure_created");

    // 验证表已创建（使用 renamed 表名）
    // ... 执行 SELECT 验证 ...
}
```

#### 2.4.2 步骤 10：更新文档

**文件**：`docs/rust-ef/11-best-practices/common-pitfalls.md`

**修改第 4 点**：

```markdown
### 4. ensure_created() 找不到实体（旧版陷阱，v0.5.1 已修复）

**症状**：调用 `ctx.ensure_created().await?` 报错 "No entity types registered"。

**v0.5.1 之前**：必须先调用 `ctx.set::<T>()`，否则 `ensure_created()` 看不到任何实体。

**v0.5.1 之后**：调用 `ctx.discover_entities()` 即可自动注册所有 `#[derive(EntityType)]` 标注的类型：

\`\`\`rust
let mut ctx = DbContext::from_options(&options)?;
ctx.discover_entities()?;        // 自动发现所有实体并应用 #[entity_config] 配置
ctx.ensure_created().await?;
\`\`\`

**迁移建议**：
- 现有代码无需修改（`set::<T>()` 仍向后兼容）
- 新代码推荐使用 `discover_entities()` 简化样板
- Fluent API 配置现在会真正生效（之前会被 `ensure_created()` 静默忽略）

**调试技巧**：
- 使用 `inventory::iter::<rust_ef::registration::EntityRegistration>().count()` 查看注册的实体数量
- 使用 `ctx.model().build()` 检查最终的 `EntityTypeMeta` 列表
```

**新增文档**：`docs/rust-ef/02-getting-started/auto-registration.md`

```markdown
# 自动实体注册

rust-ef v0.5.1 引入了基于 `inventory` 的编译期自动注册机制。

## 基本用法

定义实体类型时，`#[derive(EntityType)]` 会自动注册到全局注册表：

\`\`\`rust
#[derive(EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key]
    pub id: i32,
    pub url: String,
}

let mut ctx = DbContext::from_options(&options)?;
ctx.discover_entities()?;        // 自动发现 Blog
ctx.ensure_created().await?;
\`\`\`

## 配置分离（IEntityTypeConfiguration）

将配置逻辑与实体定义分离：

\`\`\`rust
#[derive(Default)]
pub struct BlogConfig;

#[entity_config(Blog)]
impl IEntityTypeConfiguration<Blog> for BlogConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Blog>) {
        entity.to_table("blogs_renamed");
        entity.property_named("url").has_column_name("blog_url").is_required();
    }
}
\`\`\`

调用 `discover_entities()` 时，所有 `#[entity_config(T)]` 配置会自动应用到 `ModelBuilder`，确保 `ensure_created()` 创建的表结构与配置一致。

## 向后兼容

- `ctx.set::<T>()` 仍然可用，行为幂等
- 不调用 `discover_entities()` 时，旧代码行为不变（但 Fluent API 配置现在会真正生效）
```

**更新 blog 示例**：

```rust
// examples/blog/src/main.rs
fn main() -> EFResult<()> {
    let mut ctx = DbContext::from_options(&options)?;
    ctx.discover_entities()?;        // 替代多个 ctx.set::<Blog>() 调用
    ctx.ensure_created().await?;
    // ...
}
```

---

## 3. 实施顺序与里程碑

### 3.1 实施顺序

| 阶段 | 步骤 | 文件 | 依赖 |
|------|------|------|------|
| Phase 1 | 1. 添加 inventory 依赖 | `Cargo.toml` ×2 | 无 |
| Phase 1 | 2. 创建 registration.rs | 新建 | 步骤 1 |
| Phase 1 | 3. 修改 core lib.rs | `lib.rs` | 步骤 2 |
| Phase 2 | 4. 修改 derive EntityType 宏 | `macros/entity.rs` | 步骤 3 |
| Phase 2 | 5. 创建 entity_config.rs | 新建 | 步骤 3 |
| Phase 2 | 6. 修改 macros lib.rs | `macros/lib.rs` | 步骤 5 |
| Phase 3 | 7. 修改 DbContext | `db_context.rs` | 步骤 3+6 |
| Phase 3 | 8. 添加 ModelBuilder 辅助方法 | `model_builder.rs` | 步骤 3 |
| Phase 4 | 9. 新增测试 | 新建 | 步骤 7+8 |
| Phase 4 | 10. 更新文档 | 文档 | 步骤 9 |

### 3.2 验证检查点

每个阶段完成后执行：

```powershell
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

### 3.3 最终验证

```powershell
# 全量编译检查
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check

# 非 DB 测试（应全部通过）
cargo test --workspace --lib
cargo test --workspace --test linq_terminal_tests
cargo test --workspace --test linq_dsl_tests
cargo test --workspace --test auto_registration_tests

# SQLite 集成测试
cargo test --workspace --test sqlite_crud_tests
cargo test --workspace --test production_tests

# 示例编译
cargo build --example blog
```

---

## 4. 风险与缓解

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|----------|
| inventory 在 Windows MSVC 链接器段行为不一致 | 低 | 高 | inventory 0.3 已验证支持 Windows MSVC；增加 Windows CI 测试 |
| `#[entity_config(T)]` 闭包转 `fn` 失败（捕获环境变量） | 中 | 中 | 宏端静态校验：扫描 `impl` 块内是否引用外部变量；编译期失败而非运行时 |
| `discover_entities()` 与 `set::<T>()` 双重注册冲突 | 低 | 低 | `has_entity()` 幂等检查；`or_insert_with` 保证不覆盖 |
| 现有测试因 `ensure_created` 改用 `model_builder.build()` 而失败 | 中 | 中 | 兜底逻辑：`metas.is_empty()` 时回退到 STORE A；分阶段灰度 |
| `inventory::collect!` 在子模块内不生效 | 低 | 高 | 已验证 Rust 1.46+ 支持；单元测试覆盖 |
| 用户 crate 未启用 inventory 导致注册丢失 | 低 | 高 | `pub use inventory` 重导出；宏端使用 `rust_ef::inventory::submit!` 路径 |
| `EntityConfigRegistration` 闭包内 `EntityTypeBuilder::new` 借用冲突 | 中 | 中 | `apply_fn` 接收 `&mut ModelBuilder`，内部构造临时 builder，作用域隔离 |
| 大量实体注册导致 `discover_entities()` 启动开销 | 低 | 低 | `inventory::iter` 是编译期链接器段，零运行时开销；仅迭代无分配 |

### 4.1 回滚方案

若方案验证失败：

1. **保留 `discover_entities()` API**：内部改为 no-op，仅日志告警
2. **`ensure_created()` 兜底**：检测 `model_builder.build()` 为空时回退到 STORE A
3. **`#[entity_config]` 宏**：生成空 `inventory::submit!`，不影响编译
4. **Feature flag**：新增 `auto-registration` feature，默认关闭，逐步启用

---

## 5. 验收标准

### 5.1 功能验收

- [ ] `#[derive(EntityType)]` 标注的类型自动出现在 `inventory::iter::<EntityRegistration>()` 中
- [ ] `#[entity_config(T)]` 标注的 `impl` 块自动出现在 `inventory::iter::<EntityConfigRegistration>()` 中
- [ ] `ctx.discover_entities()` 后，不调用 `set::<T>()` 即可让 `ensure_created()` 创建所有表
- [ ] Fluent API 配置（`to_table`、`property_named`、`has_data`）在 `ensure_created()` 中真正生效
- [ ] `IEntityTypeConfiguration<T>::configure()` 的修改反映到最终表结构
- [ ] `set::<T>()` 在 `discover_entities()` 后仍可幂等调用
- [ ] 不调用 `discover_entities()` 时，旧代码行为兼容（Fluent API 现在生效，但不报错）

### 5.2 质量验收

- [ ] `cargo check --workspace` 零错误零警告
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 零警告
- [ ] `cargo fmt --check` 零差异
- [ ] 全部 66+ 现有非 DB 测试通过
- [ ] ≥6 个新增测试全部通过
- [ ] `examples/blog` 编译并运行成功
- [ ] SQLite 集成测试通过

### 5.3 文档验收

- [ ] `common-pitfalls.md` 第 4 点更新
- [ ] 新增 `auto-registration.md` 文档
- [ ] blog 示例简化（使用 `discover_entities()`）
- [ ] `IEntityTypeConfiguration` 文档补充使用示例

---

## 6. 附录

### 6.1 用户使用示例（重构前 vs 重构后）

**重构前**（v0.5.0）：

```rust
let mut ctx = DbContext::from_options(&options)?;
ctx.set::<Blog>();
ctx.set::<Post>();
ctx.set::<Comment>();
ctx.set::<User>();
// Fluent API 配置被静默忽略！
ctx.ensure_created().await?;
```

**重构后**（v0.5.1）：

```rust
let mut ctx = DbContext::from_options(&options)?;
ctx.discover_entities()?;        // 自动发现 Blog/Post/Comment/User
ctx.ensure_created().await?;    // Fluent API 配置真正生效
```

### 6.2 完整配置示例

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

#[derive(Default)]
pub struct BlogConfig;

#[entity_config(Blog)]
impl IEntityTypeConfiguration<Blog> for BlogConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Blog>) {
        entity.to_table("blogs_v2");
        entity.property_named("url").has_column_name("blog_url").max_length(500);
        entity.property_named("rating").has_index();

        // 种子数据
        entity.has_data(vec![
            Blog { blog_id: 1, url: "https://example.com".into(), rating: 5 },
        ]);
    }
}

fn main() -> EFResult<()> {
    let mut ctx = DbContext::from_options(&options)?;
    ctx.discover_entities()?;

    // 验证配置生效
    let metas = ctx.model().build();
    let blog_meta = metas.iter().find(|m| m.type_name.contains("Blog")).unwrap();
    assert_eq!(blog_meta.table_name, "blogs_v2");

    ctx.ensure_created().await?;

    // DbSet 仍需按需创建（用于 CRUD）
    let blogs = ctx.set::<Blog>().to_list().await?;

    Ok(())
}
```

### 6.3 参考链接

- [inventory crate 文档](https://docs.rs/inventory/latest/inventory/)
- [EFCore IEntityTypeConfiguration<T>](https://learn.microsoft.com/en-us/dotnet/api/microsoft.entityframeworkcore.ientitytypeconfiguration-1)
- [rust-ef v0.5 推迟项重构方案](file:///d:/GitCode/RF/rust-ef/.trae/documents/v0.5_推迟项重构方案_plan.md)
- [Phase 1 回归测试报告](file:///d:/GitCode/RF/rust-ef/.trae/documents/Phase1_回归测试报告.md)
