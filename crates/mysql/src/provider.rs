use async_trait::async_trait;
use rust_ef::error::{EfError, EfResult};
use rust_ef::provider::{IDatabaseProvider, ISqlGenerator};
use crate::sql_generator::MySqlSqlGenerator;

pub struct MySqlProvider {
    pool: sqlx::MySqlPool,
}

impl MySqlProvider {
    pub async fn new(connection_string: &str) -> EfResult<Self> {
        let pool = sqlx::MySqlPool::connect(connection_string)
            .await
            .map_err(|e| EfError::Connection(format!("MySQL connection failed: {}", e)))?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: sqlx::MySqlPool) -> Self {
        Self { pool }
    }

    pub fn new_lazy(connection_string: &str) -> EfResult<Self> {
        let pool = sqlx::MySqlPool::connect_lazy(connection_string)
            .map_err(|e| EfError::Connection(format!("MySQL pool failed: {}", e)))?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl IDatabaseProvider for MySqlProvider {
    fn sql_generator(&self) -> Box<dyn ISqlGenerator> {
        Box::new(MySqlSqlGenerator::new())
    }

    async fn get_connection(&self) -> EfResult<Box<dyn rust_ef::provider::IAsyncConnection>> {
        let conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| EfError::Connection(format!("Pool acquire failed: {}", e)))?;
        Ok(Box::new(crate::connection::MySqlConnection::new(conn)))
    }

    async fn execute_migration_command(&self, sql: &str) -> EfResult<()> {
        sqlx::query(sql)
            .execute(&self.pool)
            .await
            .map_err(|e| EfError::Migration(format!("Migration execution failed: {}", e)))?;
        Ok(())
    }

    fn name(&self) -> &str {
        "MySQL"
    }

    fn migration_dialect(&self) -> rust_ef::migration::MigrationDialect {
        rust_ef::migration::MigrationDialect::MySql
    }
}

impl std::fmt::Debug for MySqlProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MySqlProvider")
            .field("name", &self.name())
            .finish()
    }
}
