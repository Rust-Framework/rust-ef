use crate::sql_generator::PostgresSqlGenerator;
use crate::tls::{pg_tls_preference, without_ssl_mode_parameter, PgTlsMode};
use async_trait::async_trait;
use deadpool_postgres::{Config, Pool, Runtime};
use rust_ef::error::{EFError, EFResult};
#[cfg(feature = "tracing")]
use rust_ef::provider::IAsyncConnection;
use rust_ef::provider::{IDatabaseProvider, ISqlGenerator};
use tokio_postgres::NoTls;

/// Options for a PostgreSQL provider.
///
/// Configured through the `use_postgres_with(connection_string, |options| { ... })`
/// delegate; additional PostgreSQL settings can be added here without new
/// `use_postgres_*` methods.
#[derive(Clone)]
pub struct PostgresOptions {
    pub(crate) connection_string: String,
    pub(crate) pool_size: Option<usize>,
    pub(crate) tls: Option<PgTlsMode>,
}

impl PostgresOptions {
    pub(crate) fn new(connection_string: impl Into<String>) -> Self {
        Self {
            connection_string: connection_string.into(),
            pool_size: None,
            tls: None,
        }
    }

    /// Sets the connection pool size. `0` uses the provider default.
    pub fn pool_size(&mut self, pool_size: usize) -> &mut Self {
        self.pool_size = Some(pool_size);
        self
    }

    /// Overrides the TLS mode.
    pub fn tls(&mut self, tls: PgTlsMode) -> &mut Self {
        self.tls = Some(tls);
        self
    }

    /// Disables TLS (local development only).
    pub fn insecure(&mut self) -> &mut Self {
        self.tls(PgTlsMode::Disable)
    }
}

pub struct PostgresProvider {
    pool: Pool,
    #[cfg(feature = "tracing")]
    slow_query_threshold_ms: std::sync::atomic::AtomicU64,
}

impl PostgresProvider {
    /// Creates a provider with TLS required unless the connection string opts
    /// out explicitly (secure by default, consistent with MySQL).
    ///
    /// `sslmode=disable` (or `allow`) in the URL selects plaintext — the same
    /// explicit switch MySQL has — so a local/test database no longer needs a
    /// separate `insecure()` call. An absent `sslmode`, or `prefer`/`require`/
    /// `verify-*`, enforces TLS (`native_tls` with the platform trust store).
    /// Use [`PostgresProvider::new_insecure`] for an explicit plaintext
    /// override and [`PostgresProvider::new_with_tls`] for a custom connector.
    pub fn new(connection_string: &str, pool_size: usize) -> EFResult<Self> {
        if pg_tls_preference(connection_string).is_encrypted() {
            let connector = native_tls::TlsConnector::builder()
                .build()
                .map_err(|e| EFError::connection(format!("TLS connector init failed: {}", e)))?;
            Self::new_with_tls(connection_string, pool_size, PgTlsMode::Require(connector))
        } else {
            Self::new_with_tls(connection_string, pool_size, PgTlsMode::Disable)
        }
    }

    /// Creates a provider with TLS disabled (plaintext, local dev only).
    pub fn new_insecure(connection_string: &str, pool_size: usize) -> EFResult<Self> {
        Self::new_with_tls(connection_string, pool_size, PgTlsMode::Disable)
    }

