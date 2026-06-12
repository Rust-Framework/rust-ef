// Template: lrdi DI container setup with add_dbcontext<T>.
//
// Registers DbContext as Arc<dyn IDbContext> for interface-oriented resolution.
// Provider extensions (use_sqlite/use_postgres/use_mysql) inject factory closures
// into DbContextOptions, so the core crate stays fully decoupled.

use lrdi::ServiceCollection;
use lref::di::*;                                  // DbContextServiceCollectionExt
use lref::db_context::DbContext;
use lref_provider_sqlite::DbContextOptionsBuilderExt as _;  // .use_sqlite()
// use lref_provider_postgres::DbContextOptionsBuilderExt as _; // .use_postgres()
// use lref_provider_mysql::DbContextOptionsBuilderExt as _;    // .use_mysql()

fn build_provider() -> lrdi::ServiceProvider {
    ServiceCollection::new()
        // --- Register additional services (optional) ---
        // .singleton(|_| Arc::new(Logger::new()))
        // .transient(|p| Arc::new(UserService::new(p.get())))

        // --- Register DbContext as dyn IDbContext ---
        .add_dbcontext::<DbContext>(|options| {
            options.use_sqlite("data source=app.db");
            // options.use_sqlite_in_memory();
            // options.use_postgres("host=localhost dbname=app user=postgres");
            // options.use_mysql("mysql://user:pass@localhost/db");
        })
        .build()
        .unwrap()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = build_provider();

    // --- Interface-oriented resolution ---
    let ctx: Arc<dyn IDbContext> = provider.get();

    // --- Or resolve as concrete type (for set::<T>() access) ---
    // let mut app_ctx = DbContext::from_options(&options)?;
    // app_ctx.set::<Blog>().add(blog);

    ctx.save_changes().await?;
    Ok(())
}

// NOTE: provider is NOT Send when using `&mut self` methods.
// `save_changes(&mut self)` requires mutable access.
// The Arc<dyn IDbContext> pattern works for read-only operations.
// For mutation, use `Arc::get_mut()` or resolve a fresh instance.
