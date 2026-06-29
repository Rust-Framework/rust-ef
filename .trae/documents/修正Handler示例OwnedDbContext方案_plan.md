# 修正 Handler 示例：Owned DbContext 模式

> **背景**：移除 `IDbContext` trait 后，`DbContext` 的写方法（`set::<T>()`、`save_changes()`、`detect_changes()`）保留 `&mut self` 签名（Rust 惯例 + 最佳性能）。但 DI 返回 `Arc<DbContext>`，仅提供 `&self`，无法调用任何写方法——甚至读操作也需要 `set::<T>()`（`&mut self`）。因此 `Arc<DbContext>` 对 Handler 几乎不可用。
>
> 用户决策：不引入 interior mutability（RwLock/RefCell），保持 `&mut self`。唯一 Rust 惯用路径是 **Handler 拥有 owned `DbContext`**。

---

## 一、现状分析

### 1.1 问题代码（无法编译）

**`templates/di-setup.rs` L51/L57**：
```rust
let ctx: Arc<DbContext> = provider.get();  // Arc<DbContext>
ctx.save_changes().await?;                  // ❌ save_changes(&mut self)
```

**`templates/web-handler-crud.rs`**（全部 5 个 Handler）：
```rust
pub struct BlogHandler { ctx: Arc<DbContext> }  // 字段
async fn handle(&self, ...) {
    ctx.set::<Blog>().add(blog);    // ❌ set::<T>(&mut self)
    ctx.save_changes().await?;       // ❌ save_changes(&mut self)
}
```
> 注：示例中 `ctx` 未加 `self.` 前缀，但即便改为 `self.ctx.set::<Blog>()` 仍无法编译。

**`references/webapp-integration.md`** 2.3–2.8 节全部 Handler/Service 示例同样依赖 `ctx: Arc<DbContext>` + `ctx.set::<Blog>()`，均无法编译。

### 1.2 根因

| 维度 | 现状 | 后果 |
|------|------|------|
| `set::<T>()` 签名 | `&mut self` | 无法在 `Arc<DbContext>` 上调用 |
| `save_changes()` 签名 | `&mut self` | 同上 |
| `detect_changes()` 签名 | `&mut self` | 同上 |
| DI `scoped` 返回类型 | `Arc<DbContext>` | 仅 `&self`，且 `Arc::try_unwrap` 因 scope 持有引用而失败 |
| 读取路径 | 也需 `set::<T>()` 获取 `DbSet<T>` | 连读操作都不可用 |

**结论**：`Arc<DbContext>` 对 Handler 完全不可用。必须改为 owned `DbContext`。

### 1.3 可用 API 确认

`DbContext::from_options(options: &DbContextOptions) -> EFResult<DbContext>` 是 public 方法（[db_context.rs:319](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L319)），返回 owned `DbContext`，可自由调用 `&mut self` 方法。

`DbContextOptions` 是 `pub struct`（[db_context.rs:62](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L62)），`#[derive(Clone)]`，内部持有 `provider_factory: Arc<dyn Fn(&str) -> EFResult<Arc<dyn IDatabaseProvider>>>`，可安全克隆/共享。

### 1.4 EFCore 对齐

EFCore 的 Scoped 生命周期含义：**每个请求一个 DbContext 实例，请求结束释放**。在 .NET 中 DI 返回实例引用（GC 管理），自然可变。在 Rust 中，等价语义是 **Handler 每次处理创建 owned `DbContext`，处理完释放**——这正是 `DbContext::from_options()` 模式。

---

## 二、方案：Owned DbContext per Request

### 2.1 核心模式

Handler 注入 `Arc<DbContextOptions>`（DI 单例），每次 `handle()` 调用 `DbContext::from_options()` 创建 owned `DbContext`：

```rust
#[derive(Inject)]
pub struct BlogHandler {
    options: Arc<DbContextOptions>,
}

#[inject]
#[async_trait]
impl IRequestHandler<CreateBlogRequest, BlogModel> for BlogHandler {
    async fn handle(&self, req: CreateBlogRequest) -> Result<BlogModel> {
        let mut ctx = DbContext::from_options(&self.options)?;
        ctx.set::<Blog>().add(blog);
        ctx.save_changes().await?;
        Ok(...)
    }
}
```

**优点**：
- Rust 惯用（owned，无锁，无 interior mutability）
- 性能最佳（无 `Arc::try_unwrap`/`Mutex` 开销）
- 编译期安全（`&mut self` 由所有权保证）
- 与 EFCore Scoped 语义一致（每请求一实例）
- 与 `examples/blog` 一致（`let mut ctx = create_blog_context().await?`）

**代价**：
- Handler 不再共享同一 `DbContext` 实例（但现有示例本就是每 Handler 独立操作，不共享）
- 跨 Handler 事务需显式传递 owned `DbContext`（高级模式，文档说明）

