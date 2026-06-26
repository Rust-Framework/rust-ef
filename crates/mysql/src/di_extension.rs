use rust_ef::provider::IDatabaseProvider;
use std::sync::Arc;

pub trait DbContextOptionsBuilderExt {
    fn use_mysql(&mut self, connection_string: &str) -> &mut Self;
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
}
