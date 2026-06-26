//! Application `DbContext` using type-map set storage.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::entity::IEntityType;
use rust_ef::error::EFResult;
use rust_ef::migration::{MigrationDialect, MigrationEngine};
use rust_ef::provider::IDatabaseProvider;
use rust_ef_sqlite::SqliteProvider;
use std::sync::Arc;

use super::entities::{Blog, Post};

/// Creates an in-memory SQLite `DbContext` with Blog/Post schema.
pub async fn create_blog_context() -> EFResult<DbContext> {
    let provider = Arc::new(SqliteProvider::new_in_memory()?);
    let metas = vec![Blog::entity_meta(), Post::entity_meta()];
    MigrationEngine::new(MigrationDialect::Sqlite)
        .ensure_created(&*provider, &metas)
        .await?;

    let shared = provider.clone();
    #[allow(clippy::type_complexity)]
    let factory: Arc<dyn Fn(&str) -> EFResult<Arc<dyn IDatabaseProvider>> + Send + Sync> =
        Arc::new(move |_| Ok(shared.clone() as Arc<dyn IDatabaseProvider>));

    let mut builder = DbContextOptionsBuilder::new();
    builder.connection_string(":memory:");
    builder.set_provider_factory("sqlite", ":memory:", factory);
    DbContext::from_options(&builder.build())
}
