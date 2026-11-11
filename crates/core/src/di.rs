//! DI integration — `AddDbContext` on `rust-dix` 0.10.
//!
//! Registers `DbContext` as **Scoped** and exposes the rust-ef-specific
//! registration/erasure helpers. rust-ef keeps only this module as its DI
//! integration surface: lifetimes, scope caching, owned resolution, disposal and
//! MEDI registration semantics are provided by rust-dix.
//!
//! # Resolution models
//!
//! - **Owned** (`get_owned` / `#[inject(owned)]`): caller-owned `DbContext` with
//!   `&mut self` access; allowed from the root provider and never cached.
//! - **Shared** (`scope.get()`): `Arc<DbContext>` with `&self` access, one
//!   instance per scope.
//!
//! ```rust,ignore
//! use rust_dix::*;
//! use rust_ef::di::*;
//!
//! let provider = ServiceCollection::new()
//!     .add_dbcontext(|options| {
//!         options.use_sqlite("data source=app.db");
//!     })
//!     .build()
//!     .unwrap();
//!
//! // One scope per request/job (`rust-webx` creates it automatically per request).
//! let scope = provider.scope();
//! let mut ctx: DbContext = scope.get_owned().expect("DbContext");
//! ctx.add::<Blog>(blog);
//! ctx.save_changes().await?;
//! ```
//!
//! # Registration variants
//!
//! | API | Lifetime of the provider initialization | Use when |
//! |-----|------------------------------------------|----------|
//! | [`DbContextServiceCollectionExt::add_dbcontext`] | eager, at registration (panics on failure) | simplest startup path; fail fast |
//! | [`DbContextServiceCollectionExt::add_dbcontext_async`] | during `build_async().await` (typed error) | async startup; provider failures become `RdiError` |
//! | [`DbContextServiceCollectionExt::add_dbcontext_keyed`] | eager, at registration | static read/write or module keys |
//! | [`DbContextServiceCollectionExt::add_dbcontext_keyed_async`] | during `build_async().await` | async startup with keyed contexts |
//! | [`DbContextServiceCollectionExt::add_dbcontext_anykey`] | lazily, per requested key (cached) | dynamic tenant/sharded contexts |
//!
//! ## Typed context keys
//!
//! [`DbContextKey`](crate::prelude::DbContextKey) generates [`IDbContextKey`]
//! (the stable string identity used by `#[context("...")]`) and rust-dix 0.10's
//! official extension contract `From<Key> for ServiceKey`:
//!
//! ```rust,ignore
//! #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, DbContextKey)]
//! enum Db { Read, Write }
//!
//! let provider = ServiceCollection::new()
//!     .add_dbcontext_keyed(Db::Read, |o| { o.use_default_entity_model(); o.use_sqlite("read.db"); })
//!     .add_dbcontext_keyed(Db::Write, |o| { o.use_default_entity_model(); o.use_sqlite("write.db"); })
//!     .build()?;
//!
//! let mut ctx = provider.get_owned_context(Db::Write)?;
//!
//! // rust-dix 0.10 macro key expressions use the generated From impl:
//! #[inject(scoped)]
//! struct WriteHandler {
//!     #[inject(owned, key = Db::Write)]
//!     ctx: DbContext,
//! }
//! ```
//!
//! Typed keys are consistent across registration, [`DbContextResolver`] and
//! handler injection. `&str` / `String` keys remain source-compatible and are
//! registered as `String` keys; typed enum keys and string keys are not
//! interchangeable. The entity model still matches `IDbContextKey::name()`.
//!
//! ## Dynamic keys (`AnyKey` fallback)
//!
//! [`DbContextServiceCollectionExt::add_dbcontext_anykey`] registers one
//! fallback for every key without an exact registration. The configure closure
//! receives the requested `ServiceKey` and builds per-key options; options and
//! the connection pool are cached per requested key in a bounded LRU
//! ([`DEFAULT_ANYKEY_CAPACITY`] entries, override with
//! [`DbContextServiceCollectionExt::add_dbcontext_anykey_with_capacity`]), so
//! repeated scopes reuse the same pool and tenant cardinality cannot grow memory
//! without bound:
//!
//! ```rust,ignore
//! let provider = ServiceCollection::new()
//!     .add_dbcontext_anykey(|key, options| {
//!         let tenant = key.to_string(); // map to a *known* tenant configuration
//!         options.use_default_entity_model();
//!         options.use_postgres(tenant_connection_string(&tenant));
//!     })
//!     .build()?;
//!
//! let scope = provider.scope();
//! let mut ctx = scope.get_keyed_owned::<DbContext>("tenant-a")?;
//! ```
//!
//! Security: never interpolate an untrusted requested key into SQL or a
//! connection string directly — resolve it through an allow-list/tenant table.
//! AnyKey contexts always use `use_default_entity_model()` (a dynamic key has no
//! `#[context("...")]` string); use global query filters for tenant isolation.
//!
//! ## Async provider initialization
//!
//! [`DbContextServiceCollectionExt::add_dbcontext_async`] defers provider
//! creation (connection pool) to `build_async().await`. A failure becomes a typed
//! `RdiError` instead of a registration-time panic. The `DbContext` itself stays
//! a synchronous Scoped service, so handlers keep using `#[inject(owned)]` /
//! `get_owned()` unchanged:
//!
//! ```rust,ignore
//! let provider = ServiceCollection::new()
//!     .add_dbcontext_async(|options| {
//!         options.use_postgres("postgres://app/db");
//!         // Optional: async validation after the pool is created.
//!         options.async_preflight(|options| {
//!             Box::pin(async move {
//!                 let provider = options.create_provider()?;
//!                 let mut conn = provider.get_connection().await?;
//!                 conn.query("SELECT 1", &[]).await?;
//!                 Ok(())
//!             })
//!         });
//!     })
//!     .build_async()
//!     .await?;
//!
//! let mut ctx: DbContext = provider.get_owned().expect("DbContext");
//! ```
//!
//! Calling `build()` (sync) with async registrations returns
//! `RdiError::AsyncServiceRequiresBuildAsync`; use `build_async().await`.
//!
//! # rust-dix 0.10 notes
//!
//! - **Macro key expressions**: `#[inject(key = <expr>)]` /
//!   `#[inject(owned, key = <expr>)]` accept any `impl Into<ServiceKey>`.
//!   [`DbContextKey`](crate::prelude::DbContextKey) implements the official
//!   `From<Key> for ServiceKey` contract; do not copy rust-dix's injection macro.
//! - **Typed vs string keys**: typed enum keys use `ServiceKey::new(key)`;
//!   `&str`/`String` registration keys stay `String` keys. They are intentionally
//!   not interchangeable (`ServiceKey::new("x")` ≠ `"x".into()`).
//! - **MSRV 1.95**; rust-ef 2.1 declares `rust-version = "1.95"`.
//! - **Async diagnostics**: a sync singleton depending on an async service reports
//!   `SyncSingletonDependsOnAsyncService`; resolving an uninitialized async
//!   service synchronously reports `AsyncServiceRequiresGetAsync`.
//! - **Async cycle detection** is chain-based; if an async provider factory ever
//!   spawns detached tasks that resolve services, capture
//!   `provider.async_context()` and resolve through it.
//! - **Async AnyKey is still not provided** by rust-dix 0.10; dynamic keys keep
//!   the synchronous per-key initialization described above.
//!
//! # Scoped lifetime & strict scope validation
//!
//! rust-dix 0.8+ enables MEDI `ValidateScopes` semantics by default: resolving a
//! Scoped service from the root provider returns
//! `RdiError::ScopedFromRootProvider` — the root provider is **not** a scope.
//! `get_owned` / `get_keyed_owned` are the exception (caller-owned, never cached);
//! their constructor's Scoped dependencies still obey validation.
//!
//! **Migrating from rust-dix 0.7 / rust-ef ≤ 2.0.** Root resolutions that relied
//! on root-scope caching must move to `provider.scope()` (shared) or owned
//! resolution. A temporary escape hatch is
//! `ServiceCollection::build_with(ServiceProviderOptions::lenient())`.
//!
//! # Disposal
//!
//! `Scope::drop` / `Scope::dispose()` release the Scoped `DbContext` instances
//! created in that scope in reverse creation order; `ServiceProvider::dispose()`
//! releases singletons (including async-initialized providers). After disposal,
//! resolution returns `ScopeDisposed` / `ProviderDisposed`.
//!
//! # Registration semantics
//!
//! `add_dbcontext*` uses normal `scoped` / `keyed_scoped` registrations, so
//! rust-dix's **last-registration-wins** rule applies: registering the same key
//! twice keeps the last registration. `try_add_*` behaves as MEDI `TryAdd`.
//!
//! # Recommended: owned resolution for handlers
//!
//! `DbContext` methods (`set::<T>()`, `save_changes()`, `detect_changes()`)
//! require `&mut self`; own the context via `get_owned()`, avoiding
//! `Arc<Mutex>` and interior mutability:
//!
//! ```rust,ignore
//! #[derive(Inject)]
//! pub struct CreateBlogHandler {
//!     #[inject(owned)]
//!     ctx: DbContext,
//! }
//! ```
//!
//! # Shared resolution (within a scope)
//!
//! Resolve `Arc<DbContext>` from a scope when several consumers in the same scope
//! must share one instance. `Arc<DbContext>` only provides `&self` access; prefer
//! owned resolution when mutation is needed.
//!
//! # Multiple databases (keyed)
//!
//! ```rust,ignore
//! let provider = ServiceCollection::new()
//!     .add_dbcontext_keyed("primary", |options| {
//!         options.use_postgres("host=primary/db");
//!     })
//!     .add_dbcontext_keyed("logs", |options| {
//!         options.use_sqlite("logs.db");
//!     })
//!     .build()?;
//!
//! let mut primary: DbContext = provider.get_keyed_owned("primary")?;
//! let scope = provider.scope();
//! let logs: Arc<DbContext> = scope.get_keyed("logs")?;
//! ```

