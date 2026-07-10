//! `DbContextOptions` and `DbContextOptionsBuilder` — provider factory and
//! context-level configuration.

use crate::error::EFResult;
use crate::provider::IDatabaseProvider;
use std::sync::Arc;

#[derive(Clone)]
pub struct DbContextOptions {
    pub(crate) connection_string: String,
    pub(crate) provider_tag: Option<String>,
    #[allow(clippy::type_complexity)]
    pub(crate) provider_factory:
        Option<Arc<dyn Fn(&str) -> EFResult<Arc<dyn IDatabaseProvider>> + Send + Sync>>,
    /// Process-level cache of the built provider (which owns the connection
    /// pool). Built once on the first `create_provider()` call and shared
    /// across every `DbContext` created from the same `Arc<DbContextOptions>`
    /// (i.e. the same `add_dbcontext` registration). Keeping the provider
    /// alive for the application lifetime means the connection pool is reused
    /// across requests instead of being recreated per request.
    pub(crate) provider_cache: Arc<std::sync::Mutex<Option<Arc<dyn IDatabaseProvider>>>>,
    pub(crate) interceptors: Vec<Arc<dyn crate::interceptor::ISaveChangesInterceptor>>,
    /// When `true`, `QueryBuilder::to_list()` attaches `LazyContext` to every
    /// navigation container on materialized entities, enabling on-demand
    /// loading via `BelongsTo::load()` / `HasMany::load()` / `HasOne::load()`.
    ///
    /// Defaults to `false` (opt-in) to preserve v1.0 eager-only behavior.
    pub(crate) lazy_loading_enabled: bool,
    pub(crate) context_key: Option<String>,
    /// Process-level cache of `discover_entities()` output, keyed by
    /// `context_key`. Shared across all `DbContext` instances created from
    /// the same `DbContextOptions` (which is `Arc`-shared per `add_dbcontext`
    /// registration). The first `from_options()` call builds the metadata;
    /// subsequent calls `Arc::clone` it.
    pub(crate) metadata_cache: Arc<crate::metadata_cache::MetadataCache>,
    /// Slow query threshold for tracing. When set, queries exceeding this
    /// duration emit a `tracing::warn!` event.
    #[cfg(feature = "tracing")]
    pub(crate) slow_query_threshold: Option<std::time::Duration>,
}

impl std::fmt::Debug for DbContextOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbContextOptions")
            .field(
                "connection_string",
                &redact_connection_string(&self.connection_string),
            )
            .field("provider_tag", &self.provider_tag)
            .finish()
    }
}

