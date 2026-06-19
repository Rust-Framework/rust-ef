<p align="center">
  <img src="../assets/logo.svg" alt="LRDI" width="80">
</p>

<h1 align="center">lrdi · Dependency Injection Container</h1>

<p align="center">
  <a href="https://crates.io/crates/lrdi"><img alt="Crates.io" src="https://img.shields.io/crates/v/lrdi.svg?style=flat-square"></a>
  <a href="https://docs.rs/lrdi"><img alt="Docs" src="https://img.shields.io/docsrs/lrdi?style=flat-square"></a>
  <a href="https://opensource.org/licenses/MIT"><img alt="License" src="https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square"></a>
</p>

<p align="center">
  <a href="README.zh.md"><strong>中文</strong></a>
</p>

`lrdi` is the core crate of the LRDI framework — a Rust dependency injection
container inspired by Microsoft.Extensions.DependencyInjection (MEDI).

```toml
[dependencies]
lrdi = "0.1"
```

---

### When should you use this?

**1. You want to decouple service creation from usage**

Instead of `MyService::new(dep1, dep2, dep3)` scattered across your codebase,
you declare once how services are constructed and let the container wire them
up automatically.

```rust
// Without DI: every call site must know how to build everything
let svc = MyService::new(
    Arc::new(Logger::new("app")),
    Arc::new(Cache::new(1024)),
);

// With DI: declare once, resolve anywhere
let provider = ServiceCollection::new()
    .singleton(|_| Arc::new(Logger::new("app")))
    .singleton(|_| Arc::new(Cache::new(1024)))
    .transient(|p| Arc::new(MyService::new(p.get(), p.get())))
    .build()
    .unwrap();

let svc: Arc<MyService> = provider.get();
```

**2. You need to swap implementations for testing**

Change one registration line — no need to touch the code that consumes the
service.

```rust
#[cfg(test)]
fn test_provider() -> ServiceProvider {
    ServiceCollection::new()
        .singleton(|_| Arc::new(MockDatabase::new()))
        .transient(|p| Arc::new(MyApi::new(p.get())))
        .build()
        .unwrap()
}
```

**3. You have objects with different lifetimes**

Some services should be shared globally (database pool), some per-request
(HTTP context), some created fresh each time (value objects). LRDI gives you
three lifetimes to model this naturally.

| Lifetime | Behavior | Typical use |
|----------|----------|-------------|
| **Singleton** | Created once, shared everywhere | Database pool, config, event bus |
| **Scoped** | Created once per scope, shared within scope | HTTP request context, unit of work, transaction |
| **Transient** | Created every time you resolve it | Value objects, DTOs, lightweight stateless services |

```rust
let provider = ServiceCollection::new()
    .singleton(|_| Arc::new(Pool::new()))           // one pool for the app
    .scoped(|_| Arc::new(RequestContext::new()))     // one per request
    .transient(|_| Arc::new(Query::new()))           // new each time
    .build()
    .unwrap();
```

**4. You want multiple instances of the same type**

Keyed services let you register several implementations of the same trait,
each distinguished by a string key.

```rust
let provider = ServiceCollection::new()
    .keyed::<dyn Strategy>("fast", |_| Arc::new(FastPath))
    .keyed::<dyn Strategy>("safe", |_| Arc::new(SafePath))
    .build()
    .unwrap();

let fast: Arc<dyn Strategy> = provider.get_keyed("fast");
let safe: Arc<dyn Strategy> = provider.get_keyed("safe");
```

**5. You are building a plugin / modular system**

- **Layered containers**: child-first, root-fallback resolution for plugin
  isolation. The plugin sees its own services first; host services are visible
  as fallback.
- **Named services**: register services by string name — critical for cdylib
  plugins where Rust's `TypeId` differs across compilation units.
- **`IServiceLocator`**: pass a unified DI interface to external modules that
  don't depend on LRDI directly.

---

### Quick Reference · API

### Registration (`ServiceCollection`)

