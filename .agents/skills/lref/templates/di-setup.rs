// Template: lrdi DI container setup with add_dbcontext.
//
// Registers DbContext as Scoped. Two resolution modes:
//   - get_owned::<DbContext>() → owned DbContext (&mut self access, recommended for handlers)
//   - get::<DbContext>()       → Arc<DbContext> (shared within a scope, &self access only)
//
// Provider extensions (use_sqlite/use_postgres/use_mysql) inject factory closures
// into DbContextOptions, so the core crate stays fully decoupled.
//
// Multi-DB: use add_dbcontext_keyed("key", |o| ...).

use rust_ef::di::*;                                  // DbContextServiceCollectionExt
use rust_ef::db_context::DbContext;
use rust_dicore::*;                                  // ServiceCollection, ServiceProvider
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;  // .use_sqlite()
// use rust_ef_postgres::DbContextOptionsBuilderExt as _; // .use_postgres()
// use rust_ef_mysql::DbContextOptionsBuilderExt as _;    // .use_mysql()

fn build_provider() -> ServiceProvider {
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

    // --- Owned resolution (recommended: &mut self access, no locks) ---
    let mut ctx: DbContext = provider.get_owned();

    // --- Shared resolution (within a scope: Arc<DbContext>, &self only) ---
    // let scope = provider.create_scope();
    // let ctx: Arc<DbContext> = scope.get();

    // --- Keyed owned resolution (multi-DB) ---
    // let mut primary: DbContext = provider.get_keyed_owned("primary");
    // let mut logs: DbContext = provider.get_keyed_owned("logs");

    ctx.save_changes().await?;
    Ok(())
}

// NOTE: In web applications, handlers own DbContext directly via owned
// resolution. Mark bare T fields with #[inject(owned)] so #[derive(Inject)]
// resolves them via get_owned(). Unmarked fields fall back to Default::default().
// Each request gets a fresh instance — no locks needed:
//
//   #[derive(Inject)]
//   pub struct MyHandler {
//       #[inject(owned)]
//       ctx: DbContext,  // bare T + #[inject(owned)] → get_owned()
//   }
//
//   #[inject(scoped)]
//   #[async_trait]
//   impl IRequestHandler<MyRequest, MyResponse> for MyHandler {
//       async fn handle(&mut self, req: MyRequest) -> Result<MyResponse> {
//           self.ctx.set::<Entity>().add(entity);
//           self.ctx.save_changes().await?;
//           // ...
//       }
//   }
//
// See templates/web-handler-crud.rs for complete handler patterns.
