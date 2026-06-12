//! PostgreSQL provider for Rust Entity Framework.

pub mod introspection;
pub mod sql_generator;
pub mod type_mapping;

use async_trait::async_trait;
use deadpool_postgres::{Config, Pool, Runtime};
use lref::error::{LrefError, LrefResult};
use lref::provider::{DbValue, IAsyncConnection, IDatabaseProvider, ISqlGenerator};
pub use sql_generator::PostgresSqlGenerator;
use tokio_postgres::{types::ToSql, NoTls};
pub use type_mapping::PostgresTypeMapping;

pub struct PostgresProvider {
    pool: Pool,
}

impl PostgresProvider {
    pub fn new(connection_string: &str, _pool_size: usize) -> LrefResult<Self> {
        let config: tokio_postgres::Config = connection_string
            .parse()
            .map_err(|e| LrefError::Connection(format!("Invalid connection string: {}", e)))?;

        let mut cfg = Config::new();
        if let Some(host) = config.get_hosts().first() {
            cfg.host = Some(format!("{:?}", host));
        }
        cfg.port = Some(config.get_ports().first().copied().unwrap_or(5432));
        cfg.dbname = Some(config.get_dbname().unwrap_or("postgres").to_string());
        cfg.user = Some(config.get_user().unwrap_or("postgres").to_string());
        cfg.password = config
            .get_password()
            .map(|p| String::from_utf8_lossy(p).to_string());

        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| LrefError::Connection(format!("Failed to create pool: {}", e)))?;

        Ok(Self { pool })
    }
}

#[async_trait]
impl IDatabaseProvider for PostgresProvider {
    fn sql_generator(&self) -> Box<dyn ISqlGenerator> {
        Box::new(PostgresSqlGenerator::new())
    }

    async fn get_connection(&self) -> LrefResult<Box<dyn IAsyncConnection>> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| LrefError::Connection(format!("Pool error: {}", e)))?;

        Ok(Box::new(PostgresConnection { client }))
    }

    async fn execute_migration_command(&self, sql: &str) -> LrefResult<()> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| LrefError::Connection(format!("Pool error: {}", e)))?;

        client
            .batch_execute(sql)
            .await
            .map_err(|e| LrefError::Migration(format!("Migration execution failed: {}", e)))?;

        Ok(())
    }

    fn name(&self) -> &str {
        "PostgreSQL"
    }
}

struct PostgresConnection {
    client: deadpool_postgres::Client,
}

#[async_trait]
impl IAsyncConnection for PostgresConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<u64> {
        let pg_params: Vec<Box<dyn ToSql + Sync + Send>> = db_values_to_pg_params(params);
        let refs: Vec<&(dyn ToSql + Sync)> = pg_params
            .iter()
            .map(|p| p.as_ref() as &(dyn ToSql + Sync))
            .collect();
        self.client
            .execute(sql, &refs)
            .await
            .map_err(|e| LrefError::Query(format!("Execution error: {}", e)))
    }

    async fn query(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<Vec<Vec<String>>> {
        let pg_params: Vec<Box<dyn ToSql + Sync + Send>> = db_values_to_pg_params(params);
        let refs: Vec<&(dyn ToSql + Sync)> = pg_params
            .iter()
            .map(|p| p.as_ref() as &(dyn ToSql + Sync))
            .collect();
        let rows = self
            .client
            .query(sql, &refs)
            .await
            .map_err(|e| LrefError::Query(format!("Query error: {}", e)))?;

        let columns: Vec<String> = if !rows.is_empty() {
            rows[0]
                .columns()
                .iter()
                .map(|c| c.name().to_string())
                .collect()
        } else {
            Vec::new()
        };

        let result = rows
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        row.try_get::<_, String>(i)
                            .unwrap_or_else(|_| "NULL".to_string())
                    })
                    .collect()
            })
            .collect();

        Ok(result)
    }

    async fn begin_transaction(&mut self) -> LrefResult<()> {
        self.client
            .simple_query("BEGIN")
            .await
            .map_err(|e| LrefError::Transaction(format!("BEGIN failed: {}", e)))?;
        Ok(())
    }

    async fn commit_transaction(&mut self) -> LrefResult<()> {
        self.client
            .simple_query("COMMIT")
            .await
            .map_err(|e| LrefError::Transaction(format!("COMMIT failed: {}", e)))?;
        Ok(())
    }

    async fn rollback_transaction(&mut self) -> LrefResult<()> {
        self.client
            .simple_query("ROLLBACK")
            .await
            .map_err(|e| LrefError::Transaction(format!("ROLLBACK failed: {}", e)))?;
        Ok(())
    }
}

fn db_values_to_pg_params(params: &[DbValue]) -> Vec<Box<dyn ToSql + Sync + Send>> {
    params
        .iter()
        .map(|v| match v {
            DbValue::Null => Box::new(None::<String>) as Box<dyn ToSql + Sync + Send>,
            DbValue::Bool(b) => Box::new(*b),
            DbValue::I16(n) => Box::new(*n),
            DbValue::I32(n) => Box::new(*n),
            DbValue::I64(n) => Box::new(*n),
            DbValue::F32(n) => Box::new(*n),
            DbValue::F64(n) => Box::new(*n),
            DbValue::String(s) => Box::new(s.clone()),
            DbValue::Bytes(b) => Box::new(b.clone()),
        })
        .collect()
}

impl std::fmt::Debug for PostgresProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresProvider")
            .field("name", &self.name())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// DbContextOptionsBuilder extension — EFCore-style .UsePostgres()
// ---------------------------------------------------------------------------

/// Extension trait that adds `.use_postgres()` to `DbContextOptionsBuilder`.
///
/// # Example
///
/// ```rust,ignore
/// use lrdi::ServiceCollection;
/// use lref::di::DbContextServiceCollectionExt;
/// use lref_provider_postgres::DbContextOptionsBuilderExt as _;
///
/// let provider = ServiceCollection::new()
///     .add_dbcontext::<MyContext>(|options| {
///         options.use_postgres("host=localhost dbname=myapp");
///     })
///     .build()
///     .unwrap();
/// ```
pub trait DbContextOptionsBuilderExt {
    /// Configures the context to use PostgreSQL.
    fn use_postgres(&mut self, connection_string: &str) -> &mut Self;
}

impl DbContextOptionsBuilderExt for lref::db_context::DbContextOptionsBuilder {
    fn use_postgres(&mut self, connection_string: &str) -> &mut Self {
        self.set_provider("postgres", connection_string)
    }
}
