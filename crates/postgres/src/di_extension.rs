use rust_ef::provider::IDatabaseProvider;
use std::sync::Arc;

pub trait DbContextOptionsBuilderExt {
    /// Registers a PostgreSQL provider with a default pool size of 5.
    fn use_postgres(&mut self, connection_string: &str) -> &mut Self;

    /// Registers a PostgreSQL provider with a configurable pool size.
    /// `pool_size = 0` uses the deadpool default (CPU count).
    fn use_postgres_with_pool(&mut self, connection_string: &str, pool_size: usize) -> &mut Self;
}

impl DbContextOptionsBuilderExt for rust_ef::db_context::DbContextOptionsBuilder {
    fn use_postgres(&mut self, connection_string: &str) -> &mut Self {
        self.use_postgres_with_pool(connection_string, 5)
    }

    fn use_postgres_with_pool(&mut self, connection_string: &str, pool_size: usize) -> &mut Self {
        let cs = connection_string.to_string();
        self.set_provider_factory(
            "postgres",
            &cs,
            Arc::new(move |cs: &str| {
                Ok(
                    Arc::new(crate::provider::PostgresProvider::new(cs, pool_size)?)
                        as Arc<dyn IDatabaseProvider>,
                )
            }),
        )
    }
}
