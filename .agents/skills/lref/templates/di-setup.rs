// Template: lrdi DI container setup with add_dbcontext<T>.
//
// Registers DbContext as Arc<DbContext> for DI resolution.
// Provider extensions (use_sqlite/use_postgres/use_mysql) inject factory closures
// into DbContextOptions, so the core crate stays fully decoupled.
//
// Multi-DB: use add_dbcontext_keyed("key", |o| ...).

use rust_dicore::ServiceCollection;
use rust_ef::di::*;                                  // DbContextServiceCollectionExt
use rust_ef::db_context::DbContext;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;  // .use_sqlite()
// use rust_ef_postgres::DbContextOptionsBuilderExt as _; // .use_postgres()
// use rust_ef_mysql::DbContextOptionsBuilderExt as _;    // .use_mysql()

fn build_provider() -> rust_dicore::ServiceProvider {
    ServiceCollection::new()
        // --- Register additional services (optional) ---
        // .singleton(|_| Arc::new(Logger::new()))
        // .transient(|p| Arc::new(UserService::new(p.get())))

        // --- Single database (recommended) ---
        .add_dbcontext(|options| {
            options.use_sqlite("data source=app.db");
            // options.use_sqlite_in_memory();
            // options.use_postgres("host=localhost dbname=app user=postgres");
            // options.use_mysql("mysql://user:pass@localhost/db");

            // --- Register SaveChanges interceptors (optional) ---
            // options.add_interceptor(AuditInterceptor);
            // options.add_interceptor(SoftDeleteInterceptor);
        })

        // --- Multiple databases (keyed) ---
        // .add_dbcontext_keyed("primary", |options| {
        //     options.use_postgres("host=primary/db");
        // })
        // .add_dbcontext_keyed("logs", |options| {
        //     options.use_sqlite("logs.db");
        // })

        .build()
        .unwrap()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = build_provider();

    // --- Default resolution (single DB) ---
    let ctx: Arc<DbContext> = provider.get();

    // --- Keyed resolution (multi-DB) ---
    // let primary: Arc<DbContext> = provider.get_keyed("primary");
    // let logs: Arc<DbContext> = provider.get_keyed("logs");

    ctx.save_changes().await?;
    Ok(())
}

// NOTE: In web applications, DbContext is injected as Arc<DbContext>
// via Scoped lifecycle — each request gets its own instance, no locks needed.
// add_dbcontext registers as Scoped by default:
//
//   #[derive(Inject)]
//   pub struct MyHandler {
//       ctx: Arc<DbContext>,
//   }
//
// See templates/web-handler-crud.rs for complete handler patterns.