use rust_ef::provider::IDatabaseProvider;
use std::sync::Arc;

pub trait DbContextOptionsBuilderExt {
    fn use_sqlite(&mut self, connection_string: &str) -> &mut Self;
    fn use_sqlite_in_memory(&mut self) -> &mut Self;
}

impl DbContextOptionsBuilderExt for rust_ef::db_context::DbContextOptionsBuilder {
    fn use_sqlite(&mut self, connection_string: &str) -> &mut Self {
        let cs = connection_string.to_string();
        self.set_provider_factory(
            "sqlite",
            &cs,
            Arc::new(move |cs: &str| {
                Ok(Arc::new(crate::provider::SqliteProvider::new(cs)?)
                    as Arc<dyn IDatabaseProvider>)
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
