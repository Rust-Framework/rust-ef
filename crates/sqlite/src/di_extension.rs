use crate::provider::SqliteOptions;
use rust_ef::provider::IDatabaseProvider;
use std::sync::Arc;

pub trait DbContextOptionsBuilderExt {
    /// Registers a SQLite provider with the default pool size (8).
    fn use_sqlite(&mut self, connection_string: &str) -> &mut Self;

    /// Registers a SQLite provider and configures it through a delegate.
    ///
    /// ```rust,ignore
    /// options.use_sqlite_with("app.db", |options| {
    ///     options
    ///         .max_pool_size(1)
    ///         .pool_acquire_timeout(Duration::from_secs(5));
    /// });
    /// ```
    fn use_sqlite_with(
        &mut self,
        connection_string: &str,
        configure: impl Fn(&mut SqliteOptions) + Send + Sync + 'static,
    ) -> &mut Self;

    fn use_sqlite_in_memory(&mut self) -> &mut Self;
}

impl DbContextOptionsBuilderExt for rust_ef::db_context::DbContextOptionsBuilder {
    fn use_sqlite(&mut self, connection_string: &str) -> &mut Self {
        if connection_string == ":memory:" {
            return self.use_sqlite_in_memory();
        }
        self.use_sqlite_with(connection_string, |_| {})
    }

    fn use_sqlite_with(
        &mut self,
        connection_string: &str,
        configure: impl Fn(&mut SqliteOptions) + Send + Sync + 'static,
    ) -> &mut Self {
        let path = connection_string.to_string();
        let path_for_factory = path.clone();
        let configure: Arc<dyn Fn(&mut SqliteOptions) + Send + Sync> = Arc::new(configure);
        self.set_provider_factory(
            "sqlite",
            &path,
            Arc::new(move |_cs: &str| {
                let mut options = SqliteOptions::new(&path_for_factory);
                configure(&mut options);
                Ok(
                    Arc::new(crate::provider::SqliteProvider::new_with_options(&options)?)
                        as Arc<dyn IDatabaseProvider>,
                )
            }),
        )
    }

    fn use_sqlite_in_memory(&mut self) -> &mut Self {
        self.set_provider_factory(
            "sqlite",
            ":memory:",
            Arc::new(|_cs: &str| {
                Ok(Arc::new(crate::provider::SqliteProvider::new_in_memory()?)
                    as Arc<dyn IDatabaseProvider>)
            }),
        )
    }
}
