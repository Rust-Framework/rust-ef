use crate::sql_generator::MySqlSqlGenerator;
use crate::tls::MySqlTlsMode;
use async_trait::async_trait;
use rust_ef::error::{EFError, EFResult};
#[cfg(feature = "tracing")]
use rust_ef::provider::IAsyncConnection;
use rust_ef::provider::{IDatabaseProvider, ISqlGenerator};

pub struct MySqlProvider {
    pool: sqlx::MySqlPool,
    #[cfg(feature = "tracing")]
    slow_query_threshold_ms: std::sync::atomic::AtomicU64,
}

impl MySqlProvider {
    pub async fn new(connection_string: &str) -> EFResult<Self> {
        let pool = sqlx::MySqlPool::connect(connection_string)
            .await
            .map_err(|e| EFError::Connection(format!("MySQL connection failed: {}", e)))?;
        Ok(Self {
            pool,
            #[cfg(feature = "tracing")]
            slow_query_threshold_ms: std::sync::atomic::AtomicU64::new(0),
        })
    }

    pub fn from_pool(pool: sqlx::MySqlPool) -> Self {
        Self {
            pool,
            #[cfg(feature = "tracing")]
            slow_query_threshold_ms: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn new_lazy(connection_string: &str) -> EFResult<Self> {
        let pool = sqlx::MySqlPool::connect_lazy(connection_string)
            .map_err(|e| EFError::Connection(format!("MySQL pool failed: {}", e)))?;
        Ok(Self {
            pool,
            #[cfg(feature = "tracing")]
            slow_query_threshold_ms: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// Creates a provider with explicit TLS configuration.
    ///
    /// The TLS mode overrides any `ssl-mode` in the connection string.
    /// For CA certificate verification (`VerifyCa` / `VerifyIdentity`),
    /// provide the CA certificate via the connection string's `ssl-ca`
    /// parameter.
    ///
    /// For backward-compatible behavior (respecting the connection string's
    /// `ssl-mode`), use [`MySqlProvider::new`] instead.
    pub async fn new_with_tls(connection_string: &str, tls: MySqlTlsMode) -> EFResult<Self> {
        let mut options: sqlx::mysql::MySqlConnectOptions = connection_string
            .parse()
            .map_err(|e| EFError::Connection(format!("MySQL URL parse: {}", e)))?;
        options = options.ssl_mode(tls.into());
        let pool = sqlx::MySqlPool::connect_with(options)
            .await
            .map_err(|e| EFError::Connection(format!("MySQL connection failed: {}", e)))?;
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
            .map_err(|e| EFError::Connection(format!("MySQL URL parse: {}", e)))?;
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
            .map_err(|e| EFError::Connection(format!("Pool acquire failed: {}", e)))?;
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
        sqlx::query(sql)
            .execute(&self.pool)
            .await
            .map_err(|e| EFError::Migration(format!("Migration execution failed: {}", e)))?;
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