use crate::db_context::{DbContext, DbContextOptions, DbContextOptionsBuilder};
use rust_dix::{IServiceResolver, RdiError, Scope};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Identity of a keyed `DbContext`.
///
/// `name()` must stay stable for the entity model: it is the string used by
/// `#[context("name")]` / `#[entity(T, "name")]`.
///
/// Prefer `#[derive(DbContextKey)]`, which implements this trait **and**
/// `From<Key> for rust_dix::ServiceKey` (typed key for registration, resolution
/// and `#[inject(owned, key = Key::Read)]`). Manual implementations must also
/// provide that `From` impl (the official rust-dix extension contract).
/// `&str`, `&String` and `String` are implemented for source compatibility and
/// register as string keys.
pub trait IDbContextKey: Send + Sync {
    /// Stable string identity of this key.
    fn name(&self) -> &str;
}

impl IDbContextKey for &str {
    fn name(&self) -> &str {
        self
    }
}

impl IDbContextKey for String {
    fn name(&self) -> &str {
        self.as_str()
    }
}

impl IDbContextKey for &String {
    fn name(&self) -> &str {
        self.as_str()
    }
}

/// Typed-context resolution helpers.
///
/// Keys are registered through rust-dix's typed `ServiceKey` path: enum key
/// types implementing `Into<ServiceKey>` (usually via `#[derive(DbContextKey)]`)
/// register a typed key; `&str`/`String` keys keep their `String` key. The two
/// forms are not interchangeable, matching rust-dix 0.10+ semantics.
/// `get_context` requires a scope (strict scope validation); use
/// [`DbContextResolver::get_owned_context`] for root-safe owned resolution.
pub trait DbContextResolver {
    /// Resolve an owned `DbContext` for `key` (allowed from the root provider).
    fn get_owned_context(
        &self,
        key: impl IDbContextKey + Into<ServiceKey>,
    ) -> Result<DbContext, RdiError>;