/// Redacts credentials from a connection string so `Debug` output never leaks
/// passwords. Handles URL form (`scheme://user:pass@host`) and key=value form
/// (`...;Password=...;...`). SQLite file paths and other credential-free
/// strings are returned unchanged for debuggability.
fn redact_connection_string(cs: &str) -> String {
    // URL form: scheme://[user[:pass]@]host...
    if let Some(scheme_end) = cs.find("://") {
        let (scheme, rest) = cs.split_at(scheme_end + 3);
        if let Some(at) = rest.find('@') {
            let (userinfo, host_and_rest) = rest.split_at(at);
            let redacted_user = match userinfo.find(':') {
                Some(colon) => &userinfo[..colon],
                None => userinfo,
            };
            return format!("{}{}***@{}", scheme, redacted_user, &host_and_rest[1..]);
        }
        return cs.to_string();
    }
    // Key=value form: redact any token whose key mentions password/pwd.
    if cs.contains('=') {
        return cs
            .split(';')
            .map(|pair| {
                let eq = match pair.find('=') {
                    Some(e) => e,
                    None => return pair.to_string(),
                };
                let key = pair[..eq].trim().to_lowercase();
                if key.contains("password") || key.contains("pwd") {
                    format!("{}=***", &pair[..eq])
                } else {
                    pair.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(";");
    }
    cs.to_string()
}

impl DbContextOptions {
    pub fn connection_string(&self) -> &str {
        &self.connection_string
    }
    pub fn provider_tag(&self) -> Option<&str> {
        self.provider_tag.as_deref()
    }
    pub fn lazy_loading_enabled(&self) -> bool {
        self.lazy_loading_enabled
    }
    pub fn context_key(&self) -> Option<&str> {
        self.context_key.as_deref()
    }
    pub fn create_provider(&self) -> EFResult<Arc<dyn IDatabaseProvider>> {
        // Recover from a poisoned lock rather than panicking (consistent with
        // `MetadataCache`): if a previous build panicked, `into_inner()` yields
        // the still-`None` cache and we retry the build below.
        let mut guard = self
            .provider_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(provider) = guard.as_ref() {
            return Ok(Arc::clone(provider));
        }
        let factory = self.provider_factory.as_ref().ok_or_else(|| {
            crate::error::EFError::configuration(
                "No provider configured. Call use_sqlite / use_postgres / use_mysql first.",
            )
        })?;
        let provider = factory(self.connection_string())?;
        #[cfg(feature = "tracing")]
        if let Some(threshold) = self.slow_query_threshold {
            provider.set_slow_query_threshold(threshold);
        }
        *guard = Some(Arc::clone(&provider));
        Ok(provider)
    }
}

#[allow(clippy::derivable_impls)]
impl Default for DbContextOptions {
    fn default() -> Self {
        Self {
            connection_string: String::new(),
            provider_tag: None,
            provider_factory: None,
            provider_cache: Arc::new(std::sync::Mutex::new(None)),
            interceptors: Vec::new(),
            lazy_loading_enabled: false,
            context_key: None,
            metadata_cache: Arc::new(crate::metadata_cache::MetadataCache::new()),
            #[cfg(feature = "tracing")]
            slow_query_threshold: None,
        }
    }
}

pub struct DbContextOptionsBuilder {
    inner: DbContextOptions,
}

impl DbContextOptionsBuilder {
    pub fn new() -> Self {
        Self {
            inner: DbContextOptions::default(),
        }
    }
    pub fn connection_string(&mut self, cs: impl Into<String>) -> &mut Self {
        self.inner.connection_string = cs.into();
        self
    }
    pub fn set_provider(&mut self, tag: &str, cs: impl Into<String>) -> &mut Self {
        self.inner.provider_tag = Some(tag.to_string());
        self.inner.connection_string = cs.into();
        self
    }
    #[allow(clippy::type_complexity)]
    pub fn set_provider_factory(
        &mut self,
        tag: &str,
        cs: impl Into<String>,
        factory: Arc<dyn Fn(&str) -> EFResult<Arc<dyn IDatabaseProvider>> + Send + Sync>,
    ) -> &mut Self {
        self.inner.provider_tag = Some(tag.to_string());
        self.inner.connection_string = cs.into();
        self.inner.provider_factory = Some(factory);
        self
    }
    /// Registers a `SaveChanges` interceptor.
    ///
    /// Interceptors are called in registration order during
    /// `save_changes()`. Use this for auditing, soft-delete,
    /// validation, and other cross-cutting concerns.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// options
    ///     .use_sqlite("app.db")
    ///     .add_interceptor(AuditInterceptor::new());
    /// ```
    pub fn add_interceptor(
        &mut self,
        interceptor: impl crate::interceptor::ISaveChangesInterceptor + 'static,
    ) -> &mut Self {
        self.inner.interceptors.push(Arc::new(interceptor));
        self
    }

    /// Enables or disables lazy loading of navigation properties.
    ///
    /// When enabled (`true`), `to_list()` attaches a `LazyContext` to every
    /// navigation container on each materialized entity. The user can then
    /// call `nav.load().await` to trigger a single-entity query on first
    /// access; subsequent accesses read from the in-memory cache.
    ///
    /// When disabled (`false`, the default), navigation properties are
    /// empty unless explicitly loaded via `Include` — matching v1.0
    /// eager-only behavior.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut options = DbContextOptionsBuilder::new();
    /// options.use_sqlite_in_memory().use_lazy_loading(true);
    /// ```
    pub fn use_lazy_loading(&mut self, enabled: bool) -> &mut Self {
        self.inner.lazy_loading_enabled = enabled;
        self
    }

    /// Sets the context key used to filter entities and configurations
    /// during `DbContext::discover_entities()`. Set automatically by
    /// `add_dbcontext_keyed`; `None` (the default) selects the default
    /// context.
    pub fn context_key(&mut self, key: impl Into<String>) -> &mut Self {
        self.inner.context_key = Some(key.into());
        self
    }

    /// Sets the slow query threshold. Queries exceeding this duration
    /// emit a `tracing::warn!` event with SQL and elapsed time.
    ///
    /// Only available when the `tracing` feature is enabled.
    #[cfg(feature = "tracing")]
    pub fn slow_query_threshold(&mut self, threshold: std::time::Duration) -> &mut Self {
        self.inner.slow_query_threshold = Some(threshold);
        self
    }

    pub fn build(self) -> DbContextOptions {
        self.inner
    }
}

impl Default for DbContextOptionsBuilder {
    fn default() -> Self {
        Self::new()
    }
}
