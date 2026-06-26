use crate::sql_generator::SqliteSqlGenerator;
use async_trait::async_trait;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{IDatabaseProvider, ISqlGenerator};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SqliteProvider {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl SqliteProvider {
    pub fn new(path: impl AsRef<Path>) -> EFResult<Self> {
        let conn = rusqlite::Connection::open(path)
            .map_err(|e| EFError::Connection(format!("SQLite open failed: {}", e)))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| EFError::Connection(format!("SQLite WAL setup failed: {}", e)))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn new_in_memory() -> EFResult<Self> {
        Self::new(":memory:")
    }
}

#[async_trait]
impl IDatabaseProvider for SqliteProvider {
    fn sql_generator(&self) -> Box<dyn ISqlGenerator> {
        Box::new(SqliteSqlGenerator::new())
    }

    async fn get_connection(&self) -> EFResult<Box<dyn rust_ef::provider::IAsyncConnection>> {
        Ok(Box::new(crate::connection::SqliteConnection::new(
            self.conn.clone(),
        )))
    }

    async fn execute_migration_command(&self, sql: &str) -> EFResult<()> {
        let conn = self.conn.lock().await;
        conn.execute_batch(sql)
            .map_err(|e| EFError::Migration(format!("Migration execution failed: {}", e)))?;
        Ok(())
    }

    fn name(&self) -> &str {
        "SQLite"
    }

    fn migration_dialect(&self) -> rust_ef::migration::MigrationDialect {
        rust_ef::migration::MigrationDialect::Sqlite
    }
}

impl std::fmt::Debug for SqliteProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteProvider")
            .field("name", &self.name())
            .finish()
    }
}
