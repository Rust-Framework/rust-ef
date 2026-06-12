# Architecture Reference

## Trait Organization

```
Object-safe (dyn compatible)          Non-object-safe (Sized required)
─────────────────────────────────     ───────────────────────────────
IDbContext                            IEntityType
IDatabaseProvider                     IFromRow
ISqlGenerator                         IGetKeyValues
IAsyncConnection                      IEntitySnapshot
                                      IDbSet<T>
                                      IQueryable<T>
                                      IDbContextExt
                                      IEntityTypeConfiguration<T>
                                      FromDbContextOptions
```

## Dependency Flow

```
User Code
    ├── lrdi::ServiceCollection
    │     └── add_dbcontext::<DbContext>(|o| o.use_sqlite(...))
    │           └── stores DbContextOptions with provider_factory
    │
    └── Arc<dyn IDbContext> (from provider.get())
          └── DbContext
                ├── set::<T>() — type-map, lazy-create DbSet<T>
                ├── save_changes() — SetOps<T> dispatchers
                ├── provider() → &dyn IDatabaseProvider
                └── change_tracker() → &ChangeTracker
```

## Provider Factory Mechanism

1. `options.use_sqlite(cs)` injects a closure:
   `Arc<dyn Fn(&str) -> LrefResult<Arc<dyn IDatabaseProvider>>>`
2. `DbContext::from_options()` calls this closure
3. Core crate never imports any provider type

## Why No DbSet<Blog> Fields?

- **Before (EFCore pattern):** Context struct has `pub blogs: DbSet<Blog>`
  for every entity — adding an entity means changing the struct
- **After (type-map):** `ctx.set::<Blog>()` lazy-creates `DbSet<Blog>`
  from entity metadata — no struct changes needed

## Why Object-Safe IDbContext?

- Enables `Arc<dyn IDbContext>` DI resolution
- Generic methods (`use_transaction`) moved to `IDbContextExt`
- `type Provider` removed; `provider()` returns `&dyn IDatabaseProvider`

## Constraint Rules

- `BelongsTo<T>`, `HasMany<T>`, `HasOne<T>`: NO trait bounds (pure containers)
- `EntityTypeBuilder<T>`: NO `IEntityType` bound
- `set::<T>()`: requires `IEntityType + IEntitySnapshot + IGetKeyValues + IFromRow`
- `save_one_set()`: requires `IEntityType + IEntitySnapshot + IGetKeyValues + IFromRow`
