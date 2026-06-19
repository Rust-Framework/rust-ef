//! rust-dicore -- Rust Dependency Injection Framework
//!
//! A dependency injection container for Rust inspired by
//! Microsoft.Extensions.DependencyInjection (MEDI).
//!
//! # Quick start
//!
//! ```rust
//! use rust_dicore::*;
//! use std::sync::Arc;
//!
//! struct Logger(String);
//! struct Worker { log: Arc<Logger> }
//!
//! let provider = ServiceCollection::new()
//!     .singleton(|_| Arc::new(Logger("INFO".into())))
//!     .singleton(|_| Arc::new(Worker { log: Arc::new(Logger("INFO".into())) }))
//!     .build()
//!     .unwrap();
//!
//! let logger: Arc<Logger> = provider.get::<Logger>();
//! ```
//!
//! # Features
//!
//! - **Three lifetimes**: Singleton, Scoped, Transient
//! - **Keyed services**: multiple instances of the same type, distinguished by key
//! - **Constructor injection**: `#[derive(Inject)]` on structs
//! - **Compile-time module scanning**: `#[rust_dicore::module]` collects service registrations
//! - **Cross-DLL support**: named service registry for cdylib plugins
//!
//! # Crate layout
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`ServiceCollection`] | Register services with a builder API |
//! | [`ServiceProvider`] | The root DI container |
//! | [`Scope`] / [`ServiceScope`] | Scoped container (one per scope) |
//! | [`IServiceResolver`] | Resolution trait (implemented by `ServiceProvider` & `Scope`) |
//! | [`ServiceProviderWrapper`] | Child-first layered container |
//! | [`ServiceLifetime`] | Enum: Singleton, Scoped, Transient |
//! | [`IServiceLocator`] / [`ServiceLocatorBridge`] | Cross-DLL / plugin integration |
//! | [`RdiError`] | Error types |
//!
//! # Proc-macros
//!
//! - `#[derive(Inject)]` 閳?auto-generates constructor injection code
//! - `#[rust_dicore::module]` 閳?compile-time module scanning
//! - `rust_dicore::inject!(...)` 閳?declare services inside `#[rust_dicore::module]`

pub mod bridge;
pub mod collection;
pub mod descriptor;
pub mod entry;
pub mod error;
pub mod lifetime;
pub mod provider;
pub mod registration;
pub mod resolve;
pub mod scope;
pub mod service_locator;
pub mod store;
pub mod wrapper;

// 閳光偓閳光偓 Core types (MEDI-inspired naming) 閳光偓閳光偓

/// Service collection 閳?register services, then call `.build()`.
///
/// Analogous to `IServiceCollection` in Microsoft.Extensions.DependencyInjection.
pub use collection::ServiceCollection;

/// Built service provider 閳?the root DI container (read-only after build).
///
/// Analogous to `IServiceProvider` in MEDI.
pub use provider::ServiceProvider;

/// Service descriptor containing registration metadata.
pub use descriptor::ServiceDescriptor;

/// Service lifetime enum: [`Singleton`](ServiceLifetime::Singleton),
/// [`Scoped`](ServiceLifetime::Scoped), [`Transient`](ServiceLifetime::Transient).
pub use lifetime::ServiceLifetime;

/// Core resolution trait. Both [`ServiceProvider`] and [`Scope`] implement this.
pub use entry::IServiceResolver;

/// A scoped service provider 閳?created via [`ServiceProvider::create_scope`].
///
/// Analogous to `IServiceScope` in MEDI.
pub use scope::Scope;

/// Alias for [`Scope`] with MEDI-inspired naming (`IServiceScope`).
pub use scope::Scope as ServiceScope;

/// Internal types used by the container.
pub use entry::{ServiceEntry, ServiceFactory};

/// Wrapper combining child + root provider with child-first resolution.
pub use wrapper::ServiceProviderWrapper;

// 閳光偓閳光偓 Plugin / cross-DLL support 閳光偓閳光偓

/// Trait for service location (type-based + named) 閳?used for external integration.
pub use service_locator::IServiceLocator;

/// Trait for named service registration.
pub use service_locator::INamedRegistrar;

/// Adapts RDI types to [`IServiceLocator`].
pub use bridge::{RdiProvider, ServiceLocatorBridge};

// 閳光偓閳光偓 Error types 閳光偓閳光偓

/// Error types returned from service resolution.
pub use error::RdiError;

// 閳光偓閳光偓 Runtime registration 閳光偓閳光偓

pub use registration::ServiceRegistration;

/// Re-export of `inventory` so that `#[rust_dicore::inject]` generated code
/// can call `rust_dicore::inventory::submit!` without requiring downstream crates
/// to add `inventory` to their own `Cargo.toml`.
#[doc(hidden)]
pub use inventory;

// 閳光偓閳光偓 Proc-macros 閳光偓閳光偓

// Note: `inject_attr` is the attribute macro; `inject` is the declare macro
// used inside `#[rust_dicore::module]`. Users write both as `#[rust_dicore::inject(...)]`
// and `rust_dicore::inject!(...)` respectively.
pub use rust_dicore_macros::{inject, inject_attr, module, Inject};