    /// Resolve a shared `Arc<DbContext>` for `key` (requires a scope).
    fn get_context(
        &self,
        key: impl IDbContextKey + Into<ServiceKey>,
    ) -> Result<Arc<DbContext>, RdiError>;
}

impl DbContextResolver for ServiceProvider {
    fn get_owned_context(
        &self,
        key: impl IDbContextKey + Into<ServiceKey>,
    ) -> Result<DbContext, RdiError> {
        self.get_keyed_owned(key)
    }

    fn get_context(
        &self,
        key: impl IDbContextKey + Into<ServiceKey>,
    ) -> Result<Arc<DbContext>, RdiError> {
        self.get_keyed(key)
    }
}

impl DbContextResolver for Scope {
    fn get_owned_context(
        &self,
        key: impl IDbContextKey + Into<ServiceKey>,
    ) -> Result<DbContext, RdiError> {
        self.get_keyed_owned(key)
    }

    fn get_context(
        &self,
        key: impl IDbContextKey + Into<ServiceKey>,
    ) -> Result<Arc<DbContext>, RdiError> {
        self.get_keyed(key)
    }
}

/// Async-initialized options wrapper registered as an async singleton by
/// `add_dbcontext_async*`. The synchronous Scoped `DbContext` factory resolves it
/// from the container, so handlers keep using owned resolution.
struct AsyncDbContextOptions(Arc<DbContextOptions>);

