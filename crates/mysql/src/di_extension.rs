use crate::provider::MySqlTlsMode;
use rust_ef::provider::IDatabaseProvider;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use std::sync::Arc;

pub trait DbContextOptionsBuilderExt {
    /// Registers a MySQL provider with default pool options.
    fn use_mysql(&mut self, connection_string: &str) -> &mut Self;

    /// Registers a MySQL provider with a configurable `PoolOptions` callback.
    ///
    /// Example:
    /// ```ignore
    /// options.use_mysql_with_pool(cs, Arc::new(|o| { o.max_connections(20); }));
    /// ```
    fn use_mysql_with_pool(
        &mut self,
        connection_string: &str,
        configure: Arc<dyn Fn(&mut MySqlPoolOptions) + Send + Sync>,
    ) -> &mut Self;

    /// Registers a MySQL provider with explicit TLS configuration.
    ///
    /// The TLS mode overrides any `ssl-mode` in the connection string.
    /// For CA certificate verification, provide the CA via the connection
    /// string's `ssl-ca` parameter.
    ///
    /// # Example
    ///
    /// ```ignore
    /// options.use_mysql_with_tls(
    ///     "mysql://user:pass@host/db",
    ///     MySqlTlsMode::Required,
    /// );
    /// ```
    fn use_mysql_with_tls(&mut self, connection_string: &str, tls: MySqlTlsMode) -> &mut Self;
}

impl DbContextOptionsBuilderExt for rust_ef::db_context::DbContextOptionsBuilder {
    fn use_mysql(&mut self, connection_string: &str) -> &mut Self {
        let cs = connection_string.to_string();
        self.set_provider_factory(
            "mysql",
            &cs,
            Arc::new(move |cs: &str| {
                Ok(Arc::new(crate::provider::MySqlProvider::new_lazy(cs)?)
                    as Arc<dyn IDatabaseProvider>)
            }),
        )
    }

    fn use_mysql_with_pool(
        &mut self,
        connection_string: &str,
        configure: Arc<dyn Fn(&mut MySqlPoolOptions) + Send + Sync>,
    ) -> &mut Self {
        let cs = connection_string.to_string();
        self.set_provider_factory(
            "mysql",
            &cs,
            Arc::new(move |cs: &str| {
                let mut options = MySqlPoolOptions::new();
                configure(&mut options);
                let connect_opts: MySqlConnectOptions = cs.parse().map_err(|e| {
                    rust_ef::error::EFError::Connection(format!("MySQL URL parse: {}", e))
                })?;
                let pool = options.connect_lazy_with(connect_opts);
                Ok(Arc::new(crate::provider::MySqlProvider::from_pool(pool))
                    as Arc<dyn IDatabaseProvider>)
            }),
        )
    }

    fn use_mysql_with_tls(&mut self, connection_string: &str, tls: MySqlTlsMode) -> &mut Self {
        let cs = connection_string.to_string();
        self.set_provider_factory(
            "mysql",
            &cs,
            Arc::new(move |cs: &str| {
                Ok(
                    Arc::new(crate::provider::MySqlProvider::new_lazy_with_tls(cs, tls)?)
                        as Arc<dyn IDatabaseProvider>,
                )
            }),
        )
    }
}
