# Security Policy

## Supported versions

| Version | Supported |
|---------|-----------|
| 2.1.x | ✅ |
| 2.0.x | ✅ (security fixes only) |
| < 2.0 | ❌ |

## Reporting a vulnerability

Please **do not** open a public issue for security-sensitive reports.
Use GitHub's private vulnerability reporting:

<https://github.com/Rust-Framework/rust-ef/security/advisories/new>

Include a description, the affected version(s), a proof of concept when
possible, and any suggested mitigation. We aim to acknowledge reports within
**72 hours** and to ship a fix or documented mitigation within **30 days** for
confirmed issues. We will credit reporters in the advisory unless asked not to.

## Security design notes

- **No `unsafe` code.** All library crates opt into `#![forbid(unsafe_code)]`.
- **Parameterized SQL.** Values are bound through `DbValue` parameters; table and
  column identifiers come from compile-time entity metadata, not user input.
  APIs that accept raw expressions/SQL place that responsibility on the caller.
- **TLS.** PostgreSQL and MySQL providers expose explicit TLS configuration
  (`PgTlsMode`, `MySqlTlsMode`); TLS remains the recommended transport.
- **Multi-tenancy.** Global query filters are applied to SELECT/UPDATE/DELETE and
  navigation loads; INSERT values (including tenant ids) are set by the
  application.
- **Panic containment.** Provider initialization failures during DI registration
  fail fast at startup; rust-dix converts factory panics into typed `RdiError`
  values during resolution.
- **Disposal contract.** A `ServiceProvider`/`Scope` must not be resolved from
  concurrently with `dispose()`; after disposal, resolution returns typed
  `ProviderDisposed` / `ScopeDisposed` errors. `get_owned` instances are
  caller-owned and are never tracked or disposed by the container.
- **Supply chain.** CI runs `cargo test`, `cargo clippy -D warnings`,
  `rustdoc -D warnings`, an MSRV (`1.95`) check, and a RustSec `cargo audit` on
  every push. Releases are published by GitHub Actions on `v*` tag pushes using
  the `CARGO_REGISTRY_TOKEN` secret (tag pushes are not signature-verified by the
  workflow; repository write access is required to create the tag).
- **MSRV.** The minimum supported Rust version is declared in `Cargo.toml`
  (`rust-version = "1.95"`) and verified in CI.

## Out of scope

- Vulnerabilities in user-provided entity configurations, interceptors, model
  builders, raw SQL/expressions, or database servers.
- Denial of service caused by resolving from a container concurrently with
  `dispose()`, or by calling the container from `Drop`/cleanup closures after
  disposal (outside the documented contract).
- `get_owned` instances: the caller owns and must dispose those.