fn resolve_arc<T: Send + Sync + 'static>(resolver: &dyn IServiceResolver) -> Option<Arc<T>> {
    let any = resolver.get_any(std::any::type_name::<T>())?;
    any.downcast::<Arc<T>>().ok().map(|arc| Arc::clone(&*arc))
}

fn resolve_keyed_arc_by_key<T: Send + Sync + 'static>(
    resolver: &dyn IServiceResolver,
    key: &ServiceKey,
) -> Option<Arc<T>> {
    let any = resolver.get_keyed_any_key(std::any::type_name::<T>(), key)?;
    any.downcast::<Arc<T>>().ok().map(|arc| Arc::clone(&*arc))
}

fn build_options(configure: impl FnOnce(&mut DbContextOptionsBuilder)) -> Arc<DbContextOptions> {
    let mut builder = DbContextOptionsBuilder::new();
    configure(&mut builder);
    Arc::new(builder.build())
}

/// Default number of requested keys whose `AnyKey` options/provider are cached.
pub const DEFAULT_ANYKEY_CAPACITY: usize = 256;

/// Bounded LRU cache for `AnyKey` options/providers.
///
/// Dynamic keys have no compile-time bound, so an unbounded cache would grow
/// with tenant/shard cardinality. Eviction only drops the cached options; live
/// `DbContext`s keep their provider `Arc`, and a later resolution for an evicted
/// key builds a fresh pool.
struct AnyKeyOptionsCache {
    capacity: usize,
    clock: u64,
    entries: HashMap<ServiceKey, (Arc<DbContextOptions>, u64)>,
}

impl AnyKeyOptionsCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            clock: 0,
            entries: HashMap::new(),
        }
    }

    fn get_or_build(
        &mut self,
        key: &ServiceKey,
        build: impl FnOnce() -> Arc<DbContextOptions>,
    ) -> Arc<DbContextOptions> {
        self.clock += 1;
        let tick = self.clock;
        if let Some((options, last_used)) = self.entries.get_mut(key) {
            *last_used = tick;
            return Arc::clone(options);
        }

        let options = build();
        if self.entries.len() >= self.capacity {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries
            .insert(key.clone(), (Arc::clone(&options), tick));
        options
    }
}

