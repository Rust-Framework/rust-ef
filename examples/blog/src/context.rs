//! DbContext definition and Fluent API configuration for the blog example.

use lref::prelude::*;
use lref::provider::IDatabaseProvider;
use lref_provider_postgres::PostgresProvider;
use std::sync::Arc;

use super::entities::{Blog, Post};

pub struct BloggingContext {
    pub blogs: DbSet<Blog>,
    pub posts: DbSet<Post>,
    change_tracker: ChangeTracker,
    provider: Arc<PostgresProvider>,
}

impl BloggingContext {
    pub async fn new() -> Result<Self, LrefError> {
        let pg_provider =
            PostgresProvider::new("postgres://postgres:postgres@localhost/blogging", 5)?;
        let provider = Arc::new(pg_provider);

        Ok(Self {
            blogs: DbSet::with_provider("blogs", provider.clone() as Arc<dyn IDatabaseProvider>),
            posts: DbSet::with_provider("posts", provider.clone() as Arc<dyn IDatabaseProvider>),
            change_tracker: ChangeTracker::new(),
            provider,
        })
    }

    pub fn change_tracker(&self) -> &ChangeTracker {
        &self.change_tracker
    }
}

#[async_trait::async_trait]
impl IDbContext for BloggingContext {
    type Provider = PostgresProvider;
    fn provider(&self) -> &Self::Provider {
        &self.provider
    }
    fn change_tracker_mut(&mut self) -> &mut ChangeTracker {
        &mut self.change_tracker
    }
    fn change_tracker(&self) -> &ChangeTracker {
        &self.change_tracker
    }
    async fn save_changes(&mut self) -> Result<SaveChangesResult, LrefError> {
        // Get provider reference via Arc clone to avoid borrow conflicts
        let provider = Arc::clone(&self.provider);
        let mut conn = provider.get_connection().await?;
        conn.begin_transaction().await?;

        let (a1, u1, d1) =
            lref::db_context::save_one_set(&mut *conn, &*provider, &mut self.blogs).await?;
        let (a2, u2, d2) =
            lref::db_context::save_one_set(&mut *conn, &*provider, &mut self.posts).await?;

        conn.commit_transaction().await?;
        self.change_tracker.accept_all_changes();
        self.blogs.clear_entries();
        self.posts.clear_entries();

        Ok(SaveChangesResult {
            added: a1 + a2,
            updated: u1 + u2,
            deleted: d1 + d2,
        })
    }
}

// ---------------------------------------------------------------------------
// Fluent API Configuration
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct BlogConfiguration;

impl IEntityTypeConfiguration<Blog> for BlogConfiguration {
    fn configure(&self, entity: &mut EntityTypeBuilder<Blog>) {
        entity
            .to_table("blogs")
            .has_key(|b| &b.blog_id)
            .property(|b| &b.url)
            .is_required()
            .has_max_length(200)
            .has_column_name("Url");

        entity
            .has_many(|b| &b.posts)
            .with_one(|p| &p.blog)
            .has_foreign_key(|p| &p.blog_id);
    }
}

#[derive(Default)]
pub struct PostConfiguration;

impl IEntityTypeConfiguration<Post> for PostConfiguration {
    fn configure(&self, entity: &mut EntityTypeBuilder<Post>) {
        entity
            .to_table("posts")
            .has_key(|p| &p.post_id)
            .property(|p| &p.title)
            .is_required()
            .has_max_length(200)
            .has_column_name("Title");

        entity.property(|p| &p.content).has_column_name("Content");
    }
}

impl BloggingContext {
    #[allow(dead_code)]
    fn on_model_creating(model_builder: &mut ModelBuilder) {
        model_builder
            .apply_configuration::<BlogConfiguration, Blog>()
            .apply_configuration::<PostConfiguration, Post>();

        model_builder.entity::<Blog>().has_data(&[Blog {
            blog_id: 1,
            url: "https://devblogs.microsoft.com/dotnet".into(),
            rating: 5,
            posts: HasMany::new(),
        }]);
    }
}
