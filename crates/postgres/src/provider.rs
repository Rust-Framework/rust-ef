use crate::sql_generator::PostgresSqlGenerator;
use crate::tls::PgTlsMode;
use async_trait::async_trait;
use deadpool_postgres::{Config, Pool, Runtime};
use rust_ef::error::{EFError, EFResult};
#[cfg(feature = "tracing")]
use rust_ef::provider::IAsyncConnection;
use rust_ef::provider::{IDatabaseProvider, ISqlGenerator};
use tokio_postgres::NoTls;

pub struct PostgresProvider {
    pool: Pool,
    #[cfg(feature = "tracing")]
    slow_query_threshold_ms: std::sync::atomic::AtomicU64,
}

impl PostgresProvider {
    /// Creates a provider with TLS required (secure-by-default, v1.6+).
    ///
    /// Uses a `native_tls::TlsConnector` built from the platform's default
    /// root certificate store. For plaintext connections (local dev only),
    /// use [`PostgresProvider::new_insecure`]. For custom CA certificates,
    /// use [`PostgresProvider::new_with_tls`].
    pub fn new(connection_string: &str, pool_size: usize) -> EFResult<Self> {
        let connector = native_tls::TlsConnector::builder()
            .build()
            .map_err(|e| EFError::connection(format!("TLS connector init failed: {}", e)))?;
        Self::new_with_tls(connection_string, pool_size, PgTlsMode::Require(connector))
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
    /// (via `Box<dyn Connect>`), so [`Pool`] and [`PostgresConnection`] are
    /// non-generic — TLS configuration is a per-pool construction-time
    /// decision, not a type-level parameter.
    pub fn new_with_tls(
        connection_string: &str,
        pool_size: usize,
        tls: PgTlsMode,
    ) -> EFResult<Self> {
        let config: tokio_postgres::Config = connection_string
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
            .map_err(|e| EFError::migration(format!("Migration execution failed: {}", e)))?;
        Ok(())
    }

    fn name(&self) -> &str {
        "PostgreSQL"
    }

    fn migration_dialect(&self) -> rust_ef::migration::MigrationDialect {
        rust_ef::migration::MigrationDialect::Postgres
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