### 2.2 DI 注册策略

`add_dbcontext` 同时注册两类服务：

1. `Arc<DbContextOptions>` — **singleton**（新增，供 Handler 注入）
2. `Arc<DbContext>` — **scoped**（保留，向后兼容，供 `ensure_created`、`IHostedService` 启动任务、只读后台服务使用）

**理由**：
- 向后兼容现有 `scoped_lifecycle_tests.rs`（3 个测试依赖 `scope.get::<Arc<DbContext>>()`）
- `IHostedService` 启动场景需 `Arc<DbContext>` 调用 `ensure_created(&self)`（只读方法，可用）
- Handler 场景需 `Arc<DbContextOptions>` 创建 owned `DbContext` 调用写方法

---

## 三、具体改动

### 3.1 框架代码：`crates/core/src/di.rs`

**改动 1**：`add_dbcontext` 追加注册 `Arc<DbContextOptions>` 为 singleton

当前 L94-107：
```rust
fn add_dbcontext(
    self,
    configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
) -> Self {
    let mut builder = DbContextOptionsBuilder::new();
    configure(&mut builder);
    let options = Arc::new(builder.build());

    self.scoped(move |_| {
        let ctx = DbContext::from_options(&options).expect("Failed to create DbContext");
        Arc::new(ctx) as Arc<DbContext>
    })
}
```

改为：
```rust
fn add_dbcontext(
    self,
    configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
) -> Self {
    let mut builder = DbContextOptionsBuilder::new();
    configure(&mut builder);
    let options = Arc::new(builder.build());

    // 注册 options 为 singleton，供 Handler 注入并创建 owned DbContext
    let options_for_singleton = Arc::clone(&options);
    self.singleton(move || Arc::clone(&options_for_singleton));

    // 同时注册 Arc<DbContext> 为 scoped（向后兼容 + 启动场景）
    self.scoped(move |_| {
        let ctx = DbContext::from_options(&options).expect("Failed to create DbContext");
        Arc::new(ctx) as Arc<DbContext>
    })
}
```

**改动 2**：`add_dbcontext_keyed` 同步追加 keyed singleton 注册（相同模式）

**改动 3**：更新模块文档（L1-58）

- L20: `let ctx: Arc<DbContext> = provider.get();` 后追加说明：此模式仅用于 `ensure_created` 等只读方法；写操作需注入 `Arc<DbContextOptions>`
- L42-58: 新增 "Handler 注入模式" 小节，说明注入 `Arc<DbContextOptions>` + `DbContext::from_options()` 模式

### 3.2 模板：`templates/di-setup.rs`

**改动**：在 `main()` 中展示两种解析模式

```rust
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = build_provider();

    // 模式 A：启动/后台场景——Arc<DbContext>（仅调用 &self 方法如 ensure_created）
    let ctx: Arc<DbContext> = provider.get();
    ctx.ensure_created().await?;

    // 模式 B：Handler/写操作场景——注入 Arc<DbContextOptions>，创建 owned DbContext
    let options: Arc<DbContextOptions> = provider.get();
    let mut ctx = DbContext::from_options(&options)?;
    ctx.set::<Blog>().add(blog);
    ctx.save_changes().await?;

    Ok(())
}
```

更新 L61-69 注释：说明 Handler 注入 `Arc<DbContextOptions>` 而非 `Arc<DbContext>`。

### 3.3 模板：`templates/web-handler-crud.rs`（全量重写）

**改动**：
1. Handler 字段 `ctx: Arc<DbContext>` → `options: Arc<DbContextOptions>`
2. 每个 `handle()` 方法首行：`let mut ctx = DbContext::from_options(&self.options)?;`
3. 所有 `ctx.set::<T>()` / `ctx.save_changes()` 改为 `self` → 局部 `ctx`
4. 更新顶部注释：说明 owned DbContext 模式 + 为什么不用 `Arc<DbContext>`

示例（CREATE Handler）：
```rust
#[derive(Inject)]
pub struct BlogHandler {
    options: Arc<DbContextOptions>,
}

#[inject]
#[async_trait]
impl IRequestHandler<CreateBlogRequest, BlogModel> for BlogHandler {
    async fn handle(&self, req: CreateBlogRequest) -> Result<BlogModel> {
        let mut ctx = DbContext::from_options(&self.options)?;

        // 1. 唯一性校验
        let set = ctx.set::<Blog>();
        let expr = linq!(|b: Blog| b.slug == req.slug);
        let exists = set.filter(expr).first_or_default().await?;
        if exists.is_some() {
            return Err("Slug already exists".into());
        }

        // 2. 插入
        let mut blog = req.to_entity(uid, now);
        ctx.set::<Blog>().add(blog);
        ctx.save_changes().await?;

        // 3. 按主键回查（带导航）
        let saved = linq!(ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
            include b.category;
            include b.author;
        ).first_or_default().await?
            .ok_or("Blog vanished after insert")?;

        Ok(saved.to_model())
    }
}
```