| Method | Lifetime | Use when |
|--------|----------|----------|
| `.from_injected()` | *mixed* | Collect all `#[lrdi::inject]` annotations (see below) |
| `.singleton(f)` | Singleton | You need exactly one instance |
| `.scoped(f)` | Scoped | You need one instance per scope |
| `.transient(f)` | Transient | You need a new instance every time |
| `.keyed(k, f)` | Singleton | You need multiple named instances |
| `.keyed_scoped(k, f)` | Scoped | Multiple named scoped instances |
| `.keyed_transient(k, f)` | Transient | Multiple named transient instances |
| `.instance(arc)` | Singleton | You already have a built `Arc<T>` |
| `.try_add(f)` | Singleton | Only register if not already present |
| `.singleton_value(v)` | Singleton | Register a plain value (wraps in `Arc`) |
| `.add(lt, f)` | *any* | Specify lifetime explicitly |

The factory closure `f` receives `&dyn IServiceResolver` so you can resolve
dependencies:

```rust
ServiceCollection::new()
    .singleton(|_| Arc::new(Pool::new()))
    .transient(|p| Arc::new(Repo::new(p.get::<Pool>())))
    .build()
    .unwrap();
```

#### `#[lrdi::inject]` — Attribute-based auto-registration (recommended)

The preferred way to register services is to annotate structs directly:

```rust
use lrdi::*;

#[lrdi::inject(singleton)]     // registers as concrete type
struct Config { url: String }

#[lrdi::inject(transient, as = dyn UserRepo)]  // registers as trait
struct PgUserRepo { db: Arc<DbPool> }

#[lrdi::inject(singleton, as = [dyn EventHandler, dyn StartupTask])]  // multi-trait
struct BootLoader;

// Then build from all annotations in one call:
let provider = ServiceCollection::from_injected().build().unwrap();
```

The `#[lrdi::inject]` attribute automatically generates constructor code
(like `#[derive(Inject)]`) and registers the service via `inventory`
(on Linux, macOS, and Windows).
Supports both named-field structs and unit structs (zero fields):

```rust
// Unit struct — zero fields, works out of the box
#[lrdi::inject(singleton, as = dyn IDynamicAuthorizer)]
struct RoleAuthorizer;

// Named struct — fields resolved from DI container
#[lrdi::inject(transient)]
struct Worker { logger: Arc<Config> }
```

#### Resolution (`ServiceProvider` / `Scope`)

| Method | Returns | Behavior |
|--------|---------|----------|
| `.get::<T>()` | `Arc<T>` | Panics if not registered |
| `.get_service::<T>()` | `Option<Arc<T>>` | Returns `None` if not registered |
| `.get_required_service::<T>()` | `Arc<T>` | Panics with descriptive message |
| `.get_keyed::<T>(key)` | `Arc<T>` | Panics if key not found |
| `.try_get_keyed::<T>(key)` | `Option<Arc<T>>` | Returns `None` if key not found |
| `.get_all::<T>()` | `Vec<Arc<T>>` | All registered instances (keyed + unkeyed) |
| `.get_named::<T>(name)` | `Arc<T>` | Named resolution (cross-DLL) |
| `.get_named_any::<T>(name)` | `Option<Arc<T>>` | Named, returns `None` if missing |
| `.create_scope()` | `Scope` | New scope for scoped services |

`T` can be a concrete type or `dyn Trait`:

```rust
let svc: Arc<MyService> = provider.get();
let plugin: Arc<dyn IPlugin> = provider.get();
let all: Vec<Arc<dyn IPlugin>> = provider.get_all();
```

---

### Flexible Application Patterns

#### 🔹 Three-layered architecture (Controller → Service → Repository)

```rust
let provider = ServiceCollection::new()
    .singleton(|_| Arc::new(DbPool::new(&config)))
    .transient(|p| Arc::new(UserRepo::new(p.get::<DbPool>())))
    .transient(|p| Arc::new(UserService::new(p.get::<UserRepo>())))
    .transient(|p| Arc::new(UserController::new(p.get::<UserService>())))
    .build()
    .unwrap();
```

