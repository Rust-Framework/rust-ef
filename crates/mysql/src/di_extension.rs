use crate::provider::MySqlOptions;
use crate::tls::MySqlTlsMode;
use rust_ef::provider::IDatabaseProvider;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use std::sync::Arc;

pub trait DbContextOptionsBuilderExt {
    /// Registers a MySQL provider with default pool options.
    fn use_mysql(&mut self, connection_string: &str) -> &mut Self;

    /// Registers a MySQL provider and configures it through a delegate.
    ///
    /// ```rust,ignore
    /// options.use_mysql_with(cs, |options| {
    ///     options.max_pool_size(20);
    /// });
    /// ```
    fn use_mysql_with(
        &mut self,
        connection_string: &str,
        configure: impl Fn(&mut MySqlOptions) + Send + Sync + 'static,
    ) -> &mut Self;

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
        self.use_mysql_with(connection_string, |_| {})
    }

    fn use_mysql_with(
        &mut self,
        connection_string: &str,
        configure: impl Fn(&mut MySqlOptions) + Send + Sync + 'static,
    ) -> &mut Self {
        let cs = connection_string.to_string();
        let cs_for_factory = cs.clone();
        let configure: Arc<dyn Fn(&mut MySqlOptions) + Send + Sync> = Arc::new(configure);
        self.set_provider_factory(
            "mysql",
            &cs,
            Arc::new(move |_cs: &str| {
                let mut options = MySqlOptions::new(&cs_for_factory);
                configure(&mut options);
                let connect_opts: MySqlConnectOptions =
                    options.connection_string.parse().map_err(|e| {
                        rust_ef::error::EFError::connection(format!("MySQL URL parse: {}", e))
                    })?;
                let connect_opts = connect_opts.ssl_mode(options.tls.into());
                let mut pool_options = MySqlPoolOptions::new();
                if let Some(max) = options.max_pool_size {
                    if max == 0 {
                        return Err(rust_ef::error::EFError::configuration(
                            "MySQL pool max size must be greater than 0",
                        ));
                    }
                    pool_options = pool_options.max_connections(max);
                }
                let pool = pool_options.connect_lazy_with(connect_opts);
                Ok(Arc::new(crate::provider::MySqlProvider::from_pool(pool))
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
                    rust_ef::error::EFError::connection(format!("MySQL URL parse: {}", e))
                })?;
                let pool = options.connect_lazy_with(connect_opts);
                Ok(Arc::new(crate::provider::MySqlProvider::from_pool(pool))
                    as Arc<dyn IDatabaseProvider>)
            }),
        )
    }

    fn use_mysql_with_tls(&mut self, connection_string: &str, tls: MySqlTlsMode) -> &mut Self {
        self.use_mysql_with(connection_string, move |options| {
            options.tls(tls);
        })
    }
}