5 个 Handler（Create/ReadList/ReadSingle/Update/Delete）全部按此模式重写。

### 3.4 文档：`references/webapp-integration.md`

**改动范围**：2.2–2.8 节全部 Handler/Service 示例

**改动 1**（2.2 Handler 注入模式）：
- Handler struct 字段 `ctx: Arc<DbContext>` → `options: Arc<DbContextOptions>`
- 删除 L92 "`&mut self` 要求：`set::<T>()` 和 `save_changes()` 都需要 `&mut self`，因此 handler 方法使用 `&mut self`" 这段错误说明
- 替换为：说明 Handler 注入 `Arc<DbContextOptions>`，每次 `handle()` 创建 owned `DbContext`，可自由调用 `&mut self` 方法

**改动 2**（2.3–2.6 读取/创建/更新/删除示例）：
- 每个 `handle()` 方法首行追加 `let mut ctx = DbContext::from_options(&self.options)?;`
- 所有 `ctx.set::<Blog>()` 保持不变（现在 `ctx` 是 owned，可调用 `&mut self`）

**改动 3**（2.8 服务层模式）：
- `BlogService` 字段 `ctx: Arc<DbContext>` → `options: Arc<DbContextOptions>`
- 服务方法内部创建 owned `DbContext`

**改动 4**（2.9 变更追踪 / 2.10 导航 / 2.11 软删除）：
- 这些是模式说明，不涉及 Handler 字段，但代码片段中的 `ctx` 需注明来自 `DbContext::from_options()`

### 3.5 文档：`references/webapp-integration.md` 2.1 启动场景

**保留 `Arc<DbContext>`**：`DbInitService` 调用 `ensure_created(&self)`、`model().entity::<T>()` 等只读方法，`Arc<DbContext>` 仍可用。无需改动。

### 3.6 不改动项

- `crates/core/src/db_context.rs` — 不改 `&mut self` 签名（用户决策）
- `crates/core/src/di.rs` 的 `scoped`/`keyed_scoped` 注册 — 保留向后兼容
- `examples/blog/src/main.rs` — 已是 owned 模式（`let mut ctx = create_blog_context()...`），无需改
- `scoped_lifecycle_tests.rs` — 仍验证 `Arc<DbContext>` scoped 行为，无需改
- 历史 plan 文档 — 已加过时标注，不再改

---

## 四、验证

### 4.1 编译验证

```powershell
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

### 4.2 测试验证

```powershell
cargo test --workspace
```

- `scoped_lifecycle_tests.rs` 3 个测试仍通过（`Arc<DbContext>` scoped 注册保留）
- 新增 1 个测试：`scoped_options_singleton_tests.rs` 验证 `Arc<DbContextOptions>` 可从 DI 解析且为单例

### 4.3 模板/文档自检

- `templates/di-setup.rs` 模式 A/B 均可编译（概念性 `rust,ignore`）
- `templates/web-handler-crud.rs` 5 个 Handler 模式统一
- `references/webapp-integration.md` 中无残留 `ctx: Arc<DbContext>` 字段定义（启动场景除外）

### 4.4 Grep 验证

```powershell
# 不应再有 Handler 字段为 ctx: Arc<DbContext>（启动场景 DbInitService 除外）
grep -rn "ctx: Arc<DbContext>" .agents/skills/lref/
# 应仅匹配 DbInitService 或 startup 场景
```

---

## 五、决策记录

| 决策项 | 选择 | 理由 |
|--------|------|------|
| Handler 持有类型 | `Arc<DbContextOptions>` | owned `DbContext` 需要可变借用，`Arc<DbContext>` 无法提供 |
| DbContext 创建方式 | `DbContext::from_options()` per request | Rust 惯用，无锁，性能最佳 |
| `add_dbcontext` 注册 | 同时注册 singleton options + scoped ctx | 向后兼容 + 新增 Handler 注入能力 |
| `Arc<DbContext>` 是否保留 | 保留 | 启动场景（`ensure_created`、`IHostedService`）仅需 `&self` 方法 |
| interior mutability | 不采用 | 用户明确拒绝（性能 + Rust 惯例）|
| `set::<T>()` 签名 | 保持 `&mut self` | 用户决策：Rust 最佳特性 |

---

## 六、实施顺序

1. **di.rs** — 追加 `Arc<DbContextOptions>` singleton 注册 + 更新文档注释
2. **di-setup.rs** — 展示两种解析模式
3. **web-handler-crud.rs** — 全量重写 5 个 Handler
4. **webapp-integration.md** — 更新 2.2–2.8 节 Handler/Service 示例
5. **新增测试** — `scoped_options_singleton_tests.rs`
6. **验证** — cargo check/clippy/fmt/test + grep 自检
