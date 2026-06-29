# Architecture Reference

## Trait Organization

```
Object-safe (dyn compatible)          Non-object-safe (Sized required)
─────────────────────────────────     ───────────────────────────────
IDatabaseProvider                     IEntityType
ISqlGenerator                         IFromRow
IAsyncConnection                      IGetKeyValues
ISaveChangesInterceptor               IEntitySnapshot
                                      IDbSet<T>
                                      IQueryable<T>
                                      IEntityTypeConfiguration<T>
```

## Dependency Flow

```
User Code
    ├── lrdi::ServiceCollection
    │    ├── add_dbcontext(|o| o.use_sqlite(...))
    │    │    └── stores DbContextOptions with provider_factory
    │    └── add_dbcontext_keyed("key", |o| ...)
    │          └── keyed registration for multi-DB
    │
    └── Arc<DbContext> (from provider.get() or provider.get_keyed("key"))
          └── DbContext
                ├── set::<T>() → type-map, lazy-create DbSet<T>
                ├── save_changes() → SetOps<T> dispatchers + interceptor pipeline
                ├── provider() → &dyn IDatabaseProvider
                └── change_tracker() → &ChangeTracker
```

## Provider Factory Mechanism

1. `options.use_sqlite(cs)` injects a closure:
   `Arc<dyn Fn(&str) -> EFResult<Arc<dyn IDatabaseProvider>>>`
2. `DbContext::from_options()` calls this closure
3. Core crate never imports any provider type

## SaveChanges Interceptor Pipeline

```
save_changes() called
    ├── detect_changes()
    ├── InterceptorPipeline::on_saving(ctx)  // pre-commit; Err aborts save
    ├── [execute SQL in transaction]
    ├── on success → InterceptorPipeline::on_saved(ctx, result)
    └── on failure → InterceptorPipeline::on_save_failed(ctx, error)
```

Interceptors are registered via `options.add_interceptor(impl ISaveChangesInterceptor)`.
Multiple interceptors run in registration order; the first error aborts the chain (fail-fast).

## Multi-DB Context (Keyed Registration)

Uses lrdi's `keyed_transient` mechanism:

```rust
.add_dbcontext_keyed("primary", |o| o.use_postgres(...))
.add_dbcontext_keyed("logs", |o| o.use_sqlite(...))
```

Resolution:
```rust
let primary: Arc<DbContext> = provider.get_keyed("primary");
let logs: Arc<DbContext> = provider.get_keyed("logs");
```

## Why No DbSet<Blog> Fields?

- **Before (EFCore pattern):** Context struct has `pub blogs: DbSet<Blog>`
  for every entity → adding an entity means changing the struct
- **After (type-map):** `ctx.set::<Blog>()` lazy-creates `DbSet<Blog>`
  from entity metadata → no struct changes needed

## DbContext 是具体上下文类型

`DbContext` 是具体上下文类型，无需 trait 抽象，直接通过 DI 注册为 `Arc<DbContext>`。
`use_transaction` 等方法是 `DbContext` 的固有方法；`provider()` 返回 `&dyn IDatabaseProvider`。

## Constraint Rules

- `BelongsTo<T>`, `HasMany<T>`, `HasOne<T>`: NO trait bounds (pure containers)
- `EntityTypeBuilder<T>`: NO `IEntityType` bound
- `set::<T>()`: requires `IEntityType + IEntitySnapshot + IGetKeyValues + IFromRow`
- `save_one_set()`: requires `IEntityType + IEntitySnapshot + IGetKeyValues + IFromRow`