fn async_options_factory(
    options: Arc<DbContextOptions>,
) -> impl Fn(
    Arc<rust_dix::ServiceProvider>,
)
    -> std::pin::Pin<Box<dyn std::future::Future<Output = Arc<AsyncDbContextOptions>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_provider| {
        let options = Arc::clone(&options);
        Box::pin(async move {
            options
                .create_provider()
                .expect("DbContext provider initialization failed during build_async");
            if let Some(preflight) = options.async_preflight.clone() {
                if let Err(err) = preflight(Arc::clone(&options)).await {
                    panic!("DbContext async preflight failed: {err}");
                }
            }
            Arc::new(AsyncDbContextOptions(options))
        })
    }
}

fn context_from_async_options(runtime: Arc<AsyncDbContextOptions>) -> Arc<DbContext> {
    let ctx = DbContext::from_options(&runtime.0).expect("Failed to create DbContext");
    Arc::new(ctx)
}

/// Adds the `DbContext` registration helpers to `rust_dix::ServiceCollection`.
pub trait DbContextServiceCollectionExt {
    /// Registers a `DbContext` as **Scoped** with the default key.
    ///
    /// The provider (connection pool) is created eagerly at registration time and
    /// a failure panics, fail-fast at startup. A later registration for the same
    /// key replaces this one (MEDI last-registration-wins).
    fn add_dbcontext(
        self,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self;

    /// Registers a typed/keyed `DbContext` as **Scoped**.
    ///
    /// `key` can be an enum deriving [`DbContextKey`](crate::prelude::DbContextKey)
    /// (which implements [`IDbContextKey`] and `Into<ServiceKey>`) or a
    /// `&str`/`String` (source-compatible; registered as a `String` key). The
    /// key's `name()` is used for the entity model.
    fn add_dbcontext_keyed(
        self,
        key: impl IDbContextKey + Into<ServiceKey>,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self;

    /// Registers **one fallback** `DbContext` for every requested key without an
    /// exact registration (rust-dix `AnyKey`).
    ///
    /// The configure closure receives the requested [`ServiceKey`] and builds
    /// per-key options. Built options/providers are cached per requested key
    /// (bounded LRU, [`DEFAULT_ANYKEY_CAPACITY`] entries), so connection pools
    /// are reused across scopes. The context always uses
    /// `use_default_entity_model()`; isolate tenants with query filters.
    ///
    /// Security: map the requested key through an allow-list/tenant table; never
    /// interpolate it directly into SQL or connection strings.
    fn add_dbcontext_anykey(
        self,
        configure: impl Fn(&ServiceKey, &mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self;

    /// [`Self::add_dbcontext_anykey`] with an explicit LRU capacity.
    ///
    /// `capacity` bounds how many requested keys keep a cached options/provider
    /// (and connection pool). Evicting an entry does not invalidate live
    /// `DbContext`s; a later resolution for that key builds a fresh pool.
    fn add_dbcontext_anykey_with_capacity(
        self,
        capacity: usize,
        configure: impl Fn(&ServiceKey, &mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self;

    /// Registers the default `DbContext` with **async-initialized** provider
    /// options.
    ///
    /// Provider creation runs during `build_async().await`; failures surface as
    /// `RdiError` instead of a registration-time panic. An
    /// [`DbContextOptionsBuilder::async_preflight`](crate::db_context::DbContextOptionsBuilder::async_preflight)
    /// hook, when configured, runs after provider creation. The `DbContext`
    /// remains a synchronous Scoped service, so owned resolution keeps working.
    /// A sync `build()` returns `RdiError::AsyncServiceRequiresBuildAsync`.
    fn add_dbcontext_async(
        self,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self;

    /// Keyed variant of [`Self::add_dbcontext_async`].
    fn add_dbcontext_keyed_async(
        self,
        key: impl IDbContextKey + Into<ServiceKey>,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self;
}

impl DbContextServiceCollectionExt for ::rust_dix::ServiceCollection {
    fn add_dbcontext(
        self,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self {
        let options = build_options(configure);
        // Eagerly build the provider (and its connection pool) at registration
        // time so a misconfigured connection string / TLS setup fails fast at
        // startup. The built provider is cached in `DbContextOptions`, so the
        // per-request scoped factory below reuses it via `Arc::clone`.
        options
            .create_provider()
            .expect("DbContext provider initialization failed at startup");
        self.scoped(move |_| {
            let ctx = DbContext::from_options(&options).expect("Failed to create DbContext");
            Arc::new(ctx) as Arc<DbContext>
        })
    }

    fn add_dbcontext_keyed(
        self,
        key: impl IDbContextKey + Into<ServiceKey>,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self {
        let key_name = key.name().to_string();
        let service_key: ServiceKey = key.into();
        let mut builder = DbContextOptionsBuilder::new();
        configure(&mut builder);
        if !builder.uses_default_entity_model() {
            builder.context_key(key_name);
        }
        let options = Arc::new(builder.build());
        options
            .create_provider()
            .expect("DbContext provider initialization failed at startup");
        self.keyed_scoped(service_key, move |_| {
            let ctx = DbContext::from_options(&options).expect("Failed to create DbContext");
            Arc::new(ctx) as Arc<DbContext>
        })
    }

    fn add_dbcontext_anykey(
        self,
        configure: impl Fn(&ServiceKey, &mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self {
        self.add_dbcontext_anykey_with_capacity(DEFAULT_ANYKEY_CAPACITY, configure)
    }

    fn add_dbcontext_anykey_with_capacity(
        self,
        capacity: usize,
        configure: impl Fn(&ServiceKey, &mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self {
        let options_cache = Arc::new(Mutex::new(AnyKeyOptionsCache::new(capacity)));
        self.keyed_any_scoped::<DbContext>(move |_resolver, key| {
            let options = {
                let mut cache = options_cache
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                cache.get_or_build(key, || {
                    let mut builder = DbContextOptionsBuilder::new();
                    configure(key, &mut builder);
                    // A dynamic requested key has no compile-time `#[context]`
                    // name, so the shared default entity model is the only model
                    // that can be resolved. Isolation belongs in query filters.
                    builder.use_default_entity_model();
                    let options = Arc::new(builder.build());
                    options
                        .create_provider()
                        .expect("AnyKey DbContext provider initialization failed");
                    options
                })
            };
            let ctx = DbContext::from_options(&options).expect("Failed to create DbContext");
            Arc::new(ctx) as Arc<DbContext>
        })
    }

    fn add_dbcontext_async(
        self,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self {
        let options = build_options(configure);
        self.async_singleton::<AsyncDbContextOptions>(async_options_factory(options))
            .scoped(move |resolver| {
                let runtime = resolve_arc::<AsyncDbContextOptions>(resolver).expect(
                    "async DbContext options are not initialized; call ServiceCollection::build_async()",
                );
                context_from_async_options(runtime)
            })
    }

    fn add_dbcontext_keyed_async(
        self,
        key: impl IDbContextKey + Into<ServiceKey>,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self {
        let key_name = key.name().to_string();
        let service_key: ServiceKey = key.into();
        let mut builder = DbContextOptionsBuilder::new();
        configure(&mut builder);
        if !builder.uses_default_entity_model() {
            builder.context_key(key_name);
        }
        let options = Arc::new(builder.build());
        let scoped_key = service_key.clone();
        self.async_keyed_singleton::<AsyncDbContextOptions>(
            service_key,
            async_options_factory(options),
        )
        .keyed_scoped(scoped_key.clone(), move |resolver| {
            let runtime = resolve_keyed_arc_by_key::<AsyncDbContextOptions>(resolver, &scoped_key)
                .expect("async keyed DbContext options are not initialized; call build_async()");
            context_from_async_options(runtime)
        })
    }
}

// Re-export the rust-dix 0.10 types users need for registration/build/scope
// control so a Rust project can integrate rust-ef without also depending on
// rust-dix directly. `ServiceProviderOptions` powers the strict/lenient
// scope-validation choice in `ServiceCollection::build_with`.
pub use rust_dix::{ServiceCollection, ServiceKey, ServiceProvider, ServiceProviderOptions};
