use crate::sql_generator::MySqlSqlGenerator;
use crate::tls::MySqlTlsMode;
use async_trait::async_trait;
use rust_ef::error::{EFError, EFResult};
#[cfg(feature = "tracing")]
use rust_ef::provider::IAsyncConnection;
use rust_ef::provider::{IDatabaseProvider, ISqlGenerator};

/// Options for a MySQL provider.
///
/// Configured through the `use_mysql_with(connection_string, |options| { ... })`
/// delegate; additional MySQL settings can be added here without new
/// `use_mysql_*` methods.
#[derive(Clone, Debug)]
pub struct MySqlOptions {
    pub(crate) connection_string: String,
    pub(crate) max_pool_size: Option<u32>,
    pub(crate) tls: MySqlTlsMode,
}

impl MySqlOptions {
    pub(crate) fn new(connection_string: impl Into<String>) -> Self {
        Self {
            connection_string: connection_string.into(),
            max_pool_size: None,
            tls: MySqlTlsMode::Required,
        }
    }

    /// Sets the connection pool's maximum size.
    pub fn max_pool_size(&mut self, max_pool_size: u32) -> &mut Self {
        self.max_pool_size = Some(max_pool_size);
        self
    }

    /// Overrides the TLS mode.
    pub fn tls(&mut self, tls: MySqlTlsMode) -> &mut Self {
        self.tls = tls;
        self
    }

    /// Disables TLS (local development only).
    pub fn insecure(&mut self) -> &mut Self {
        self.tls(MySqlTlsMode::Disabled)
    }
}

pub struct MySqlProvider {
    pool: sqlx::MySqlPool,
    #[cfg(feature = "tracing")]
    slow_query_threshold_ms: std::sync::atomic::AtomicU64,
}

impl MySqlProvider {
    /// Creates a provider with TLS required (secure-by-default, v1.6+).
    ///
    /// Overrides any `ssl-mode` in the connection string. For plaintext
    /// connections (local dev only), use [`MySqlProvider::new_insecure`].
    pub async fn new(connection_string: &str) -> EFResult<Self> {
        Self::new_with_tls(connection_string, MySqlTlsMode::Required).await
    }

    /// Creates a provider with TLS disabled (plaintext, local dev only).
    pub async fn new_insecure(connection_string: &str) -> EFResult<Self> {
        Self::new_with_tls(connection_string, MySqlTlsMode::Disabled).await
    }

    pub fn from_pool(pool: sqlx::MySqlPool) -> Self {
        Self {
            pool,
            #[cfg(feature = "tracing")]
            slow_query_threshold_ms: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Creates a lazy provider with TLS required (secure-by-default, v1.6+).
    pub fn new_lazy(connection_string: &str) -> EFResult<Self> {
        Self::new_lazy_with_tls(connection_string, MySqlTlsMode::Required)
    }

    /// Creates a lazy provider with TLS disabled (plaintext, local dev only).
    pub fn new_lazy_insecure(connection_string: &str) -> EFResult<Self> {
        Self::new_lazy_with_tls(connection_string, MySqlTlsMode::Disabled)
    }

    /// Creates a provider with explicit TLS configuration.
    ///
    /// The TLS mode overrides any `ssl-mode` in the connection string.
    /// For CA certificate verification (`VerifyCa` / `VerifyIdentity`),
    /// provide the CA certificate via the connection string's `ssl-ca`
    /// parameter.
    ///
    /// For the secure-by-default convenience constructor, use
    /// [`MySqlProvider::new`]. For plaintext (local dev), use
    /// [`MySqlProvider::new_insecure`].
    pub async fn new_with_tls(connection_string: &str, tls: MySqlTlsMode) -> EFResult<Self> {
        let mut options: sqlx::mysql::MySqlConnectOptions = connection_string
            .parse()
            .map_err(|e| EFError::connection(format!("MySQL URL parse: {}", e)))?;
        options = options.ssl_mode(tls.into());
        let pool = sqlx::MySqlPool::connect_with(options)
            .await
            .map_err(|e| EFError::connection(format!("MySQL connection failed: {}", e)))?;
        Ok(Self {
            pool,
            #[cfg(feature = "tracing")]
            slow_query_threshold_ms: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// Creates a lazy provider (no immediate connection) with explicit TLS.
    ///
    /// See [`MySqlProvider::new_with_tls`] for TLS mode details.
    pub fn new_lazy_with_tls(connection_string: &str, tls: MySqlTlsMode) -> EFResult<Self> {
        let mut options: sqlx::mysql::MySqlConnectOptions = connection_string
            .parse()
            .map_err(|e| EFError::connection(format!("MySQL URL parse: {}", e)))?;
        options = options.ssl_mode(tls.into());
        let pool = sqlx::MySqlPool::connect_lazy_with(options);
        Ok(Self {
            pool,
            #[cfg(feature = "tracing")]
            slow_query_threshold_ms: std::sync::atomic::AtomicU64::new(0),
        })
    }
}

#[async_trait]
impl IDatabaseProvider for MySqlProvider {
    fn sql_generator(&self) -> &'static dyn ISqlGenerator {
        &MySqlSqlGenerator
    }

    async fn get_connection(&self) -> EFResult<Box<dyn rust_ef::provider::IAsyncConnection>> {
        let _guard = rust_ef::observability::PoolAcquireGuard::new("MySQL");
        let conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| EFError::connection(format!("Pool acquire failed: {}", e)))?;
        #[cfg_attr(not(feature = "tracing"), allow(unused_mut))]
        let mut conn = Box::new(crate::connection::MySqlConnection::new(conn));
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
        // DDL is not supported by the prepared-statement protocol (MySQL error
        // 1295); use the text protocol.
        sqlx::raw_sql(sql)
            .execute(&self.pool)
            .await
            .map_err(|e| EFError::migration(format!("Migration execution failed: {}", e)))?;
        Ok(())
    }

    fn name(&self) -> &str {
        "MySQL"
    }

    fn migration_dialect(&self) -> rust_ef::migration::MigrationDialect {
        rust_ef::migration::MigrationDialect::MySql
    }

    #[cfg(feature = "tracing")]
    fn set_slow_query_threshold(&self, threshold: std::time::Duration) {
        self.slow_query_threshold_ms.store(
            threshold.as_millis() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

impl std::fmt::Debug for MySqlProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MySqlProvider")
            .field("name", &self.name())
            .finish()
    }
}