#### 🔹 Strategy pattern with keyed services

```rust
let provider = ServiceCollection::new()
    .keyed::<dyn PaymentGateway>("credit", |_| Arc::new(CreditCardGateway))
    .keyed::<dyn PaymentGateway>("alipay", |_| Arc::new(AlipayGateway))
    .keyed::<dyn PaymentGateway>("wechat", |_| Arc::new(WechatGateway))
    .build()
    .unwrap();

fn checkout(provider: &ServiceProvider, method: &str) {
    let gateway: Arc<dyn PaymentGateway> = provider.get_keyed(method);
    gateway.charge(100);
}
```

#### 🔹 Scoped per-request (web server)

```rust
fn handle_request(container: &ServiceProvider) {
    let scope = container.create_scope();
    let ctx: Arc<RequestContext> = scope.get();
    let svc: Arc<MyService> = scope.get();
    // ctx and MyService share the same scope — scoped services are cached
    // Drops when scope goes out of scope
}
```

#### 🔹 Plugin isolation with ServiceProviderWrapper

```rust
let host_provider = ServiceCollection::new()
    .singleton(|_| Arc::new(HostService::new()))
    .build().unwrap();

let plugin_provider = ServiceCollection::new()
    .singleton(|_| Arc::new(PluginService::new()))
    .build().unwrap();

let wrapper = ServiceProviderWrapper::new(plugin_provider, host_provider);
// PluginService resolved from plugin container
// HostService falls back to host container
```

#### 🔹 Cross-DLL plugin with named services

```rust
// Host process
let provider = ServiceCollection::new()
    .singleton(|_| Arc::new(EventBus::new()))
    .build().unwrap();
provider.register_named("event_bus", provider.get::<EventBus>());

// Plugin loaded from cdylib (separate compilation unit)
let bus = host.get_named::<EventBus>("event_bus");
```

#### 🔹 External system integration via IServiceLocator

```rust
let locator: Arc<dyn IServiceLocator> = Arc::new(ServiceLocatorBridge::new(
    Arc::new(RdiProvider::Root(provider)),
));
// Pass to third-party code or FFI boundary
```

---

### Architecture

```
┌──────────────────────────────────────────────┐
│               ServiceCollection               │
│  ┌──────────┐ ┌──────────┐ ┌──────────────┐ │
│  │singleton │ │  scoped  │ │  transient   │ │
│  │ .keyed() │ │.keyed_*()│ │ .instance()  │ │
│  └──────────┘ └──────────┘ └──────────────┘ │
│                   │ .build()                  │
│                   ▼                           │
│              ServiceProvider                  │
│  ┌──────────┐ ┌──────────┐ ┌──────────────┐ │
│  │  .get()  │ │.get_keyed│ │ .create_scope│ │
│  │ .get_all │ │.get_named│ │              │ │
│  └──────────┘ └──────────┘ └──────┬───────┘ │
│                                    │         │
└────────────────────────────────────┼─────────┘
                                     │
                                     ▼
                               ┌──────────┐
                               │   Scope  │
                               │ .get()   │
                               │ scoped   │
                               │ cached   │
                               └──────────┘
```

---

### Relationship with lrdi-macros

`lrdi` works with or without [`lrdi-macros`](../lrdi-macros/). The macros
provide three compile-time conveniences:

- **`#[lrdi::inject(...)]`** (recommended) — Attribute macro that combines
  constructor generation + auto-registration. One annotation on a struct is
  all you need — `ServiceCollection::from_injected()` collects everything.
- **`#[derive(Inject)]`** — Generate the factory function automatically from
  struct fields. Used internally by `#[lrdi::inject]`.
- **`#[lrdi::module]`** — Collect `lrdi::inject!()` declarations at compile
  time and generate a complete provider builder. Useful for external types,
  conditional compilation, and centralized management.

See [`lrdi-macros/README.md`](../lrdi-macros/README.md) for details.

---

### License

MIT.

