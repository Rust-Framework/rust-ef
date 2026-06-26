use async_trait::async_trait;
use deadpool_postgres::{Config, Pool, Runtime};
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{IDatabaseProvider, ISqlGenerator};
use crate::sql_generator::PostgresSqlGenerator;
use tokio_postgres::NoTls;

pub struct PostgresProvider {
    pool: Pool,
}

impl PostgresProvider {
    pub fn new(connection_string: &str, _pool_size: usize) -> EFResult<Self> {
        let config: tokio_postgres::Config = connection_string
            .parse()
            .map_err(|e| EFError::Connection(format!("Invalid connection string: {}", e)))?;
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
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| EFError::Connection(format!("Failed to create pool: {}", e)))?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl IDatabaseProvider for PostgresProvider {
    fn sql_generator(&self) -> Box<dyn ISqlGenerator> {
        Box::new(PostgresSqlGenerator::new())
    }

    async fn get_connection(&self) -> EFResult<Box<dyn rust_ef::provider::IAsyncConnection>> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| EFError::Connection(format!("Pool error: {}", e)))?;
        Ok(Box::new(crate::connection::PostgresConnection { client }))
    }

    async fn execute_migration_command(&self, sql: &str) -> EFResult<()> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| EFError::Connection(format!("Pool error: {}", e)))?;
        client
            .batch_execute(sql)
            .await
            .map_err(|e| EFError::Migration(format!("Migration execution failed: {}", e)))?;
        Ok(())
    }

    fn name(&self) -> &str {
        "PostgreSQL"
    }

    fn migration_dialect(&self) -> rust_ef::migration::MigrationDialect {
        rust_ef::migration::MigrationDialect::Postgres
    }
}

impl std::fmt::Debug for PostgresProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresProvider")
            .field("name", &self.name())
            .finish()
    }
}
