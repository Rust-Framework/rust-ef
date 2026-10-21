use crate::provider::PostgresOptions;
use crate::tls::PgTlsMode;
use rust_ef::provider::IDatabaseProvider;
use std::sync::Arc;

pub trait DbContextOptionsBuilderExt {
    /// Registers a PostgreSQL provider with a default pool size of 5.
    fn use_postgres(&mut self, connection_string: &str) -> &mut Self;

    /// Registers a PostgreSQL provider and configures it through a delegate.
    ///
    /// ```rust,ignore
    /// options.use_postgres_with(cs, |options| {
    ///     options.pool_size(20);
    /// });
    /// ```
    fn use_postgres_with(
        &mut self,
        connection_string: &str,
        configure: impl Fn(&mut PostgresOptions) + Send + Sync + 'static,
    ) -> &mut Self;

    /// Registers a PostgreSQL provider with a configurable pool size.
    /// `pool_size = 0` uses the deadpool default (CPU count).
    fn use_postgres_with_pool(&mut self, connection_string: &str, pool_size: usize) -> &mut Self;

    /// Registers a PostgreSQL provider with configurable TLS.
    ///
    /// `PgTlsMode::Disable` is equivalent to [`DbContextOptionsBuilderExt::use_postgres_with_pool`].
    /// `PgTlsMode::Require(connector)` enforces TLS for all pooled connections
    /// using the platform's native TLS implementation.
    fn use_postgres_with_tls(
        &mut self,
        connection_string: &str,
        pool_size: usize,
        tls: PgTlsMode,
    ) -> &mut Self;
}

impl DbContextOptionsBuilderExt for rust_ef::db_context::DbContextOptionsBuilder {
    fn use_postgres(&mut self, connection_string: &str) -> &mut Self {
        self.use_postgres_with(connection_string, |_| {})
    }

    fn use_postgres_with(
        &mut self,
        connection_string: &str,
        configure: impl Fn(&mut PostgresOptions) + Send + Sync + 'static,
    ) -> &mut Self {
        let cs = connection_string.to_string();
        let cs_for_factory = cs.clone();
        let configure: Arc<dyn Fn(&mut PostgresOptions) + Send + Sync> = Arc::new(configure);
        self.set_provider_factory(
            "postgres",
            &cs,
            Arc::new(move |_cs: &str| {
                let mut options = PostgresOptions::new(&cs_for_factory);
                configure(&mut options);
                let pool_size = options.pool_size.unwrap_or(5);
                let provider = match options.tls.clone() {
                    Some(tls) => crate::provider::PostgresProvider::new_with_tls(
                        &options.connection_string,
                        pool_size,
                        tls,
                    )?,
                    None => crate::provider::PostgresProvider::new(
                        &options.connection_string,
                        pool_size,
                    )?,
                };
                Ok(Arc::new(provider) as Arc<dyn IDatabaseProvider>)
            }),
        )
    }

    fn use_postgres_with_pool(&mut self, connection_string: &str, pool_size: usize) -> &mut Self {
        self.use_postgres_with(connection_string, move |options| {
            options.pool_size(pool_size);
        })
    }

    fn use_postgres_with_tls(
        &mut self,
        connection_string: &str,
        pool_size: usize,
        tls: PgTlsMode,
    ) -> &mut Self {
        self.use_postgres_with(connection_string, move |options| {
            options.pool_size(pool_size).tls(tls.clone());
        })
    }
}
