/// TLS mode for PostgreSQL connections.
///
/// `Disable` uses plaintext connections (`tokio_postgres::NoTls`) — the
/// pre-v1.4 default, kept for backward compatibility.
///
/// `Require` enforces TLS via the platform's native TLS implementation
/// (SChannel on Windows, OpenSSL on Linux, Secure Transport on macOS). The
/// connector is cloned per pool acquisition, so it must be `Clone`
/// (`native_tls::TlsConnector` satisfies this).
#[derive(Clone)]
pub enum PgTlsMode {
    /// Plaintext connection (backward compatible with v1.3).
    Disable,
    /// Enforce TLS using the provided `native_tls::TlsConnector`.
    Require(native_tls::TlsConnector),
}