    /// Creates a provider with configurable TLS.
    ///
    /// `PgTlsMode::Disable` is equivalent to [`PostgresProvider::new_insecure`].
    /// `PgTlsMode::Require(connector)` enforces TLS for all pooled connections.
    ///
    /// The TLS connector type is erased inside `deadpool_postgres::Manager`
    /// (via `Box<dyn Connect>`), so `Pool` and `PostgresConnection` are
    /// non-generic — TLS configuration is a per-pool construction-time
    /// decision, not a type-level parameter.
    pub fn new_with_tls(
        connection_string: &str,
        pool_size: usize,
        tls: PgTlsMode,
    ) -> EFResult<Self> {
        // `tokio_postgres::Config` rejects `sslmode=verify-ca/verify-full`;
        // TLS is decided by `pg_tls_preference` and the `PgTlsMode`, so the
        // parameter is removed before parsing (EF-PG-02).
        let driver_url = without_ssl_mode_parameter(connection_string);
        let config: tokio_postgres::Config = driver_url
            .parse()
            .map_err(|e| EFError::connection(format!("Invalid connection string: {}", e)))?;
        let mut cfg = Config::new();
        if let Some(tokio_postgres::config::Host::Tcp(h)) = config.get_hosts().first() {
            cfg.host = Some(h.clone());
        }
        cfg.port = Some(config.get_ports().first().copied().unwrap_or(5432));
        cfg.dbname = Some(config.get_dbname().unwrap_or("postgres").to_string());
        cfg.user = Some(config.get_user().unwrap_or("postgres").to_string());
        cfg.password = config
            .get_password()
            .map(|p| String::from_utf8_lossy(p).to_string());
        if pool_size > 0 {
            cfg.pool.get_or_insert_default().max_size = pool_size;
        }

        let pool = match tls {
            PgTlsMode::Disable => cfg
                .create_pool(Some(Runtime::Tokio1), NoTls)
                .map_err(|e| EFError::connection(format!("Failed to create pool: {}", e)))?,
            PgTlsMode::Require(connector) => {
                let tls = postgres_native_tls::MakeTlsConnector::new(connector);
                cfg.create_pool(Some(Runtime::Tokio1), tls)
                    .map_err(|e| EFError::connection(format!("Failed to create pool: {}", e)))?
            }
        };
        Ok(Self {
            pool,
            #[cfg(feature = "tracing")]
            slow_query_threshold_ms: std::sync::atomic::AtomicU64::new(0),
        })
    }
}

#[async_trait]
impl IDatabaseProvider for PostgresProvider {
    fn sql_generator(&self) -> &'static dyn ISqlGenerator {
        &PostgresSqlGenerator
    }

    async fn get_connection(&self) -> EFResult<Box<dyn rust_ef::provider::IAsyncConnection>> {
        let _guard = rust_ef::observability::PoolAcquireGuard::new("PostgreSQL");
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| EFError::connection(format!("Pool error: {}", e)))?;
        #[cfg_attr(not(feature = "tracing"), allow(unused_mut))]
        let mut conn = Box::new(crate::connection::PostgresConnection::new(client));
        #[cfg(feature = "tracing")]
        {
            let ms = self
                .slow_query_threshold_ms
                .load(std::sync::atomic::Ordering::Relaxed);
            if ms > 0 {
                conn.set_slow_query_threshold(std::time::Duration::from_millis(ms));
            }
        }
        Ok(conn)
    }

    async fn execute_migration_command(&self, sql: &str) -> EFResult<()> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| EFError::connection(format!("Pool error: {}", e)))?;
        client
            .batch_execute(sql)
            .await
            .map_err(|e| crate::error_map::map_pg_error(e, "Migration execution failed", sql))?;
        Ok(())
    }

    fn name(&self) -> &str {
        "PostgreSQL"
    }

    fn migration_dialect(&self) -> rust_ef::migration::MigrationDialect {
        rust_ef::migration::MigrationDialect::Postgres
    }

    async fn introspect(&self) -> EFResult<Vec<rust_ef::schema::DbTable>> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| EFError::connection(format!("Pool error: {}", e)))?;
        crate::introspection::introspect_postgres_client(&client).await
    }

    #[cfg(feature = "tracing")]
    fn set_slow_query_threshold(&self, threshold: std::time::Duration) {
        self.slow_query_threshold_ms.store(
            threshold.as_millis() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

impl std::fmt::Debug for PostgresProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresProvider")
            .field("name", &self.name())
            .finish()
    }
}
